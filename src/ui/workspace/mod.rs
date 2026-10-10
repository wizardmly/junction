//! The main window, laid out like IntelliJ's new UI: title bar with project
//! and VCS widgets, a left tool window stripe, the Commit tool window, the
//! editor area (diff), the bottom Git tool window (Log / Console), and the
//! status bar.

use std::collections::HashMap;
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
    v_flex, v_resizable, h_resizable,
};
use gpui_kit::assets::IconName;
use gpui_kit::{
    App, AppContext as _, Context, Entity, FontWeight, SharedString, InteractiveElement as _, IntoElement, KeyBinding,
    ParentElement as _, PathPromptOptions, Render, actions, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _, px,
};

use crate::model::{OpenProblem, RepoEvent, RepoModel};
use crate::theme::{self, ActivePalette as _};
use crate::ui::branches_popup::{self, BranchesPopup};
use crate::ui::commit_view::{CommitEvent, CommitView};
use crate::ui::common::tool_button;
use crate::ui::diff_view::{CompareNextFile, ComparePreviousFile, DiffView, JumpToSource, NextDifference, PreviousDifference};
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
use crate::index::service::{CodeIndex, IndexEvent};
use crate::ui::search_everywhere::SeTab;
use crate::ui::tool_windows::{DraggedToolWindow, Side, ToolWindow, ToolWindows};

actions!(
    workspace,
    [
        CommitChanges,
        PushChanges,
        UpdateProject,
        ShowBranches,
        ToggleGitWindow,
        Refresh,
        StashChanges,
        OpenSettings,
        VcsOperations,
        GotoFile,
        GotoClass,
        GotoSymbol,
        NavigateBack,
        NavigateForward,
        ToggleProjectWindow,
        ToggleFindWindow,
        SelectInProject,
        FindInPath,
        ReplaceInPath,
        SearchEverywhere,
        FindAction,
        RecentFiles,
        FileStructure,
        GotoLine,
        CloseTab,
        NextTab,
        PreviousTab,
        ReopenClosedTab,
        ToggleCommitWindow,
        HideAllToolWindows,
        HideActiveToolWindow,
        NextOccurrence,
        PreviousOccurrence
    ]
);

mod tabs;
mod history;
mod chrome;
mod files;
mod menu;
mod navigation;
mod panels;
mod search;
mod vcs;
use tabs::{EditorTab, Front};

const CONTEXT: &str = "Workspace";
/// The name people see; the binary, settings folder and repository stay "junction".
pub const APP_NAME: &str = "Junction Studio";
/// The editor area while it shows a file: Alt+Left / Right switch tabs there.
const TABS_CONTEXT: &str = "EditorTabs";

/// IntelliJ's default keymap for the VCS actions.
pub fn init(cx: &mut gpui_kit::App) {
    cx.bind_keys([
        KeyBinding::new("secondary-k", CommitChanges, Some(CONTEXT)),
        KeyBinding::new("secondary-shift-k", PushChanges, Some(CONTEXT)),
        KeyBinding::new("secondary-t", UpdateProject, Some(CONTEXT)),
        KeyBinding::new("secondary-shift-`", ShowBranches, Some(CONTEXT)),
        // With Shift held, X11 and Wayland report the shifted key ("~"), so
        // Ctrl+Shift+` arrives as Ctrl+~ (with or without the Shift flag).
        KeyBinding::new("secondary-shift-~", ShowBranches, Some(CONTEXT)),
        KeyBinding::new("secondary-~", ShowBranches, Some(CONTEXT)),
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-9" } else { "alt-9" }, ToggleGitWindow, Some(CONTEXT)),
        KeyBinding::new("secondary-alt-y", Refresh, Some(CONTEXT)),
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-," } else { "ctrl-alt-s" }, OpenSettings, Some(CONTEXT)),
        // VCS Operations popup: Alt+` (Ctrl+V on macOS).
        #[cfg(target_os = "macos")]
        KeyBinding::new("ctrl-v", VcsOperations, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("alt-`", VcsOperations, Some(CONTEXT)),
        KeyBinding::new("f7", NextDifference, Some(CONTEXT)),
        KeyBinding::new("f4", JumpToSource, Some(CONTEXT)),
        KeyBinding::new("alt-right", CompareNextFile, Some(CONTEXT)),
        KeyBinding::new("alt-left", ComparePreviousFile, Some(CONTEXT)),
        KeyBinding::new("secondary-v", crate::ui::text_panes::Paste, Some(crate::ui::text_panes::PANE_CONTEXT)),
        KeyBinding::new("shift-insert", crate::ui::text_panes::Paste, Some(crate::ui::text_panes::PANE_CONTEXT)),
        KeyBinding::new("shift-f7", PreviousDifference, Some(CONTEXT)),
        // Navigation, IntelliJ's default keymap.
        KeyBinding::new("secondary-shift-n", GotoFile, Some(CONTEXT)),
        KeyBinding::new("secondary-n", GotoClass, Some(CONTEXT)),
        KeyBinding::new("secondary-alt-shift-n", GotoSymbol, Some(CONTEXT)),
        // Tool windows and navigation: Alt+digit and Ctrl+Alt+arrows on Windows / Linux,
        // ⌘digit and ⌘[ / ⌘] in IntelliJ's macOS keymap.
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-alt-left", NavigateBack, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-alt-right", NavigateForward, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-[", NavigateBack, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-]", NavigateForward, Some(CONTEXT)),
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-1" } else { "alt-1" }, ToggleProjectWindow, Some(CONTEXT)),
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-3" } else { "alt-3" }, ToggleFindWindow, Some(CONTEXT)),
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-0" } else { "alt-0" }, ToggleCommitWindow, Some(CONTEXT)),
        KeyBinding::new("secondary-shift-f12", HideAllToolWindows, Some(CONTEXT)),
        KeyBinding::new("shift-escape", HideActiveToolWindow, Some(CONTEXT)),
        KeyBinding::new("secondary-shift-f", FindInPath, Some(CONTEXT)),
        // The text field library binds ⇧⌘F to its own replace panel on macOS.
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-shift-f", FindInPath, Some("Input")),
        KeyBinding::new("secondary-shift-r", ReplaceInPath, Some(CONTEXT)),
        KeyBinding::new("shift shift", SearchEverywhere, Some(CONTEXT)),
        KeyBinding::new("secondary-shift-a", FindAction, Some(CONTEXT)),
        KeyBinding::new("secondary-e", RecentFiles, Some(CONTEXT)),
        KeyBinding::new("secondary-f12", FileStructure, Some(CONTEXT)),
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-l" } else { "ctrl-g" }, GotoLine, Some(CONTEXT)),
        KeyBinding::new("alt-f1", SelectInProject, Some(CONTEXT)),
        // Next / Previous Occurrence in the Find tool window, from anywhere
        // (the editor binds them over its add-cursor keys, see file_editor).
        KeyBinding::new("secondary-alt-down", NextOccurrence, Some(CONTEXT)),
        KeyBinding::new("secondary-alt-up", PreviousOccurrence, Some(CONTEXT)),
        // Editor tabs: Ctrl+F4 closes; Alt+Left / Alt+Right switch (Cmd+Shift+[ / ] on macOS).
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-w" } else { "ctrl-f4" }, CloseTab, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("alt-left", PreviousTab, Some(TABS_CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("alt-right", NextTab, Some(TABS_CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-shift-[", PreviousTab, Some(TABS_CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-shift-]", NextTab, Some(TABS_CONTEXT)),
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
    /// Non-modal commit off: the Commit tool window's tabs move here, as
    /// in IntelliJ's Git tool window (Local Changes, Shelf, Stash).
    LocalChanges,
    Shelf,
    Stash,
    Log,
    Worktrees,
    Submodules,
    /// Find Usages results.
    Find,
    Console,
}

struct LogTab {
    title: String,
    filter: crate::git::LogFilter,
    /// Each tab keeps its own selected commit.
    selected: Option<String>,
}

type VcsRun = Rc<dyn Fn(&mut Workspace, &mut Window, &mut Context<Workspace>)>;

pub struct Workspace {
    model: Entity<RepoModel>,
    log: Entity<LogView>,
    commit: Entity<CommitView>,
    /// The modal Commit Changes dialog is open (non-modal commit off).
    modal_commit: Rc<std::cell::Cell<bool>>,
    stash: Entity<StashView>,
    shelf: Entity<ShelfView>,
    diff: Entity<DiffView>,
    branches_popup: Entity<BranchesPopup>,
    worktrees: Entity<crate::ui::worktree_view::WorktreeView>,
    submodules: Entity<crate::ui::submodule_view::SubmoduleView>,
    /// The code index, and the tool windows built on it.
    code_index: Entity<CodeIndex>,
    find: Entity<crate::ui::find_view::FindView>,
    /// Find / Replace in Files, kept between uses like IntelliJ's.
    find_popup: Option<(Entity<crate::ui::find_popup::FindPopup>, Subscription)>,
    show_find_popup: bool,
    /// Search Everywhere (Shift twice), kept between uses.
    search_everywhere: Option<(Entity<crate::ui::search_everywhere::SearchEverywhere>, Subscription)>,
    show_search_everywhere: bool,
    /// Recently opened files, newest first, for Recent Files and the Scope tab.
    recent_files: history::RecentPaths,
    /// Files saved or replaced this session, newest first.
    recently_changed: history::RecentPaths,
    project: Entity<crate::ui::project_view::ProjectView>,
    /// Navigate › Back / Forward.
    nav: history::NavHistory,
    /// The Pull Requests tool window, sharing the left side with Commit.
    prs: Entity<crate::ui::pull_requests::PullRequestsView>,
    /// The Changes tool window (compare results); its stripe button shows while it has tabs.
    changes: Entity<crate::ui::changes_view::ChangesView>,
    /// A pull request's timeline, shown in the editor area until closed.
    timeline: Option<Entity<crate::ui::pull_requests::PrTimelineView>>,
    /// The merge tool, shown in the editor area instead of the diff.
    merge: Option<(Entity<MergeView>, Subscription)>,
    /// Git tool window Log tabs; the first is the main "Log".
    log_tabs: Vec<LogTab>,
    active_log: usize,
    /// Editor tabs, pinned first.
    editors: Vec<EditorTab>,
    /// What the editor area shows: a file tab, or the diff / merge / annotate / PR view.
    front: Front,
    tab_clock: u64,
    /// The second tab group after Split Right / Down, and which group is focused.
    split: Option<tabs::SplitGroup>,
    window_title: String,
    active_group: usize,
    /// Reopen Closed Tab, newest last.
    closed_tabs: Vec<String>,
    tools: ToolWindows,
    notifications: Vec<crate::ui::status_bar::NotificationRecord>,
    /// Git console toolbar state.
    console_wrap: bool,
    console_autoscroll: bool,
    console_scroll: gpui_kit::ScrollHandle,
    console_seen: std::cell::Cell<usize>,
    unread_notifications: usize,
    caret: Entity<crate::ui::status_bar::CaretStatus>,
    left_tab: LeftTab,
    bottom_tab: BottomTab,
    branches_open: bool,
    focus: gpui_kit::FocusHandle,
    /// Each tool window's content tracks focus, so Shift+Escape knows the
    /// active one.
    tool_focus: HashMap<ToolWindow, gpui_kit::FocusHandle>,
    /// The tool window activated last (opened or focused).
    last_tool: Option<ToolWindow>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(model: Entity<RepoModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let log = cx.new(|cx| LogView::new(model.clone(), window, cx));
        let commit = cx.new(|cx| CommitView::new(model.clone(), window, cx));
        let diff = cx.new(DiffView::new);
        let stash = cx.new(|cx| StashView::new(model.clone(), cx));
        let shelf = cx.new(|cx| ShelfView::new(model.clone(), cx));
        let branches_popup = cx.new(|cx| BranchesPopup::new(model.clone(), window, cx));
        let worktrees = cx.new(|cx| crate::ui::worktree_view::WorktreeView::new(model.clone(), cx));
        let submodules = cx.new(|cx| crate::ui::submodule_view::SubmoduleView::new(model.clone(), cx));
        let prs = cx.new(|cx| crate::ui::pull_requests::PullRequestsView::new(model.clone(), window, cx));
        let changes = cx.new(|cx| crate::ui::changes_view::ChangesView::new(model.clone(), cx));
        let code_index = cx.new(CodeIndex::new);
        let find = cx.new(|_| crate::ui::find_view::FindView::new(code_index.clone()));
        let project = cx.new(|cx| crate::ui::project_view::ProjectView::new(code_index.clone(), cx));
        let weak = cx.entity().downgrade();
        branches_popup.update(cx, |popup, _| {
            popup.on_commit = Some(Rc::new(move |window, cx| {
                weak.update(cx, |this, cx| this.on_commit(&CommitChanges, window, cx)).ok();
            }));
        });
        // Right-click menus in the Project and Commit tool windows act through the workspace.
        let file_actions: crate::ui::file_menus::FileActions = {
            let weak = cx.entity().downgrade();
            Rc::new(move |action, window, cx| {
                weak.update(cx, |this, cx| this.file_action(action, window, cx)).ok();
            })
        };
        project.update(cx, |p, cx| p.set_menu(model.clone(), file_actions.clone(), cx));
        commit.update(cx, |c, _| c.set_file_actions(file_actions));
        let mut subscriptions = vec![
            cx.subscribe_in(&log, window, |this, _, event: &LogEvent, window, cx| match event {
                LogEvent::OpenDiff(source) => this.open_diff(source.clone(), cx),
                LogEvent::Annotate { path, revision } => this.annotate(path.clone(), revision.clone(), window, cx),
                LogEvent::OpenFile { path, revision } => this.open_file(path.clone(), revision.clone(), window, cx),
            }),
            cx.subscribe_in(&changes, window, |this, _, event: &crate::ui::changes_view::ChangesEvent, _window, cx| {
                use crate::ui::changes_view::ChangesEvent;
                match event {
                    ChangesEvent::OpenDiff(source) => this.open_diff(source.clone(), cx),
                    ChangesEvent::Closed | ChangesEvent::Hide => {
                        this.tools.hide(ToolWindow::Changes);
                        cx.notify();
                    }
                }
            }),
            cx.subscribe_in(&commit, window, |this, commit, event: &CommitEvent, window, cx| match event {
                CommitEvent::OpenDiff(source) => {
                    let order = commit.read(cx).file_order().to_vec();
                    this.diff.update(cx, |diff, _| diff.set_file_order(order));
                    this.open_diff(source.clone(), cx)
                }
                CommitEvent::RefreshDiff => {
                    let order = commit.read(cx).file_order().to_vec();
                    this.diff.update(cx, |diff, cx| {
                        diff.set_file_order(order);
                        diff.refresh_local(cx)
                    })
                }
                CommitEvent::Committed => {
                    if this.modal_commit.replace(false) {
                        window.close_dialog(cx);
                        this.commit.update(cx, |commit, cx| commit.set_in_dialog(false, cx));
                    }
                }
                CommitEvent::OpenPush => dialogs::push_after_commit(this.model.clone(), window, cx),
                CommitEvent::OpenMerge(conflict) => this.open_merge(conflict.clone(), window, cx),
                CommitEvent::EditSource(path) => this.open_file(path.clone(), None, window, cx),
            }),
            cx.subscribe_in(&prs, window, |this, _, event: &crate::ui::pull_requests::PrEvent, window, cx| match event {
                crate::ui::pull_requests::PrEvent::OpenDiff(source) => this.open_diff(source.clone(), cx),
                crate::ui::pull_requests::PrEvent::OpenReviewDiff(source, review) => {
                    this.open_diff(source.clone(), cx);
                    let review = review.clone();
                    this.diff.update(cx, |diff, cx| diff.set_review(Some(review), cx));
                }
                crate::ui::pull_requests::PrEvent::OpenTimeline(target, pr) => {
                    let (target, pr) = (target.clone(), pr.clone());
                    this.timeline = Some(cx.new(|cx| crate::ui::pull_requests::PrTimelineView::new(target, pr, window, cx)));
                    this.focus_group(0, cx);
                    this.front = Front::Timeline;
                    cx.notify();
                }
            }),
            cx.subscribe_in(&find, window, |this, _, event: &crate::ui::navigate::OpenTarget, window, cx| {
                this.go_to_target(event.0.clone(), window, cx)
            }),
            cx.subscribe_in(&find, window, |this, _, event: &crate::ui::find_view::FindViewEvent, window, cx| match event {
                crate::ui::find_view::FindViewEvent::FilesChanged(paths) => this.files_changed(paths.clone(), window, cx),
            }),
            cx.subscribe_in(&project, window, |this, _, event: &crate::ui::navigate::OpenTarget, window, cx| {
                this.go_to_target(event.0.clone(), window, cx)
            }),
            cx.subscribe_in(&project, window, |this, _, event: &crate::ui::project_view::PreviewFile, window, cx| {
                if this.model.read(cx).repository().is_some() {
                    this.open_preview(event.0.clone(), window, cx);
                }
            }),
            cx.subscribe(&code_index, |_, _, _: &IndexEvent, cx| cx.notify()),
            cx.subscribe(&stash, |this, _, event: &StashEvent, cx| match event {
                StashEvent::OpenDiff(source) => this.open_diff(source.clone(), cx),
            }),
            cx.subscribe(&shelf, |this, _, event: &ShelfEvent, cx| match event {
                ShelfEvent::OpenDiff(source) => this.open_diff(source.clone(), cx),
            }),
            cx.subscribe_in(&model, window, |this, _, event, window, cx| {
                if let RepoEvent::Reloaded = event {
                    // Open files follow what git did to the working tree.
                    for tab in &this.editors {
                        tab.view.update(cx, |editor, cx| editor.sync_with_disk(window, cx));
                    }
                    // Index the opened project; re-index what changed on disk.
                    let root = this.model.read(cx).project_root().map(|p| p.to_path_buf());
                    this.code_index.update(cx, |index, cx| {
                        if index.root() != root.as_deref() {
                            index.set_root(root, cx);
                        } else {
                            index.refresh(cx);
                        }
                    });
                }
                if let RepoEvent::OpenLogTab { title, filter } = event {
                    this.open_log_tab(title.clone(), filter.clone(), cx);
                }
                if let RepoEvent::Cloned(dir) = event {
                    clone_dialog::open_cloned(this.model.clone(), dir.clone(), window, cx);
                }
                if let RepoEvent::ShowConflicts = event {
                    // Not over the merge tool or a dialog the user has open.
                    if this.merge.is_none() && !window.has_active_dialog(cx) {
                        this.show_conflicts(window, cx);
                    }
                }
                if let RepoEvent::PrefillCommitMessage(_) = event {
                    this.show_left_tab(LeftTab::Commit, window, cx);
                }
                if let RepoEvent::Compare { old, new } = event {
                    this.changes.update(cx, |changes, cx| changes.compare(old.clone(), new.clone(), cx));
                    this.tools.open(ToolWindow::Changes);
                    cx.notify();
                }
                if let RepoEvent::Notify { title, message, error } = event {
                    // Quiet operations (Stage / Unstage) report only failures.
                    if message.is_empty() && !*error {
                        return;
                    }
                    // Update Project: "N files updated in M commits" with View Commits.
                    let (warning, message) = match message.strip_prefix(dialogs::PARTIAL_FAILURE) {
                        Some(rest) => (true, rest.to_owned()),
                        None => (false, message.clone()),
                    };
                    let (message, updated_range) = match message.split_once('\u{1f}') {
                        Some((text, range)) => (text.to_owned(), Some(range.to_owned())),
                        None => (message.clone(), None),
                    };
                    let message = &message;
                    let title = &if warning { format!("{title} finished with errors") } else { title.clone() };
                    let entity = cx.entity();
                    let mut notification = if *error {
                        // A short summary; the full output is in the Console tab.
                        let detail = message.split_once(" failed: ").map_or(message.as_str(), |(_, rest)| rest);
                        Notification::error(error_summary(detail)).title(title.clone())
                    } else if warning {
                        Notification::warning(message.clone()).title(title.clone())
                    } else {
                        Notification::success(message.clone()).title(title.clone())
                    };
                    // A rejected push asks how to update, in IntelliJ's
                    // Push Rejected dialog rather than a balloon.
                    if message.split_once(" failed: ").map_or(message.as_str(), |(_, rest)| rest).starts_with(dialogs::PUSH_REJECTED)
                        || message.starts_with(dialogs::PUSH_REJECTED)
                    {
                        dialogs::push_rejected(this.model.clone(), window, cx);
                        return;
                    }
                    // IntelliJ's balloon actions: the obvious next step.
                    if *error {
                        notification = notification.action(move |_, _, _| {
                            let entity = entity.clone();
                            Button::new("notify-details").label("Show Details").small().outline().on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.tools.open(ToolWindow::Git);
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
                                    let files = range.split(' ').next().unwrap_or_default();
                                    let Some((old, new)) = files.split_once("..") else { return };
                                    let (old, new) = (old.to_owned(), new.to_owned());
                                    entity.update(cx, |this, cx| this.model.update(cx, |m, cx| m.compare(old, Some(new), cx)));
                                })
                                .into_any_element()
                        });
                        notification = notification.action(move |_, _, _| {
                            let entity = entity.clone();
                            let range = range.clone();
                            Button::new("notify-view-commits").label("View Commits").small().outline().on_click(move |_, _, cx| {
                                let commits = range.split(' ').last().unwrap_or_default().to_owned();
                                entity.update(cx, |this, cx| {
                                    let filter = crate::git::LogFilter { branches: vec![commits], ..Default::default() };
                                    this.open_log_tab("Update Info".into(), filter, cx);
                                })
                            })
                        })
                        .autohide(true);
                    } else if let Some((name, tip)) = (title == "Delete Branch")
                        .then(|| message.strip_prefix("Deleted branch ")?.strip_suffix(')')?.split_once(" (was "))
                        .flatten()
                    {
                        // IntelliJ's "Restore" link brings the deleted branch back.
                        let (name, tip) = (name.to_owned(), tip.to_owned());
                        notification = notification.content(move |_, _, cx| {
                            let palette = cx.palette().clone();
                            let (entity, name, tip) = (entity.clone(), name.clone(), tip.clone());
                            div()
                                .id("notify-restore")
                                .text_sm()
                                .text_color(palette.link)
                                .cursor_pointer()
                                .child("Restore")
                                .on_click(move |_, _, cx| {
                                    let (name, tip) = (name.clone(), tip.clone());
                                    let model = entity.read(cx).model.clone();
                                    model.update(cx, |m, cx| {
                                        m.run_operation("Restore Branch", move |repo| {
                                            repo.run(["branch", &name, &tip])?;
                                            Ok(format!("Restored branch {name}"))
                                        }, cx)
                                    });
                                })
                                .into_any_element()
                        });
                    } else if title == crate::ui::rebase_dialog::DROP_TITLE && message.starts_with(crate::ui::rebase_dialog::DROPPED) {
                        // "Undo" puts the dropped commits back while the branch
                        // is still where the drop left it (rebase saved the old
                        // tip in ORIG_HEAD).
                        let heads = this.model.read(cx).repository().and_then(|r| {
                            let head = |name: &str| r.run(["rev-parse", "--verify", "-q", name]).ok().map(|h| h.trim().to_owned());
                            Some((head("ORIG_HEAD")?, head("HEAD")?))
                        });
                        notification = notification.content(move |_, _, cx| {
                            let palette = cx.palette().clone();
                            let (entity, heads) = (entity.clone(), heads.clone());
                            div()
                                .id("notify-undo-drop")
                                .text_sm()
                                .text_color(palette.link)
                                .cursor_pointer()
                                .child("Undo")
                                .on_click(move |_, _, cx| {
                                    let Some((old, new)) = heads.clone() else { return };
                                    let model = entity.read(cx).model.clone();
                                    model.update(cx, |m, cx| {
                                        m.run_operation("Undo Drop", move |repo| {
                                            crate::git::rebase::undo_drop(repo, &old, &new)?;
                                            Ok("The dropped commits are back".to_owned())
                                        }, cx)
                                    });
                                })
                                .into_any_element()
                        });
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
                                    this.tools.open(ToolWindow::Git);
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
                    // Kept in the Notifications tool window, newest first.
                    this.notifications.insert(0, crate::ui::status_bar::NotificationRecord {
                        title: title.clone(),
                        message: message.clone(),
                        error: *error,
                        warning,
                        time: chrono::Local::now(),
                    });
                    this.notifications.truncate(200);
                    if !this.tools.is_open(ToolWindow::Notifications) {
                        this.unread_notifications += 1;
                    }
                }
            }),
            // Panels are drawn from their last frame until they change; the
            // repository and the index changing redraws the ones showing them.
            cx.observe(&model, |this, _, cx| {
                this.redraw_panels(cx);
                cx.notify();
            }),
            cx.observe(&code_index, |this, _, cx| this.redraw_panels(cx)),
            // A gutter Rollback / Stage / Unstage in the diff changed files.
            cx.subscribe_in(&diff, window, |this, _, event: &crate::ui::diff_view::CommentLine, window, cx| {
                let (path, line) = (event.path.clone(), event.line);
                this.prs.update(cx, |prs, cx| prs.comment_line(path, line, window, cx));
            }),
            cx.subscribe_in(&diff, window, |this, _, event: &crate::ui::navigate::OpenTarget, window, cx| {
                this.go_to_target(event.0.clone(), window, cx)
            }),
            cx.subscribe(&diff, |this, _, _: &crate::ui::diff_view::FilesChanged, cx| {
                this.model.update(cx, |m, cx| m.reload(cx));
            }),
            // Theme "Sync with OS" follows the system's light / dark switch.
            window.observe_window_appearance(|window, cx| {
                crate::theme::refresh(cx);
                window.refresh();
            }),
            // Running an action from the branches popup closes it.
            cx.subscribe(&branches_popup, |this, _, _: &gpui_kit::DismissEvent, cx| {
                this.branches_open = false;
                cx.notify();
            }),
        ];
        // Shortcuts work from the start, and keep working when the focused
        // element goes away (a tool window hidden, a popup closed): focus
        // falls back to the workspace, as IntelliJ returns it to the editor.
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        subscriptions.push(cx.on_focus_lost(window, |this, window, cx| window.focus(&this.focus, cx)));
        let tool_focus = ToolWindow::ALL.into_iter().map(|w| (w, cx.focus_handle())).collect();
        Self {
            model,
            log,
            commit,
            modal_commit: Default::default(),
            stash,
            shelf,
            diff,
            branches_popup,
            worktrees,
            submodules,
            prs,
            code_index,
            find,
            find_popup: None,
            show_find_popup: false,
            search_everywhere: None,
            show_search_everywhere: false,
            recent_files: history::RecentPaths::with_limit(50),
            recently_changed: history::RecentPaths::default(),
            project,
            nav: history::NavHistory::default(),
            changes,
            timeline: None,
            merge: None,
            log_tabs: vec![LogTab { title: "Log".into(), filter: Default::default(), selected: None }],
            active_log: 0,
            editors: Vec::new(),
            front: Front::Diff,
            tab_clock: 0,
            split: None,
            window_title: String::new(),
            active_group: 0,
            closed_tabs: Vec::new(),
            tools: ToolWindows::load(cx),
            notifications: Vec::new(),
            console_wrap: true,
            console_autoscroll: true,
            console_scroll: gpui_kit::ScrollHandle::new(),
            console_seen: std::cell::Cell::new(0),
            unread_notifications: 0,
            caret: cx.new(|_| crate::ui::status_bar::CaretStatus::new()),
            left_tab: LeftTab::Commit,
            branches_open: false,
            focus,
            tool_focus,
            last_tool: None,
            bottom_tab: BottomTab::Log,
            _subscriptions: subscriptions,
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // "project – Junction Studio", as Android Studio titles its windows.
        let title = match self.model.read(cx).repository().and_then(|r| r.root().file_name().map(|n| n.to_string_lossy().into_owned())) {
            Some(name) => format!("{name} – {APP_NAME}"),
            None => APP_NAME.to_owned(),
        };
        if self.window_title != title {
            window.set_window_title(&title);
            self.window_title = title;
        }
        let active = self.editor().cloned();
        self.caret.update(cx, |caret, cx| caret.set_editor(active, cx));
        if !Settings::get(cx).non_modal_commit && self.tools.is_open(ToolWindow::Commit) {
            self.tools.hide(ToolWindow::Commit);
        }
        if Settings::get(cx).non_modal_commit && matches!(self.bottom_tab, BottomTab::LocalChanges | BottomTab::Shelf | BottomTab::Stash) {
            self.bottom_tab = BottomTab::Log;
        }
        let git_on_side = self.tools.side(ToolWindow::Git) != Side::Bottom;
        self.log.update(cx, |log, cx| log.set_details_below(git_on_side, cx));
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
            .child(self.render_editor_groups(has_repo, cx));

        // Left and right tool windows beside the editor, the bottom one under
        // them; sizes are remembered (Settings.tool_window_sizes).
        let sizes = Settings::get(cx).tool_window_sizes;
        let panel_of = |side: Side, this: &Self, cx: &mut Context<Self>| {
            this.tools.active(side).filter(|_| has_repo).filter(|w| *w != ToolWindow::Changes || !this.changes.read(cx).is_empty())
        };
        let (left, right, bottom) = (panel_of(Side::Left, self, cx), panel_of(Side::Right, self, cx), panel_of(Side::Bottom, self, cx));
        let left_content = left.map(|w| self.tool_window_content(w, cx));
        let right_content = right.map(|w| self.tool_window_content(w, cx));
        let bottom_content = bottom.map(|w| self.tool_window_content(w, cx));
        // (settings slot, panel index) pairs a resize may have changed.
        let remember = |pairs: Vec<(usize, usize)>| {
            move |state: &Entity<gpui_kit::component::resizable::ResizableState>, _: &mut Window, cx: &mut gpui_kit::App| {
                for &(slot, panel) in &pairs {
                    let Some(size) = state.read(cx).sizes().get(panel).copied() else { continue };
                    let size = f32::from(size).round() as u32;
                    if size >= 120 && Settings::get(cx).tool_window_sizes[slot] != size {
                        Settings::update(cx, |s| s.tool_window_sizes[slot] = size);
                    }
                }
            }
        };
        let mut sides = Vec::new();
        if left.is_some() {
            sides.push((0, 0));
        }
        if right.is_some() {
            sides.push((1, 2));
        }
        let top = h_resizable("top-split")
            .on_resize(remember(sides))
            .child(
                resizable_panel()
                    .size(px(sizes[0] as f32))
                    .size_range(px(160.)..px(900.))
                    // Side windows keep their width; only the editor gives
                    // way when the other side opens, as in IntelliJ.
                    .flex_none()
                    .visible(left_content.is_some())
                    .children(left_content),
            )
            .child(resizable_panel().child(editor))
            .child(
                resizable_panel()
                    .size(px(sizes[1] as f32))
                    .size_range(px(160.)..px(900.))
                    .flex_none()
                    .visible(right_content.is_some())
                    .children(right_content),
            );

        let main = v_resizable("main-split")
            .on_resize(remember(if bottom.is_some() { vec![(2, 1)] } else { Vec::new() }))
            .child(resizable_panel().child(top))
            .child(
                resizable_panel()
                    .size(px(sizes[2] as f32))
                    .size_range(px(120.)..px(1200.))
                    .visible(bottom_content.is_some())
                    .children(bottom_content),
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
            .on_action(cx.listener(|this, _: &GotoFile, window, cx| this.open_search_everywhere(SeTab::Files, window, cx)))
            .on_action(cx.listener(|this, _: &GotoClass, window, cx| this.open_search_everywhere(SeTab::Classes, window, cx)))
            .on_action(cx.listener(|this, _: &GotoSymbol, window, cx| this.open_search_everywhere(SeTab::Symbols, window, cx)))
            .on_action(cx.listener(|this, _: &SearchEverywhere, window, cx| this.open_search_everywhere(SeTab::All, window, cx)))
            .on_action(cx.listener(|this, _: &FindAction, window, cx| this.open_search_everywhere(SeTab::Actions, window, cx)))
            .on_action(cx.listener(|this, _: &RecentFiles, window, cx| this.recent_files_popup(window, cx)))
            .on_action(cx.listener(|this, _: &CloseTab, _, cx| this.close_active_tab(cx)))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| this.step_tab(1, window, cx)))
            .on_action(cx.listener(|this, _: &PreviousTab, window, cx| this.step_tab(-1, window, cx)))
            .on_action(cx.listener(|this, _: &ReopenClosedTab, window, cx| this.reopen_closed_tab(window, cx)))
            .on_action(cx.listener(|this, _: &FileStructure, window, cx| this.file_structure(window, cx)))
            .on_action(cx.listener(|this, _: &GotoLine, window, cx| this.goto_line(window, cx)))
            .on_action(cx.listener(Self::navigate_back))
            .on_action(cx.listener(|this, _: &NextOccurrence, _, cx| this.step_occurrence(1, cx)))
            .on_action(cx.listener(|this, _: &PreviousOccurrence, _, cx| this.step_occurrence(-1, cx)))
            .on_action(cx.listener(Self::navigate_forward))
            .on_action(cx.listener(Self::toggle_project))
            .on_action(cx.listener(|this, _: &ToggleCommitWindow, window, cx| this.toggle_commit_window(window, cx)))
            .on_action(cx.listener(|this, _: &HideAllToolWindows, window, cx| {
                this.tools.toggle_all();
                window.focus(&this.focus, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &HideActiveToolWindow, window, cx| this.hide_active_tool_window(window, cx)))
            .on_action(cx.listener(Self::select_in_project))
            .on_action(cx.listener(Self::toggle_find))
            .on_action(cx.listener(|this, _: &FindInPath, window, cx| this.open_find_popup(false, window, cx)))
            .on_action(cx.listener(|this, _: &ReplaceInPath, window, cx| this.open_find_popup(true, window, cx)))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| crate::ui::settings_dialog::open(Some(this.model.clone()), window, cx)))
            .on_action(cx.listener(|this, _: &NextDifference, _, cx| match this.front_merge() {
                Some(merge) => merge.update(cx, |m, cx| m.next_difference(cx)),
                None => this.diff.update(cx, |d, cx| d.next_difference(cx)),
            }))
            .on_action(cx.listener(|this, _: &PreviousDifference, _, cx| match this.front_merge() {
                Some(merge) => merge.update(cx, |m, cx| m.previous_difference(cx)),
                None => this.diff.update(cx, |d, cx| d.previous_difference(cx)),
            }))
            .on_action(cx.listener(|this, _: &JumpToSource, _, cx| this.diff.update(cx, |d, cx| d.jump_to_source(cx))))
            .on_action(cx.listener(|this, _: &CompareNextFile, _, cx| this.diff.update(cx, |d, cx| d.compare_next_file(cx))))
            .on_action(cx.listener(|this, _: &ComparePreviousFile, _, cx| this.diff.update(cx, |d, cx| d.compare_previous_file(cx))))
            .size_full()
            .bg(palette.window)
            .text_color(palette.text)
            .text_size(px(13.))
            .child(self.render_title_bar(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .children(self.render_stripe(false, cx))
                    .child(div().flex_1().h_full().child(main))
                    .children(self.render_stripe(true, cx)),
            )
            .child(self.render_status_bar(cx))
            .children(self.render_popups(cx))
    }
}

/// Folders with a build file, as IntelliJ's modules: Gradle, Cargo, Go, Dart,
/// Swift, CMake, npm, Maven. "" is the project root.
pub fn modules_of(files: &[String]) -> Vec<String> {
    use crate::ui::common::BUILD_FILES;
    let mut modules: Vec<String> = files
        .iter()
        .filter_map(|f| {
            let (dir, name) = f.rsplit_once('/').unwrap_or(("", f));
            BUILD_FILES.contains(&name).then(|| dir.to_owned())
        })
        .collect();
    modules.push(String::new());
    modules.sort();
    modules.dedup();
    modules
}


/// The lines of a failed command's output worth a notification: git's
/// `fatal:` / `error:` lines (a multi-remote fetch prints progress for the
/// remotes that worked first), else the first two lines.
fn error_summary(detail: &str) -> String {
    let lines: Vec<&str> = detail.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let errors: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|l| l.starts_with("fatal:") || l.starts_with("error:") || l.starts_with("ERROR:"))
        .collect();
    let chosen = if errors.is_empty() { lines.into_iter().take(2).collect() } else { errors.into_iter().take(3).collect::<Vec<_>>() };
    chosen.join("\n")
}

#[cfg(test)]
mod error_summary_tests {
    #[test]
    fn prefers_fatal_lines() {
        let output = "Fetching origin\nFetching bad\nfatal: '/nonexistent' does not appear to be a git repository\nfatal: Could not read from remote repository.\n\nPlease make sure you have the correct access rights\nerror: could not fetch bad";
        assert_eq!(
            super::error_summary(output),
            "fatal: '/nonexistent' does not appear to be a git repository\nfatal: Could not read from remote repository.\nerror: could not fetch bad"
        );
        assert_eq!(super::error_summary("a\nb\nc"), "a\nb");
    }
}
