//! The editor area's diff viewer, after IntelliJ's: side-by-side and unified
//! viewers, word / line highlighting, whitespace options, collapsed unchanged
//! fragments, and Previous / Next Difference (Shift+F7 / F7).

use std::collections::HashSet;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, h_flex,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::assets::IconName;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, HighlightStyle, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, ScrollStrategy, StatefulInteractiveElement as _, Styled as _,
    StyledText, Task, UniformListScrollHandle, Window, actions, div, prelude::FluentBuilder as _, px,
    uniform_list,
};

use crate::git::Repository;
use crate::git::diff::{self, DiffOptions, DiffRow, FileDiff, HighlightMode, HunkAction, IgnoreWhitespace, Revisions, RowKind, Side};
use crate::theme::{ActivePalette as _, Palette};
use crate::ui::common::{self, tool_button};

actions!(diff_view, [NextDifference, PreviousDifference]);

const LINE_HEIGHT: f32 = 20.;
const GUTTER_WIDTH: f32 = 44.;
const TAB_WIDTH: usize = 4;

/// What to compare.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffSource {
    /// A file as changed by a commit (against its first parent).
    Commit { hash: String, path: String, old_path: Option<String> },
    /// A file in the working tree against HEAD.
    WorkingTree { path: String, unversioned: bool },
    /// Staging-area mode: staged (HEAD vs index) or unstaged (index vs work tree).
    Staged { path: String },
    Unstaged { path: String },
    /// Two revisions, or a revision against the working tree (`new: None`).
    Between { old: String, new: Option<String>, path: String, old_path: Option<String> },
}

impl DiffSource {
    fn path(&self) -> &str {
        match self {
            DiffSource::Commit { path, .. }
            | DiffSource::WorkingTree { path, .. }
            | DiffSource::Staged { path }
            | DiffSource::Unstaged { path }
            | DiffSource::Between { path, .. } => path,
        }
    }

    fn revisions(&self) -> Revisions {
        match self.clone() {
            DiffSource::Commit { hash, path, old_path } => Revisions::Commit { hash, path, old_path },
            DiffSource::WorkingTree { path, .. } => Revisions::WorkingTree { path },
            DiffSource::Staged { path } => Revisions::Staged { path },
            DiffSource::Unstaged { path } => Revisions::Unstaged { path },
            DiffSource::Between { old, new, path, old_path } => Revisions::Between { old, new, path, old_path },
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ViewerMode {
    SideBySide,
    Unified,
}

/// A row as displayed: folds that were expanded are inlined, and in the
/// unified viewer each side of a changed row gets its own row.
#[derive(Clone)]
enum Display {
    Line { kind: RowKind, left: Option<Side>, right: Option<Side>, change: Option<usize> },
    Fold { id: usize, count: usize },
}

struct Loaded {
    old: String,
    new: String,
    old_title: String,
    new_title: String,
}

/// The diff changed a file (a gutter Revert / Stage / Unstage).
pub struct FilesChanged;

impl gpui_kit::EventEmitter<FilesChanged> for DiffView {}

pub struct DiffView {
    repository: Option<Repository>,
    source: Option<DiffSource>,
    loaded: Option<Loaded>,
    diff: FileDiff,
    options: DiffOptions,
    mode: ViewerMode,
    expanded: HashSet<usize>,
    rows: Rc<Vec<Display>>,
    current: Option<usize>,
    error: Option<String>,
    scroll: UniformListScrollHandle,
    _task: Option<Task<()>>,
}

impl DiffView {
    pub fn new() -> Self {
        Self {
            repository: None,
            source: None,
            loaded: None,
            diff: FileDiff::default(),
            options: DiffOptions::default(),
            mode: ViewerMode::SideBySide,
            expanded: HashSet::new(),
            rows: Rc::new(Vec::new()),
            current: None,
            error: None,
            scroll: UniformListScrollHandle::new(),
            _task: None,
        }
    }

    pub fn show(&mut self, repository: Repository, source: DiffSource, cx: &mut Context<Self>) {
        if self.source.as_ref() == Some(&source) {
            return;
        }
        self.repository = Some(repository.clone());
        self.source = Some(source.clone());
        self.loaded = None;
        self.diff = FileDiff::default();
        self.rows = Rc::new(Vec::new());
        self.error = None;
        cx.notify();
        let options = self.options;
        self._task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let (old, new, old_title, new_title) = diff::load_versions(&repository, &source.revisions())?;
                    let file_diff = diff::compute(&old, &new, options);
                    anyhow::Ok((Loaded { old, new, old_title, new_title }, file_diff))
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok((loaded, file_diff)) => {
                        this.loaded = Some(loaded);
                        this.set_diff(file_diff);
                        this.go_to_change(0);
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// The gutter actions IntelliJ offers for this kind of diff.
    fn hunk_actions(&self) -> Vec<(HunkAction, IconName, &'static str)> {
        match &self.source {
            Some(DiffSource::WorkingTree { unversioned: false, .. }) => vec![(HunkAction::Revert, IconName::Undo2, "Rollback")],
            Some(DiffSource::Unstaged { .. }) => {
                vec![(HunkAction::Stage, IconName::Plus, "Stage"), (HunkAction::Revert, IconName::Undo2, "Rollback")]
            }
            Some(DiffSource::Staged { .. }) => vec![(HunkAction::Unstage, IconName::Minus, "Unstage")],
            _ => Vec::new(),
        }
    }

    fn apply_hunk(&mut self, change: usize, action: HunkAction, cx: &mut Context<Self>) {
        let (Some(repository), Some(source), Some(loaded)) = (self.repository.clone(), self.source.clone(), self.loaded.as_ref()) else {
            return;
        };
        let Some(hunk) = self.diff.hunks.get(change) else { return };
        match diff::apply_hunk(&repository, &source.revisions(), &loaded.old, &loaded.new, hunk, action) {
            Ok(()) => {
                // Reload this diff and let the workspace refresh the status.
                self.source = None;
                self.show(repository, source, cx);
                cx.emit(FilesChanged);
            }
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
            }
        }
    }

    /// Partial commit checkboxes: working-tree diffs in changelist mode, whitespace not ignored.
    fn partial_path(&self, cx: &App) -> Option<String> {
        match &self.source {
            Some(DiffSource::WorkingTree { path, unversioned: false })
                if !crate::settings::Settings::get(cx).staging_area
                    && self.options.ignore_whitespace == diff::IgnoreWhitespace::None
                    && !self.diff.binary =>
            {
                Some(path.clone())
            }
            _ => None,
        }
    }

    fn signature(&self, change: usize) -> Option<u64> {
        let loaded = self.loaded.as_ref()?;
        Some(diff::hunk_signature(&loaded.old, &loaded.new, self.diff.hunks.get(change)?))
    }

    fn toggle_hunk(&mut self, change: usize, include: bool, cx: &mut Context<Self>) {
        let (Some(path), Some(signature)) = (self.partial_path(cx), self.signature(change)) else { return };
        crate::model::ExcludedHunks::update(cx, |map| {
            let set = map.entry(path).or_default();
            if include {
                set.remove(&signature);
            } else {
                set.insert(signature);
            }
        });
        cx.notify();
    }

    fn set_diff(&mut self, file_diff: FileDiff) {
        self.diff = file_diff;
        self.expanded.clear();
        self.current = None;
        self.rebuild_rows();
    }

    fn recompute(&mut self, cx: &mut Context<Self>) {
        if let Some(loaded) = &self.loaded {
            let file_diff = diff::compute(&loaded.old, &loaded.new, self.options);
            self.set_diff(file_diff);
        }
        cx.notify();
    }

    fn rebuild_rows(&mut self) {
        let mut out = Vec::new();
        flatten(&self.diff.rows, &self.expanded, self.mode, &mut out);
        self.rows = Rc::new(out);
    }

    fn set_mode(&mut self, mode: ViewerMode, cx: &mut Context<Self>) {
        self.mode = mode;
        self.rebuild_rows();
        if let Some(change) = self.current {
            self.go_to_change(change);
        }
        cx.notify();
    }

    fn update_options(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut DiffOptions)) {
        f(&mut self.options);
        self.recompute(cx);
    }

    fn expand_fold(&mut self, id: usize, cx: &mut Context<Self>) {
        self.expanded.insert(id);
        self.rebuild_rows();
        cx.notify();
    }

    fn change_row(&self, change: usize) -> Option<usize> {
        self.rows.iter().position(|row| matches!(row, Display::Line { change: Some(c), .. } if *c == change))
    }

    fn go_to_change(&mut self, change: usize) {
        if let Some(row) = self.change_row(change) {
            self.current = Some(change);
            self.scroll.scroll_to_item(row, ScrollStrategy::Center);
        }
    }

    fn has_next(&self) -> bool {
        self.current.map_or(self.diff.changes > 0, |c| c + 1 < self.diff.changes)
    }

    fn has_previous(&self) -> bool {
        self.current.is_some_and(|c| c > 0)
    }

    pub fn next_difference(&mut self, cx: &mut Context<Self>) {
        if self.has_next() {
            self.go_to_change(self.current.map_or(0, |c| c + 1));
            cx.notify();
        }
    }

    pub fn previous_difference(&mut self, cx: &mut Context<Self>) {
        if let Some(c) = self.current.filter(|c| *c > 0) {
            self.go_to_change(c - 1);
            cx.notify();
        }
    }

    fn render_toolbar(&self, source: &DiffSource, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let entity = cx.entity();
        let mode = self.mode;
        let options = self.options;

        let viewer_label = match mode {
            ViewerMode::SideBySide => "Side-by-side viewer",
            ViewerMode::Unified => "Unified viewer",
        };
        let whitespace_label = match options.ignore_whitespace {
            IgnoreWhitespace::None => "Do not ignore",
            IgnoreWhitespace::Trim => "Trim whitespaces",
            IgnoreWhitespace::All => "Ignore whitespaces",
        };
        let highlight_label = match options.highlight {
            HighlightMode::Words => "Highlight words",
            HighlightMode::Lines => "Highlight lines",
            HighlightMode::None => "Do not highlight",
        };
        let separator = || div().w(px(1.)).h(px(16.)).mx_1().bg(palette.border);
        let summary = if self.diff.binary {
            "Binary files differ".to_owned()
        } else if self.loaded.is_none() {
            String::new()
        } else {
            match self.diff.changes {
                0 => "No differences".to_owned(),
                1 => "1 difference".to_owned(),
                n => format!("{n} differences"),
            }
        };

        h_flex()
            .h(px(30.))
            .px_2()
            .gap_0p5()
            .border_b_1()
            .border_color(palette.border)
            .bg(palette.toolbar)
            .child(
                tool_button("diff-prev", IconName::ChevronUp, "Previous Difference (Shift+F7)")
                    .disabled(!self.has_previous())
                    .on_click(cx.listener(|this, _, _, cx| this.previous_difference(cx))),
            )
            .child(
                tool_button("diff-next", IconName::ChevronDown, "Next Difference (F7)")
                    .disabled(!self.has_next())
                    .on_click(cx.listener(|this, _, _, cx| this.next_difference(cx))),
            )
            .child(separator())
            .child(Button::new("diff-viewer").ghost().xsmall().label(viewer_label).dropdown_menu({
                let entity = entity.clone();
                move |menu, _, _| {
                    let (a, b) = (entity.clone(), entity.clone());
                    menu.item(
                        PopupMenuItem::new("Side-by-side viewer")
                            .checked(mode == ViewerMode::SideBySide)
                            .on_click(move |_, _, cx| a.update(cx, |this, cx| this.set_mode(ViewerMode::SideBySide, cx))),
                    )
                    .item(
                        PopupMenuItem::new("Unified viewer")
                            .checked(mode == ViewerMode::Unified)
                            .on_click(move |_, _, cx| b.update(cx, |this, cx| this.set_mode(ViewerMode::Unified, cx))),
                    )
                }
            }))
            .child(Button::new("diff-whitespace").ghost().xsmall().label(whitespace_label).dropdown_menu({
                let entity = entity.clone();
                move |mut menu, _, _| {
                    for (label, value) in [
                        ("Do not ignore", IgnoreWhitespace::None),
                        ("Trim whitespaces", IgnoreWhitespace::Trim),
                        ("Ignore whitespaces", IgnoreWhitespace::All),
                    ] {
                        let entity = entity.clone();
                        menu = menu.item(PopupMenuItem::new(label).checked(options.ignore_whitespace == value).on_click(
                            move |_, _, cx| entity.update(cx, |this, cx| this.update_options(cx, |o| o.ignore_whitespace = value)),
                        ));
                    }
                    menu
                }
            }))
            .child(Button::new("diff-highlight").ghost().xsmall().label(highlight_label).dropdown_menu({
                let entity = entity.clone();
                move |mut menu, _, _| {
                    for (label, value) in [
                        ("Highlight words", HighlightMode::Words),
                        ("Highlight lines", HighlightMode::Lines),
                        ("Do not highlight", HighlightMode::None),
                    ] {
                        let entity = entity.clone();
                        menu = menu.item(PopupMenuItem::new(label).checked(options.highlight == value).on_click(
                            move |_, _, cx| entity.update(cx, |this, cx| this.update_options(cx, |o| o.highlight = value)),
                        ));
                    }
                    menu
                }
            }))
            .child(
                tool_button("diff-collapse", IconName::FoldVertical, "Collapse Unchanged Fragments")
                    .selected(options.context.is_some())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.update_options(cx, |o| o.context = if o.context.is_some() { None } else { Some(4) })
                    })),
            )
            .child(separator())
            .child(common::icon(common::file_icon(source.path())).text_color(palette.text_secondary))
            .child(div().ml_1().text_sm().overflow_hidden().whitespace_nowrap().text_ellipsis().child(source.path().to_owned()))
            .child(div().flex_1())
            .when(!self.diff.binary && self.loaded.is_some(), |el| {
                el.child(
                    h_flex()
                        .gap_1p5()
                        .text_xs()
                        .mr_2()
                        .child(div().text_color(palette.status_added).child(format!("+{}", self.diff.inserted)))
                        .child(div().text_color(palette.status_deleted).child(format!("−{}", self.diff.deleted))),
                )
            })
            .child(div().text_xs().text_color(palette.text_secondary).child(summary))
    }
}

/// Flattens diff rows for display. Unified mode lists a change block's
/// deleted lines before its inserted ones, like `git diff`.
fn flatten(rows: &[DiffRow], expanded: &HashSet<usize>, mode: ViewerMode, out: &mut Vec<Display>) {
    let mut block: Option<usize> = None;
    let (mut deleted, mut inserted): (Vec<Display>, Vec<Display>) = (Vec::new(), Vec::new());
    let flush = |deleted: &mut Vec<Display>, inserted: &mut Vec<Display>, out: &mut Vec<Display>| {
        out.append(deleted);
        out.append(inserted);
    };
    for row in rows {
        match row {
            DiffRow::Fold { id, rows } => {
                flush(&mut deleted, &mut inserted, out);
                block = None;
                if expanded.contains(id) {
                    flatten(rows, expanded, mode, out);
                } else {
                    out.push(Display::Fold { id: *id, count: rows.len() });
                }
            }
            DiffRow::Line { kind, left, right, change } => {
                if mode == ViewerMode::SideBySide || change.is_none() {
                    flush(&mut deleted, &mut inserted, out);
                    block = None;
                    out.push(Display::Line { kind: *kind, left: left.clone(), right: right.clone(), change: *change });
                    continue;
                }
                if block != *change {
                    flush(&mut deleted, &mut inserted, out);
                    block = *change;
                }
                if let Some(left) = left {
                    deleted.push(Display::Line { kind: *kind, left: Some(left.clone()), right: None, change: *change });
                }
                if let Some(right) = right {
                    inserted.push(Display::Line { kind: *kind, left: None, right: Some(right.clone()), change: *change });
                }
            }
        }
    }
    flush(&mut deleted, &mut inserted, out);
}

/// Expands tabs to spaces, moving highlight ranges along with the text.
fn expand_tabs(text: &str, ranges: &[Range<usize>]) -> (String, Vec<Range<usize>>) {
    if !text.contains('\t') {
        return (text.to_owned(), ranges.to_vec());
    }
    let mut out = String::with_capacity(text.len() + 16);
    let mut map = Vec::with_capacity(text.len() + 1);
    let mut column = 0;
    for ch in text.chars() {
        for _ in 0..ch.len_utf8() {
            map.push(out.len());
        }
        if ch == '\t' {
            let spaces = TAB_WIDTH - column % TAB_WIDTH;
            out.extend(std::iter::repeat_n(' ', spaces));
            column += spaces;
        } else {
            out.push(ch);
            column += 1;
        }
    }
    map.push(out.len());
    let ranges = ranges.iter().map(|r| map[r.start]..map[r.end]).collect();
    (out, ranges)
}

fn line_text(side: &Side, word_color: Hsla, text_color: Hsla) -> AnyElement {
    let (text, ranges) = expand_tabs(&side.text, &side.changed);
    let highlights = ranges
        .into_iter()
        .filter(|r| !r.is_empty())
        .map(|r| (r, HighlightStyle { background_color: Some(word_color), ..Default::default() }));
    div()
        .pl_2()
        .text_color(text_color)
        .whitespace_nowrap()
        .child(StyledText::new(text).with_highlights(highlights))
        .into_any_element()
}

fn gutter(number: Option<usize>, palette: &Palette) -> impl IntoElement {
    div()
        .w(px(GUTTER_WIDTH))
        .flex_shrink_0()
        .text_right()
        .pr_2()
        .text_color(palette.text_disabled)
        .child(number.map(|n| n.to_string()).unwrap_or_default())
}

/// Line background and word highlight colors for one side of a row.
fn side_colors(kind: RowKind, is_left: bool, unified: bool, palette: &Palette) -> Option<(Hsla, Hsla)> {
    match kind {
        RowKind::Equal => None,
        RowKind::Modified if !unified => Some((palette.diff_modified, palette.diff_modified_word)),
        RowKind::Deleted | RowKind::Modified if is_left => Some((palette.diff_deleted, palette.diff_deleted_word)),
        RowKind::Inserted | RowKind::Modified if !is_left => Some((palette.diff_inserted, palette.diff_inserted_word)),
        _ => None,
    }
}

impl Render for DiffView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let Some(source) = self.source.clone() else {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .text_color(palette.text_secondary)
                .child(div().text_lg().child("Select a file to view changes"))
                .child(div().text_sm().child("Double-click a file in the Log or the Commit tool window"))
                .into_any_element();
        };
        let mono = cx.theme().mono_font_family.clone();
        let rows = self.rows.clone();
        let count = rows.len();
        let actions = Rc::new(self.hunk_actions());
        let partial = self.partial_path(cx);
        // Per change: whether it goes into the next commit (partial commits).
        let included: Rc<Vec<bool>> = Rc::new(match &partial {
            Some(path) => {
                let excluded = crate::model::ExcludedHunks::get(cx).get(path).cloned().unwrap_or_default();
                (0..self.diff.hunks.len()).map(|c| self.signature(c).is_none_or(|s| !excluded.contains(&s))).collect()
            }
            None => Vec::new(),
        });
        let has_partial = partial.is_some();
        let has_actions = !actions.is_empty() || has_partial;
        let button_count = actions.len() + has_partial as usize;
        // The first row of each change carries its gutter buttons.
        let starts: Rc<HashSet<usize>> = Rc::new(
            rows.iter()
                .enumerate()
                .filter_map(|(ix, row)| match row {
                    Display::Line { change: Some(c), .. } => {
                        let previous = ix.checked_sub(1).and_then(|p| match &rows[p] {
                            Display::Line { change, .. } => *change,
                            Display::Fold { .. } => None,
                        });
                        (previous != Some(*c)).then_some(ix)
                    }
                    _ => None,
                })
                .collect(),
        );
        let mode = self.mode;
        let highlight = self.options.highlight;
        let current = self.current;
        let titles = self.loaded.as_ref().map(|l| (l.old_title.clone(), l.new_title.clone()));

        let banner = match &self.loaded {
            Some(_) if self.diff.binary => Some("Binary files differ".to_owned()),
            Some(loaded) if self.diff.changes == 0 && loaded.old != loaded.new => {
                Some("Contents have differences only in whitespaces".to_owned())
            }
            Some(_) if self.diff.changes == 0 => Some("Contents are identical".to_owned()),
            _ => None,
        };

        let list = uniform_list(
            "diff-lines",
            count,
            cx.processor(move |_, range: Range<usize>, _, cx| {
                let palette = cx.palette().clone();
                // Gutter buttons for a change's first row.
                let hunk_buttons = |ix: usize, change: Option<usize>, cx: &mut Context<DiffView>| {
                    let mut el = h_flex().w(px(if has_actions { 18. * button_count as f32 } else { 1. })).h_full().flex_shrink_0().justify_center().items_center();
                    if !has_actions {
                        return el.bg(palette.border);
                    }
                    if let (Some(change), true) = (change, starts.contains(&ix)) {
                        if has_partial {
                            let checked = included.get(change).copied().unwrap_or(true);
                            el = el.child(
                                Checkbox::new(gpui_kit::ElementId::NamedInteger("hunk-include".into(), ix as u64))
                                    .checked(checked)
                                    .tooltip("Include in commit")
                                    .on_change(cx.listener(move |this, value: &bool, _, cx| this.toggle_hunk(change, *value, cx))),
                            );
                        }
                        for (action, icon, tooltip) in actions.iter().copied() {
                            el = el.child(
                                tool_button(gpui_kit::ElementId::NamedInteger(format!("hunk-{tooltip}").into(), ix as u64), icon, tooltip)
                                    .on_click(cx.listener(move |this, _, _, cx| this.apply_hunk(change, action, cx))),
                            );
                        }
                    }
                    el
                };
                range
                    .map(|ix| match &rows[ix] {
                        Display::Fold { id, count } => {
                            let id = *id;
                            h_flex()
                                .id(("diff-fold", id))
                                .h(px(LINE_HEIGHT))
                                .w_full()
                                .gap_1()
                                .pl(px(GUTTER_WIDTH * if mode == ViewerMode::Unified { 2. } else { 1. } - 14.))
                                .bg(palette.diff_header)
                                .text_color(palette.text_secondary)
                                .cursor_pointer()
                                .hover(|s| s.text_color(palette.text))
                                .on_click(cx.listener(move |this, _, _, cx| this.expand_fold(id, cx)))
                                .child(common::icon(IconName::ChevronRight))
                                .child(format!("{count} unchanged lines"))
                                .into_any_element()
                        }
                        Display::Line { kind, left, right, change } => {
                            let kind = *kind;
                            let is_current = current.is_some() && *change == current;
                            let unified = mode == ViewerMode::Unified;
                            let paint = |is_left: bool| {
                                let colors = side_colors(kind, is_left, unified, &palette);
                                match highlight {
                                    HighlightMode::None => (None, palette.text, colors.map(|c| c.1)),
                                    _ => (colors.map(|c| c.0), palette.text, colors.map(|c| c.1)),
                                }
                            };
                            // IntelliJ marks changed lines with a stripe even when highlighting is off.
                            let marker = |color: Option<Hsla>| {
                                div()
                                    .w(px(3.))
                                    .h_full()
                                    .flex_shrink_0()
                                    .when_some(color.filter(|_| highlight == HighlightMode::None || is_current), |el, c| {
                                        el.bg(c)
                                    })
                            };
                            if unified {
                                let is_left = right.is_none();
                                let (bg, fg, word) = paint(is_left);
                                let side = left.as_ref().or(right.as_ref());
                                h_flex()
                                    .id(ix)
                                    .h(px(LINE_HEIGHT))
                                    .w_full()
                                    .when_some(bg, |el, bg| el.bg(bg))
                                    .when(has_actions, |el| el.child(hunk_buttons(ix, *change, cx)))
                                    .child(marker(word))
                                    .child(gutter(left.as_ref().map(|s| s.line), &palette))
                                    .child(gutter(right.as_ref().map(|s| s.line), &palette))
                                    .when_some(side, |el, side| {
                                        el.child(line_text(side, word.unwrap_or(palette.diff_header), fg))
                                    })
                                    .into_any_element()
                            } else {
                                let cell = |side: Option<&Side>, is_left: bool| {
                                    let (bg, fg, word) = paint(is_left);
                                    let bg = if side.is_some() { bg } else { None };
                                    h_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .h_full()
                                        .overflow_hidden()
                                        .when_some(bg, |el, bg| el.bg(bg))
                                        .child(marker(word.filter(|_| side.is_some())))
                                        .child(gutter(side.map(|s| s.line), &palette))
                                        .when_some(side, |el, side| {
                                            el.child(line_text(side, word.unwrap_or(palette.diff_header), fg))
                                        })
                                };
                                h_flex()
                                    .id(ix)
                                    .h(px(LINE_HEIGHT))
                                    .w_full()
                                    .child(cell(left.as_ref(), true))
                                    .child(hunk_buttons(ix, *change, cx))
                                    .child(cell(right.as_ref(), false))
                                    .into_any_element()
                            }
                        }
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.scroll)
        .font_family(mono)
        .text_size(px(12.5))
        .flex_1()
        .w_full();

        v_flex()
            .size_full()
            .child(self.render_toolbar(&source, cx))
            .when(mode == ViewerMode::SideBySide, |el| {
                el.when_some(titles, |el, (old_title, new_title)| {
                    el.child(
                        h_flex()
                            .h(px(22.))
                            .text_xs()
                            .text_color(palette.text_secondary)
                            .border_b_1()
                            .border_color(palette.border)
                            .child(div().flex_1().pl(px(GUTTER_WIDTH + 11.)).child(old_title))
                            .child(div().w(px(1.)).h_full().bg(palette.border))
                            .child(div().flex_1().pl(px(GUTTER_WIDTH + 11.)).child(new_title)),
                    )
                })
            })
            .when_some(banner, |el, banner| {
                el.child(
                    div()
                        .px_3()
                        .py_1()
                        .text_sm()
                        .bg(palette.diff_header)
                        .border_b_1()
                        .border_color(palette.border)
                        .child(banner),
                )
            })
            .when_some(self.error.clone(), |el, error| el.child(div().p_3().text_color(palette.status_conflict).child(error)))
            .child(list)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unified_lists_deletions_before_insertions() {
        let file_diff = diff::compute("a\nb\nc\n", "a\nB\nC\n", DiffOptions { context: None, ..Default::default() });
        let mut out = Vec::new();
        flatten(&file_diff.rows, &HashSet::new(), ViewerMode::Unified, &mut out);
        let shape: Vec<(bool, bool)> = out
            .iter()
            .map(|d| match d {
                Display::Line { left, right, .. } => (left.is_some(), right.is_some()),
                Display::Fold { .. } => (false, false),
            })
            .collect();
        assert_eq!(shape, vec![(true, true), (true, false), (true, false), (false, true), (false, true)]);
    }

    #[test]
    fn tabs_expand_with_highlights() {
        let (text, ranges) = expand_tabs("\tab\tc", &[1..3, 4..5]);
        assert_eq!(text, "    ab  c");
        assert_eq!(&text[ranges[0].clone()], "ab");
        assert_eq!(&text[ranges[1].clone()], "c");
    }
}
