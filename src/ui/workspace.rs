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

#[path = "workspace_tabs.rs"]
mod tabs;
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
    recent_files: Vec<String>,
    /// Files saved or replaced this session, newest first.
    recently_changed: Vec<String>,
    project: Entity<crate::ui::project_view::ProjectView>,
    /// Navigate › Back / Forward: (path, line, column).
    nav_back: Vec<(String, u32, u32)>,
    nav_forward: Vec<(String, u32, u32)>,
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
                    let (message, updated_range) = match message.split_once('\u{1f}') {
                        Some((text, range)) => (text.to_owned(), Some(range.to_owned())),
                        None => (message.clone(), None),
                    };
                    let message = &message;
                    let entity = cx.entity();
                    let mut notification = if *error {
                        // A short summary; the full output is in the Console tab.
                        let detail = message.split_once(" failed: ").map_or(message.as_str(), |(_, rest)| rest);
                        Notification::error(error_summary(detail)).title(title.clone())
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
                                            if repo.run(["rev-parse", "HEAD"])?.trim() != new {
                                                anyhow::bail!("The branch has changed since the commits were dropped");
                                            }
                                            repo.run(["reset", "--keep", &old])?;
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
                        time: chrono::Local::now(),
                    });
                    this.notifications.truncate(200);
                    if !this.tools.is_open(ToolWindow::Notifications) {
                        this.unread_notifications += 1;
                    }
                }
            }),
            cx.observe(&model, |_, _, cx| cx.notify()),
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
            recent_files: Vec::new(),
            recently_changed: Vec::new(),
            project,
            nav_back: Vec::new(),
            nav_forward: Vec::new(),
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

    fn show_conflicts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = cx.entity();
        dialogs::conflicts(
            self.model.clone(),
            Rc::new(move |conflict, window, cx| workspace.update(cx, |this, cx| this.open_merge(conflict, window, cx))),
            window,
            cx,
        );
    }

    /// The merge tool, when it is what the editor area shows (F7 goes to it).
    fn front_merge(&self) -> Option<Entity<MergeView>> {
        self.merge.as_ref().filter(|_| self.front == Front::Merge).map(|(m, _)| m.clone())
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
            MergeEvent::Compare(source) => this.open_diff(source.clone(), cx),
            MergeEvent::Closed(applied) => {
                this.merge = None;
                this.fix_front(cx);
                // Back to the Conflicts dialog while files still conflict
                // (the status doesn't know yet of one just resolved).
                let left = merge::conflicts(this.model.read(cx).status()).len();
                if left > usize::from(*applied) {
                    this.show_conflicts(window, cx);
                }
                cx.notify();
            }
        });
        self.merge = Some((view, subscription));
        self.focus_group(0, cx);
        self.front = Front::Merge;
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
        let editing = state == RepositoryState::Rebasing
            && model.repository().is_some_and(|r| r.git_dir().join("rebase-merge").join("amend").exists());
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
                    // An `edit` step of an interactive rebase, not a conflict.
                    0 if editing => "Stopped for editing".to_owned(),
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
                .child(Button::new("op-abort").small().outline().label("Abort").on_click({
                    let model = self.model.clone();
                    move |_, window, cx| dialogs::abort_operation(model.clone(), state, window, cx)
                })),
        )
    }

    /// Annotate with Git Blame: opens the file (at `revision`, read-only)
    /// with the annotations in its gutter, as IntelliJ does.
    pub fn annotate(&mut self, path: String, revision: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.open_file(path, revision, window, cx);
        if let Some(editor) = self.editor() {
            editor.update(cx, |editor, cx| editor.show_annotations(cx));
        }
    }

    /// Show History: a Log tab for one file, following renames, as IntelliJ's "History: name" tab.
    /// Opens a file in the editor: the working tree (`None`) or a revision, read-only.
    pub fn open_file(&mut self, path: String, revision: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.read(cx).repository().is_none() {
            return;
        }
        if revision.is_none() {
            self.recent_files.retain(|p| *p != path);
            self.recent_files.insert(0, path.clone());
            self.recent_files.truncate(50);
        }
        self.open_tab(path, revision, window, cx);
    }

    fn on_file_editor_event(&mut self, view: &Entity<FileEditor>, event: &FileEditorEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            FileEditorEvent::Edited => self.keep_tab(view, cx),
            FileEditorEvent::Navigate(targets) => self.navigate(targets.clone(), window, cx),
            FileEditorEvent::NoDeclaration { text, offset } => {
                let Some(path) = self.editor().map(|e| e.read(cx).path().to_owned()) else { return };
                if self.code_index.read(cx).declared_at(&path, text, *offset) {
                    self.show_usages_popup(path, text.clone(), *offset, window, cx);
                } else {
                    Self::nav_hint("Cannot find declaration to go to", window, cx);
                }
            }
            FileEditorEvent::FindUsages { text, offset } => {
                let Some(path) = self.editor().map(|e| e.read(cx).path().to_owned()) else { return };
                let (index, text, offset) = (self.code_index.clone(), text.clone(), *offset);
                let _ = index;
                self.find.update(cx, |view, cx| view.find_usages(path, text, offset, cx));
                self.tools.open(ToolWindow::Git);
                self.bottom_tab = BottomTab::Find;
                cx.notify();
            }
            FileEditorEvent::Saved(path) => {
                let path = path.clone();
                self.recently_changed.retain(|p| *p != path);
                self.recently_changed.insert(0, path.clone());
                self.code_index.update(cx, |index, cx| index.refresh_file(&path, cx));
            }
            FileEditorEvent::CreateGist { name, content } => {
                crate::ui::github_dialogs::create_gist(self.model.clone(), vec![(name.clone(), content.clone())], window, cx)
            }
            FileEditorEvent::Annotate { path, revision } => self.annotate(path.clone(), revision.clone(), window, cx),
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
                self.tools.open(ToolWindow::Git);
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
        self.tools.open(ToolWindow::Git);
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

    /// Go to Declaration's result: one target opens, several ask.
    fn navigate(&mut self, targets: Vec<crate::index::nav::Target>, window: &mut Window, cx: &mut Context<Self>) {
        match targets.len() {
            0 => Self::nav_hint("Cannot find declaration to go to", window, cx),
            1 => self.go_to_target(targets.into_iter().next().unwrap(), window, cx),
            _ => {
                let workspace = cx.entity().downgrade();
                let locations = targets.iter().map(|t| self.location_label(t, cx)).collect();
                crate::ui::navigate::choose_target(
                    "Choose Declaration",
                    targets,
                    locations,
                    Rc::new(move |target, window, cx| {
                        workspace.update(cx, |this, cx| this.go_to_target(target, window, cx)).ok();
                    }),
                    window,
                    cx,
                );
            }
        }
    }

    /// A navigation miss, as IntelliJ's hint: information, not success.
    fn nav_hint(message: &str, window: &mut Window, cx: &mut Context<Self>) {
        window.push_notification(Notification::info(message.to_owned()).title("Go to Declaration"), cx);
    }

    /// Where a target is, for a chooser row: "dir/File.kt:12", or for a
    /// library file "JDK 17 › java/lang/String.java:120".
    fn location_label(&self, target: &crate::index::nav::Target, cx: &gpui_kit::App) -> String {
        let line = target.line + 1;
        if crate::index::store::ProjectIndex::is_external(&target.path) {
            let index = self.code_index.read(cx).index.clone();
            let library = index.read().ok().and_then(|index| {
                let library = index.external.library_of(&target.path)?;
                let rel = std::path::Path::new(&target.path).strip_prefix(&library.root).ok()?.to_string_lossy().replace('\\', "/");
                Some(format!("{} › {rel}:{line}", library.name))
            });
            if let Some(label) = library {
                return label;
            }
        }
        format!("{}:{line}", target.path)
    }

    /// Go to Declaration on a declaration: its usages, as IntelliJ's Show
    /// Usages popup (one usage jumps straight there).
    fn show_usages_popup(&mut self, path: String, text: String, offset: usize, window: &mut Window, cx: &mut Context<Self>) {
        let task = self.code_index.read(cx).usages(path, text, offset, cx);
        cx.spawn_in(window, async move |this, cx| {
            let (word, usages) = task.await;
            this.update_in(cx, |this, window, cx| {
                let targets: Vec<crate::index::nav::Target> = usages
                    .into_iter()
                    .filter(|u| u.group != "Declarations")
                    .map(|u| crate::index::nav::Target { path: u.path, line: u.line, col: u.col, name: u.text, label: String::new(), container: None })
                    .collect();
                match targets.len() {
                    0 => window.push_notification(Notification::info(format!("No usages of {word} found")).title("Show Usages"), cx),
                    1 => this.go_to_target(targets.into_iter().next().unwrap(), window, cx),
                    _ => {
                        let locations = targets.iter().map(|t| this.location_label(t, cx)).collect();
                        crate::ui::navigate::choose_target("Usages", targets, locations, this.picker_callback(cx), window, cx);
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Opens a file at a position, remembering where we were for Back.
    fn go_to_target(&mut self, target: crate::index::nav::Target, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(here) = self.current_position(cx) {
            if self.nav_back.last() != Some(&here) {
                self.nav_back.push(here);
            }
        }
        self.nav_forward.clear();
        self.open_at(target.path, target.line, target.col, window, cx);
    }

    fn current_position(&self, cx: &gpui_kit::App) -> Option<(String, u32, u32)> {
        let editor = self.editor()?.read(cx);
        if editor.revision().is_some() {
            return None;
        }
        let (line, col) = editor.cursor(cx);
        Some((editor.path().to_owned(), line, col))
    }

    fn open_at(&mut self, path: String, line: u32, col: u32, window: &mut Window, cx: &mut Context<Self>) {
        self.open_file(path, None, window, cx);
        if let Some(editor) = self.editor().cloned() {
            editor.update(cx, |editor, cx| editor.go_to(line, col, window, cx));
        }
    }

    /// Ctrl+Alt+Down / Up: the Find window's next / previous result, opened.
    /// Without results the keys go on to the editor (add a caret).
    fn step_occurrence(&mut self, delta: isize, cx: &mut Context<Self>) {
        if !self.find.read(cx).has_occurrences() {
            cx.propagate();
            return;
        }
        self.find.update(cx, |find, cx| find.step(delta, cx));
    }

    fn navigate_back(&mut self, _: &NavigateBack, window: &mut Window, cx: &mut Context<Self>) {
        let Some((path, line, col)) = self.nav_back.pop() else { return };
        if let Some(here) = self.current_position(cx) {
            self.nav_forward.push(here);
        }
        self.open_at(path, line, col, window, cx);
    }

    fn navigate_forward(&mut self, _: &NavigateForward, window: &mut Window, cx: &mut Context<Self>) {
        let Some((path, line, col)) = self.nav_forward.pop() else { return };
        if let Some(here) = self.current_position(cx) {
            self.nav_back.push(here);
        }
        self.open_at(path, line, col, window, cx);
    }

    /// The Module / Scope tabs' view of the project.
    fn scope_data(&self, cx: &gpui_kit::App) -> crate::ui::find_popup::ScopeData {
        let current_file = self.editor().filter(|e| e.read(cx).revision().is_none()).map(|e| e.read(cx).path().to_owned());
        let open_files: Vec<String> = self.editors.iter().filter(|t| t.view.read(cx).revision().is_none()).map(|t| t.view.read(cx).path().to_owned()).collect();
        let local_changes: Vec<String> = self.model.read(cx).status().entries.iter().map(|e| e.path.clone()).collect();
        let mut recently_changed = self.recently_changed.clone();
        recently_changed.extend(local_changes.iter().filter(|p| !self.recently_changed.contains(p)).cloned());
        let modules = self.code_index.read(cx).root().map(|root| modules_of(&crate::index::store::list_files(root))).unwrap_or_default();
        crate::ui::find_popup::ScopeData {
            open_files,
            current_file,
            recent_files: self.recent_files.clone(),
            recently_changed,
            local_changes,
            modules,
        }
    }

    /// Find in Files (Ctrl+Shift+F) / Replace in Files (Ctrl+Shift+R).
    fn open_find_popup(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.read(cx).repository().is_none() {
            return;
        }
        if self.find_popup.is_none() {
            let index = self.code_index.clone();
            let popup = cx.new(|cx| crate::ui::find_popup::FindPopup::new(index, window, cx));
            let subscription = cx.subscribe_in(&popup, window, |this, _, event: &crate::ui::find_popup::FindEvent, window, cx| {
                use crate::ui::find_popup::FindEvent;
                match event {
                    FindEvent::Open(target) => this.go_to_target(target.clone(), window, cx),
                    FindEvent::Close => this.close_popups(window, cx),
                    FindEvent::ShowInFindWindow(request) => {
                        let request = request.clone();
                        this.find.update(cx, |view, cx| view.find_text(request, cx));
                        this.tools.open(ToolWindow::Git);
                        this.bottom_tab = BottomTab::Find;
                        cx.notify();
                    }
                    FindEvent::FilesChanged(paths) => this.files_changed(paths.clone(), window, cx),
                }
            });
            self.find_popup = Some((popup, subscription));
        }
        let text = self.editor().and_then(|e| e.read(cx).search_text(cx));
        let data = self.scope_data(cx);
        if let Some((popup, _)) = &self.find_popup {
            popup.update(cx, |p, cx| p.show(replace, text, data, window, cx));
        }
        self.show_find_popup = true;
        cx.notify();
    }

    fn close_popups(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_find_popup = false;
        self.show_search_everywhere = false;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Files changed on disk by Replace: re-index, refresh VCS, reload an unmodified editor.
    fn files_changed(&mut self, paths: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        for path in &paths {
            self.recently_changed.retain(|p| p != path);
            self.recently_changed.insert(0, path.clone());
            let path = path.clone();
            self.code_index.update(cx, |index, cx| index.refresh_file(&path, cx));
        }
        self.model.update(cx, |m, cx| m.reload(cx));
        // Unmodified tabs of changed files reload from disk.
        let stale: Vec<usize> = (0..self.editors.len())
            .filter(|&i| {
                let e = self.editors[i].view.read(cx);
                e.revision().is_none() && paths.iter().any(|p| p == e.path()) && !e.is_dirty(cx)
            })
            .collect();
        for ix in stale {
            let Some(repository) = self.model.read(cx).repository().cloned() else { break };
            let old = self.editors[ix].view.clone();
            let (path, (line, col)) = (old.read(cx).path().to_owned(), old.read(cx).cursor(cx));
            let view = cx.new(|cx| FileEditor::new(repository, path, None, window, cx));
            let subscription = cx.subscribe_in(&view, window, Self::on_file_editor_event);
            let index = self.code_index.clone();
            view.update(cx, |editor, cx| {
                editor.attach_index(index, cx);
                editor.go_to(line, col, window, cx);
            });
            let tab = &mut self.editors[ix];
            tab.view = view;
            tab._subscription = subscription;
        }
        cx.notify();
    }

    /// One tab group: its tab bar and what its selected tab shows.
    fn render_group(&self, focused: bool, has_repo: bool, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let group = self.active_group;
        let entity = cx.entity();
        v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .on_mouse_down(gpui_kit::MouseButton::Left, move |_, _, cx| entity.update(cx, |this, cx| this.focus_group(group, cx)))
            .children(self.render_tab_bar(focused, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .when(focused && matches!(self.front, Front::Editor(_)), |el| el.key_context(TABS_CONTEXT))
                    .map(|el| {
                        if !has_repo {
                            return el.child(self.render_welcome(cx));
                        }
                        match self.front {
                            Front::Editor(ix) if ix < self.editors.len() => el.child(self.editors[ix].view.clone()),
                            _ if group != 0 => el,
                            Front::Timeline if self.timeline.is_some() => el.child(self.timeline.clone().unwrap()),
                            Front::Merge if self.merge.is_some() => el.child(self.merge.as_ref().unwrap().0.clone()),
                            _ => el.child(self.diff.clone()),
                        }
                    }),
            )
            .into_any_element()
    }

    /// The editor area: one tab group, or two after Split Right / Down.
    fn render_editor_groups(&mut self, has_repo: bool, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let Some(vertical) = self.split.as_ref().map(|s| s.vertical) else {
            return div().flex_1().min_h_0().flex().child(self.render_group(true, has_repo, cx)).into_any_element();
        };
        let focused = self.active_group;
        let mine = self.render_group(true, has_repo, cx);
        self.swap_split();
        let other = self.render_group(false, has_repo, cx);
        self.swap_split();
        let (first, second) = if focused == 0 { (mine, other) } else { (other, mine) };
        let palette = cx.palette().clone();
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .when(vertical, |el| el.flex_col())
            .child(first)
            .child(if vertical { div().h(px(1.)).w_full().bg(palette.border) } else { div().w(px(1.)).h_full().bg(palette.border) })
            .child(second)
            .into_any_element()
    }

    fn render_popups(&self, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let mut out = Vec::new();
        let popup = self.find_popup.as_ref().filter(|_| self.show_find_popup).map(|(p, _)| (p.clone(), p.read(cx).is_pinned()));
        if let Some((popup, pinned)) = popup {
            if !pinned {
                out.push(
                    div()
                        .id("popup-backdrop")
                        .absolute()
                        .inset_0()
                        .on_mouse_down(gpui_kit::MouseButton::Left, cx.listener(|this, _, window, cx| this.close_popups(window, cx)))
                        .into_any_element(),
                );
            }
            out.push(div().absolute().top(px(72.)).left_0().right_0().flex().justify_center().child(popup).into_any_element());
        }
        if let Some((se, _)) = self.search_everywhere.as_ref().filter(|_| self.show_search_everywhere) {
            out.push(
                div()
                    .id("se-backdrop")
                    .absolute()
                    .inset_0()
                    .on_mouse_down(gpui_kit::MouseButton::Left, cx.listener(|this, _, window, cx| this.close_popups(window, cx)))
                    .into_any_element(),
            );
            out.push(div().absolute().top(px(96.)).left_0().right_0().flex().justify_center().child(se.clone()).into_any_element());
        }
        out
    }

    /// Every command Find Action (Ctrl+Shift+A) and the Actions tab know.
    fn action_entries(&self, cx: &mut Context<Self>) -> Vec<crate::ui::search_everywhere::ActionEntry> {
        use crate::ui::search_everywhere::ActionEntry;
        let weak = cx.entity().downgrade();
        let mut out = Vec::new();
        let mut add = |name: &str, shortcut: &str, group: &str, run: VcsRun| {
            let weak = weak.clone();
            out.push(ActionEntry {
                name: name.to_owned(),
                shortcut: crate::ui::file_menus::shortcut_label(shortcut),
                group: group.to_owned(),
                run: Rc::new(move |window, cx| {
                    weak.update(cx, |this, cx| run(this, window, cx)).ok();
                }),
            });
        };
        let op = |f: fn(&mut Workspace, &mut Window, &mut Context<Workspace>)| -> VcsRun { Rc::new(f) };
        for (name, shortcut, run) in Self::vcs_operation_list().into_iter().flatten() {
            add(name, shortcut, "Git", run);
        }
        let navigate: Vec<(&str, &str, VcsRun)> = vec![
            ("Search Everywhere", "Double Shift", op(|this, window, cx| this.open_search_everywhere(SeTab::All, window, cx))),
            ("Find in Files…", "Ctrl+Shift+F", op(|this, window, cx| this.open_find_popup(false, window, cx))),
            ("Replace in Files…", "Ctrl+Shift+R", op(|this, window, cx| this.open_find_popup(true, window, cx))),
            ("Go to Class…", "Ctrl+N", op(|this, window, cx| this.open_search_everywhere(SeTab::Classes, window, cx))),
            ("Go to File…", "Ctrl+Shift+N", op(|this, window, cx| this.open_search_everywhere(SeTab::Files, window, cx))),
            ("Go to Symbol…", "Ctrl+Alt+Shift+N", op(|this, window, cx| this.open_search_everywhere(SeTab::Symbols, window, cx))),
            ("Go to Text…", "", op(|this, window, cx| this.open_search_everywhere(SeTab::Text, window, cx))),
            ("Recent Files", "Ctrl+E", op(|this, window, cx| this.recent_files_popup(window, cx))),
            ("File Structure", "Ctrl+F12", op(|this, window, cx| this.file_structure(window, cx))),
            ("Go to Line:Column…", "Ctrl+G", op(|this, window, cx| this.goto_line(window, cx))),
            ("Back", "Ctrl+Alt+Left", op(|this, window, cx| this.navigate_back(&NavigateBack, window, cx))),
            ("Forward", "Ctrl+Alt+Right", op(|this, window, cx| this.navigate_forward(&NavigateForward, window, cx))),
            ("Select in Project View", "Alt+F1", op(|this, window, cx| this.select_in_project(&SelectInProject, window, cx))),
        ];
        for (name, shortcut, run) in navigate {
            add(name, shortcut, "Navigate", run);
        }
        let windows: Vec<(&str, &str, VcsRun)> = vec![
            ("Project", "Alt+1", op(|this, window, cx| this.toggle_project(&ToggleProjectWindow, window, cx))),
            ("Find", "Alt+3", op(|this, window, cx| this.toggle_find(&ToggleFindWindow, window, cx))),
            ("Git", "Alt+9", op(|this, window, cx| this.on_toggle_git(&ToggleGitWindow, window, cx))),
            ("Commit", "Alt+0", op(|this, window, cx| this.show_left_tab(LeftTab::Commit, window, cx))),
            ("Refresh", "Ctrl+Alt+Y", op(|this, window, cx| this.on_refresh(&Refresh, window, cx))),
            ("VCS Operations Popup…", "Alt+`", op(|this, window, cx| this.on_vcs_operations(&VcsOperations, window, cx))),
            ("Close Tab", "Ctrl+F4", op(|this, _, cx| this.close_active_tab(cx))),
            ("Select Next Tab", "Alt+Right", op(|this, window, cx| this.step_tab(1, window, cx))),
            ("Select Previous Tab", "Alt+Left", op(|this, window, cx| this.step_tab(-1, window, cx))),
            ("Reopen Closed Tab", "", op(|this, window, cx| this.reopen_closed_tab(window, cx))),
            ("Close All Tabs", "", op(|this, _, cx| {
                let all = (0..this.editors.len()).collect();
                this.close_tabs(all, cx);
            })),
        ];
        for (name, shortcut, run) in windows {
            add(name, shortcut, "Window", run);
        }
        out
    }

    /// Search Everywhere on a tab; its own shortcut again toggles non-project items.
    fn open_search_everywhere(&mut self, tab: SeTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.read(cx).repository().is_none() {
            return;
        }
        if let Some((se, _)) = &self.search_everywhere {
            if self.show_search_everywhere && se.read(cx).tab() == tab {
                se.update(cx, |se, cx| se.toggle_non_project(cx));
                return;
            }
        }
        if self.search_everywhere.is_none() {
            let index = self.code_index.clone();
            let se = cx.new(|cx| crate::ui::search_everywhere::SearchEverywhere::new(index, window, cx));
            let subscription = cx.subscribe_in(&se, window, |this, _, event: &crate::ui::search_everywhere::SeEvent, window, cx| {
                use crate::ui::search_everywhere::SeEvent;
                match event {
                    SeEvent::Open(target) => this.go_to_target(target.clone(), window, cx),
                    SeEvent::Close => this.close_popups(window, cx),
                    SeEvent::Run(run) => {
                        let run = run.clone();
                        window.defer(cx, move |window, cx| run(window, cx));
                    }
                    SeEvent::FindWindowText(request) => {
                        let request = request.clone();
                        this.find.update(cx, |view, cx| view.find_text(request, cx));
                        this.tools.open(ToolWindow::Git);
                        this.bottom_tab = BottomTab::Find;
                        cx.notify();
                    }
                    SeEvent::FindWindowItems(title, items) => {
                        let (title, items) = (title.clone(), items.clone());
                        this.find.update(cx, |view, cx| view.show_items(title, items, cx));
                        this.tools.open(ToolWindow::Git);
                        this.bottom_tab = BottomTab::Find;
                        cx.notify();
                    }
                }
            });
            self.search_everywhere = Some((se, subscription));
        }
        self.show_find_popup = false;
        let text = self.editor().and_then(|e| e.read(cx).selected_text(cx));
        let actions = self.action_entries(cx);
        if let Some((se, _)) = &self.search_everywhere {
            se.update(cx, |se, cx| se.show(tab, text, actions, window, cx));
        }
        self.show_search_everywhere = true;
        cx.notify();
    }

    fn picker_callback(&self, cx: &mut Context<Self>) -> Rc<dyn Fn(crate::index::nav::Target, &mut Window, &mut gpui_kit::App)> {
        let workspace = cx.entity().downgrade();
        Rc::new(move |target, window, cx| {
            workspace.update(cx, |this, cx| this.go_to_target(target, window, cx)).ok();
        })
    }

    /// Recent Files (Ctrl+E).
    fn recent_files_popup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.editor().map(|e| e.read(cx).path().to_owned());
        // IntelliJ preselects the previous file, so Ctrl+E Enter switches back.
        let mut files: Vec<String> = self.recent_files.iter().filter(|p| Some(*p) != current.as_ref()).cloned().collect();
        files.extend(current);
        let items = files
            .into_iter()
            .map(|path| {
                let name = path.rsplit(['/', '\\']).next().unwrap_or(&path).to_owned();
                // A library file shows its library and folder, not a cache path.
                let target = crate::index::nav::Target { path: path.clone(), line: 0, col: 0, name: String::new(), label: String::new(), container: None };
                let location = self.location_label(&target, cx);
                let location = location.rsplit_once(':').map_or(location.as_str(), |(l, _)| l);
                let dir = match location.rsplit_once(['/', '\\']) {
                    Some((d, _)) => d.to_owned(),
                    None => location.split_once(" › ").map(|(library, _)| library.to_owned()).unwrap_or_default(),
                };
                crate::ui::find_view::FoundItem {
                    title: name.clone(),
                    detail: dir,
                    icon: crate::ui::common::file_icon(&path),
                    target: crate::index::nav::Target { path, line: 0, col: 0, name, label: String::new(), container: None },
                }
            })
            .collect::<Vec<_>>();
        let n = items.len();
        let on_pick = self.picker_callback(cx);
        let recent: Vec<String> = self.recent_files.clone();
        let workspace = cx.entity().downgrade();
        // Re-opening a recent file keeps its last caret: open without a position.
        let open_plain: Rc<dyn Fn(crate::index::nav::Target, &mut Window, &mut gpui_kit::App)> = Rc::new(move |target, window, cx| {
            let _ = &recent;
            let path = target.path.clone();
            workspace.update(cx, |this, cx| this.open_file(path, None, window, cx)).ok();
        });
        let _ = on_pick;
        crate::ui::navigate::pick_from_list("Recent Files", items, vec![0; n], open_plain, window, cx);
    }

    /// File Structure (Ctrl+F12): the current file's declarations.
    fn file_structure(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.editor() else { return };
        let path = editor.read(cx).path().to_owned();
        let symbols = self.code_index.read(cx).file_symbols(&path);
        let mut indents = Vec::new();
        let items = symbols
            .into_iter()
            .map(|m| {
                indents.push(m.target.container.as_deref().map(|c| c.split('.').count()).unwrap_or(0));
                crate::ui::find_view::FoundItem {
                    title: m.target.name.clone(),
                    detail: m.target.label.split(" · ").next().unwrap_or_default().to_owned(),
                    icon: crate::ui::navigate::symbol_icon(m.kind),
                    target: m.target,
                }
            })
            .collect();
        let on_pick = self.picker_callback(cx);
        crate::ui::navigate::pick_from_list("File Structure", items, indents, on_pick, window, cx);
    }

    /// Go to Line:Column (Ctrl+G).
    fn goto_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.editor() else { return };
        let editor = editor.clone();
        let (line, col) = editor.read(cx).cursor(cx);
        let lines = editor.read(cx).line_count(cx);
        let input = cx.new(|cx| gpui_kit::component::input::InputState::new(window, cx).default_value(format!("{}:{}", line + 1, col + 1)));
        let workspace = cx.entity().downgrade();
        let go = {
            let (input, editor, workspace) = (input.clone(), editor.clone(), workspace.clone());
            move |window: &mut Window, cx: &mut gpui_kit::App| {
                let text = input.read(cx).value().to_string();
                let mut parts = text.split([':', ',']).map(|p| p.trim().parse::<u32>().ok());
                let Some(Some(line)) = parts.next() else { return };
                let col = parts.next().flatten().unwrap_or(1);
                let target_line = line.saturating_sub(1);
                let (path, here) = { let e = editor.read(cx); (e.path().to_owned(), e.cursor(cx)) };
                workspace
                    .update(cx, |this, cx| {
                        let here = (path.clone(), here.0, here.1);
                        if this.nav_back.last() != Some(&here) {
                            this.nav_back.push(here);
                        }
                        editor.update(cx, |e, cx| e.go_to(target_line, col.saturating_sub(1), window, cx));
                    })
                    .ok();
            }
        };
        let go = Rc::new(go);
        let sub_go = go.clone();
        let subscription = cx.subscribe_in(&input, window, move |_, _, event: &gpui_kit::component::input::InputEvent, window, cx| {
            if let gpui_kit::component::input::InputEvent::PressEnter { .. } = event {
                sub_go(window, cx);
                window.close_dialog(cx);
            }
        });
        let input2 = input.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let _ = &subscription;
            let go = go.clone();
            dialog
                .title("Go to Line:Column")
                .w(px(360.))
                .child(
                    v_flex()
                        .gap_1()
                        .child(div().text_sm().child(format!("[Line] [:column]   (1–{lines})")))
                        .child(gpui_kit::component::input::Input::new(&input2)),
                )
                .footer(dialogs::footer("OK"))
                .on_ok(move |_, window, cx| {
                    go(window, cx);
                    true
                })
        });
        // IntelliJ selects the prefilled position, so typing replaces it.
        let select = input.clone();
        window.defer(cx, move |window, cx| {
            select.update(cx, |state, cx| {
                state.focus(window, cx);
                state.select_all(window, cx);
            })
        });
    }

    fn toggle_project(&mut self, _: &ToggleProjectWindow, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_tool(ToolWindow::Project, window, cx);
    }

    /// Select In › Project View: shows the current editor's file in the tree.
    fn select_in_project(&mut self, _: &SelectInProject, _: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.editor() else { return };
        let path = editor.read(cx).path().to_owned();
        self.tools.open(ToolWindow::Project);
        self.project.update(cx, |project, cx| project.reveal(&path, cx));
        cx.notify();
    }

    fn toggle_find(&mut self, _: &ToggleFindWindow, _: &mut Window, cx: &mut Context<Self>) {
        if self.tools.is_open(ToolWindow::Git) && self.bottom_tab == BottomTab::Find {
            self.tools.hide(ToolWindow::Git);
        } else {
            self.tools.open(ToolWindow::Git);
            self.bottom_tab = BottomTab::Find;
        }
        cx.notify();
    }

    fn file_action(&mut self, action: crate::ui::file_menus::FileAction, window: &mut Window, cx: &mut Context<Self>) {
        use crate::ui::diff_view::DiffSource;
        use crate::ui::file_menus::FileAction;
        match action {
            FileAction::ShowDiff(path) => {
                let unversioned = self.model.read(cx).status().entries.iter().any(|e| e.path == path && e.kind == crate::git::StatusKind::Unversioned);
                self.open_diff(DiffSource::WorkingTree { path, unversioned }, cx)
            }
            FileAction::OpenFile(path) => self.open_file(path, None, window, cx),
            FileAction::Annotate(path) => self.annotate(path, None, window, cx),
            FileAction::ShowHistory(path) => self.show_history(path, cx),
            FileAction::CompareWithRevision(path) => {
                let workspace = cx.entity();
                dialogs::compare_file_with_revision(
                    self.model.clone(),
                    path,
                    Rc::new(move |source, _, cx| workspace.update(cx, |this, cx| this.open_diff(source, cx))),
                    window,
                    cx,
                );
            }
            FileAction::CompareWithBranch(path) => {
                let workspace = cx.entity();
                dialogs::compare_file_with(
                    self.model.clone(),
                    path,
                    Rc::new(move |source, _, cx| workspace.update(cx, |this, cx| this.open_diff(source, cx))),
                    window,
                    cx,
                );
            }
            FileAction::CompareWithClipboard(path) => {
                let text = cx.read_from_clipboard().and_then(|c| c.text()).unwrap_or_default();
                self.open_diff(DiffSource::Clipboard { path, text }, cx)
            }
            FileAction::CompareWithFile(path) => {
                let paths = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Compare With".into()) });
                cx.spawn(async move |this, cx| {
                    let Ok(Ok(Some(picked))) = paths.await else { return };
                    let Some(other) = picked.into_iter().next() else { return };
                    this.update(cx, |this, cx| this.open_diff(DiffSource::Files { path, other }, cx)).ok();
                })
                .detach();
            }
            FileAction::CommitFiles(paths) => {
                self.show_left_tab(LeftTab::Commit, window, cx);
                self.commit.update(cx, |commit, cx| commit.commit_only(paths, window, cx));
                cx.notify();
            }
            FileAction::Branches => self.open_branches(window, cx),
            FileAction::Unstash => {
                self.show_left_tab(LeftTab::Stash, window, cx);
            }
            FileAction::FilesChanged => {
                self.code_index.update(cx, |index, cx| index.refresh(cx));
                self.model.update(cx, |m, cx| m.reload(cx));
            }
        }
    }

    fn open_diff(&mut self, source: crate::ui::diff_view::DiffSource, cx: &mut Context<Self>) {
        let Some(repository) = self.model.read(cx).repository().cloned() else { return };
        // A diff replaces the annotations and the file editor in the editor area.
        self.focus_group(0, cx);
        self.front = Front::Diff;
        self.diff.update(cx, |diff, cx| diff.show(repository, source, cx));
        cx.notify();
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
            .child(div().text_xl().font_weight(FontWeight::BOLD).child(format!("Welcome to {APP_NAME}")))
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
                let problem = self.model.read(cx).open_problem().cloned();
                let (title, hint) = match &problem {
                    Some(OpenProblem::GitMissing) => (
                        "Git is not installed",
                        "Junction Studio runs the git command-line tool. Install Git for Windows (or point Settings › Git to an existing git.exe), then retry.",
                    ),
                    Some(OpenProblem::Unsafe(_)) => (
                        "The folder is owned by another user",
                        "Git only opens repositories you own unless you trust them. Trusting adds the folder to safe.directory in your global git config.",
                    ),
                    None => ("Cannot open the folder", ""),
                };
                let mut buttons = h_flex().gap_2();
                match problem {
                    Some(OpenProblem::GitMissing) => {
                        buttons = buttons
                            .child(Button::new("welcome-get-git").small().primary().label("Download Git").on_click(|_, _, cx| {
                                cx.open_url(if cfg!(windows) { "https://git-scm.com/download/win" } else { "https://git-scm.com/downloads" })
                            }))
                            .child(Button::new("welcome-git-settings").small().label("Set Path to Git…").on_click(cx.listener(|this, _, window, cx| crate::ui::settings_dialog::open(Some(this.model.clone()), window, cx))))
                            .child(Button::new("welcome-retry").small().label("Retry").on_click(cx.listener(|this, _, _, cx| this.model.update(cx, |m, cx| m.retry_open(cx)))));
                    }
                    Some(OpenProblem::Unsafe(root)) => {
                        buttons = buttons.child(Button::new("welcome-trust").small().primary().label("Trust Directory and Open").on_click(cx.listener(
                            move |this, _, window, cx| match crate::git::trust_directory(&root) {
                                Ok(()) => this.model.update(cx, |m, cx| m.retry_open(cx)),
                                Err(e) => window.push_notification(Notification::error(e.to_string()), cx),
                            },
                        )));
                    }
                    None => {}
                }
                el.child(
                    v_flex()
                        .max_w(px(560.))
                        .p_3()
                        .gap_2()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(palette.status_conflict)
                        .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).text_color(palette.status_conflict).child(title))
                        .child(div().text_xs().text_color(palette.text_secondary).child(error))
                        .when(!hint.is_empty(), |el| el.child(div().text_xs().child(hint)))
                        .child(buttons),
                )
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
        // The project keeps its name while another of its roots is active.
        let project = model
            .project_root()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .or_else(|| model.repository().map(|r| r.name()))
            .unwrap_or_else(|| "No Project".into());
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
                        .tooltip("Main Menu")
                        .dropdown_menu({
                            let entity = entity.clone();
                            let focus = self.focus.clone();
                            move |menu, window, cx| main_menu(menu, entity.clone(), focus.clone(), window, cx)
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
                .child(tool_button("tb-update", IconName::ArrowDownToLine, if cfg!(target_os = "macos") { "Update Project…  ⌘T" } else { "Update Project…  Ctrl+T" }).on_click(cx.listener(
                    |this, _, window, cx| dialogs::update_project(this.model.clone(), window, cx),
                )))
                .child(tool_button("tb-commit", IconName::Check, if cfg!(target_os = "macos") { "Commit…  ⌘K" } else { "Commit…  Ctrl+K" }).on_click(cx.listener(
                    |this, _, window, cx| this.on_commit(&CommitChanges, window, cx),
                )))
                .child(tool_button("tb-push", IconName::ArrowUpFromLine, if cfg!(target_os = "macos") { "Push…  ⇧⌘K" } else { "Push…  Ctrl+Shift+K" }).on_click(cx.listener(
                    |this, _, window, cx| dialogs::push(this.model.clone(), window, cx),
                )))
                .child(tool_button("tb-fetch", IconName::CloudDownload, "Fetch").on_click(op(
                    "Fetch",
                    &["fetch", "--all", "--prune"],
                    "Fetched all remotes",
                )))
                // The new UI's right corner: Search Everywhere and Settings.
                .child(div().w(px(1.)).h(px(16.)).mx_1().bg(palette.border))
                .child(tool_button("tb-search", IconName::Search, "Search Everywhere  Double Shift").on_click(cx.listener(
                    |this, _, window, cx| this.open_search_everywhere(SeTab::All, window, cx),
                )))
                .child(tool_button("tb-settings", IconName::Settings, if cfg!(target_os = "macos") { "Settings…  ⌘," } else { "Settings…  Ctrl+Alt+S" }).on_click(
                    cx.listener(|this, _, window, cx| crate::ui::settings_dialog::open(Some(this.model.clone()), window, cx)),
                )),
        )
    }

    /// The open tool window that holds the focus.
    fn focused_tool(&self, window: &Window, cx: &App) -> Option<ToolWindow> {
        ToolWindow::ALL
            .into_iter()
            .filter(|w| self.tools.is_open(*w))
            .find(|w| self.tool_focus.get(w).is_some_and(|f| f.contains_focused(window, cx)))
    }

    /// Shift+Escape: hides the active tool window, the one with the focus
    /// (else the one activated last), and gives the focus back.
    fn hide_active_tool_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tool = self
            .focused_tool(window, cx)
            .or(self.last_tool.filter(|w| self.tools.is_open(*w)))
            .or_else(|| [Side::Bottom, Side::Left, Side::Right].into_iter().find_map(|s| self.tools.active(s)));
        if let Some(tool) = tool {
            self.tools.hide(tool);
            self.last_tool = None;
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    /// Opens or hides a tool window from its stripe button or shortcut; an
    /// opened window takes the focus, as IntelliJ activates it.
    fn toggle_tool(&mut self, tool: ToolWindow, window: &mut Window, cx: &mut Context<Self>) {
        self.tools.toggle(tool);
        if tool == ToolWindow::PullRequests && self.tools.is_open(tool) {
            self.prs.update(cx, |prs, cx| prs.refresh(cx));
        }
        if tool == ToolWindow::Notifications {
            self.unread_notifications = 0;
        }
        if self.tools.is_open(tool) {
            self.focus_tool(tool, window, cx);
        } else {
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    /// Moves the focus into a tool window: the Log's table for Git, else the
    /// window itself.
    fn focus_tool(&mut self, tool: ToolWindow, window: &mut Window, cx: &mut Context<Self>) {
        self.last_tool = Some(tool);
        let handle = match tool {
            ToolWindow::Git if self.bottom_tab == BottomTab::Log => gpui_kit::Focusable::focus_handle(self.log.read(cx), cx),
            ToolWindow::Project => gpui_kit::Focusable::focus_handle(self.project.read(cx), cx),
            _ => match self.tool_focus.get(&tool) {
                Some(handle) => handle.clone(),
                None => return,
            },
        };
        window.focus(&handle, cx);
    }

    fn tool_window_info(&self, window: ToolWindow, cx: &App) -> (IconName, SharedString, &'static str) {
        match window {
            ToolWindow::Project => (IconName::FolderTree, "Project".into(), "Alt+1"),
            ToolWindow::Commit => (IconName::GitCommitVertical, "Commit".into(), "Alt+0"),
            ToolWindow::PullRequests => (IconName::GitPullRequest, self.prs.read(cx).title().into(), ""),
            ToolWindow::Changes => (IconName::FileDiff, "Changes".into(), ""),
            ToolWindow::Git => (IconName::GitGraph, "Git".into(), "Alt+9"),
            ToolWindow::Notifications => {
                (if self.unread_notifications > 0 { IconName::BellDot } else { IconName::Bell }, "Notifications".into(), "")
            }
        }
    }

    /// One stripe button: click toggles the window, drag moves it (drop on
    /// another button to go before it), right-click offers Move To and Hide.
    fn stripe_button(&self, window: ToolWindow, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let (icon, title, shortcut) = self.tool_window_info(window, cx);
        let tooltip: SharedString = if shortcut.is_empty() {
            title.clone()
        } else {
            format!("{title} ({})", crate::ui::file_menus::shortcut_label(shortcut)).into()
        };
        let side = self.tools.side(window);
        let entity = cx.entity();
        div()
            .id(SharedString::from(format!("stripe-{window:?}")))
            .on_drag(DraggedToolWindow(window, icon), |dragged, _, _, cx| cx.new(|_| *dragged))
            .drag_over::<DraggedToolWindow>(move |el, _, _, _| el.border_t_2().border_color(palette.accent))
            .on_drop(cx.listener(move |this, dragged: &DraggedToolWindow, _, cx| {
                this.tools.move_to(dragged.0, side, Some(window), cx);
                cx.notify();
            }))
            .child(
                Button::new(SharedString::from(format!("stripe-button-{window:?}")))
                    .ghost()
                    .icon(Icon::new(icon))
                    .tooltip(tooltip)
                    .when(self.tools.is_open(window), |b| b.selected(true))
                    .on_click(cx.listener(move |this, _, w, cx| this.toggle_tool(window, w, cx))),
            )
            .context_menu(move |menu, _, _| {
                let mut menu = menu.label(title.clone());
                for target in Side::ALL {
                    let entity = entity.clone();
                    menu = menu.item(
                        PopupMenuItem::new(format!("Move to {}", target.label()))
                            .disabled(target == side)
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.tools.move_to(window, target, None, cx);
                                    cx.notify();
                                })
                            }),
                    );
                }
                let entity = entity.clone();
                menu.separator().item(PopupMenuItem::new("Hide").on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.tools.hide(window);
                        cx.notify();
                    })
                }))
            })
    }

    /// The left stripe holds the left windows, then the bottom ones at its
    /// foot, as in the new UI; the right stripe holds the right windows.
    fn render_stripe(&self, right: bool, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let palette = cx.palette().clone();
        // Non-modal commit off: no Commit tool window, as in IntelliJ.
        let non_modal = Settings::get(cx).non_modal_commit;
        let visible = |w: &ToolWindow| match w {
            ToolWindow::Changes => !self.changes.read(cx).is_empty(),
            ToolWindow::Commit => non_modal,
            _ => true,
        };
        let top: Vec<ToolWindow> = self.tools.on_side(if right { Side::Right } else { Side::Left }).into_iter().filter(visible).collect();
        let bottom: Vec<ToolWindow> = if right { Vec::new() } else { self.tools.on_side(Side::Bottom).into_iter().filter(visible).collect() };
        if right && top.is_empty() && !cx.has_active_drag() {
            return None;
        }
        let compact = Settings::get(cx).compact;
        let drop_zone = |id: &'static str, side: Side, cx: &mut Context<Self>| {
            div()
                .id(id)
                .flex_1()
                .w_full()
                .min_h(px(24.))
                .drag_over::<DraggedToolWindow>(move |el, _, _, _| el.bg(palette.selection))
                .on_drop(cx.listener(move |this, dragged: &DraggedToolWindow, _, cx| {
                    this.tools.move_to(dragged.0, side, None, cx);
                    cx.notify();
                }))
        };
        let mut stripe = v_flex()
            .w(px(if compact { 32. } else { 40. }))
            .h_full()
            .py_1()
            .gap_1()
            .items_center()
            .when(right, |el| el.border_l_1())
            .when(!right, |el| el.border_r_1())
            .border_color(palette.border)
            .bg(palette.toolbar);
        for window in top {
            stripe = stripe.child(self.stripe_button(window, cx));
        }
        if right {
            stripe = stripe.child(drop_zone("stripe-drop-right", Side::Right, cx));
        } else {
            stripe = stripe
                .child(drop_zone("stripe-drop-left", Side::Left, cx))
                .child(div().w(px(20.)).h(px(1.)).bg(palette.border))
                .child(drop_zone("stripe-drop-bottom", Side::Bottom, cx).flex_none().h(px(24.)));
            for window in bottom {
                stripe = stripe.child(self.stripe_button(window, cx));
            }
        }
        Some(stripe)
    }

    /// The Notifications tool window: every balloon of this session, newest first.
    fn render_notifications(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mut list = v_flex().id("notification-list").flex_1().min_h_0().overflow_y_scroll().p_2().gap_2();
        if self.notifications.is_empty() {
            list = list.child(div().pt_8().w_full().text_center().text_sm().text_color(palette.text_secondary).child("No notifications"));
        }
        for record in &self.notifications {
            list = list.child(
                v_flex()
                    .gap_0p5()
                    .p_2()
                    .rounded_md()
                    .border_1()
                    .border_color(palette.border)
                    .text_sm()
                    .child(
                        h_flex()
                            .gap_1()
                            .child(Icon::new(if record.error { IconName::CircleX } else { IconName::CircleCheck }).xsmall().text_color(
                                if record.error { palette.status_conflict } else { palette.status_added },
                            ))
                            .child(div().flex_1().font_weight(FontWeight::SEMIBOLD).child(record.title.clone()))
                            .child(div().text_xs().text_color(palette.text_secondary).child(record.time.format("%H:%M").to_string())),
                    )
                    .child(div().text_color(palette.text_secondary).child(record.message.lines().take(6).collect::<Vec<_>>().join("\n"))),
            );
        }
        v_flex()
            .size_full()
            .bg(palette.panel)
            .child(
                h_flex()
                    .h(px(crate::ui::common::toolbar_height()))
                    .px_2()
                    .gap_1()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(div().flex_1().text_sm().font_weight(FontWeight::SEMIBOLD).child("Notifications"))
                    .child(tool_button("notifications-clear", IconName::Delete, "Clear All").on_click(cx.listener(|this, _, _, cx| {
                        this.notifications.clear();
                        cx.notify();
                    })))
                    .child(tool_button("notifications-hide", IconName::Minus, "Hide").on_click(cx.listener(|this, _, _, cx| {
                        this.tools.hide(ToolWindow::Notifications);
                        cx.notify();
                    }))),
            )
            .child(list)
    }

    /// The content of a tool window, shown in the panel of its side.
    fn tool_window_content(&self, window: ToolWindow, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        // Out of the flow (absolute), wide content (a long filter label, a
        // narrow side panel) is clipped inside the panel instead of pushing
        // the stripes and the other panels out of the window.
        div()
            .id(SharedString::from(format!("tool-window-{window:?}")))
            .relative()
            .size_full()
            .when_some(self.tool_focus.get(&window), |el, focus| el.track_focus(focus))
            .child(div().absolute().inset_0().overflow_hidden().child(self.tool_window_view(window, cx)))
            .into_any_element()
    }

    fn tool_window_view(&self, window: ToolWindow, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        match window {
            ToolWindow::Project => self.project.clone().into_any_element(),
            ToolWindow::Commit => self.render_left(cx).into_any_element(),
            ToolWindow::PullRequests => self.prs.clone().into_any_element(),
            ToolWindow::Changes => self.changes.clone().into_any_element(),
            ToolWindow::Git => self.render_bottom(cx).into_any_element(),
            ToolWindow::Notifications => self.render_notifications(cx).into_any_element(),
        }
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
                    .h(px(crate::ui::common::header_height()))
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
                        this.tools.hide(ToolWindow::Commit);
                        cx.notify();
                    }))),
            )
            .child(div().flex_1().min_h_0().map(|el| match current {
                LeftTab::Commit => el.child(self.commit.clone()),
                LeftTab::Stash => el.child(self.stash.clone()),
                LeftTab::Shelf => el.child(self.shelf.clone()),
            }))
    }

    /// Shows a tab of the Commit tool window. With non-modal commit off
    /// there is no Commit tool window: Commit opens the Commit Changes
    /// dialog and Shelf and Stash are tabs of the Git tool window.
    fn show_left_tab(&mut self, tab: LeftTab, window: &mut Window, cx: &mut Context<Self>) {
        if Settings::get(cx).non_modal_commit {
            self.tools.open(ToolWindow::Commit);
            self.left_tab = tab;
        } else if tab == LeftTab::Commit {
            return self.open_modal_commit(window, cx);
        } else {
            self.tools.open(ToolWindow::Git);
            self.bottom_tab = if tab == LeftTab::Shelf { BottomTab::Shelf } else { BottomTab::Stash };
        }
        cx.notify();
    }

    /// Alt+0: the Commit tool window, or (non-modal commit off) the Git
    /// tool window's Local Changes tab.
    fn toggle_commit_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if Settings::get(cx).non_modal_commit {
            return self.toggle_tool(ToolWindow::Commit, window, cx);
        }
        if self.tools.is_open(ToolWindow::Git) && self.bottom_tab == BottomTab::LocalChanges {
            self.tools.hide(ToolWindow::Git);
        } else {
            self.tools.open(ToolWindow::Git);
            self.bottom_tab = BottomTab::LocalChanges;
        }
        cx.notify();
    }

    fn on_commit(&mut self, _: &CommitChanges, window: &mut Window, cx: &mut Context<Self>) {
        if !Settings::get(cx).non_modal_commit {
            return self.open_modal_commit(window, cx);
        }
        self.tools.open(ToolWindow::Commit);
        self.left_tab = LeftTab::Commit;
        self.commit.update(cx, |commit, cx| commit.focus_message(window, cx));
        cx.notify();
    }

    /// Settings › Commit › "Use non-modal commit interface" off: Commit
    /// opens IntelliJ's modal Commit Changes dialog (the changes, the
    /// message and the commit buttons) instead of the tool window.
    fn open_modal_commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_commit.get() {
            return;
        }
        // The view is shown in one place at a time.
        self.tools.hide(ToolWindow::Commit);
        self.modal_commit.set(true);
        self.commit.update(cx, |commit, cx| commit.set_in_dialog(true, cx));
        let (commit, open) = (self.commit.clone(), self.modal_commit.clone());
        window.open_dialog(cx, move |dialog, _, _| {
            let (open, commit_view) = (open.clone(), commit.clone());
            dialog
                .title("Commit Changes")
                .w(px(760.))
                .child(div().h(px(560.)).child(commit.clone()))
                .on_close(move |_, _, cx| {
                    open.set(false);
                    commit_view.update(cx, |commit, cx| commit.set_in_dialog(false, cx));
                })
        });
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
        // Opened from the keyboard, the popover focuses itself when it renders;
        // take the focus back for the search field afterwards.
        let popup = self.branches_popup.clone();
        window.on_next_frame(move |window, cx| popup.update(cx, |popup, cx| popup.focus_search(window, cx)));
        cx.notify();
    }

    fn on_toggle_git(&mut self, _: &ToggleGitWindow, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_tool(ToolWindow::Git, window, cx);
    }

    fn on_refresh(&mut self, _: &Refresh, _: &mut Window, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.reload(cx));
    }

    fn on_stash(&mut self, _: &StashChanges, window: &mut Window, cx: &mut Context<Self>) {
        dialogs::stash(self.model.clone(), window, cx);
    }

    /// IntelliJ's VCS Operations quick list, in order; `None` is a separator.
    fn vcs_operation_list() -> Vec<Option<(&'static str, &'static str, VcsRun)>> {
        let op = |f: fn(&mut Workspace, &mut Window, &mut Context<Workspace>)| -> VcsRun { Rc::new(f) };
        vec![
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
            Some(("New Worktree…", "", op(|this, window, cx| crate::ui::worktree_view::new_worktree(this.model.clone(), window, cx)))),
            Some(("Reset HEAD…", "", op(|this, window, cx| dialogs::reset_to(this.model.clone(), "HEAD".into(), window, cx)))),
            None,
            Some(("Stash Changes…", "", op(|this, window, cx| dialogs::stash(this.model.clone(), window, cx)))),
            Some(("Unstash Changes…", "", op(|this, window, cx| {
                this.show_left_tab(LeftTab::Stash, window, cx);
            }))),
            Some(("Shelve Changes…", "", op(|this, window, cx| this.commit.update(cx, |c, cx| c.shelve(window, cx))))),
            Some(("Unshelve Changes…", "", op(|this, window, cx| {
                this.show_left_tab(LeftTab::Shelf, window, cx);
            }))),
            None,
            Some(("Create Patch…", "", op(|this, window, cx| this.commit.update(cx, |c, cx| c.create_patch(window, cx))))),
            Some(("Apply Patch…", "", op(|this, window, cx| patch_dialogs::apply_patch(this.model.clone(), false, window, cx)))),
            Some(("Apply Patch from Clipboard…", "", op(|this, window, cx| patch_dialogs::apply_patch(this.model.clone(), true, window, cx)))),
            None,
            Some(("Show Git Log", "Alt+9", op(|this, _, cx| {
                this.tools.open(ToolWindow::Git);
                this.bottom_tab = BottomTab::Log;
                cx.notify();
            }))),
            Some(("Settings…", "Ctrl+Alt+S", op(|this, window, cx| crate::ui::settings_dialog::open(Some(this.model.clone()), window, cx)))),
        ]
    }

    /// IntelliJ's VCS Operations quick list, numbered like the original:
    /// 1–9 run an entry at once, ↑ ↓ and Enter pick one.
    fn on_vcs_operations(&mut self, _: &VcsOperations, window: &mut Window, cx: &mut Context<Self>) {
        let items: Rc<Vec<Option<(&'static str, &'static str, VcsRun)>>> = Rc::new(Self::vcs_operation_list());
        let runs: Rc<Vec<VcsRun>> = Rc::new(items.iter().flatten().map(|(_, _, run)| run.clone()).collect());
        let selected = Rc::new(std::cell::Cell::new(0usize));
        let workspace = cx.entity();
        let focus = cx.focus_handle();
        let dialog_focus = focus.clone();
        window.open_dialog(cx, move |dialog, _, cx| {
            let palette = cx.palette().clone();
            let pick = {
                let (runs, workspace) = (runs.clone(), workspace.clone());
                Rc::new(move |n: usize, window: &mut Window, cx: &mut gpui_kit::App| {
                    let Some(run) = runs.get(n).cloned() else { return };
                    window.close_dialog(cx);
                    workspace.update(cx, |this, cx| run(this, window, cx));
                })
            };
            let mut list = v_flex()
                .id("vcs-operations")
                .track_focus(&dialog_focus)
                .gap_px()
                .on_key_down({
                    let (pick, selected, count) = (pick.clone(), selected.clone(), runs.len());
                    move |e: &gpui_kit::KeyDownEvent, window, cx| {
                        let key = e.keystroke.key.as_str();
                        match key {
                            "up" => selected.set((selected.get() + count - 1) % count.max(1)),
                            "down" => selected.set((selected.get() + 1) % count.max(1)),
                            "enter" => pick(selected.get(), window, cx),
                            _ => match key.parse::<usize>() {
                                Ok(n @ 1..=9) if !e.keystroke.modifiers.modified() => pick(n - 1, window, cx),
                                _ => return,
                            },
                        }
                        cx.stop_propagation();
                        window.refresh();
                    }
                });
            let mut number = 0;
            for (ix, item) in items.iter().enumerate() {
                let Some((label, shortcut, _)) = item else {
                    list = list.child(div().my_1().h(px(1.)).bg(palette.border));
                    continue;
                };
                let n = number;
                number += 1;
                let pick = pick.clone();
                list = list.child(
                    h_flex()
                        .id(("vcs-op", ix))
                        .h(px(26.))
                        .px_2()
                        .gap_2()
                        .rounded(px(4.))
                        .text_sm()
                        .cursor_pointer()
                        .when(n == selected.get(), |el| el.bg(palette.selection))
                        .hover(|s| s.bg(palette.hover))
                        .on_click(move |_, window, cx| pick(n, window, cx))
                        .child(div().w(px(16.)).text_color(palette.text_secondary).child(if n < 9 { (n + 1).to_string() } else { String::new() }))
                        .child(div().flex_1().child(*label))
                        .child(div().text_xs().text_color(palette.text_secondary).child(crate::ui::file_menus::shortcut_label(shortcut))),
                );
            }
            dialog.title("VCS Operations").w(px(340.)).child(list)
        });
        window.focus(&focus, cx);
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
        let non_modal = Settings::get(cx).non_modal_commit;
        let has_submodules = self.model.read(cx).repository().is_some_and(|r| r.root().join(".gitmodules").exists());
        v_flex()
            .size_full()
            .bg(palette.panel)
            .child(
                h_flex()
                    .h(px(crate::ui::common::header_height()))
                    .px_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Git"))
                    .when(!non_modal, |el| {
                        el.child(tab("tab-local-changes", "Local Changes", BottomTab::LocalChanges, current).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.bottom_tab = BottomTab::LocalChanges;
                                cx.notify();
                            },
                        )).child("Local Changes"))
                    })
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
                    .when(!non_modal, |el| {
                        el.child(tab("tab-shelf", "Shelf", BottomTab::Shelf, current).on_click(cx.listener(|this, _, _, cx| {
                            this.bottom_tab = BottomTab::Shelf;
                            cx.notify();
                        })).child("Shelf"))
                        .child(tab("tab-stash", "Stash", BottomTab::Stash, current).on_click(cx.listener(|this, _, _, cx| {
                            this.bottom_tab = BottomTab::Stash;
                            cx.notify();
                        })).child("Stash"))
                    })
                    .child(tab("tab-worktrees", "Worktrees", BottomTab::Worktrees, current).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.bottom_tab = BottomTab::Worktrees;
                            cx.notify();
                        },
                    )).child("Worktrees"))
                    .when(has_submodules, |el| {
                        el.child(tab("tab-submodules", "Submodules", BottomTab::Submodules, current).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.bottom_tab = BottomTab::Submodules;
                                cx.notify();
                            },
                        )).child("Submodules"))
                    })
                    .child(tab("tab-find", "Find", BottomTab::Find, current).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.bottom_tab = BottomTab::Find;
                            cx.notify();
                        },
                    )).child("Find"))
                    .child(tab("tab-console", "Console", BottomTab::Console, current).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.bottom_tab = BottomTab::Console;
                            cx.notify();
                        },
                    )).child("Console"))
                    .child(div().flex_1())
                    .child(tool_button("git-hide", IconName::Minus, "Hide").on_click(cx.listener(|this, _, _, cx| {
                        this.tools.hide(ToolWindow::Git);
                        cx.notify();
                    }))),
            )
            .child(div().flex_1().min_h_0().map(|el| match current {
                // The view is shown in one place at a time: the dialog has it while open.
                BottomTab::LocalChanges if self.modal_commit.get() => el.child(
                    div().size_full().flex().items_center().justify_center().text_sm().text_color(palette.text_secondary).child("Shown in the Commit Changes dialog"),
                ),
                BottomTab::LocalChanges => el.child(self.commit.clone()),
                BottomTab::Shelf => el.child(self.shelf.clone()),
                BottomTab::Stash => el.child(self.stash.clone()),
                BottomTab::Log => el.child(self.log.clone()),
                BottomTab::Worktrees => el.child(self.worktrees.clone()),
                BottomTab::Submodules => el.child(self.submodules.clone()),
                BottomTab::Find => el.child(self.find.clone()),
                BottomTab::Console => el.child(self.render_console(cx)),
            }))
    }

    /// The Git console, as IntelliJ's: "14:05:02.123: [root] git …" per
    /// command with its output below, failures in red; a side toolbar with
    /// Soft-Wrap, Scroll to the End and Clear All.
    fn render_console(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let console = self.model.read(cx).console().clone();
        let entries = console.entries();
        let mono = cx.theme().mono_font_family.clone();
        let wrap = self.console_wrap;
        let mut list = v_flex().p_2().gap_0p5().font_family(mono).text_size(px(12.));
        for entry in entries.iter().rev().take(500).rev() {
            let line = format!("{}: [{}] {}", entry.time.format("%H:%M:%S%.3f"), entry.root, entry.command_line);
            list = list
                .child(
                    div()
                        .text_color(if entry.success { palette.text } else { palette.status_conflict })
                        .when(!wrap, |el| el.whitespace_nowrap())
                        .child(line),
                )
                .when(!entry.output.is_empty(), |el| {
                    el.child(
                        div()
                            .text_color(if entry.success { palette.text_secondary } else { palette.status_conflict })
                            .when(wrap, |el| el.whitespace_normal())
                            .when(!wrap, |el| el.whitespace_nowrap())
                            .child(entry.output.clone()),
                    )
                });
        }
        // Scroll to the End: follow new commands while it is on.
        if self.console_autoscroll && entries.len() != self.console_seen.get() {
            self.console_seen.set(entries.len());
            self.console_scroll.scroll_to_bottom();
        }
        let toggle = |id: &'static str, icon: IconName, tip: &'static str, on: bool| {
            tool_button(id, icon, tip).when(on, |b| b.selected(true))
        };
        h_flex()
            .size_full()
            .child(
                v_flex()
                    .h_full()
                    .w(px(28.))
                    .flex_shrink_0()
                    .py_1()
                    .gap_0p5()
                    .items_center()
                    .border_r_1()
                    .border_color(palette.border)
                    .child(toggle("console-wrap", IconName::TextWrap, "Soft-Wrap", wrap).on_click(cx.listener(|this, _, _, cx| {
                        this.console_wrap = !this.console_wrap;
                        cx.notify();
                    })))
                    .child(
                        toggle("console-end", IconName::ArrowDownToLine, "Scroll to the End", self.console_autoscroll).on_click(cx.listener(|this, _, _, cx| {
                            this.console_autoscroll = !this.console_autoscroll;
                            if this.console_autoscroll {
                                this.console_scroll.scroll_to_bottom();
                            }
                            cx.notify();
                        })),
                    )
                    .child(tool_button("console-clear", IconName::Delete, "Clear All").on_click(cx.listener(move |this, _, _, cx| {
                        console.clear();
                        this.console_seen.set(0);
                        cx.notify();
                    }))),
            )
            .child(
                div()
                    .id("git-console")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .track_scroll(&self.console_scroll)
                    .overflow_y_scroll()
                    .when(!wrap, |el| el.overflow_x_scroll())
                    .child(list),
            )
    }

    /// IntelliJ's status bar: the file's path on the left; background task,
    /// caret, line separator, encoding, indent and the branch on the right.
    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let model = self.model.read(cx);
        let project = model.project_root().or_else(|| model.repository().map(|r| r.root())).and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
        let file = self.editor().map(|e| e.read(cx).path().to_owned());
        let branch = branches_popup::branch_widget_label(model);
        let task = model.busy().map(|b| format!("{b}…")).or_else(|| model.is_loading().then(|| "Refreshing VCS history…".to_owned()));
        let index = self.code_index.read(cx).summary();
        let compact = Settings::get(cx).compact;
        // Breadcrumbs: project › folders › file.
        let mut crumbs: Vec<String> = project.into_iter().collect();
        if let Some(file) = &file {
            // A library file: External Libraries › library › path inside it.
            let library = crate::index::store::ProjectIndex::is_external(file)
                .then(|| {
                    let index = self.code_index.read(cx).index.read().ok()?;
                    let library = index.external.library_of(file)?;
                    let rel = std::path::Path::new(file).strip_prefix(&library.root).ok()?.to_string_lossy().replace('\\', "/");
                    Some((library.name.clone(), rel))
                })
                .flatten();
            match library {
                Some((name, rel)) => {
                    crumbs = vec!["External Libraries".to_owned(), name];
                    crumbs.extend(rel.split('/').map(str::to_owned));
                }
                None => crumbs.extend(file.split(['/', '\\']).filter(|p| !p.is_empty()).map(str::to_owned)),
            }
        }
        let crumb_count = crumbs.len();
        let mut path = h_flex().gap_0p5().min_w_0().overflow_hidden();
        for (ix, crumb) in crumbs.into_iter().enumerate() {
            if ix > 0 {
                path = path.child(Icon::new(IconName::ChevronRight).xsmall());
            }
            path = path.child(div().when(ix + 1 == crumb_count && file.is_some(), |el| el.text_color(palette.text)).child(crumb));
        }
        h_flex()
            .h(px(if compact { 20. } else { 24. }))
            .px_3()
            .gap_3()
            .border_t_1()
            .border_color(palette.border)
            .bg(palette.toolbar)
            .text_xs()
            .text_color(palette.text_secondary)
            .child(path)
            .child(div().flex_1())
            .when_some(task, |el, task| {
                el.child(
                    h_flex()
                        .gap_1()
                        .child(Icon::new(IconName::LoaderCircle).xsmall().text_color(palette.accent))
                        .child(task),
                )
            })
            .child(index)
            .child(self.caret.clone())
            .child(
                div()
                    .id("status-branch")
                    .px_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .hover(|s| s.bg(palette.hover))
                    .child(h_flex().gap_1().child(Icon::new(IconName::GitBranch).xsmall()).child(branch))
                    .on_click(cx.listener(|this, _, window, cx| this.open_branches(window, cx))),
            )
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

/// The new UI's main menu (☰), grouped as IntelliJ's menu bar: File, View,
/// Navigate, Git, Window, Help. Items backed by actions show their shortcut.
fn main_menu(
    menu: gpui_kit::component::menu::PopupMenu,
    entity: Entity<Workspace>,
    focus: gpui_kit::FocusHandle,
    window: &mut Window,
    cx: &mut Context<gpui_kit::component::menu::PopupMenu>,
) -> gpui_kit::component::menu::PopupMenu {
    let on = |entity: &Entity<Workspace>, f: fn(&mut Workspace, &mut Window, &mut Context<Workspace>)| {
        let entity = entity.clone();
        move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| entity.update(cx, |this, cx| f(this, window, cx))
    };
    let model_op = |entity: &Entity<Workspace>, f: fn(Entity<RepoModel>, &mut Window, &mut App)| {
        let entity = entity.clone();
        move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| {
            let model = entity.read(cx).model.clone();
            f(model, window, cx)
        }
    };
    let e = entity.clone();
    let f = focus.clone();
    let menu = menu.action_context(focus.clone()).submenu("File", window, cx, move |menu, window, cx| {
        let e2 = e.clone();
        menu.action_context(f.clone())
            .item(PopupMenuItem::new("Open…").on_click(on(&e, |this, window, cx| this.open_repository(window, cx))))
            .item(PopupMenuItem::new("Get from Version Control…").on_click(model_op(&e, clone_dialog::clone)))
            .item(PopupMenuItem::new("Create Git Repository…").on_click(model_op(&e, |model, _, cx| clone_dialog::init(model, cx))))
            .submenu("Recent Projects", window, cx, move |mut menu, _, cx| {
                let current = e2.read(cx).model.read(cx).repository().map(|r| r.root().to_path_buf());
                let recent = crate::settings::recent_projects();
                if recent.is_empty() {
                    return menu.item(PopupMenuItem::new("No recent projects").disabled(true));
                }
                for path in recent.into_iter().take(15) {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let entity = e2.clone();
                    let is_current = current.as_deref() == Some(path.as_path());
                    menu = menu.item(PopupMenuItem::new(name).checked(is_current).on_click(move |_, _, cx| {
                        let path = path.clone();
                        let model = entity.read(cx).model.clone();
                        model.update(cx, |m, cx| m.open(path, cx));
                    }));
                }
                menu
            })
            .separator()
            .menu("Settings…", Box::new(OpenSettings))
            .separator()
            .item(PopupMenuItem::new("Exit").on_click(|_, _, cx| cx.quit()))
    });
    let (e, f) = (entity.clone(), focus.clone());
    let menu = menu.submenu("View", window, cx, move |menu, window, cx| {
        let f2 = f.clone();
        let e2 = e.clone();
        let prs_title = e.read(cx).prs.read(cx).title();
        menu.action_context(f.clone())
            .submenu("Tool Windows", window, cx, move |menu, _, _| {
                let e3 = e2.clone();
                menu.action_context(f2.clone())
                    .menu("Project", Box::new(ToggleProjectWindow))
                    .menu("Commit", Box::new(ToggleCommitWindow))
                    .menu("Git", Box::new(ToggleGitWindow))
                    .menu("Find", Box::new(ToggleFindWindow))
                    .item(PopupMenuItem::new(prs_title).on_click(move |_, window, cx| {
                        e3.update(cx, |this, cx| this.toggle_tool(ToolWindow::PullRequests, window, cx))
                    }))
            })
            .menu("Hide All Tool Windows", Box::new(HideAllToolWindows))
            .separator()
            .submenu("Appearance", window, cx, |menu, _, cx| {
                let settings = Settings::get(cx).clone();
                let theme = |label: &'static str, dark: bool, system: bool, checked: bool| {
                    PopupMenuItem::new(label).checked(checked).on_click(move |_, window, cx| {
                        Settings::update(cx, |s| {
                            s.theme_follows_system = system;
                            if !system {
                                s.dark = dark;
                            }
                        });
                        theme::refresh(cx);
                        window.refresh();
                    })
                };
                menu.label("Theme")
                    .item(theme("Dark", true, false, !settings.theme_follows_system && settings.dark))
                    .item(theme("Light", false, false, !settings.theme_follows_system && !settings.dark))
                    .item(theme("Sync with OS", settings.dark, true, settings.theme_follows_system))
                    .separator()
                    .item(PopupMenuItem::new("Compact Mode").checked(settings.compact).on_click(|_, window, cx| {
                        Settings::update(cx, |s| s.compact = !s.compact);
                        window.refresh();
                    }))
            })
    });
    let f = focus.clone();
    let menu = menu.submenu("Navigate", window, cx, move |menu, _, _| {
        menu.action_context(f.clone())
            .menu("Search Everywhere", Box::new(SearchEverywhere))
            .menu("Find Action…", Box::new(FindAction))
            .separator()
            .menu("Class…", Box::new(GotoClass))
            .menu("File…", Box::new(GotoFile))
            .menu("Symbol…", Box::new(GotoSymbol))
            .menu("Line/Column…", Box::new(GotoLine))
            .separator()
            .menu("Recent Files", Box::new(RecentFiles))
            .menu("File Structure", Box::new(FileStructure))
            .separator()
            .menu("Back", Box::new(NavigateBack))
            .menu("Forward", Box::new(NavigateForward))
            .menu("Select in Project View", Box::new(SelectInProject))
            .separator()
            .menu("Find in Files…", Box::new(FindInPath))
            .menu("Replace in Files…", Box::new(ReplaceInPath))
    });
    let (e, f) = (entity.clone(), focus.clone());
    let menu = menu.submenu("Git", window, cx, move |menu, window, cx| {
        let e2 = e.clone();
        let e3 = e.clone();
        let f2 = f.clone();
        menu.action_context(f.clone())
            .menu("Commit…", Box::new(CommitChanges))
            .menu("Push…", Box::new(PushChanges))
            .menu("Update Project…", Box::new(UpdateProject))
            .item(PopupMenuItem::new("Pull…").on_click(model_op(&e, remote_dialogs::pull)))
            .item(PopupMenuItem::new("Fetch").on_click(model_op(&e, |model, _, cx| {
                model.update(cx, |model, cx| {
                    model.run_operation("Fetch", |repo| {
                        repo.run(&["fetch", "--all", "--prune"])?;
                        Ok("Fetched all remotes".to_owned())
                    }, cx)
                })
            })))
            .separator()
            .item(PopupMenuItem::new("Merge…").on_click(model_op(&e, dialogs::merge)))
            .item(PopupMenuItem::new("Rebase…").on_click(model_op(&e, dialogs::rebase)))
            .menu("Branches…", Box::new(ShowBranches))
            .item(PopupMenuItem::new("New Branch…").on_click(model_op(&e, |model, window, cx| dialogs::new_branch(model, "HEAD".into(), window, cx))))
            .item(PopupMenuItem::new("New Tag…").on_click(model_op(&e, |model, window, cx| dialogs::new_tag(model, "HEAD".into(), window, cx))))
            .item(PopupMenuItem::new("Reset HEAD…").on_click(model_op(&e, |model, window, cx| dialogs::reset_to(model, "HEAD".into(), window, cx))))
            .separator()
            // IntelliJ's Git › Current File, for the file in the editor.
            .submenu("Current File", window, cx, move |menu, _, cx| {
                let no_file = e3.read(cx).editor().is_none();
                menu.item(PopupMenuItem::new("Annotate with Git Blame").disabled(no_file).on_click(on(&e3, |this, _, cx| {
                    if let Some(editor) = this.editor().cloned() {
                        editor.update(cx, |editor, cx| editor.show_annotations(cx));
                    }
                })))
                .item(PopupMenuItem::new("Show History").disabled(no_file).on_click(on(&e3, |this, _, cx| {
                    if let Some(path) = this.editor().map(|e| e.read(cx).path().to_owned()) {
                        this.show_history(path, cx);
                    }
                })))
                .item(PopupMenuItem::new("Show Diff").disabled(no_file).on_click(on(&e3, |this, _, cx| {
                    if let Some(path) = this.editor().map(|e| e.read(cx).path().to_owned()) {
                        this.open_diff(crate::ui::diff_view::DiffSource::WorkingTree { path, unversioned: false }, cx);
                    }
                })))
            })
            .separator()
            .menu("Stash Changes…", Box::new(StashChanges))
            .item(PopupMenuItem::new("Unstash Changes…").on_click(on(&e, |this, window, cx| {
                this.show_left_tab(LeftTab::Stash, window, cx);
            })))
            .separator()
            .item(PopupMenuItem::new("Create Patch…").on_click(on(&e, |this, window, cx| {
                let commit = this.commit.clone();
                commit.update(cx, |c, cx| c.create_patch(window, cx))
            })))
            .item(PopupMenuItem::new("Apply Patch…").on_click(model_op(&e, |model, window, cx| patch_dialogs::apply_patch(model, false, window, cx))))
            .item(PopupMenuItem::new("Apply Patch from Clipboard…").on_click(model_op(&e, |model, window, cx| patch_dialogs::apply_patch(model, true, window, cx))))
            .separator()
            .item(PopupMenuItem::new("Manage Remotes…").on_click(model_op(&e, remote_dialogs::manage_remotes)))
            .item(PopupMenuItem::new("New Worktree…").on_click(model_op(&e, crate::ui::worktree_view::new_worktree)))
            .item(PopupMenuItem::new("Update Submodules").on_click(model_op(&e, |model, _, cx| {
                model.update(cx, |m, cx| {
                    m.run_operation("Update Submodules", |repo| {
                        crate::git::submodule::update(repo, &[])?;
                        Ok("Submodules updated".into())
                    }, cx)
                })
            })))
            .item(PopupMenuItem::new("Directory Mappings…").on_click(model_op(&e, crate::ui::mappings_dialog::directory_mappings)))
            .separator()
            // IntelliJ's Git › GitHub group.
            .submenu("GitHub / GitLab", window, cx, move |menu, _, cx| {
                let e3 = e2.clone();
                let (e4, e5, e6, e7) = (e2.clone(), e2.clone(), e2.clone(), e2.clone());
                let web = e2.read(cx).model.read(cx).web_repo().map(|w| w.base.clone());
                let has_file = e2.read(cx).editor().is_some();
                menu.item(PopupMenuItem::new("Share Project on GitHub…").on_click(model_op(&e2, crate::ui::github_dialogs::share_project)))
                    .item(PopupMenuItem::new("Create Pull Request…").on_click(move |_, window, cx| {
                        let prs = e4.read(cx).prs.clone();
                        prs.update(cx, |prs, cx| prs.create_pull_request(window, cx))
                    }))
                    .item(PopupMenuItem::new("View Pull Requests").on_click(move |_, window, cx| {
                        e5.update(cx, |this, cx| {
                            if !this.tools.is_open(ToolWindow::PullRequests) {
                                this.toggle_tool(ToolWindow::PullRequests, window, cx)
                            }
                        })
                    }))
                    .separator()
                    .item(PopupMenuItem::new("Open on GitHub / GitLab").disabled(web.is_none()).on_click(move |_, window, cx| {
                        // The file in the editor at its lines, else the repository's page.
                        match e6.read(cx).editor().cloned() {
                            Some(editor) => editor.update(cx, |editor, cx| editor.open_on_hosting(&crate::ui::file_editor::OpenOnHosting, window, cx)),
                            None => {
                                if let Some(web) = &web {
                                    cx.open_url(web)
                                }
                            }
                        }
                    }))
                    .item(PopupMenuItem::new("Create Gist…").disabled(!has_file).on_click(move |_, window, cx| {
                        if let Some(editor) = e7.read(cx).editor().cloned() {
                            editor.update(cx, |editor, cx| editor.create_gist(&crate::ui::file_editor::CreateGist, window, cx))
                        }
                    }))
                    .separator()
                    .item(PopupMenuItem::new("Manage Accounts…").on_click(move |_, window, cx| {
                        let prs = e3.read(cx).prs.clone();
                        crate::ui::accounts_dialog::accounts(
                            Some(Rc::new(move |cx: &mut App| prs.update(cx, |prs, cx| prs.refresh(cx)))),
                            window,
                            cx,
                        )
                    }))
            })
            .separator()
            .item(PopupMenuItem::new("VCS Operations…").action(Box::new(VcsOperations)))
            .action_context(f2)
    });
    let f = focus.clone();
    let menu = menu.submenu("Window", window, cx, move |menu, _, _| {
        menu.action_context(f.clone())
            .menu("Select Next Tab", Box::new(NextTab))
            .menu("Select Previous Tab", Box::new(PreviousTab))
            .menu("Close Tab", Box::new(CloseTab))
            .menu("Reopen Closed Tab", Box::new(ReopenClosedTab))
    });
    menu.submenu("Help", window, cx, move |menu, _, _| {
        menu.item(PopupMenuItem::new("Keyboard Shortcuts").on_click(|_, window, cx| dialogs::keymap_reference(window, cx)))
            .item(PopupMenuItem::new(format!("About {APP_NAME}")).on_click(|_, window, cx| dialogs::about(window, cx)))
    })
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
