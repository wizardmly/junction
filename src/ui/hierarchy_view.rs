//! IntelliJ's Hierarchy tool window. Call Hierarchy (Ctrl+Alt+H): the
//! function at the caret, and under it the functions that call it (its
//! callers' callers on expanding), each with how many times it calls, as
//! "Callers of total". Type Hierarchy (Ctrl+H): the type's superclasses
//! down to it and its subtypes, or (the toolbar) all its supertypes, or all
//! its subtypes. A double click opens the call or the type.

use std::collections::HashSet;
use std::path::PathBuf;

use gpui_kit::component::{Icon, Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AppContext as _, ClickEvent, Context, Entity, EventEmitter, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _,
    Task, Window, div, prelude::FluentBuilder as _, px,
};

use crate::index::call_hierarchy::{caller_at, offset_of};
use crate::index::hierarchy::{TypeNode, TypeView};
use crate::index::lang::Lang;
use crate::index::service::CodeIndex;
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::common::tool_button;

pub enum HierarchyEvent {
    /// Open a file (relative to the project) at a 0-based line and column.
    Open { path: String, line: u32, col: u32 },
    Hide,
}

/// A function in the tree: where it's declared and where its calls to the
/// parent are.
#[derive(Clone)]
struct Node {
    name: String,
    path: String,
    /// The name's offset in the file, its line and column.
    offset: usize,
    line: u32,
    col: u32,
    /// Its calls of the parent function (line, column).
    calls: Vec<(u32, u32)>,
    /// None: not looked up yet.
    children: Option<Vec<Node>>,
    expanded: bool,
    loading: bool,
    /// Already an ancestor: recursion, not expanded again.
    recursive: bool,
}

/// A Type Hierarchy: the type, the view, the tree once looked up, and the
/// nodes folded.
struct Types {
    name: String,
    view: TypeView,
    tree: Option<TypeNode>,
    collapsed: HashSet<Vec<usize>>,
}

pub struct HierarchyView {
    index: Entity<CodeIndex>,
    root: Option<Node>,
    types: Option<Types>,
    selected: Vec<usize>,
    _tasks: Vec<Task<()>>,
}

impl EventEmitter<HierarchyEvent> for HierarchyView {}

fn node_at<'a>(node: &'a mut Node, path: &[usize]) -> Option<&'a mut Node> {
    match path.split_first() {
        None => Some(node),
        Some((ix, rest)) => node_at(node.children.as_mut()?.get_mut(*ix)?, rest),
    }
}

impl HierarchyView {
    pub fn new(index: Entity<CodeIndex>) -> Self {
        Self { index, root: None, types: None, selected: Vec::new(), _tasks: Vec::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.root.is_none() && self.types.is_none()
    }

    /// Type Hierarchy of the type named `name`.
    pub fn type_hierarchy(&mut self, name: String, cx: &mut Context<Self>) {
        self.root = None;
        self.types = Some(Types { name, view: TypeView::Type, tree: None, collapsed: HashSet::new() });
        self.load_types(cx);
    }

    fn set_type_view(&mut self, view: TypeView, cx: &mut Context<Self>) {
        let Some(types) = self.types.as_mut() else { return };
        if types.view != view {
            types.view = view;
            self.load_types(cx);
        }
    }

    fn load_types(&mut self, cx: &mut Context<Self>) {
        let Some(types) = self.types.as_mut() else { return };
        types.tree = None;
        types.collapsed.clear();
        let (name, view) = (types.name.clone(), types.view);
        let task = self.index.read(cx).type_hierarchy(name.clone(), view, cx);
        self._tasks.push(cx.spawn(async move |this, cx| {
            let tree = task.await;
            this.update(cx, |this, cx| {
                let Some(types) = this.types.as_mut().filter(|t| t.name == name && t.view == view) else { return };
                // The type itself selected, as IntelliJ opens it.
                fn find(n: &TypeNode, name: &str, at: Vec<usize>) -> Option<Vec<usize>> {
                    if n.name == name {
                        return Some(at);
                    }
                    n.children.iter().enumerate().find_map(|(i, c)| {
                        let mut at = at.clone();
                        at.push(i);
                        find(c, name, at)
                    })
                }
                this.selected = find(&tree, &name, Vec::new()).unwrap_or_default();
                types.tree = Some(tree);
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn type_node(&self, at: &[usize]) -> Option<&TypeNode> {
        let mut cur = self.types.as_ref()?.tree.as_ref()?;
        for ix in at {
            cur = cur.children.get(*ix)?;
        }
        Some(cur)
    }

    fn type_rows(&self) -> Vec<(Vec<usize>, usize, &TypeNode)> {
        fn walk<'a>(n: &'a TypeNode, at: Vec<usize>, depth: usize, collapsed: &HashSet<Vec<usize>>, out: &mut Vec<(Vec<usize>, usize, &'a TypeNode)>) {
            let open = !collapsed.contains(&at);
            out.push((at.clone(), depth, n));
            if open {
                for (i, c) in n.children.iter().enumerate() {
                    let mut path = at.clone();
                    path.push(i);
                    walk(c, path, depth + 1, collapsed, out);
                }
            }
        }
        let mut out = Vec::new();
        if let Some(types) = &self.types {
            if let Some(tree) = &types.tree {
                walk(tree, Vec::new(), 0, &types.collapsed, &mut out);
            }
        }
        out
    }

    /// Call Hierarchy of the function at `offset` of this file.
    pub fn call_hierarchy(&mut self, path: String, text: String, offset: usize, cx: &mut Context<Self>) -> Result<(), String> {
        self.types = None;
        let lang = Lang::from_path(&path).ok_or("Call Hierarchy works in Kotlin and Java files")?;
        let file_name = path.rsplit('/').next().unwrap_or(&path).to_owned();
        let caller = caller_at(lang, &text, offset, &file_name).ok_or("No method at the caret")?;
        self.root = Some(Node {
            name: caller.name,
            path,
            offset: caller.offset,
            line: caller.line,
            col: caller.col,
            calls: Vec::new(),
            children: None,
            expanded: true,
            loading: false,
            recursive: false,
        });
        self.selected = Vec::new();
        self.expand(Vec::new(), cx);
        Ok(())
    }

    /// Looks up a node's callers.
    fn expand(&mut self, at: Vec<usize>, cx: &mut Context<Self>) {
        let Some(root_dir) = self.index.read(cx).root().map(|r| r.to_path_buf()) else { return };
        let Some(root) = self.root.as_mut() else { return };
        // The names on the way down, to stop at recursion.
        let mut ancestors = vec![root.name.clone()];
        let mut cur: &Node = root;
        for ix in &at {
            let Some(next) = cur.children.as_ref().and_then(|c| c.get(*ix)) else { return };
            ancestors.push(next.name.clone());
            cur = next;
        }
        let Some(node) = node_at(root, &at) else { return };
        node.expanded = true;
        if node.children.is_some() || node.loading || node.recursive {
            cx.notify();
            return;
        }
        node.loading = true;
        let file = root_dir.join(&node.path);
        let Ok(text) = std::fs::read_to_string(&file) else {
            node.loading = false;
            node.children = Some(Vec::new());
            return;
        };
        let task = self.index.read(cx).usages(node.path.clone(), text, node.offset, cx);
        self._tasks.push(cx.spawn(async move |this, cx| {
            let (_, usages) = task.await;
            // Group the calls by the function they're in.
            let callers = cx
                .background_spawn(async move {
                    let mut out: Vec<Node> = Vec::new();
                    for u in usages.into_iter().filter(|u| u.group != "Declarations") {
                        let Some(lang) = Lang::from_path(&u.path) else { continue };
                        let path: PathBuf = if std::path::Path::new(&u.path).is_absolute() { PathBuf::from(&u.path) } else { root_dir.join(&u.path) };
                        let Ok(text) = std::fs::read_to_string(&path) else { continue };
                        let offset = offset_of(&text, u.line, u.col);
                        let file_name = u.path.rsplit('/').next().unwrap_or(&u.path).to_owned();
                        let Some(caller) = caller_at(lang, &text, offset, &file_name) else { continue };
                        match out.iter_mut().find(|n| n.path == u.path && n.offset == caller.offset) {
                            Some(n) => n.calls.push((u.line, u.col)),
                            None => out.push(Node {
                                recursive: ancestors.contains(&caller.name),
                                name: caller.name,
                                path: u.path.clone(),
                                offset: caller.offset,
                                line: caller.line,
                                col: caller.col,
                                calls: vec![(u.line, u.col)],
                                children: None,
                                expanded: false,
                                loading: false,
                            }),
                        }
                    }
                    out.sort_by(|a, b| a.name.cmp(&b.name));
                    out
                })
                .await;
            this.update(cx, |this, cx| {
                if let Some(node) = this.root.as_mut().and_then(|r| node_at(r, &at)) {
                    node.loading = false;
                    node.children = Some(callers);
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn toggle(&mut self, at: Vec<usize>, cx: &mut Context<Self>) {
        let expanded = self.root.as_mut().and_then(|r| node_at(r, &at)).map(|n| n.expanded).unwrap_or(false);
        if expanded {
            if let Some(n) = self.root.as_mut().and_then(|r| node_at(r, &at)) {
                n.expanded = false;
            }
            cx.notify();
        } else {
            self.expand(at, cx);
        }
    }

    /// Opens a node's first call (the root: its declaration).
    fn open(&mut self, at: Vec<usize>, cx: &mut Context<Self>) {
        let Some(n) = self.root.as_mut().and_then(|r| node_at(r, &at)) else { return };
        let (line, col) = n.calls.first().copied().unwrap_or((n.line, n.col));
        cx.emit(HierarchyEvent::Open { path: n.path.clone(), line, col });
    }

    fn rows(&self, node: &Node, at: Vec<usize>, depth: usize, out: &mut Vec<(Vec<usize>, usize, Node)>) {
        out.push((at.clone(), depth, Node { children: None, ..node.clone() }));
        if node.expanded {
            for (i, c) in node.children.iter().flatten().enumerate() {
                let mut path = at.clone();
                path.push(i);
                self.rows(c, path, depth + 1, out);
            }
        }
    }
}

impl HierarchyView {
    fn type_list(&self, palette: &crate::theme::Palette, cx: &mut Context<Self>) -> gpui_kit::Stateful<gpui_kit::Div> {
        let mut list = v_flex().id("type-hierarchy-rows").flex_1().min_h_0().overflow_y_scroll().py_1().text_sm();
        let collapsed = self.types.as_ref().map(|t| t.collapsed.clone()).unwrap_or_default();
        if self.types.as_ref().is_some_and(|t| t.tree.is_none()) {
            return list.child(div().px_3().py_1().text_color(palette.text_secondary).child("Building hierarchy…"));
        }
        for (at, depth, n) in self.type_rows() {
            let selected = self.selected == at;
            let id = at.iter().fold(depth, |h, i| h * 31 + i + 1);
            let chevron = if n.children.is_empty() {
                div().w(px(16.)).into_any_element()
            } else {
                let at = at.clone();
                div()
                    .id(("type-hierarchy-toggle", id))
                    .w(px(16.))
                    .child(Icon::new(if collapsed.contains(&at) { icons::CHEVRON_RIGHT } else { icons::CHEVRON_DOWN }).xsmall())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(types) = this.types.as_mut() {
                            if !types.collapsed.remove(&at) {
                                types.collapsed.insert(at.clone());
                            }
                        }
                        cx.notify();
                    }))
                    .into_any_element()
            };
            let icon: Icon = match &n.symbol {
                Some(sym) => crate::ui::project_view::type_icon(sym, n.kotlin).into(),
                None => icons::CLASS.into(),
            };
            // IntelliJ's "(package)" after the name: here the file.
            let place = n.target.as_ref().map(|t| t.path.rsplit(['/', '\\']).next().unwrap_or(&t.path).to_owned());
            let at_click = at.clone();
            list = list.child(
                h_flex()
                    .id(("type-hierarchy-row", id))
                    .h(px(22.))
                    .pl(px(8. + depth as f32 * 16.))
                    .pr_2()
                    .gap_1()
                    .whitespace_nowrap()
                    .when(selected, |el| el.bg(palette.selection))
                    .when(!selected, |el| el.hover(|s| s.bg(palette.hover)))
                    .child(chevron)
                    .child(icon.xsmall())
                    .child(div().when(n.target.is_none(), |el| el.text_color(palette.text_secondary)).child(n.name.clone()))
                    .when_some(place, |el, p| el.child(div().text_color(palette.text_secondary).child(format!("({p})"))))
                    .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                        this.selected = at_click.clone();
                        if e.click_count() == 2 {
                            if let Some(t) = this.type_node(&at_click).and_then(|n| n.target.clone()) {
                                cx.emit(HierarchyEvent::Open { path: t.path, line: t.line, col: t.col });
                            }
                        }
                        cx.notify();
                    })),
            );
        }
        list
    }
}

impl Render for HierarchyView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let title = match (&self.root, &self.types) {
            (Some(r), _) => format!("Callers of {}", r.name),
            (None, Some(t)) => match t.view {
                TypeView::Type => format!("Type Hierarchy of {}", t.name),
                TypeView::Supertypes => format!("Supertypes of {}", t.name),
                TypeView::Subtypes => format!("Subtypes of {}", t.name),
            },
            _ => "Call Hierarchy".into(),
        };
        let type_view = self.types.as_ref().map(|t| t.view);
        let mut rows = Vec::new();
        if let Some(root) = &self.root {
            self.rows(root, Vec::new(), 0, &mut rows);
        }
        let expandable: Vec<bool> = rows
            .iter()
            .map(|(at, _, n)| {
                let full = self.root.as_ref().and_then(|r| {
                    let mut cur = r;
                    for ix in at {
                        cur = cur.children.as_ref()?.get(*ix)?;
                    }
                    Some(cur)
                });
                !n.recursive && full.is_none_or(|f| f.children.as_ref().is_none_or(|c| !c.is_empty()))
            })
            .collect();
        let mut list = v_flex().id("hierarchy-rows").flex_1().min_h_0().overflow_y_scroll().py_1().text_sm();
        for ((at, depth, n), expandable) in rows.into_iter().zip(expandable) {
            let selected = self.selected == at;
            let file = n.path.rsplit('/').next().unwrap_or(&n.path).to_owned();
            let (at_toggle, at_click) = (at.clone(), at.clone());
            let chevron = if !expandable {
                div().w(px(16.)).into_any_element()
            } else {
                div()
                    .id(("hierarchy-toggle", at.iter().fold(depth, |h, i| h * 31 + i)))
                    .w(px(16.))
                    .child(Icon::new(if n.expanded { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT }).xsmall())
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle(at_toggle.clone(), cx)))
                    .into_any_element()
            };
            list = list.child(
                h_flex()
                    .id(("hierarchy-row", at.iter().fold(depth * 7919, |h, i| h * 31 + i + 1)))
                    .h(px(22.))
                    .pl(px(8. + depth as f32 * 16.))
                    .pr_2()
                    .gap_1()
                    .whitespace_nowrap()
                    .when(selected, |el| el.bg(palette.selection))
                    .when(!selected, |el| el.hover(|s| s.bg(palette.hover)))
                    .child(chevron)
                    .child(Icon::new(icons::METHOD).xsmall())
                    .child(div().child(format!("{}()", n.name)))
                    .when(n.calls.len() > 1, |el| el.child(div().text_color(palette.text_secondary).child(format!("({} usages)", n.calls.len()))))
                    .child(div().text_color(palette.text_secondary).child(format!("({file})")))
                    .when(n.recursive, |el| el.child(div().text_color(palette.text_secondary).child("recursive call")))
                    .when(n.loading, |el| el.child(div().text_color(palette.text_secondary).child("…")))
                    .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                        this.selected = at_click.clone();
                        if e.click_count() == 2 {
                            this.open(at_click.clone(), cx);
                        }
                        cx.notify();
                    })),
            );
        }
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
                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Hierarchy"))
                    .child(div().text_sm().text_color(palette.text_secondary).child(title))
                    .child(div().flex_1())
                    .when_some(type_view, |el, view| {
                        let button = |id: &'static str, icon, tip: &'static str, v: TypeView| {
                            tool_button(id, icon, tip).selected(view == v).on_click(cx.listener(move |this, _, _, cx| this.set_type_view(v, cx)))
                        };
                        el.child(button("hierarchy-type", icons::CLASS_HIERARCHY, "Type Hierarchy", TypeView::Type))
                            .child(button("hierarchy-supertypes", icons::SUPERTYPES, "Supertypes Hierarchy", TypeView::Supertypes))
                            .child(button("hierarchy-subtypes", icons::SUBTYPES, "Subtypes Hierarchy", TypeView::Subtypes))
                    })
                    .child(tool_button("hierarchy-hide", icons::HIDE, "Hide").on_click(cx.listener(|_, _, _, cx| cx.emit(HierarchyEvent::Hide)))),
            )
            .child(if type_view.is_some() { self.type_list(&palette, cx) } else { list })
    }
}
