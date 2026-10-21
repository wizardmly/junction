//! Build › Analyze APK…, as Android Studio's APK Analyzer in an editor tab:
//! the APK and download sizes, the files as a tree with Raw File Size,
//! Download Size and % of Total Download Size, and under it the selected
//! file: the decoded AndroidManifest.xml (or another binary XML), a dex
//! file's packages and classes with their method counts, or text.

use std::collections::HashSet;
use std::ops::Range;
use std::path::PathBuf;

use gpui_kit::component::{
    Icon, Sizable as _, h_flex,
    input::{Editor, EditorState},
    v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Task, UniformListScrollHandle, Window,
    div, prelude::FluentBuilder as _, px, uniform_list,
};

use crate::domain::apk::{self, ApkNode};
use crate::domain::dex::{self, DexNode, DexSummary};
use crate::infra::apk_file::ApkFile;
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::usecase::analyze_apk::{self, Detail};

const ROW: f32 = 24.;
const SIZE_COL: f32 = 110.;
const PERCENT_COL: f32 = 170.;

#[derive(Clone)]
struct FileRow {
    depth: usize,
    name: String,
    path: String,
    raw: u64,
    download: u64,
    dir: bool,
}

#[derive(Clone)]
struct DexRow {
    depth: usize,
    key: String,
    name: String,
    class: bool,
    defined_here: bool,
    defined: u32,
    referenced: u32,
    parent: bool,
}

enum Shown {
    Loading,
    Text(Entity<EditorState>),
    Dex(DexSummary),
    Message(String),
}

pub struct ApkAnalyzer {
    path: PathBuf,
    file_size: u64,
    root: Option<ApkNode>,
    expanded: HashSet<String>,
    rows: Vec<FileRow>,
    selected: Option<String>,
    scroll: UniformListScrollHandle,
    shown: Option<Shown>,
    dex_expanded: HashSet<String>,
    dex_rows: Vec<DexRow>,
    dex_selected: Option<String>,
    dex_scroll: UniformListScrollHandle,
    _task: Option<Task<()>>,
}

fn flatten(node: &ApkNode, depth: usize, expanded: &HashSet<String>, out: &mut Vec<FileRow>) {
    for c in &node.children {
        out.push(FileRow { depth, name: c.name.clone(), path: c.path.clone(), raw: c.raw, download: c.download, dir: c.is_dir() });
        if c.is_dir() && expanded.contains(&c.path) {
            flatten(c, depth + 1, expanded, out);
        }
    }
}

fn flatten_dex(nodes: &[DexNode], depth: usize, prefix: &str, expanded: &HashSet<String>, out: &mut Vec<DexRow>) {
    for n in nodes {
        let key = if prefix.is_empty() { n.name.clone() } else { format!("{prefix}.{}", n.name) };
        out.push(DexRow {
            depth,
            key: key.clone(),
            name: n.name.clone(),
            class: n.class,
            defined_here: n.defined_here,
            defined: n.defined_methods,
            referenced: n.referenced_methods,
            parent: !n.children.is_empty(),
        });
        if expanded.contains(&key) {
            flatten_dex(&n.children, depth + 1, &key, expanded, out);
        }
    }
}

impl ApkAnalyzer {
    pub fn new(path: PathBuf, cx: &mut Context<Self>) -> Self {
        let file_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let archive = ApkFile { path: path.clone() };
        let task = cx.spawn(async move |this, cx| {
            let root = cx.background_spawn(async move { analyze_apk::sizes(&archive) }).await;
            this.update(cx, |this, cx| {
                this.root = Some(root);
                this.refresh_rows();
                cx.notify();
            })
            .ok();
        });
        Self {
            path,
            file_size,
            root: None,
            expanded: HashSet::new(),
            rows: Vec::new(),
            selected: None,
            scroll: UniformListScrollHandle::new(),
            shown: None,
            dex_expanded: HashSet::new(),
            dex_rows: Vec::new(),
            dex_selected: None,
            dex_scroll: UniformListScrollHandle::new(),
            _task: Some(task),
        }
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn title(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    fn refresh_rows(&mut self) {
        self.rows.clear();
        if let Some(root) = &self.root {
            flatten(root, 0, &self.expanded, &mut self.rows);
        }
    }

    fn refresh_dex_rows(&mut self) {
        self.dex_rows.clear();
        if let Some(Shown::Dex(summary)) = &self.shown {
            flatten_dex(&summary.tree, 0, "", &self.dex_expanded, &mut self.dex_rows);
        }
    }

    fn toggle(&mut self, path: &str, cx: &mut Context<Self>) {
        if !self.expanded.remove(path) {
            self.expanded.insert(path.to_owned());
        }
        self.refresh_rows();
        cx.notify();
    }

    fn select(&mut self, row: &FileRow, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.as_deref() == Some(row.path.as_str()) {
            return;
        }
        self.selected = Some(row.path.clone());
        if row.dir {
            self.shown = None;
            cx.notify();
            return;
        }
        self.shown = Some(Shown::Loading);
        let archive = ApkFile { path: self.path.clone() };
        let path = row.path.clone();
        self._task = Some(cx.spawn_in(window, async move |this, cx| {
            let detail = cx.background_spawn(async move { analyze_apk::detail(&archive, &path) }).await;
            this.update_in(cx, |this, window, cx| {
                this.shown = Some(match detail {
                    Detail::Text { text, language } => Shown::Text(cx.new(|cx| {
                        let mut state = EditorState::new(window, cx).language(language).line_number(true).soft_wrap(false).default_value(text);
                        state.set_readonly(true, cx);
                        state
                    })),
                    Detail::Dex(summary) => Shown::Dex(summary),
                    Detail::None(message) => Shown::Message(message),
                });
                this.dex_expanded.clear();
                this.dex_selected = None;
                this.refresh_dex_rows();
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn render_rows(&mut self, range: Range<usize>, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let _ = window;
        let palette = cx.palette().clone();
        let total = self.root.as_ref().map_or(0, |r| r.download);
        range
            .filter_map(|ix| self.rows.get(ix).cloned().map(|r| (ix, r)))
            .map(|(ix, row)| {
                let selected = self.selected.as_deref() == Some(row.path.as_str());
                let expanded = self.expanded.contains(&row.path);
                let icon = if row.dir { Icon::new(icons::FOLDER) } else { crate::ui::common::file_icon(&row.name) };
                let (toggle_row, click_row) = (row.clone(), row.clone());
                h_flex()
                    .id(("apk-row", ix))
                    .h(px(ROW))
                    .w_full()
                    .pr_2()
                    .text_sm()
                    .whitespace_nowrap()
                    .when(selected, |el| el.bg(palette.selection))
                    .when(!selected, |el| el.hover(|el| el.bg(palette.hover)))
                    .on_click(cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                        this.select(&click_row, window, cx);
                        if click_row.dir && event.click_count() >= 2 {
                            this.toggle(&click_row.path, cx);
                        }
                    }))
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .pl(px(6. + row.depth as f32 * 16.))
                            .child(
                                div()
                                    .id(("apk-chevron", ix))
                                    .w(px(14.))
                                    .flex_none()
                                    .when(row.dir, |el| el.child(Icon::new(if expanded { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT }).xsmall()))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if toggle_row.dir {
                                            cx.stop_propagation();
                                            this.toggle(&toggle_row.path, cx);
                                        }
                                    })),
                            )
                            .child(icon.xsmall())
                            .child(div().min_w_0().overflow_hidden().text_ellipsis().child(row.name.clone())),
                    )
                    .child(div().w(px(SIZE_COL)).flex_none().text_right().child(apk::size(row.raw)))
                    .child(div().w(px(SIZE_COL)).flex_none().text_right().child(apk::size(row.download)))
                    .child(div().w(px(PERCENT_COL)).flex_none().text_right().child(apk::percent(row.download, total)))
                    .into_any_element()
            })
            .collect()
    }

    fn render_dex_rows(&mut self, range: Range<usize>, _: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let palette = cx.palette().clone();
        range
            .filter_map(|ix| self.dex_rows.get(ix).cloned().map(|r| (ix, r)))
            .map(|(ix, row)| {
                let selected = self.dex_selected.as_deref() == Some(row.key.as_str());
                let expanded = self.dex_expanded.contains(&row.key);
                let key = row.key.clone();
                let parent = row.parent;
                h_flex()
                    .id(("dex-row", ix))
                    .h(px(ROW))
                    .w_full()
                    .pr_2()
                    .text_sm()
                    .whitespace_nowrap()
                    .when(!row.defined_here, |el| el.text_color(palette.text_secondary))
                    .when(selected, |el| el.bg(palette.selection))
                    .when(!selected, |el| el.hover(|el| el.bg(palette.hover)))
                    .on_click(cx.listener(move |this, event: &gpui_kit::ClickEvent, _, cx| {
                        this.dex_selected = Some(key.clone());
                        if parent && event.click_count() >= 2 {
                            if !this.dex_expanded.remove(&key) {
                                this.dex_expanded.insert(key.clone());
                            }
                            this.refresh_dex_rows();
                        }
                        cx.notify();
                    }))
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .pl(px(6. + row.depth as f32 * 16.))
                            .child({
                                let key = row.key.clone();
                                div()
                                    .id(("dex-chevron", ix))
                                    .w(px(14.))
                                    .flex_none()
                                    .when(parent, |el| el.child(Icon::new(if expanded { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT }).xsmall()))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if parent {
                                            cx.stop_propagation();
                                            if !this.dex_expanded.remove(&key) {
                                                this.dex_expanded.insert(key.clone());
                                            }
                                            this.refresh_dex_rows();
                                            cx.notify();
                                        }
                                    }))
                            })
                            .child(Icon::new(if row.class { icons::CLASS } else { icons::PACKAGE }).xsmall())
                            .child(div().min_w_0().overflow_hidden().text_ellipsis().child(row.name.clone())),
                    )
                    .child(div().w(px(SIZE_COL + 20.)).flex_none().text_right().child(dex::group(row.defined)))
                    .child(div().w(px(SIZE_COL + 40.)).flex_none().text_right().child(dex::group(row.referenced)))
                    .into_any_element()
            })
            .collect()
    }
}

fn header(columns: Vec<(&'static str, Option<f32>)>, palette: &crate::theme::Palette) -> gpui_kit::Div {
    let mut row = h_flex().h(px(ROW)).flex_none().pr_2().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(palette.text_secondary).border_b_1().border_color(palette.border);
    for (label, width) in columns {
        row = row.child(match width {
            None => div().flex_1().pl(px(26.)).child(label),
            Some(w) => div().w(px(w)).flex_none().text_right().child(label),
        });
    }
    row
}

impl Render for ApkAnalyzer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let summary = match &self.root {
            Some(root) => format!("APK size: {}, Download Size: {}", apk::size(self.file_size), apk::size(root.download)),
            None => format!("APK size: {}, analyzing…", apk::size(self.file_size)),
        };
        let files = v_flex()
            .flex_1()
            .min_h_0()
            .child(header(vec![("File", None), ("Raw File Size", Some(SIZE_COL)), ("Download Size", Some(SIZE_COL)), ("% of Total Download Size", Some(PERCENT_COL))], &palette))
            .child(div().flex_1().min_h_0().child(uniform_list("apk-files", self.rows.len(), cx.processor(Self::render_rows)).track_scroll(&self.scroll).size_full()));
        let detail: Option<AnyElement> = match &self.shown {
            None => None,
            Some(Shown::Loading) => Some(div().p_3().text_sm().text_color(palette.text_secondary).child("Loading…").into_any_element()),
            Some(Shown::Message(message)) => Some(div().p_3().text_sm().text_color(palette.text_secondary).child(message.clone()).into_any_element()),
            Some(Shown::Text(state)) => Some(Editor::new(state).bordered(false).h_full().into_any_element()),
            Some(Shown::Dex(summary)) => Some(
                v_flex()
                    .size_full()
                    .child(div().px_2().py_1().text_sm().flex_none().child(summary.headline()))
                    .child(header(vec![("Class", None), ("Defined Methods", Some(SIZE_COL + 20.)), ("Referenced Methods", Some(SIZE_COL + 40.))], &palette))
                    .child(div().flex_1().min_h_0().child(uniform_list("apk-dex", self.dex_rows.len(), cx.processor(Self::render_dex_rows)).track_scroll(&self.dex_scroll).size_full()))
                    .into_any_element(),
            ),
        };
        v_flex()
            .size_full()
            .bg(palette.panel)
            .child(h_flex().h(px(30.)).flex_none().px_2().text_sm().border_b_1().border_color(palette.border).child(summary))
            .child(files)
            .when_some(detail, |el, detail| el.child(div().flex_1().min_h_0().border_t_1().border_color(palette.border).child(detail)))
    }
}
