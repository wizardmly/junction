//! Git tool window › Log tab: branches panel, filter bar, commit table with
//! graph, and the changes + details pane for the selected commit.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::{
    Selectable as _, WindowExt as _,
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
use crate::ui::common::{self, DIR_PREFIX, FILE_PREFIX, ROW_HEIGHT, tool_button};
use crate::ui::diff_view::DiffSource;
use crate::ui::dialogs;
use crate::ui::rebase_dialog;
use crate::ui::graph_paint::graph_canvas;

actions!(git_log, [SelectPrevious, SelectNext, SelectFirst, SelectLast, CopyRevision, GoToHash]);

const CONTEXT: &str = "GitLog";

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", SelectPrevious, Some(CONTEXT)),
        KeyBinding::new("down", SelectNext, Some(CONTEXT)),
        KeyBinding::new("home", SelectFirst, Some(CONTEXT)),
        KeyBinding::new("end", SelectLast, Some(CONTEXT)),
        KeyBinding::new("secondary-c", CopyRevision, Some(CONTEXT)),
        KeyBinding::new("secondary-f", GoToHash, Some(CONTEXT)),
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
    last_change_selection: Option<SharedString>,
    last_branch_selection: Option<SharedString>,
    show_branches: bool,
    show_details: bool,
    /// Commits reachable from HEAD among the loaded ones, for the Current
    /// Branch / Not Merged highlighters; keyed by the commit list and HEAD.
    head_reachable: Option<(usize, Option<String>, Rc<HashSet<String>>)>,
    /// Commits selected besides the model's selected (lead) commit, by
    /// Ctrl/Cmd-click or Shift-click.
    extra_selection: HashSet<String>,
    /// Where a Shift-click range starts.
    anchor: Option<usize>,
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
                    this.extra_selection.retain(|h| commits.iter().any(|c| &c.hash == h));
                    this.rebuild_branches(cx);
                    cx.notify();
                }
                RepoEvent::SelectionChanged | RepoEvent::DetailsLoaded => {
                    this.rebuild_changes(cx);
                    if let Some(ix) = this.model.read(cx).selected_index() {
                        this.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
                    }
                    cx.notify();
                }
                RepoEvent::Notify { .. } | RepoEvent::Compare { .. } | RepoEvent::PrefillCommitMessage(_) | RepoEvent::OpenLogTab { .. } => {}
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
            last_change_selection: None,
            last_branch_selection: None,
            show_branches: true,
            show_details: true,
            head_reachable: None,
            extra_selection: HashSet::new(),
            anchor: None,
            _search_debounce: None,
            _subscriptions: subscriptions,
        };
        this.rebuild_branches(cx);
        this
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
        self.model.update(cx, |model, cx| model.set_filter(filter, cx));
    }

    fn rebuild_branches(&mut self, cx: &mut Context<Self>) {
        let mut refs = self.model.read(cx).refs().clone();
        // Speed search: keep matching refs only, with every folder open.
        let query = self.branch_search.read(cx).value().trim().to_lowercase();
        let searching = !query.is_empty();
        if searching {
            refs.refs.retain(|r| r.name.to_lowercase().contains(&query));
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
        items.push(TreeItem::new("group:remote", "Remote").expanded(true).children(remote_items));
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
        }
        self.branches.update(cx, |tree, cx| tree.set_items(items, cx));
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
        let items = match (&combined, &details) {
            (Some((ranges, changes)), _) => {
                for change in changes {
                    self.change_kinds.insert(change.path.clone(), (change.kind, change.old_path.clone()));
                }
                self.combined = Some(ranges.clone());
                common::file_tree(changes.iter().map(|c| c.path.clone()), "")
            }
            (None, Some(details)) => {
                for change in &details.changes {
                    self.change_kinds.insert(change.path.clone(), (change.kind, change.old_path.clone()));
                }
                common::file_tree(details.changes.iter().map(|c| c.path.clone()), "")
            }
            (None, None) => Vec::new(),
        };
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

    /// Go to Hash / Branch / Tag (`Ctrl+F` in the Log).
    fn on_go_to_hash(&mut self, _: &GoToHash, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Hash, branch or tag"));
        let entity = cx.entity();
        window.open_dialog(cx, {
            let input = input.clone();
            move |dialog, _, _| {
            let input_ok = input.clone();
            let entity = entity.clone();
            dialog
                .title("Go to Hash/Branch/Tag")
                .w(px(420.))
                .child(Input::new(&input))
                .footer(
                    gpui_kit::component::dialog::DialogFooter::new()
                        .gap_2()
                        .child(gpui_kit::component::dialog::DialogClose::new().child(Button::new("goto-cancel").label("Cancel").outline()))
                        .child(gpui_kit::component::dialog::DialogAction::new().child(Button::new("goto-ok").label("Go").primary())),
                )
                .on_ok(move |_, window, cx| {
                    let text = input_ok.read(cx).value().trim().to_owned();
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

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.extra_selection.clear();
        let model = self.model.read(cx);
        let count = model.commits().len();
        if count == 0 {
            return;
        }
        let current = model.selected_index().map(|ix| ix as isize).unwrap_or(-1);
        let next = (current + delta).clamp(0, count as isize - 1) as usize;
        self.model.update(cx, |model, cx| model.select_index(next, cx));
    }

    fn on_select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(-1, cx);
    }

    fn on_select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(1, cx);
    }

    fn on_select_first(&mut self, _: &SelectFirst, _: &mut Window, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.select_index(0, cx));
    }

    fn on_select_last(&mut self, _: &SelectLast, _: &mut Window, cx: &mut Context<Self>) {
        let last = self.model.read(cx).commits().len().saturating_sub(1);
        self.model.update(cx, |model, cx| model.select_index(last, cx));
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
        let mut authors: Vec<String> = model.commits().iter().map(|c| c.author_name.clone()).collect();
        authors.sort();
        authors.dedup();
        authors.truncate(40);

        let branch_label = match filter.branches.as_slice() {
            [] => "Branch".to_owned(),
            [one] => format!("Branch: {one}"),
            many => format!("Branch: {} selected", many.len()),
        };
        let user_label = filter.author.as_ref().map_or("User".to_owned(), |a| format!("User: {a}"));
        let date_label = match (&filter.since, &filter.until) {
            (Some(since), Some(until)) => format!("Date: {since} – {until}"),
            (Some(since), None) => format!("Date: since {since}"),
            (None, Some(until)) => format!("Date: until {until}"),
            (None, None) => "Date".to_owned(),
        };
        let entity = cx.entity();

        let filter_button = |id: &'static str, label: String, active: bool| {
            Button::new(id)
                .ghost()
                .xsmall()
                .label(label)
                .when(active, |b| b.selected(true))
                .icon(Icon::new(IconName::ChevronDown).xsmall())
        };

        h_flex()
            .h(px(32.))
            .px_1()
            .gap_1()
            .border_b_1()
            .border_color(palette.border)
            .child(
                div().w(px(260.)).child(
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
                move |mut menu, _, _| {
                    let set = |entity: &Entity<LogView>, branches: Vec<String>| {
                        let entity = entity.clone();
                        move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
                            let branches = branches.clone();
                            entity.update(cx, |this, cx| this.update_filter(cx, |f| f.branches = branches));
                        }
                    };
                    menu = menu.item(PopupMenuItem::new("All").checked(selected.is_empty()).on_click(set(&entity, vec![])));
                    if let Some(current) = &refs.current_branch {
                        menu = menu.item(PopupMenuItem::new("HEAD").on_click(set(&entity, vec!["HEAD".into()])));
                        let _ = current;
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
            .child(filter_button("filter-user", user_label, filter.author.is_some()).dropdown_menu({
                let entity = entity.clone();
                let current = filter.author.clone();
                move |mut menu, _, _| {
                    let set = |author: Option<String>| {
                        let entity = entity.clone();
                        move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
                            let author = author.clone();
                            entity.update(cx, |this, cx| this.update_filter(cx, |f| f.author = author));
                        }
                    };
                    menu = menu.item(PopupMenuItem::new("All").checked(current.is_none()).on_click(set(None)));
                    if let Some(email) = &user_email {
                        menu = menu.item(
                            PopupMenuItem::new("me")
                                .checked(current.as_deref() == Some(email.as_str()))
                                .on_click(set(Some(email.clone()))),
                        );
                    }
                    menu = menu.separator();
                    for author in &authors {
                        menu = menu.item(
                            PopupMenuItem::new(author.clone())
                                .checked(current.as_ref() == Some(author))
                                .on_click(set(Some(author.clone()))),
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
                filter_button("filter-path", label, !filter.paths.is_empty()).dropdown_menu(move |mut menu, _, _| {
                    let clear = entity.clone();
                    menu = menu.item(PopupMenuItem::new("All").checked(current.is_empty()).on_click(move |_, _, cx| {
                        clear.update(cx, |this, cx| this.update_filter(cx, |f| f.paths.clear()))
                    }));
                    let pick = entity.clone();
                    menu = menu.item(PopupMenuItem::new("Select Folders…").on_click(move |_, _, cx| {
                        pick.update(cx, |this, cx| this.select_paths(cx))
                    }));
                    if !current.is_empty() {
                        menu = menu.separator();
                        for path in &current {
                            menu = menu.item(PopupMenuItem::new(path.clone()).checked(true));
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

    /// Paths › Select Folders…: folders or files to limit the Log to.
    fn select_paths(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.model.read(cx).repository().map(|r| r.root().to_path_buf()) else { return };
        let receiver = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: true,
            multiple: true,
            prompt: Some("Filter by Paths".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else { return };
            let relative: Vec<String> = paths
                .iter()
                .filter_map(|p| p.strip_prefix(&root).ok())
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .map(|p| if p.is_empty() { ".".to_owned() } else { p })
                .collect();
            if relative.is_empty() {
                return;
            }
            this.update(cx, |this, cx| this.update_filter(cx, |f| f.paths = relative)).ok();
        })
        .detach();
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

    fn render_rows(&mut self, range: std::ops::Range<usize>, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let palette = cx.palette().clone();
        let focused = self.focus.contains_focused(window, cx);
        let model = self.model.read(cx);
        let commits = model.commits().clone();
        let graph = model.graph().clone();
        let refs = model.refs().clone();
        let selected = model.selected_index();
        let extra = self.extra_selection.clone();
        let me = model.user_email().map(str::to_owned);
        let log = Settings::get(cx).log.clone();
        let show_hash = log.show_hash;
        let entity = cx.entity();
        let reachable = if log.highlight_current_branch || log.highlight_not_merged { Some(self.head_reachable(cx)) } else { None };
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

                let mut subject = h_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_hidden()
                    .child(graph_canvas(row, &palette, is_head));
                let shown = if log.compact_refs { 1 } else { 4 };
                let mut ref_labels = h_flex().flex_shrink_0();
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
                        .h(px(ROW_HEIGHT))
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
                                    .w(px(150.))
                                    .flex_shrink_0()
                                    .pl_2()
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
                                    .w(px(140.))
                                    .flex_shrink_0()
                                    .pl_2()
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
                                    .w(px(76.))
                                    .flex_shrink_0()
                                    .pl_2()
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
        let entity = cx.entity();
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(px(32.))
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
                    .child(div().flex_1().ml_1().child(Input::new(&self.branch_search).xsmall().cleanable(true))),
            )
            .child(
                div().flex_1().min_h_0().child(
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
                            .h(px(ROW_HEIGHT))
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
                                    .child(item.label.clone())
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
                    .h(px(32.))
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
                            .h(px(ROW_HEIGHT))

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
                                            Some(p) => common::file_icon(p),
                                            None => IconName::Folder,
                                        })
                                        .small()
                                        .text_color(palette.text_secondary),
                                    )
                                    .child(div().text_color(color).child(item.label.clone()))
                                    .when_some(kind.and_then(|(_, old)| old), |el, old| {
                                        el.child(div().text_xs().text_color(palette.text_secondary).child(format!("from {old}")))
                                    })
                                    .when(id.contains(DIR_PREFIX), |el| {
                                        let n = counts.get(&item.id).copied().unwrap_or(0);
                                        el.child(
                                            div()
                                                .text_xs()
                                                .text_color(palette.text_secondary)
                                                .child(format!("{n} {}", if n == 1 { "file" } else { "files" })),
                                        )
                                    })
                                    .context_menu(move |menu, _, cx| match &menu_path {
                                        Some(path) => change_menu(menu, &menu_entity, path, cx),
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
                        .child(div().whitespace_normal().child(message_text(&d.message, &entity_for_links, &palette)))
                        .child(
                            h_flex()
                                .gap_1()
                                .flex_wrap()
                                .text_color(palette.text_secondary)
                                .child(
                                    div()
                                        .font_family(cx.theme().mono_font_family.clone())
                                        .text_color(palette.link)
                                        .child(d.hash[..d.hash.len().min(10)].to_owned()),
                                )
                                .child(format!(
                                    "{} <{}> on {}",
                                    d.author_name,
                                    d.author_email,
                                    common::format_full_date(d.author_time)
                                )),
                        )
                        .when(d.committer_email != d.author_email || d.committer_time != d.author_time, |el| {
                            el.child(div().text_color(palette.text_secondary).child(format!(
                                "committed by {} <{}> on {}",
                                d.committer_name,
                                d.committer_email,
                                common::format_full_date(d.committer_time)
                            )))
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
                                    .child(div().flex_1().min_w_0().whitespace_normal().child(signature.describe())),
                            )
                        })
                        .when(!labels.is_empty(), |el| {
                            let mut row = h_flex().gap_1().flex_wrap();
                            for label in labels {
                                row = row.child(ref_label(label, refs.current_branch.as_deref(), &palette));
                            }
                            el.child(row)
                        })
                        .child(div().text_color(palette.text_secondary).child(match d.containing_branches.len() {
                            0 => "Not in any branch".to_owned(),
                            n if n <= 5 => format!("In {} branches: {}", n, d.containing_branches.join(", ")),
                            n => format!("In {} branches: {}, …", n, d.containing_branches[..5].join(", ")),
                        })),
                )
            });

        v_resizable("log-details-split")
            .child(resizable_panel().child(changes))
            .child(resizable_panel().size(px(220.)).child(info))
    }
}

/// Git's empty tree, the "parent" of a root commit.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// The commit message with URLs and commit hashes clickable, as in IntelliJ.
fn message_text(message: &str, entity: &Entity<LogView>, palette: &crate::theme::Palette) -> gpui_kit::InteractiveText {
    let links = common::find_links(message);
    let style = gpui_kit::HighlightStyle {
        color: Some(palette.link),
        underline: Some(gpui_kit::UnderlineStyle { thickness: px(1.), color: Some(palette.link), wavy: false }),
        ..Default::default()
    };
    let text = gpui_kit::StyledText::new(message.to_owned()).with_highlights(links.iter().map(|(range, _)| (range.clone(), style)));
    let ranges = links.iter().map(|(range, _)| range.clone()).collect();
    let entity = entity.clone();
    gpui_kit::InteractiveText::new("commit-message", text).on_click(ranges, move |ix, window, cx| match &links[ix].1 {
        common::Link::Url(url) => cx.open_url(url),
        common::Link::Commit(hash) => {
            let hash = hash.clone();
            entity.update(cx, |this, cx| this.go_to(&hash, window, cx));
        }
    })
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
    cx: &mut App,
) -> gpui_kit::component::menu::PopupMenu {
    let model = entity.read(cx).model.clone();
    let Some(hash) = model.read(cx).selected_hash().map(str::to_owned) else { return menu };
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
    // Push All up to Here: the current branch's upstream, when the commit is on it.
    let push_target = {
        let refs = model.read(cx).refs();
        let current = refs.current_branch.clone();
        let upstream = refs.local_branches().find(|r| Some(&r.name) == current.as_ref()).and_then(|r| r.upstream.clone());
        let repository = model.read(cx).repository().cloned();
        upstream
            .filter(|_| repository.is_some_and(|repo| crate::git::rebase::is_on_current_branch(&repo, &commit.hash)))
            .and_then(|u| u.split_once('/').map(|(r, b)| (r.to_owned(), b.to_owned())))
    };
    let compare_model = model.clone();
    let compare = if selected.len() == 2 { Some((selected[0].hash.clone(), selected[1].hash.clone())) } else { None };
    let local_model = model.clone();
    let local_hash = hash.clone();
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
    .item(PopupMenuItem::new("Cherry-Pick").disabled(is_head && !multi).on_click(op("Cherry-Pick", cherry_pick, picked)))
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
    .item(PopupMenuItem::new("Edit Commit Message…").disabled(multi).on_click({
        let model = model.clone();
        let hash = hash.clone();
        move |_, window, cx| rebase_dialog::reword(model.clone(), hash.clone(), window, cx)
    }))
    .item(PopupMenuItem::new(if multi { "Drop Commits" } else { "Drop Commit" }).on_click({
        let model = model.clone();
        let picks = picks.clone();
        move |_, window, cx| rebase_dialog::drop_commits(model.clone(), picks.clone(), window, cx)
    }))
    .item(PopupMenuItem::new("Squash Commits…").disabled(!multi).on_click({
        let model = model.clone();
        let picks = picks.clone();
        move |_, window, cx| rebase_dialog::squash(model.clone(), picks.clone(), window, cx)
    }))
    .item(PopupMenuItem::new("Fixup…").disabled(multi).on_click({
        let model = model.clone();
        let message = format!("fixup! {}", commit.subject);
        move |_, _, cx| model.update(cx, |m, cx| m.prefill_commit_message(message.clone(), cx))
    }))
    .item(PopupMenuItem::new("Squash Into…").disabled(multi).on_click({
        let model = model.clone();
        let message = format!("squash! {}", commit.subject);
        move |_, _, cx| model.update(cx, |m, cx| m.prefill_commit_message(message.clone(), cx))
    }))
    .item(PopupMenuItem::new("Interactively Rebase from Here…").disabled(multi).on_click({
        let model = model.clone();
        let hash = hash.clone();
        move |_, window, cx| rebase_dialog::open(model.clone(), hash.clone(), window, cx)
    }))
    .separator()
    .item(PopupMenuItem::new("Push All up to Here…").disabled(multi || push_target.is_none()).on_click({
        let model = model.clone();
        let hash = hash.clone();
        let short = short.clone();
        move |_, _, cx| {
            let Some((remote, branch)) = push_target.clone() else { return };
            let hash = hash.clone();
            let short = short.clone();
            model.update(cx, |model, cx| {
                model.run_operation("Push", move |repo| {
                    repo.run(["push", remote.as_str(), &format!("{hash}:refs/heads/{branch}")])?;
                    Ok(format!("Pushed commits up to {short} to {remote}/{branch}"))
                }, cx)
            });
        }
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
                    }),
            );

        h_resizable("log-split")
            .child(
                resizable_panel()
                    .size(px(240.))
                    .size_range(px(120.)..px(500.))
                    .visible(self.show_branches)
                    .child(self.render_branches(cx)),
            )
            .child(resizable_panel().child(table))
            .child(
                resizable_panel()
                    .size(px(380.))
                    .size_range(px(200.)..px(800.))
                    .visible(self.show_details)
                    .child(self.render_details(cx)),
            )
    }
}

