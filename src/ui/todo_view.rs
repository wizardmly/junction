//! The TODO tool window, as IntelliJ's: the Project tab lists every TODO
//! and FIXME comment of the project by file, Current File those of the
//! open editor. Kept up to date while it shows.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use gpui_kit::component::{Selectable as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription,
    Task, Window, div, prelude::FluentBuilder as _, px,
};

use crate::domain::todo;
use crate::index::service::{CodeIndex, IndexEvent};
use crate::index::text_search::{FileScope, SearchContext, TextMatch, TextQuery};
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::common::tool_button;
use crate::ui::item_tree::{self, ItemTree, ItemTreeEvent, TreeFile, TreeItem};

/// IntelliJ stops a TODO search at this many items too.
const LIMIT: usize = 10_000;

pub enum TodoEvent {
    OpenFile { path: PathBuf, line: u32, col: u32 },
    Hide,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Project,
    File,
}

pub struct TodoView {
    index: Entity<CodeIndex>,
    project: Rc<dyn Fn(&App) -> Option<PathBuf>>,
    tab: Tab,
    tree: Entity<ItemTree>,
    /// The open editor's file, relative to the project.
    current: Option<String>,
    all: Vec<TextMatch>,
    shown: bool,
    stale: bool,
    _search: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TodoEvent> for TodoView {}

impl TodoView {
    pub fn new(index: Entity<CodeIndex>, project: Rc<dyn Fn(&App) -> Option<PathBuf>>, cx: &mut Context<Self>) -> Self {
        let tree = cx.new(|cx| ItemTree::new(("TODO item", "TODO items"), cx));
        let subscriptions = vec![
            cx.subscribe(&tree, |_, _, event: &ItemTreeEvent, cx| match event {
                ItemTreeEvent::Open { path, line, col } => cx.emit(TodoEvent::OpenFile { path: path.clone(), line: *line, col: *col }),
            }),
            cx.subscribe(&index, |this, _, event: &IndexEvent, cx| {
                if matches!(event, IndexEvent::Changed) {
                    this.stale = true;
                    if this.shown {
                        this.search(cx);
                    }
                }
            }),
        ];
        Self { index, project, tab: Tab::Project, tree, current: None, all: Vec::new(), shown: false, stale: true, _search: None, _subscriptions: subscriptions }
    }

    /// Searches while shown; the open editor's file for Current File.
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        self.shown = shown;
        if shown && self.stale {
            self.search(cx);
        }
    }

    pub fn set_current_file(&mut self, file: Option<String>, cx: &mut Context<Self>) {
        if self.current != file {
            self.current = file;
            self.fill(cx);
        }
    }

    fn search(&mut self, cx: &mut Context<Self>) {
        self.stale = false;
        let index = self.index.clone();
        self._search = Some(cx.spawn(async move |this, cx| {
            // Saves come in bursts.
            cx.background_executor().timer(Duration::from_millis(300)).await;
            let query = TextQuery { text: todo::DEFAULT_PATTERN.into(), case_sensitive: false, whole_words: false, regex: true, mask: None, context: SearchContext::InComments };
            let task = cx.update(|cx| index.read(cx).find_text(query, FileScope::Project, LIMIT, Arc::new(AtomicBool::new(false)), cx));
            let result = task.await;
            this.update(cx, |this, cx| {
                this.all = result.map(|r| r.matches).unwrap_or_default();
                this.fill(cx);
            })
            .ok();
        }));
    }

    fn fill(&mut self, cx: &mut Context<Self>) {
        let Some(root) = (self.project)(cx) else { return };
        let mut files: Vec<TreeFile> = Vec::new();
        for m in &self.all {
            if self.tab == Tab::File && self.current.as_deref() != Some(m.path.as_str()) {
                continue;
            }
            let path = root.join(&m.path);
            if files.last().is_none_or(|f| f.path != path) {
                let (name, dir) = item_tree::name_and_dir(&path, Some(&root));
                files.push(TreeFile { path: path.clone(), name, dir, items: Vec::new() });
            }
            let highlight = m.ranges.first().map(|k| todo::item_range(&m.text, k));
            files.last_mut().unwrap().items.push(TreeItem {
                icon: None,
                prefix: Some(format!("({}, {})", m.line + 1, m.col + 1).into()),
                text: m.text.clone().into(),
                highlight,
                line: m.line,
                col: m.col,
            });
        }
        self.tree.update(cx, |t, cx| t.set_files(files, cx));
        cx.notify();
    }
}

impl Render for TodoView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let tab = |id: &'static str, label: &'static str, which: Tab, active: bool| {
            h_flex()
                .id(id)
                .h(px(24.))
                .px_2()
                .rounded(px(4.))
                .text_sm()
                .cursor_pointer()
                .when(active, |el| el.bg(palette.selection))
                .when(!active, |el| el.hover(|el| el.bg(palette.hover)))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.tab = which;
                    this.fill(cx);
                }))
        };
        let header = h_flex()
            .h(px(crate::ui::common::header_height()))
            .px_2()
            .gap_2()
            .flex_shrink_0()
            .border_b_1()
            .border_color(palette.border)
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("TODO:"))
            .child(tab("todo-tab-project", "Project", Tab::Project, self.tab == Tab::Project))
            .child(tab("todo-tab-file", "Current File", Tab::File, self.tab == Tab::File))
            .child(div().flex_1())
            .child(tool_button("todo-hide", icons::HIDE, "Hide").on_click(cx.listener(|_, _, _, cx| cx.emit(TodoEvent::Hide))));
        let tree = self.tree.clone();
        let autoscroll = tree.read(cx).autoscroll;
        let (t1, t2, t3, t4, t5) = (tree.clone(), tree.clone(), tree.clone(), tree.clone(), tree.clone());
        let toolbar = v_flex()
            .w(px(30.))
            .flex_none()
            .items_center()
            .py_1()
            .gap_0p5()
            .border_r_1()
            .border_color(palette.border)
            .child(tool_button("todo-prev", icons::UP, "Previous TODO").on_click(move |_, _, cx| t1.update(cx, |t, cx| t.step(-1, cx))))
            .child(tool_button("todo-next", icons::DOWN, "Next TODO").on_click(move |_, _, cx| t2.update(cx, |t, cx| t.step(1, cx))))
            .child(tool_button("todo-expand", icons::EXPAND_ALL, "Expand All").on_click(move |_, _, cx| t3.update(cx, |t, cx| t.set_all_collapsed(false, cx))))
            .child(tool_button("todo-collapse", icons::COLLAPSE_ALL, "Collapse All").on_click(move |_, _, cx| t4.update(cx, |t, cx| t.set_all_collapsed(true, cx))))
            .child(tool_button("todo-autoscroll", icons::AUTOSCROLL_FROM_SOURCE, "Autoscroll to Source").selected(autoscroll).on_click(move |_, _, cx| {
                t5.update(cx, |t, cx| {
                    t.autoscroll = !t.autoscroll;
                    cx.notify();
                })
            }));
        let (items, files) = {
            let t = self.tree.read(cx);
            (t.item_count(), t.file_count())
        };
        let summary = match (self.tab, items) {
            (Tab::File, 0) if self.current.is_none() => "No file is open in the editor".to_owned(),
            (_, 0) => "No TODO items found".to_owned(),
            (_, n) => format!("Found {n} TODO item{} in {files} file{}", if n == 1 { "" } else { "s" }, if files == 1 { "" } else { "s" }),
        };
        v_flex().size_full().bg(palette.panel).child(header).child(
            h_flex().flex_1().min_h_0().items_stretch().child(toolbar).child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(div().px_2().py_1().text_sm().text_color(palette.text_secondary).child(summary))
                    .child(div().flex_1().min_h_0().child(tree)),
            ),
        )
    }
}
