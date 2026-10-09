//! The editor area's diff viewer, after IntelliJ's: side-by-side and unified
//! viewers, word / line highlighting, whitespace options, collapsed unchanged
//! fragments, and Previous / Next Difference (Shift+F7 / F7).

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::{
    Disableable as _, Selectable as _, Sizable as _, h_flex,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::assets::IconName;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, MouseButton, canvas, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _,
    Image, ImageFormat, ObjectFit, StyledImage as _, Task, img, Window, actions, div, prelude::FluentBuilder as _, px,
};

use crate::git::Repository;
use crate::git::blob::{self, ImageInfo, ImageKind};
use crate::git::diff::{self, DiffOptions, DiffRow, FileDiff, HighlightMode, HunkAction, IgnoreWhitespace, Revisions, RowKind, Side};
use crate::theme::{ActivePalette as _, Palette};
use crate::ui::common::{self, tool_button};
use crate::ui::diff_panes::{BlockColors, Connector, DIVIDER_WIDTH, LINE_HEIGHT, PaneRow, TwoSide, fold_links, paint_divider};
use crate::ui::text_panes::{BUTTON_WIDTH, GUTTER_WIDTH, PaneContent, PaneLayout, RowLook, RowTarget, STRIPE_WIDTH, TextPanes, pane_area};

pub(crate) mod edit;
mod files;
mod menu;

actions!(diff_view, [NextDifference, PreviousDifference, JumpToSource, CompareNextFile, ComparePreviousFile]);

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
    /// Compare With…: a file against another file on disk.
    Files { path: String, other: std::path::PathBuf },
    /// Two texts in memory (the merge tool's Compare … with Base), read-only.
    Texts { path: String, old: String, new: String, old_title: String, new_title: String },
    /// Compare with Clipboard: the clipboard on the left, the file (editable)
    /// on the right.
    Clipboard { path: String, text: String },
}

impl DiffSource {
    fn path(&self) -> &str {
        match self {
            DiffSource::Commit { path, .. }
            | DiffSource::WorkingTree { path, .. }
            | DiffSource::Staged { path }
            | DiffSource::Unstaged { path }
            | DiffSource::Between { path, .. }
            | DiffSource::Files { path, .. }
            | DiffSource::Texts { path, .. }
            | DiffSource::Clipboard { path, .. } => path,
        }
    }

    fn revisions(&self) -> Revisions {
        match self.clone() {
            DiffSource::Commit { hash, path, old_path } => Revisions::Commit { hash, path, old_path },
            DiffSource::WorkingTree { path, .. } => Revisions::WorkingTree { path },
            DiffSource::Staged { path } => Revisions::Staged { path },
            DiffSource::Unstaged { path } => Revisions::Unstaged { path },
            DiffSource::Between { old, new, path, old_path } => Revisions::Between { old, new, path, old_path },
            DiffSource::Files { path, other } => Revisions::Files { path, other },
            DiffSource::Texts { path, old, new, old_title, new_title } => Revisions::Texts { path, old, new, old_title, new_title },
            DiffSource::Clipboard { path, text } => Revisions::Clipboard { path, text },
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
    /// The rows were built with Align Changes on.
    aligned: bool,
    mode: ViewerMode,
    expanded: HashSet<usize>,
    rows: Rc<Vec<Display>>,
    current: Option<usize>,
    error: Option<String>,
    review: Option<Rc<Review>>,
    /// The side-by-side viewer's two panes.
    two: Rc<TwoSide>,
    /// The side-by-side viewer's text panes; the right one edits a
    /// working-tree file in place.
    panes: TextPanes,
    save_task: Option<Task<()>>,
    _task: Option<Task<()>>,
    /// The files of the shown change set, for Compare Previous / Next File.
    files: Vec<DiffSource>,
    files_key: String,
    _files_task: Option<Task<()>>,
    /// F7 stopped at the end of the file.
    edge: Option<files::Edge>,
    /// The file being loaded opens at its last change (Shift+F7 into it).
    arrive_at_end: bool,
}

impl DiffView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            repository: None,
            source: None,
            loaded: None,
            diff: FileDiff::default(),
            options: DiffOptions::default(),
            aligned: false,
            mode: ViewerMode::SideBySide,
            expanded: HashSet::new(),
            rows: Rc::new(Vec::new()),
            current: None,
            error: None,
            review: None,
            two: Rc::default(),
            panes: TextPanes::new(2, cx),
            save_task: None,
            _task: None,
            files: Vec::new(),
            files_key: String::new(),
            _files_task: None,
            edge: None,
            arrive_at_end: false,
        }
    }

    /// The editor tab's title: "name (Changes)" for the working tree, "name (abc12345)" for a commit.
    pub fn title(&self) -> Option<String> {
        let source = self.source.as_ref()?;
        let path = source.path();
        let name = path.rsplit('/').next().unwrap_or(path);
        Some(match source {
            DiffSource::Commit { hash, .. } => format!("{name} ({})", &hash[..hash.len().min(8)]),
            DiffSource::Files { other, .. } => {
                format!("{name} vs {}", other.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
            }
            DiffSource::Between { .. } => format!("{name} (Compare)"),
            DiffSource::Texts { old_title, new_title, .. } => format!("{name} ({old_title} vs {new_title})"),
            DiffSource::Clipboard { .. } => format!("Clipboard vs {name}"),
            _ => format!("{name} (Changes)"),
        })
    }

    /// Closing the diff's editor tab.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.flush_save();
        self.source = None;
        self.loaded = None;
        self.diff = FileDiff::default();
        self.rows = Rc::new(Vec::new());
        self.review = None;
        self._task = None;
        cx.notify();
    }

    pub fn show(&mut self, repository: Repository, source: DiffSource, cx: &mut Context<Self>) {
        if self.source.as_ref() == Some(&source) {
            return;
        }
        self.flush_save();
        self.repository = Some(repository.clone());
        self.source = Some(source.clone());
        self.edge = None;
        self.load_files(&repository, &source, cx);
        self.panes.set_texts(vec![String::new(), String::new()], "plaintext");
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
                        let language = crate::ui::file_editor::language_for(this.source.as_ref().map(|s| s.path()).unwrap_or_default());
                        this.panes.set_texts(vec![loaded.old.clone(), loaded.new.clone()], language);
                        this.loaded = Some(loaded);
                        this.panes.editable = this.edit_pane();
                        this.set_diff(file_diff);
                        let last = this.diff.changes.saturating_sub(1);
                        let first = if std::mem::take(&mut this.arrive_at_end) { last } else { 0 };
                        this.go_to_change(first);
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
            // Two files, the clipboard and a file, or a revision and the
            // local file: copy a change across into the editable side.
            Some(DiffSource::Files { .. } | DiffSource::Clipboard { .. } | DiffSource::Between { new: None, .. }) if self.editable() => {
                vec![(HunkAction::Revert, IconName::ChevronsRight, "Replace")]
            }
            _ => Vec::new(),
        }
    }

    fn apply_hunk(&mut self, change: usize, action: HunkAction, window: &mut Window, cx: &mut Context<Self>) {
        if action == HunkAction::Revert && self.editable() {
            let append = self.panes.ctrl_held;
            self.revert_change(change, append, window, cx);
            return;
        }
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
                    && self.options.highlight != HighlightMode::Split
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
        let hunk = self.diff.hunks[change].clone();
        crate::model::ExcludedHunks::update(cx, |map| {
            let set = map.entry(path).or_default();
            for k in 0..hunk.old.len() {
                set.remove(&diff::line_id(signature, false, k));
            }
            for k in 0..hunk.new.len() {
                set.remove(&diff::line_id(signature, true, k));
            }
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

    /// Follows the gear menu: context lines and Align Changes.
    fn take_settings(&mut self, cx: &App) {
        let settings = &crate::settings::Settings::get(cx).diff;
        let context = self.options.context.map(|_| settings.context_lines);
        if context != self.options.context {
            self.options.context = context;
            if let Some(loaded) = &self.loaded {
                let file_diff = diff::compute(&loaded.old, &loaded.new, self.options);
                self.set_diff(file_diff);
            }
        }
        if settings.align_changes != self.aligned {
            self.aligned = settings.align_changes;
            self.rebuild_rows();
        }
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
        let mut two = TwoSide::build(&self.diff.rows, &self.expanded);
        if self.aligned {
            two.align();
        }
        // The empty line after a final line break is a line of its own, as in an editor.
        let mut added = [false; 2];
        for pane in 0..2 {
            let buffer = &self.panes.buffers[pane];
            let lines = buffer.text().lines().count();
            if buffer.line_count() > lines && self.loaded.is_some() {
                let side = Side { line: lines + 1, text: String::new(), changed: Vec::new(), kinds: Vec::new(), whole: None };
                let row = PaneRow::Line { side, kind: None, change: None, first: false };
                if pane == 0 { two.left.push(row) } else { two.right.push(row) }
                added[pane] = true;
            }
        }
        if added == [true, true] {
            let (l, r) = (two.left.len() - 1, two.right.len() - 1);
            two.segments.push(crate::ui::diff_panes::Segment { left: l..l + 1, right: r..r + 1, change: None, kind: RowKind::Equal });
        }
        let targets = |rows: &[PaneRow]| -> Vec<RowTarget> {
            rows.iter()
                .map(|row| match row {
                    PaneRow::Line { side, .. } => RowTarget::Line(side.line - 1),
                    PaneRow::Fold { id, count } => RowTarget::Fold { id: *id, count: *count },
                    PaneRow::Filler => RowTarget::Filler,
                })
                .collect()
        };
        self.panes.set_rows(0, targets(&two.left));
        if self.mode == ViewerMode::Unified {
            // One pane: the new text, each block's deleted lines shown above it.
            let mut rows: Vec<RowTarget> = self
                .rows
                .iter()
                .map(|row| match row {
                    Display::Fold { id, count } => RowTarget::Fold { id: *id, count: *count },
                    Display::Line { right: Some(side), .. } => RowTarget::Line(side.line - 1),
                    Display::Line { left: Some(side), .. } => RowTarget::Other { pane: 0, line: side.line - 1 },
                    Display::Line { .. } => RowTarget::Filler,
                })
                .collect();
            if added[1] {
                rows.push(RowTarget::Line(self.panes.buffers[1].line_count() - 1));
            }
            self.panes.set_rows(1, rows);
            self.panes.links = Vec::new();
        } else {
            self.panes.set_rows(1, targets(&two.right));
            self.panes.links = vec![(0, 1, two.segments.clone())];
        }
        self.two = Rc::new(two);
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

    fn change_row(&self, change: usize) -> Option<usize> {
        self.rows.iter().position(|row| matches!(row, Display::Line { change: Some(c), .. } if *c == change))
    }

    fn go_to_change(&mut self, change: usize) {
        if self.mode == ViewerMode::SideBySide {
            let Some(seg) = self.two.change_segment(change).cloned() else { return };
            self.current = Some(change);
            self.panes.show_rows(vec![(0, seg.left.start), (1, seg.right.start)]);
            return;
        }
        if let Some(row) = self.change_row(change) {
            self.current = Some(change);
            self.panes.show_rows(vec![(1, row)]);
        }
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
        if self.mode == ViewerMode::Unified {
            let top = self.panes.row_at(1, self.panes.scroll[1].1);
            let row = self.current.and_then(|c| self.change_row(c)).unwrap_or(top);
            let line = self.rows[row.min(self.rows.len().saturating_sub(1))..]
                .iter()
                .find_map(|r| match r {
                    Display::Line { right: Some(side), .. } => Some(side.line.saturating_sub(1)),
                    _ => None,
                })
                .unwrap_or(0);
            return Some(crate::index::nav::Target { path, line: line as u32, col: 0, name: String::new(), label: String::new(), container: None });
        }
        let row = match self.current.and_then(|c| self.two.change_segment(c)) {
            Some(seg) => seg.right.start,
            None => self.panes.row_at(1, self.panes.scroll[1].1),
        };
        let line = self.two.right[row.min(self.two.right.len().saturating_sub(1))..]
            .iter()
            .find_map(|r| match r {
                PaneRow::Line { side, .. } => Some(side.line.saturating_sub(1)),
                PaneRow::Fold { .. } | PaneRow::Filler => None,
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
            self.edge = None;
            self.go_to_change(self.current.map_or(0, |c| c + 1));
            cx.notify();
        } else {
            self.past_end(files::Edge::Next, cx);
        }
    }

    pub fn previous_difference(&mut self, cx: &mut Context<Self>) {
        if let Some(c) = self.current.filter(|c| *c > 0) {
            self.edge = None;
            self.go_to_change(c - 1);
            cx.notify();
        } else {
            self.past_end(files::Edge::Previous, cx);
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
            IgnoreWhitespace::AllAndEmptyLines => "Ignore whitespaces and empty lines",
        };
        let highlight_label = match options.highlight {
            HighlightMode::Words => "Highlight words",
            HighlightMode::Lines => "Highlight lines",
            HighlightMode::Split => "Highlight split changes",
            HighlightMode::Characters => "Highlight characters",
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
            .h(px(crate::ui::common::header_height()))
            .px_2()
            .gap_0p5()
            .border_b_1()
            .border_color(palette.border)
            .bg(palette.toolbar)
            .child(
                tool_button("diff-prev", IconName::ChevronUp, "Previous Difference (Shift+F7)")
                    .disabled(!self.has_previous() && self.neighbor(false).is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.previous_difference(cx))),
            )
            .child(
                tool_button("diff-next", IconName::ChevronDown, "Next Difference (F7)")
                    .disabled(!self.has_next() && self.neighbor(true).is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.next_difference(cx))),
            )
            .child(
                tool_button("diff-jump", IconName::Pencil, "Jump to Source (F4)")
                    .disabled(self.jump_target().is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.jump_to_source(cx))),
            )
            .child(separator())
            .child(
                tool_button("diff-prev-file", IconName::ArrowLeft, "Compare Previous File (Alt+Left)")
                    .disabled(self.neighbor(false).is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.compare_previous_file(cx))),
            )
            .child(
                tool_button("diff-next-file", IconName::ArrowRight, "Compare Next File (Alt+Right)")
                    .disabled(self.neighbor(true).is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.compare_next_file(cx))),
            )
            .when_some(self.file_position().filter(|(_, n)| *n > 1), |el, (ix, n)| {
                let files = self.files.clone();
                let entity = entity.clone();
                el.child(Button::new("diff-files").ghost().xsmall().icon(IconName::List).label(format!("{} of {n}", ix + 1)).tooltip("Go to Changed File").dropdown_menu(
                    move |mut menu, _, _| {
                        for (i, file) in files.iter().enumerate() {
                            let (entity, file) = (entity.clone(), file.clone());
                            menu = menu.item(
                                PopupMenuItem::new(file.path().to_owned())
                                    .checked(i == ix)
                                    .on_click(move |_, _, cx| entity.update(cx, |this, cx| this.open_file(file.clone(), false, cx))),
                            );
                        }
                        menu
                    },
                ))
            })
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
                        ("Ignore whitespaces and empty lines", IgnoreWhitespace::AllAndEmptyLines),
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
                        ("Highlight split changes", HighlightMode::Split),
                        ("Highlight characters", HighlightMode::Characters),
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
                        let lines = crate::settings::Settings::get(cx).diff.context_lines;
                        this.update_options(cx, |o| o.context = if o.context.is_some() { None } else { Some(lines) })
                    })),
            )
            .when(mode == ViewerMode::SideBySide, |el| {
                el.child(
                    tool_button("diff-sync", IconName::Link2, "Synchronize Scrolling")
                        .selected(self.panes.sync)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.panes.sync = !this.panes.sync;
                            cx.notify();
                        })),
                )
            })
            .child(Button::new("diff-gear").ghost().xsmall().icon(IconName::Settings).tooltip("Settings").dropdown_menu(
                move |menu, window, cx| crate::ui::text_panes::gear_menu(menu, mode == ViewerMode::SideBySide, window, cx),
            ))
            .child(
                tool_button("diff-help", IconName::CircleQuestionMark, "Help")
                    .on_click(|_, _, cx| cx.open_url("https://www.jetbrains.com/help/idea/differences-viewer.html")),
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
            .when_some(self.edge, |el, edge| {
                el.child(div().text_xs().mr_2().text_color(palette.link).child(match edge {
                    files::Edge::Next => "Press F7 again to go to the next file",
                    files::Edge::Previous => "Press Shift+F7 again to go to the previous file",
                }))
            })
            .child(div().text_xs().text_color(palette.text_secondary).child(summary))
    }
}

pub(crate) fn line_color(kind: RowKind, p: &Palette) -> Hsla {
    match kind {
        RowKind::Modified => p.diff_modified,
        RowKind::Inserted => p.diff_inserted,
        RowKind::Deleted => p.diff_deleted,
        RowKind::Equal => gpui_kit::transparent_black(),
    }
}

pub(crate) fn word_color(kind: RowKind, p: &Palette) -> Hsla {
    match kind {
        RowKind::Inserted => p.diff_inserted_word,
        RowKind::Deleted => p.diff_deleted_word,
        _ => p.diff_modified_word,
    }
}

pub(crate) fn border_color(kind: RowKind, p: &Palette) -> Hsla {
    match kind {
        RowKind::Inserted => p.diff_inserted_border,
        RowKind::Deleted => p.diff_deleted_border,
        _ => p.diff_modified_border,
    }
}

/// A review diff's new-side line number: click to comment; marked when commented.
fn review_number(review: &Review, path: Rc<str>, ix: usize, line: usize, palette: &Palette, cx: &mut Context<DiffView>) -> AnyElement {
    let notes = review.comments.get(&line).cloned();
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
        .text_color(palette.text_disabled)
        .hover(|st| st.bg(palette.hover).text_color(palette.text))
        .when(notes.is_some(), |el| el.child(common::icon(IconName::MessageSquare).text_color(palette.link)))
        .child(line.to_string())
        .tooltip(move |window, cx| {
            let text = match &notes {
                Some(notes) => notes.join("\n\n"),
                None => "Add review comment".to_owned(),
            };
            gpui_kit::component::tooltip::Tooltip::new(text).build(window, cx)
        })
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |_, _, _, cx| cx.emit(CommentLine { path: path.to_string(), line })))
        .into_any_element()
}

impl DiffView {
    /// Per change: whether it goes into the next commit (partial commits).
    fn included(&self, cx: &App) -> Vec<bool> {
        self.exclusions(cx).iter().map(|(old, new)| old.iter().chain(new).any(|out| !*out) || old.len() + new.len() == 0).collect()
    }

    /// How each visible row of a pane looks: its block's color (or the
    /// inner fragment's, for a line inserted inside a modified block), its
    /// changed words, and the gutter marker when highlighting is off.
    fn row_looks(&self, pane: usize, palette: &Palette, cx: &mut Context<Self>) -> Vec<RowLook> {
        let rows = if pane == 0 { &self.two.left } else { &self.two.right };
        let highlight = self.options.highlight;
        let review = self.review.clone().filter(|_| pane == 1);
        let path: Rc<str> = self.source.as_ref().map(|s| s.path()).unwrap_or_default().into();
        let exclusions = self.exclusions(cx);
        // A line left out of the commit is painted faint.
        let excluded = |change: Option<usize>, line: usize| -> bool {
            let Some((c, hunk)) = change.and_then(|c| Some((c, self.diff.hunks.get(c)?))) else { return false };
            let (range, lines) = if pane == 0 { (&hunk.old, exclusions.get(c).map(|e| &e.0)) } else { (&hunk.new, exclusions.get(c).map(|e| &e.1)) };
            lines.and_then(|l| l.get(line.wrapping_sub(range.start))).copied().unwrap_or(false)
        };
        self.panes
            .visible_rows(pane)
            .map(|ix| match &rows[ix] {
                PaneRow::Fold { .. } | PaneRow::Filler => RowLook::default(),
                PaneRow::Line { side, kind, change, .. } => {
                    let faint = if excluded(*change, side.line - 1) { 0.35 } else { 1. };
                    let background = match (highlight, kind) {
                        (HighlightMode::None, _) | (_, None) => None,
                        (h, Some(k)) if h.inner() => Some(side.whole.map_or(line_color(*k, palette), |w| word_color(w, palette))),
                        (_, Some(k)) => Some(line_color(*k, palette)),
                    };
                    let words = if highlight.inner() && side.whole.is_none() {
                        side.changed
                            .iter()
                            .enumerate()
                            .filter(|(_, r)| !r.is_empty())
                            .map(|(i, r)| (r.clone(), word_color(side.kinds.get(i).copied().unwrap_or(RowKind::Modified), palette)))
                            .collect()
                    } else {
                        Vec::new()
                    };
                    let marker = kind.filter(|_| highlight == HighlightMode::None).map(|k| border_color(k, palette));
                    let number = review.as_ref().map(|r| review_number(r, path.clone(), ix, side.line, palette, cx));
                    let background = background.map(|c| c.opacity(faint));
                    let words = words.into_iter().map(|(r, c)| (r, c.opacity(faint))).collect();
                    // The block's color and edges run across the gutter to the divider.
                    let block = kind.filter(|_| highlight != HighlightMode::None);
                    let gutter = block.map(|k| line_color(k, palette).opacity(faint));
                    let same = |r: Option<&PaneRow>| matches!(r, Some(PaneRow::Line { change: c, .. }) if c == change);
                    let top = block.filter(|_| ix == 0 || !same(rows.get(ix - 1))).map(|k| border_color(k, palette));
                    let bottom = block.filter(|_| !same(rows.get(ix + 1))).map(|k| border_color(k, palette));
                    RowLook { background, words, marker, number, gutter, top, bottom }
                }
            })
            .collect()
    }

    /// IntelliJ's side-by-side viewer: two panes with only their own lines,
    /// gutters against the divider that connects their change blocks.
    fn render_two_side(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let palette = cx.palette().clone();
        let actions = self.hunk_actions();
        let partial = self.partial_path(cx).is_some();
        let included = self.included(cx);
        let two = self.two.clone();
        let actions_width = if actions.is_empty() { 0. } else { BUTTON_WIDTH * actions.len() as f32 + 2. };
        let check_width = if partial { BUTTON_WIDTH + 2. } else { 0. };
        // With the left pane editable the arrows are `<<` on the right gutter.
        let left_edits = self.edit_pane() == Some(0);
        // The arrows sit inside the line-number column, by the text.
        let (left_inline, right_inline) = if left_edits { (0., actions_width) } else { (actions_width, 0.) };
        self.panes.layouts = vec![
            PaneLayout { mirrored: true, inline_buttons: left_inline, ..Default::default() },
            PaneLayout { mirrored: false, buttons: check_width, inline_buttons: right_inline, ..Default::default() },
        ];
        self.panes.apply_settings(cx);
        // A review's comment buttons take the fixed-width number column.
        if self.review.is_some() {
            self.panes.layouts[1].digits = 0;
        }
        self.panes.primary = 0;
        // A review's line numbers are its comment buttons.
        self.panes.layouts[1].hide_numbers &= self.review.is_none();
        let height = self.panes.view_height.get();
        let visible = if height > 0. { height } else { 1600. };
        let append = self.panes.ctrl_held && self.editable();

        let mut panes = Vec::new();
        for pane in 0..2 {
            let looks = self.row_looks(pane, &palette, cx);
            let layout = self.panes.layouts[pane];
            let mut overlays = Vec::new();
            // Per change: the insertion line on an empty side, and the gutter buttons.
            for seg in two.segments.iter() {
                let Some(change) = seg.change else { continue };
                let range = if pane == 0 { seg.left.clone() } else { seg.right.clone() };
                let y = self.panes.row_top(pane, range.start);
                if self.panes.row_top(pane, range.end) < -LINE_HEIGHT || y > visible + LINE_HEIGHT {
                    continue;
                }
                let rows = if pane == 0 { &two.left[range.clone()] } else { &two.right[range.clone()] };
                let empty = TwoSide::no_lines(rows);
                if empty {
                    overlays.push(div().absolute().left_0().right_0().top(px(y)).h(px(1.)).bg(border_color(seg.kind, &palette)).into_any_element());
                }
                // On an empty side the buttons sit on the row below the insertion line, as in IntelliJ.
                let button_top = y;
                let column = || {
                    h_flex()
                        .absolute()
                        .top(px(button_top))
                        .h(px(LINE_HEIGHT))
                        .justify_center()
                        .items_center()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                };
                if pane == left_edits as usize && !actions.is_empty() {
                    let mut el = if left_edits { column().left(px(layout.buttons_offset())) } else { column().right(px(layout.buttons_offset())) }.w(px(actions_width));
                    for (action, icon, tooltip) in actions.iter().copied() {
                        let (icon, tooltip) = match action {
                            HunkAction::Revert if append && left_edits => (IconName::ArrowLeftToLine, "Append"),
                            HunkAction::Revert if append => (IconName::ArrowRightToLine, "Append"),
                            HunkAction::Revert if left_edits => (IconName::ChevronsLeft, "Replace"),
                            HunkAction::Revert => (IconName::ChevronsRight, if tooltip == "Replace" { "Replace" } else { "Revert" }),
                            _ => (icon, tooltip),
                        };
                        el = el.child(
                            tool_button(gpui_kit::ElementId::NamedInteger(format!("pane-{tooltip}").into(), change as u64), icon, tooltip)
                                .on_click(cx.listener(move |this, _, window, cx| this.apply_hunk(change, action, window, cx))),
                        );
                    }
                    overlays.push(el.into_any_element());
                }
                if pane == 1 && partial {
                    let checked = included.get(change).copied().unwrap_or(true);
                    overlays.push(
                        column()
                            .left(px(layout.buttons_offset()))
                            .w(px(check_width))
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
            panes.push(self.panes.render_pane(pane, PaneContent { looks, overlays }, &palette, window, cx));
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
        let scroll = (self.panes.scroll[0].1, self.panes.scroll[1].1);
        let tops = (self.panes.tops(0).to_vec(), self.panes.tops(1).to_vec());
        let folds = fold_links(&two);
        let fold_color = palette.border;
        let divider = self.panes.divider_area(0, cx).child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    window.with_content_mask(Some(gpui_kit::ContentMask { bounds }), |window| {
                        paint_divider(bounds, scroll, (&tops.0, &tops.1), &connectors, &folds, fold_color, window)
                    })
                },
            )
            .size_full(),
        );
        let marks = |pane: usize| -> Vec<(Range<usize>, Hsla)> {
            two.segments
                .iter()
                .filter(|s| s.change.is_some())
                .map(|s| (if pane == 0 { s.left.clone() } else { s.right.clone() }, border_color(s.kind, &palette)))
                .collect()
        };
        let thumb = palette.text_disabled.opacity(0.25);
        let (left_marks, right_marks) = (marks(0), marks(1));
        let mut panes = panes.into_iter();
        let (left, right) = (panes.next().unwrap(), panes.next().unwrap());
        let entity = cx.entity();
        pane_area("diff-two-side", &self.panes.focus, cx)
            .child(self.panes.render_stripe(0, left_marks, thumb, cx))
            .child(left)
            .child(divider)
            .child(right)
            .child(self.panes.render_stripe(1, right_marks, thumb, cx))
            .context_menu(move |menu, _, cx| DiffView::context_menu(&entity, menu, cx))
            .into_any_element()
    }

    /// How each visible row of the unified pane looks: deleted lines red,
    /// inserted green, with changed words, the block's edges and two line
    /// number columns.
    fn unified_looks(&self, palette: &Palette, cx: &mut Context<Self>) -> Vec<RowLook> {
        let rows = self.rows.clone();
        let highlight = self.options.highlight;
        let review = self.review.clone();
        let path: Rc<str> = self.source.as_ref().map(|s| s.path()).unwrap_or_default().into();
        let exclusions = self.exclusions(cx);
        let hide = crate::settings::Settings::get(cx).diff.show_line_numbers == false;
        let cell = |n: Option<usize>| {
            div()
                .w(px(GUTTER_WIDTH))
                .h_full()
                .flex_shrink_0()
                .text_right()
                .pr_1()
                .text_color(palette.text_disabled)
                .children(n.map(|n| n.to_string()))
        };
        let extra = self.panes.buffers[1].line_count();
        self.panes
            .visible_rows(1)
            .map(|ix| {
                let Some(Display::Line { kind, left, right, change }) = rows.get(ix) else {
                    // The empty last line after a final line break.
                    return RowLook {
                        number: (ix == rows.len() && !hide).then(|| h_flex().child(cell(None)).child(cell(Some(extra))).into_any_element()),
                        ..Default::default()
                    };
                };
                let is_left = right.is_none();
                let side = right.as_ref().or(left.as_ref());
                let new_number = match (&review, right) {
                    (Some(r), Some(side)) => review_number(r, path.clone(), ix, side.line, palette, cx),
                    _ => cell(right.as_ref().map(|s| s.line)).into_any_element(),
                };
                let number = (!hide).then(|| h_flex().h_full().child(cell(left.as_ref().map(|s| s.line))).child(new_number).into_any_element());
                let Some(change) = *change else {
                    return RowLook { number, ..Default::default() };
                };
                let colors = side_colors(*kind, is_left, true, palette);
                let faint = {
                    let hunk = self.diff.hunks.get(change);
                    let lines = exclusions.get(change).map(|e| if is_left { &e.0 } else { &e.1 });
                    let at = side.zip(hunk).map(|(s, h)| (s.line - 1).wrapping_sub(if is_left { h.old.start } else { h.new.start }));
                    if lines.zip(at).and_then(|(l, at)| l.get(at)).copied().unwrap_or(false) { 0.35 } else { 1. }
                };
                let background = colors.filter(|_| highlight != HighlightMode::None).map(|c| match side.and_then(|s| s.whole) {
                    Some(_) if highlight.inner() => c.1.opacity(faint),
                    _ => c.0.opacity(faint),
                });
                let words = match (side, colors) {
                    (Some(side), Some(colors)) if highlight.inner() && side.whole.is_none() => {
                        side.changed.iter().filter(|r| !r.is_empty()).map(|r| (r.clone(), colors.1.opacity(faint))).collect()
                    }
                    _ => Vec::new(),
                };
                let marker = colors.filter(|_| highlight == HighlightMode::None).map(|c| c.1);
                let block = (highlight != HighlightMode::None).then(|| border_color(*kind, palette));
                let same = |r: Option<&Display>| matches!(r, Some(Display::Line { change: Some(c), .. }) if *c == change);
                let top = block.filter(|_| ix == 0 || !same(rows.get(ix - 1)));
                let bottom = block.filter(|_| !same(rows.get(ix + 1)));
                let gutter = colors.filter(|_| highlight != HighlightMode::None).map(|c| c.0.opacity(faint));
                RowLook { background, words, marker, number, gutter, top, bottom }
            })
            .collect()
    }

    /// IntelliJ's unified viewer: one editor with the new text (editable for
    /// the working tree), each block's deleted lines read-only above it,
    /// old and new line numbers, and the block's buttons in the gutter.
    fn render_unified(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let palette = cx.palette().clone();
        // The unified viewer edits only the right side.
        let actions = if self.edit_pane() == Some(0) { Vec::new() } else { self.hunk_actions() };
        let partial = self.partial_path(cx).is_some();
        let included = self.included(cx);
        let buttons = actions.len() + partial as usize;
        let buttons_width = if buttons == 0 { 0. } else { BUTTON_WIDTH * buttons as f32 + 2. };
        self.panes.layouts = vec![
            PaneLayout::default(),
            PaneLayout { mirrored: false, buttons: buttons_width, double_numbers: true, ..Default::default() },
        ];
        self.panes.apply_settings(cx);
        self.panes.layouts[1].double_numbers = true;
        self.panes.primary = 1;
        // The hidden left pane must not catch clicks.
        self.panes.bounds_reset(0);
        let looks = self.unified_looks(&palette, cx);
        let layout = self.panes.layouts[1];
        let visible = self.panes.visible_rows(1);
        let rows = self.rows.clone();
        let mut overlays = Vec::new();
        for ix in visible {
            let Some(Display::Line { change: Some(change), .. }) = rows.get(ix) else { continue };
            let change = *change;
            let first = ix == 0 || !matches!(rows.get(ix - 1), Some(Display::Line { change: Some(c), .. }) if *c == change);
            if !first || buttons == 0 {
                continue;
            }
            let mut el = h_flex()
                .absolute()
                .top(px(self.panes.row_top(1, ix)))
                .left(px(layout.buttons_offset()))
                .w(px(buttons_width))
                .h(px(LINE_HEIGHT))
                .justify_center()
                .items_center()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
            if partial {
                let checked = included.get(change).copied().unwrap_or(true);
                el = el.child(
                    Checkbox::new(gpui_kit::ElementId::NamedInteger("unified-include".into(), change as u64))
                        .checked(checked)
                        .tooltip("Include into commit")
                        .on_change(cx.listener(move |this, value: &bool, _, cx| this.toggle_hunk(change, *value, cx))),
                );
            }
            for (action, icon, tooltip) in actions.iter().copied() {
                let (icon, tooltip) = if action == HunkAction::Revert { (IconName::Close, "Revert") } else { (icon, tooltip) };
                el = el.child(
                    tool_button(gpui_kit::ElementId::NamedInteger(format!("unified-{tooltip}").into(), change as u64), icon, tooltip)
                        .on_click(cx.listener(move |this, _, window, cx| this.apply_hunk(change, action, window, cx))),
                );
            }
            overlays.push(el.into_any_element());
        }
        let pane = self.panes.render_pane(1, PaneContent { looks, overlays }, &palette, window, cx);
        // Error stripe marks: each block's rows.
        let mut marks: Vec<(Range<usize>, Hsla)> = Vec::new();
        for (ix, row) in rows.iter().enumerate() {
            if let Display::Line { change: Some(c), kind, .. } = row {
                match marks.last_mut() {
                    Some((range, _)) if range.end == ix && matches!(rows.get(ix - 1), Some(Display::Line { change: Some(p), .. }) if p == c) => {
                        range.end = ix + 1
                    }
                    _ => marks.push((ix..ix + 1, border_color(*kind, &palette))),
                }
            }
        }
        let thumb = palette.text_disabled.opacity(0.25);
        let entity = cx.entity();
        pane_area("diff-unified", &self.panes.focus, cx)
            .child(pane)
            .child(self.panes.render_stripe(1, marks, thumb, cx))
            .context_menu(move |menu, _, cx| DiffView::context_menu(&entity, menu, cx))
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
        let edit_pane = self.edit_pane();
        h_flex()
            .h(px(26.))
            .flex_shrink_0()
            .text_xs()
            .border_b_1()
            .border_color(palette.border)
            .child(div().w(px(STRIPE_WIDTH)).flex_shrink_0())
            .child(
                h_flex()
                    .flex_basis(px(0.))
                    .map(|mut el| {
                        el.style().flex_grow = Some(self.panes.weight(0));
                        el
                    })
                    .min_w_0()
                    .pl(px(6.))
                    .gap_1p5()
                    // A lock marks a read-only side.
                    .when(edit_pane != Some(0), |el| el.child(common::icon(IconName::Lock).text_color(palette.text_secondary)))
                    .child(div().flex_shrink_0().child(old_title))
                    .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(palette.text_secondary).child(path)),
            )
            .child(div().w(px(DIVIDER_WIDTH)).flex_shrink_0())
            .child(
                h_flex()
                    .flex_basis(px(0.))
                    .map(|mut el| {
                        el.style().flex_grow = Some(self.panes.weight(1));
                        el
                    })
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
                    .when(edit_pane != Some(1), |el| el.child(common::icon(IconName::Lock).text_color(palette.text_secondary)))
                    .child(new_title),
            )
            .child(div().w(px(STRIPE_WIDTH)).flex_shrink_0())
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        self.panes.before_render();
        self.take_settings(cx);
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
        let mode = self.mode;
        let titles = self.loaded.as_ref().map(|l| (l.old_title.clone(), l.new_title.clone()));
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
                None if two_side => el.child(self.render_two_side(window, cx)),
                None => el.child(self.render_unified(window, cx)),
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

}
