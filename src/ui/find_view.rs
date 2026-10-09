//! The Find tool window (Alt+3): one tab per search, as IntelliJ's usage
//! view. Find Usages groups by kind; Find in Files lists "Found Occurrences"
//! by directory › file › line and can replace; Search Everywhere results land
//! here as a plain list.

use std::collections::{BTreeMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable as _, Icon, Selectable as _, Sizable as _, WindowExt as _, h_flex,
    button::{Button, ButtonVariants as _},
    input::{Editor, EditorState},
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Task, Window, div, prelude::FluentBuilder as _, px, relative, uniform_list,
};

use crate::index::nav::{Target, Usage};
use crate::index::service::CodeIndex;
use crate::index::text_search::{SearchResult, TextMatch};
use crate::theme::ActivePalette as _;
use crate::ui::common::{self, row_height, tool_button};
use crate::ui::find_popup::{FindRequest, highlighted, target_of};
use crate::ui::navigate::OpenTarget;

/// A Search Everywhere result shown in the Find window.
#[derive(Clone)]
pub struct FoundItem {
    pub title: String,
    pub detail: String,
    pub icon: IconName,
    pub target: Target,
}

enum Content {
    /// The searched word, and its usages.
    Usages(String, Vec<Usage>),
    Text { request: FindRequest, result: SearchResult },
    Items(Vec<FoundItem>),
}

struct FindTab {
    title: String,
    header: String,
    content: Content,
    searching: bool,
    collapsed: HashSet<String>,
    excluded: HashSet<(String, u32)>,
    selected: Option<usize>,
    _task: Option<Task<()>>,
}

pub enum FindViewEvent {
    FilesChanged(Vec<String>),
}

impl EventEmitter<OpenTarget> for FindView {}
impl EventEmitter<FindViewEvent> for FindView {}

pub struct FindView {
    index: Entity<CodeIndex>,
    tabs: Vec<FindTab>,
    active: usize,
    group_by_directory: bool,
    /// Preview Source: the selected result's file beside the list.
    show_preview: bool,
    /// The previewed file, and the result it shows.
    preview: Option<(String, Entity<EditorState>)>,
    previewed: Option<Target>,
}

const WINDOW_LIMIT: usize = 20_000;

#[derive(Clone)]
enum Row {
    Root(String),
    Group(String, usize),
    Dir(String, usize),
    File { path: String, count: usize, indent: usize },
    Usage(usize, usize),
    Text(usize, usize),
    Item(usize),
}

impl FindView {
    pub fn new(index: Entity<CodeIndex>) -> Self {
        Self { index, tabs: Vec::new(), active: 0, group_by_directory: true, show_preview: true, preview: None, previewed: None }
    }

    fn place(&mut self, tab: FindTab, new_tab: bool) -> usize {
        // A pinned-less IntelliJ usage view reuses the current tab unless asked not to.
        if new_tab || self.tabs.is_empty() {
            self.tabs.push(tab);
            self.active = self.tabs.len() - 1;
        } else {
            self.tabs[self.active] = tab;
        }
        self.active
    }

    fn empty_tab(title: String, content: Content) -> FindTab {
        FindTab {
            title,
            header: "Searching…".into(),
            content,
            searching: true,
            collapsed: HashSet::new(),
            excluded: HashSet::new(),
            selected: None,
            _task: None,
        }
    }

    pub fn find_usages(&mut self, path: String, text: String, offset: usize, cx: &mut Context<Self>) {
        let task = self.index.read(cx).usages(path, text, offset, cx);
        let ix = self.place(Self::empty_tab("Usages".into(), Content::Usages(String::new(), Vec::new())), false);
        let task = cx.spawn(async move |this, cx| {
            let (word, usages) = task.await;
            this.update(cx, |this, cx| {
                let Some(tab) = this.tabs.get_mut(ix) else { return };
                tab.searching = false;
                tab.title = if word.is_empty() { "Usages".into() } else { format!("Usages of {word}") };
                tab.header = if word.is_empty() { "Nothing to search for at the caret".into() } else { format!("Usages of {word} — {}", plural(usages.len(), "result")) };
                tab.content = Content::Usages(word, usages);
                cx.notify();
            })
            .ok();
        });
        self.tabs[ix]._task = Some(task);
        cx.notify();
    }

    pub fn find_text(&mut self, request: FindRequest, cx: &mut Context<Self>) {
        let title = format!("{} '{}'", if request.replacement.is_some() { "Replace" } else { "Find" }, request.query.text);
        let new_tab = request.new_tab;
        let ix = self.place(Self::empty_tab(title, Content::Text { request: request.clone(), result: SearchResult::default() }), new_tab);
        self.run_text(ix, cx);
    }

    fn run_text(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(ix) else { return };
        let Content::Text { request, .. } = &tab.content else { return };
        let request = request.clone();
        tab.searching = true;
        tab.header = "Searching…".into();
        let task = self.index.read(cx).find_text(request.query.clone(), request.scope.clone(), WINDOW_LIMIT, Arc::new(AtomicBool::new(false)), cx);
        let task = cx.spawn(async move |this, cx| {
            let found = task.await;
            this.update(cx, |this, cx| {
                let Some(tab) = this.tabs.get_mut(ix) else { return };
                tab.searching = false;
                let Content::Text { request, result } = &mut tab.content else { return };
                match found {
                    Ok(found) => {
                        let plus = if found.truncated { "+" } else { "" };
                        tab.header = match &request.replacement {
                            Some(r) => format!("Occurrences of '{}' to be replaced with '{}' in {} — {}{plus} {}", request.query.text, r, request.scope_label, found.matches.len(), if found.matches.len() == 1 { "result" } else { "results" }),
                            None => format!("Occurrences of '{}' in {} — {}{plus} {}", request.query.text, request.scope_label, found.matches.len(), if found.matches.len() == 1 { "result" } else { "results" }),
                        };
                        *result = found;
                    }
                    Err(error) => {
                        tab.header = if error.is_empty() { "Nothing to search for".into() } else { error };
                        *result = SearchResult::default();
                    }
                }
                tab.excluded.clear();
                cx.notify();
            })
            .ok();
        });
        self.tabs[ix]._task = Some(task);
        cx.notify();
    }

    pub fn show_items(&mut self, title: String, items: Vec<FoundItem>, cx: &mut Context<Self>) {
        let mut tab = Self::empty_tab(title.clone(), Content::Items(Vec::new()));
        tab.searching = false;
        tab.header = format!("{title} — {}", plural(items.len(), "result"));
        tab.content = Content::Items(items);
        self.place(tab, true);
        cx.notify();
    }

    fn close_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.tabs.len() {
            self.tabs.remove(ix);
            self.active = self.active.min(self.tabs.len().saturating_sub(1));
            cx.notify();
        }
    }

    fn rows(&self) -> Vec<Row> {
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        let mut rows = Vec::new();
        match &tab.content {
            Content::Usages(_, usages) => {
                let mut last_group = None;
                let mut last_file = None;
                for (i, u) in usages.iter().enumerate() {
                    if last_group.as_ref() != Some(&u.group) {
                        rows.push(Row::Group(u.group.clone(), usages.iter().filter(|x| x.group == u.group).count()));
                        last_group = Some(u.group.clone());
                        last_file = None;
                    }
                    if last_file.as_ref() != Some(&u.path) {
                        let count = usages.iter().filter(|x| x.group == u.group && x.path == u.path).count();
                        rows.push(Row::File { path: u.path.clone(), count, indent: 1 });
                        last_file = Some(u.path.clone());
                    }
                    if !tab.collapsed.contains(&u.path) {
                        rows.push(Row::Usage(i, 2));
                    }
                }
            }
            Content::Text { request, result } => {
                let visible: Vec<(usize, &TextMatch)> = result.matches.iter().enumerate().filter(|(_, m)| !tab.excluded.contains(&(m.path.clone(), m.line))).collect();
                if result.matches.is_empty() {
                    return rows;
                }
                rows.push(Row::Root(format!("Found Occurrences in {}", request.scope_label)));
                let mut by_dir: BTreeMap<String, Vec<(usize, &TextMatch)>> = BTreeMap::new();
                for (i, m) in &visible {
                    let dir = if self.group_by_directory { m.path.rsplit_once('/').map(|(d, _)| d.to_owned()).unwrap_or_default() } else { String::new() };
                    by_dir.entry(dir).or_default().push((*i, m));
                }
                for (dir, hits) in by_dir {
                    let file_indent = if self.group_by_directory {
                        rows.push(Row::Dir(dir.clone(), hits.len()));
                        if tab.collapsed.contains(&format!("d:{dir}")) {
                            continue;
                        }
                        2
                    } else {
                        1
                    };
                    let mut k = 0;
                    while k < hits.len() {
                        let path = hits[k].1.path.clone();
                        let count = hits[k..].iter().take_while(|(_, m)| m.path == path).count();
                        rows.push(Row::File { path: path.clone(), count, indent: file_indent });
                        if !tab.collapsed.contains(&path) {
                            for (i, _) in &hits[k..k + count] {
                                rows.push(Row::Text(*i, file_indent + 1));
                            }
                        }
                        k += count;
                    }
                }
            }
            Content::Items(items) => rows.extend((0..items.len()).map(Row::Item)),
        }
        rows
    }

    fn row_target(&self, row: &Row) -> Option<Target> {
        let tab = self.tabs.get(self.active)?;
        match (row, &tab.content) {
            (Row::Usage(i, _), Content::Usages(_, u)) => u.get(*i).map(|u| Target { path: u.path.clone(), line: u.line, col: u.col, name: String::new(), label: u.group.clone(), container: None }),
            (Row::Text(i, _), Content::Text { result, .. }) => result.matches.get(*i).map(target_of),
            (Row::Item(i), Content::Items(items)) => items.get(*i).map(|it| it.target.clone()),
            (Row::File { path, .. }, _) => Some(Target { path: path.clone(), line: 0, col: 0, name: String::new(), label: String::new(), container: None }),
            _ => None,
        }
    }

    fn click(&mut self, ix: usize, open: bool, cx: &mut Context<Self>) {
        let rows = self.rows();
        let Some(row) = rows.get(ix).cloned() else { return };
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        tab.selected = Some(ix);
        match &row {
            Row::File { path, .. } if !open => {
                if !tab.collapsed.remove(path) {
                    tab.collapsed.insert(path.clone());
                }
            }
            Row::Dir(dir, _) => {
                let key = format!("d:{dir}");
                if !tab.collapsed.remove(&key) {
                    tab.collapsed.insert(key);
                }
            }
            _ => {}
        }
        if open || matches!(row, Row::Usage(..) | Row::Text(..) | Row::Item(_)) {
            if let Some(target) = self.row_target(&row) {
                cx.emit(OpenTarget(target));
            }
        }
        cx.notify();
    }

    /// Whether the active tab has results to step through.
    pub fn has_occurrences(&self) -> bool {
        self.rows().iter().any(|r| matches!(r, Row::Usage(..) | Row::Text(..) | Row::Item(_)))
    }

    /// Next / Previous Occurrence (Ctrl+Alt+Down / Up).
    pub fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let rows = self.rows();
        let Some(tab) = self.tabs.get(self.active) else { return };
        let hits: Vec<usize> = rows.iter().enumerate().filter(|(_, r)| matches!(r, Row::Usage(..) | Row::Text(..) | Row::Item(_))).map(|(i, _)| i).collect();
        if hits.is_empty() {
            return;
        }
        let current = tab.selected.and_then(|s| hits.iter().position(|h| *h >= s));
        let next = match (current, delta > 0) {
            (None, _) => 0,
            (Some(p), true) if tab.selected == Some(hits[p]) => (p + 1).min(hits.len() - 1),
            (Some(p), true) => p,
            (Some(p), false) => p.saturating_sub(1),
        };
        self.click(hits[next], true, cx);
    }

    fn set_all_collapsed(&mut self, collapsed: bool, cx: &mut Context<Self>) {
        let keys: Vec<String> = self
            .rows()
            .into_iter()
            .filter_map(|r| match r {
                Row::File { path, .. } => Some(path),
                Row::Dir(dir, _) => Some(format!("d:{dir}")),
                _ => None,
            })
            .collect();
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        if collapsed {
            tab.collapsed.extend(keys);
        } else {
            tab.collapsed.clear();
        }
        cx.notify();
    }

    /// Exclude (Delete): drops the selected line or file from the results.
    fn exclude_selected(&mut self, cx: &mut Context<Self>) {
        let rows = self.rows();
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let Some(row) = tab.selected.and_then(|s| rows.get(s)) else { return };
        let Content::Text { result, .. } = &tab.content else { return };
        match row {
            Row::Text(i, _) => {
                if let Some(m) = result.matches.get(*i) {
                    tab.excluded.insert((m.path.clone(), m.line));
                }
            }
            Row::File { path, .. } => {
                for m in result.matches.iter().filter(|m| &m.path == path) {
                    tab.excluded.insert((m.path.clone(), m.line));
                }
            }
            _ => return,
        }
        cx.notify();
    }

    /// Replace All / Replace Selected in a Replace in Files tab.
    fn replace(&mut self, selected_only: bool, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.rows();
        let Some(tab) = self.tabs.get(self.active) else { return };
        let Content::Text { request, result } = &tab.content else { return };
        let Some(replacement) = request.replacement.clone() else { return };
        let picked: Vec<&TextMatch> = match (selected_only, tab.selected.and_then(|s| rows.get(s))) {
            (false, _) => result.matches.iter().filter(|m| !tab.excluded.contains(&(m.path.clone(), m.line))).collect(),
            (true, Some(Row::Text(i, _))) => result.matches.get(*i).into_iter().collect(),
            (true, Some(Row::File { path, .. })) => result.matches.iter().filter(|m| &m.path == path && !tab.excluded.contains(&(m.path.clone(), m.line))).collect(),
            _ => return,
        };
        let Some(root) = self.index.read(cx).root().map(|r| r.to_path_buf()) else { return };
        let Ok(re) = crate::index::text_search::compile(&request.query) else { return };
        let mut by_file: BTreeMap<String, HashSet<u32>> = BTreeMap::new();
        for m in picked {
            by_file.entry(m.path.clone()).or_default().insert(m.line);
        }
        let mut changed = Vec::new();
        let mut total = 0;
        for (rel, lines) in by_file {
            let path = root.join(&rel);
            let Some(text) = crate::index::text_search::read_text(&path) else { continue };
            let starts: Vec<usize> = crate::index::text_search::occurrences(&rel, &text, &re, request.query.context)
                .into_iter()
                .filter(|r| lines.contains(&(crate::ui::find_popup::line_of(&text, r.start) as u32)))
                .map(|r| r.start)
                .collect();
            let (new_text, count) = crate::index::text_search::replace(&rel, &text, &request.query, &re, &replacement, Some(&starts));
            if count > 0 && std::fs::write(&path, new_text).is_ok() {
                total += count;
                changed.push(rel);
            }
        }
        let files = changed.len();
        if !changed.is_empty() {
            cx.emit(FindViewEvent::FilesChanged(changed));
        }
        window.push_notification(format!("{total} occurrences replaced in {files} files"), cx);
        let ix = self.active;
        self.run_text(ix, cx);
    }

    /// Points the preview at the selected result (after a render, since
    /// it needs the window).
    fn update_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.rows();
        let target = self.tabs.get(self.active).and_then(|t| t.selected).and_then(|s| rows.get(s)).filter(|r| matches!(r, Row::Usage(..) | Row::Text(..) | Row::Item(_))).and_then(|r| self.row_target(r));
        if target == self.previewed {
            return;
        }
        self.previewed = target.clone();
        let Some(target) = target else {
            self.preview = None;
            return;
        };
        let fresh = self.preview.as_ref().is_none_or(|(path, _)| *path != target.path);
        let file = std::path::Path::new(&target.path);
        let file = if file.is_absolute() { file.to_path_buf() } else { self.index.read(cx).root().map(|r| r.join(file)).unwrap_or_default() };
        let text = crate::index::text_search::read_text(&file).unwrap_or_default();
        if fresh {
            let language = crate::ui::file_editor::language_for(&target.path);
            let value = text.clone();
            let state = cx.new(|cx| {
                let mut state = EditorState::new(window, cx).language(language).line_number(true).soft_wrap(false).default_value(value);
                state.set_readonly(true, cx);
                state
            });
            self.preview = Some((target.path.clone(), state));
        }
        let Some((_, state)) = &self.preview else { return };
        // Select the result's line and show it a few lines from the top,
        // without moving focus from where the user is.
        let line = target.line as usize;
        let start = text.split_inclusive('\n').take(line).map(str::len).sum::<usize>();
        let end = start + text[start.min(text.len())..].find('\n').unwrap_or(text.len().saturating_sub(start));
        let height = state.read(cx).line_height().unwrap_or(window.line_height());
        state.update(cx, |state, cx| {
            state.set_selected_range(start..end, cx);
            state.set_scroll_offset(gpui_kit::point(px(0.), -(height * line.saturating_sub(4) as f32)), cx);
        });
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mut tabs = h_flex().gap_0p5().min_w_0().overflow_x_hidden();
        for (ix, tab) in self.tabs.iter().enumerate() {
            let active = ix == self.active;
            tabs = tabs.child(
                h_flex()
                    .id(("find-tab", ix))
                    .h(px(24.))
                    .pl_2()
                    .pr_0p5()
                    .gap_1()
                    .rounded(px(4.))
                    .text_sm()
                    .cursor_pointer()
                    .when(active, |el| el.bg(palette.selection))
                    .when(!active, |el| el.hover(|el| el.bg(palette.hover)))
                    .child(div().whitespace_nowrap().child(tab.title.clone()))
                    .child(
                        Button::new(("find-tab-close", ix))
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(IconName::Close))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.close_tab(ix, cx)
                            })),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.active = ix;
                        cx.notify();
                    })),
            );
        }
        h_flex()
            .h(px(crate::ui::common::header_height()))
            .px_2()
            .gap_2()
            .flex_shrink_0()
            .border_b_1()
            .border_color(palette.border)
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Find:"))
            .child(tabs)
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let tab = self.tabs.get(self.active);
        let text = tab.is_some_and(|t| matches!(t.content, Content::Text { .. }));
        let replace = tab.is_some_and(|t| matches!(&t.content, Content::Text { request, .. } if request.replacement.is_some()));
        h_flex()
            .h(px(crate::ui::common::header_height()))
            .px_1()
            .gap_0p5()
            .flex_shrink_0()
            .child(
                tool_button("find-rerun", IconName::RefreshCw, "Rerun")
                    .disabled(!text)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let ix = this.active;
                        this.run_text(ix, cx)
                    })),
            )
            .child(div().w(px(1.)).h(px(16.)).mx_1().bg(palette.border))
            .child(tool_button("find-prev", IconName::ArrowUp, "Previous Occurrence (Ctrl+Alt+Up)").on_click(cx.listener(|this, _, _, cx| this.step(-1, cx))))
            .child(tool_button("find-next", IconName::ArrowDown, "Next Occurrence (Ctrl+Alt+Down)").on_click(cx.listener(|this, _, _, cx| this.step(1, cx))))
            .child(tool_button("find-expand", IconName::ChevronsUpDown, "Expand All").on_click(cx.listener(|this, _, _, cx| this.set_all_collapsed(false, cx))))
            .child(tool_button("find-collapse", IconName::ChevronsDownUp, "Collapse All").on_click(cx.listener(|this, _, _, cx| this.set_all_collapsed(true, cx))))
            .child(
                tool_button("find-group-dir", IconName::FolderTree, "Group by Directory")
                    .selected(self.group_by_directory)
                    .disabled(!text)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.group_by_directory = !this.group_by_directory;
                        cx.notify();
                    })),
            )
            .child(
                tool_button("find-preview", IconName::Eye, "Preview Source")
                    .selected(self.show_preview)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_preview = !this.show_preview;
                        cx.notify();
                    })),
            )
            .child(tool_button("find-exclude", IconName::Ban, "Exclude (Delete)").disabled(!text).on_click(cx.listener(|this, _, _, cx| this.exclude_selected(cx))))
            .child(div().flex_1())
            .when(replace, |el| {
                el.child(Button::new("find-replace-selected").small().outline().label("Replace Selected").on_click(cx.listener(|this, _, window, cx| this.replace(true, window, cx))))
                    .child(Button::new("find-replace-all").small().primary().label("Replace All").on_click(cx.listener(|this, _, window, cx| this.replace(false, window, cx))))
            })
    }
}

impl Render for FindView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        if self.show_preview {
            self.update_preview(window, cx);
        }
        if self.tabs.is_empty() {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(palette.text_secondary)
                .child("Use Find in Files (Ctrl+Shift+F) or Find Usages (Alt+F7) to search")
                .into_any_element();
        }
        let rows = Rc::new(self.rows());
        let tab = &self.tabs[self.active];
        let selected = tab.selected;
        let collapsed = tab.collapsed.clone();
        let (word, usages): (String, Rc<Vec<Usage>>) = match &tab.content {
            Content::Usages(w, u) => (w.clone(), Rc::new(u.clone())),
            _ => (String::new(), Rc::new(Vec::new())),
        };
        // Same-named files show their folder.
        let mut names: std::collections::HashMap<&str, HashSet<&str>> = std::collections::HashMap::new();
        for row in rows.iter() {
            if let Row::File { path, .. } = row {
                names.entry(path.rsplit('/').next().unwrap_or(path)).or_default().insert(path);
            }
        }
        let ambiguous: Rc<HashSet<String>> = Rc::new(names.into_iter().filter(|(_, p)| p.len() > 1).map(|(n, _)| n.to_owned()).collect());
        let matches: Rc<Vec<TextMatch>> = Rc::new(match &tab.content {
            Content::Text { result, .. } => result.matches.clone(),
            _ => Vec::new(),
        });
        let items: Rc<Vec<FoundItem>> = Rc::new(match &tab.content {
            Content::Items(i) => i.clone(),
            _ => Vec::new(),
        });
        let total = matches.len();
        let list_rows = rows.clone();
        let list = uniform_list(
            "find-view",
            rows.len(),
            cx.processor(move |_, range: Range<usize>, _, cx| {
                let palette = cx.palette().clone();
                let chevron = |open: bool| common::icon(if open { IconName::ChevronDown } else { IconName::ChevronRight }).text_color(palette.text_secondary);
                range
                    .map(|ix| {
                        let base = h_flex()
                            .id(ix)
                            .h(px(row_height()))
                            .gap_1()
                            .text_sm()
                            .cursor_pointer()
                            .when(selected == Some(ix), |el| el.bg(palette.selection))
                            .when(selected != Some(ix), |el| el.hover(|el| el.bg(palette.hover)))
                            .on_click(cx.listener(move |this, event: &gpui_kit::ClickEvent, _, cx| this.click(ix, event.click_count() > 1, cx)));
                        let pad = |n: usize| px(8. + 16. * n as f32);
                        match &list_rows[ix] {
                            Row::Root(label) => base
                                .pl(pad(0))
                                .child(chevron(true))
                                .child(div().font_weight(FontWeight::SEMIBOLD).child(label.clone()))
                                .child(div().text_color(palette.text_secondary).child(plural(total, "result"))),
                            Row::Group(name, count) => base
                                .pl(pad(0))
                                .child(chevron(true))
                                .child(div().font_weight(FontWeight::SEMIBOLD).child(name.clone()))
                                .child(div().text_color(palette.text_secondary).child(plural(*count, "usage"))),
                            Row::Dir(dir, count) => base
                                .pl(pad(1))
                                .child(chevron(!collapsed.contains(&format!("d:{dir}"))))
                                .child(common::icon(IconName::FolderClosed).text_color(palette.text_secondary))
                                .child(if dir.is_empty() { "<root>".to_owned() } else { dir.clone() })
                                .child(div().text_color(palette.text_secondary).child(plural(*count, "result"))),
                            Row::File { path, count, indent } => {
                                let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
                                base.pl(pad(*indent))
                                    .child(chevron(!collapsed.contains(path)))
                                    .child(common::icon(common::file_icon(path)).text_color(palette.text_secondary))
                                    .child(name.to_owned())
                                    .when(ambiguous.contains(name) && !dir.is_empty(), |el| el.child(div().text_color(palette.text_secondary).child(dir.to_owned())))
                                    .child(div().text_color(palette.text_secondary).child(plural(*count, if usages.is_empty() { "result" } else { "usage" })))
                            }
                            Row::Usage(i, indent) => {
                                let u = &usages[*i];
                                base.pl(pad(*indent))
                                    .gap_2()
                                    .child(div().w(px(36.)).text_right().text_color(palette.text_secondary).child((u.line + 1).to_string()))
                                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(highlighted(&u.text, &word_ranges(&u.text, &word), &palette)))
                            }
                            Row::Text(i, indent) => {
                                let m = &matches[*i];
                                base.pl(pad(*indent))
                                    .gap_2()
                                    .child(div().w(px(36.)).text_right().text_color(palette.text_secondary).child((m.line + 1).to_string()))
                                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(highlighted(&m.text, &m.ranges, &palette)))
                            }
                            Row::Item(i) => {
                                let it = &items[*i];
                                base.pl(pad(0))
                                    .gap_2()
                                    .child(common::icon(it.icon).text_color(palette.text_secondary))
                                    .child(it.title.clone())
                                    .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().text_xs().text_color(palette.text_secondary).child(it.detail.clone()))
                            }
                        }
                        .into_any_element()
                    })
                    .collect()
            }),
        )
        .size_full();
        v_flex()
            .size_full()
            .key_context("FindView")
            .on_key_down(cx.listener(|this, event: &gpui_kit::KeyDownEvent, _, cx| {
                let k = &event.keystroke;
                match (k.key.as_str(), k.modifiers.control && k.modifiers.alt) {
                    ("delete", false) => this.exclude_selected(cx),
                    ("down", true) => this.step(1, cx),
                    ("up", true) => this.step(-1, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(self.render_tabs(cx))
            .child(self.render_toolbar(cx))
            .child(
                h_flex()
                    .h(px(24.))
                    .px_2()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(palette.text_secondary)
                    .child(tab.header.clone()),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(div().flex_1().min_w_0().h_full().child(list))
                    .when_some(self.preview.as_ref().filter(|_| self.show_preview), |el, (_, state)| {
                        el.child(div().w(relative(0.45)).h_full().flex_shrink_0().border_l_1().border_color(palette.border).child(Editor::new(state).bordered(false).h_full()))
                    }),
            )
            .into_any_element()
    }
}

/// "1 usage", "3 usages".
fn plural(n: usize, noun: &str) -> String {
    if n == 1 { format!("1 {noun}") } else { format!("{n} {noun}s") }
}

/// Whole-word occurrences of `word` in a result line.
fn word_ranges(text: &str, word: &str) -> Vec<Range<usize>> {
    if word.is_empty() {
        return Vec::new();
    }
    let is_word = crate::index::nav::is_word_char;
    text.match_indices(word)
        .filter(|(i, _)| !text[..*i].chars().next_back().is_some_and(is_word) && !text[i + word.len()..].chars().next().is_some_and(is_word))
        .map(|(i, _)| i..i + word.len())
        .collect()
}
