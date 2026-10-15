//! A file-grouped tree of locations, as IntelliJ's Problems and TODO tool
//! windows show them: a file node (icon, name, directory, count) with its
//! items below (icon, text, line). Click selects, double-click or Enter
//! opens; Autoscroll to Source opens on selection.

use std::collections::HashSet;
use std::ops::Range;
use std::path::PathBuf;

use gpui_kit::component::{Icon, Sizable as _, h_flex};
use gpui_kit::{
    AnyElement, App, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, UniformListScrollHandle, Window, div, prelude::FluentBuilder as _, px, uniform_list,
};

use crate::theme::ActivePalette as _;
use crate::ui::as_icons::{self as icons, AsIcon};

const ROW: f32 = 22.;

#[derive(Clone)]
pub struct TreeItem {
    pub icon: Option<AsIcon>,
    /// Grey text before `text`, in place of the ":line" after it (TODO's "(12, 8)").
    pub prefix: Option<SharedString>,
    pub text: SharedString,
    /// Bytes of `text` to highlight (a TODO pattern).
    pub highlight: Option<Range<usize>>,
    /// 0-based.
    pub line: u32,
    pub col: u32,
}

#[derive(Clone)]
pub struct TreeFile {
    pub path: PathBuf,
    /// The file's name and the directory shown after it.
    pub name: SharedString,
    pub dir: SharedString,
    pub items: Vec<TreeItem>,
}

pub enum ItemTreeEvent {
    Open { path: PathBuf, line: u32, col: u32 },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Row {
    File(usize),
    Item(usize, usize),
}

pub struct ItemTree {
    files: Vec<TreeFile>,
    collapsed: HashSet<PathBuf>,
    rows: Vec<Row>,
    selected: Option<Row>,
    /// "3 problems", "2 TODO items".
    noun: (&'static str, &'static str),
    pub autoscroll: bool,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
}

impl EventEmitter<ItemTreeEvent> for ItemTree {}

impl Focusable for ItemTree {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ItemTree {
    pub fn new(noun: (&'static str, &'static str), cx: &mut Context<Self>) -> Self {
        Self { files: Vec::new(), collapsed: HashSet::new(), rows: Vec::new(), selected: None, noun, autoscroll: false, scroll: UniformListScrollHandle::new(), focus: cx.focus_handle() }
    }

    pub fn set_files(&mut self, files: Vec<TreeFile>, cx: &mut Context<Self>) {
        // Keep the selection on the same item when it is still there.
        let keep = self.selected.and_then(|row| self.location(row));
        self.files = files;
        self.rebuild();
        self.selected = keep.and_then(|(path, line, col)| {
            self.rows.iter().copied().find(|row| match *row {
                Row::Item(f, i) => self.files[f].path == path && self.files[f].items[i].line == line && self.files[f].items[i].col == col,
                Row::File(f) => line == u32::MAX && self.files[f].path == path,
            })
        });
        cx.notify();
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    pub fn item_count(&self) -> usize {
        self.files.iter().map(|f| f.items.len()).sum()
    }

    pub fn set_all_collapsed(&mut self, collapsed: bool, cx: &mut Context<Self>) {
        self.collapsed = if collapsed { self.files.iter().map(|f| f.path.clone()).collect() } else { HashSet::new() };
        self.rebuild();
        cx.notify();
    }

    fn rebuild(&mut self) {
        self.rows.clear();
        for (f, file) in self.files.iter().enumerate() {
            self.rows.push(Row::File(f));
            if !self.collapsed.contains(&file.path) {
                self.rows.extend((0..file.items.len()).map(|i| Row::Item(f, i)));
            }
        }
    }

    /// An item's place; a file's is line `u32::MAX`.
    fn location(&self, row: Row) -> Option<(PathBuf, u32, u32)> {
        match row {
            Row::File(f) => Some((self.files.get(f)?.path.clone(), u32::MAX, 0)),
            Row::Item(f, i) => {
                let file = self.files.get(f)?;
                let item = file.items.get(i)?;
                Some((file.path.clone(), item.line, item.col))
            }
        }
    }

    fn open(&mut self, row: Row, cx: &mut Context<Self>) {
        match row {
            Row::Item(..) => {
                if let Some((path, line, col)) = self.location(row) {
                    cx.emit(ItemTreeEvent::Open { path, line, col });
                }
            }
            Row::File(f) => self.toggle(f, cx),
        }
    }

    fn toggle(&mut self, f: usize, cx: &mut Context<Self>) {
        let path = self.files[f].path.clone();
        if !self.collapsed.remove(&path) {
            self.collapsed.insert(path);
        }
        self.rebuild();
        cx.notify();
    }

    fn select(&mut self, row: Row, cx: &mut Context<Self>) {
        self.selected = Some(row);
        if let Some(ix) = self.rows.iter().position(|r| *r == row) {
            self.scroll.scroll_to_item(ix, gpui_kit::ScrollStrategy::Nearest);
        }
        if self.autoscroll && matches!(row, Row::Item(..)) {
            self.open(row, cx);
        }
        cx.notify();
    }

    /// Previous / Next Problem: the item before or after the selection.
    pub fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let items: Vec<Row> = self.files.iter().enumerate().flat_map(|(f, file)| (0..file.items.len()).map(move |i| Row::Item(f, i))).collect();
        if items.is_empty() {
            return;
        }
        let at = self.selected.and_then(|s| items.iter().position(|r| *r == s));
        let next = match at {
            Some(at) => (at as isize + delta).rem_euclid(items.len() as isize) as usize,
            None if delta < 0 => items.len() - 1,
            None => 0,
        };
        let row = items[next];
        if let Row::Item(f, _) = row {
            if self.collapsed.remove(&self.files[f].path) {
                self.rebuild();
            }
        }
        self.selected = Some(row);
        if let Some(ix) = self.rows.iter().position(|r| *r == row) {
            self.scroll.scroll_to_item(ix, gpui_kit::ScrollStrategy::Nearest);
        }
        self.open(row, cx);
        cx.notify();
    }

    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let at = self.selected.and_then(|s| self.rows.iter().position(|r| *r == s));
        match event.keystroke.key.as_str() {
            "down" => {
                let ix = at.map_or(0, |i| (i + 1).min(self.rows.len().saturating_sub(1)));
                if let Some(row) = self.rows.get(ix).copied() {
                    self.select(row, cx);
                }
            }
            "up" => {
                let ix = at.map_or(0, |i| i.saturating_sub(1));
                if let Some(row) = self.rows.get(ix).copied() {
                    self.select(row, cx);
                }
            }
            "enter" | "f4" => {
                if let Some(row) = self.selected {
                    self.open(row, cx);
                }
            }
            "left" | "right" => {
                if let Some(Row::File(f)) = self.selected {
                    let collapsed = self.collapsed.contains(&self.files[f].path);
                    if collapsed == (event.keystroke.key == "right") {
                        self.toggle(f, cx);
                    }
                } else if let (Some(Row::Item(f, _)), "left") = (self.selected, event.keystroke.key.as_str()) {
                    self.select(Row::File(f), cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    fn render_rows(&mut self, range: Range<usize>, _: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let palette = cx.palette().clone();
        let focused = true;
        range
            .filter_map(|ix| self.rows.get(ix).copied().map(|r| (ix, r)))
            .map(|(ix, row)| {
                let selected = self.selected == Some(row);
                let base = h_flex()
                    .id(("item-tree-row", ix))
                    .h(px(ROW))
                    .w_full()
                    .gap_1()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .when(selected, |el| el.bg(if focused { palette.selection } else { palette.selection_inactive }))
                    .when(!selected, |el| el.hover(|el| el.bg(palette.hover)))
                    .on_click(cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                        window.focus(&this.focus, cx);
                        this.selected = Some(row);
                        if event.click_count() >= 2 || (this.autoscroll && matches!(row, Row::Item(..))) {
                            this.open(row, cx);
                        }
                        cx.notify();
                    }));
                match row {
                    Row::File(f) => {
                        let file = &self.files[f];
                        let collapsed = self.collapsed.contains(&file.path);
                        let n = file.items.len();
                        base.pl_1()
                            .child(
                                div()
                                    .id(("item-tree-chevron", ix))
                                    .child(Icon::new(if collapsed { icons::CHEVRON_RIGHT } else { icons::CHEVRON_DOWN }).xsmall())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.toggle(f, cx);
                                    })),
                            )
                            .child(crate::ui::common::file_icon(&file.name).xsmall())
                            .child(file.name.clone())
                            .child(div().text_color(palette.text_secondary).child(file.dir.clone()))
                            .child(div().text_color(palette.text_secondary).child(format!("{n} {}", if n == 1 { self.noun.0 } else { self.noun.1 })))
                            .into_any_element()
                    }
                    Row::Item(f, i) => {
                        let item = &self.files[f].items[i];
                        let text = match &item.highlight {
                            Some(r) if r.end <= item.text.len() && item.text.is_char_boundary(r.start) && item.text.is_char_boundary(r.end) => h_flex()
                                .child(item.text[..r.start].to_owned())
                                .child(div().text_color(palette.link).child(item.text[r.clone()].to_owned()))
                                .child(item.text[r.end..].to_owned())
                                .into_any_element(),
                            _ => div().child(item.text.clone()).into_any_element(),
                        };
                        base.pl(px(36.))
                            .when_some(item.icon, |el, icon| el.child(Icon::new(icon).xsmall()))
                            .when_some(item.prefix.clone(), |el, prefix| el.child(div().text_color(palette.text_secondary).child(prefix)))
                            .child(text)
                            .when(item.prefix.is_none(), |el| el.child(div().text_color(palette.text_secondary).child(format!(":{}", item.line + 1))))
                            .into_any_element()
                    }
                }
            })
            .collect()
    }
}

impl Render for ItemTree {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .text_sm()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .child(uniform_list("item-tree", self.rows.len(), cx.processor(Self::render_rows)).track_scroll(&self.scroll).size_full())
    }
}

/// A file's name and the directory IntelliJ shows after it: relative to
/// the project when inside it.
pub fn name_and_dir(path: &std::path::Path, root: Option<&std::path::Path>) -> (SharedString, SharedString) {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let parent = path.parent().unwrap_or(path);
    let dir = match root.and_then(|r| parent.strip_prefix(r).ok()) {
        Some(rel) => rel.to_string_lossy().replace('\\', "/"),
        None => parent.to_string_lossy().into_owned(),
    };
    (name.into(), dir.into())
}
