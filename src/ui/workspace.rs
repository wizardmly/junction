//! The main window, laid out like IntelliJ's new UI: title bar with project
//! and VCS widgets, a left tool window stripe, the Commit tool window, the
//! editor area (diff), the bottom Git tool window (Log / Console), and the
//! status bar.

use std::rc::Rc;
use gpui_kit::component::{
    Disableable as _,
    Selectable as _,
    ActiveTheme as _, Icon, Sizable as _, TitleBar, WindowExt as _, h_flex,
    button::{Button, ButtonVariants as _},
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem},
    notification::Notification,
    popover::Popover,
    resizable_panel,
    scroll::ScrollableElement as _,
    v_flex, v_resizable, h_resizable,
};
use gpui_kit::assets::IconName;
use gpui_kit::{
    AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, KeyBinding,
    ParentElement as _, PathPromptOptions, Render, actions, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _, px,
};

use crate::model::{RepoEvent, RepoModel};
use crate::theme::{self, ActivePalette as _};
use crate::ui::blame_view::{BlameEvent, BlameView};
use crate::ui::branches_popup::{self, BranchesPopup};
use crate::ui::commit_view::{CommitEvent, CommitView};
use crate::ui::common::tool_button;
use crate::ui::diff_view::{DiffView, NextDifference, PreviousDifference};
use crate::settings::Settings;
use crate::ui::dialogs;
use crate::ui::merge_view::{MergeEvent, MergeView};
use crate::git::RepositoryState;
use crate::git::merge::{self, Conflict, OperationStep};
use crate::ui::log_view::{LogEvent, LogView};
use crate::ui::clone_dialog;
use crate::ui::remote_dialogs;
use crate::ui::file_editor::{FileEditor, FileEditorEvent};
use crate::ui::patch_dialogs;
use crate::ui::shelf_view::{ShelfEvent, ShelfView};
use crate::ui::stash_view::{StashEvent, StashView};

actions!(workspace, [CommitChanges, PushChanges, UpdateProject, ShowBranches, ToggleGitWindow, Refresh, StashChanges, OpenSettings, VcsOperations]);

const CONTEXT: &str = "Workspace";

/// IntelliJ's default keymap for the VCS actions.
pub fn init(cx: &mut gpui_kit::App) {
    cx.bind_keys([
        KeyBinding::new("secondary-k", CommitChanges, Some(CONTEXT)),
        KeyBinding::new("secondary-shift-k", PushChanges, Some(CONTEXT)),
        KeyBinding::new("secondary-t", UpdateProject, Some(CONTEXT)),
        KeyBinding::new("secondary-shift-`", ShowBranches, Some(CONTEXT)),
        KeyBinding::new("alt-9", ToggleGitWindow, Some(CONTEXT)),
        KeyBinding::new("secondary-alt-y", Refresh, Some(CONTEXT)),
        KeyBinding::new("secondary-alt-s", OpenSettings, Some(CONTEXT)),
        // VCS Operations popup: Alt+` (Ctrl+V on macOS).
        #[cfg(target_os = "macos")]
        KeyBinding::new("ctrl-v", VcsOperations, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("alt-`", VcsOperations, Some(CONTEXT)),
        KeyBinding::new("f7", NextDifference, Some(CONTEXT)),
        KeyBinding::new("shift-f7", PreviousDifference, Some(CONTEXT)),
    ]);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LeftTab {
    Commit,
    Stash,
    Shelf,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BottomTab {
    Log,
    Console,
}

struct LogTab {
    title: String,
    filter: crate::git::LogFilter,
    /// Each tab keeps its own selected commit.
    selected: Option<String>,
}

pub struct Workspace {
    model: Entity<RepoModel>,
    log: Entity<LogView>,
    commit: Entity<CommitView>,
    stash: Entity<StashView>,
    shelf: Entity<ShelfView>,
    diff: Entity<DiffView>,
    branches_popup: Entity<BranchesPopup>,
    /// The merge tool, shown in the editor area instead of the diff.
    merge: Option<(Entity<MergeView>, Subscription)>,
    /// Git tool window Log tabs; the first is the main "Log".
    log_tabs: Vec<LogTab>,
    active_log: usize,
    /// Annotate with Git Blame, shown instead of the diff until closed.
    blame: Option<(Entity<BlameView>, Subscription)>,
    editor: Option<(Entity<FileEditor>, Subscription)>,
    show_commit: bool,
    show_git: bool,
    left_tab: LeftTab,
    bottom_tab: BottomTab,
    branches_open: bool,
    focus: gpui_kit::FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(model: Entity<RepoModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let log = cx.new(|cx| LogView::new(model.clone(), window, cx));
        let commit = cx.new(|cx| CommitView::new(model.clone(), window, cx));
        let diff = cx.new(|_| DiffView::new());
        let stash = cx.new(|cx| StashView::new(model.clone(), cx));
        let shelf = cx.new(|cx| ShelfView::new(model.clone(), cx));
        let branches_popup = cx.new(|cx| BranchesPopup::new(model.clone(), window, cx));
        let weak = cx.entity().downgrade();
        branches_popup.update(cx, |popup, _| {
            popup.on_commit = Some(Rc::new(move |window, cx| {
                weak.update(cx, |this, cx| this.on_commit(&CommitChanges, window, cx)).ok();
            }));
        });
        let subscriptions = vec![
            cx.subscribe_in(&log, window, |this, _, event: &LogEvent, window, cx| match event {
                LogEvent::OpenDiff(source) => this.open_diff(source.clone(), cx),
                LogEvent::Annotate { path, revision } => this.annotate(path.clone(), revision.clone(), cx),
                LogEvent::OpenFile { path, revision } => this.open_file(path.clone(), revision.clone(), window, cx),
            }),
            cx.subscribe_in(&commit, window, |this, _, event: &CommitEvent, window, cx| match event {
                CommitEvent::OpenDiff(source) => this.open_diff(source.clone(), cx),
                CommitEvent::OpenPush => dialogs::push(this.model.clone(), window, cx),
                CommitEvent::OpenMerge(conflict) => this.open_merge(conflict.clone(), window, cx),
                CommitEvent::Annotate(path) => this.annotate(path.clone(), None, cx),
                CommitEvent::ShowHistory(path) => this.show_history(path.clone(), cx),
                CommitEvent::EditSource(path) => this.open_file(path.clone(), None, window, cx),
                CommitEvent::CompareWith(path) => {
                    let workspace = cx.entity();
                    dialogs::compare_file_with(
                        this.model.clone(),
                        path.clone(),
                        Rc::new(move |source, _, cx| workspace.update(cx, |this, cx| this.open_diff(source, cx))),
                        window,
                        cx,
                    );
                }
            }),
            cx.subscribe(&stash, |this, _, event: &StashEvent, cx| match event {
                StashEvent::OpenDiff(source) => this.open_diff(source.clone(), cx),
            }),
            cx.subscribe(&shelf, |this, _, event: &ShelfEvent, cx| match event {
                ShelfEvent::OpenDiff(source) => this.open_diff(source.clone(), cx),
            }),
            cx.subscribe_in(&model, window, |this, _, event, window, cx| {
                if let RepoEvent::OpenLogTab { title, filter } = event {
                    this.open_log_tab(title.clone(), filter.clone(), cx);
                }
                if let RepoEvent::PrefillCommitMessage(_) = event {
                    this.show_commit = true;
                    this.left_tab = LeftTab::Commit;
                    cx.notify();
                }
                if let RepoEvent::Compare { old, new } = event {
                    let workspace = cx.entity();
                    dialogs::compare_files(
                        this.model.clone(),
                        old.clone(),
                        new.clone(),
                        Rc::new(move |source, _, cx| workspace.update(cx, |this, cx| this.open_diff(source, cx))),
                        window,
                        cx,
                    );
                }
                if let RepoEvent::Notify { title, message, error } = event {
                    // Quiet operations (Stage / Unstage) report only failures.
                    if message.is_empty() && !*error {
                        return;
                    }
                    // Update Project: "N files updated in M commits" with View Commits.
                    let (message, updated_range) = match message.split_once('\u{1f}') {
                        Some((text, range)) => (text.to_owned(), Some(range.to_owned())),
                        None => (message.clone(), None),
                    };
                    let message = &message;
                    let entity = cx.entity();
                    let mut notification = if *error {
                        // A short summary; the full output is in the Console tab.
                        let detail = message.split_once(" failed: ").map_or(message.as_str(), |(_, rest)| rest);
                        let summary: Vec<&str> = detail.lines().filter(|l| !l.trim().is_empty()).take(2).collect();
                        Notification::error(summary.join("\n")).title(title.clone())
                    } else {
                        Notification::success(message.clone()).title(title.clone())
                    };
                    // IntelliJ's balloon actions: the obvious next step.
                    if message.starts_with("Push rejected") {
                        notification = notification.action(move |_, _, _| {
                            let entity = entity.clone();
                            Button::new("notify-update").label("Update Project…").small().primary().on_click(move |_, window, cx| {
                                let model = entity.read(cx).model.clone();
                                dialogs::update_project(model, window, cx);
                            })
                        });
                    } else if *error {
                        notification = notification.action(move |_, _, _| {
                            let entity = entity.clone();
                            Button::new("notify-details").label("Show Details").small().outline().on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.show_git = true;
                                    this.bottom_tab = BottomTab::Console;
                                    cx.notify();
                                })
                            })
                        });
                    } else if let Some(range) = updated_range {
                        // "View Files": the Updated Files tree for the pulled range.
                        let files_entity = entity.clone();
                        let files_range = range.clone();
                        notification = notification.content(move |_, _, cx| {
                            let palette = cx.palette().clone();
                            let (entity, range) = (files_entity.clone(), files_range.clone());
                            div()
                                .id("notify-view-files")
                                .text_sm()
                                .text_color(palette.link)
                                .cursor_pointer()
                                .child("View Files")
                                .on_click(move |_, _, cx| {
                                    let Some((old, new)) = range.split_once("..") else { return };
                                    let (old, new) = (old.to_owned(), new.to_owned());
                                    entity.update(cx, |this, cx| this.model.update(cx, |m, cx| m.compare(old, Some(new), cx)));
                                })
                                .into_any_element()
                        });
                        notification = notification.action(move |_, _, _| {
                            let entity = entity.clone();
                            let range = range.clone();
                            Button::new("notify-view-commits").label("View Commits").small().outline().on_click(move |_, _, cx| {
                                let range = range.clone();
                                entity.update(cx, |this, cx| {
                                    let filter = crate::git::LogFilter { branches: vec![range], ..Default::default() };
                                    this.open_log_tab("Update Info".into(), filter, cx);
                                })
                            })
                        })
                        .autohide(true);
                    } else if title == "Commit" {
                        // The balloon's "Undo" link undoes exactly this commit.
                        let committed = this.model.read(cx).repository().and_then(|r| r.run(["rev-parse", "HEAD"]).ok()).map(|h| h.trim().to_owned());
                        let undo_entity = entity.clone();
                        notification = notification.content(move |_, _, cx| {
                            let palette = cx.palette().clone();
                            let (entity, hash) = (undo_entity.clone(), committed.clone());
                            v_flex()
                                .child(
                                    div()
                                        .id("notify-undo")
                                        .text_sm()
                                        .text_color(palette.link)
                                        .cursor_pointer()
                                        .child("Undo")
                                        .on_click(move |_, _, cx| {
                                            let Some(hash) = hash.clone() else { return };
                                            entity.update(cx, |this, cx| this.model.update(cx, |m, cx| m.undo_commit(hash, cx)));
                                        }),
                                )
                                .into_any_element()
                        });
                        notification = notification.action(move |_, _, _| {
                            let entity = entity.clone();
                            Button::new("notify-view").label("View Commit").small().outline().on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.show_git = true;
                                    this.bottom_tab = BottomTab::Log;
                                    let head = this.model.read(cx).refs().head_commit.clone();
                                    this.model.update(cx, |m, cx| m.select_hash(head, cx));
                                    cx.notify();
                                })
                            })
                        })
                        .autohide(true);
                    }
                    window.push_notification(notification, cx);
                }
            }),
            cx.observe(&model, |_, _, cx| cx.notify()),
            // A gutter Rollback / Stage / Unstage in the diff changed files.
            cx.subscribe(&diff, |this, _, _: &crate::ui::diff_view::FilesChanged, cx| {
                this.model.update(cx, |m, cx| m.reload(cx));
            }),
            // Running an action from the branches popup closes it.
            cx.subscribe(&branches_popup, |this, _, _: &gpui_kit::DismissEvent, cx| {
                this.branches_open = false;
                cx.notify();
            }),
        ];
        Self {
            model,
            log,
            commit,
            stash,
            shelf,
            diff,
            branches_popup,
            merge: None,
            log_tabs: vec![LogTab { title: "Log".into(), filter: Default::default(), selected: None }],
            active_log: 0,
            blame: None,
            editor: None,
            show_commit: true,
            show_git: true,
            left_tab: LeftTab::Commit,
            branches_open: false,
            focus: cx.focus_handle(),
            bottom_tab: BottomTab::Log,
            _subscriptions: subscriptions,
        }
    }

    fn show_conflicts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = cx.entity();
        dialogs::conflicts(
            self.model.clone(),
            Rc::new(move |conflict, window, cx| workspace.update(cx, |this, cx| this.open_merge(conflict, window, cx))),
            window,
            cx,
        );
    }

    pub fn open_merge(&mut self, conflict: Conflict, window: &mut Window, cx: &mut Context<Self>) {
        if !conflict.kind.can_merge() {
            return;
        }
        let Some(repository) = self.model.read(cx).repository().cloned() else { return };
        if self.merge.as_ref().is_some_and(|(m, _)| m.read(cx).path() == conflict.path) {
            return;
        }
        let model = self.model.clone();
        let view = cx.new(|cx| MergeView::new(model, repository, conflict, cx));
        let subscription = cx.subscribe_in(&view, window, |this, _, event: &MergeEvent, window, cx| match event {
            MergeEvent::Closed(applied) => {
                this.merge = None;
                // Back to the Conflicts dialog while other files still conflict.
                let others = merge::conflicts(this.model.read(cx).status()).len() > 1;
                if *applied && others {
                    this.show_conflicts(window, cx);
                }
                cx.notify();
            }
        });
        self.merge = Some((view, subscription));
        cx.notify();
    }

    /// "Rebasing main · 2 conflicts · Resolve… Continue Skip Abort", above the editor.
    fn render_operation_banner(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let model = self.model.read(cx);
        let state = model.state();
        if state == RepositoryState::Normal {
            return None;
        }
        let palette = cx.palette().clone();
        let conflicts = merge::conflicts(model.status());
        let label = match state {
            RepositoryState::Rebasing => "Rebase in progress",
            RepositoryState::Merging => "Merge in progress",
            RepositoryState::CherryPicking => "Cherry-pick in progress",
            RepositoryState::Reverting => "Revert in progress",
            RepositoryState::Normal => "",
        };
        let step = |step: OperationStep| {
            let model = self.model.clone();
            move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut gpui_kit::App| {
                model.update(cx, |m, cx| m.run_operation("Git", move |repo| merge::step(repo, state, step), cx));
            }
        };
        let entity = cx.entity();
        Some(
            h_flex()
                .h(px(34.))
                .px_3()
                .gap_2()
                .text_sm()
                .bg(if conflicts.is_empty() { palette.diff_header } else { palette.diff_deleted })
                .border_b_1()
                .border_color(palette.border)
                .child(Icon::new(IconName::GitMergeConflict).small())
                .child(div().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(label))
                .child(div().text_color(palette.text_secondary).child(match conflicts.len() {
                    0 => "All conflicts resolved".to_owned(),
                    1 => "1 file with conflicts".to_owned(),
                    n => format!("{n} files with conflicts"),
                }))
                .child(div().flex_1())
                .when(!conflicts.is_empty(), |el| {
                    el.child(Button::new("op-resolve").small().primary().label("Resolve…").on_click(move |_, window, cx| {
                        entity.update(cx, |this, cx| this.show_conflicts(window, cx))
                    }))
                })
                .child(
                    Button::new("op-continue")
                        .small()
                        .outline()
                        .label("Continue")
                        .disabled(!conflicts.is_empty())
                        .on_click(step(OperationStep::Continue)),
                )
                .when(state != RepositoryState::Merging, |el| {
                    el.child(Button::new("op-skip").small().outline().label("Skip").on_click(step(OperationStep::Skip)))
                })
                .child(Button::new("op-abort").small().outline().label("Abort").on_click(step(OperationStep::Abort))),
        )
    }

    pub fn annotate(&mut self, path: String, revision: Option<String>, cx: &mut Context<Self>) {
        let Some(repository) = self.model.read(cx).repository().cloned() else { return };
        if self.blame.as_ref().is_some_and(|(b, _)| b.read(cx).path() == path && b.read(cx).revision() == revision.as_deref()) {
            return;
        }
        let model = self.model.clone();
        let view = cx.new(|cx| BlameView::new(model, repository, path, revision, cx));
        let subscription = cx.subscribe(&view, |this, _, event: &BlameEvent, cx| match event {
            BlameEvent::SelectCommit(hash) => {
                this.show_git = true;
                this.bottom_tab = BottomTab::Log;
                let hash = hash.clone();
                this.model.update(cx, |m, cx| m.select_hash(Some(hash), cx));
                cx.notify();
            }
            BlameEvent::ShowDiff(source) => this.open_diff(source.clone(), cx),
            BlameEvent::Closed => {
                this.blame = None;
                cx.notify();
            }
        });
        self.blame = Some((view, subscription));
        cx.notify();
    }

    /// Show History: a Log tab for one file, following renames, as IntelliJ's "History: name" tab.
    /// Opens a file in the editor: the working tree (`None`) or a revision, read-only.
    pub fn open_file(&mut self, path: String, revision: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repository) = self.model.read(cx).repository().cloned() else { return };
        self.blame = None;
        let same = self.editor.as_ref().is_some_and(|(e, _)| e.read(cx).path() == path && e.read(cx).revision() == revision.as_deref());
        if !same {
            let view = cx.new(|cx| FileEditor::new(repository, path, revision, window, cx));
            let subscription = cx.subscribe(&view, Self::on_file_editor_event);
            self.editor = Some((view, subscription));
        }
        cx.notify();
    }

    fn on_file_editor_event(&mut self, _: Entity<FileEditor>, event: &FileEditorEvent, cx: &mut Context<Self>) {
        match event {
            FileEditorEvent::Closed => {
                self.editor = None;
                cx.notify();
            }
            FileEditorEvent::Annotate { path, revision } => self.annotate(path.clone(), revision.clone(), cx),
            FileEditorEvent::ShowHistory(path) => self.show_history(path.clone(), cx),
            FileEditorEvent::SelectionHistory { path, lines } => {
                let name = path.rsplit('/').next().unwrap_or(path);
                let filter = crate::git::LogFilter {
                    paths: vec![path.clone()],
                    lines: Some(*lines),
                    branches: vec!["HEAD".into()],
                    ..Default::default()
                };
                self.open_log_tab(format!("History for Selection: {name}:{}-{}", lines.0, lines.1), filter, cx);
            }
            FileEditorEvent::SelectCommit(hash) => {
                self.show_git = true;
                self.bottom_tab = BottomTab::Log;
                let hash = hash.clone();
                self.model.update(cx, |m, cx| m.select_hash(Some(hash), cx));
                cx.notify();
            }
            FileEditorEvent::OpenDiff(source) => self.open_diff(source.clone(), cx),
            FileEditorEvent::FilesChanged => self.model.update(cx, |m, cx| m.reload(cx)),
        }
    }

    pub fn show_history(&mut self, path: String, cx: &mut Context<Self>) {
        let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
        let filter = crate::git::LogFilter { paths: vec![path], ..Default::default() };
        self.open_log_tab(format!("History: {name}"), filter, cx);
    }

    /// Opens another Log tab with its own filters. Tabs share the repository;
    /// switching tabs swaps the Log's filters.
    pub fn open_log_tab(&mut self, title: String, filter: crate::git::LogFilter, cx: &mut Context<Self>) {
        self.save_log_tab(cx);
        self.log_tabs.push(LogTab { title, filter: filter.clone(), selected: None });
        self.active_log = self.log_tabs.len() - 1;
        self.show_git = true;
        self.bottom_tab = BottomTab::Log;
        self.model.update(cx, |m, cx| m.set_filter(filter, cx));
        cx.notify();
    }

    fn save_log_tab(&mut self, cx: &mut Context<Self>) {
        let current = self.model.read(cx).filter().clone();
        let selected = self.model.read(cx).selected_hash().map(str::to_owned);
        if let Some(tab) = self.log_tabs.get_mut(self.active_log) {
            tab.filter = current;
            tab.selected = selected;
        }
    }

    fn switch_log_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.bottom_tab == BottomTab::Log && ix == self.active_log {
            return;
        }
        self.save_log_tab(cx);
        self.active_log = ix.min(self.log_tabs.len() - 1);
        self.bottom_tab = BottomTab::Log;
        let filter = self.log_tabs[self.active_log].filter.clone();
        let selected = self.log_tabs[self.active_log].selected.clone();
        self.model.update(cx, |m, cx| {
            m.set_filter(filter, cx);
            m.select_hash(selected, cx);
        });
        cx.notify();
    }

    fn close_log_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix == 0 || ix >= self.log_tabs.len() {
            return;
        }
        let was_active = ix == self.active_log;
        self.log_tabs.remove(ix);
        if was_active {
            self.active_log = 0;
            let filter = self.log_tabs[0].filter.clone();
            self.model.update(cx, |m, cx| m.set_filter(filter, cx));
        } else if ix < self.active_log {
            self.active_log -= 1;
        }
        cx.notify();
    }

    fn open_diff(&mut self, source: crate::ui::diff_view::DiffSource, cx: &mut Context<Self>) {
        let Some(repository) = self.model.read(cx).repository().cloned() else { return };
        // A diff replaces the annotations and the file editor in the editor area.
        self.blame = None;
        self.editor = None;
        self.diff.update(cx, |diff, cx| diff.show(repository, source, cx));
    }

    fn open_repository(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open Git Repository".into()),
        });
        let model = self.model.clone();
        cx.spawn(async move |_, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                if let Some(path) = paths.into_iter().next() {
                    model.update(cx, |model, cx| model.open(path, cx));
                }
            }
        })
        .detach();
    }

    /// The Welcome screen shown when no repository is open.
    fn render_welcome(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let recent = crate::settings::recent_projects();
        let error = self.model.read(cx).error().map(str::to_owned);
        let action = |id: &'static str, icon: IconName, label: &'static str| {
            Button::new(id).small().icon(Icon::new(icon)).label(label)
        };
        let mut list = v_flex().gap_px();
        for (ix, path) in recent.iter().enumerate() {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let open = path.clone();
            let forget = path.clone();
            let model = self.model.clone();
            list = list.child(
                h_flex()
                    .id(("recent", ix))
                    .px_2()
                    .py_1()
                    .gap_2()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|s| s.bg(palette.hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let open = open.clone();
                        this.model.update(cx, |m, cx| m.open(open, cx))
                    }))
                    .context_menu(move |menu, _, _| {
                        let forget = forget.clone();
                        let model = model.clone();
                        menu.item(PopupMenuItem::new("Remove from Recent Projects").on_click(move |_, window, cx| {
                            crate::settings::forget_project(&forget);
                            model.update(cx, |_, cx| cx.notify());
                            window.refresh();
                        }))
                    })
                    .child(
                        div()
                            .size(px(28.))
                            .rounded(px(6.))
                            .bg(palette.graph[ix % palette.graph.len()])
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(gpui_kit::white())
                            .child(name.chars().take(2).collect::<String>().to_uppercase()),
                    )
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(name))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(palette.text_secondary)
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(path.display().to_string()),
                            ),
                    ),
            );
        }
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_4()
            .child(div().text_xl().font_weight(FontWeight::BOLD).child("Welcome to GitGlass"))
            .child(
                h_flex()
                    .gap_2()
                    .child(action("welcome-open", IconName::FolderOpen, "Open").on_click(cx.listener(
                        |this, _, window, cx| this.open_repository(window, cx),
                    )))
                    .child(action("welcome-clone", IconName::ArrowDownToLine, "Get from VCS").on_click(cx.listener(
                        |this, _, window, cx| clone_dialog::clone(this.model.clone(), window, cx),
                    )))
                    .child(action("welcome-init", IconName::Plus, "New Repository").on_click(cx.listener(
                        |this, _, _, cx| clone_dialog::init(this.model.clone(), cx),
                    ))),
            )
            .when_some(error, |el, error| {
                el.child(div().max_w(px(560.)).text_xs().text_color(palette.text_secondary).child(error))
            })
            .when(!recent.is_empty(), |el| {
                el.child(
                    v_flex()
                        .w(px(460.))
                        .gap_1()
                        .child(div().text_xs().text_color(palette.text_secondary).child("Recent Projects"))
                        .child(list),
                )
            })
    }

    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let model = self.model.read(cx);
        let project = model.repository().map(|r| r.name()).unwrap_or_else(|| "No Project".into());
        let branch = branches_popup::branch_widget_label(model);
        // Incoming / outgoing commits of the current branch, next to its name.
        let track = model
            .refs()
            .current_branch
            .as_ref()
            .and_then(|name| model.refs().find(&format!("refs/heads/{name}")))
            .map(|r| (r.behind, r.ahead))
            .filter(|&(behind, ahead)| behind > 0 || ahead > 0);
        let busy = model.busy().map(str::to_owned);
        let entity = cx.entity();
        let popup = self.branches_popup.clone();
        let operation_model = self.model.clone();
        let op = move |title: &'static str, args: &'static [&'static str], done: &'static str| {
            let model = operation_model.clone();
            move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut gpui_kit::App| {
                model.update(cx, |model, cx| {
                    model.run_operation(title, move |repo| {
                        repo.run(args)?;
                        Ok(done.to_owned())
                    }, cx)
                });
            }
        };

        TitleBar::new().child(
            h_flex()
                .w_full()
                .pr_2()
                .gap_1()
                .child(
                    Button::new("main-menu")
                        .ghost()
                        .small()
                        .icon(Icon::new(IconName::Menu))
                        .dropdown_menu({
                            let entity = entity.clone();
                            move |menu, _, cx| {
                                let dark = cx.palette().dark;
                                let open = entity.clone();
                                menu.item(PopupMenuItem::new("Open Repository…").on_click(move |_, window, cx| {
                                    open.update(cx, |this, cx| this.open_repository(window, cx))
                                }))
                                .item(PopupMenuItem::new("Get from Version Control…").on_click({
                                    let entity = entity.clone();
                                    move |_, window, cx| clone_dialog::clone(entity.read(cx).model.clone(), window, cx)
                                }))
                                .item(PopupMenuItem::new("Create Git Repository…").on_click({
                                    let entity = entity.clone();
                                    move |_, _, cx| clone_dialog::init(entity.read(cx).model.clone(), cx)
                                }))
                                .separator()
                                .item(PopupMenuItem::new("Merge…").on_click({
                                    let entity = entity.clone();
                                    move |_, window, cx| dialogs::merge(entity.read(cx).model.clone(), window, cx)
                                }))
                                .item(PopupMenuItem::new("Rebase…").on_click({
                                    let entity = entity.clone();
                                    move |_, window, cx| dialogs::rebase(entity.read(cx).model.clone(), window, cx)
                                }))
                                .separator()
                                .item(PopupMenuItem::new("Pull…").on_click({
                                    let entity = entity.clone();
                                    move |_, window, cx| remote_dialogs::pull(entity.read(cx).model.clone(), window, cx)
                                }))
                                .item(PopupMenuItem::new("Manage Remotes…").on_click({
                                    let entity = entity.clone();
                                    move |_, window, cx| remote_dialogs::manage_remotes(entity.read(cx).model.clone(), window, cx)
                                }))
                                .separator()
                                .item(PopupMenuItem::new("Create Patch…").on_click({
                                    let entity = entity.clone();
                                    move |_, window, cx| {
                                        let commit = entity.read(cx).commit.clone();
                                        commit.update(cx, |c, cx| c.create_patch(window, cx))
                                    }
                                }))
                                .item(PopupMenuItem::new("Apply Patch…").on_click({
                                    let entity = entity.clone();
                                    move |_, window, cx| patch_dialogs::apply_patch(entity.read(cx).model.clone(), false, window, cx)
                                }))
                                .item(PopupMenuItem::new("Apply Patch from Clipboard…").on_click({
                                    let entity = entity.clone();
                                    move |_, window, cx| patch_dialogs::apply_patch(entity.read(cx).model.clone(), true, window, cx)
                                }))
                                .separator()
                                .item(PopupMenuItem::new("Settings…").on_click(|_, window, cx| dialogs::settings(window, cx)))
                                .separator()
                                .item(PopupMenuItem::new("Light Theme").checked(!dark).on_click(|_, window, cx| {
                                    theme::apply(false, cx);
                                    Settings::update(cx, |s| s.dark = false);
                                    window.refresh();
                                }))
                                .item(PopupMenuItem::new("Dark Theme").checked(dark).on_click(|_, window, cx| {
                                    theme::apply(true, cx);
                                    Settings::update(cx, |s| s.dark = true);
                                    window.refresh();
                                }))
                                .separator()
                                .item(PopupMenuItem::new("Exit").on_click(|_, _, cx| cx.quit()))
                            }
                        }),
                )
                .child(
                    Button::new("project-widget")
                        .ghost()
                        .small()
                        .child(
                            h_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .size(px(18.))
                                        .rounded(px(4.))
                                        .bg(palette.accent)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .text_xs()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(gpui_kit::white())
                                        .child(project.chars().take(2).collect::<String>().to_uppercase()),
                                )
                                .child(div().font_weight(FontWeight::SEMIBOLD).child(project))
                                .child(Icon::new(IconName::ChevronDown).xsmall()),
                        )
                        .dropdown_menu({
                            let entity = entity.clone();
                            move |menu, _, cx| {
                                let current = entity.read(cx).model.read(cx).repository().map(|r| r.root().to_path_buf());
                                let mut menu = menu.label("Recent Projects");
                                for path in crate::settings::recent_projects().into_iter().take(10) {
                                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                                    let entity = entity.clone();
                                    let is_current = current.as_deref() == Some(path.as_path());
                                    menu = menu.item(PopupMenuItem::new(name).checked(is_current).on_click(move |_, _, cx| {
                                        let path = path.clone();
                                        let model = entity.read(cx).model.clone();
                                        model.update(cx, |m, cx| m.open(path, cx));
                                    }));
                                }
                                let (open, clone, init) = (entity.clone(), entity.clone(), entity.clone());
                                menu.separator()
                                    .item(PopupMenuItem::new("Open…").on_click(move |_, window, cx| {
                                        open.update(cx, |this, cx| this.open_repository(window, cx))
                                    }))
                                    .item(PopupMenuItem::new("Get from Version Control…").on_click(move |_, window, cx| {
                                        clone_dialog::clone(clone.read(cx).model.clone(), window, cx)
                                    }))
                                    .item(PopupMenuItem::new("Create Git Repository…").on_click(move |_, _, cx| {
                                        clone_dialog::init(init.read(cx).model.clone(), cx)
                                    }))
                            }
                        }),
                )
                .child(
                    Popover::new("branches-popover")
                        .anchor(gpui_kit::Anchor::TopLeft)
                        .open(self.branches_open)
                        .on_open_change({
                            let entity = entity.clone();
                            move |open, window, cx| {
                                let open = *open;
                                entity.update(cx, |this, cx| {
                                    if open {
                                        this.open_branches(window, cx);
                                    } else {
                                        this.branches_open = false;
                                        cx.notify();
                                    }
                                })
                            }
                        })
                        .trigger(
                            Button::new("vcs-widget")
                                .ghost()
                                .small()
                                .icon(Icon::new(IconName::GitBranch).small())
                                .label(branch)
                                .when_some(track, |el, (behind, ahead)| {
                                    el.child(
                                        h_flex()
                                            .gap_1()
                                            .text_xs()
                                            .when(behind > 0, |el| el.child(div().text_color(palette.link).child(format!("↓{behind}"))))
                                            .when(ahead > 0, |el| el.child(div().text_color(palette.status_added).child(format!("↑{ahead}")))),
                                    )
                                })
                                .child(Icon::new(IconName::ChevronDown).xsmall()),
                        )
                        .child(popup),
                )
                .child(div().flex_1())
                .when_some(busy, |el, busy| {
                    el.child(div().text_xs().text_color(palette.text_secondary).child(format!("{busy}…")))
                })
                .child(tool_button("tb-update", IconName::ArrowDownToLine, "Update Project…  Ctrl+T").on_click(cx.listener(
                    |this, _, window, cx| dialogs::update_project(this.model.clone(), window, cx),
                )))
                .child(tool_button("tb-commit", IconName::Check, "Commit…  Ctrl+K").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.show_commit = true;
                        cx.notify();
                    },
                )))
                .child(tool_button("tb-push", IconName::ArrowUpFromLine, "Push…  Ctrl+Shift+K").on_click(cx.listener(
                    |this, _, window, cx| dialogs::push(this.model.clone(), window, cx),
                )))
                .child(tool_button("tb-fetch", IconName::CloudDownload, "Fetch").on_click(op(
                    "Fetch",
                    &["fetch", "--all", "--prune"],
                    "Fetched all remotes",
                ))),
        )
    }

    fn render_stripe(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let stripe_button = |id: &'static str, icon: IconName, tooltip: &'static str, active: bool| {
            Button::new(id)
                .ghost()
                .icon(Icon::new(icon))
                .tooltip(tooltip)
                .when(active, |b| b.selected(true))
        };
        v_flex()
            .w(px(40.))
            .h_full()
            .py_1()
            .gap_1()
            .items_center()
            .border_r_1()
            .border_color(palette.border)
            .bg(palette.toolbar)
            .child(stripe_button("stripe-commit", IconName::GitCommitVertical, "Commit", self.show_commit).on_click(
                cx.listener(|this, _, _, cx| {
                    this.show_commit = !this.show_commit;
                    cx.notify();
                }),
            ))
            .child(div().flex_1())
            .child(stripe_button("stripe-git", IconName::GitGraph, "Git", self.show_git).on_click(cx.listener(
                |this, _, _, cx| {
                    this.show_git = !this.show_git;
                    cx.notify();
                },
            )))
    }

    fn render_left(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let current = self.left_tab;
        let tab = |id: &'static str, label: &'static str, value: LeftTab| {
            div()
                .id(id)
                .px_2()
                .h_full()
                .flex()
                .items_center()
                .text_sm()
                .cursor_pointer()
                .when(value == current, |el| el.border_b_2().border_color(palette.accent).text_color(palette.text))
                .when(value != current, |el| el.text_color(palette.text_secondary))
                .child(label)
        };
        v_flex()
            .size_full()
            .bg(palette.panel)
            .child(
                h_flex()
                    .h(px(30.))
                    .px_2()
                    .gap_1()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(tab("left-commit", "Commit", LeftTab::Commit).on_click(cx.listener(|this, _, _, cx| {
                        this.left_tab = LeftTab::Commit;
                        cx.notify();
                    })))
                    .child(tab("left-stash", "Stash", LeftTab::Stash).on_click(cx.listener(|this, _, _, cx| {
                        this.left_tab = LeftTab::Stash;
                        cx.notify();
                    })))
                    .child(tab("left-shelf", "Shelf", LeftTab::Shelf).on_click(cx.listener(|this, _, _, cx| {
                        this.left_tab = LeftTab::Shelf;
                        cx.notify();
                    })))
                    .child(div().flex_1())
                    .child(tool_button("commit-hide", IconName::Minus, "Hide").on_click(cx.listener(|this, _, _, cx| {
                        this.show_commit = false;
                        cx.notify();
                    }))),
            )
            .child(div().flex_1().min_h_0().map(|el| match current {
                LeftTab::Commit => el.child(self.commit.clone()),
                LeftTab::Stash => el.child(self.stash.clone()),
                LeftTab::Shelf => el.child(self.shelf.clone()),
            }))
    }

    fn on_commit(&mut self, _: &CommitChanges, window: &mut Window, cx: &mut Context<Self>) {
        self.show_commit = true;
        self.left_tab = LeftTab::Commit;
        self.commit.update(cx, |commit, cx| commit.focus_message(window, cx));
        cx.notify();
    }

    fn on_push(&mut self, _: &PushChanges, window: &mut Window, cx: &mut Context<Self>) {
        dialogs::push(self.model.clone(), window, cx);
    }

    fn on_update(&mut self, _: &UpdateProject, window: &mut Window, cx: &mut Context<Self>) {
        dialogs::update_project(self.model.clone(), window, cx);
    }

    fn on_show_branches(&mut self, _: &ShowBranches, window: &mut Window, cx: &mut Context<Self>) {
        self.open_branches(window, cx);
    }

    /// Opens the branches popup with its search field focused, so typing
    /// filters right away (IntelliJ's speed search).
    fn open_branches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.branches_open = true;
        self.branches_popup.update(cx, |popup, cx| popup.reset_search(window, cx));
        let popup = self.branches_popup.clone();
        window.defer(cx, move |window, cx| popup.update(cx, |popup, cx| popup.focus_search(window, cx)));
        cx.notify();
    }

    fn on_toggle_git(&mut self, _: &ToggleGitWindow, _: &mut Window, cx: &mut Context<Self>) {
        self.show_git = !self.show_git;
        cx.notify();
    }

    fn on_refresh(&mut self, _: &Refresh, _: &mut Window, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.reload(cx));
    }

    fn on_stash(&mut self, _: &StashChanges, window: &mut Window, cx: &mut Context<Self>) {
        dialogs::stash(self.model.clone(), window, cx);
    }

    /// IntelliJ's VCS Operations quick list, numbered like the original.
    fn on_vcs_operations(&mut self, _: &VcsOperations, window: &mut Window, cx: &mut Context<Self>) {
        type Run = Rc<dyn Fn(&mut Workspace, &mut Window, &mut Context<Workspace>)>;
        let op = |f: fn(&mut Workspace, &mut Window, &mut Context<Workspace>)| -> Run { Rc::new(f) };
        let items: Vec<Option<(&'static str, &'static str, Run)>> = vec![
            Some(("Commit…", "Ctrl+K", op(|this, window, cx| this.on_commit(&CommitChanges, window, cx)))),
            Some(("Push…", "Ctrl+Shift+K", op(|this, window, cx| dialogs::push(this.model.clone(), window, cx)))),
            Some(("Update Project…", "Ctrl+T", op(|this, window, cx| dialogs::update_project(this.model.clone(), window, cx)))),
            Some(("Pull…", "", op(|this, window, cx| remote_dialogs::pull(this.model.clone(), window, cx)))),
            Some(("Fetch", "", op(|this, _, cx| {
                this.model.update(cx, |m, cx| m.run_operation("Fetch", |repo| {
                    repo.run(["fetch", "--all", "--prune"])?;
                    Ok("Fetched all remotes".into())
                }, cx))
            }))),
            None,
            Some(("Branches…", "Ctrl+Shift+`", op(|this, window, cx| this.open_branches(window, cx)))),
            Some(("New Branch…", "", op(|this, window, cx| dialogs::new_branch(this.model.clone(), "HEAD".into(), window, cx)))),
            Some(("New Tag…", "", op(|this, window, cx| dialogs::new_tag(this.model.clone(), "HEAD".into(), window, cx)))),
            Some(("Merge…", "", op(|this, window, cx| dialogs::merge(this.model.clone(), window, cx)))),
            Some(("Rebase…", "", op(|this, window, cx| dialogs::rebase(this.model.clone(), window, cx)))),
            Some(("Manage Remotes…", "", op(|this, window, cx| remote_dialogs::manage_remotes(this.model.clone(), window, cx)))),
            Some(("Reset HEAD…", "", op(|this, window, cx| dialogs::reset_to(this.model.clone(), "HEAD".into(), window, cx)))),
            None,
            Some(("Stash Changes…", "", op(|this, window, cx| dialogs::stash(this.model.clone(), window, cx)))),
            Some(("Unstash Changes…", "", op(|this, _, cx| {
                this.show_commit = true;
                this.left_tab = LeftTab::Stash;
                cx.notify();
            }))),
            Some(("Shelve Changes…", "", op(|this, window, cx| this.commit.update(cx, |c, cx| c.shelve(window, cx))))),
            Some(("Unshelve Changes…", "", op(|this, _, cx| {
                this.show_commit = true;
                this.left_tab = LeftTab::Shelf;
                cx.notify();
            }))),
            None,
            Some(("Create Patch…", "", op(|this, window, cx| this.commit.update(cx, |c, cx| c.create_patch(window, cx))))),
            Some(("Apply Patch…", "", op(|this, window, cx| patch_dialogs::apply_patch(this.model.clone(), false, window, cx)))),
            Some(("Apply Patch from Clipboard…", "", op(|this, window, cx| patch_dialogs::apply_patch(this.model.clone(), true, window, cx)))),
            None,
            Some(("Show Git Log", "Alt+9", op(|this, _, cx| {
                this.show_git = true;
                this.bottom_tab = BottomTab::Log;
                cx.notify();
            }))),
            Some(("Settings…", "Ctrl+Alt+S", op(|_, window, cx| dialogs::settings(window, cx)))),
        ];
        let workspace = cx.entity();
        window.open_dialog(cx, move |dialog, _, cx| {
            let palette = cx.palette().clone();
            let mut list = v_flex().gap_px();
            let mut number = 0;
            for (ix, item) in items.iter().enumerate() {
                let Some((label, shortcut, run)) = item else {
                    list = list.child(div().my_1().h(px(1.)).bg(palette.border));
                    continue;
                };
                number += 1;
                let run = run.clone();
                let workspace = workspace.clone();
                list = list.child(
                    h_flex()
                        .id(("vcs-op", ix))
                        .h(px(26.))
                        .px_2()
                        .gap_2()
                        .rounded(px(4.))
                        .text_sm()
                        .cursor_pointer()
                        .hover(|s| s.bg(palette.hover))
                        .on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            let run = run.clone();
                            workspace.update(cx, |this, cx| run(this, window, cx));
                        })
                        .child(div().w(px(16.)).text_color(palette.text_secondary).child(if number < 10 { number.to_string() } else { String::new() }))
                        .child(div().flex_1().child(*label))
                        .child(div().text_xs().text_color(palette.text_secondary).child(*shortcut)),
                );
            }
            dialog.title("VCS Operations").w(px(340.)).child(list)
        });
    }

    fn render_bottom(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let tab = |id: &'static str, _label: &'static str, value: BottomTab, current: BottomTab| {
            div()
                .id(id)
                .px_2()
                .h_full()
                .flex()
                .items_center()
                .text_sm()
                .cursor_pointer()
                .when(value == current, |el| el.border_b_2().border_color(palette.accent).text_color(palette.text))
                .when(value != current, |el| el.text_color(palette.text_secondary))
        };
        let current = self.bottom_tab;
        v_flex()
            .size_full()
            .bg(palette.panel)
            .child(
                h_flex()
                    .h(px(30.))
                    .px_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Git"))
                    .children(self.log_tabs.iter().enumerate().map(|(ix, log_tab)| {
                        let active = current == BottomTab::Log && ix == self.active_log;
                        div()
                            .id(("tab-log", ix))
                            .px_2()
                            .h_full()
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_sm()
                            .cursor_pointer()
                            .when(active, |el| el.border_b_2().border_color(palette.accent).text_color(palette.text))
                            .when(!active, |el| el.text_color(palette.text_secondary))
                            .on_click(cx.listener(move |this, _, _, cx| this.switch_log_tab(ix, cx)))
                            .child(log_tab.title.clone())
                            .when(ix > 0, |el| {
                                el.child(
                                    tool_button(("tab-log-close", ix), IconName::X, "Close Tab")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.close_log_tab(ix, cx)
                                        })),
                                )
                            })
                    }))
                    .child(tool_button("tab-log-new", IconName::Plus, "New Log Tab").on_click(cx.listener(|this, _, _, cx| {
                        let title = format!("Log {}", this.log_tabs.len() + 1);
                        this.open_log_tab(title, Default::default(), cx);
                    })))
                    .child(tab("tab-console", "Console", BottomTab::Console, current).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.bottom_tab = BottomTab::Console;
                            cx.notify();
                        },
                    )).child("Console"))
                    .child(div().flex_1())
                    .child(tool_button("git-hide", IconName::Minus, "Hide").on_click(cx.listener(|this, _, _, cx| {
                        this.show_git = false;
                        cx.notify();
                    }))),
            )
            .child(div().flex_1().min_h_0().map(|el| match current {
                BottomTab::Log => el.child(self.log.clone()),
                BottomTab::Console => el.child(self.render_console(cx)),
            }))
    }

    fn render_console(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let entries = self.model.read(cx).console().entries();
        let mono = cx.theme().mono_font_family.clone();
        let mut list = v_flex().p_2().gap_1().font_family(mono).text_size(px(12.));
        for entry in entries.iter().rev().take(300).rev() {
            list = list.child(
                v_flex()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().text_color(if entry.success { palette.link } else { palette.status_conflict }).child(entry.command_line.clone()))
                            .child(div().text_color(palette.text_disabled).child(format!("{} ms", entry.duration.as_millis()))),
                    )
                    .when(!entry.output.is_empty(), |el| {
                        el.child(div().pl_4().text_color(palette.text_secondary).whitespace_normal().child(entry.output.clone()))
                    }),
            );
        }
        div().id("git-console").size_full().overflow_y_scrollbar().child(list)
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let model = self.model.read(cx);
        let project = model.repository().map(|r| r.root().display().to_string()).unwrap_or_default();
        let branch = branches_popup::branch_widget_label(model);
        let changes = model.status().entries.len();
        h_flex()
            .h(px(24.))
            .px_3()
            .gap_3()
            .border_t_1()
            .border_color(palette.border)
            .bg(palette.toolbar)
            .text_xs()
            .text_color(palette.text_secondary)
            .child(project)
            .child(div().flex_1())
            .when(model.is_loading(), |el| el.child("Refreshing VCS history…"))
            .child(format!("{changes} changed"))
            .child(h_flex().gap_1().child(Icon::new(IconName::GitBranch).xsmall()).child(branch))
            .child("UTF-8")
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let error = self.model.read(cx).error().map(str::to_owned);
        let has_repo = self.model.read(cx).repository().is_some();

        let editor = v_flex()
            .size_full()
            .bg(palette.panel)
            .when_some(error.filter(|_| has_repo), |el, error| {
                el.child(div().p_2().text_sm().text_color(palette.status_conflict).child(error))
            })
            .children(self.render_operation_banner(cx))
            .child(div().flex_1().min_h_0().map(|el| match (&self.merge, &self.blame, &self.editor) {
                _ if !has_repo => el.child(self.render_welcome(cx)),
                (Some((merge, _)), _, _) => el.child(merge.clone()),
                (None, Some((blame, _)), _) => el.child(blame.clone()),
                (None, None, Some((editor, _))) => el.child(editor.clone()),
                (None, None, None) => el.child(self.diff.clone()),
            }));

        let top = h_resizable("top-split")
            .child(
                resizable_panel()
                    .size(px(340.))
                    .size_range(px(220.)..px(700.))
                    .visible(self.show_commit && has_repo)
                    .child(self.render_left(cx)),
            )
            .child(resizable_panel().child(editor));

        let main = v_resizable("main-split")
            .child(resizable_panel().child(top))
            .child(
                resizable_panel()
                    .size(px(380.))
                    .size_range(px(120.)..px(1200.))
                    .visible(self.show_git && has_repo)
                    .child(self.render_bottom(cx)),
            );

        v_flex()
            .id("workspace")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_commit))
            .on_action(cx.listener(Self::on_push))
            .on_action(cx.listener(Self::on_update))
            .on_action(cx.listener(Self::on_show_branches))
            .on_action(cx.listener(Self::on_toggle_git))
            .on_action(cx.listener(Self::on_refresh))
            .on_action(cx.listener(Self::on_stash))
            .on_action(cx.listener(Self::on_vcs_operations))
            .on_action(cx.listener(|_, _: &OpenSettings, window, cx| dialogs::settings(window, cx)))
            .on_action(cx.listener(|this, _: &NextDifference, _, cx| this.diff.update(cx, |d, cx| d.next_difference(cx))))
            .on_action(cx.listener(|this, _: &PreviousDifference, _, cx| this.diff.update(cx, |d, cx| d.previous_difference(cx))))
            .size_full()
            .bg(palette.window)
            .text_color(palette.text)
            .text_size(px(13.))
            .child(self.render_title_bar(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_stripe(cx))
                    .child(div().flex_1().h_full().child(main)),
            )
            .child(self.render_status_bar(cx))
    }
}
