//! Code navigation UI: the Find tool window (Find Usages), Go to File /
//! Class / Symbol popups, the "Choose Declaration" list, and the Project
//! tool window's file tree.

use std::collections::{BTreeMap, HashSet};
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    WindowExt as _, h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement as _,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div, prelude::FluentBuilder as _, px,
    uniform_list,
};

use crate::index::nav::{Target, Usage};
use crate::index::service::{CodeIndex, IndexEvent};
use crate::theme::ActivePalette as _;
use crate::ui::common::{self, ROW_HEIGHT, tool_button};

/// Something to open: a path and position.
#[derive(Clone)]
pub struct OpenTarget(pub Target);

/// The Find tool window: usages grouped by kind and language.
pub struct UsagesView {
    title: String,
    usages: Vec<Usage>,
    searching: bool,
    selected: Option<usize>,
    _task: Option<Task<()>>,
}

impl EventEmitter<OpenTarget> for UsagesView {}

impl UsagesView {
    pub fn new() -> Self {
        Self { title: String::new(), usages: Vec::new(), searching: false, selected: None, _task: None }
    }

    pub fn search(&mut self, index: &Entity<CodeIndex>, path: String, text: String, offset: usize, cx: &mut Context<Self>) {
        let task = index.read(cx).usages(path, text, offset, cx);
        self.searching = true;
        self.title = "Searching…".into();
        self.usages.clear();
        self.selected = None;
        cx.notify();
        self._task = Some(cx.spawn(async move |this, cx| {
            let (word, usages) = task.await;
            this.update(cx, |this, cx| {
                this.searching = false;
                this.title = if word.is_empty() { "Nothing to search for at the caret".into() } else { format!("Usages of {word} — {} results", usages.len()) };
                this.usages = usages;
                cx.notify();
            })
            .ok();
        }));
    }

    fn open(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(u) = self.usages.get(ix) else { return };
        self.selected = Some(ix);
        cx.emit(OpenTarget(Target { path: u.path.clone(), line: u.line, col: u.col, name: String::new(), label: u.group.clone(), container: None }));
        cx.notify();
    }
}

enum UsageRow {
    Group(String, usize),
    File(String, usize),
    Hit(usize),
}

impl Render for UsagesView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        // Group › file › line, as IntelliJ's usage tree.
        let mut rows: Vec<UsageRow> = Vec::new();
        let mut last_group = None;
        let mut last_file = None;
        for (i, u) in self.usages.iter().enumerate() {
            if last_group.as_ref() != Some(&u.group) {
                let count = self.usages.iter().filter(|x| x.group == u.group).count();
                rows.push(UsageRow::Group(u.group.clone(), count));
                last_group = Some(u.group.clone());
                last_file = None;
            }
            if last_file.as_ref() != Some(&u.path) {
                let count = self.usages.iter().filter(|x| x.group == u.group && x.path == u.path).count();
                rows.push(UsageRow::File(u.path.clone(), count));
                last_file = Some(u.path.clone());
            }
            rows.push(UsageRow::Hit(i));
        }
        let rows = Rc::new(rows);
        let usages = Rc::new(self.usages.clone());
        let selected = self.selected;
        let list = uniform_list(
            "usages",
            rows.len(),
            cx.processor(move |_, range: Range<usize>, _, cx| {
                let palette = cx.palette().clone();
                range
                    .map(|ix| match &rows[ix] {
                        UsageRow::Group(name, count) => h_flex()
                            .id(ix)
                            .h(px(ROW_HEIGHT))
                            .px_2()
                            .gap_1()
                            .text_sm()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child(common::icon(IconName::ChevronDown).text_color(palette.text_secondary))
                            .child(name.clone())
                            .child(div().text_color(palette.text_secondary).child(format!("{count} usages")))
                            .into_any_element(),
                        UsageRow::File(path, count) => h_flex()
                            .id(ix)
                            .h(px(ROW_HEIGHT))
                            .pl(px(24.))
                            .gap_1()
                            .text_sm()
                            .child(common::icon(common::file_icon(path)).text_color(palette.text_secondary))
                            .child(path.rsplit('/').next().unwrap_or(path).to_owned())
                            .child(div().text_xs().text_color(palette.text_secondary).child(format!("{path} · {count}")))
                            .into_any_element(),
                        UsageRow::Hit(i) => {
                            let u = &usages[*i];
                            let i = *i;
                            h_flex()
                                .id(ix)
                                .h(px(ROW_HEIGHT))
                                .pl(px(48.))
                                .gap_2()
                                .text_sm()
                                .cursor_pointer()
                                .when(selected == Some(i), |el| el.bg(palette.selection))
                                .hover(|el| el.bg(palette.hover))
                                .child(div().w(px(40.)).text_right().text_color(palette.text_secondary).child((u.line + 1).to_string()))
                                .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().child(u.text.clone()))
                                .on_click(cx.listener(move |this, _, _, cx| this.open(i, cx)))
                                .into_any_element()
                        }
                    })
                    .collect()
            }),
        )
        .size_full();
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(px(28.))
                    .px_2()
                    .border_b_1()
                    .border_color(palette.border)
                    .text_sm()
                    .text_color(palette.text_secondary)
                    .child(self.title.clone()),
            )
            .child(div().flex_1().min_h_0().child(list))
    }
}

/// What a Go to popup searches.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GotoKind {
    File,
    Class,
    Symbol,
}

impl GotoKind {
    fn title(self) -> &'static str {
        match self {
            GotoKind::File => "Go to File",
            GotoKind::Class => "Go to Class",
            GotoKind::Symbol => "Go to Symbol",
        }
    }
}

struct GotoItem {
    title: String,
    detail: String,
    icon: IconName,
    target: Target,
}

pub struct GotoView {
    kind: GotoKind,
    index: Entity<CodeIndex>,
    input: Entity<InputState>,
    items: Vec<GotoItem>,
    selected: usize,
    on_pick: Rc<dyn Fn(Target, &mut Window, &mut App)>,
    _task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl GotoView {
    fn new(kind: GotoKind, index: Entity<CodeIndex>, on_pick: Rc<dyn Fn(Target, &mut Window, &mut App)>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let placeholder = match kind {
            GotoKind::File => "File name",
            GotoKind::Class => "Class name",
            GotoKind::Symbol => "Symbol name",
        };
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let subscriptions = vec![cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::Change => this.search(cx),
            InputEvent::PressEnter { .. } => this.pick(this.selected, window, cx),
            _ => {}
        })];
        Self { kind, index, input, items: Vec::new(), selected: 0, on_pick, _task: None, _subscriptions: subscriptions }
    }

    fn search(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).value().to_string();
        let kind = self.kind;
        let index = self.index.read(cx);
        self._task = Some(match kind {
            GotoKind::File => {
                let task = index.search_files(query, cx);
                cx.spawn(async move |this, cx| {
                    let found = task.await;
                    this.update(cx, |this, cx| {
                        this.items = found
                            .into_iter()
                            .map(|(path, _)| {
                                let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
                                let dir = path.rsplit_once('/').map(|(d, _)| d.to_owned()).unwrap_or_default();
                                GotoItem {
                                    title: name.clone(),
                                    detail: dir,
                                    icon: common::file_icon(&path),
                                    target: Target { path, line: 0, col: 0, name, label: String::new(), container: None },
                                }
                            })
                            .collect();
                        this.selected = 0;
                        cx.notify();
                    })
                    .ok();
                })
            }
            GotoKind::Class | GotoKind::Symbol => {
                let task = index.search_symbols(query, kind == GotoKind::Class, cx);
                cx.spawn(async move |this, cx| {
                    let found = task.await;
                    this.update(cx, |this, cx| {
                        this.items = found
                            .into_iter()
                            .map(|m| {
                                let detail = match &m.target.container {
                                    Some(c) => format!("{c} · {} · {}:{}", m.target.label, m.target.path, m.target.line + 1),
                                    None => format!("{} · {}:{}", m.target.label, m.target.path, m.target.line + 1),
                                };
                                GotoItem { title: m.target.name.clone(), detail, icon: symbol_icon(m.kind), target: m.target }
                            })
                            .collect();
                        this.selected = 0;
                        cx.notify();
                    })
                    .ok();
                })
            }
        });
    }

    fn pick(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.items.get(ix) else { return };
        let target = item.target.clone();
        window.close_dialog(cx);
        (self.on_pick)(target, window, cx);
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.items.is_empty() {
            return;
        }
        self.selected = (self.selected as isize + delta).rem_euclid(self.items.len() as isize) as usize;
        cx.notify();
    }
}

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

impl Render for GotoView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mut list = v_flex();
        for (ix, item) in self.items.iter().enumerate() {
            list = list.child(
                h_flex()
                    .id(("goto", ix))
                    .h(px(26.))
                    .px_2()
                    .gap_2()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .when(ix == self.selected, |el| el.bg(palette.selection))
                    .hover(|el| el.bg(palette.hover))
                    .child(common::icon(item.icon).text_color(palette.text_secondary))
                    .child(div().text_sm().whitespace_nowrap().child(item.title.clone()))
                    .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().text_xs().text_color(palette.text_secondary).child(item.detail.clone()))
                    .on_click(cx.listener(move |this, _, window, cx| this.pick(ix, window, cx))),
            );
        }
        let empty = self.items.is_empty() && !self.input.read(cx).value().trim().is_empty();
        v_flex()
            .gap_2()
            .on_key_down(cx.listener(|this, event: &gpui_kit::KeyDownEvent, _, cx| match event.keystroke.key.as_str() {
                "down" => this.move_selection(1, cx),
                "up" => this.move_selection(-1, cx),
                _ => {}
            }))
            .child(Input::new(&self.input))
            .child(
                div()
                    .id("goto-list")
                    .h(px(360.))
                    .overflow_y_scrollbar()
                    .when(empty, |el| el.child(div().p_2().text_sm().text_color(palette.text_secondary).child("Nothing found")))
                    .child(list),
            )
    }
}

/// Go to File (Ctrl+Shift+N), Go to Class (Ctrl+N), Go to Symbol (Ctrl+Alt+Shift+N).
pub fn goto(kind: GotoKind, index: Entity<CodeIndex>, on_pick: Rc<dyn Fn(Target, &mut Window, &mut App)>, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| GotoView::new(kind, index, on_pick, window, cx));
    let focus = view.read(cx).input.clone();
    window.open_dialog(cx, move |dialog, _, _| dialog.title(kind.title()).w(px(640.)).child(view.clone()));
    crate::ui::dialogs::focus_input(&focus, window, cx);
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
        let mut this = Self { index, expanded: HashSet::new(), selected: None, rows: Rc::default(), files: Rc::default(), _subscription: subscription };
        this.reload(cx);
        this
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

