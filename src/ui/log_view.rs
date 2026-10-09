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
    show_branches: bool,
    show_details: bool,
    /// Commits reachable from HEAD among the loaded ones, for the Current
    /// Branch / Not Merged highlighters; keyed by the commit list and HEAD.
    head_reachable: Option<(usize, Option<String>, Rc<HashSet<String>>)>,
    /// Recently used Branch / User / Paths filters, newest first (filter history).
    recent_branch_filters: Vec<Vec<String>>,
    recent_user_filters: Vec<Vec<String>>,
    /// Every author seen in this repository's log, for the User filter:
    /// it stays the same when a filter narrows the list (keyed by root).
    known_authors: (Option<std::path::PathBuf>, std::collections::BTreeSet<String>),
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
    short_graph: Option<(usize, Arc<crate::git::GraphLayout>)>,
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
                RepoEvent::Reloaded => {
                    let commits = this.model.read(cx).commits().clone();
                    let root = this.model.read(cx).repository().map(|r| r.root().to_path_buf());
                    if this.known_authors.0 != root {
                        this.known_authors = (root, Default::default());
                    }
                    this.known_authors.1.extend(commits.iter().map(|c| c.author_name.clone()));
                    this.extra_selection.retain(|h| commits.iter().any(|c| &c.hash == h));
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
                RepoEvent::Notify { .. } | RepoEvent::Compare { .. } | RepoEvent::PrefillCommitMessage(_) | RepoEvent::OpenLogTab { .. } | RepoEvent::ShowConflicts => {}
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
                    // Selecting a branch navigates the log to its tip.
                    if let Some(full_name) = selected.as_deref().and_then(|id| id.strip_prefix(BRANCH_PREFIX)) {
                        let target = this.model.read(cx).refs().find(full_name).map(|r| r.target.clone());
                        this.model.update(cx, |model, cx| model.select_hash(target, cx));
                    }
                }
            }),
        ];
        let mut this = Self {
            model,
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
            show_branches: true,
            show_details: true,
            head_reachable: None,
            recent_branch_filters: Vec::new(),
            recent_user_filters: Vec::new(),
            known_authors: (None, Default::default()),
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
            short_graph: None,
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

    fn apply_text_filter(&mut self, cx: &mut Context<Self>) {
        let text = self.search.read(cx).value().to_string();
        let mut filter = self.model.read(cx).filter().clone();
        filter.text = text;
        self.model.update(cx, |model, cx| model.set_filter(filter, cx));
    }

    /// File History in its own Log tab, optionally starting from a commit.
    fn open_history_tab(&mut self, path: String, from: Option<String>, cx: &mut Context<Self>) {
        let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
        let filter = LogFilter { paths: vec![path], branches: from.into_iter().collect(), ..Default::default() };
        self.model.update(cx, |m, cx| m.open_log_tab(format!("History: {name}"), filter, cx));
    }

    fn update_filter(&mut self, cx: &mut Context<Self>, edit: impl FnOnce(&mut LogFilter)) {
        let mut filter = self.model.read(cx).filter().clone();
        edit(&mut filter);
        // Filter history: each filter remembers its last few values.
        fn remember<T: PartialEq + Clone>(list: &mut Vec<T>, value: &T) {
            list.retain(|v| v != value);
            list.insert(0, value.clone());
            list.truncate(5);
        }
        if filter.branches.len() > 1 || filter.branches.first().is_some_and(|b| b != "HEAD") {
            remember(&mut self.recent_branch_filters, &filter.branches);
        }
        if !filter.authors.is_empty() {
            remember(&mut self.recent_user_filters, &filter.authors);
        }
        if !filter.paths.is_empty() && filter.lines.is_none() {
            remember(&mut self.recent_path_filters, &filter.paths);
        }
        self.model.update(cx, |model, cx| model.set_filter(filter, cx));
    }

    /// Branch › Select…: several branches at once.
    fn select_branches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let refs = self.model.read(cx).refs().clone();
        let names: Vec<String> = refs.local_branches().chain(refs.remote_branches()).chain(refs.tags()).map(|r| r.name.clone()).collect();
        let checked: Rc<std::cell::RefCell<HashSet<String>>> =
            Rc::new(std::cell::RefCell::new(self.model.read(cx).filter().branches.iter().cloned().collect()));
        let entity = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let mut list = v_flex().gap_1().max_h(px(360.)).overflow_y_scrollbar().id("select-branches");
            for (ix, name) in names.iter().enumerate() {
                let (state, name_c) = (checked.clone(), name.clone());
                list = list.child(
                    gpui_kit::component::checkbox::Checkbox::new(SharedString::from(format!("sel-branch-{ix}")))
                        .label(name.clone())
                        .checked(checked.borrow().contains(name))
                        .on_change(move |v, window, _| {
                            if *v {
                                state.borrow_mut().insert(name_c.clone());
                            } else {
                                state.borrow_mut().remove(&name_c);
                            }
                            window.refresh();
                        }),
                );
            }
            let (checked, entity, names) = (checked.clone(), entity.clone(), names.clone());
            dialog
                .title("Select Branches")
                .w(px(380.))
                .child(list)
                .footer(dialogs::footer("OK"))
                .on_ok(move |_, _, cx| {
                    let chosen = checked.borrow();
                    let branches: Vec<String> = names.iter().filter(|n| chosen.contains(*n)).cloned().collect();
                    entity.update(cx, |this, cx| this.update_filter(cx, |f| f.branches = branches));
                    true
                })
        });
    }

    /// User › Select…: several users, checked from the log's authors or
    /// typed (names or emails, comma-separated), as IntelliJ's dialog.
    fn select_users(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let names: Vec<String> = self.known_authors.1.iter().cloned().collect();
        let current = self.model.read(cx).filter().authors.clone();
        let checked: Rc<std::cell::RefCell<HashSet<String>>> =
            Rc::new(std::cell::RefCell::new(current.iter().filter(|a| names.contains(a)).cloned().collect()));
        let others: Vec<String> = current.iter().filter(|a| !names.contains(a)).cloned().collect();
        let typed = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Other users: name or email, comma-separated").default_value(others.join(", "))
        });
        let entity = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let mut list = v_flex().gap_1().max_h(px(320.)).overflow_y_scrollbar().id("select-users");
            for (ix, name) in names.iter().enumerate() {
                let (state, name_c) = (checked.clone(), name.clone());
                list = list.child(
                    gpui_kit::component::checkbox::Checkbox::new(SharedString::from(format!("sel-user-{ix}")))
                        .label(name.clone())
                        .checked(checked.borrow().contains(name))
                        .on_change(move |v, window, _| {
                            if *v {
                                state.borrow_mut().insert(name_c.clone());
                            } else {
                                state.borrow_mut().remove(&name_c);
                            }
                            window.refresh();
                        }),
                );
            }
            let (checked, entity, names, typed_ok) = (checked.clone(), entity.clone(), names.clone(), typed.clone());
            dialog
                .title("Select Users")
                .w(px(380.))
                .child(v_flex().gap_2().child(list).child(Input::new(&typed).small()))
                .footer(dialogs::footer("OK"))
                .on_ok(move |_, _, cx| {
                    let chosen = checked.borrow();
                    let mut authors: Vec<String> = names.iter().filter(|n| chosen.contains(*n)).cloned().collect();
                    let text = typed_ok.read(cx).value().to_string();
                    authors.extend(text.split(',').map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned));
                    authors.dedup();
                    entity.update(cx, |this, cx| this.update_filter(cx, |f| f.authors = authors));
                    true
                })
        });
    }

    /// Runs the branch popup's action (matched by label) on the branch
    /// selected in the Branches panel, so the toolbar and menus agree.
    fn run_selected_branch_action(&mut self, matches: fn(&str) -> bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.branches.read(cx).selected_item().map(|i| i.id.to_string()) else { return };
        let Some(full) = id.strip_prefix(BRANCH_PREFIX) else { return };
        let refs = self.model.read(cx).refs().clone();
        let Some(reference) = refs.find(full).cloned() else { return };
        let remotes = crate::ui::branches_popup::remote_names(&refs);
        let actions = crate::ui::branches_popup::branch_actions(&self.model, &reference, refs.current_branch.as_deref(), &remotes);
        if let Some(action) = actions.into_iter().find(|a| a.enabled && matches(&a.label)) {
            (action.run)(window, cx);
        }
    }

    fn rebuild_branches(&mut self, cx: &mut Context<Self>) {
        // What the user opened or closed, unless it was search results.
        if !self.branch_searching {
            collect_expanded(&self.branch_items, &mut self.branch_expansion);
        }
        let mut refs = self.model.read(cx).refs().clone();
        // Speed search: keep matching refs only, with every folder open.
        let query = self.branch_search.read(cx).value().trim().to_lowercase();
        let searching = !query.is_empty();
        if searching {
            refs.refs.retain(|r| r.name.to_lowercase().contains(&query));
        }
        if self.my_branches {
            let mine = self.my_refs(cx);
            refs.refs.retain(|r| r.kind == RefKind::Tag || mine.contains(&r.full_name));
        }
        // Favorites first within each group, as IntelliJ pins them.
        refs.refs.sort_by_key(|r| (r.kind, !refs.favorites.contains(&r.full_name)));
        let mut items = Vec::new();
        if let Some(branch) = &refs.current_branch {
            items.push(TreeItem::new(format!("{BRANCH_PREFIX}refs/heads/{branch}"), "HEAD (Current Branch)"));
        } else if refs.head_commit.is_some() {
            items.push(TreeItem::new("head", "HEAD (Detached)"));
        }
        let local: Vec<&RefName> = refs.local_branches().collect();
        items.push(
            TreeItem::new("group:local", "Local")
                .expanded(true)
                .children(group_branches(&local, "local", |r| r.name.clone())),
        );
        let remote: Vec<&RefName> = refs.remote_branches().collect();
        let mut remotes: Vec<String> = remote.iter().filter_map(|r| r.remote().map(str::to_owned)).collect();
        remotes.dedup();
        let remote_items: Vec<TreeItem> = remotes
            .iter()
            .map(|name| {
                let branches: Vec<&RefName> = remote.iter().copied().filter(|r| r.remote() == Some(name)).collect();
                TreeItem::new(format!("group:remote/{name}"), name.clone())
                    .expanded(true)
                    .children(group_branches(&branches, &format!("remote/{name}"), |r| r.branch_without_remote().to_owned()))
            })
            .collect();
        // No remotes, no Remote node, as in IntelliJ.
        if !remote_items.is_empty() {
            items.push(TreeItem::new("group:remote", "Remote").expanded(true).children(remote_items));
        }
        let tags: Vec<&RefName> = refs.tags().collect();
        if !tags.is_empty() {
            items.push(
                TreeItem::new("group:tags", "Tags").expanded(searching).children(
                    tags.iter().map(|t| TreeItem::new(format!("{BRANCH_PREFIX}{}", t.full_name), t.name.clone())),
                ),
            );
        }
        if searching {
            items = items.into_iter().map(expand_all).collect();
        } else if let Some(expand) = self.branch_tree_expanded.take() {
            // Expand All / Collapse All, once.
            items = items.into_iter().map(if expand { expand_all } else { collapse_all }).collect();
        } else {
            // A reload or a filter keeps the folders the user opened or closed.
            restore_expanded(&items, &self.branch_expansion);
        }
        self.branch_items = items.clone();
        self.branch_searching = searching;
        let selected = self.branches.read(cx).selected_item().map(|item| item.id.clone());
        self.branches.update(cx, |tree, cx| {
            tree.set_items(items, cx);
            // The selection stays on the same branch, wherever it now is.
            let ix = selected.and_then(|id| tree.index_of(&id));
            tree.set_selected_index(ix, cx);
        });
    }

    fn set_branch_search(&mut self, value: String, window: &mut Window, cx: &mut Context<Self>) {
        self.branch_search.update(cx, |state, cx| state.set_value(value, window, cx));
        self.rebuild_branches(cx);
        cx.notify();
    }

    /// Opens or closes a folder of the branches tree, keeping the selection.
    fn toggle_branch_folder(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(entry) = self.branches.read(cx).entry(ix).cloned() else { return };
        if !entry.is_folder() {
            return;
        }
        entry.item().clone().expanded(!entry.is_expanded());
        let (items, id) = (self.branch_items.clone(), entry.item().id.clone());
        self.branches.update(cx, |tree, cx| {
            tree.set_items(items, cx);
            let ix = tree.index_of(&id);
            tree.set_selected_index(ix, cx);
        });
    }

    /// Enter: opens or closes a folder; on a branch, filters the Log by it
    /// (as a double click).
    fn on_branch_confirm(&mut self, _: &BranchConfirm, _: &mut Window, cx: &mut Context<Self>) {
        let tree = self.branches.read(cx);
        let Some(ix) = tree.selected_index() else { return };
        let Some(entry) = tree.entry(ix) else { return };
        if entry.is_folder() {
            return self.toggle_branch_folder(ix, cx);
        }
        let full = entry.item().id.strip_prefix(BRANCH_PREFIX).map(str::to_owned);
        if let Some(name) = full.and_then(|full| self.model.read(cx).refs().find(&full).map(|r| r.name.clone())) {
            self.update_filter(cx, |f| f.branches = vec![name]);
        }
    }

    /// ←: on a branch or a closed folder, goes to the parent folder (the tree
    /// itself closes an open one).
    fn on_branch_left(&mut self, _: &gpui_kit::base::actions::SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        let tree = self.branches.read(cx);
        let Some(ix) = tree.selected_index() else { return };
        let Some(entry) = tree.entry(ix) else { return };
        if entry.is_folder() && entry.is_expanded() {
            return;
        }
        let depth = entry.depth();
        let parent = (0..ix).rev().find(|&i| tree.entry(i).is_some_and(|e| e.depth() + 1 == depth));
        if let Some(parent) = parent {
            cx.stop_propagation();
            self.branches.update(cx, |tree, cx| {
                tree.set_selected_index(Some(parent), cx);
                tree.scroll_to_item(parent, ScrollStrategy::Nearest);
            });
        }
    }

    /// →: on an open folder, goes to its first child (the tree itself opens
    /// a closed one).
    fn on_branch_right(&mut self, _: &gpui_kit::base::actions::SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        let tree = self.branches.read(cx);
        let Some(ix) = tree.selected_index() else { return };
        if tree.entry(ix).is_some_and(|e| e.is_folder() && e.is_expanded()) {
            cx.stop_propagation();
            self.branches.update(cx, |tree, cx| {
                tree.set_selected_index(Some(ix + 1), cx);
                tree.scroll_to_item(ix + 1, ScrollStrategy::Nearest);
            });
        }
    }

    /// Refs holding a commit I authored (Show My Branches), computed once
    /// per set of refs.
    fn my_refs(&mut self, cx: &App) -> HashSet<String> {
        let model = self.model.read(cx);
        let key: Vec<(String, String)> = model.refs().refs.iter().map(|r| (r.full_name.clone(), r.target.clone())).collect();
        if let Some((k, mine)) = &self.my_refs {
            if *k == key {
                return mine.clone();
            }
        }
        let me = model.user_email().map(str::to_owned);
        let mine: HashSet<String> = match (model.repository(), me) {
            (Some(repo), Some(me)) => model
                .refs()
                .refs
                .iter()
                .filter(|r| r.kind != RefKind::Tag)
                .filter(|r| {
                    let author = format!("--author=<{me}>");
                    repo.run(["log", "-1", "--format=%H", "--regexp-ignore-case", "--fixed-strings", &author, &r.full_name])
                        .is_ok_and(|out| !out.trim().is_empty())
                })
                .map(|r| r.full_name.clone())
                .collect(),
            _ => HashSet::new(),
        };
        self.my_refs = Some((key, mine.clone()));
        mine
    }

    fn rebuild_changes(&mut self, cx: &mut Context<Self>) {
        let details = self.model.read(cx).details().cloned();
        self.change_kinds.clear();
        self.combined = None;
        // Several commits selected: IntelliJ shows their changes merged. Each
        // file compares its state before the oldest selected commit that
        // touched it with its state after the newest one.
        let selected = if self.extra_selection.is_empty() { Vec::new() } else { self.selected_commits(cx) };
        let combined = match self.model.read(cx).repository() {
            // Many commits (Ctrl+A): one diff from before the oldest to the
            // newest, rather than a git call per commit.
            Some(repo) if selected.len() > MAX_COMBINED => {
                let (oldest, newest) = (&selected[0], &selected[selected.len() - 1]);
                let old = if oldest.parents.is_empty() { EMPTY_TREE.to_owned() } else { format!("{}^", oldest.hash) };
                let mut changes = crate::git::diff::changed_files(repo, &old, Some(&newest.hash)).unwrap_or_default();
                changes.sort_by(|a, b| a.path.cmp(&b.path));
                let ranges = changes.iter().map(|c| (c.path.clone(), (old.clone(), newest.hash.clone()))).collect();
                Some((ranges, changes))
            }
            Some(repo) if selected.len() > 1 => {
                let mut merged: Vec<crate::git::log::FileChange> = Vec::new();
                let mut ranges: HashMap<String, (String, String)> = HashMap::new();
                for commit in &selected {
                    let parent = format!("{}^", commit.hash);
                    let old = if commit.parents.is_empty() { EMPTY_TREE.to_owned() } else { parent };
                    let Ok(changes) = crate::git::diff::changed_files(repo, &old, Some(&commit.hash)) else { continue };
                    for change in changes {
                        match ranges.get_mut(&change.path) {
                            Some(range) => range.1 = commit.hash.clone(),
                            None => {
                                ranges.insert(change.path.clone(), (old.clone(), commit.hash.clone()));
                                merged.push(change);
                            }
                        }
                    }
                }
                merged.sort_by(|a, b| a.path.cmp(&b.path));
                Some((ranges, merged))
            }
            _ => None,
        };
        let paths: Vec<String> = match (&combined, &details) {
            (Some((ranges, changes)), _) => {
                for change in changes {
                    self.change_kinds.insert(change.path.clone(), (change.kind, change.old_path.clone()));
                }
                self.combined = Some(ranges.clone());
                changes.iter().map(|c| c.path.clone()).collect()
            }
            (None, Some(details)) => {
                for change in &details.changes {
                    self.change_kinds.insert(change.path.clone(), (change.kind, change.old_path.clone()));
                }
                details.changes.iter().map(|c| c.path.clone()).collect()
            }
            (None, None) => Vec::new(),
        };
        // View Options › Group By: Directory and / or Module, else a flat list.
        let settings = crate::settings::Settings::get(cx).log.clone();
        let root = self.model.read(cx).repository().map(|r| (r.root().to_path_buf(), r.name()));
        let mut cache = HashMap::new();
        let modules: HashMap<String, String> = match (&root, settings.changes_by_module) {
            (Some((root, _)), true) => paths.iter().map(|p| (p.clone(), common::module_of(root, p, &mut cache))).collect(),
            _ => HashMap::new(),
        };
        let module_of = |path: &str| modules.get(path).cloned().unwrap_or_default();
        let root_name = root.map(|(_, name)| name).unwrap_or_default();
        let items = common::grouped_file_tree(
            paths,
            "",
            self.changes_expanded,
            settings.changes_by_directory,
            settings.changes_by_module.then_some((root_name.as_str(), &module_of as &dyn Fn(&str) -> String)),
            None,
        );
        self.change_counts.clear();
        common::count_files(&items, &mut self.change_counts);
        self.last_change_selection = None;
        self.changes.update(cx, |tree, cx| {
            tree.set_items(items, cx);
            tree.set_selected_index(None, cx);
        });
    }

    fn diff_source_for(&self, id: &str, cx: &App) -> Option<DiffSource> {
        let path = id.strip_prefix(FILE_PREFIX)?;
        let old_path = self.change_kinds.get(path).and_then(|(_, old)| old.clone());
        if let Some(ranges) = &self.combined {
            let (old, new) = ranges.get(path)?.clone();
            return Some(DiffSource::Between { old, new: Some(new), path: path.to_owned(), old_path });
        }
        let hash = self.model.read(cx).selected_hash()?.to_owned();
        Some(DiffSource::Commit { hash, path: path.to_owned(), old_path })
    }

    /// Mouse selection: plain click selects one commit, Ctrl/Cmd-click
    /// toggles, Shift-click selects the range from the last plain click.
    fn click_row(&mut self, ix: usize, toggle: bool, range: bool, cx: &mut Context<Self>) {
        self.update_selection(ix, toggle, range, cx);
        // The details pane shows the selection's combined changes.
        self.rebuild_changes(cx);
    }

    fn update_selection(&mut self, ix: usize, toggle: bool, range: bool, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let commits = model.commits().clone();
        let lead = model.selected_hash().map(str::to_owned);
        let Some(hash) = commits.get(ix).map(|c| c.hash.clone()) else { return };
        if range {
            let anchor = self.anchor.or(model.selected_index()).unwrap_or(ix);
            let (from, to) = (anchor.min(ix), anchor.max(ix));
            self.extra_selection = commits[from..=to].iter().map(|c| c.hash.clone()).filter(|h| *h != hash).collect();
        } else if toggle {
            if lead.as_deref() == Some(hash.as_str()) {
                // Deselect the lead: another selected commit takes its place.
                if let Some(next) = self.extra_selection.iter().next().cloned() {
                    self.extra_selection.remove(&next);
                    self.model.update(cx, |m, cx| m.select_hash(Some(next), cx));
                }
                cx.notify();
                return;
            }
            if !self.extra_selection.remove(&hash) {
                self.extra_selection.extend(lead);
            } else {
                cx.notify();
                return;
            }
            self.anchor = Some(ix);
        } else {
            self.extra_selection.clear();
            self.anchor = Some(ix);
        }
        self.model.update(cx, |m, cx| m.select_index(ix, cx));
        cx.notify();
    }

    /// Selected commits, oldest first (the order cherry-pick applies them).
    fn selected_commits(&self, cx: &App) -> Vec<Commit> {
        let model = self.model.read(cx);
        let lead = model.selected_hash();
        model
            .commits()
            .iter()
            .rev()
            .filter(|c| Some(c.hash.as_str()) == lead || self.extra_selection.contains(&c.hash))
            .cloned()
            .collect()
    }

    /// Go to Hash / Branch / Tag (`Ctrl+F` in the Log): branches and tags
    /// matching what is typed are offered below the field, as IntelliJ's
    /// completion; ↑↓ pick one, Enter goes to it (or to the typed hash).
    fn on_go_to_hash(&mut self, _: &GoToHash, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Hash, branch or tag"));
        let refs = self.model.read(cx).refs().clone();
        let names: Vec<(String, RefKind)> =
            refs.local_branches().chain(refs.remote_branches()).chain(refs.tags()).map(|r| (r.name.clone(), r.kind)).collect();
        let log = cx.entity();
        let suggestions = cx.new(|cx| GoToSuggestions::new(input.clone(), names, log, cx));
        let entity = cx.entity();
        window.open_dialog(cx, {
            let input = input.clone();
            move |dialog, _, _| {
            let (input_ok, suggestions_ok) = (input.clone(), suggestions.clone());
            let (up, down) = (suggestions.clone(), suggestions.clone());
            let entity = entity.clone();
            dialog
                .title("Go to Hash/Branch/Tag")
                .w(px(420.))
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .capture_action(move |_: &gpui_kit::component::input::MoveUp, _, cx| {
                                    cx.stop_propagation();
                                    up.update(cx, |s, cx| s.step(-1, cx));
                                })
                                .capture_action(move |_: &gpui_kit::component::input::MoveDown, _, cx| {
                                    cx.stop_propagation();
                                    down.update(cx, |s, cx| s.step(1, cx));
                                })
                                .child(Input::new(&input)),
                        )
                        .child(suggestions.clone()),
                )
                .footer(
                    gpui_kit::component::dialog::DialogFooter::new()
                        .gap_2()
                        .child(gpui_kit::component::dialog::DialogClose::new().child(Button::new("goto-cancel").label("Cancel").outline()))
                        .child(gpui_kit::component::dialog::DialogAction::new().child(Button::new("goto-ok").label("Go").primary())),
                )
                .on_ok(move |_, window, cx| {
                    let text = suggestions_ok.read(cx).chosen().unwrap_or_else(|| input_ok.read(cx).value().trim().to_owned());
                    entity.update(cx, |this, cx| this.go_to(&text, window, cx));
                    true
                })
            }
        });
        dialogs::focus_input(&input, window, cx);
    }

    fn go_to(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if text.is_empty() {
            return;
        }
        let model = self.model.read(cx);
        let needle = text.to_ascii_lowercase();
        let target = model
            .refs()
            .find(text)
            .map(|r| r.target.clone())
            .or_else(|| {
                model.refs().refs.iter().find(|r| r.name == text).map(|r| r.target.clone())
            })
            .or_else(|| model.commits().iter().find(|c| c.hash.starts_with(&needle)).map(|c| c.hash.clone()))
            .or_else(|| {
                model.repository().and_then(|repo| {
                    repo.run(["rev-parse", "--verify", "-q", &format!("{text}^{{commit}}")]).ok().map(|h| h.trim().to_owned())
                })
            });
        match target.filter(|hash| model.commits().iter().any(|c| &c.hash == hash)) {
            Some(hash) => {
                self.extra_selection.clear();
                self.model.update(cx, |m, cx| m.select_hash(Some(hash), cx));
                window.focus(&self.focus, cx);
            }
            None => window.push_notification(
                gpui_kit::component::notification::Notification::warning(format!("'{text}' is not in the log")),
                cx,
            ),
        }
    }

    /// Arrow keys, Page Up / Down, Home / End: one commit selected, `delta`
    /// rows from the lead one (clamped to the list).
    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let count = model.commits().len();
        if count == 0 {
            return;
        }
        let current = model.selected_index().map(|ix| ix as isize).unwrap_or(-1);
        let next = (current + delta).clamp(0, count as isize - 1) as usize;
        self.extra_selection.clear();
        self.anchor = Some(next);
        self.model.update(cx, |model, cx| model.select_index(next, cx));
        self.rebuild_changes(cx);
        cx.notify();
    }

    /// Shift with the arrows, Home or End: the range from the anchor grows
    /// or shrinks, as a Shift-click does.
    fn extend_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let count = model.commits().len();
        let Some(current) = model.selected_index() else { return self.move_selection(delta.signum(), cx) };
        if self.anchor.is_none() {
            self.anchor = Some(current);
        }
        let next = (current as isize + delta).clamp(0, count as isize - 1) as usize;
        self.click_row(next, false, true, cx);
    }

    /// Rows visible in the table, for Page Up / Page Down.
    fn page_rows(&self) -> isize {
        let state = self.scroll.0.borrow();
        let height = f32::from(state.base_handle.bounds().size.height);
        ((height / row_height()).floor() as isize - 1).max(1)
    }

    fn on_select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(-1, cx);
    }

    fn on_select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(1, cx);
    }

    fn on_select_first(&mut self, _: &SelectFirst, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(isize::MIN / 2, cx);
    }

    fn on_select_last(&mut self, _: &SelectLast, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(isize::MAX / 2, cx);
    }

    fn on_page_up(&mut self, _: &SelectPageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(-self.page_rows(), cx);
    }

    fn on_page_down(&mut self, _: &SelectPageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(self.page_rows(), cx);
    }

    fn on_extend_previous(&mut self, _: &ExtendPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.extend_selection(-1, cx);
    }

    fn on_extend_next(&mut self, _: &ExtendNext, _: &mut Window, cx: &mut Context<Self>) {
        self.extend_selection(1, cx);
    }

    fn on_extend_first(&mut self, _: &ExtendFirst, _: &mut Window, cx: &mut Context<Self>) {
        self.extend_selection(isize::MIN / 2, cx);
    }

    fn on_extend_last(&mut self, _: &ExtendLast, _: &mut Window, cx: &mut Context<Self>) {
        self.extend_selection(isize::MAX / 2, cx);
    }

    /// Ctrl+A: every loaded commit, the lead one staying where it is.
    fn on_select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let lead = model.selected_hash().map(str::to_owned).or_else(|| model.commits().first().map(|c| c.hash.clone()));
        let Some(lead) = lead else { return };
        self.extra_selection = model.commits().iter().map(|c| c.hash.clone()).filter(|h| *h != lead).collect();
        if model.selected_hash() != Some(lead.as_str()) {
            self.model.update(cx, |m, cx| m.select_hash(Some(lead), cx));
        }
        self.rebuild_changes(cx);
        cx.notify();
    }

    fn on_copy_revision(&mut self, _: &CopyRevision, _: &mut Window, cx: &mut Context<Self>) {
        // Newest first, like IntelliJ's Copy Revision Number with several selected.
        let hashes: Vec<String> = self.selected_commits(cx).into_iter().rev().map(|c| c.hash).collect();
        if !hashes.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(hashes.join("\n")));
        }
    }

    /// Compare with Current shows `current..branch`; this says what the list
    /// means and offers Swap Branches, as IntelliJ's compare tab does.
    fn render_compare_banner(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let filter = self.model.read(cx).filter().clone();
        let [range] = filter.branches.as_slice() else { return None };
        if range.contains("...") {
            return None;
        }
        let (base, branch) = range.split_once("..")?;
        let is_hash = |r: &str| r.len() == 40 && r.bytes().all(|b| b.is_ascii_hexdigit());
        let updated = is_hash(base) && is_hash(branch);
        let (diff_base, diff_branch) = (base.to_owned(), branch.to_owned());
        let shorten = |r: &str| if is_hash(r) { r[..8].to_owned() } else { r.to_owned() };
        let (base, branch) = (shorten(base), shorten(branch));
        let palette = cx.palette().clone();
        let count = self.model.read(cx).commits().len();
        let swapped = format!("{diff_branch}..{diff_base}");
        Some(
            h_flex()
                .h(px(28.))
                .px_2()
                .gap_2()
                .text_sm()
                .bg(palette.diff_header)
                .border_b_1()
                .border_color(palette.border)
                .child(Icon::new(IconName::GitCompare).small().text_color(palette.text_secondary))
                .child(div().child(match count {
                    // Update Project's "View Commits".
                    n if updated => format!("{n} commit{} received by Update Project ({base}..{branch})", if n == 1 { "" } else { "s" }),
                    0 => format!("'{branch}' has no commits that '{base}' doesn't have"),
                    n => format!("{n} commit{} in '{branch}' that {} not in '{base}'", if n == 1 { "" } else { "s" }, if n == 1 { "is" } else { "are" }),
                }))
                .child(div().flex_1())
                .when(!updated, |el| el.child(Button::new("compare-swap").xsmall().ghost().label("Swap Branches").on_click(cx.listener(move |this, _, _, cx| {
                    let swapped = swapped.clone();
                    this.update_filter(cx, |f| f.branches = vec![swapped]);
                }))))
                .child(Button::new("compare-files").xsmall().ghost().label("Show Files").on_click(cx.listener(move |this, _, _, cx| {
                    let (old, new) = (diff_base.clone(), diff_branch.clone());
                    this.model.update(cx, |m, cx| m.compare(old, Some(new), cx));
                })))
                .child(
                    tool_button("compare-close", IconName::Close, "Close Comparison")
                        .on_click(cx.listener(|this, _, _, cx| this.update_filter(cx, |f| f.branches.clear()))),
                ),
        )
    }

    fn render_filter_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let model = self.model.read(cx);
        let filter = model.filter().clone();
        let refs = model.refs().clone();
        let user_email = model.user_email().map(str::to_owned);
        let authors: Vec<String> = self.known_authors.1.iter().take(40).cloned().collect();

        let branch_label = match filter.branches.as_slice() {
            [] => "Branch".to_owned(),
            [one] => format!("Branch: {one}"),
            many => format!("Branch: {} selected", many.len()),
        };
        // My email shows as "me", as IntelliJ labels it.
        let user_name = |a: &String| if Some(a) == user_email.as_ref() { "me".to_owned() } else { a.clone() };
        let user_label = match filter.authors.as_slice() {
            [] => "User".to_owned(),
            authors => format!("User: {}", authors.iter().map(user_name).collect::<Vec<_>>().join(", ")),
        };
        let date_label = match (&filter.since, &filter.until) {
            (Some(since), Some(until)) => format!("Date: {since} – {until}"),
            (Some(since), None) => format!("Date: since {since}"),
            (None, Some(until)) => format!("Date: until {until}"),
            (None, None) => "Date".to_owned(),
        };
        let entity = cx.entity();

        // Long values (several users, a date range, a path) are cut short
        // so the bar keeps its size, as IntelliJ elides its filter labels.
        let filter_button = |id: &'static str, label: String, active: bool| {
            let label = if label.chars().count() > 28 { format!("{}…", label.chars().take(27).collect::<String>()) } else { label };
            Button::new(id)
                .ghost()
                .xsmall()
                .label(label)
                .when(active, |b| b.selected(true))
                .icon(Icon::new(IconName::ChevronDown).xsmall())
        };

        h_flex()
            .h(px(crate::ui::common::toolbar_height()))
            .px_1()
            .gap_1()
            .overflow_hidden()
            .border_b_1()
            .border_color(palette.border)
            .child(
                div().w(px(260.)).min_w(px(120.)).flex_shrink(1.).child(
                    Input::new(&self.search)
                        .xsmall()
                        .cleanable(true)
                        .prefix(Icon::new(IconName::Search).xsmall().text_color(palette.text_secondary))
                        .suffix(
                            h_flex()
                                .gap_0p5()
                                .child(
                                    Button::new("search-case")
                                        .ghost()
                                        .xsmall()
                                        .label("Cc")
                                        .tooltip("Match Case")
                                        .selected(filter.match_case)
                                        .on_click(cx.listener(|this, _, _, cx| this.update_filter(cx, |f| f.match_case = !f.match_case))),
                                )
                                .child(
                                    Button::new("search-regex")
                                        .ghost()
                                        .xsmall()
                                        .label(".*")
                                        .tooltip("Regex")
                                        .selected(filter.regex)
                                        .on_click(cx.listener(|this, _, _, cx| this.update_filter(cx, |f| f.regex = !f.regex))),
                                ),
                        ),
                ),
            )
            .child(filter_button("filter-branch", branch_label, !filter.branches.is_empty()).dropdown_menu({
                let entity = entity.clone();
                let refs = refs.clone();
                let selected = filter.branches.clone();
                let recent_branches = self.recent_branch_filters.clone();
                move |mut menu, _, _| {
                    let set = |entity: &Entity<LogView>, branches: Vec<String>| {
                        let entity = entity.clone();
                        move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
                            let branches = branches.clone();
                            entity.update(cx, |this, cx| this.update_filter(cx, |f| f.branches = branches));
                        }
                    };
                    menu = menu.item(PopupMenuItem::new("All").checked(selected.is_empty()).on_click(set(&entity, vec![])));
                    if refs.current_branch.is_some() || refs.head_commit.is_some() {
                        menu = menu.item(PopupMenuItem::new("HEAD").checked(selected == ["HEAD"]).on_click(set(&entity, vec!["HEAD".into()])));
                    }
                    let favorites: Vec<String> = refs.refs.iter().filter(|r| refs.favorites.contains(&r.full_name)).map(|r| r.name.clone()).collect();
                    if !favorites.is_empty() {
                        menu = menu.item(PopupMenuItem::new("Favorites").checked(selected == favorites).on_click(set(&entity, favorites.clone())));
                    }
                    let select_entity = entity.clone();
                    menu = menu.item(PopupMenuItem::new("Select…").on_click(move |_, window, cx| {
                        select_entity.update(cx, |this, cx| this.select_branches(window, cx))
                    }));
                    if !recent_branches.is_empty() {
                        menu = menu.separator().label("Recent");
                        for recent in &recent_branches {
                            menu = menu.item(PopupMenuItem::new(recent.join(", ")).checked(&selected == recent).on_click(set(&entity, recent.clone())));
                        }
                    }
                    menu = menu.separator().label("Local");
                    for branch in refs.local_branches() {
                        let name = branch.name.clone();
                        menu = menu.item(
                            PopupMenuItem::new(name.clone())
                                .checked(selected.contains(&name))
                                .on_click(set(&entity, vec![name])),
                        );
                    }
                    menu = menu.separator().label("Remote");
                    for branch in refs.remote_branches() {
                        let name = branch.name.clone();
                        menu = menu.item(
                            PopupMenuItem::new(name.clone())
                                .checked(selected.contains(&name))
                                .on_click(set(&entity, vec![name])),
                        );
                    }
                    menu.max_h(px(420.))
                }
            }))
            .child(filter_button("filter-user", user_label, !filter.authors.is_empty()).dropdown_menu({
                let entity = entity.clone();
                let current = filter.authors.clone();
                let recent_users = self.recent_user_filters.clone();
                move |mut menu, _, _| {
                    let set = |authors: Vec<String>| {
                        let entity = entity.clone();
                        move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
                            let authors = authors.clone();
                            entity.update(cx, |this, cx| this.update_filter(cx, |f| f.authors = authors));
                        }
                    };
                    menu = menu.item(PopupMenuItem::new("All").checked(current.is_empty()).on_click(set(Vec::new())));
                    if let Some(email) = &user_email {
                        menu = menu.item(PopupMenuItem::new("me").checked(current == [email.clone()]).on_click(set(vec![email.clone()])));
                    }
                    let select = entity.clone();
                    menu = menu.item(PopupMenuItem::new("Select…").on_click(move |_, window, cx| {
                        select.update(cx, |this, cx| this.select_users(window, cx))
                    }));
                    let recent: Vec<&Vec<String>> =
                        recent_users.iter().filter(|u| user_email.as_ref().is_none_or(|me| **u != [me.clone()])).collect();
                    if !recent.is_empty() {
                        menu = menu.separator().label("Recent");
                        for authors in recent {
                            let label = authors
                                .iter()
                                .map(|a| if Some(a) == user_email.as_ref() { "me" } else { a.as_str() })
                                .collect::<Vec<_>>()
                                .join(", ");
                            menu = menu.item(PopupMenuItem::new(label).checked(&current == authors).on_click(set(authors.clone())));
                        }
                    }
                    menu = menu.separator();
                    for author in &authors {
                        menu = menu.item(
                            PopupMenuItem::new(author.clone())
                                .checked(current == [author.clone()])
                                .on_click(set(vec![author.clone()])),
                        );
                    }
                    menu.max_h(px(420.))
                }
            }))
            .child(filter_button("filter-date", date_label, filter.since.is_some() || filter.until.is_some()).dropdown_menu({
                let entity = entity.clone();
                let current = filter.since.clone();
                move |mut menu, _, _| {
                    for (label, since) in [
                        ("All", None),
                        ("Last 24 hours", Some("24 hours ago")),
                        ("Last 7 days", Some("7 days ago")),
                        ("Last 30 days", Some("30 days ago")),
                        ("Last year", Some("1 year ago")),
                    ] {
                        let entity = entity.clone();
                        let since = since.map(str::to_owned);
                        menu = menu.item(PopupMenuItem::new(label).checked(current == since).on_click(move |_, _, cx| {
                            let since = since.clone();
                            entity.update(cx, |this, cx| this.update_filter(cx, |f| {
                                f.since = since;
                                f.until = None;
                            }));
                        }));
                    }
                    let entity = entity.clone();
                    menu.separator().item(PopupMenuItem::new("Select…").on_click(move |_, window, cx| {
                        entity.update(cx, |this, cx| this.select_date_range(window, cx))
                    }))
                }
            }))
            .child({
                let label = match filter.paths.as_slice() {
                    [] => "Paths".to_owned(),
                    [one] => format!("Path: {one}"),
                    many => format!("Paths: {}", many.len()),
                };
                let entity = entity.clone();
                let current = filter.paths.clone();
                let recent_paths = self.recent_path_filters.clone();
                filter_button("filter-path", label, !filter.paths.is_empty()).dropdown_menu(move |mut menu, _, _| {
                    let clear = entity.clone();
                    menu = menu.item(PopupMenuItem::new("All").checked(current.is_empty()).on_click(move |_, _, cx| {
                        clear.update(cx, |this, cx| this.update_filter(cx, |f| f.paths.clear()))
                    }));
                    let pick = entity.clone();
                    menu = menu.item(PopupMenuItem::new("Select Folders…").on_click(move |_, window, cx| {
                        pick.update(cx, |this, cx| this.select_paths(window, cx))
                    }));
                    if !recent_paths.is_empty() {
                        menu = menu.separator().label("Recent");
                        for paths in &recent_paths {
                            let (entity, paths_c) = (entity.clone(), paths.clone());
                            menu = menu.item(PopupMenuItem::new(paths.join(", ")).checked(&current == paths).on_click(move |_, _, cx| {
                                let paths = paths_c.clone();
                                entity.update(cx, |this, cx| this.update_filter(cx, |f| f.paths = paths))
                            }));
                        }
                    }
                    menu
                })
            })
            .child(div().flex_1())
            .when(model.is_loading(), |el| {
                el.child(div().text_xs().text_color(palette.text_secondary).child("Loading…"))
            })
            .child(tool_button("log-refresh", IconName::RefreshCw, "Refresh").on_click(cx.listener(|this, _, _, cx| {
                this.model.update(cx, |model, cx| model.reload(cx));
            })))
            .child(
                tool_button("log-toggle-branches", IconName::PanelLeft, "Show Branches")
                    .selected(self.show_branches)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_branches = !this.show_branches;
                        cx.notify();
                    })),
            )
            .child(
                tool_button("log-toggle-details", IconName::Rows3, "Show Details")
                    .selected(self.show_details)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_details = !this.show_details;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("log-options")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::Eye))
                    .tooltip("View Options")
                    .dropdown_menu({
                        let entity = entity.clone();
                        let log = Settings::get(cx).log.clone();
                        let collapse = self.model.read(cx).collapse_linear();
                        move |menu, _, _| {
                            let collapse_entity = entity.clone();
                            // Each toggle flips one View Options flag and reloads when it changes the order.
                            let toggle = |label: &'static str, on: bool, set: fn(&mut crate::settings::LogSettings, bool), reload: bool| {
                                let entity = entity.clone();
                                PopupMenuItem::new(label).checked(on).on_click(move |_, _, cx| {
                                    Settings::update(cx, |s| set(&mut s.log, !on));
                                    entity.update(cx, |this, cx| {
                                        if reload {
                                            this.model.update(cx, |model, cx| model.reload(cx));
                                        }
                                        cx.notify();
                                    });
                                })
                            };
                            menu.item(PopupMenuItem::new("Collapse Linear Branches").checked(collapse).on_click(
                                move |_, _, cx| {
                                    collapse_entity.update(cx, |this, cx| {
                                        this.model.update(cx, |model, cx| model.set_collapse_linear(!collapse, cx));
                                    })
                                },
                            ))
                            .item(toggle("Show Long Edges", log.show_long_edges, |l, v| l.show_long_edges = v, false))
                            .separator()
                            .label("Sort")
                            .item(toggle("IntelliSort", !log.sort_by_date, |l, _| l.sort_by_date = false, true))
                            .item(toggle("By Date", log.sort_by_date, |l, _| l.sort_by_date = true, true))
                            .separator()
                            .label("Highlight")
                            .item(toggle("My Commits", log.highlight_mine, |l, v| l.highlight_mine = v, false))
                            .item(toggle("Merge Commits", log.highlight_merges, |l, v| l.highlight_merges = v, false))
                            .item(toggle("Current Branch", log.highlight_current_branch, |l, v| l.highlight_current_branch = v, false))
                            .item(toggle("Not Merged into Current Branch", log.highlight_not_merged, |l, v| l.highlight_not_merged = v, false))
                            .separator()
                            .label("References")
                            .item(toggle("Compact References View", log.compact_refs, |l, v| l.compact_refs = v, false))
                            .item(toggle("Show References on the Left", log.refs_on_left, |l, v| l.refs_on_left = v, false))
                            .separator()
                            .label("Show Columns")
                            .item(toggle("Author", log.show_author, |l, v| l.show_author = v, false))
                            .item(toggle("Date", log.show_date, |l, v| l.show_date = v, false))
                            .item(toggle("Hash", log.show_hash, |l, v| l.show_hash = v, false))
                            .separator()
                            .item(toggle("Relative Dates", log.relative_dates, |l, v| l.relative_dates = v, false))
                        }
                    }),
            )
    }

    /// Paths › Select Folders…: the repository's folders and files as a
    /// tree to check, as IntelliJ's structure filter shows the project.
    fn select_paths(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = self.model.read(cx).repository().cloned() else { return };
        let files: Vec<String> = repo.run(["ls-files"]).unwrap_or_default().lines().map(str::to_owned).collect();
        let tree_state = cx.new(|cx| TreeState::new(cx).items(common::file_tree_with(files, "", false)));
        let checked: Rc<std::cell::RefCell<HashSet<String>>> =
            Rc::new(std::cell::RefCell::new(self.model.read(cx).filter().paths.iter().cloned().collect()));
        let entity = cx.entity();
        let palette = cx.palette().clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let rows_checked = checked.clone();
            let palette = palette.clone();
            let list = tree(&tree_state, move |ix, entry, _, _, _| {
                let id = entry.item().id.to_string();
                let path = id.strip_prefix(DIR_PREFIX).or_else(|| id.strip_prefix(FILE_PREFIX)).unwrap_or_default().to_owned();
                let set = rows_checked.borrow();
                // A path under a checked folder is part of the filter too.
                let inherited = set.iter().any(|p| path.starts_with(&format!("{p}/")));
                let state = rows_checked.clone();
                let toggle_path = path.clone();
                ListItem::new(ix).py_0().px_1().h(px(row_height())).child(
                    h_flex()
                        .w_full()
                        .gap_1()
                        .pl(px(entry.depth() as f32 * 14.))
                        .text_sm()
                        .child(if entry.is_folder() {
                            Icon::new(if entry.is_expanded() { IconName::ChevronDown } else { IconName::ChevronRight })
                                .xsmall()
                                .text_color(palette.text_secondary)
                        } else {
                            Icon::new(IconName::Circle).xsmall().text_color(gpui_kit::transparent_black())
                        })
                        .child(
                            div().on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(
                                gpui_kit::component::checkbox::Checkbox::new(SharedString::from(format!("path-{ix}")))
                                    .checked(inherited || set.contains(&path))
                                    .disabled(inherited)
                                    .on_change(move |v, window, _| {
                                        let mut set = state.borrow_mut();
                                        if *v {
                                            set.retain(|p| !p.starts_with(&format!("{toggle_path}/")));
                                            set.insert(toggle_path.clone());
                                        } else {
                                            set.remove(&toggle_path);
                                        }
                                        window.refresh();
                                    }),
                            ),
                        )
                        .child(
                            Icon::new(if entry.is_folder() { IconName::Folder } else { common::file_icon(&path) })
                                .small()
                                .text_color(palette.text_secondary),
                        )
                        .child(entry.item().label.clone()),
                )
            });
            let (checked, entity) = (checked.clone(), entity.clone());
            dialog
                .title("Select Folders and Files")
                .w(px(460.))
                .child(div().h(px(380.)).border_1().border_color(palette.border).child(list.size_full()))
                .footer(dialogs::footer("OK"))
                .on_ok(move |_, _, cx| {
                    let mut paths: Vec<String> = checked.borrow().iter().cloned().collect();
                    paths.sort();
                    entity.update(cx, |this, cx| this.update_filter(cx, |f| f.paths = paths));
                    true
                })
        });
    }

    /// Date › Select…: a from / to range (`--since` / `--until`).
    fn select_date_range(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let filter = self.model.read(cx).filter().clone();
        let from = cx.new(|cx| InputState::new(window, cx).placeholder("YYYY-MM-DD").default_value(filter.since.clone().unwrap_or_default()));
        let to = cx.new(|cx| InputState::new(window, cx).placeholder("YYYY-MM-DD").default_value(filter.until.clone().unwrap_or_default()));
        let entity = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let (from_ok, to_ok, entity) = (from.clone(), to.clone(), entity.clone());
            dialog
                .title("Select Period")
                .w(px(360.))
                .child(
                    v_flex()
                        .gap_2()
                        .child(h_flex().gap_2().child(div().w(px(40.)).text_sm().child("From:")).child(div().flex_1().child(Input::new(&from).small())))
                        .child(h_flex().gap_2().child(div().w(px(40.)).text_sm().child("To:")).child(div().flex_1().child(Input::new(&to).small()))),
                )
                .footer(dialogs::footer("OK"))
                .on_ok(move |_, _, cx| {
                    let value = |input: &Entity<InputState>, cx: &App| {
                        let v = input.read(cx).value().trim().to_owned();
                        (!v.is_empty()).then_some(v)
                    };
                    let (since, until) = (value(&from_ok, cx), value(&to_ok, cx));
                    entity.update(cx, |this, cx| this.update_filter(cx, |f| {
                        f.since = since;
                        f.until = until;
                    }));
                    true
                })
        });
    }

    /// The graph with long edges cut into arrows (cached per layout).
    fn graph_without_long_edges(&mut self, graph: Arc<crate::git::GraphLayout>) -> Arc<crate::git::GraphLayout> {
        let key = Arc::as_ptr(&graph) as usize;
        match &self.short_graph {
            Some((k, hidden)) if *k == key => hidden.clone(),
            _ => {
                let hidden = Arc::new(graph.hide_long_edges());
                self.short_graph = Some((key, hidden.clone()));
                hidden
            }
        }
    }

    /// Hashes reachable from HEAD within the loaded log (cached per load).
    fn head_reachable(&mut self, cx: &App) -> Rc<HashSet<String>> {
        let model = self.model.read(cx);
        let commits = model.commits().clone();
        let head = model.refs().head_commit.clone();
        let key = Arc::as_ptr(&commits) as usize;
        if let Some((k, h, set)) = &self.head_reachable {
            if *k == key && *h == head {
                return set.clone();
            }
        }
        let parents: HashMap<&str, &Vec<String>> = commits.iter().map(|c| (c.hash.as_str(), &c.parents)).collect();
        let mut reachable = HashSet::new();
        let mut stack: Vec<String> = head.iter().cloned().collect();
        while let Some(hash) = stack.pop() {
            if let Some(ps) = parents.get(hash.as_str()) {
                stack.extend(ps.iter().filter(|p| !reachable.contains(*p)).cloned());
            }
            reachable.insert(hash);
        }
        let set = Rc::new(reachable);
        self.head_reachable = Some((key, head, set.clone()));
        set
    }

    /// Follows a column edge drag anywhere in the window; the widths are
    /// saved when the mouse is released.
    fn column_drag_tracker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        gpui_kit::canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let moving = entity.clone();
                window.on_mouse_event(move |e: &gpui_kit::MouseMoveEvent, phase, _, cx| {
                    if phase != gpui_kit::DispatchPhase::Bubble {
                        return;
                    }
                    moving.update(cx, |this, cx| {
                        let (Some((column, start, width)), Some(mut widths)) = (this.column_drag, this.columns) else { return };
                        // The edge is on the column's left: dragging left widens it.
                        let w = (width as f32 + start - f32::from(e.position.x)).clamp(40., 600.) as u32;
                        if widths[column] != w {
                            widths[column] = w;
                            this.columns = Some(widths);
                            cx.notify();
                        }
                    });
                });
                let released = entity.clone();
                window.on_mouse_event(move |_: &gpui_kit::MouseUpEvent, phase, _, cx| {
                    if phase != gpui_kit::DispatchPhase::Bubble {
                        return;
                    }
                    released.update(cx, |this, cx| {
                        this.column_drag = None;
                        if let Some(widths) = this.columns.take() {
                            Settings::update(cx, |s| s.log.columns = widths);
                        }
                        cx.notify();
                    });
                });
            },
        )
        .absolute()
        .size_full()
    }

    fn render_rows(&mut self, range: std::ops::Range<usize>, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let palette = cx.palette().clone();
        let focused = self.focus.contains_focused(window, cx);
        let model = self.model.read(cx);
        let commits = model.commits().clone();
        let graph = model.graph().clone();
        let refs = model.refs().clone();
        let selected = model.selected_index();
        let show_long_edges = Settings::get(cx).log.show_long_edges;
        let graph = if show_long_edges { graph } else { self.graph_without_long_edges(graph) };
        let model = self.model.read(cx);
        let extra = self.extra_selection.clone();
        let me = model.user_email().map(str::to_owned);
        let log = Settings::get(cx).log.clone();
        let show_hash = log.show_hash;
        let entity = cx.entity();
        let widths = self.columns.unwrap_or(log.columns).map(|w| px(w as f32));
        // A column's left edge: drag it to resize the column, as in IntelliJ's log table.
        let edge = |column: usize, ix: usize, entity: &Entity<Self>| {
            let entity = entity.clone();
            div()
                .id(("log-column-edge", column * 10_000_000 + ix))
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(px(5.))
                .cursor(gpui_kit::CursorStyle::ResizeLeftRight)
                .on_mouse_down(gpui_kit::MouseButton::Left, move |event: &gpui_kit::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    entity.update(cx, |this, cx| {
                        let widths = this.columns.unwrap_or(Settings::get(cx).log.columns);
                        this.columns = Some(widths);
                        this.column_drag = Some((column, f32::from(event.position.x), widths[column]));
                        cx.notify();
                    });
                })
        };
        let reachable = if log.highlight_current_branch || log.highlight_not_merged { Some(self.head_reachable(cx)) } else { None };
        // Clicking a long edge's arrow goes to the edge's other end.
        let on_arrow: crate::ui::graph_paint::OnArrow = {
            let entity = entity.clone();
            Rc::new(move |target, window, cx| {
                entity.update(cx, |this, cx| {
                    window.focus(&this.focus, cx);
                    this.click_row(target, false, false, cx);
                })
            })
        };
        let model = self.model.read(cx);

        range
            .filter_map(|ix| {
                let commit = commits.get(ix)?;
                let row = graph.rows.get(ix).cloned().unwrap_or_default();
                let is_selected = selected == Some(ix) || extra.contains(&commit.hash);
                let is_head = refs.head_commit.as_deref() == Some(commit.hash.as_str());
                let mine = log.highlight_mine && me.as_deref().is_some_and(|me| me == commit.author_email);
                let labels = refs.for_commit(&commit.hash);
                let is_merge = commit.parents.len() > 1;
                let on_head = reachable.as_ref().map(|r| r.contains(&commit.hash));
                // Merge commits and commits not merged into HEAD are greyed, as IntelliJ's highlighters do.
                let dim = (log.highlight_merges && is_merge) || (log.highlight_not_merged && on_head == Some(false));
                let text_color = if dim { palette.text_secondary } else { palette.text };
                let branch_tint = log.highlight_current_branch && on_head == Some(true);

                // The subject keeps some width in a narrow table; the other
                // columns are clipped instead.
                let mut subject = h_flex()
                    .flex_1()
                    .min_w(px(160.))
                    .h_full()
                    .overflow_hidden()
                    .child(graph_canvas(row, &palette, is_head, ix, &on_arrow));
                let shown = if log.compact_refs { 1 } else { 4 };
                let mut ref_labels = h_flex().flex_shrink_0();
                // A detached HEAD gets its own label, as in IntelliJ.
                if is_head && refs.current_branch.is_none() {
                    ref_labels = ref_labels.child(
                        h_flex()
                            .flex_shrink_0()
                            .gap_0p5()
                            .mr_1p5()
                            .text_xs()
                            .child(Icon::new(IconName::GitBranch).xsmall().text_color(palette.ref_head))
                            .child(div().text_color(palette.ref_head).child("HEAD")),
                    );
                }
                for label in labels.iter().take(shown) {
                    ref_labels = ref_labels.child(ref_label(label, refs.current_branch.as_deref(), &palette));
                }
                if labels.len() > shown {
                    ref_labels = ref_labels.child(
                        div().mr_1().text_xs().text_color(palette.text_secondary).child(format!("+{}", labels.len() - shown)),
                    );
                }
                if log.refs_on_left {
                    subject = subject.child(ref_labels);
                    ref_labels = h_flex();
                }
                subject = subject.child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_color(text_color)
                        .when(mine, |el| el.font_weight(FontWeight::SEMIBOLD))
                        .child(commit.subject.clone()),
                );
                if !log.refs_on_left && !labels.is_empty() {
                    subject = subject.child(div().flex_1()).child(ref_labels);
                }
                if let Some(count) = model.hidden_below(&commit.hash) {
                    let run = commit.hash.clone();
                    let model_entity = self.model.clone();
                    subject = subject.child(
                        div()
                            .id(SharedString::from(format!("expand-{}", commit.hash)))
                            .ml_2()
                            .px_1()
                            .rounded_sm()
                            .text_xs()
                            .text_color(palette.text_secondary)
                            .bg(palette.text_secondary.opacity(0.12))
                            .hover(|el| el.bg(palette.text_secondary.opacity(0.25)))
                            .cursor_pointer()
                            .child(format!("⋯ {count} commits"))
                            .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                model_entity.update(cx, |model, cx| model.expand_run(run.clone(), cx));
                            }),
                    );
                }

                let hash = commit.hash.clone();
                let menu_commit = commit.clone();
                let menu_entity = entity.clone();
                Some(
                    h_flex()
                        .id(SharedString::from(format!("commit-{}", commit.hash)))
                        .h(px(row_height()))
                        .w_full()
                        .pr_2()
                        .text_sm()
                        .when(is_selected, |el| el.bg(if focused { palette.selection } else { palette.selection_inactive }))
                        .when(!is_selected && branch_tint, |el| el.bg(palette.accent.opacity(0.08)))
                        .when(!is_selected, |el| el.hover(|s| s.bg(palette.hover)))
                        .on_mouse_down(gpui_kit::MouseButton::Left, {
                            let entity = entity.clone();
                            move |event: &gpui_kit::MouseDownEvent, window, cx| {
                                let (toggle, range) = (event.modifiers.secondary(), event.modifiers.shift);
                                entity.update(cx, |this, cx| {
                                    window.focus(&this.focus, cx);
                                    this.click_row(ix, toggle, range, cx);
                                });
                            }
                        })
                        .on_mouse_down(gpui_kit::MouseButton::Right, {
                            let entity = entity.clone();
                            let hash = hash.clone();
                            move |_, _, cx| {
                                let hash = hash.clone();
                                entity.update(cx, |this, cx| {
                                    // Right-clicking inside a multi-selection keeps it.
                                    let in_selection = this.extra_selection.contains(&hash)
                                        || this.model.read(cx).selected_hash() == Some(hash.as_str());
                                    if !in_selection {
                                        this.extra_selection.clear();
                                        this.model.update(cx, |model, cx| model.select_hash(Some(hash), cx));
                                    }
                                });
                            }
                        })
                        .context_menu(move |menu, window, cx| commit_menu(menu, &menu_entity, &menu_commit, window, cx))
                        .child(subject)
                        .when(log.show_author, |el| {
                            el.child(
                                div()
                                    .relative()
                                    .w(widths[0])
                                    .flex_shrink_0()
                                    .pl_2()
                                    .child(edge(0, ix, &entity))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_color(palette.text_secondary)
                                    .when(mine, |el| el.font_weight(FontWeight::SEMIBOLD))
                                    .child(commit.author_name.clone()),
                            )
                        })
                        .when(log.show_date, |el| {
                            el.child(
                                div()
                                    .relative()
                                    .w(widths[1])
                                    .flex_shrink_0()
                                    .pl_2()
                                    .child(edge(1, ix, &entity))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_color(palette.text_secondary)
                                    .child(if log.relative_dates {
                                        common::format_relative_date(commit.author_time)
                                    } else {
                                        common::format_date(commit.author_time)
                                    }),
                            )
                        })
                        .when(show_hash, |el| {
                            el.child(
                                div()
                                    .relative()
                                    .w(widths[2])
                                    .flex_shrink_0()
                                    .pl_2()
                                    .child(edge(2, ix, &entity))
                                    .overflow_hidden()
                                    .font_family(cx.theme().mono_font_family.clone())
                                    .text_color(palette.text_secondary)
                                    .child(commit.short_hash().to_owned()),
                            )
                        })
                        .into_any_element(),
                )
            })
            .collect()
    }

    fn render_branches(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let refs = self.model.read(cx).refs().clone();
        let query = self.branch_search.read(cx).value().trim().to_owned();
        let entity = cx.entity();
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(px(crate::ui::common::toolbar_height()))
                    .px_1()
                    .gap_0p5()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(tool_button("branches-new", IconName::Plus, "New Branch…").on_click(cx.listener(
                        |this, _, window, cx| {
                            let start = this.model.read(cx).selected_hash().unwrap_or("HEAD").to_owned();
                            dialogs::new_branch(this.model.clone(), start, window, cx);
                        },
                    )))
                    .child(tool_button("branches-fetch", IconName::CloudDownload, "Fetch").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.model.update(cx, |model, cx| {
                                model.run_operation("Fetch", |repo| {
                                    repo.run(["fetch", "--all", "--prune"])?;
                                    Ok("Fetched all remotes".into())
                                }, cx)
                            });
                        },
                    )))
                    .child(tool_button("branches-update", IconName::ArrowDownToLine, "Update Selected").on_click(cx.listener(
                        |this, _, window, cx| this.run_selected_branch_action(|label| label == "Update", window, cx),
                    )))
                    .child(tool_button("branches-delete", IconName::Delete, "Delete").on_click(cx.listener(
                        |this, _, window, cx| this.run_selected_branch_action(|label| label == "Delete", window, cx),
                    )))
                    .child(tool_button("branches-compare", IconName::GitCompare, "Compare with Current").on_click(cx.listener(
                        |this, _, window, cx| this.run_selected_branch_action(|label| label.starts_with("Compare with"), window, cx),
                    )))
                    .child(
                        tool_button("branches-mine", IconName::User, "Show My Branches")
                            .selected(self.my_branches)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.my_branches = !this.my_branches;
                                this.rebuild_branches(cx);
                                cx.notify();
                            })),
                    )
                    .child(tool_button("branches-expand", IconName::ChevronsUpDown, "Expand All").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.branch_tree_expanded = Some(true);
                            this.rebuild_branches(cx);
                        },
                    )))
                    .child(tool_button("branches-collapse", IconName::ChevronsDownUp, "Collapse All").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.branch_tree_expanded = Some(false);
                            this.rebuild_branches(cx);
                        },
                    )))
                    .child(tool_button("branches-filter", IconName::ListFilter, "Filter Log by Selected Branch").on_click(
                        cx.listener(|this, _, _, cx| {
                            let selected = this.branches.read(cx).selected_item().map(|i| i.id.clone());
                            let name = selected
                                .as_deref()
                                .and_then(|id| id.strip_prefix(BRANCH_PREFIX))
                                .and_then(|full| this.model.read(cx).refs().find(full).map(|r| r.name.clone()));
                            this.update_filter(cx, |f| f.branches = name.into_iter().collect());
                        }),
                    ))
            )
            .child(
                div()
                    .px_1()
                    .py_1()
                    .border_b_1()
                    .border_color(palette.border)
                    // Escape clears the search first, as IntelliJ's search fields do.
                    .capture_action(cx.listener(|this, _: &gpui_kit::component::input::Escape, window, cx| {
                        if !this.branch_search.read(cx).value().is_empty() {
                            cx.stop_propagation();
                            this.set_branch_search(String::new(), window, cx);
                        }
                    }))
                    .child(Input::new(&self.branch_search).xsmall().cleanable(true)),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .key_context(BRANCHES_CONTEXT)
                    .on_action(cx.listener(Self::on_branch_confirm))
                    .capture_action(cx.listener(Self::on_branch_left))
                    .capture_action(cx.listener(Self::on_branch_right))
                    // Typing in the tree starts the speed search.
                    .on_key_down(cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, cx| {
                        let keystroke = &event.keystroke;
                        let modifiers = keystroke.modifiers;
                        if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
                            return;
                        }
                        let Some(text) = keystroke.key_char.clone().filter(|t| t.chars().all(|c| !c.is_control())) else { return };
                        cx.stop_propagation();
                        let value = format!("{}{text}", this.branch_search.read(cx).value());
                        this.set_branch_search(value, window, cx);
                        let input = this.branch_search.clone();
                        input.update(cx, |state, cx| state.focus(window, cx));
                    }))
                    .child(
                    tree(&self.branches, move |ix, entry, _selected, _, _| {
                        let item = entry.item();
                        let id = item.id.to_string();
                        let full_name = id.strip_prefix(BRANCH_PREFIX).map(str::to_owned);
                        let reference = full_name.as_deref().and_then(|full| refs.find(full)).cloned();
                        let is_current = reference.as_ref().is_some_and(|r| {
                            r.kind == RefKind::LocalBranch && Some(&r.name) == refs.current_branch.as_ref()
                        });
                        let icon_name = if entry.is_folder() {
                            if id.starts_with("group:") { IconName::FolderGit2 } else { IconName::Folder }
                        } else {
                            match reference.as_ref().map(|r| r.kind) {
                                Some(RefKind::Tag) => IconName::Tag,
                                _ => IconName::GitBranch,
                            }
                        };
                        let icon_color = match reference.as_ref().map(|r| r.kind) {
                            _ if is_current || id == "head" => palette.ref_head,
                            Some(RefKind::LocalBranch) => palette.ref_local,
                            Some(RefKind::RemoteBranch) => palette.ref_remote,
                            Some(RefKind::Tag) => palette.ref_tag,
                            _ => palette.text_secondary,
                        };
                        let is_favorite = full_name.as_ref().is_some_and(|f| refs.favorites.contains(f));
                        let icon_name = if is_favorite && !entry.is_folder() { IconName::Star } else { icon_name };
                        let menu_reference = reference.clone();
                        let (menu_entity, menu_refs) = (entity.clone(), refs.clone());
                        let entity = entity.clone();
                        let double_click_name = reference.as_ref().map(|r| r.name.clone());
                        ListItem::new(ix)
                            .py_0()
                            .px_1()
                            .h(px(row_height()))
                            .on_click(move |event, _, cx| {
                                if event.click_count() == 2 {
                                    // Double click filters the log by this branch.
                                    if let Some(name) = double_click_name.clone() {
                                        entity.update(cx, |this, cx| this.update_filter(cx, |f| f.branches = vec![name]));
                                    }
                                }
                            })
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_1()
                                    .pl(px(entry.depth() as f32 * 14.))
                                    .text_sm()
                                    .child(if entry.is_folder() {
                                        Icon::new(if entry.is_expanded() { IconName::ChevronDown } else { IconName::ChevronRight })
                                            .xsmall()
                                            .text_color(palette.text_secondary)
                                    } else {
                                        Icon::new(IconName::Circle).xsmall().text_color(gpui_kit::transparent_black())
                                    })
                                    .child(Icon::new(icon_name).small().text_color(icon_color))
                                    .child(match label_match(&item.label, &query) {
                                        Some(range) => crate::ui::find_popup::highlighted(&item.label, &[range], &palette).into_any_element(),
                                        None => item.label.clone().into_any_element(),
                                    })
                                    .when_some(reference.filter(|r| r.ahead > 0 || r.behind > 0), |el, r| {
                                        el.child(
                                            h_flex()
                                                .gap_1()
                                                .text_xs()
                                                .when(r.behind > 0, |el| {
                                                    el.child(div().text_color(palette.link).child(format!("↓{}", r.behind)))
                                                })
                                                .when(r.ahead > 0, |el| {
                                                    el.child(div().text_color(palette.status_added).child(format!("↑{}", r.ahead)))
                                                }),
                                        )
                                    })
                                    .context_menu(move |menu, _, cx| {
                                        let Some(reference) = menu_reference.clone() else { return menu };
                                        branch_menu(menu, &menu_entity, &menu_refs, &reference, cx)
                                    }),
                            )
                    })
                    .size_full(),
                ),
            )
    }

    fn render_details(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let submodules = self.model.read(cx).submodule_paths().clone();
        let palette = cx.palette().clone();
        let model = self.model.read(cx);
        let details = model.details().cloned();
        let refs = model.refs().clone();
        let kinds = self.change_kinds.clone();
        let counts = self.change_counts.clone();
        let entity = cx.entity();

        let tree_palette = palette.clone();
        let changes = v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(px(crate::ui::common::toolbar_height()))
                    .px_2()
                    .border_b_1()
                    .border_color(palette.border)
                    .text_sm()
                    .text_color(palette.text_secondary)
                    .child(match &details {
                        _ if self.combined.is_some() => {
                            let n = self.combined.as_ref().map_or(0, |c| c.len());
                            format!("{} commits selected · {n} {} changed", self.extra_selection.len() + 1, if n == 1 { "file" } else { "files" })
                        }
                        Some(d) => format!("{} {} changed", d.changes.len(), if d.changes.len() == 1 { "file" } else { "files" }),
                        None => "No commit selected".into(),
                    })
                    .child(div().flex_1())
                    .child(tool_button("log-changes-expand", IconName::ChevronsUpDown, "Expand All").on_click(cx.listener(|this, _, _, cx| {
                        this.changes_expanded = true;
                        this.rebuild_changes(cx);
                        cx.notify();
                    })))
                    .child(tool_button("log-changes-collapse", IconName::ChevronsDownUp, "Collapse All").on_click(cx.listener(|this, _, _, cx| {
                        this.changes_expanded = false;
                        this.rebuild_changes(cx);
                        cx.notify();
                    })))
                    .child({
                        let settings = Settings::get(cx).log.clone();
                        let entity = cx.entity();
                        Button::new("log-changes-options").ghost().xsmall().icon(IconName::Eye).tooltip("View Options").dropdown_menu(move |menu, _, _| {
                            let toggle = |label: &'static str, on: bool, set: fn(&mut crate::settings::LogSettings, bool)| {
                                let entity = entity.clone();
                                PopupMenuItem::new(label).checked(on).on_click(move |_, _, cx| {
                                    Settings::update(cx, |s| set(&mut s.log, !on));
                                    entity.update(cx, |this, cx| {
                                        this.rebuild_changes(cx);
                                        cx.notify();
                                    });
                                })
                            };
                            menu.label("Group By")
                                .item(toggle("Directory", settings.changes_by_directory, |s, v| s.changes_by_directory = v))
                                .item(toggle("Module", settings.changes_by_module, |s, v| s.changes_by_module = v))
                        })
                    }),
            )
            .child(
                div().flex_1().min_h_0().child(
                    tree(&self.changes, move |ix, entry, _, _, _| {
                        let palette = &tree_palette;
                        let item = entry.item();
                        let id = item.id.to_string();
                        let path = id.strip_prefix(FILE_PREFIX).map(str::to_owned);
                        let kind = path.as_ref().and_then(|p| kinds.get(p)).cloned();
                        let color = kind.as_ref().map_or(palette.text, |(k, _)| common::change_color(*k, &palette));
                        let entity = entity.clone();
                        let open_path = path.clone();
                        let (menu_entity, menu_path) = (entity.clone(), path.clone());
                        ListItem::new(ix)
                            .py_0()
                            .px_1()
                            .h(px(row_height()))

                            .on_click(move |event, _, cx| {
                                if event.click_count() == 2 {
                                    if let Some(path) = open_path.clone() {
                                        entity.update(cx, |this, cx| {
                                            if let Some(source) = this.diff_source_for(&format!("{FILE_PREFIX}{path}"), cx) {
                                                cx.emit(LogEvent::OpenDiff(source));
                                            }
                                        });
                                    }
                                }
                            })
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_1()
                                    .pl(px(entry.depth() as f32 * 14.))
                                    .text_sm()
                                    .when(entry.is_folder(), |el| {
                                        el.child(
                                            Icon::new(if entry.is_expanded() { IconName::ChevronDown } else { IconName::ChevronRight })
                                                .xsmall()
                                                .text_color(palette.text_secondary),
                                        )
                                    })
                                    .child(
                                        Icon::new(match &path {
                                            Some(p) if submodules.contains(p.as_str()) => IconName::FolderGit2,
                                            Some(p) => common::file_icon(p),
                                            None if id.starts_with(common::MODULE_PREFIX) => IconName::Layers,
                                            None => IconName::Folder,
                                        })
                                        .small()
                                        .text_color(palette.text_secondary),
                                    )
                                    .child(div().text_color(color).child(item.label.clone()))
                                    .when_some(kind.and_then(|(_, old)| old), |el, old| {
                                        el.child(div().text_xs().text_color(palette.text_secondary).child(format!("from {old}")))
                                    })
                                    .when(id.contains(DIR_PREFIX) || id.starts_with(common::MODULE_PREFIX), |el| {
                                        let n = counts.get(&item.id).copied().unwrap_or(0);
                                        el.child(
                                            div()
                                                .text_xs()
                                                .text_color(palette.text_secondary)
                                                .child(format!("{n} {}", if n == 1 { "file" } else { "files" })),
                                        )
                                    })
                                    .context_menu(move |menu, _, cx| match &menu_path {
                                        Some(path) => {
                                            let hash = menu_entity.read(cx).model.read(cx).selected_hash().map(str::to_owned);
                                            match hash {
                                                Some(hash) => change_menu(menu, &menu_entity, path, &hash, cx),
                                                None => menu,
                                            }
                                        }
                                        None => menu,
                                    }),
                            )
                    })
                    .size_full(),
                ),
            );

        let entity_for_links = cx.entity();
        let info = div()
            .id("commit-details")
            .size_full()
            .p_3()
            .text_sm()
            .overflow_y_scrollbar()
            .when_some(details, |el, d| {
                let labels = refs.for_commit(&d.hash);
                el.child(
                    v_flex()
                        .gap_2()
                        // The text is selectable and copies (Ctrl+C), as IntelliJ's details pane.
                        .child(message_text(&d.message, palette.link, &entity_for_links))
                        .child(
                            h_flex()
                                .gap_1()
                                .flex_wrap()
                                .child(selectable("commit-hash", 1, &d.hash[..d.hash.len().min(10)], palette.link).font_family(Some(cx.theme().mono_font_family.clone())))
                                .child(selectable(
                                    "commit-author",
                                    2,
                                    &format!("{} <{}> on {}", d.author_name, d.author_email, common::format_full_date(d.author_time)),
                                    palette.text_secondary,
                                )),
                        )
                        .when(d.committer_email != d.author_email || d.committer_time != d.author_time, |el| {
                            el.child(selectable(
                                "commit-committer",
                                3,
                                &format!(
                                    "committed by {} <{}> on {}",
                                    d.committer_name,
                                    d.committer_email,
                                    common::format_full_date(d.committer_time)
                                ),
                                palette.text_secondary,
                            ))
                        })
                        .when_some(d.signature.clone(), |el, signature| {
                            let color = if signature.is_good() {
                                palette.status_added
                            } else if signature.status == 'B' {
                                palette.status_conflict
                            } else {
                                palette.text_secondary
                            };
                            el.child(
                                h_flex()
                                    .gap_1()
                                    .text_color(color)
                                    .items_start()
                                    .child(Icon::new(if signature.is_good() { IconName::CircleCheck } else { IconName::TriangleAlert }).small())
                                    .child(div().flex_1().min_w_0().child(selectable("commit-signature", 4, &signature.describe(), color))),
                            )
                        })
                        .when(!labels.is_empty(), |el| {
                            let mut row = h_flex().gap_1().flex_wrap();
                            for label in labels {
                                row = row.child(ref_label(label, refs.current_branch.as_deref(), &palette));
                            }
                            el.child(row)
                        })
                        .child({
                            let n = d.containing_branches.len();
                            let all = self.show_all_branches || n <= 5;
                            let shown = if all { &d.containing_branches[..] } else { &d.containing_branches[..5] };
                            let text = match n {
                                0 => "Not in any branch".to_owned(),
                                1 => format!("In 1 branch: {}", shown[0]),
                                n => format!("In {n} branches: {}", shown.join(", ")),
                            };
                            // More than five: "Show all" lists the rest, as in IntelliJ.
                            v_flex().child(selectable("commit-branches", 5, &text, palette.text_secondary)).when(!all, |el| {
                                el.child(
                                    div()
                                        .id("commit-branches-all")
                                        .text_color(palette.link)
                                        .cursor_pointer()
                                        .child(format!("Show all {n}"))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.show_all_branches = true;
                                            cx.notify();
                                        })),
                                )
                            })
                        }),
                )
            });

        v_resizable("log-details-split")
            // Text measures itself unwrapped; out of the flow (absolute), a
            // long message or path stays inside the pane instead of widening
            // the window.
            .child(resizable_panel().child(div().relative().size_full().child(div().absolute().inset_0().child(changes))))
            .child(resizable_panel().size(px(220.)).child(div().relative().size_full().child(div().absolute().inset_0().child(info))))
    }
}

/// The branches and tags offered under the Go to Hash/Branch/Tag field.
struct GoToSuggestions {
    input: Entity<InputState>,
    names: Vec<(String, RefKind)>,
    log: Entity<LogView>,
    matches: Vec<usize>,
    selected: Option<usize>,
    _subscription: Subscription,
}

impl GoToSuggestions {
    fn new(input: Entity<InputState>, names: Vec<(String, RefKind)>, log: Entity<LogView>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&input, |this, _, cx| this.refresh(cx));
        let mut this = Self { input, names, log, matches: Vec::new(), selected: None, _subscription: subscription };
        this.refresh(cx);
        this
    }

    /// Matches for the typed text: names starting with it first; the first
    /// is preselected once something is typed.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().trim().to_lowercase();
        let mut matches: Vec<usize> = (0..self.names.len()).filter(|&ix| self.names[ix].0.to_lowercase().contains(&text)).collect();
        matches.sort_by_key(|&ix| !self.names[ix].0.to_lowercase().starts_with(&text));
        matches.truncate(8);
        if matches != self.matches {
            self.selected = (!text.is_empty() && !matches.is_empty()).then_some(0);
            self.matches = matches;
            cx.notify();
        }
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.matches.is_empty() {
            return;
        }
        let last = self.matches.len() as isize - 1;
        let next = self.selected.map_or(if delta > 0 { 0 } else { last }, |s| (s as isize + delta).clamp(0, last));
        self.selected = Some(next as usize);
        cx.notify();
    }

    fn chosen(&self) -> Option<String> {
        self.selected.and_then(|s| self.matches.get(s)).map(|&ix| self.names[ix].0.clone())
    }
}

impl Render for GoToSuggestions {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mut list = v_flex().min_h(px(row_height() * 4.));
        for (row, &ix) in self.matches.iter().enumerate() {
            let (name, kind) = self.names[ix].clone();
            let (icon, color) = match kind {
                RefKind::Tag => (IconName::Tag, palette.ref_tag),
                RefKind::RemoteBranch => (IconName::GitBranch, palette.ref_remote),
                RefKind::LocalBranch => (IconName::GitBranch, palette.ref_local),
            };
            let log = self.log.clone();
            list = list.child(
                h_flex()
                    .id(("goto-suggestion", row))
                    .h(px(row_height()))
                    .px_2()
                    .gap_1()
                    .rounded_sm()
                    .text_sm()
                    .cursor_pointer()
                    .when(self.selected == Some(row), |el| el.bg(palette.selection))
                    .when(self.selected != Some(row), |el| el.hover(|s| s.bg(palette.hover)))
                    .child(Icon::new(icon).xsmall().text_color(color))
                    .child(name.clone())
                    .on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        log.update(cx, |this, cx| this.go_to(&name, window, cx));
                    }),
            );
        }
        list
    }
}

/// Above this many selected commits, their changes come from one diff.
const MAX_COMBINED: usize = 100;

/// Git's empty tree, the "parent" of a root commit.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// The commit message as selectable text: URLs and hash-like words are
/// links (a hash goes to that commit in the Log), as in IntelliJ.
fn message_text(message: &str, link_color: gpui_kit::Hsla, entity: &Entity<LogView>) -> impl IntoElement {
    let (ranges, links): (Vec<_>, Vec<_>) = common::find_links(message).into_iter().unzip();
    let entity = entity.clone();
    selectable_text("commit-message", message.to_owned()).order(0).links(ranges, link_color, move |ix, window, cx| {
        match &links[ix] {
            common::Link::Url(url) => cx.open_url(url),
            common::Link::Commit(hash) => {
                let hash = hash.clone();
                entity.update(cx, |this, cx| this.go_to(&hash, window, cx));
            }
        }
    })
}

/// One line of the details pane; `order` places it for a drag across lines.
fn selectable(id: &'static str, order: u64, text: &str, color: gpui_kit::Hsla) -> SelectableText {
    selectable_text(id, text.to_owned()).order(order).color(color)
}

/// Right-click on a branch or tag in the branches panel: the branch popup's
/// actions plus Add to / Remove from Favorites.
fn branch_menu(
    menu: gpui_kit::component::menu::PopupMenu,
    entity: &Entity<LogView>,
    refs: &crate::git::RepositoryRefs,
    reference: &RefName,
    cx: &mut App,
) -> gpui_kit::component::menu::PopupMenu {
    use crate::ui::branches_popup;
    let model = entity.read(cx).model.clone();
    let remotes = branches_popup::remote_names(refs);
    let actions = branches_popup::branch_actions(&model, reference, refs.current_branch.as_deref(), &remotes);
    let mut menu = menu;
    for action in actions {
        let run = action.run.clone();
        menu = menu.item(PopupMenuItem::new(action.label).disabled(!action.enabled).on_click(move |_, window, cx| run(window, cx)));
    }
    let favorite = refs.favorites.contains(&reference.full_name);
    let full_name = reference.full_name.clone();
    menu.separator().item(PopupMenuItem::new(if favorite { "Remove from Favorites" } else { "Add to Favorites" }).on_click(
        move |_, _, cx| {
            model.update(cx, |model, cx| {
                if let Some(repo) = model.repository() {
                    let _ = crate::git::refs::set_favorite(repo, &full_name, !favorite);
                }
                model.reload(cx);
            })
        },
    ))
}

/// Where the speed search query appears in a tree label (case-insensitive).
fn label_match(label: &str, query: &str) -> Option<std::ops::Range<usize>> {
    if query.is_empty() {
        return None;
    }
    let start = label.to_ascii_lowercase().find(&query.to_ascii_lowercase())?;
    Some(start..start + query.len())
}

/// Folder ids of a tree and whether each is open.
fn collect_expanded(items: &[TreeItem], out: &mut HashMap<SharedString, bool>) {
    for item in items.iter().filter(|i| i.is_folder()) {
        out.insert(item.id.clone(), item.is_expanded());
        collect_expanded(&item.children, out);
    }
}

/// Opens or closes the folders that were known before, by id.
fn restore_expanded(items: &[TreeItem], expanded: &HashMap<SharedString, bool>) {
    for item in items.iter().filter(|i| i.is_folder()) {
        if let Some(&open) = expanded.get(&item.id) {
            item.clone().expanded(open);
        }
        restore_expanded(&item.children, expanded);
    }
}

fn collapse_all(mut item: TreeItem) -> TreeItem {
    if item.children.is_empty() {
        return item;
    }
    item.children = std::mem::take(&mut item.children).into_iter().map(collapse_all).collect();
    item.expanded(false)
}

fn expand_all(mut item: TreeItem) -> TreeItem {
    if item.children.is_empty() {
        return item;
    }
    item.children = std::mem::take(&mut item.children).into_iter().map(expand_all).collect();
    item.expanded(true)
}

/// Groups branches by their `/`-separated path into folders.
fn group_branches(branches: &[&RefName], scope: &str, short: impl Fn(&RefName) -> String) -> Vec<TreeItem> {
    #[derive(Default)]
    struct Node {
        dirs: std::collections::BTreeMap<String, Node>,
        leaves: Vec<(String, String)>,
    }
    let mut root = Node::default();
    for branch in branches {
        let name = short(branch);
        let mut parts: Vec<&str> = name.split('/').collect();
        let leaf = parts.pop().unwrap_or_default().to_owned();
        let mut node = &mut root;
        for part in parts {
            node = node.dirs.entry(part.to_owned()).or_default();
        }
        node.leaves.push((leaf, branch.full_name.clone()));
    }
    fn build(node: &Node, path: &str, scope: &str) -> Vec<TreeItem> {
        let mut items: Vec<TreeItem> = node
            .dirs
            .iter()
            .map(|(name, child)| {
                let path = format!("{path}/{name}");
                TreeItem::new(format!("dir:{scope}{path}"), name.clone()).children(build(child, &path, scope))
            })
            .collect();
        items.extend(node.leaves.iter().map(|(leaf, full)| TreeItem::new(format!("{BRANCH_PREFIX}{full}"), leaf.clone())));
        items
    }
    build(&root, "", scope)
}

fn ref_label(reference: &RefName, current_branch: Option<&str>, palette: &crate::theme::Palette) -> impl IntoElement {
    let is_current = reference.kind == RefKind::LocalBranch && Some(reference.name.as_str()) == current_branch;
    let (icon, color) = match reference.kind {
        _ if is_current => (IconName::GitBranch, palette.ref_head),
        RefKind::LocalBranch => (IconName::GitBranch, palette.ref_local),
        RefKind::RemoteBranch => (IconName::GitBranch, palette.ref_remote),
        RefKind::Tag => (IconName::Tag, palette.ref_tag),
    };
    h_flex()
        .flex_shrink_0()
        .gap_0p5()
        .mr_1p5()
        .text_xs()
        .child(Icon::new(icon).xsmall().text_color(color))
        .child(div().text_color(color).child(reference.name.clone()))
}

/// Right-click on a file in the commit details, after IntelliJ's.
fn change_menu(
    menu: gpui_kit::component::menu::PopupMenu,
    entity: &Entity<LogView>,
    path: &str,
    hash: &str,
    cx: &mut App,
) -> gpui_kit::component::menu::PopupMenu {
    let model = entity.read(cx).model.clone();
    let hash = hash.to_owned();
    let web_file = model.read(cx).web_repo().map(|w| (w.host.name(), w.file_url(&hash, path, None)));
    let short = hash[..hash.len().min(8)].to_owned();
    let path = path.to_owned();
    let (e_diff, e_blame, e_history, e_here) = (entity.clone(), entity.clone(), entity.clone(), entity.clone());
    let (p_diff, p_blame, p_history, p_here, p_copy) = (path.clone(), path.clone(), path.clone(), path.clone(), path.clone());
    let (h_blame, h_here) = (hash.clone(), hash.clone());
    let (m_get, m_revert) = (model.clone(), model.clone());
    let (p_get, h_get, p_revert, h_revert) = (path.clone(), hash.clone(), path.clone(), hash.clone());
    menu.item(PopupMenuItem::new("Show Diff").on_click(move |_, _, cx| {
        e_diff.update(cx, |this, cx| {
            if let Some(source) = this.diff_source_for(&format!("{FILE_PREFIX}{p_diff}"), cx) {
                cx.emit(LogEvent::OpenDiff(source));
            }
        })
    }))
    .item(PopupMenuItem::new("Open Repository Version").on_click({
        let (entity, path, hash) = (entity.clone(), path.clone(), hash.clone());
        move |_, _, cx| {
            let (path, revision) = (path.clone(), Some(hash.clone()));
            entity.update(cx, |_, cx| cx.emit(LogEvent::OpenFile { path, revision }))
        }
    }))
    .item(PopupMenuItem::new("Edit Source").on_click({
        let (entity, path) = (entity.clone(), path.clone());
        move |_, _, cx| {
            let path = path.clone();
            entity.update(cx, |_, cx| cx.emit(LogEvent::OpenFile { path, revision: None }))
        }
    }))
    .item(PopupMenuItem::new("Annotate Revision").on_click(move |_, _, cx| {
        e_blame.update(cx, |_, cx| cx.emit(LogEvent::Annotate { path: p_blame.clone(), revision: Some(h_blame.clone()) }))
    }))
    .separator()
    .item(PopupMenuItem::new("Show History").on_click(move |_, _, cx| {
        let path = p_history.clone();
        e_history.update(cx, |this, cx| this.open_history_tab(path, None, cx))
    }))
    .item(PopupMenuItem::new("History Up to Here").on_click(move |_, _, cx| {
        let (path, hash) = (p_here.clone(), h_here.clone());
        e_here.update(cx, |this, cx| this.open_history_tab(path, Some(hash), cx))
    }))
    .separator()
    .item(PopupMenuItem::new(format!("Get from Revision {short}")).on_click(move |_, _, cx| {
        let (path, hash) = (p_get.clone(), h_get.clone());
        m_get.update(cx, |model, cx| {
            model.run_operation("Get from Revision", move |repo| {
                repo.run(["checkout", hash.as_str(), "--", path.as_str()])?;
                Ok(format!("{path} restored from {}", &hash[..hash.len().min(8)]))
            }, cx)
        })
    }))
    .item(PopupMenuItem::new("Revert Selected Changes").on_click(move |_, _, cx| {
        let (path, hash) = (p_revert.clone(), h_revert.clone());
        m_revert.update(cx, |model, cx| {
            model.run_operation("Revert Changes", move |repo| {
                let patch = repo.run(["show", "--format=", "--binary", hash.as_str(), "--", path.as_str()])?;
                repo.run_with_input(["apply", "-R", "--3way", "-"], Some(&patch))?;
                Ok(format!("Changes to {path} reverted in the working tree"))
            }, cx)
        })
    }))
    .separator()
    .item(PopupMenuItem::new("Copy Path").on_click(move |_, _, cx| {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(p_copy.clone()))
    }))
    .when_some(web_file, |menu, (name, url)| {
        menu.item(PopupMenuItem::new(format!("Open on {name}")).on_click(move |_, _, cx| cx.open_url(&url)))
    })
}

/// Cherry-picks, skipping commits whose changes are already in the
/// current branch (git stops on them, "now empty"), as IntelliJ does
/// rather than leaving a cherry-pick in progress.
fn cherry_pick_skipping_empty(repo: &crate::git::Repository, args: &[String], picked: String) -> anyhow::Result<String> {
    let total = args.len() - 1;
    let mut skipped = 0;
    let mut result = repo.run(args);
    while let Err(error) = result {
        let text = format!("{error:#}");
        if skipped >= total || !(text.contains("is now empty") || text.contains("nothing to commit")) {
            return Err(error);
        }
        skipped += 1;
        result = repo.run(["cherry-pick", "--skip"]);
    }
    Ok(match (skipped, total) {
        (0, _) => picked,
        (s, t) if s == t && t == 1 => "Nothing to cherry-pick: the commit's changes are already in the current branch".to_owned(),
        (s, t) if s == t => "Nothing to cherry-pick: the commits' changes are already in the current branch".to_owned(),
        (s, t) => format!("Cherry-picked {} of {t} commits; {s} skipped, their changes are already in the current branch", t - s),
    })
}

fn commit_menu(
    menu: gpui_kit::component::menu::PopupMenu,
    entity: &Entity<LogView>,
    commit: &Commit,
    _: &mut Window,
    cx: &mut App,
) -> gpui_kit::component::menu::PopupMenu {
    let model = entity.read(cx).model.clone();
    let selected = entity.read(cx).selected_commits(cx);
    let multi = selected.len() > 1;
    let head = model.read(cx).refs().head_commit.clone();
    let is_head = head.as_deref() == Some(commit.hash.as_str());
    let hash = commit.hash.clone();
    let short = commit.short_hash().to_owned();
    // The newest loaded commit that has this one as a parent.
    let child = model.read(cx).commits().iter().find(|c| c.parents.contains(&commit.hash)).map(|c| c.hash.clone());
    let web = model.read(cx).web_repo().cloned();

    let op = |title: &'static str, args: Vec<String>, done: String| {
        let model = model.clone();
        move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
            let args = args.clone();
            let done = done.clone();
            model.update(cx, |model, cx| {
                model.run_operation(title, move |repo| {
                    repo.run(&args)?;
                    Ok(done)
                }, cx)
            });
        }
    };

    let picks: Vec<String> = selected.iter().map(|c| c.hash.clone()).collect();
    let copy_text = picks.iter().rev().cloned().collect::<Vec<_>>().join("\n");
    let mut cherry_pick = vec!["cherry-pick".to_owned()];
    cherry_pick.extend(picks.iter().cloned());
    let picked = if multi { format!("Cherry-picked {} commits", picks.len()) } else { format!("Cherry-picked {short}") };
    // Edit Message, Drop, Squash, Fixup and Interactively Rebase rewrite the
    // current branch: IntelliJ greys them out for commits not on it.
    let on_branch = model.read(cx).repository().is_some_and(|repo| {
        if picks.len() <= 20 {
            picks.iter().all(|pick| crate::git::rebase::is_on_current_branch(repo, pick))
        } else {
            // Many selected (Ctrl+A): one rev-list instead of a check each.
            let on_head: HashSet<String> = repo.run(["rev-list", "HEAD"]).unwrap_or_default().lines().map(str::to_owned).collect();
            picks.iter().all(|pick| on_head.contains(pick))
        }
    });
    // Push All up to Here: commits of the current branch (not detached).
    let can_push_here = model.read(cx).refs().current_branch.is_some()
        && model.read(cx).repository().is_some_and(|repo| crate::git::rebase::is_on_current_branch(repo, &commit.hash));
    let compare_model = model.clone();
    let compare = if selected.len() == 2 { Some((selected[0].hash.clone(), selected[1].hash.clone())) } else { None };
    let local_model = model.clone();
    let local_hash = hash.clone();
    // A file's History tab: IntelliJ puts the file's own actions first
    // (Show Diff, Open Repository Version, Annotate Revision, Get, …).
    let history_path = {
        let model = model.read(cx);
        let filter = model.filter();
        (filter.paths.len() == 1 && filter.lines.is_none() && !multi).then(|| {
            // The file's name in that commit, if it was renamed since.
            let path = filter.paths[0].clone();
            model
                .details()
                .filter(|d| d.hash == commit.hash && d.changes.len() == 1)
                .map(|d| d.changes[0].path.clone())
                .or_else(|| {
                    // Follow the file back from HEAD to that commit.
                    let output = model.repository()?.run(["log", "--follow", "--name-only", "--format=%H", "HEAD", "--", &path]).ok()?;
                    let mut current = "";
                    for line in output.lines().filter(|l| !l.is_empty()) {
                        if line.len() == 40 && line.bytes().all(|b| b.is_ascii_hexdigit()) {
                            current = line;
                        } else if current == commit.hash {
                            return Some(line.to_owned());
                        }
                    }
                    None
                })
                .unwrap_or(path)
        })
    };
    let menu = match history_path {
        Some(path) => change_menu(menu, entity, &path, &hash, cx).separator(),
        None => menu,
    };
    menu.item(PopupMenuItem::new("Copy Revision Number").on_click(move |_, _, cx| {
        cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()))
    }))
    .item(PopupMenuItem::new("Compare with Local").disabled(multi).on_click(move |_, _, cx| {
        let hash = local_hash.clone();
        local_model.update(cx, |m, cx| m.compare(hash, None, cx))
    }))
    .when_some(compare, |menu, (old, new)| {
        menu.item(PopupMenuItem::new("Compare Versions").on_click(move |_, _, cx| {
            let (old, new) = (old.clone(), new.clone());
            compare_model.update(cx, |m, cx| m.compare(old, Some(new), cx))
        }))
    })
    .item(PopupMenuItem::new("Show Repository at Revision").disabled(multi).on_click({
        let (model, hash, entity) = (model.clone(), hash.clone(), entity.clone());
        move |_, window, cx| {
            let entity = entity.clone();
            let open_file: crate::ui::revision_browser::OpenFile = std::rc::Rc::new(move |revision, path, _, cx| {
                entity.update(cx, |_, cx| cx.emit(LogEvent::OpenFile { path, revision: Some(revision) }))
            });
            crate::ui::revision_browser::open(model.clone(), hash.clone(), open_file, window, cx)
        }
    }))
    .item(PopupMenuItem::new("Create Patch…").on_click({
        let model = model.clone();
        let oldest = picks.first().cloned().unwrap_or_default();
        let newest = picks.last().cloned().unwrap_or_default();
        let label = if multi { format!("{}_{}", &oldest[..oldest.len().min(8)], &newest[..newest.len().min(8)]) } else { short.clone() };
        move |_, window, cx| {
            let Some(repository) = model.read(cx).repository().cloned() else { return };
            let old = crate::git::patch::parent_of(&repository, &oldest);
            let source = crate::ui::patch_dialogs::PatchSource::Commits { old, new: newest.clone(), label: label.clone() };
            crate::ui::patch_dialogs::create_patch(model.clone(), source, window, cx)
        }
    }))
    .separator()
    // Commits already on the current branch have nothing to pick.
    .item(PopupMenuItem::new("Cherry-Pick").disabled(on_branch).on_click({
        let model = model.clone();
        move |_, _, cx| {
            let (args, picked) = (cherry_pick.clone(), picked.clone());
            model.update(cx, |model, cx| model.run_operation("Cherry-Pick", move |repo| cherry_pick_skipping_empty(repo, &args, picked), cx));
        }
    }))
    .item(PopupMenuItem::new("Checkout Revision").disabled(multi).on_click(op(
        "Checkout",
        vec!["checkout".into(), "--detach".into(), hash.clone()],
        format!("Checked out {short}"),
    )))
    .separator()
    .item(PopupMenuItem::new("Reset Current Branch to Here…").disabled(multi).on_click({
        let model = model.clone();
        let hash = hash.clone();
        move |_, window, cx| dialogs::reset_to(model.clone(), hash.clone(), window, cx)
    }))
    .item(PopupMenuItem::new(if multi { "Revert Commits" } else { "Revert Commit" }).on_click(op(
        "Revert",
        // Newest first, so each revert applies cleanly on top of the last.
        ["revert".to_owned(), "--no-edit".to_owned()].into_iter().chain(picks.iter().rev().cloned()).collect(),
        if multi { format!("Reverted {} commits", picks.len()) } else { format!("Reverted {short}") },
    )))
    .item(PopupMenuItem::new("Undo Commit…").disabled(!is_head || multi).on_click(op(
        "Undo Commit",
        vec!["reset".into(), "--soft".into(), "HEAD~1".into()],
        "Commit undone; changes kept in the working tree".into(),
    )))
    .item(PopupMenuItem::new("Edit Commit Message…").disabled(multi || !on_branch).on_click({
        let model = model.clone();
        let hash = hash.clone();
        move |_, window, cx| rebase_dialog::reword(model.clone(), hash.clone(), window, cx)
    }))
    .item(PopupMenuItem::new(if multi { "Drop Commits" } else { "Drop Commit" }).disabled(!on_branch).on_click({
        let model = model.clone();
        let picks = picks.clone();
        move |_, window, cx| rebase_dialog::drop_commits(model.clone(), picks.clone(), window, cx)
    }))
    .item(PopupMenuItem::new("Squash Commits…").disabled(!multi || !on_branch).on_click({
        let model = model.clone();
        let picks = picks.clone();
        move |_, window, cx| rebase_dialog::squash(model.clone(), picks.clone(), window, cx)
    }))
    .item(PopupMenuItem::new("Fixup…").disabled(multi || !on_branch).on_click({
        let model = model.clone();
        let message = format!("fixup! {}", commit.subject);
        move |_, _, cx| model.update(cx, |m, cx| m.prefill_commit_message(message.clone(), cx))
    }))
    .item(PopupMenuItem::new("Squash Into…").disabled(multi || !on_branch).on_click({
        let model = model.clone();
        let message = format!("squash! {}", commit.subject);
        move |_, _, cx| model.update(cx, |m, cx| m.prefill_commit_message(message.clone(), cx))
    }))
    .item(PopupMenuItem::new("Interactively Rebase from Here…").disabled(multi || !on_branch).on_click({
        let model = model.clone();
        let hash = hash.clone();
        move |_, window, cx| rebase_dialog::open(model.clone(), hash.clone(), window, cx)
    }))
    .separator()
    // Opens the Push dialog with the commits up to this one, as IntelliJ
    // does, also for a branch without an upstream yet.
    .item(PopupMenuItem::new("Push All up to Here…").disabled(multi || !can_push_here).on_click({
        let model = model.clone();
        let hash = hash.clone();
        move |_, window, cx| dialogs::push_up_to(model.clone(), Some(hash.clone()), window, cx)
    }))
    .separator()
    .item(PopupMenuItem::new("New Branch…").disabled(multi).on_click({
        let model = model.clone();
        let hash = hash.clone();
        move |_, window, cx| dialogs::new_branch(model.clone(), hash.clone(), window, cx)
    }))
    .item(PopupMenuItem::new("New Tag…").disabled(multi).on_click({
        let model = model.clone();
        let hash = hash.clone();
        move |_, window, cx| dialogs::new_tag(model.clone(), hash.clone(), window, cx)
    }))
    .separator()
    .item(PopupMenuItem::new("Go to Child Commit").disabled(child.is_none() || multi).on_click({
        let model = model.clone();
        move |_, _, cx| model.update(cx, |m, cx| m.select_hash(child.clone(), cx))
    }))
    .item(PopupMenuItem::new("Go to Parent Commit").disabled(commit.parents.is_empty() || multi).on_click({
        let model = model.clone();
        let parent = commit.parents.first().cloned();
        move |_, _, cx| model.update(cx, |m, cx| m.select_hash(parent.clone(), cx))
    }))
    .when_some(web.filter(|_| !multi), |menu, web| {
        let url = web.commit_url(&hash);
        menu.separator().item(PopupMenuItem::new(format!("Open on {}", web.host.name())).on_click(move |_, _, cx| cx.open_url(&url)))
    })
}

impl Focusable for LogView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for LogView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let count = self.model.read(cx).commits().len();
        let empty = count == 0 && !self.model.read(cx).is_loading();

        let table = v_flex()
            .size_full()
            .child(self.render_filter_bar(cx))
            .children(self.render_compare_banner(cx))
            .child(
                div()
                    .id("log-table")
                    .key_context(CONTEXT)
                    .track_focus(&self.focus)
                    .on_action(cx.listener(Self::on_select_previous))
                    .on_action(cx.listener(Self::on_select_next))
                    .on_action(cx.listener(Self::on_select_first))
                    .on_action(cx.listener(Self::on_select_last))
                    .on_action(cx.listener(Self::on_page_up))
                    .on_action(cx.listener(Self::on_page_down))
                    .on_action(cx.listener(Self::on_extend_previous))
                    .on_action(cx.listener(Self::on_extend_next))
                    .on_action(cx.listener(Self::on_extend_first))
                    .on_action(cx.listener(Self::on_extend_last))
                    .on_action(cx.listener(Self::on_select_all))
                    .on_action(cx.listener(Self::on_copy_revision))
                    .on_action(cx.listener(Self::on_go_to_hash))
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .when(empty, |el| {
                        el.child(
                            v_flex()
                                .size_full()
                                .items_center()
                                .justify_center()
                                .text_color(palette.text_secondary)
                                .child("No commits matching filters"),
                        )
                    })
                    .when(!empty, |el| {
                        el.child(
                            uniform_list("log-rows", count, cx.processor(Self::render_rows))
                                .track_scroll(&self.scroll)
                                .size_full(),
                        )
                        .vertical_scrollbar(&self.scroll)
                        .when(self.column_drag.is_some(), |el| el.child(self.column_drag_tracker(cx)))
                    }),
            );

        let branches = resizable_panel()
            .size(px(if self.details_below { 180. } else { 240. }))
            .size_range(px(120.)..px(500.))
            .visible(self.show_branches)
            .child(self.render_branches(cx));
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
                                    .child(self.render_details(cx)),
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
                    .child(self.render_details(cx)),
            )
            .into_any_element()
    }
}

