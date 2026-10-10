//! Git tool window › Log tab: branches panel, filter bar, commit table with
//! graph, and the changes + details pane for the selected commit.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::{
    Disableable as _, Selectable as _, WindowExt as _,
    ActiveTheme as _, Icon, Sizable as _, h_flex, h_resizable,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState},
    list::ListItem,
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem},
    resizable_panel,
    scroll::ScrollableElement as _,
    tree::{TreeItem, TreeState, tree},
    v_flex, v_resizable,
};
use gpui_kit::assets::IconName;
use gpui_kit::{
    AnyElement, App, StatefulInteractiveElement as _, AppContext as _, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Render, ScrollStrategy,
    SharedString, Styled as _, Subscription, Task, UniformListScrollHandle,
    Window, actions, div, prelude::FluentBuilder as _, px, uniform_list,
};

use crate::git::{Commit, FileChangeKind, LogFilter, RefKind, RefName};
use crate::settings::Settings;
use crate::model::{RepoEvent, RepoModel};
use crate::theme::ActivePalette as _;
use crate::ui::common::{self, DIR_PREFIX, FILE_PREFIX, row_height, tool_button};
use crate::ui::diff_view::DiffSource;
use crate::ui::dialogs;
use crate::ui::selectable_text::{SelectableText, selectable_text};
use crate::ui::rebase_dialog;
use crate::ui::graph_paint::graph_canvas;

mod branches;
mod commit_menu;
mod details;
mod filter_bar;
mod go_to;
mod table;

use branches::*;
use commit_menu::*;
use details::*;

actions!(
    git_log,
    [
        SelectPrevious, SelectNext, SelectFirst, SelectLast, SelectPageUp, SelectPageDown, ExtendPrevious, ExtendNext, ExtendFirst,
        ExtendLast, SelectAll, CopyRevision, GoToHash, BranchConfirm
    ]
);

const CONTEXT: &str = "GitLog";
const BRANCHES_CONTEXT: &str = "GitLogBranches";

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", SelectPrevious, Some(CONTEXT)),
        KeyBinding::new("down", SelectNext, Some(CONTEXT)),
        KeyBinding::new("home", SelectFirst, Some(CONTEXT)),
        KeyBinding::new("end", SelectLast, Some(CONTEXT)),
        KeyBinding::new("pageup", SelectPageUp, Some(CONTEXT)),
        KeyBinding::new("pagedown", SelectPageDown, Some(CONTEXT)),
        KeyBinding::new("shift-up", ExtendPrevious, Some(CONTEXT)),
        KeyBinding::new("shift-down", ExtendNext, Some(CONTEXT)),
        KeyBinding::new("shift-home", ExtendFirst, Some(CONTEXT)),
        KeyBinding::new("shift-end", ExtendLast, Some(CONTEXT)),
        KeyBinding::new("secondary-a", SelectAll, Some(CONTEXT)),
        KeyBinding::new("secondary-c", CopyRevision, Some(CONTEXT)),
        KeyBinding::new("secondary-f", GoToHash, Some(CONTEXT)),
        KeyBinding::new("enter", BranchConfirm, Some(BRANCHES_CONTEXT)),
    ]);
}

pub enum LogEvent {
    OpenDiff(DiffSource),
    /// Annotate with Git Blame: a path at a revision (`None` for the work tree).
    Annotate { path: String, revision: Option<String> },
    /// Open Repository Version (`revision`) or Edit Source (`None`).
    OpenFile { path: String, revision: Option<String> },
}

impl EventEmitter<LogEvent> for LogView {}

const BRANCH_PREFIX: &str = "ref:";

pub struct LogView {
    /// The table, branch tree and details, each drawn as a view of its own.
    parts: Vec<Entity<LogPart>>,
    model: Entity<RepoModel>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    search: Entity<InputState>,
    branches: Entity<TreeState>,
    /// Speed search over the branches panel.
    branch_search: Entity<InputState>,
    changes: Entity<TreeState>,
    change_kinds: HashMap<String, (FileChangeKind, Option<String>)>,
    /// Multi-selection: per file, the revisions its merged change spans.
    combined: Option<HashMap<String, (String, String)>>,
    change_counts: HashMap<SharedString, usize>,
    /// Expand All / Collapse All of the changed files.
    changes_expanded: bool,
    last_change_selection: Option<SharedString>,
    last_branch_selection: Option<SharedString>,
    /// A branch selected in the Branches panel whose tip the active filters
    /// hide: (name, hash), for IntelliJ's "does not match active filters"
    /// balloon (shown on the next render, which has the window).
    hidden_branch_tip: Option<(String, String)>,
    /// View and Reset Filters: the commit to select once the log reloads.
    select_after_reload: Option<String>,
    show_branches: bool,
    show_details: bool,
    /// Commits reachable from HEAD among the loaded ones, for the Current
    /// Branch / Not Merged highlighters; keyed by the commit list and HEAD.
    head_reachable: Option<(usize, Option<String>, Rc<Vec<bool>>)>,
    /// Recently used Branch / User / Paths filters, newest first (filter history).
    recent_branch_filters: Vec<Vec<String>>,
    recent_user_filters: Vec<Vec<String>>,
    /// Every author seen in this repository's log, for the User filter:
    /// it stays the same when a filter narrows the list (keyed by root).
    known_authors: (Option<std::path::PathBuf>, std::collections::BTreeSet<String>),
    authors_task: Option<Task<()>>,
    /// The log `known_authors` last took names from.
    authors_from: usize,
    recent_path_filters: Vec<Vec<String>>,
    /// Branches panel › Expand All / Collapse All, until the next toggle.
    branch_tree_expanded: Option<bool>,
    /// Branches panel › Show My Branches: only refs with commits I authored.
    my_branches: bool,
    /// The refs Show My Branches keeps, for the refs they were computed from.
    my_refs: Option<(Vec<(String, String)>, HashSet<String>)>,
    /// The branches tree as last built; its items share their expanded
    /// state with the tree, so a rebuild can keep what the user opened.
    branch_items: Vec<TreeItem>,
    /// Whether `branch_items` are speed search results (all folders open).
    branch_searching: bool,
    /// Open (true) or closed folders of the branches tree, by id.
    branch_expansion: HashMap<SharedString, bool>,
    /// Commits selected besides the model's selected (lead) commit, by
    /// Ctrl/Cmd-click or Shift-click.
    extra_selection: HashSet<String>,
    /// Where a Shift-click range starts.
    anchor: Option<usize>,
    /// A column edge being dragged: the column (author, date, hash), where
    /// the drag started and the width then; the widths while dragging.
    column_drag: Option<(usize, f32, u32)>,
    columns: Option<[u32; 3]>,
    /// The graph with long edges hidden, keyed by the layout it came from.
    /// The details list every branch holding the commit, not only five.
    show_all_branches: bool,
    /// The Git window is docked left or right: details under the table.
    details_below: bool,
    _search_debounce: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl LogView {
    pub fn new(model: Entity<RepoModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Text or hash"));
        let branches = cx.new(|cx| TreeState::new(cx));
        let branch_search = cx.new(|cx| InputState::new(window, cx).placeholder("Search branches"));
        let changes = cx.new(|cx| TreeState::new(cx));
        let subscriptions = vec![
            cx.subscribe(&model, |this, _, event, cx| match event {
                RepoEvent::Reloaded | RepoEvent::LogLoaded => {
                    let commits = this.model.read(cx).commits().clone();
                    let root = this.model.read(cx).repository().map(|r| r.root().to_path_buf());
                    if this.known_authors.0 != root {
                        this.known_authors = (root, Default::default());
                        this.authors_from = 0;
                    }
                    // Reloads that keep the log (after staging, say) skip this.
                    if std::mem::replace(&mut this.authors_from, Arc::as_ptr(&commits) as usize) != Arc::as_ptr(&commits) as usize {
                        // A million commits take a while: gathered off the main thread.
                        let root = this.known_authors.0.clone();
                        this.authors_task = Some(cx.spawn(async move |this, cx| {
                            let names = cx
                                .background_spawn(async move {
                                    // Names are shared between commits: one look per distinct name.
                                    let mut seen = std::collections::HashSet::new();
                                    let mut names = Vec::new();
                                    for commit in commits.iter() {
                                        if seen.insert(Arc::as_ptr(&commit.author_name) as *const u8 as usize) {
                                            names.push(commit.author_name.clone());
                                        }
                                    }
                                    names
                                })
                                .await;
                            this.update(cx, |this, _| {
                                if this.known_authors.0 == root {
                                    this.known_authors.1.extend(names.iter().map(|n| n.to_string()));
                                }
                            })
                            .ok();
                        }));
                    }
                    let model = this.model.read(cx);
                    this.extra_selection.retain(|h| model.row_of(h).is_some());
                    if let Some(hash) = this.select_after_reload.take_if(|hash| model.row_of(hash).is_some()) {
                        this.model.update(cx, |model, cx| model.select_hash(Some(hash), cx));
                    }
                    this.rebuild_branches(cx);
                    cx.notify();
                }
                RepoEvent::SelectionChanged | RepoEvent::DetailsLoaded => {
                    if matches!(event, RepoEvent::SelectionChanged) {
                        this.show_all_branches = false;
                    }
                    this.rebuild_changes(cx);
                    if let Some(ix) = this.model.read(cx).selected_index() {
                        this.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
                    }
                    cx.notify();
                }
                RepoEvent::Notify { .. } | RepoEvent::Compare { .. } | RepoEvent::PrefillCommitMessage(_) | RepoEvent::OpenLogTab { .. } | RepoEvent::ShowConflicts | RepoEvent::Cloned(_) => {}
            }),
            cx.subscribe_in(&search, window, |this, _, event, _, cx| match event {
                InputEvent::Change => {
                    this._search_debounce = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(Duration::from_millis(350)).await;
                        this.update(cx, |this, cx| this.apply_text_filter(cx)).ok();
                    }));
                }
                InputEvent::PressEnter { .. } => this.apply_text_filter(cx),
                _ => {}
            }),
            cx.subscribe(&branch_search, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.rebuild_branches(cx);
                    cx.notify();
                }
            }),
            cx.observe(&changes, |this, changes, cx| {
                let selected = changes.read(cx).selected_item().map(|item| item.id.clone());
                if selected != this.last_change_selection {
                    this.last_change_selection = selected.clone();
                    if let Some(source) = selected.and_then(|id| this.diff_source_for(&id, cx)) {
                        cx.emit(LogEvent::OpenDiff(source));
                    }
                }
            }),
            cx.observe(&branches, |this, branches, cx| {
                let selected = branches.read(cx).selected_item().map(|item| item.id.clone());
                if selected != this.last_branch_selection {
                    this.last_branch_selection = selected.clone();
                    // Selecting a branch navigates the log to its tip. A tip the
                    // filters hide isn't selected behind the list's back: IntelliJ
                    // says it doesn't match and offers to reset the filters.
                    if let Some(full_name) = selected.as_deref().and_then(|id| id.strip_prefix(BRANCH_PREFIX)) {
                        let model = this.model.read(cx);
                        let target = model.refs().find(full_name).map(|r| (r.name.clone(), r.target.clone()));
                        let filtered = *model.filter() != LogFilter { date_order: model.filter().date_order, ..Default::default() };
                        match target {
                            Some((name, hash)) if filtered && model.row_of(&hash).is_none() => {
                                this.hidden_branch_tip = Some((name, hash));
                                cx.notify();
                            }
                            target => this.model.update(cx, |model, cx| model.select_hash(target.map(|(_, hash)| hash), cx)),
                        }
                    }
                }
            }),
        ];
        let mut this = Self {
            model: model.clone(),
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            search,
            branches,
            branch_search,
            changes,
            change_kinds: HashMap::new(),
            combined: None,
            change_counts: HashMap::new(),
            changes_expanded: true,
            last_change_selection: None,
            last_branch_selection: None,
            hidden_branch_tip: None,
            select_after_reload: None,
            show_branches: true,
            show_details: true,
            head_reachable: None,
            recent_branch_filters: Vec::new(),
            recent_user_filters: Vec::new(),
            known_authors: (None, Default::default()),
            authors_task: None,
            authors_from: 0,
            parts: [Part::Table, Part::Branches, Part::Details]
                .into_iter()
                .map(|part| {
                    let log = cx.entity();
                    cx.new(|cx| {
                        // Redrawn whenever the Log or the repository changes.
                        cx.observe(&log, |_, _, cx| cx.notify()).detach();
                        cx.observe(&model, |_, _, cx| cx.notify()).detach();
                        LogPart { log: log.downgrade(), part }
                    })
                })
                .collect(),
            recent_path_filters: Vec::new(),
            branch_tree_expanded: None,
            my_branches: false,
            my_refs: None,
            branch_items: Vec::new(),
            branch_searching: false,
            branch_expansion: HashMap::new(),
            extra_selection: HashSet::new(),
            anchor: None,
            column_drag: None,
            columns: None,
            show_all_branches: false,
            details_below: false,
            _search_debounce: None,
            _subscriptions: subscriptions,
        };
        this.rebuild_branches(cx);
        this
    }

    /// The Git window moved to a side (or back to the bottom).
    pub fn set_details_below(&mut self, below: bool, cx: &mut Context<Self>) {
        if self.details_below != below {
            self.details_below = below;
            cx.notify();
        }
    }

    /// File History in its own Log tab, optionally starting from a commit.
    fn open_history_tab(&mut self, path: String, from: Option<String>, cx: &mut Context<Self>) {
        let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
        // The Log's paths are relative to the project; the commit's are its root's.
        let path = {
            let model = self.model.read(cx);
            let prefix = model.repository().filter(|_| model.is_multi_root()).map(|r| model.prefix_of(r.root())).unwrap_or_default();
            if prefix.is_empty() { path } else { format!("{prefix}/{path}") }
        };
        let filter = LogFilter { paths: vec![path], branches: from.into_iter().collect(), ..Default::default() };
        self.model.update(cx, |m, cx| m.open_log_tab(format!("History: {name}"), filter, cx));
    }
}

/// Above this many selected commits, their changes come from one diff.
const MAX_COMBINED: usize = 100;

/// Git's empty tree, the "parent" of a root commit.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

impl Focusable for LogView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for LogView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some((name, hash)) = self.hidden_branch_tip.take() {
            let entity = cx.entity();
            window.push_notification(
                gpui_kit::component::notification::Notification::warning(format!("{name} does not match active filters")).action(move |_, _, _| {
                    let (entity, hash) = (entity.clone(), hash.clone());
                    Button::new("log-reset-filters").label("View and Reset Filters").small().outline().on_click(move |_, window, cx| {
                        entity.update(cx, |this, cx| this.reset_filters_and_select(hash.clone(), window, cx))
                    })
                }),
                cx,
            );
        }
        let table = v_flex()
            .size_full()
            .child(self.render_filter_bar(cx))
            .children(self.render_compare_banner(cx))
            .child(div().flex_1().min_h_0().child(crate::ui::common::cached(&self.parts[0])));

        let branches = resizable_panel()
            .size(px(if self.details_below { 180. } else { 240. }))
            .size_range(px(120.)..px(500.))
            .visible(self.show_branches)
            .child(crate::ui::common::cached(&self.parts[1]));
        if self.details_below {
            // Docked at a side, the window is tall and narrow: the details go
            // under the table, as IntelliJ lays out a vertical Log.
            return h_resizable("log-split-side")
                .child(branches)
                .child(
                    resizable_panel().child(
                        v_resizable("log-table-details")
                            .child(resizable_panel().child(table))
                            .child(
                                resizable_panel()
                                    .size(px(320.))
                                    .size_range(px(120.)..px(1200.))
                                    .visible(self.show_details)
                                    .child(crate::ui::common::cached(&self.parts[2])),
                            ),
                    ),
                )
                .into_any_element();
        }
        h_resizable("log-split")
            .child(branches)
            .child(resizable_panel().child(table))
            .child(
                resizable_panel()
                    .size(px(380.))
                    .size_range(px(200.)..px(800.))
                    .visible(self.show_details)
                    .child(crate::ui::common::cached(&self.parts[2])),
            )
            .into_any_element()
    }
}


#[derive(Clone, Copy)]
enum Part {
    Table,
    Branches,
    Details,
}

/// A part of the Log drawn as its own view: a menu hovered or a row
/// selected redraws only the parts that render it, the rest is reused.
struct LogPart {
    log: gpui_kit::WeakEntity<LogView>,
    part: Part,
}

impl Render for LogPart {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let part = self.part;
        self.log
            .update(cx, |log, cx| match part {
                Part::Table => log.render_table(cx),
                Part::Branches => log.render_branches(cx).into_any_element(),
                Part::Details => log.render_details(cx).into_any_element(),
            })
            .unwrap_or_else(|_| div().into_any_element())
    }
}
