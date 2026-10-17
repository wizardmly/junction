//! IntelliJ's Hierarchy tool window. Call Hierarchy (Ctrl+Alt+H): the
//! function at the caret, and under it the functions that call it (its
//! callers' callers on expanding), each with how many times it calls, as
//! "Callers of total". Type Hierarchy (Ctrl+H): the type's superclasses
//! down to it and its subtypes, or (the toolbar) all its supertypes, or all
//! its subtypes. Method Hierarchy (Ctrl+Shift+H): the types from the
//! topmost one declaring the method down, each marked + (defines it),
//! − (inherits it) or ! (should define it). A double click opens the call,
//! the type or the method.

use std::collections::HashSet;

use gpui_kit::component::{Icon, Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    ClickEvent, Context, Entity, EventEmitter, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _,
    Task, Window, div, prelude::FluentBuilder as _, px,
};

use crate::index::call_hierarchy::{CallNode, caller_at};
use crate::index::hierarchy::{MethodMark, TypeNode, TypeView};
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
    /// A Method Hierarchy's method; the views don't apply then.
    method: Option<String>,
    tree: Option<TypeNode>,
    collapsed: HashSet<Vec<usize>>,
}

pub struct HierarchyView {
    index: Entity<CodeIndex>,
    root: Option<Node>,
    /// A Call Hierarchy's direction: what the function calls instead of
    /// what calls it (IntelliJ's Callee Hierarchy).
    callees: bool,
    types: Option<Types>,
    selected: Vec<usize>,
    _tasks: Vec<Task<()>>,
}

impl EventEmitter<HierarchyEvent> for HierarchyView {}

impl From<CallNode> for Node {
    fn from(n: CallNode) -> Self {
        Self { name: n.name, path: n.path, offset: n.offset, line: n.line, col: n.col, calls: n.calls, children: None, expanded: false, loading: false, recursive: n.recursive }
    }
}

fn node_at<'a>(node: &'a mut Node, path: &[usize]) -> Option<&'a mut Node> {
    match path.split_first() {
        None => Some(node),
        Some((ix, rest)) => node_at(node.children.as_mut()?.get_mut(*ix)?, rest),
    }
}

impl HierarchyView {
    pub fn new(index: Entity<CodeIndex>) -> Self {
        Self { index, root: None, callees: false, types: None, selected: Vec::new(), _tasks: Vec::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.root.is_none() && self.types.is_none()
    }

    /// Type Hierarchy of the type named `name`.
    pub fn type_hierarchy(&mut self, name: String, cx: &mut Context<Self>) {
        self.root = None;
        self.types = Some(Types { name, view: TypeView::Type, method: None, tree: None, collapsed: HashSet::new() });
        self.load_types(cx);
    }

    /// Method Hierarchy of the function at `offset` of this file.
    pub fn method_hierarchy(&mut self, path: &str, text: &str, offset: usize, cx: &mut Context<Self>) -> Result<(), String> {
        let lang = Lang::from_path(path).ok_or("Method Hierarchy works in Kotlin and Java files")?;
        let file_name = path.rsplit('/').next().unwrap_or(path);
        let caller = caller_at(lang, text, offset, file_name).ok_or("No method at the caret")?;
        let (owner, method) = caller.name.rsplit_once('.').ok_or("No method of a class at the caret")?;
        self.root = None;
        self.types = Some(Types { name: owner.to_owned(), view: TypeView::Type, method: Some(method.to_owned()), tree: None, collapsed: HashSet::new() });
        self.load_types(cx);
        Ok(())
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
        let task = match types.method.clone() {
            Some(method) => self.index.read(cx).method_hierarchy(name.clone(), method, cx),
            None => self.index.read(cx).type_hierarchy(name.clone(), view, cx),
        };
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
        self.callees = false;
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
        let (path, offset) = (node.path.clone(), node.offset);
        let task = if self.callees {
            CodeIndex::callees_of(&self.index, path, offset, ancestors, cx)
        } else {
            CodeIndex::callers_of(&self.index, path, offset, ancestors, cx)
        };
        self._tasks.push(cx.spawn(async move |this, cx| {
            let found = task.await;
            this.update(cx, |this, cx| {
                if let Some(node) = this.root.as_mut().and_then(|r| node_at(r, &at)) {
                    node.loading = false;
                    node.children = Some(found.into_iter().map(Node::from).collect());
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// The toolbar's Caller / Callee Hierarchy: the same function, the
    /// other direction.
    fn set_callees(&mut self, callees: bool, cx: &mut Context<Self>) {
        if self.callees == callees {
            return;
        }
        self.callees = callees;
        if let Some(root) = self.root.as_mut() {
            root.children = None;
            root.loading = false;
            self.selected = Vec::new();
            self.expand(Vec::new(), cx);
        }
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
        // A caller: its call; a callee (its calls are in the parent): itself.
        let (line, col) = if self.callees { (n.line, n.col) } else { n.calls.first().copied().unwrap_or((n.line, n.col)) };
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
                    .when_some(n.method.as_ref(), |el, (mark, _)| {
                        let (badge, tip) = match mark {
                            MethodMark::Defined => (icons::METHOD_DEFINED, "defines the method"),
                            MethodMark::Inherited => (icons::METHOD_NOT_DEFINED, "inherits the method"),
                            MethodMark::ShouldDefine => (icons::SHOULD_DEFINE_METHOD, "should define the method"),
                        };
                        el.child(div().id(("method-mark", id)).w(px(10.)).flex().justify_center().child(Icon::from(badge).size(px(8.))).tooltip(move |w, cx| gpui_kit::component::tooltip::Tooltip::new(tip).build(w, cx)))
                    })
                    .child(icon.xsmall())
                    .child(div().when(n.target.is_none(), |el| el.text_color(palette.text_secondary)).child(n.name.clone()))
                    .when_some(place, |el, p| el.child(div().text_color(palette.text_secondary).child(format!("({p})"))))
                    .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                        this.selected = at_click.clone();
                        if e.click_count() == 2 {
                            let node = this.type_node(&at_click);
                            let method = node.and_then(|n| n.method.as_ref()).and_then(|(_, t)| t.clone());
                            if let Some(t) = method.or_else(|| node.and_then(|n| n.target.clone())) {
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
            (Some(r), _) if self.callees => format!("Callees of {}", r.name),
            (Some(r), _) => format!("Callers of {}", r.name),
            (None, Some(Types { method: Some(m), name, .. })) => format!("Method Hierarchy of {name}.{m}()"),
            (None, Some(t)) => match t.view {
                TypeView::Type => format!("Type Hierarchy of {}", t.name),
                TypeView::Supertypes => format!("Supertypes of {}", t.name),
                TypeView::Subtypes => format!("Subtypes of {}", t.name),
            },
            _ => "Call Hierarchy".into(),
        };
        let type_view = self.types.as_ref().map(|t| t.view);
        let views = self.types.as_ref().is_some_and(|t| t.method.is_none());
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
                    .when(self.root.is_some(), |el| {
                        let callees = self.callees;
                        el.child(tool_button("hierarchy-callers", icons::SUPERTYPES, "Caller Hierarchy").selected(!callees).on_click(cx.listener(|this, _, _, cx| this.set_callees(false, cx))))
                            .child(tool_button("hierarchy-callees", icons::SUBTYPES, "Callee Hierarchy").selected(callees).on_click(cx.listener(|this, _, _, cx| this.set_callees(true, cx))))
                    })
                    .when_some(type_view.filter(|_| views), |el, view| {
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
