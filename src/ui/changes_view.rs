//! The Changes tool window: "Changes Between X and local" for Compare with
//! Local, Show Diff with Working Tree, Compare Versions and branch compares.
//! Each comparison is a tab; its files are a directory tree under the
//! repository root, and selecting a file shows its diff in the editor area.

use std::collections::HashMap;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable as _, Icon, Sizable as _, h_flex,
    button::{Button, ButtonVariants as _},
    list::ListItem,
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem},
    tree::{TreeItem, TreeState, tree},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, ClipboardItem, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px,
};

use crate::git::FileChangeKind;
use crate::model::RepoModel;
use crate::theme::ActivePalette as _;
use crate::ui::common::{self, DIR_PREFIX, FILE_PREFIX, ROW_HEIGHT, tool_button};
use crate::ui::diff_view::DiffSource;

const ROOT_ID: &str = "r:";

pub enum ChangesEvent {
    OpenDiff(DiffSource),
    OpenFile(String),
    /// The last tab was closed: the tool window goes away.
    Closed,
    Hide,
}

/// One comparison: `old` against `new`, or against the working tree.
struct Comparison {
    old: String,
    new: Option<String>,
    files: Vec<crate::git::log::FileChange>,
    error: Option<String>,
}

impl Comparison {
    /// The tab label when several comparisons are open.
    fn short_title(&self) -> String {
        self.title().trim_start_matches("Changes Between ").to_owned()
    }

    fn title(&self) -> String {
        let short = |r: &str| if r.len() == 40 { r[..8].to_owned() } else { r.to_owned() };
        match &self.new {
            None => format!("Changes Between {} and local", short(&self.old)),
            Some(new) => format!("Changes Between {} and {}", short(&self.old), short(new)),
        }
    }
}

pub struct ChangesView {
    model: Entity<RepoModel>,
    tabs: Vec<Comparison>,
    active: usize,
    tree: Entity<TreeState>,
    /// Kind and old path of each file of the active tab.
    kinds: HashMap<String, (FileChangeKind, Option<String>)>,
    counts: HashMap<SharedString, usize>,
    group_by_directory: bool,
    last_selection: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ChangesEvent> for ChangesView {}

impl ChangesView {
    pub fn new(model: Entity<RepoModel>, cx: &mut Context<Self>) -> Self {
        let tree = cx.new(|cx| TreeState::new(cx));
        let subscriptions = vec![cx.observe(&tree, |this, tree, cx| {
            let selected = tree.read(cx).selected_item().map(|item| item.id.clone());
            if selected != this.last_selection {
                this.last_selection = selected.clone();
                // Selecting a file previews its diff, as IntelliJ does.
                if let Some(source) = selected.and_then(|id| this.diff_source(&id)) {
                    cx.emit(ChangesEvent::OpenDiff(source));
                }
            }
        })];
        Self {
            model,
            tabs: Vec::new(),
            active: 0,
            tree,
            kinds: HashMap::new(),
            counts: HashMap::new(),
            group_by_directory: true,
            last_selection: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    /// Opens a comparison in a new tab, or selects the tab already showing it.
    pub fn compare(&mut self, old: String, new: Option<String>, cx: &mut Context<Self>) {
        match self.tabs.iter().position(|t| t.old == old && t.new == new) {
            Some(ix) => {
                self.active = ix;
                self.reload(cx);
            }
            None => {
                self.tabs.push(Comparison { old, new, files: Vec::new(), error: None });
                self.active = self.tabs.len() - 1;
                self.reload(cx);
            }
        }
    }

    /// Re-reads the active comparison's files (Refresh).
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(repository) = self.model.read(cx).repository().cloned() else { return };
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        match crate::git::diff::changed_files(&repository, &tab.old, tab.new.as_deref()) {
            Ok(files) => {
                tab.files = files;
                tab.error = None;
            }
            Err(error) => {
                tab.files.clear();
                tab.error = Some(error.to_string());
            }
        }
        self.rebuild(None, cx);
    }

    fn rebuild(&mut self, expanded: Option<bool>, cx: &mut Context<Self>) {
        self.kinds.clear();
        self.counts.clear();
        let Some(tab) = self.tabs.get(self.active) else {
            self.tree.update(cx, |tree, cx| tree.set_items(Vec::new(), cx));
            cx.notify();
            return;
        };
        for file in &tab.files {
            self.kinds.insert(file.path.clone(), (file.kind, file.old_path.clone()));
        }
        let paths = tab.files.iter().map(|f| f.path.clone());
        let children = if self.group_by_directory { common::file_tree(paths, "") } else { common::flat_file_list(paths, "") };
        let root_name = self
            .model
            .read(cx)
            .repository()
            .and_then(|r| r.root().file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let mut root = TreeItem::new(ROOT_ID, root_name).expanded(true).children(children);
        if let Some(expand) = expanded {
            root = set_expanded(root, expand);
            // The root stays open so Collapse All still shows it.
            root = root.expanded(true);
        }
        let items = if tab.files.is_empty() { Vec::new() } else { vec![root] };
        common::count_files(&items, &mut self.counts);
        self.last_selection = None;
        self.tree.update(cx, |tree, cx| {
            tree.set_items(items, cx);
            tree.set_selected_index(None, cx);
        });
        cx.notify();
    }

    fn diff_source(&self, id: &str) -> Option<DiffSource> {
        let path = id.strip_prefix(FILE_PREFIX)?;
        let tab = self.tabs.get(self.active)?;
        let old_path = self.kinds.get(path).and_then(|(_, old)| old.clone());
        Some(DiffSource::Between { old: tab.old.clone(), new: tab.new.clone(), path: path.to_owned(), old_path })
    }

    /// Files under the selected node: one file, a directory's, or all.
    fn selected_files(&self, cx: &App) -> Vec<String> {
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        let Some(id) = self.tree.read(cx).selected_item().map(|item| item.id.to_string()) else { return Vec::new() };
        if let Some(path) = id.strip_prefix(FILE_PREFIX) {
            return vec![path.to_owned()];
        }
        let prefix = id.strip_prefix(DIR_PREFIX).map(|d| format!("{d}/"));
        tab.files.iter().map(|f| f.path.clone()).filter(|p| prefix.as_ref().is_none_or(|d| p.starts_with(d.as_str()))).collect()
    }

    fn select_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.active = ix;
        self.reload(cx);
    }

    fn close_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        self.tabs.remove(ix);
        if self.tabs.is_empty() {
            self.active = 0;
            self.rebuild(None, cx);
            cx.emit(ChangesEvent::Closed);
            return;
        }
        if self.active >= ix && self.active > 0 {
            self.active -= 1;
        }
        self.reload(cx);
    }

    fn close_all(&mut self, cx: &mut Context<Self>) {
        self.tabs.clear();
        self.active = 0;
        self.rebuild(None, cx);
        cx.emit(ChangesEvent::Closed);
    }

    /// Swap Sides: compares the two revisions the other way round.
    fn swap_sides(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let Some(new) = tab.new.take() else { return };
        tab.new = Some(std::mem::replace(&mut tab.old, new));
        self.reload(cx);
    }

    /// Get: replaces the selected local files with their version in the
    /// compared revision; files the revision lacks are deleted.
    fn get_from_revision(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        if tab.new.is_some() {
            return;
        }
        let files = self.selected_files(cx);
        if files.is_empty() {
            return;
        }
        let revision = tab.old.clone();
        let (restore, remove): (Vec<String>, Vec<String>) = files
            .into_iter()
            .partition(|p| !matches!(self.kinds.get(p), Some((FileChangeKind::Added | FileChangeKind::Copied, _))));
        let entity = cx.entity();
        self.model.update(cx, |model, cx| {
            model.run_operation(
                "Get from Revision",
                move |repo| {
                    if !restore.is_empty() {
                        let mut args = vec!["checkout".to_owned(), revision.clone(), "--".to_owned()];
                        args.extend(restore.iter().cloned());
                        repo.run(args.iter().map(String::as_str))?;
                    }
                    for path in &remove {
                        let _ = repo.run(["rm", "-q", "-f", "--ignore-unmatch", "--", path.as_str()]);
                        let _ = std::fs::remove_file(repo.root().join(path));
                    }
                    let n = restore.len() + remove.len();
                    Ok(format!("{n} {} restored from {}", if n == 1 { "file" } else { "files" }, &revision[..revision.len().min(8)]))
                },
                cx,
            )
        });
        // The operation runs in the background; refresh once it has had time.
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(600)).await;
            entity.update(cx, |this, cx| this.reload(cx));
        })
        .detach();
    }

    fn file_menu(menu: PopupMenu, entity: &Entity<Self>, path: &str, local: bool) -> PopupMenu {
        let (e_diff, e_source, e_get) = (entity.clone(), entity.clone(), entity.clone());
        let (p_diff, p_source, p_copy) = (path.to_owned(), path.to_owned(), path.to_owned());
        menu.item(PopupMenuItem::new("Show Diff").on_click(move |_, _, cx| {
            e_diff.update(cx, |this, cx| {
                if let Some(source) = this.diff_source(&format!("{FILE_PREFIX}{p_diff}")) {
                    cx.emit(ChangesEvent::OpenDiff(source));
                }
            })
        }))
        .item(PopupMenuItem::new("Jump to Source").on_click(move |_, _, cx| {
            let path = p_source.clone();
            e_source.update(cx, |_, cx| cx.emit(ChangesEvent::OpenFile(path)))
        }))
        .separator()
        .item(PopupMenuItem::new("Get from Revision").disabled(!local).on_click(move |_, _, cx| {
            e_get.update(cx, |this, cx| this.get_from_revision(cx))
        }))
        .separator()
        .item(PopupMenuItem::new("Copy Path").on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(p_copy.clone()))))
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let entity = cx.entity();
        let mut tabs = h_flex().flex_1().min_w_0().h_full().gap_1().overflow_hidden();
        if self.tabs.len() == 1 {
            tabs = tabs.child(div().text_sm().truncate().child(self.tabs[0].title()));
        } else {
            for (ix, tab) in self.tabs.iter().enumerate() {
                let active = ix == self.active;
                tabs = tabs.child(
                    h_flex()
                        .id(SharedString::from(format!("changes-tab-{ix}")))
                        .h_full()
                        .px_1()
                        .gap_1()
                        .min_w_0()
                        .flex_shrink(1.)
                        .text_sm()
                        .cursor_pointer()
                        .when(active, |el| el.border_b_2().border_color(palette.accent).text_color(palette.text))
                        .when(!active, |el| el.text_color(palette.text_secondary))
                        .on_click(cx.listener(move |this, _, _, cx| this.select_tab(ix, cx)))
                        .child(div().truncate().child(tab.short_title()))
                        .child(
                            tool_button(SharedString::from(format!("changes-tab-close-{ix}")), IconName::X, "Close Tab")
                                .on_click(cx.listener(move |this, _, _, cx| this.close_tab(ix, cx))),
                        ),
                );
            }
        }
        let active = self.active;
        h_flex()
            .h(px(30.))
            .px_2()
            .gap_1()
            .flex_shrink_0()
            .border_b_1()
            .border_color(palette.border)
            .child(tabs)
            .child(
                Button::new("changes-options")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::EllipsisVertical))
                    .tooltip("Options")
                    .dropdown_menu(move |menu, _, _| {
                        let (close, close_all) = (entity.clone(), entity.clone());
                        menu.item(PopupMenuItem::new("Close Tab").on_click(move |_, _, cx| close.update(cx, |this, cx| this.close_tab(active, cx))))
                            .item(PopupMenuItem::new("Close All Tabs").on_click(move |_, _, cx| close_all.update(cx, |this, cx| this.close_all(cx))))
                    }),
            )
            .child(tool_button("changes-hide", IconName::Minus, "Hide").on_click(cx.listener(|_, _, _, cx| cx.emit(ChangesEvent::Hide))))
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let entity = cx.entity();
        let tab = self.tabs.get(self.active);
        let local = tab.is_some_and(|t| t.new.is_none());
        let group = self.group_by_directory;
        h_flex()
            .h(px(30.))
            .px_1()
            .gap_0p5()
            .flex_shrink_0()
            .child(tool_button("changes-refresh", IconName::RefreshCw, "Refresh").on_click(cx.listener(|this, _, _, cx| this.reload(cx))))
            .child(
                tool_button("changes-swap", IconName::ArrowLeftRight, "Swap Sides")
                    .disabled(local || tab.is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.swap_sides(cx))),
            )
            .child(
                tool_button("changes-get", IconName::Download, "Get from Revision")
                    .disabled(!local)
                    .on_click(cx.listener(|this, _, _, cx| this.get_from_revision(cx))),
            )
            .child(div().w(px(1.)).h(px(16.)).mx_1().bg(palette.border))
            .child(Button::new("changes-view-options").ghost().xsmall().icon(Icon::new(IconName::Eye)).tooltip("View Options").dropdown_menu(
                move |menu, _, _| {
                    let entity = entity.clone();
                    menu.label("Group By").item(PopupMenuItem::new("Directory").checked(group).on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.group_by_directory = !this.group_by_directory;
                            this.rebuild(None, cx);
                        })
                    }))
                },
            ))
            .child(div().flex_1())
            .child(tool_button("changes-expand", IconName::ChevronsUpDown, "Expand All").on_click(cx.listener(|this, _, _, cx| this.rebuild(Some(true), cx))))
            .child(tool_button("changes-collapse", IconName::ChevronsDownUp, "Collapse All").on_click(cx.listener(|this, _, _, cx| this.rebuild(Some(false), cx))))
    }
}

fn set_expanded(mut item: TreeItem, expanded: bool) -> TreeItem {
    if item.children.is_empty() {
        return item;
    }
    item.children = std::mem::take(&mut item.children).into_iter().map(|c| set_expanded(c, expanded)).collect();
    item.expanded(expanded)
}

impl Render for ChangesView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let entity = cx.entity();
        let kinds = self.kinds.clone();
        let counts = self.counts.clone();
        let local = self.tabs.get(self.active).is_some_and(|t| t.new.is_none());
        let tab = self.tabs.get(self.active);
        let message = match tab {
            Some(Comparison { error: Some(error), .. }) => Some((error.clone(), palette.status_conflict)),
            Some(t) if t.files.is_empty() => Some(("No differences".to_owned(), palette.text_secondary)),
            _ => None,
        };
        let tree_palette = palette.clone();
        v_flex()
            .size_full()
            .bg(palette.panel)
            .child(self.render_header(cx))
            .child(self.render_toolbar(cx))
            .when_some(message, |el, (text, color)| el.child(div().p_3().text_sm().text_color(color).child(text)))
            .child(
                div().flex_1().min_h_0().child(
                    tree(&self.tree, move |ix, entry, _, _, _| {
                        let palette = &tree_palette;
                        let item = entry.item();
                        let id = item.id.to_string();
                        let path = id.strip_prefix(FILE_PREFIX).map(str::to_owned);
                        let kind = path.as_ref().and_then(|p| kinds.get(p)).cloned();
                        let color = kind.as_ref().map_or(palette.text, |(k, _)| common::change_color(*k, palette));
                        let is_root = id == ROOT_ID;
                        let (open_entity, open_path) = (entity.clone(), path.clone());
                        let (menu_entity, menu_path) = (entity.clone(), path.clone());
                        let n = counts.get(&item.id).copied().unwrap_or(0);
                        ListItem::new(ix)
                            .py_0()
                            .px_1()
                            .h(px(ROW_HEIGHT))
                            .on_click(move |event, _, cx| {
                                // Double-click (or a second click) opens the file's diff again.
                                if event.click_count() == 2 {
                                    if let Some(path) = open_path.clone() {
                                        open_entity.update(cx, |this, cx| {
                                            if let Some(source) = this.diff_source(&format!("{FILE_PREFIX}{path}")) {
                                                cx.emit(ChangesEvent::OpenDiff(source));
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
                                            None if is_root => IconName::FolderGit2,
                                            None => IconName::Folder,
                                        })
                                        .small()
                                        .text_color(palette.text_secondary),
                                    )
                                    .child(div().text_color(color).child(item.label.clone()))
                                    .when_some(kind.and_then(|(_, old)| old), |el, old| {
                                        el.child(div().text_xs().text_color(palette.text_secondary).child(format!("from {old}")))
                                    })
                                    .when(path.is_none(), |el| {
                                        el.child(
                                            div().text_xs().text_color(palette.text_secondary).child(format!("{n} {}", if n == 1 { "file" } else { "files" })),
                                        )
                                    })
                                    .context_menu(move |menu, _, _| match &menu_path {
                                        Some(path) => Self::file_menu(menu, &menu_entity, path, local),
                                        None => menu,
                                    }),
                            )
                    })
                    .size_full(),
                ),
            )
    }
}
