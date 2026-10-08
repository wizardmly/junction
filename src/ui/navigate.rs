//! Code navigation UI: the "Choose Declaration" list, Recent Files and
//! File Structure pickers, and the Project tool window's file tree.

use std::collections::{BTreeMap, HashSet};
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    WindowExt as _, h_flex,
    menu::ContextMenuExt as _,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement as _,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px,
    uniform_list,
};

use crate::index::nav::Target;
use crate::index::service::{CodeIndex, IndexEvent};
use crate::theme::ActivePalette as _;
use crate::ui::common::{self, ROW_HEIGHT, tool_button};

/// Something to open: a path and position.
#[derive(Clone)]
pub struct OpenTarget(pub Target);

pub fn symbol_icon(kind: crate::index::symbols::SymbolKind) -> IconName {
    use crate::index::symbols::SymbolKind as K;
    match kind {
        K::Class | K::Struct | K::Interface | K::Protocol | K::Trait | K::Extension => IconName::Layers,
        K::Enum | K::EnumMember => IconName::Rows3,
        K::Function | K::Method | K::Constructor => IconName::CircleDot,
        K::Field | K::Property | K::Variable | K::Constant => IconName::Circle,
        K::Module | K::Namespace => IconName::FolderClosed,
        K::TypeAlias => IconName::Hash,
        K::Macro => IconName::Hash,
    }
}

/// "Choose Declaration": several targets for one name.
pub fn choose_target(targets: Vec<Target>, on_pick: Rc<dyn Fn(Target, &mut Window, &mut App)>, window: &mut Window, cx: &mut App) {
    let targets = Rc::new(targets);
    window.open_dialog(cx, move |dialog, _, cx| {
        let palette = cx.palette().clone();
        let mut list = v_flex();
        for (ix, t) in targets.iter().enumerate() {
            let (t2, on_pick) = (t.clone(), on_pick.clone());
            let name = if t.name.is_empty() { t.path.rsplit('/').next().unwrap_or(&t.path).to_owned() } else { t.name.clone() };
            list = list.child(
                h_flex()
                    .id(("choose", ix))
                    .h(px(28.))
                    .px_2()
                    .gap_2()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|el| el.bg(palette.hover))
                    .child(common::icon(common::file_icon(&t.path)).text_color(palette.text_secondary))
                    .child(div().text_sm().child(match &t.container {
                        Some(c) => format!("{c}.{name}"),
                        None => name,
                    }))
                    .child(div().text_xs().text_color(palette.text_secondary).child(t.label.clone()))
                    .child(div().flex_1())
                    .child(div().text_xs().text_color(palette.text_secondary).child(format!("{}:{}", t.path, t.line + 1)))
                    .on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        on_pick(t2.clone(), window, cx);
                    }),
            );
        }
        dialog.title("Choose Declaration").w(px(640.)).child(div().id("choose-list").max_h(px(420.)).overflow_y_scrollbar().child(list))
    });
}

/// The Project tool window: the repository's files as a tree.
pub struct ProjectView {
    index: Entity<CodeIndex>,
    expanded: HashSet<String>,
    selected: Option<String>,
    /// Flattened visible rows: (depth, name, path, is_dir).
    rows: Rc<Vec<(usize, String, String, bool)>>,
    files: Rc<Vec<String>>,
    /// The right-click menu's repository and workspace actions, once set.
    menu: Option<(Entity<crate::model::RepoModel>, crate::ui::file_menus::FileActions)>,
    clipboard: crate::ui::file_menus::FileClipboard,
    _subscription: Subscription,
}

impl EventEmitter<OpenTarget> for ProjectView {}

#[derive(Default)]
struct Dir {
    dirs: BTreeMap<String, Dir>,
    files: Vec<String>,
}

impl ProjectView {
    pub fn new(index: Entity<CodeIndex>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&index, |this, _, _: &IndexEvent, cx| this.reload(cx));
        let mut this = Self { index, expanded: HashSet::new(), selected: None, rows: Rc::default(), files: Rc::default(), menu: None, clipboard: Rc::default(), _subscription: subscription };
        this.reload(cx);
        this
    }

    pub fn set_menu(&mut self, model: Entity<crate::model::RepoModel>, actions: crate::ui::file_menus::FileActions) {
        self.menu = Some((model, actions));
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let files = self.index.read(cx).index.read().map(|i| i.all_files.clone()).unwrap_or_default();
        if *self.files == files {
            return;
        }
        self.files = Rc::new(files);
        self.flatten();
        cx.notify();
    }

    fn flatten(&mut self) {
        let mut root = Dir::default();
        for path in self.files.iter() {
            let mut dir = &mut root;
            let mut parts: Vec<&str> = path.split('/').collect();
            let file = parts.pop().unwrap_or_default();
            for part in parts {
                dir = dir.dirs.entry(part.to_owned()).or_default();
            }
            dir.files.push(file.to_owned());
        }
        let mut rows = Vec::new();
        fn walk(dir: &Dir, prefix: &str, depth: usize, expanded: &HashSet<String>, rows: &mut Vec<(usize, String, String, bool)>) {
            for (name, sub) in &dir.dirs {
                // Single-child directory chains collapse into one row ("src/main/java").
                let (mut label, mut path, mut node) = (name.clone(), format!("{prefix}{name}"), sub);
                while node.files.is_empty() && node.dirs.len() == 1 {
                    let (n, s) = node.dirs.iter().next().unwrap();
                    label = format!("{label}/{n}");
                    path = format!("{path}/{n}");
                    node = s;
                }
                rows.push((depth, label, path.clone(), true));
                if expanded.contains(&path) {
                    walk(node, &format!("{path}/"), depth + 1, expanded, rows);
                }
            }
            for f in &dir.files {
                rows.push((depth, f.clone(), format!("{prefix}{f}"), false));
            }
        }
        walk(&root, "", 0, &self.expanded, &mut rows);
        self.rows = Rc::new(rows);
    }

    fn click(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some((_, _, path, is_dir)) = self.rows.get(ix).cloned() else { return };
        self.selected = Some(path.clone());
        if is_dir {
            if !self.expanded.remove(&path) {
                self.expanded.insert(path);
            }
            self.flatten();
        } else {
            let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
            cx.emit(OpenTarget(Target { path, line: 0, col: 0, name, label: String::new(), container: None }));
        }
        cx.notify();
    }

    /// Select In › Project View: reveal a file.
    pub fn reveal(&mut self, path: &str, cx: &mut Context<Self>) {
        let mut prefix = String::new();
        for part in path.split('/').collect::<Vec<_>>().split_last().map(|(_, dirs)| dirs.to_vec()).unwrap_or_default() {
            prefix = if prefix.is_empty() { part.to_owned() } else { format!("{prefix}/{part}") };
            self.expanded.insert(prefix.clone());
        }
        self.selected = Some(path.to_owned());
        self.flatten();
        cx.notify();
    }

    fn collapse_all(&mut self, cx: &mut Context<Self>) {
        self.expanded.clear();
        self.flatten();
        cx.notify();
    }
}

impl Render for ProjectView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let rows = self.rows.clone();
        let selected = self.selected.clone();
        let expanded = self.expanded.clone();
        let root_name: SharedString = self
            .index
            .read(cx)
            .root()
            .and_then(|r| r.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
            .into();
        let menu = self.menu.clone();
        let clipboard = self.clipboard.clone();
        let all_files = self.files.clone();
        let root = self.index.read(cx).root().map(|r| r.to_path_buf());
        let list = uniform_list(
            "project-files",
            rows.len(),
            cx.processor(move |_, range: Range<usize>, _, cx| {
                let palette = cx.palette().clone();
                range
                    .map(|ix| {
                        let (depth, name, path, is_dir) = &rows[ix];
                        let open = expanded.contains(path);
                        h_flex()
                            .id(ix)
                            .h(px(ROW_HEIGHT))
                            .pl(px(8. + *depth as f32 * 16.))
                            .gap_1()
                            .text_sm()
                            .cursor_pointer()
                            .when(selected.as_deref() == Some(path.as_str()), |el| el.bg(palette.selection))
                            .hover(|el| el.bg(palette.hover))
                            .child(div().w(px(14.)).when(*is_dir, |el| {
                                el.child(common::icon(if open { IconName::ChevronDown } else { IconName::ChevronRight }).text_color(palette.text_secondary))
                            }))
                            .child(
                                common::icon(if *is_dir {
                                    if open { IconName::FolderOpen } else { IconName::FolderClosed }
                                } else {
                                    common::file_icon(path)
                                })
                                .text_color(palette.text_secondary),
                            )
                            .child(div().whitespace_nowrap().child(name.clone()))
                            .on_click(cx.listener(move |this, _, _, cx| this.click(ix, cx)))
                            .on_mouse_down(gpui_kit::MouseButton::Right, {
                                let path = path.clone();
                                cx.listener(move |this, _, _, cx| {
                                    this.selected = Some(path.clone());
                                    cx.notify();
                                })
                            })
                            .context_menu({
                                let (menu, clipboard, all_files, root) = (menu.clone(), clipboard.clone(), all_files.clone(), root.clone());
                                let (path, is_dir) = (path.clone(), *is_dir);
                                move |m, window, cx| {
                                    let (Some((model, actions)), Some(root)) = (menu.clone(), root.clone()) else { return m };
                                    let prefix = format!("{path}/");
                                    let files = if is_dir { all_files.iter().filter(|f| f.starts_with(&prefix)).cloned().collect() } else { Vec::new() };
                                    let target = crate::ui::file_menus::ProjectTarget {
                                        model,
                                        root,
                                        path: path.clone(),
                                        is_dir,
                                        files,
                                        actions,
                                        clipboard: clipboard.clone(),
                                    };
                                    crate::ui::file_menus::project_menu(m, target, window, cx)
                                }
                            })
                            .into_any_element()
                    })
                    .collect()
            }),
        )
        .size_full();
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
                    .child(div().text_sm().font_weight(gpui_kit::FontWeight::SEMIBOLD).child("Project"))
                    .child(div().text_xs().text_color(palette.text_secondary).child(root_name))
                    .child(div().flex_1())
                    .child(tool_button("project-collapse", IconName::ChevronsDownUp, "Collapse All").on_click(cx.listener(|this, _, _, cx| this.collapse_all(cx)))),
            )
            .child(div().flex_1().min_h_0().child(list))
    }
}


/// A filterable list popup: Recent Files (Ctrl+E), File Structure (Ctrl+F12).
pub struct ListPicker {
    items: Vec<crate::ui::find_view::FoundItem>,
    shown: Vec<usize>,
    input: Entity<InputState>,
    selected: usize,
    on_pick: Rc<dyn Fn(Target, &mut Window, &mut App)>,
    /// Indent per item (File Structure nesting).
    indents: Vec<usize>,
    _subscriptions: Vec<Subscription>,
}

impl ListPicker {
    fn filter(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).value().trim().to_owned();
        let mut scored: Vec<(usize, i32)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, it)| crate::index::nav::fuzzy_score(&query, &it.title).map(|s| (i, s)))
            .collect();
        if !query.is_empty() {
            scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        }
        self.shown = scored.into_iter().map(|(i, _)| i).collect();
        self.selected = 0;
        cx.notify();
    }

    fn pick(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.shown.get(ix).and_then(|i| self.items.get(*i)) else { return };
        let target = item.target.clone();
        window.close_dialog(cx);
        (self.on_pick)(target, window, cx);
    }
}

impl Render for ListPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mut list = v_flex();
        for (row, &i) in self.shown.iter().enumerate() {
            let item = &self.items[i];
            let indent = if self.input.read(cx).value().is_empty() { self.indents.get(i).copied().unwrap_or(0) } else { 0 };
            list = list.child(
                h_flex()
                    .id(("pick", row))
                    .h(px(26.))
                    .pl(px(8. + 16. * indent as f32))
                    .pr_2()
                    .gap_2()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .when(row == self.selected, |el| el.bg(palette.selection))
                    .hover(|el| el.bg(palette.hover))
                    .child(common::icon(item.icon).text_color(palette.text_secondary))
                    .child(div().text_sm().whitespace_nowrap().child(item.title.clone()))
                    .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().text_xs().text_color(palette.text_secondary).child(item.detail.clone()))
                    .on_click(cx.listener(move |this, _, window, cx| this.pick(row, window, cx))),
            );
        }
        v_flex()
            .gap_2()
            .on_key_down(cx.listener(|this, event: &gpui_kit::KeyDownEvent, _, cx| {
                let n = this.shown.len().max(1);
                match event.keystroke.key.as_str() {
                    "down" => this.selected = (this.selected + 1) % n,
                    "up" => this.selected = (this.selected + n - 1) % n,
                    _ => return,
                }
                cx.notify();
            }))
            .child(Input::new(&self.input))
            .child(
                div()
                    .id("pick-list")
                    .max_h(px(420.))
                    .overflow_y_scrollbar()
                    .when(self.shown.is_empty(), |el| el.child(div().p_2().text_sm().text_color(palette.text_secondary).child("Nothing found")))
                    .child(list),
            )
    }
}

/// Opens a ListPicker dialog.
pub fn pick_from_list(
    title: &'static str,
    items: Vec<crate::ui::find_view::FoundItem>,
    indents: Vec<usize>,
    on_pick: Rc<dyn Fn(Target, &mut Window, &mut App)>,
    window: &mut Window,
    cx: &mut App,
) {
    let view = cx.new(|cx| {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Type to filter"));
        let subscriptions = vec![cx.subscribe_in(&input, window, |this: &mut ListPicker, _, event: &InputEvent, window, cx| match event {
            InputEvent::Change => this.filter(cx),
            InputEvent::PressEnter { .. } => this.pick(this.selected, window, cx),
            _ => {}
        })];
        let shown = (0..items.len()).collect();
        ListPicker { items, shown, input, selected: 0, on_pick, indents, _subscriptions: subscriptions }
    });
    let focus = view.read(cx).input.clone();
    window.open_dialog(cx, move |dialog, _, _| dialog.title(title).w(px(560.)).child(view.clone()));
    crate::ui::dialogs::focus_input(&focus, window, cx);
}
