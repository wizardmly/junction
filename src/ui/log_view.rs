//! Git tool window › Log tab: branches panel, filter bar, commit table with
//! graph, and the changes + details pane for the selected commit.

use std::collections::{HashMap, HashSet};
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
    AnyElement, App, AppContext as _, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Render, ScrollStrategy,
    SharedString, Styled as _, Subscription, Task, UniformListScrollHandle,
    Window, actions, div, prelude::FluentBuilder as _, px, uniform_list,
};

use crate::git::{Commit, FileChangeKind, LogFilter, RefKind, RefName};
use crate::model::{RepoEvent, RepoModel};
use crate::theme::ActivePalette as _;
use crate::ui::common::{self, DIR_PREFIX, FILE_PREFIX, ROW_HEIGHT, tool_button};
use crate::ui::diff_view::DiffSource;
use crate::ui::dialogs;
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
}

impl EventEmitter<LogEvent> for LogView {}

const BRANCH_PREFIX: &str = "ref:";

pub struct LogView {
    model: Entity<RepoModel>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    search: Entity<InputState>,
    branches: Entity<TreeState>,
    changes: Entity<TreeState>,
    change_kinds: HashMap<String, (FileChangeKind, Option<String>)>,
    change_counts: HashMap<SharedString, usize>,
    last_change_selection: Option<SharedString>,
    last_branch_selection: Option<SharedString>,
    show_branches: bool,
    show_details: bool,
    show_hash: bool,
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
                RepoEvent::Notify { .. } => {}
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
            changes,
            change_kinds: HashMap::new(),
            change_counts: HashMap::new(),
            last_change_selection: None,
            last_branch_selection: None,
            show_branches: true,
            show_details: true,
            show_hash: false,
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

    fn update_filter(&mut self, cx: &mut Context<Self>, edit: impl FnOnce(&mut LogFilter)) {
        let mut filter = self.model.read(cx).filter().clone();
        edit(&mut filter);
        self.model.update(cx, |model, cx| model.set_filter(filter, cx));
    }

    fn rebuild_branches(&mut self, cx: &mut Context<Self>) {
        let refs = self.model.read(cx).refs().clone();
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
                TreeItem::new("group:tags", "Tags").children(
                    tags.iter().map(|t| TreeItem::new(format!("{BRANCH_PREFIX}{}", t.full_name), t.name.clone())),
                ),
            );
        }
        self.branches.update(cx, |tree, cx| tree.set_items(items, cx));
    }

    fn rebuild_changes(&mut self, cx: &mut Context<Self>) {
        let details = self.model.read(cx).details().cloned();
        self.change_kinds.clear();
        let items = match &details {
            Some(details) => {
                for change in &details.changes {
                    self.change_kinds.insert(change.path.clone(), (change.kind, change.old_path.clone()));
                }
                common::file_tree(details.changes.iter().map(|c| c.path.clone()), "")
            }
            None => Vec::new(),
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
        let hash = self.model.read(cx).selected_hash()?.to_owned();
        let old_path = self.change_kinds.get(path).and_then(|(_, old)| old.clone());
        Some(DiffSource::Commit { hash, path: path.to_owned(), old_path })
    }

    /// Mouse selection: plain click selects one commit, Ctrl/Cmd-click
    /// toggles, Shift-click selects the range from the last plain click.
    fn click_row(&mut self, ix: usize, toggle: bool, range: bool, cx: &mut Context<Self>) {
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
        let date_label = filter.since.as_ref().map_or("Date".to_owned(), |s| format!("Date: since {s}"));
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
                div().w(px(220.)).child(
                    Input::new(&self.search)
                        .xsmall()
                        .cleanable(true)
                        .prefix(Icon::new(IconName::Search).xsmall().text_color(palette.text_secondary)),
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
            .child(filter_button("filter-date", date_label, filter.since.is_some()).dropdown_menu({
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
                            entity.update(cx, |this, cx| this.update_filter(cx, |f| f.since = since));
                        }));
                    }
                    menu
                }
            }))
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
                        let show_hash = self.show_hash;
                        move |menu, _, _| {
                            let entity = entity.clone();
                            menu.label("Show Columns").item(PopupMenuItem::new("Hash").checked(show_hash).on_click(
                                move |_, _, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.show_hash = !this.show_hash;
                                        cx.notify();
                                    })
                                },
                            ))
                        }
                    }),
            )
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
        let show_hash = self.show_hash;
        let entity = cx.entity();

        range
            .filter_map(|ix| {
                let commit = commits.get(ix)?;
                let row = graph.rows.get(ix).cloned().unwrap_or_default();
                let is_selected = selected == Some(ix) || extra.contains(&commit.hash);
                let is_head = refs.head_commit.as_deref() == Some(commit.hash.as_str());
                let mine = me.as_deref().is_some_and(|me| me == commit.author_email);
                let labels = refs.for_commit(&commit.hash);
                let text_color = if is_selected && focused && !palette.dark { palette.text } else { palette.text };

                let mut subject = h_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_hidden()
                    .child(graph_canvas(row, &palette, is_head));
                for label in labels.iter().take(4) {
                    subject = subject.child(ref_label(label, refs.current_branch.as_deref(), &palette));
                }
                if labels.len() > 4 {
                    subject = subject.child(
                        div().mr_1().text_xs().text_color(palette.text_secondary).child(format!("+{}", labels.len() - 4)),
                    );
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
                        .child(
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
                        .child(
                            div()
                                .w(px(140.))
                                .flex_shrink_0()
                                .pl_2()
                                .whitespace_nowrap()
                                .text_color(palette.text_secondary)
                                .child(common::format_date(commit.author_time)),
                        )
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
                    )),
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
                        _ if !self.extra_selection.is_empty() => {
                            format!("{} commits selected · changes of the selected commit", self.extra_selection.len() + 1)
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
                                    }),
                            )
                    })
                    .size_full(),
                ),
            );

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
                        .child(div().whitespace_normal().child(d.message.clone()))
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
    menu.item(PopupMenuItem::new("Copy Revision Number").on_click(move |_, _, cx| {
        cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()))
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

