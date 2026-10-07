//! The editor area's diff viewer, after IntelliJ's: side-by-side and unified
//! viewers, word / line highlighting, whitespace options, collapsed unchanged
//! fragments, and Previous / Next Difference (Shift+F7 / F7).

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, h_flex,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::assets::IconName;
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, Context, HighlightStyle, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, ScrollWheelEvent, canvas, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, ScrollStrategy, StatefulInteractiveElement as _, Styled as _,
    Image, ImageFormat, ObjectFit, StyledImage as _, StyledText, Task, img, UniformListScrollHandle, Window, actions, div, prelude::FluentBuilder as _, px,
    uniform_list, fill, point, size,
};

use crate::git::Repository;
use crate::git::blob::{self, ImageInfo, ImageKind};
use crate::git::diff::{self, DiffOptions, DiffRow, FileDiff, HighlightMode, HunkAction, IgnoreWhitespace, Revisions, RowKind, Side};
use crate::theme::{ActivePalette as _, Palette};
use crate::ui::common::{self, tool_button};
use crate::ui::diff_panes::{BlockColors, Connector, DIVIDER_WIDTH, PaneRow, TwoSide, fold_links, map_row, paint_divider};

actions!(diff_view, [NextDifference, PreviousDifference, JumpToSource]);

use crate::ui::diff_panes::LINE_HEIGHT;
const GUTTER_WIDTH: f32 = 44.;
const TAB_WIDTH: usize = 4;
/// Approximate advance of the 12.5px monospace font, for horizontal scroll bounds.
const CHAR_WIDTH: f32 = 7.6;
/// One gutter button column.
const BUTTON_WIDTH: f32 = 18.;
const STRIPE_WIDTH: f32 = 12.;

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
    /// Binary files: each side's image or size, shown instead of lines.
    binary: Option<[Option<BinaryPane>; 2]>,
}

/// One side of IntelliJ's binary / image diff.
struct BinaryPane {
    size: usize,
    image: Option<(Arc<Image>, ImageInfo)>,
}

impl BinaryPane {
    fn new(bytes: Vec<u8>) -> Self {
        let info = blob::image_info(&bytes);
        let size = bytes.len();
        let image = info.map(|info| {
            let format = match info.kind {
                ImageKind::Png => ImageFormat::Png,
                ImageKind::Jpeg => ImageFormat::Jpeg,
                ImageKind::Gif => ImageFormat::Gif,
                ImageKind::Bmp => ImageFormat::Bmp,
                ImageKind::Webp => ImageFormat::Webp,
                ImageKind::Ico => ImageFormat::Ico,
            };
            (Arc::new(Image::from_bytes(format, bytes)), info)
        });
        Self { size, image }
    }

    /// "64x32 PNG 1.2 kB", the image viewer's status line.
    fn describe(&self) -> String {
        match &self.image {
            Some((_, info)) => format!("{}x{} {} {}", info.width, info.height, info.kind.name(), blob::format_size(self.size)),
            None => blob::format_size(self.size),
        }
    }
}

/// The diff changed a file (a gutter Revert / Stage / Unstage).
pub struct FilesChanged;

impl gpui_kit::EventEmitter<FilesChanged> for DiffView {}

/// A pull request diff: new-side line numbers take review comments.
#[derive(Clone, Default)]
pub struct Review {
    /// Existing comments per new-side line, as "author: text".
    pub comments: HashMap<usize, Vec<String>>,
}

/// A click on a new-side line number of a review diff.
pub struct CommentLine {
    pub path: String,
    pub line: usize,
}

impl gpui_kit::EventEmitter<CommentLine> for DiffView {}
impl gpui_kit::EventEmitter<crate::ui::navigate::OpenTarget> for DiffView {}

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
    review: Option<Rc<Review>>,
    /// The side-by-side viewer's two panes.
    two: Rc<TwoSide>,
    /// Each pane's scroll position (x, y) in pixels.
    pane_scroll: [(f32, f32); 2],
    /// Synchronize Scrolling (the toolbar toggle).
    sync_scroll: bool,
    /// The panes' visible height, measured while painting.
    view_height: Rc<Cell<f32>>,
    /// The error stripes' bounds, for clicks and drags on them.
    stripe_bounds: Rc<Cell<[Bounds<Pixels>; 2]>>,
    stripe_drag: Option<usize>,
    /// A change to scroll to once the panes have been measured.
    pending_change: Option<usize>,
    /// Each pane's longest line, in columns, bounding horizontal scroll.
    max_cols: [usize; 2],
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
            review: None,
            two: Rc::default(),
            pane_scroll: [(0., 0.); 2],
            sync_scroll: true,
            view_height: Rc::new(Cell::new(0.)),
            stripe_bounds: Rc::new(Cell::new([Bounds::default(); 2])),
            stripe_drag: None,
            pending_change: None,
            max_cols: [0; 2],
            _task: None,
        }
    }

    pub fn show(&mut self, repository: Repository, source: DiffSource, cx: &mut Context<Self>) {
        if self.source.as_ref() == Some(&source) {
            return;
        }
        self.repository = Some(repository.clone());
        self.source = Some(source.clone());
        self.review = None;
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
                    let binary = file_diff.binary.then(|| {
                        let sides = blob::load(&repository, &source.revisions());
                        [sides.old.map(BinaryPane::new), sides.new.map(BinaryPane::new)]
                    });
                    anyhow::Ok((Loaded { old, new, old_title, new_title, binary }, file_diff))
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

    /// Marks the shown diff as a pull request's, taking line comments.
    pub fn set_review(&mut self, review: Option<Review>, cx: &mut Context<Self>) {
        self.review = review.map(Rc::new);
        cx.notify();
    }

    /// The gutter actions IntelliJ offers for this kind of diff.
    fn hunk_actions(&self) -> Vec<(HunkAction, IconName, &'static str)> {
        // A submodule pointer has no lines to roll back or stage piecewise.
        let submodule = |text: &str| text.starts_with("Subproject commit ");
        if self.loaded.as_ref().is_some_and(|l| submodule(&l.old) || submodule(&l.new)) {
            return Vec::new();
        }
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
        self.two = Rc::new(TwoSide::build(&self.diff.rows, &self.expanded));
        for pane in 0..2 {
            self.max_cols[pane] = self
                .pane_rows(pane)
                .iter()
                .map(|row| match row {
                    PaneRow::Line { side, .. } => side.text.chars().map(|c| if c == '\t' { TAB_WIDTH } else { 1 }).sum(),
                    PaneRow::Fold { .. } => 0,
                })
                .max()
                .unwrap_or(0);
        }
        for pane in 0..2 {
            self.pane_scroll[pane].1 = self.pane_scroll[pane].1.min(self.max_scroll_y(pane));
        }
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
        if self.mode == ViewerMode::SideBySide {
            let Some(seg) = self.two.change_segment(change).cloned() else { return };
            self.current = Some(change);
            let height = self.view_height.get();
            if height <= 0. {
                self.pending_change = Some(change);
                return;
            }
            // Both panes put the change's first row a third of the way down.
            let top = height / 3.;
            self.pane_scroll[0].1 = (seg.left.start as f32 * LINE_HEIGHT - top).clamp(0., self.max_scroll_y(0));
            self.pane_scroll[1].1 = (seg.right.start as f32 * LINE_HEIGHT - top).clamp(0., self.max_scroll_y(1));
            return;
        }
        if let Some(row) = self.change_row(change) {
            self.current = Some(change);
            self.scroll.scroll_to_item(row, ScrollStrategy::Center);
        }
    }

    fn pane_rows(&self, pane: usize) -> &[PaneRow] {
        if pane == 0 { &self.two.left } else { &self.two.right }
    }

    /// Lets the last line scroll up to the middle of the pane.
    fn max_scroll_y(&self, pane: usize) -> f32 {
        let content = self.pane_rows(pane).len() as f32 * LINE_HEIGHT;
        (content - self.view_height.get() / 2.).max(0.)
    }

    /// Scrolls one pane; with Synchronize Scrolling the other follows so
    /// the rows at the middle of both stay paired.
    fn scroll_pane_to(&mut self, pane: usize, x: f32, y: f32) {
        let y = y.clamp(0., self.max_scroll_y(pane));
        let x = x.clamp(0., (self.max_cols[0].max(self.max_cols[1]) as f32 * CHAR_WIDTH - 40.).max(0.));
        self.pane_scroll[pane] = (x, y);
        if self.sync_scroll {
            let other = 1 - pane;
            let half = self.view_height.get() / 2.;
            let row = (y + half) / LINE_HEIGHT;
            let mapped = map_row(&self.two.segments, pane == 0, row);
            let other_y = (mapped * LINE_HEIGHT - half).clamp(0., self.max_scroll_y(other));
            self.pane_scroll[other] = (x, other_y);
        }
    }

    fn on_pane_wheel(&mut self, pane: usize, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(px(LINE_HEIGHT));
        let (mut dx, dy) = (f32::from(delta.x), f32::from(delta.y));
        if event.modifiers.shift && dx == 0. {
            dx = dy;
            let (x, y) = self.pane_scroll[pane];
            self.scroll_pane_to(pane, x - dx, y);
        } else {
            let (x, y) = self.pane_scroll[pane];
            self.scroll_pane_to(pane, x - dx, y - dy);
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Error stripe click or drag: centers the pane on that spot.
    fn stripe_seek(&mut self, pane: usize, y: Pixels, cx: &mut Context<Self>) {
        let bounds = self.stripe_bounds.get()[pane];
        let h = f32::from(bounds.size.height);
        if h <= 0. {
            return;
        }
        let t = ((f32::from(y - bounds.origin.y)) / h).clamp(0., 1.);
        let view = self.view_height.get();
        let total = (self.pane_rows(pane).len() as f32 * LINE_HEIGHT + view / 2.).max(h);
        let x = self.pane_scroll[pane].0;
        self.scroll_pane_to(pane, x, t * total - view / 2.);
        cx.notify();
    }

    fn set_all_included(&mut self, include: bool, cx: &mut Context<Self>) {
        let Some(path) = self.partial_path(cx) else { return };
        let signatures: Vec<u64> = (0..self.diff.hunks.len()).filter_map(|c| self.signature(c)).collect();
        crate::model::ExcludedHunks::update(cx, |map| {
            let set = map.entry(path).or_default();
            for s in signatures {
                if include {
                    set.remove(&s);
                } else {
                    set.insert(s);
                }
            }
        });
        cx.notify();
    }

    /// The working-tree line Jump to Source opens: the current change's,
    /// else the line at the top of the right pane.
    fn jump_target(&self) -> Option<crate::index::nav::Target> {
        let path = match self.source.as_ref()? {
            DiffSource::WorkingTree { path, .. } | DiffSource::Unstaged { path } => path.clone(),
            _ => return None,
        };
        let row = match self.current.and_then(|c| self.two.change_segment(c)) {
            Some(seg) => seg.right.start,
            None => (self.pane_scroll[1].1 / LINE_HEIGHT) as usize,
        };
        let line = self.two.right[row.min(self.two.right.len().saturating_sub(1))..]
            .iter()
            .find_map(|r| match r {
                PaneRow::Line { side, .. } => Some(side.line.saturating_sub(1)),
                PaneRow::Fold { .. } => None,
            })
            .unwrap_or(0);
        Some(crate::index::nav::Target { path, line: line as u32, col: 0, name: String::new(), label: String::new(), container: None })
    }

    pub fn jump_to_source(&mut self, cx: &mut Context<Self>) {
        if let Some(target) = self.jump_target() {
            cx.emit(crate::ui::navigate::OpenTarget(target));
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
            let differences = match self.diff.changes {
                0 => "No differences".to_owned(),
                1 => "1 difference".to_owned(),
                n => format!("{n} differences"),
            };
            let included = self.included(cx);
            if included.is_empty() || self.diff.changes == 0 {
                differences
            } else {
                format!("{differences}, {} included", included.iter().filter(|i| **i).count())
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
            .child(
                tool_button("diff-jump", IconName::Pencil, "Jump to Source (F4)")
                    .disabled(self.jump_target().is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.jump_to_source(cx))),
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
            .when(mode == ViewerMode::SideBySide, |el| {
                el.child(
                    tool_button("diff-sync", IconName::Link2, "Synchronize Scrolling")
                        .selected(self.sync_scroll)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.sync_scroll = !this.sync_scroll;
                            cx.notify();
                        })),
                )
            })
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

/// What a pane row needs to paint itself.
struct RowStyle {
    palette: Palette,
    highlight: HighlightMode,
    scroll_x: f32,
    /// The left gutter's button column.
    actions_width: f32,
    /// The right gutter's checkbox column.
    check_width: f32,
    review: Option<Rc<Review>>,
    review_path: Rc<str>,
}

fn line_color(kind: RowKind, p: &Palette) -> Hsla {
    match kind {
        RowKind::Modified => p.diff_modified,
        RowKind::Inserted => p.diff_inserted,
        RowKind::Deleted => p.diff_deleted,
        RowKind::Equal => gpui_kit::transparent_black(),
    }
}

fn word_color(kind: RowKind, p: &Palette) -> Hsla {
    match kind {
        RowKind::Inserted => p.diff_inserted_word,
        RowKind::Deleted => p.diff_deleted_word,
        _ => p.diff_modified_word,
    }
}

fn border_color(kind: RowKind, p: &Palette) -> Hsla {
    match kind {
        RowKind::Inserted => p.diff_inserted_border,
        RowKind::Deleted => p.diff_deleted_border,
        _ => p.diff_modified_border,
    }
}

/// A line's text, scrolled horizontally, with its changed words marked
/// in their fragment's color.
fn pane_text(side: &Side, words: bool, s: &RowStyle) -> impl IntoElement {
    let (text, ranges) = expand_tabs(&side.text, &side.changed);
    let highlights: Vec<_> = if words {
        ranges
            .into_iter()
            .enumerate()
            .filter(|(_, r)| !r.is_empty())
            .map(|(i, r)| {
                let kind = side.kinds.get(i).copied().unwrap_or(RowKind::Modified);
                (r, HighlightStyle { background_color: Some(word_color(kind, &s.palette)), ..Default::default() })
            })
            .collect()
    } else {
        Vec::new()
    };
    div().flex_1().min_w_0().h_full().overflow_hidden().child(
        div()
            .relative()
            .left(px(-s.scroll_x))
            .pl_2()
            .whitespace_nowrap()
            .text_color(s.palette.text)
            .child(StyledText::new(text).with_highlights(highlights)),
    )
}

/// A line number; on a review diff's new side it takes comments.
fn pane_number(pane: usize, ix: usize, line: usize, s: &RowStyle, cx: &mut Context<DiffView>) -> AnyElement {
    let p = &s.palette;
    let number = div()
        .w(px(GUTTER_WIDTH))
        .h_full()
        .flex_shrink_0()
        .text_right()
        .map(|el| if pane == 0 { el.pr_2() } else { el.pr_1() })
        .text_color(p.text_disabled);
    let Some(review) = s.review.as_ref().filter(|_| pane == 1) else {
        return number.child(line.to_string()).into_any_element();
    };
    let notes = review.comments.get(&line).cloned();
    let path = s.review_path.clone();
    h_flex()
        .id(("diff-comment", ix))
        .w(px(GUTTER_WIDTH))
        .h_full()
        .flex_shrink_0()
        .justify_end()
        .items_center()
        .gap_0p5()
        .pr_1()
        .cursor_pointer()
        .text_color(p.text_disabled)
        .hover(|st| st.bg(p.hover).text_color(p.text))
        .when(notes.is_some(), |el| el.child(common::icon(IconName::MessageSquare).text_color(p.link)))
        .child(line.to_string())
        .tooltip(move |window, cx| {
            let text = match &notes {
                Some(notes) => notes.join("\n\n"),
                None => "Add review comment".to_owned(),
            };
            gpui_kit::component::tooltip::Tooltip::new(text).build(window, cx)
        })
        .on_click(cx.listener(move |_, _, _, cx| cx.emit(CommentLine { path: path.to_string(), line })))
        .into_any_element()
}

/// One row of a pane, placed `top` pixels below the pane's top. The left
/// pane is mirrored: text, then its gutter against the divider.
fn pane_row(pane: usize, ix: usize, row: &PaneRow, top: f32, s: &RowStyle, cx: &mut Context<DiffView>) -> AnyElement {
    let p = &s.palette;
    let base = div().absolute().left_0().right_0().top(px(top)).h(px(LINE_HEIGHT));
    match row {
        PaneRow::Fold { id, count } => {
            let id = *id;
            base.id(("pane-fold", pane * 10_000_000 + ix))
                .flex()
                .items_center()
                .gap_1()
                .bg(p.diff_header)
                .text_color(p.text_secondary)
                .cursor_pointer()
                .hover(|st| st.text_color(p.text))
                .on_click(cx.listener(move |this, _, _, cx| this.expand_fold(id, cx)))
                .map(|el| if pane == 0 { el.pl_2() } else { el.pl(px(GUTTER_WIDTH + s.check_width + 4.)) })
                .child(common::icon(IconName::ChevronRight))
                .child(format!("{count} unchanged lines"))
                .into_any_element()
        }
        PaneRow::Line { side, kind, .. } => {
            let background = match (s.highlight, kind) {
                (HighlightMode::None, _) | (_, None) => None,
                (HighlightMode::Words, Some(k)) => Some(side.whole.map_or(line_color(*k, p), |w| word_color(w, p))),
                (_, Some(k)) => Some(line_color(*k, p)),
            };
            let words = s.highlight == HighlightMode::Words && side.whole.is_none();
            // With highlighting off IntelliJ still marks changed lines in the gutter.
            let marker = div()
                .w(px(3.))
                .h_full()
                .flex_shrink_0()
                .when_some(kind.filter(|_| s.highlight == HighlightMode::None), |el, k| el.bg(border_color(k, p)));
            let text = pane_text(side, words, s);
            let number = pane_number(pane, ix, side.line, s, cx);
            base.flex()
                .when_some(background, |el, bg| el.bg(bg))
                .map(|el| {
                    if pane == 0 {
                        el.child(text).child(div().w(px(s.actions_width)).flex_shrink_0()).child(number).child(marker)
                    } else {
                        el.child(marker).child(number).child(div().w(px(s.check_width)).flex_shrink_0()).child(text)
                    }
                })
                .into_any_element()
        }
    }
}

impl DiffView {
    /// Per change: whether it goes into the next commit (partial commits).
    fn included(&self, cx: &App) -> Vec<bool> {
        match self.partial_path(cx) {
            Some(path) => {
                let excluded = crate::model::ExcludedHunks::get(cx).get(&path).cloned().unwrap_or_default();
                (0..self.diff.hunks.len()).map(|c| self.signature(c).is_none_or(|s| !excluded.contains(&s))).collect()
            }
            None => Vec::new(),
        }
    }

    /// Error stripe: every change's place in the whole file, the visible
    /// part as a thumb; click or drag to scroll there.
    fn render_stripe(&self, pane: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let content = self.pane_rows(pane).len() as f32 * LINE_HEIGHT;
        let view = self.view_height.get();
        let scroll = self.pane_scroll[pane].1;
        let marks: Vec<(f32, f32, Hsla)> = self
            .two
            .segments
            .iter()
            .filter(|s| s.change.is_some())
            .map(|s| {
                let range = if pane == 0 { s.left.clone() } else { s.right.clone() };
                (range.start as f32 * LINE_HEIGHT, range.len() as f32 * LINE_HEIGHT, border_color(s.kind, &palette))
            })
            .collect();
        let cell = self.stripe_bounds.clone();
        let thumb = palette.text_disabled.opacity(0.25);
        div()
            .id(("diff-stripe", pane))
            .w(px(STRIPE_WIDTH))
            .h_full()
            .flex_shrink_0()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    this.stripe_drag = Some(pane);
                    this.stripe_seek(pane, e.position.y, cx);
                }),
            )
            .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _, cx| {
                if this.stripe_drag == Some(pane) && e.pressed_button == Some(MouseButton::Left) {
                    this.stripe_seek(pane, e.position.y, cx);
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.stripe_drag = None))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, _| this.stripe_drag = None))
            .child(
                canvas(
                    move |bounds, _, _| {
                        let mut all = cell.get();
                        all[pane] = bounds;
                        cell.set(all);
                    },
                    move |bounds, _, window, _| {
                        let h = f32::from(bounds.size.height);
                        let total = (content + view / 2.).max(h).max(1.);
                        let scale = h / total;
                        let x = bounds.origin.x + px(2.);
                        let w = bounds.size.width - px(4.);
                        for (y, len, color) in &marks {
                            let top = bounds.origin.y + px(y * scale);
                            let height = px((len * scale).max(2.));
                            window.paint_quad(fill(Bounds::new(point(x, top), size(w, height)), *color));
                        }
                        if view > 0. && view < total {
                            let top = bounds.origin.y + px(scroll * scale);
                            window.paint_quad(fill(Bounds::new(point(bounds.origin.x, top), size(bounds.size.width, px(view * scale))), thumb));
                        }
                    },
                )
                .size_full(),
            )
    }

    /// IntelliJ's side-by-side viewer: two panes with only their own lines,
    /// gutters against the divider that connects their change blocks.
    fn render_two_side(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = cx.palette().clone();
        let mono = cx.theme().mono_font_family.clone();
        let actions = self.hunk_actions();
        let partial = self.partial_path(cx).is_some();
        let included = self.included(cx);
        let two = self.two.clone();
        let height = self.view_height.get();
        let actions_width = if actions.is_empty() { 0. } else { BUTTON_WIDTH * actions.len() as f32 + 2. };
        let check_width = if partial { BUTTON_WIDTH + 2. } else { 0. };

        let mut panes = Vec::new();
        for pane in 0..2 {
            let rows = if pane == 0 { &two.left } else { &two.right };
            let (scroll_x, scroll_y) = self.pane_scroll[pane];
            let style = RowStyle {
                palette: palette.clone(),
                highlight: self.options.highlight,
                scroll_x,
                actions_width,
                check_width,
                review: self.review.clone(),
                review_path: self.source.as_ref().map(|s| s.path()).unwrap_or_default().into(),
            };
            // Until measured, paint as much as a tall window shows.
            let visible = if height > 0. { height } else { 1600. };
            let first = ((scroll_y / LINE_HEIGHT).floor() as usize).min(rows.len());
            let last = (((scroll_y + visible) / LINE_HEIGHT).ceil() as usize + 1).min(rows.len());
            let mut children: Vec<AnyElement> = Vec::new();
            if pane == 0 {
                let measured = self.view_height.clone();
                children.push(
                    canvas(
                        move |bounds, window, _| {
                            let h = f32::from(bounds.size.height);
                            if (measured.get() - h).abs() > 0.5 {
                                measured.set(h);
                                window.refresh();
                            }
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full()
                    .into_any_element(),
                );
            }
            children.extend((first..last).map(|ix| pane_row(pane, ix, &rows[ix], ix as f32 * LINE_HEIGHT - scroll_y, &style, cx)));

            // Per change: the insertion line on an empty side, and the gutter buttons.
            for seg in two.segments.iter() {
                let Some(change) = seg.change else { continue };
                let range = if pane == 0 { seg.left.clone() } else { seg.right.clone() };
                let y = range.start as f32 * LINE_HEIGHT - scroll_y;
                if y + range.len() as f32 * LINE_HEIGHT < -LINE_HEIGHT || y > visible + LINE_HEIGHT {
                    continue;
                }
                let color = border_color(seg.kind, &palette);
                if range.is_empty() {
                    children.push(div().absolute().left_0().right_0().top(px(y)).h(px(1.)).bg(color).into_any_element());
                }
                let button_top = if range.is_empty() { y - LINE_HEIGHT / 2. } else { y };
                if pane == 0 && !actions.is_empty() {
                    let mut el = h_flex()
                        .absolute()
                        .top(px(button_top))
                        .right(px(GUTTER_WIDTH + 3.))
                        .w(px(actions_width))
                        .h(px(LINE_HEIGHT))
                        .justify_center()
                        .items_center();
                    for (action, icon, tooltip) in actions.iter().copied() {
                        let icon = if action == HunkAction::Revert { IconName::ChevronsRight } else { icon };
                        let tooltip = if action == HunkAction::Revert { "Revert" } else { tooltip };
                        el = el.child(
                            tool_button(gpui_kit::ElementId::NamedInteger(format!("pane-{tooltip}").into(), change as u64), icon, tooltip)
                                .on_click(cx.listener(move |this, _, _, cx| this.apply_hunk(change, action, cx))),
                        );
                    }
                    children.push(el.into_any_element());
                }
                if pane == 1 && partial {
                    let checked = included.get(change).copied().unwrap_or(true);
                    children.push(
                        h_flex()
                            .absolute()
                            .top(px(button_top))
                            .left(px(GUTTER_WIDTH + 3.))
                            .w(px(check_width))
                            .h(px(LINE_HEIGHT))
                            .justify_center()
                            .items_center()
                            .child(
                                Checkbox::new(gpui_kit::ElementId::NamedInteger("hunk-include".into(), change as u64))
                                    .checked(checked)
                                    .tooltip("Include into commit")
                                    .on_change(cx.listener(move |this, value: &bool, _, cx| this.toggle_hunk(change, *value, cx))),
                            )
                            .into_any_element(),
                    );
                }
            }
            panes.push(
                div()
                    .id(("diff-pane", pane))
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_hidden()
                    .font_family(mono.clone())
                    .text_size(px(12.5))
                    .on_scroll_wheel(cx.listener(move |this, e: &ScrollWheelEvent, _, cx| this.on_pane_wheel(pane, e, cx)))
                    .children(children),
            );
        }

        let connectors: Vec<Connector> = two
            .segments
            .iter()
            .filter(|s| s.change.is_some())
            .map(|s| Connector {
                left: s.left.clone(),
                right: s.right.clone(),
                colors: BlockColors { fill: line_color(s.kind, &palette), border: border_color(s.kind, &palette) },
            })
            .collect();
        let scroll = (self.pane_scroll[0].1, self.pane_scroll[1].1);
        let folds = fold_links(&two);
        let fold_color = palette.border;
        let divider = div().w(px(DIVIDER_WIDTH)).h_full().flex_shrink_0().child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    window.with_content_mask(Some(gpui_kit::ContentMask { bounds }), |window| {
                        paint_divider(bounds, scroll, &connectors, &folds, fold_color, window)
                    })
                },
            )
            .size_full(),
        );

        let mut panes = panes.into_iter();
        let (left, right) = (panes.next().unwrap(), panes.next().unwrap());
        h_flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(self.render_stripe(0, cx))
            .child(left)
            .child(divider)
            .child(right)
            .child(self.render_stripe(1, cx))
            .into_any_element()
    }

    /// The panes' title bars: read-only lock, revision and path on the left;
    /// the include-all checkbox and revision on the right.
    fn render_two_side_header(&self, old_title: String, new_title: String, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let path = self.source.as_ref().map(|s| s.path().to_owned()).unwrap_or_default();
        let included = self.included(cx);
        let partial = self.partial_path(cx).is_some() && !included.is_empty();
        let all = included.iter().all(|i| *i);
        h_flex()
            .h(px(26.))
            .flex_shrink_0()
            .text_xs()
            .border_b_1()
            .border_color(palette.border)
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .pl(px(STRIPE_WIDTH + 6.))
                    .gap_1p5()
                    .child(common::icon(IconName::Lock).text_color(palette.text_secondary))
                    .child(div().flex_shrink_0().child(old_title))
                    .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(palette.text_secondary).child(path)),
            )
            .child(div().w(px(DIVIDER_WIDTH)).flex_shrink_0())
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .pl(px(6.))
                    .gap_1p5()
                    .when(partial, |el| {
                        el.child(
                            Checkbox::new("hunk-include-all")
                                .checked(all)
                                .tooltip("Include all changes into commit")
                                .on_change(cx.listener(|this, value: &bool, _, cx| this.set_all_included(*value, cx))),
                        )
                    })
                    .child(new_title),
            )
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
        if self.view_height.get() > 0. {
            if let Some(change) = self.pending_change.take() {
                self.go_to_change(change);
            }
        }
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
        let review = self.review.clone();
        let review_path: Rc<str> = source.path().into();
        let two_side = mode == ViewerMode::SideBySide && !self.diff.binary;

        let banner = match &self.loaded {
            Some(loaded) if self.diff.binary => Some(match &loaded.binary {
                Some([Some(_), Some(_)]) if loaded.old == loaded.new => "Contents are identical".to_owned(),
                Some([None, Some(_)]) => "File was added".to_owned(),
                Some([Some(_), None]) => "File was deleted".to_owned(),
                _ => "Binary files differ".to_owned(),
            }),
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
                // A review diff's new-side line number: click to comment, marked when commented.
                let new_gutter = |ix: usize, line: Option<usize>, cx: &mut Context<DiffView>| -> AnyElement {
                    let (Some(review), Some(line)) = (review.as_ref(), line) else {
                        return gutter(line, &palette).into_any_element();
                    };
                    let notes = review.comments.get(&line).cloned();
                    let path = review_path.clone();
                    h_flex()
                        .id(("diff-comment", ix))
                        .w(px(GUTTER_WIDTH))
                        .h_full()
                        .flex_shrink_0()
                        .justify_end()
                        .items_center()
                        .gap_0p5()
                        .pr_2()
                        .cursor_pointer()
                        .text_color(palette.text_disabled)
                        .hover(|s| s.bg(palette.hover).text_color(palette.text))
                        .when(notes.is_some(), |el| el.child(common::icon(IconName::MessageSquare).text_color(palette.link)))
                        .child(line.to_string())
                        .tooltip(move |window, cx| {
                            let text = match &notes {
                                Some(notes) => notes.join("\n\n"),
                                None => "Add review comment".to_owned(),
                            };
                            gpui_kit::component::tooltip::Tooltip::new(text).build(window, cx)
                        })
                        .on_click(cx.listener(move |_, _, _, cx| cx.emit(CommentLine { path: path.to_string(), line })))
                        .into_any_element()
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
                                    .child(new_gutter(ix, right.as_ref().map(|s| s.line), cx))
                                    .when_some(side, |el, side| {
                                        el.child(line_text(side, word.unwrap_or(palette.diff_header), fg))
                                    })
                                    .into_any_element()
                            } else {
                                let cell = |side: Option<&Side>, is_left: bool, cx: &mut Context<DiffView>| {
                                    let (bg, fg, word) = paint(is_left);
                                    let bg = if side.is_some() { bg } else { None };
                                    h_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .h_full()
                                        .overflow_hidden()
                                        .when_some(bg, |el, bg| el.bg(bg))
                                        .child(marker(word.filter(|_| side.is_some())))
                                        .map(|el| if is_left {
                                            el.child(gutter(side.map(|s| s.line), &palette))
                                        } else {
                                            el.child(new_gutter(ix, side.map(|s| s.line), cx))
                                        })
                                        .when_some(side, |el, side| {
                                            el.child(line_text(side, word.unwrap_or(palette.diff_header), fg))
                                        })
                                };
                                h_flex()
                                    .id(ix)
                                    .h(px(LINE_HEIGHT))
                                    .w_full()
                                    .child(cell(left.as_ref(), true, cx))
                                    .child(hunk_buttons(ix, *change, cx))
                                    .child(cell(right.as_ref(), false, cx))
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
            .when(two_side, |el| {
                el.when_some(titles, |el, (old_title, new_title)| el.child(self.render_two_side_header(old_title, new_title, cx)))
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
            .map(|el| match self.loaded.as_ref().and_then(|l| l.binary.as_ref()) {
                Some(panes) => el.child(render_binary(panes, &palette)),
                None if two_side => el.child(self.render_two_side(cx)),
                None => el.child(list),
            })
            .into_any_element()
    }
}

/// IntelliJ's binary diff: both sides next to each other, images on a
/// checkerboard at their real size (scaled down to fit), each with its
/// dimensions, format and file size underneath.
fn render_binary(panes: &[Option<BinaryPane>; 2], palette: &Palette) -> impl IntoElement {
    let pane = |pane: &Option<BinaryPane>| {
        let body = match pane {
            None => div().text_color(palette.text_secondary).child("No file").into_any_element(),
            Some(BinaryPane { image: Some((image, _)), .. }) => div()
                .p_2()
                .max_w_full()
                .max_h_full()
                .flex()
                .border_1()
                .border_color(palette.border)
                // The transparency backdrop.
                .bg(gpui_kit::hsla(0., 0., 0.5, 0.15))
                .child(img(image.clone()).object_fit(ObjectFit::ScaleDown).max_w_full().max_h_full())
                .into_any_element(),
            Some(_) => div().text_color(palette.text_secondary).child("Binary content").into_any_element(),
        };
        v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .child(div().flex_1().min_h_0().p_4().flex().items_center().justify_center().overflow_hidden().child(body))
            .child(
                div()
                    .h(px(24.))
                    .px_3()
                    .flex()
                    .items_center()
                    .text_xs()
                    .text_color(palette.text_secondary)
                    .border_t_1()
                    .border_color(palette.border)
                    .children(pane.as_ref().map(BinaryPane::describe)),
            )
    };
    h_flex()
        .flex_1()
        .w_full()
        .min_h_0()
        .child(pane(&panes[0]))
        .child(div().w(px(1.)).h_full().bg(palette.border))
        .child(pane(&panes[1]))
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
