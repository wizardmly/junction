//! The editor panes IntelliJ's diff and merge viewers are made of: each pane
//! shows one text (rows may be collapsed into folds), with its gutter on the
//! side that faces the divider, syntax colors, a caret and selection, and,
//! for the one editable pane, typing, IME, undo and the clipboard. Panes
//! scroll together through the change blocks that link them.
//!
//! A view owns a [`TextPanes`] and implements [`PaneHost`]; the generic
//! glue here turns events into caret moves and edits, and tells the host
//! when the editable text changed.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{
    AnyElement, App, Bounds, ClipboardItem, Context, DispatchPhase, ElementInputHandler, FocusHandle, HighlightStyle, Hsla,
    InteractiveElement as _, IntoElement, KeyDownEvent, ModifiersChangedEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    ParentElement as _, Pixels, ScrollWheelEvent, SharedString, StatefulInteractiveElement as _, Styled as _, StyledText,
    TextRun, UTF16Selection, Window, actions, canvas, div, fill, font, point, prelude::FluentBuilder as _, px, size,
};

use crate::theme::Palette;
use crate::ui::common;
use crate::ui::diff_panes::{Segment, font_size, line_height, map_row};
use crate::ui::text_buffer::{Buffer, EditKind, History, Selection};

actions!(text_panes, [Paste]);

/// The key context of the panes.
pub const PANE_CONTEXT: &str = "TextPanes";
/// Text starts this far right of its cell's left edge.
pub const TEXT_PADDING: f32 = 8.;
pub const GUTTER_WIDTH: f32 = 44.;
/// One gutter button column.
pub const BUTTON_WIDTH: f32 = 20.;
pub const STRIPE_WIDTH: f32 = 12.;
/// The gutter's change marker, against the divider.
const MARKER_WIDTH: f32 = 3.;
const TAB_WIDTH: usize = 4;
/// Approximate advance of the monospace font, for horizontal scroll bounds
/// until the font is measured.
fn char_width_guess() -> f32 {
    7.6 * font_size() / 12.5
}

/// What a pane row shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowTarget {
    /// A buffer line (0-based).
    Line(usize),
    /// A collapsed run of lines.
    Fold { id: usize, count: usize },
    /// Empty space that keeps a change block level with the other pane's
    /// ("Align Changes").
    Filler,
    /// A read-only line of another pane's buffer, shown here: the unified
    /// viewer's deleted lines above the inserted ones.
    Other { pane: usize, line: usize },
}

/// Where a buffer line is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineRow {
    Row(usize),
    /// Inside a collapsed fragment: (fold id, its row).
    Fold(usize, usize),
}

/// How one row is painted, as the host decides.
#[derive(Default)]
pub struct RowLook {
    pub background: Option<Hsla>,
    /// Changed words, as byte ranges of the line.
    pub words: Vec<(Range<usize>, Hsla)>,
    /// The gutter marker (shown when line highlighting is off).
    pub marker: Option<Hsla>,
    /// Replaces the plain line number (a review diff's comment button).
    pub number: Option<AnyElement>,
    /// The block's color across the whole row, gutter included, under the
    /// text's own `background`.
    pub gutter: Option<Hsla>,
    /// The block's edge lines above and below the row (its first / last).
    pub top: Option<Hsla>,
    pub bottom: Option<Hsla>,
}

/// A pane's gutter: on the right of the text (`mirrored`, the left pane of
/// a diff) or on its left. The button column (`>>`, `<<`, checkboxes) sits
/// against the divider, the line numbers between it and the text.
#[derive(Clone, Copy, Debug, Default)]
pub struct PaneLayout {
    pub mirrored: bool,
    pub buttons: f32,
    /// Gear menu › Show Line Numbers turned off.
    pub hide_numbers: bool,
    /// Two number columns (the unified viewer's old and new line numbers,
    /// which the host supplies as each row's `number`).
    pub double_numbers: bool,
    /// Digits of the largest line number: the column fits them, as in
    /// IntelliJ (0: the fixed width).
    pub digits: usize,
    /// Buttons drawn inside the number column on its text side (the
    /// diff's `>>`), so they take no column of their own.
    pub inline_buttons: f32,
}

impl PaneLayout {
    pub fn numbers_width(&self) -> f32 {
        let numbers = if self.hide_numbers {
            0.
        } else if self.double_numbers {
            2. * GUTTER_WIDTH
        } else if self.digits > 0 {
            self.digits.max(2) as f32 * char_width_guess() + 8.
        } else {
            GUTTER_WIDTH
        };
        numbers + self.inline_buttons
    }

    /// Marker, line numbers and buttons.
    pub fn gutter_width(&self) -> f32 {
        MARKER_WIDTH + self.numbers_width() + self.buttons
    }

    /// The pane-local x where text begins (before horizontal scrolling).
    pub fn text_left(&self) -> f32 {
        if self.mirrored { TEXT_PADDING } else { self.gutter_width() + TEXT_PADDING }
    }

    /// The pane-local x of the button column, from the pane's left (normal)
    /// or right (mirrored) edge. As in IntelliJ, the line numbers sit
    /// against the divider and the buttons between them and the text.
    pub fn buttons_offset(&self) -> f32 {
        MARKER_WIDTH + self.numbers_width() - self.inline_buttons
    }
}

/// A line-level edit of the editable buffer: the lines `first..=old_last`
/// became `first..=new_last`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineEdit {
    pub first: usize,
    pub old_last: usize,
    pub new_last: usize,
}

pub struct TextPanes<T = ()> {
    pub buffers: Vec<Buffer>,
    /// The pane that measures the view and follows drags (the first one
    /// shown; the unified viewer shows only the new side's).
    pub primary: usize,
    /// The pane that edits, if any.
    pub editable: Option<usize>,
    /// The pane with the caret, and its selection.
    pub caret: Option<(usize, Selection)>,
    /// The display column Up / Down keep to.
    goal: Option<usize>,
    selecting: bool,
    pub history: History<T>,
    /// The host's state that undoes with the text.
    pub extra: T,
    marked: Option<Range<usize>>,
    pub focus: FocusHandle,
    highlighters: Vec<Option<gpui_kit::component::highlighter::SyntaxHighlighter>>,
    rows: Vec<Vec<RowTarget>>,
    line_rows: Vec<Vec<LineRow>>,
    pub layouts: Vec<PaneLayout>,
    /// Each pane's scroll position (x, y) in pixels.
    pub scroll: Vec<(f32, f32)>,
    /// Pairs of panes that scroll together, through their change blocks.
    pub links: Vec<(usize, usize, Vec<Segment>)>,
    /// Synchronize Scrolling.
    pub sync: bool,
    pub view_height: Rc<Cell<f32>>,
    bounds: Rc<RefCell<Vec<Bounds<Pixels>>>>,
    stripe_bounds: Rc<RefCell<Vec<Bounds<Pixels>>>>,
    stripe_drag: Option<usize>,
    max_cols: Vec<usize>,
    /// Ctrl is held (diff `>>` appends, merge arrows append).
    pub ctrl_held: bool,
    /// Rows to bring to a third of the height once the panes are measured.
    pending: Option<Vec<(usize, usize)>>,
    /// Edits since the host last took them.
    pub line_edits: Vec<LineEdit>,
    /// The last change came from undo / redo (the host's state was restored).
    pub restored: bool,
    /// Gear menu › Show Whitespaces.
    pub show_whitespace: bool,
    /// Gear menu › Show Indent Guides.
    pub indent_guides: bool,
    /// Each pane's share of the width (the dividers are draggable).
    weights: Vec<f32>,
    drag: Option<Drag>,
    /// The font's advance, measured when painting.
    char_width: Rc<Cell<f32>>,
    /// Gear menu › Soft-Wrap.
    pub soft_wrap: bool,
    /// Where each row's line breaks, as byte offsets in the line (the first
    /// is 0); empty when soft wrap is off.
    wraps: Vec<Vec<Vec<usize>>>,
    /// Each row's top before scrolling (one more entry: the content height).
    /// Wrapped rows are taller, and level with their paired rows.
    tops: Vec<Vec<f32>>,
    /// The columns the wraps were made for.
    wrap_cols: Vec<usize>,
    /// The panes changed rows since the heights were paired.
    layout_dirty: bool,
    /// Space the host fills below a row (a review's inline comment thread),
    /// per pane by row.
    blocks: Vec<std::collections::HashMap<usize, f32>>,
}

/// A mouse drag the panes follow anywhere in the window.
#[derive(Clone, Debug)]
enum Drag {
    /// The divider after pane `left`, resizing it and the next pane.
    Divider { left: usize, start_x: f32, widths: Vec<f32> },
    /// A pane's horizontal scrollbar thumb.
    HScroll { pane: usize, start_x: f32, start_scroll: f32 },
}

/// Height of the horizontal scrollbar at the bottom of a pane.
const HSCROLL_HEIGHT: f32 = 10.;

/// What an event did, for the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Ignored,
    Moved,
    Edited,
    OpenFold(usize),
}

impl<T: Clone + Default> TextPanes<T> {
    pub fn new(panes: usize, cx: &mut App) -> Self {
        TextPanes {
            buffers: vec![Buffer::default(); panes],
            primary: 0,
            editable: None,
            caret: None,
            goal: None,
            selecting: false,
            history: History::default(),
            extra: T::default(),
            marked: None,
            focus: cx.focus_handle(),
            highlighters: (0..panes).map(|_| None).collect(),
            rows: vec![Vec::new(); panes],
            line_rows: vec![Vec::new(); panes],
            layouts: vec![PaneLayout::default(); panes],
            scroll: vec![(0., 0.); panes],
            links: Vec::new(),
            sync: true,
            view_height: Rc::new(Cell::new(0.)),
            bounds: Rc::new(RefCell::new(vec![Bounds::default(); panes])),
            stripe_bounds: Rc::new(RefCell::new(vec![Bounds::default(); panes])),
            stripe_drag: None,
            max_cols: vec![0; panes],
            ctrl_held: false,
            pending: None,
            line_edits: Vec::new(),
            restored: false,
            show_whitespace: false,
            indent_guides: true,
            weights: vec![1.; panes],
            drag: None,
            char_width: Rc::new(Cell::new(char_width_guess())),
            soft_wrap: false,
            wraps: vec![Vec::new(); panes],
            tops: vec![vec![0.]; panes],
            wrap_cols: vec![0; panes],
            layout_dirty: false,
            blocks: vec![Default::default(); panes],
        }
    }

    /// New texts: forgets the caret, undo and highlighting.
    pub fn set_texts(&mut self, texts: Vec<String>, language: &str) {
        self.set_plain_texts(texts);
        for pane in 0..self.buffers.len() {
            self.highlight(pane, language);
        }
    }

    /// New texts without syntax colors yet: they come from
    /// [`highlight_texts`] off the main thread, through [`Self::set_highlighters`].
    pub fn set_plain_texts(&mut self, texts: Vec<String>) {
        self.buffers = texts.into_iter().map(Buffer::new).collect();
        self.caret = None;
        self.marked = None;
        self.history.clear();
        self.line_edits.clear();
        self.highlighters = (0..self.buffers.len()).map(|_| None).collect();
    }

    /// Installs highlighters made for `texts`, for the panes still showing them.
    pub fn set_highlighters(&mut self, texts: &[String], highlighters: Vec<Highlighter>) {
        for (pane, (text, highlighter)) in texts.iter().zip(highlighters).enumerate() {
            if self.buffers.get(pane).is_some_and(|b| b.text() == text) {
                self.highlighters[pane] = Some(highlighter);
            }
        }
    }

    /// Re-parses one pane for syntax colors (keeping its highlighter, whose
    /// queries are costly to build, when the language is the same).
    pub fn highlight(&mut self, pane: usize, language: &str) {
        let mut highlighter = match self.highlighters[pane].take() {
            Some(h) if h.language().as_ref() == language => h,
            _ => take_highlighter(language),
        };
        highlighter.update(None, &gpui_kit::component::Rope::from_str(self.buffers[pane].text()), Some(Duration::from_millis(200)));
        self.highlighters[pane] = Some(highlighter);
    }

    pub fn line_row(&self, pane: usize, line: usize) -> Option<LineRow> {
        self.line_rows[pane].get(line).copied()
    }

    /// The row a line is on (a folded line: its fold's row; past the end:
    /// the row count).
    pub fn row_of(&self, pane: usize, line: usize) -> usize {
        match self.line_row(pane, line) {
            Some(LineRow::Row(row) | LineRow::Fold(_, row)) => row,
            None => self.rows[pane].len(),
        }
    }

    /// The host's rows for a pane (after a re-diff or a fold change).
    pub fn set_rows(&mut self, pane: usize, rows: Vec<RowTarget>) {
        let mut map = Vec::new();
        for (ix, row) in rows.iter().enumerate() {
            match *row {
                RowTarget::Line(line) => {
                    map.resize(line, LineRow::Row(ix));
                    map.push(LineRow::Row(ix));
                }
                RowTarget::Fold { id, count } => map.extend(std::iter::repeat_n(LineRow::Fold(id, ix), count)),
                RowTarget::Filler | RowTarget::Other { .. } => {}
            }
        }
        self.line_rows[pane] = map;
        let cols = |text: &str| -> usize { text.chars().map(|c| if c == '\t' { TAB_WIDTH } else { 1 }).sum() };
        let b = &self.buffers[pane];
        let own = (0..b.line_count()).map(|l| cols(b.line(l))).max().unwrap_or(0);
        let others = rows
            .iter()
            .filter_map(|r| match *r {
                RowTarget::Other { pane: p, line } => self.buffers.get(p).filter(|b| line < b.line_count()).map(|b| cols(b.line(line))),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        self.max_cols[pane] = own.max(others);
        self.rows[pane] = rows;
        self.relayout_pane(pane);
        self.layout_dirty = true;
        let max = self.max_scroll_y(pane);
        self.scroll[pane].1 = self.scroll[pane].1.min(max);
    }

    // Row heights (soft wrap).

    /// The text columns that fit a pane, for soft wrap.
    fn fit_cols(&self, pane: usize) -> usize {
        let width = f32::from(self.bounds.borrow()[pane].size.width) - self.layouts[pane].gutter_width() - 2. * TEXT_PADDING;
        if !self.soft_wrap || width <= 0. { usize::MAX } else { ((width / self.char_width.get()) as usize).max(8) }
    }

    /// One pane's wraps and row tops, unpaired.
    fn relayout_pane(&mut self, pane: usize) {
        let cols = self.fit_cols(pane);
        self.wrap_cols[pane] = cols;
        let rows = &self.rows[pane];
        let mut tops = Vec::with_capacity(rows.len() + 1);
        let mut wraps = Vec::new();
        let blocks = &self.blocks[pane];
        if cols == usize::MAX && blocks.is_empty() {
            tops.extend((0..=rows.len()).map(|r| r as f32 * line_height()));
        } else if cols == usize::MAX {
            let mut y = 0.;
            for row in 0..rows.len() {
                tops.push(y);
                y += line_height() + blocks.get(&row).copied().unwrap_or(0.);
            }
            tops.push(y);
        } else {
            let mut y = 0.;
            for (ix, row) in rows.iter().enumerate() {
                let text = match *row {
                    RowTarget::Line(line) => Some(&self.buffers[pane]).filter(|b| line < b.line_count()).map(|b| b.line(line)),
                    RowTarget::Other { pane: p, line } => self.buffers.get(p).filter(|b| line < b.line_count()).map(|b| b.line(line)),
                    _ => None,
                };
                let starts = text.map_or_else(|| vec![0], |t| wrap_starts(t, cols));
                tops.push(y);
                y += starts.len() as f32 * line_height() + blocks.get(&ix).copied().unwrap_or(0.);
                wraps.push(starts);
            }
            tops.push(y);
        }
        self.wraps[pane] = wraps;
        self.tops[pane] = tops;
    }

    /// Re-wraps when soft wrap or a pane's width changed, and keeps paired
    /// rows (the same row of linked panes with as many rows) equally tall.
    fn relayout(&mut self) {
        let changed = (0..self.rows.len()).any(|p| self.fit_cols(p) != self.wrap_cols[p]);
        if !changed && !self.layout_dirty {
            return;
        }
        self.layout_dirty = false;
        for pane in 0..self.rows.len() {
            self.relayout_pane(pane);
        }
        if !self.soft_wrap {
            return;
        }
        let mut lines: Vec<Vec<usize>> = self.wraps.iter().map(|w| w.iter().map(Vec::len).collect()).collect();
        for _ in 0..2 {
            for (a, b, _) in &self.links {
                let (a, b) = (*a, *b);
                if a >= lines.len() || b >= lines.len() || lines[a].len() != lines[b].len() {
                    continue;
                }
                for row in 0..lines[a].len() {
                    let n = lines[a][row].max(lines[b][row]);
                    lines[a][row] = n;
                    lines[b][row] = n;
                }
            }
        }
        for (pane, lines) in lines.into_iter().enumerate() {
            if lines.is_empty() {
                continue;
            }
            let mut y = 0.;
            let tops = &mut self.tops[pane];
            tops.clear();
            for (row, n) in lines.into_iter().enumerate() {
                tops.push(y);
                y += n as f32 * line_height() + self.blocks[pane].get(&row).copied().unwrap_or(0.);
            }
            tops.push(y);
        }
    }

    /// A row's top before scrolling.
    pub fn row_y(&self, pane: usize, row: usize) -> f32 {
        let tops = &self.tops[pane];
        tops.get(row).or(tops.last()).copied().unwrap_or(0.)
    }

    /// The height of all of a pane's rows.
    pub fn content_height(&self, pane: usize) -> f32 {
        self.tops[pane].last().copied().unwrap_or(0.)
    }

    /// The row at a content y.
    pub fn row_at(&self, pane: usize, y: f32) -> usize {
        let tops = &self.tops[pane];
        let rows = tops.len().saturating_sub(1);
        tops.partition_point(|t| *t <= y).saturating_sub(1).min(rows.saturating_sub(1))
    }

    /// A content y as a fractional row (for pairing scroll positions).
    fn y_to_row(&self, pane: usize, y: f32) -> f32 {
        let row = self.row_at(pane, y);
        let (top, bottom) = (self.row_y(pane, row), self.row_y(pane, row + 1));
        if bottom <= top { (y / line_height()).max(0.) } else { row as f32 + ((y - top) / (bottom - top)).clamp(0., 1.) }
    }

    fn row_to_y(&self, pane: usize, row: f32) -> f32 {
        let whole = row.floor().max(0.) as usize;
        let (top, bottom) = (self.row_y(pane, whole), self.row_y(pane, whole + 1));
        top + (row - whole as f32) * (bottom - top).max(0.)
    }

    /// A row's line breaks.
    fn row_wraps(&self, pane: usize, row: usize) -> &[usize] {
        self.wraps[pane].get(row).map_or(&[0][..], Vec::as_slice)
    }

    /// Which wrapped part of its row an offset (in its line) is on: the
    /// part's index and its start.
    fn wrap_part(&self, pane: usize, row: usize, in_line: usize) -> (usize, usize) {
        let wraps = self.row_wraps(pane, row);
        let part = wraps.partition_point(|s| *s <= in_line).saturating_sub(1);
        (part, wraps.get(part).copied().unwrap_or(0))
    }

    /// Where an offset's caret goes in its pane, before scrolling: x from
    /// the text start, and the top of its visual line.
    fn caret_point(&self, pane: usize, offset: usize, window: &Window, cx: &App) -> Option<(f32, f32)> {
        let buffer = &self.buffers[pane];
        let line = buffer.line_of(offset);
        let LineRow::Row(row) = self.line_row(pane, line)? else { return None };
        let text = buffer.line(line);
        let in_line = offset - buffer.line_range(line).start;
        let (part, start) = self.wrap_part(pane, row, in_line);
        let end = self.row_wraps(pane, row).get(part + 1).copied().unwrap_or(text.len());
        let piece = &text[start..end.max(start)];
        let (display, _) = expand_tabs(piece, &[]);
        let at = display_offset(piece, in_line - start);
        let mut x = f32::from(shape(&display, window, cx).x_for_index(at));
        if part > 0 {
            x += wrap_indent(text, self.wrap_cols[pane]) as f32 * self.char_width.get();
        }
        Some((x, self.row_y(pane, row) + part as f32 * line_height()))
    }

    /// The pane rows every buffer line gets, with nothing folded.
    pub fn plain_rows(&self, pane: usize) -> Vec<RowTarget> {
        (0..self.buffers[pane].line_count()).map(RowTarget::Line).collect()
    }

    // Scrolling.

    /// Lets the last line scroll up to the middle of the pane.
    pub fn max_scroll_y(&self, pane: usize) -> f32 {
        (self.content_height(pane) - self.view_height.get() / 2.).max(0.)
    }

    /// Scrolls one pane; with Synchronize Scrolling the linked panes follow
    /// so that the rows at the middle stay paired.
    /// How wide a pane's text is, and how wide its view of it.
    fn text_extent(&self, pane: usize) -> (f32, f32) {
        let content = self.max_cols[pane] as f32 * self.char_width.get() + 2. * TEXT_PADDING;
        let view = f32::from(self.bounds.borrow()[pane].size.width) - self.layouts[pane].gutter_width();
        (content, view.max(0.))
    }

    fn max_scroll_x(&self, pane: usize) -> f32 {
        if self.soft_wrap {
            return 0.;
        }
        let (content, view) = self.text_extent(pane);
        if view <= 0. { 0. } else { (content - view).max(0.) }
    }

    pub fn scroll_to(&mut self, pane: usize, x: f32, y: f32) {
        let x = x.clamp(0., self.max_scroll_x(pane));
        let y = y.clamp(0., self.max_scroll_y(pane));
        self.scroll[pane] = (x, y);
        if !self.sync {
            return;
        }
        let half = self.view_height.get() / 2.;
        let mut done = vec![false; self.scroll.len()];
        done[pane] = true;
        let mut queue = vec![pane];
        while let Some(from) = queue.pop() {
            let row = self.y_to_row(from, self.scroll[from].1 + half);
            for (a, b, segments) in &self.links {
                let (to, from_left) = if *a == from { (*b, true) } else if *b == from { (*a, false) } else { continue };
                if done[to] {
                    continue;
                }
                let mapped = map_row(segments, from_left, row);
                let y = (self.row_to_y(to, mapped) - half).clamp(0., self.max_scroll_y(to));
                self.scroll[to] = (x.min(self.max_scroll_x(to)), y);
                done[to] = true;
                queue.push(to);
            }
        }
    }

    /// Puts rows a third of the way down their panes (Next Difference),
    /// once the panes have a height.
    pub fn show_rows(&mut self, targets: Vec<(usize, usize)>) {
        let height = self.view_height.get();
        if height <= 0. {
            self.pending = Some(targets);
            return;
        }
        for (pane, row) in targets {
            self.scroll[pane].1 = (self.row_y(pane, row) - height / 3.).clamp(0., self.max_scroll_y(pane));
        }
    }

    /// Forgets where a pane was painted, for a pane no longer shown.
    pub fn bounds_reset(&mut self, pane: usize) {
        if let Some(b) = self.bounds.borrow_mut().get_mut(pane) {
            *b = Bounds::default();
        }
    }

    /// Call at the start of the host's render.
    pub fn before_render(&mut self) {
        self.relayout();
        for pane in 0..self.scroll.len() {
            let (x, y) = self.scroll[pane];
            self.scroll[pane] = (x.min(self.max_scroll_x(pane)), y.min(self.max_scroll_y(pane)));
        }
        if self.view_height.get() > 0. {
            if let Some(targets) = self.pending.take() {
                self.show_rows(targets);
            }
        }
    }

    fn drag_move(&mut self, event: &MouseMoveEvent) -> bool {
        let Some(drag) = self.drag.clone() else { return false };
        if event.pressed_button != Some(MouseButton::Left) {
            self.drag = None;
            return true;
        }
        let x = f32::from(event.position.x);
        match drag {
            Drag::Divider { left, start_x, widths } => {
                let pair = widths[left] + widths[left + 1];
                let new_left = (widths[left] + x - start_x).clamp(80., (pair - 80.).max(80.));
                self.weights = widths;
                self.weights[left] = new_left;
                self.weights[left + 1] = pair - new_left;
            }
            Drag::HScroll { pane, start_x, start_scroll } => {
                let (content, view) = self.text_extent(pane);
                let thumb = (view * view / content).max(30.);
                let per_px = (content - view) / (view - thumb).max(1.);
                let y = self.scroll[pane].1;
                self.scroll_to(pane, start_scroll + (x - start_x) * per_px, y);
            }
        }
        true
    }

    /// A pane's share of the width, for headers that line up with it.
    pub fn weight(&self, pane: usize) -> f32 {
        self.weights.get(pane).copied().unwrap_or(1.)
    }

    /// Starts dragging the divider after pane `left`.
    fn start_divider(&mut self, left: usize, x: f32) {
        let widths: Vec<f32> = self.bounds.borrow().iter().map(|b| f32::from(b.size.width).max(1.)).collect();
        self.drag = Some(Drag::Divider { left, start_x: x, widths });
    }

    /// The divider after pane `left`: drag it to resize the two panes.
    pub fn divider_area<V: PaneHost<Extra = T>>(&self, left: usize, cx: &mut Context<V>) -> gpui_kit::Stateful<gpui_kit::Div> {
        div()
            .id(("pane-divider", left))
            .w(px(crate::ui::diff_panes::DIVIDER_WIDTH))
            .h_full()
            .flex_shrink_0()
            .cursor(gpui_kit::CursorStyle::ResizeLeftRight)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view: &mut V, e: &MouseDownEvent, _, cx| {
                    view.panes().start_divider(left, f32::from(e.position.x));
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
    }

    fn on_wheel(&mut self, pane: usize, event: &ScrollWheelEvent) {
        let delta = event.delta.pixel_delta(px(line_height()));
        let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
        let (x, y) = self.scroll[pane];
        if event.modifiers.shift && dx == 0. {
            self.scroll_to(pane, x - dy, y);
        } else {
            self.scroll_to(pane, x - dx, y - dy);
        }
    }

    /// The error stripe spans the whole pane content plus half a view.
    fn stripe_total(&self, pane: usize, height: f32) -> f32 {
        (self.content_height(pane) + self.view_height.get() / 2.).max(height).max(1.)
    }

    fn stripe_seek(&mut self, pane: usize, y: Pixels) {
        let bounds = self.stripe_bounds.borrow()[pane];
        let h = f32::from(bounds.size.height);
        if h <= 0. {
            return;
        }
        let t = (f32::from(y - bounds.origin.y) / h).clamp(0., 1.);
        let total = self.stripe_total(pane, h);
        let x = self.scroll[pane].0;
        self.scroll_to(pane, x, t * total - self.view_height.get() / 2.);
    }

    // Hit testing and shaping.

    /// Where a point lands: the pane, and a buffer offset (or a fold).
    fn hit(&self, position: gpui_kit::Point<Pixels>, window: &Window, cx: &App) -> Option<(usize, Result<usize, usize>)> {
        let bounds = self.bounds.borrow().clone();
        let pane = (0..bounds.len()).find(|p| bounds[*p].contains(&position)).or_else(|| self.caret.map(|c| c.0))?;
        let b = bounds[pane];
        let (scroll_x, scroll_y) = self.scroll[pane];
        let rows = &self.rows[pane];
        let y = f32::from(position.y - b.origin.y) + scroll_y;
        rows.len().checked_sub(1)?;
        let row = self.row_at(pane, y.max(0.));
        let line = match rows[row] {
            RowTarget::Fold { id, .. } => return Some((pane, Err(id))),
            RowTarget::Line(line) => line,
            // Filler (and another pane's line) belongs to the line above it
            // (or below, at the top).
            RowTarget::Filler | RowTarget::Other { .. } => {
                let line = |r: &RowTarget| if let RowTarget::Line(l) = r { Some(*l) } else { None };
                rows[..row].iter().rev().find_map(line).or_else(|| rows[row..].iter().find_map(line))?
            }
        };
        let buffer = &self.buffers[pane];
        let text = buffer.line(line);
        // On a wrapped line, the visual line under the point.
        let (start, end, indent) = match self.line_row(pane, line) {
            Some(LineRow::Row(own)) if self.wraps[pane].get(own).is_some_and(|w| w.len() > 1) => {
                let wraps = self.row_wraps(pane, own);
                let part = if own == row { (((y - self.row_y(pane, row)) / line_height()).floor().max(0.) as usize).min(wraps.len() - 1) } else { wraps.len() - 1 };
                let indent = if part > 0 { wrap_indent(text, self.wrap_cols[pane]) as f32 * self.char_width.get() } else { 0. };
                (wraps[part], wraps.get(part + 1).copied().unwrap_or(text.len()), indent)
            }
            _ => (0, text.len(), 0.),
        };
        let piece = &text[start..end];
        let (display, _) = expand_tabs(piece, &[]);
        let x = f32::from(position.x - b.origin.x) - self.layouts[pane].text_left() + scroll_x - indent;
        let index = shape(&display, window, cx).closest_index_for_x(px(x.max(0.)));
        let mut at = byte_offset(piece, index);
        // The end of a wrapped part is the start of the next one.
        if at == piece.len() && end < text.len() && at > 0 {
            at = piece.char_indices().last().map_or(0, |(i, _)| i);
        }
        Some((pane, Ok(buffer.line_range(line).start + start + at)))
    }

    /// Right click: the caret moves there unless it lands in the selection,
    /// so the context menu acts on what was clicked.
    fn right_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut App) -> Outcome {
        window.focus(&self.focus, cx);
        let Some((pane, Ok(offset))) = self.hit(event.position, window, cx) else { return Outcome::Ignored };
        if let Some((p, sel)) = self.caret {
            if p == pane && !sel.is_empty() && sel.range().contains(&offset) {
                return Outcome::Ignored;
            }
        }
        self.history.break_run();
        self.caret = Some((pane, Selection::caret(offset)));
        self.goal = None;
        Outcome::Moved
    }

    /// The caret pane's selected lines (the caret's line when nothing is
    /// selected; a selection ending at a line start leaves that line out).
    pub fn selected_lines(&self) -> Option<(usize, Range<usize>)> {
        let (pane, sel) = self.caret?;
        let buffer = &self.buffers[pane];
        let range = sel.range();
        let first = buffer.line_of(range.start);
        let mut last = buffer.line_of(range.end);
        if !sel.is_empty() && last > first && buffer.line_range(last).start == range.end {
            last -= 1;
        }
        Some((pane, first..last + 1))
    }

    fn select_all(&mut self) -> Outcome {
        let pane = self.caret.map(|c| c.0).or(self.editable).unwrap_or(0);
        self.caret = Some((pane, Selection { anchor: 0, head: self.buffers[pane].text().len() }));
        Outcome::Moved
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut App) -> Outcome {
        window.focus(&self.focus, cx);
        let Some((pane, target)) = self.hit(event.position, window, cx) else { return Outcome::Ignored };
        let offset = match target {
            Err(fold) => return Outcome::OpenFold(fold),
            Ok(offset) => offset,
        };
        let buffer = &self.buffers[pane];
        let selection = match event.click_count {
            2 => {
                let word = buffer.word_at(offset);
                Selection { anchor: word.start, head: word.end }
            }
            n if n >= 3 => {
                let line = buffer.line_of(offset);
                let end = if line + 1 < buffer.line_count() { buffer.line_range(line + 1).start } else { buffer.line_range(line).end };
                Selection { anchor: buffer.line_range(line).start, head: end }
            }
            _ => match self.caret {
                Some((p, sel)) if p == pane && event.modifiers.shift => Selection { anchor: sel.anchor, head: offset },
                _ => Selection::caret(offset),
            },
        };
        self.history.break_run();
        self.caret = Some((pane, selection));
        self.goal = None;
        self.selecting = true;
        Outcome::Moved
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut App) -> Outcome {
        if !self.selecting || event.pressed_button != Some(MouseButton::Left) {
            self.selecting = false;
            return Outcome::Ignored;
        }
        let Some((pane, sel)) = self.caret else { return Outcome::Ignored };
        // Dragging past an edge scrolls.
        let b = self.bounds.borrow()[pane];
        let (x, y) = self.scroll[pane];
        if event.position.y < b.origin.y {
            self.scroll_to(pane, x, y - line_height());
        } else if event.position.y > b.origin.y + b.size.height {
            self.scroll_to(pane, x, y + line_height());
        }
        let position = point(
            event.position.x.clamp(b.origin.x, b.origin.x + b.size.width),
            event.position.y.clamp(b.origin.y, b.origin.y + b.size.height - px(1.)),
        );
        match self.hit(position, window, cx) {
            Some((p, Ok(offset))) if p == pane && offset != sel.head => {
                self.caret = Some((pane, Selection { anchor: sel.anchor, head: offset }));
                Outcome::Moved
            }
            _ => Outcome::Ignored,
        }
    }

    // Keyboard.

    fn caret_editable(&self) -> bool {
        self.caret.is_some() && self.caret.map(|c| c.0) == self.editable
    }

    fn set_head(&mut self, head: usize, select: bool, keep_goal: bool) -> Outcome {
        let Some((pane, sel)) = self.caret else { return Outcome::Ignored };
        let sel = if select { Selection { anchor: sel.anchor, head } } else { Selection::caret(head) };
        if !keep_goal {
            self.goal = None;
        }
        self.history.break_run();
        self.caret = Some((pane, sel));
        Outcome::Moved
    }

    /// Up / Down / Page keys: whole lines, keeping the starting column.
    fn vertical(&mut self, lines: isize, select: bool) -> Outcome {
        let Some((pane, sel)) = self.caret else { return Outcome::Ignored };
        let buffer = &self.buffers[pane];
        let line = buffer.line_of(sel.head);
        let goal = self.goal.unwrap_or_else(|| display_offset(buffer.line(line), sel.head - buffer.line_range(line).start));
        let target = (line as isize + lines).clamp(0, buffer.line_count() as isize - 1) as usize;
        let offset = buffer.line_range(target).start + byte_offset(buffer.line(target), goal);
        self.goal = Some(goal);
        self.set_head(offset, select, true)
    }

    fn key_down(&mut self, event: &KeyDownEvent, cx: &mut App) -> Outcome {
        let Some((pane, sel)) = self.caret else { return Outcome::Ignored };
        let keystroke = &event.keystroke;
        let m = keystroke.modifiers;
        let (ctrl, shift) = (m.secondary(), m.shift);
        if m.alt {
            return Outcome::Ignored;
        }
        let page = ((self.view_height.get() / line_height()) as isize - 1).max(1);
        let b = &self.buffers[pane];
        match (keystroke.key.as_str(), ctrl) {
            ("left", false) => {
                let head = if !shift && !sel.is_empty() { sel.range().start } else { b.prev_char(sel.head) };
                self.set_head(head, shift, false)
            }
            ("right", false) => {
                let head = if !shift && !sel.is_empty() { sel.range().end } else { b.next_char(sel.head) };
                self.set_head(head, shift, false)
            }
            ("left", true) => self.set_head(b.prev_word(sel.head), shift, false),
            ("right", true) => self.set_head(b.next_word(sel.head), shift, false),
            ("up", false) => self.vertical(-1, shift),
            ("down", false) => self.vertical(1, shift),
            ("pageup", _) => self.vertical(-page, shift),
            ("pagedown", _) => self.vertical(page, shift),
            // Smart Home: the first non-blank, then the line start.
            ("home", false) => {
                let line = b.line_of(sel.head);
                let start = b.line_range(line).start;
                let text_start = start + b.indent(line).len();
                self.set_head(if sel.head == text_start { start } else { text_start }, shift, false)
            }
            ("end", false) => self.set_head(b.line_range(b.line_of(sel.head)).end, shift, false),
            ("home", true) => self.set_head(0, shift, false),
            ("end", true) => self.set_head(b.text().len(), shift, false),
            ("a", true) => self.select_all(),
            ("c", true) | ("insert", true) => {
                self.copy(false, cx);
                Outcome::Moved
            }
            ("x", true) => self.copy(true, cx),
            ("z", true) if shift => self.redo(),
            ("z", true) => self.undo(),
            ("escape", false) if !sel.is_empty() => self.set_head(sel.head, false, false),
            _ if !self.caret_editable() => Outcome::Ignored,
            ("backspace", _) => self.delete(false, ctrl),
            ("delete", _) => self.delete(true, ctrl),
            ("enter", false) => self.newline(),
            ("tab", false) if shift => self.unindent(),
            ("tab", false) => {
                let unit = self.indent_unit();
                self.insert(unit, EditKind::Typing)
            }
            ("d", true) => self.duplicate_line(),
            ("y", true) => self.delete_line(),
            _ => Outcome::Ignored,
        }
    }

    fn indent_unit(&self) -> &'static str {
        let Some(pane) = self.editable else { return "    " };
        let b = &self.buffers[pane];
        if (0..b.line_count()).any(|l| b.indent(l).starts_with('\t')) { "\t" } else { "    " }
    }

    /// Copies the selection, or the whole line when nothing is selected
    /// (IntelliJ); with `cut`, removes it too.
    fn copy(&mut self, cut: bool, cx: &mut App) -> Outcome {
        let Some((pane, sel)) = self.caret else { return Outcome::Ignored };
        let b = &self.buffers[pane];
        let range = if sel.is_empty() {
            let line = b.line_of(sel.head);
            let end = if line + 1 < b.line_count() { b.line_range(line + 1).start } else { b.text().len() };
            b.line_range(line).start..end
        } else {
            sel.range()
        };
        let mut text = b.text()[range.clone()].to_owned();
        if sel.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        if cut && self.caret_editable() {
            self.edit(range, "", EditKind::Other, None);
            return Outcome::Edited;
        }
        Outcome::Moved
    }

    fn paste(&mut self, cx: &mut App) -> Outcome {
        let Some(pane) = self.editable.filter(|_| self.caret_editable()) else { return Outcome::Ignored };
        let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else { return Outcome::Ignored };
        let text = text.replace("\r\n", "\n").replace('\n', self.buffers[pane].newline());
        self.insert(&text, EditKind::Other)
    }

    fn undo(&mut self) -> Outcome {
        let Some(pane) = self.editable.filter(|_| self.caret_editable()) else { return Outcome::Ignored };
        let mut sel = self.caret.map(|c| c.1).unwrap_or_default();
        if !self.history.undo(&mut self.buffers[pane], &mut sel, &mut self.extra) {
            return Outcome::Ignored;
        }
        self.caret = Some((pane, sel));
        self.restored = true;
        self.line_edits.clear();
        Outcome::Edited
    }

    fn redo(&mut self) -> Outcome {
        let Some(pane) = self.editable.filter(|_| self.caret_editable()) else { return Outcome::Ignored };
        let mut sel = self.caret.map(|c| c.1).unwrap_or_default();
        if !self.history.redo(&mut self.buffers[pane], &mut sel, &mut self.extra) {
            return Outcome::Ignored;
        }
        self.caret = Some((pane, sel));
        self.restored = true;
        self.line_edits.clear();
        Outcome::Edited
    }

    /// Replaces a range of the editable buffer, recording undo (with the
    /// host's state as it was before). The caret lands after the new text
    /// unless `caret` says where; it stays in its pane if that isn't the
    /// editable one.
    pub fn edit(&mut self, range: Range<usize>, text: &str, kind: EditKind, caret: Option<usize>) {
        let Some(pane) = self.editable else { return };
        let sel = self.caret.filter(|c| c.0 == pane).map(|c| c.1).unwrap_or_default();
        self.history.record(&self.buffers[pane], sel, &self.extra, kind);
        let b = &mut self.buffers[pane];
        let (first, old_last) = (b.line_of(range.start), b.line_of(range.end));
        b.replace(range.clone(), text);
        let new_last = b.line_of(range.start + text.len());
        self.line_edits.push(LineEdit { first, old_last, new_last });
        self.restored = false;
        if self.caret.is_none_or(|c| c.0 == pane) || caret.is_none() {
            self.caret = Some((pane, Selection::caret(caret.unwrap_or(range.start + text.len()))));
            self.goal = None;
        }
    }

    /// Records the host's state for undo before a change that edits no text
    /// (merge: ignoring a side).
    pub fn record_extra(&mut self) {
        let Some(pane) = self.editable else { return };
        let sel = self.caret.filter(|c| c.0 == pane).map(|c| c.1).unwrap_or_default();
        self.history.record(&self.buffers[pane], sel, &self.extra, EditKind::Other);
    }

    fn insert(&mut self, text: &str, kind: EditKind) -> Outcome {
        let Some((_, sel)) = self.caret else { return Outcome::Ignored };
        let kind = if sel.is_empty() { kind } else { EditKind::Other };
        self.edit(sel.range(), text, kind, None);
        Outcome::Edited
    }

    fn delete(&mut self, forward: bool, word: bool) -> Outcome {
        let Some((pane, sel)) = self.caret else { return Outcome::Ignored };
        let b = &self.buffers[pane];
        let range = if !sel.is_empty() {
            sel.range()
        } else if forward {
            sel.head..if word { b.next_word(sel.head) } else { b.next_char(sel.head) }
        } else {
            (if word { b.prev_word(sel.head) } else { b.prev_char(sel.head) })..sel.head
        };
        if range.is_empty() {
            return Outcome::Ignored;
        }
        // A "\r\n" goes as one.
        let text = b.text();
        let range = if text[range.clone()] == *"\r" {
            range.start..b.next_char(range.start).max(range.end + 1).min(text.len())
        } else if text[range.clone()] == *"\n" && range.start > 0 && text.as_bytes()[range.start - 1] == b'\r' {
            range.start - 1..range.end
        } else {
            range
        };
        self.edit(range, "", EditKind::Deleting, None);
        Outcome::Edited
    }

    /// Enter: a line break keeping the current line's indentation.
    fn newline(&mut self) -> Outcome {
        let Some((pane, sel)) = self.caret else { return Outcome::Ignored };
        let b = &self.buffers[pane];
        let line = b.line_of(sel.range().start);
        let indent = b.indent(line);
        let indent = &indent[..indent.len().min(sel.range().start - b.line_range(line).start)];
        let text = format!("{}{}", b.newline(), indent);
        self.insert(&text, EditKind::Other)
    }

    /// Shift+Tab: one indent level less on the selected lines.
    fn unindent(&mut self) -> Outcome {
        let Some((pane, sel)) = self.caret else { return Outcome::Ignored };
        let unit = self.indent_unit();
        let b = &self.buffers[pane];
        let (first, last) = (b.line_of(sel.range().start), b.line_of(sel.range().end));
        let span = b.line_range(first).start..b.line_range(last).end;
        let mut text = String::new();
        for line in first..=last {
            let l = b.line(line);
            let removed = if l.starts_with(unit) {
                unit.len()
            } else if l.starts_with('\t') {
                1
            } else {
                (l.len() - l.trim_start_matches(' ').len()).min(unit.len())
            };
            text.push_str(&l[removed..]);
            if line < last {
                text.push_str(&b.text()[b.line_range(line).end..b.line_range(line + 1).start]);
            }
        }
        let caret = span.start + text.len();
        self.edit(span, &text, EditKind::Other, Some(caret));
        Outcome::Edited
    }

    /// Ctrl+D: duplicates the caret's line below it.
    fn duplicate_line(&mut self) -> Outcome {
        let Some((pane, sel)) = self.caret else { return Outcome::Ignored };
        let b = &self.buffers[pane];
        let line = b.line_of(sel.head);
        let range = b.line_range(line);
        let text = format!("{}{}", b.newline(), b.line(line));
        let caret = range.end + b.newline().len() + (sel.head - range.start);
        self.edit(range.end..range.end, &text, EditKind::Other, Some(caret));
        Outcome::Edited
    }

    /// Ctrl+Y: deletes the caret's line.
    fn delete_line(&mut self) -> Outcome {
        let Some((pane, sel)) = self.caret else { return Outcome::Ignored };
        let b = &self.buffers[pane];
        let line = b.line_of(sel.head);
        let start = b.line_range(line).start;
        let range = if line + 1 < b.line_count() {
            start..b.line_range(line + 1).start
        } else if line > 0 {
            b.line_range(line - 1).end..b.text().len()
        } else {
            0..b.text().len()
        };
        let caret = range.start;
        self.edit(range, "", EditKind::Other, Some(caret));
        Outcome::Edited
    }

    /// Scrolls so that the caret shows; a fold hiding it is reported.
    fn reveal_caret(&mut self, window: &Window, cx: &App) -> Option<usize> {
        let (pane, sel) = self.caret?;
        let line = self.buffers[pane].line_of(sel.head);
        if let LineRow::Fold(id, _) = self.line_row(pane, line)? {
            return Some(id);
        }
        let (caret_x, top) = self.caret_point(pane, sel.head, window, cx)?;
        let (mut x, mut y) = self.scroll[pane];
        let height = self.view_height.get();
        if top < y {
            y = top;
        } else if height > 0. && top + line_height() > y + height {
            y = top + line_height() - height;
        }
        let width = self.text_extent(pane).1 - TEXT_PADDING;
        if caret_x < x {
            x = (caret_x - 40.).max(0.);
        } else if width > 0. && caret_x > x + width - 20. {
            x = caret_x - width + 60.;
        }
        // A caret already in view scrolls nothing: the linked panes keep
        // their places too.
        if (x, y) != self.scroll[pane] {
            self.scroll_to(pane, x, y);
        }
        None
    }

    // Text input (IME), on the editable pane.

    fn editable_buffer(&self) -> Option<&Buffer> {
        self.editable.map(|p| &self.buffers[p])
    }

    pub fn text_for_range(&self, range: Range<usize>, adjusted: &mut Option<Range<usize>>) -> Option<String> {
        let b = self.editable_buffer()?;
        let (start, end) = (b.utf16_to_offset(range.start), b.utf16_to_offset(range.end));
        *adjusted = Some(b.offset_to_utf16(start)..b.offset_to_utf16(end));
        Some(b.text()[start..end].to_owned())
    }

    pub fn selected_text_range(&self) -> Option<UTF16Selection> {
        let (pane, sel) = self.caret?;
        let b = &self.buffers[pane];
        let range = sel.range();
        Some(UTF16Selection { range: b.offset_to_utf16(range.start)..b.offset_to_utf16(range.end), reversed: sel.head < sel.anchor })
    }

    pub fn marked_text_range(&self) -> Option<Range<usize>> {
        let b = self.editable_buffer()?;
        self.marked.as_ref().map(|r| b.offset_to_utf16(r.start)..b.offset_to_utf16(r.end))
    }

    pub fn unmark_text(&mut self) {
        self.marked = None;
    }

    fn input_range(&self, range: Option<Range<usize>>) -> Option<Range<usize>> {
        let b = self.editable_buffer()?;
        Some(
            range
                .map(|r| b.utf16_to_offset(r.start)..b.utf16_to_offset(r.end))
                .or(self.marked.clone())
                .unwrap_or_else(|| self.caret.map(|c| c.1.range()).unwrap_or(0..0)),
        )
    }

    pub fn replace_text_in_range(&mut self, range: Option<Range<usize>>, text: &str) -> Outcome {
        if !self.caret_editable() {
            return Outcome::Ignored;
        }
        let Some(range) = self.input_range(range) else { return Outcome::Ignored };
        self.marked = None;
        let kind = if range.is_empty() && !text.chars().any(char::is_whitespace) { EditKind::Typing } else { EditKind::Other };
        self.edit(range, text, kind, None);
        Outcome::Edited
    }

    pub fn replace_and_mark_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, selected: Option<Range<usize>>) -> Outcome {
        if !self.caret_editable() {
            return Outcome::Ignored;
        }
        let Some(range) = self.input_range(range) else { return Outcome::Ignored };
        self.edit(range.clone(), text, EditKind::Typing, None);
        self.marked = (!text.is_empty()).then(|| range.start..range.start + text.len());
        if let (Some(selected), Some(pane)) = (selected, self.editable) {
            // `selected` counts UTF-16 units of the new text.
            let to_byte = |units: usize| {
                let mut n = 0;
                for (i, c) in text.char_indices() {
                    if n >= units {
                        return i;
                    }
                    n += c.len_utf16();
                }
                text.len()
            };
            self.caret = Some((pane, Selection { anchor: range.start + to_byte(selected.start), head: range.start + to_byte(selected.end) }));
        }
        Outcome::Edited
    }

    pub fn bounds_for_range(&self, range: Range<usize>, window: &Window, cx: &App) -> Option<Bounds<Pixels>> {
        let pane = self.editable?;
        let offset = self.buffers[pane].utf16_to_offset(range.start);
        let (cx_, cy) = self.caret_point(pane, offset, window, cx)?;
        let b = self.bounds.borrow()[pane];
        let (sx, sy) = self.scroll[pane];
        let x = b.origin.x + px(self.layouts[pane].text_left() + cx_ - sx);
        let y = b.origin.y + px(cy - sy);
        Some(Bounds::new(point(x, y), size(px(2.), px(line_height()))))
    }

    pub fn character_index_for_point(&self, position: gpui_kit::Point<Pixels>, window: &Window, cx: &App) -> Option<usize> {
        match self.hit(position, window, cx)? {
            (pane, Ok(offset)) if Some(pane) == self.editable => Some(self.buffers[pane].offset_to_utf16(offset)),
            _ => None,
        }
    }
}

/// A view built from text panes.
pub trait PaneHost: Sized + gpui_kit::EntityInputHandler + 'static {
    type Extra: Clone + Default + 'static;
    fn panes(&mut self) -> &mut TextPanes<Self::Extra>;
    fn text_panes(&self) -> &TextPanes<Self::Extra>;
    /// The editable text changed: re-diff, re-highlight, save, ….
    fn edited(&mut self, window: &mut Window, cx: &mut Context<Self>);
    /// A collapsed fragment was clicked, or the caret went into one.
    fn open_fold(&mut self, _id: usize, _cx: &mut Context<Self>) {}
}

/// Applies an event's outcome: tells the host about edits and folds, and
/// keeps the caret in view.
pub fn settle<V: PaneHost>(view: &mut V, outcome: Outcome, reveal: bool, window: &mut Window, cx: &mut Context<V>) {
    match outcome {
        Outcome::Ignored => return,
        Outcome::OpenFold(id) => view.open_fold(id, cx),
        Outcome::Edited => view.edited(window, cx),
        Outcome::Moved => {}
    }
    if reveal {
        // Opening the caret's fold changes the rows; then scroll again.
        for _ in 0..2 {
            match view.panes().reveal_caret(window, cx) {
                Some(fold) => view.open_fold(fold, cx),
                None => break,
            }
        }
    }
    cx.notify();
}

/// The panes' container: focus, keys, Paste and the Ctrl state.
pub fn pane_area<V: PaneHost>(id: &'static str, focus: &FocusHandle, cx: &mut Context<V>) -> gpui_kit::Stateful<gpui_kit::Div> {
    h_flex()
        .id(id)
        .key_context(PANE_CONTEXT)
        .track_focus(focus)
        .on_key_down(cx.listener(|view: &mut V, e: &KeyDownEvent, window, cx| {
            let outcome = view.panes().key_down(e, cx);
            if outcome != Outcome::Ignored {
                cx.stop_propagation();
            }
            settle(view, outcome, true, window, cx);
        }))
        .on_action(cx.listener(|view: &mut V, _: &Paste, window, cx| {
            let outcome = view.panes().paste(cx);
            settle(view, outcome, true, window, cx);
        }))
        .on_modifiers_changed(cx.listener(|view: &mut V, e: &ModifiersChangedEvent, _, cx| {
            let panes = view.panes();
            if panes.ctrl_held != e.modifiers.secondary() {
                panes.ctrl_held = e.modifiers.secondary();
                cx.notify();
            }
        }))
        // Mouse moves carry the modifiers too, also while another window
        // has the keyboard.
        .on_mouse_move(cx.listener(|view: &mut V, e: &MouseMoveEvent, _, cx| {
            let panes = view.panes();
            if panes.ctrl_held != e.modifiers.secondary() {
                panes.ctrl_held = e.modifiers.secondary();
                cx.notify();
            }
        }))
        .flex_1()
        .min_h_0()
        .w_full()
}

/// What the host gives a pane to paint.
pub struct PaneContent {
    /// Looks for the visible rows, from `visible_rows(pane).start`.
    pub looks: Vec<RowLook>,
    /// Gutter buttons, insertion lines and the like, positioned by the host.
    pub overlays: Vec<AnyElement>,
}

impl<T: Clone + Default + 'static> TextPanes<T> {
    /// The rows a pane paints.
    pub fn visible_rows(&self, pane: usize) -> Range<usize> {
        let height = self.view_height.get();
        let visible = if height > 0. { height } else { 1600. };
        let rows = self.rows[pane].len();
        let scroll_y = self.scroll[pane].1;
        if rows == 0 {
            return 0..0;
        }
        let first = self.row_at(pane, scroll_y);
        let last = (self.row_at(pane, scroll_y + visible) + 2).min(rows);
        first..last
    }

    /// A row's top, relative to its pane.
    pub fn row_top(&self, pane: usize, row: usize) -> f32 {
        self.row_y(pane, row) - self.scroll[pane].1
    }

    /// A row's height (taller when wrapped), without the block below it.
    pub fn row_height(&self, pane: usize, row: usize) -> f32 {
        let block = self.blocks[pane].get(&row).copied().unwrap_or(0.);
        (self.row_y(pane, row + 1) - self.row_y(pane, row) - block).max(line_height())
    }

    /// Reserves space below rows for the host to fill (inline review
    /// threads); a change re-lays the pane out.
    pub fn set_blocks(&mut self, pane: usize, blocks: std::collections::HashMap<usize, f32>) {
        if self.blocks[pane] != blocks {
            self.blocks[pane] = blocks;
            self.relayout_pane(pane);
            self.layout_dirty = true;
        }
    }

    /// Each row's top before scrolling, and the content height after them,
    /// for painting connectors between panes.
    pub fn tops(&self, pane: usize) -> &[f32] {
        &self.tops[pane]
    }

    /// Paints one pane: rows with syntax colors, selection and the caret.
    pub fn render_pane<V: PaneHost<Extra = T>>(
        &self,
        pane: usize,
        content: PaneContent,
        palette: &Palette,
        window: &mut Window,
        cx: &mut Context<V>,
    ) -> AnyElement {
        let entity = cx.entity();
        let focused = self.focus.is_focused(window);
        let layout = self.layouts[pane];
        let (scroll_x, _) = self.scroll[pane];
        let theme = cx.theme().highlight_theme.clone();
        let selection_color = cx.theme().selection;
        let selection = self.caret.filter(|c| c.0 == pane && !c.1.is_empty()).map(|c| c.1.range());
        let mut children: Vec<AnyElement> = Vec::new();

        children.push(self.pane_input_canvas(pane, focused, &entity));

        let range = self.visible_rows(pane);
        let char_width = f32::from(shape("        ", window, cx).width) / 8.;
        if char_width > 0. {
            self.char_width.set(char_width);
        }
        let unit = if self.indent_unit() == "\t" { TAB_WIDTH } else { self.indent_unit().len() };
        let mut guides: Vec<AnyElement> = Vec::new();
        let mut edges: Vec<AnyElement> = Vec::new();
        for (ix, look) in range.clone().zip(content.looks) {
            let top = self.row_top(pane, ix);
            let height = self.row_height(pane, ix);
            let row = div().absolute().left_0().right_0().top(px(top)).h(px(height));
            match self.rows[pane][ix] {
                RowTarget::Fold { id, count } => {
                    children.push(
                        row.id(("pane-fold", pane * 10_000_000 + ix))
                            .flex()
                            .items_center()
                            .gap_1()
                            .bg(palette.diff_header)
                            .text_color(palette.text_secondary)
                            .cursor_pointer()
                            .hover(|st| st.text_color(palette.text))
                            .map(|el| if layout.mirrored { el.pl_2() } else { el.pl(px(layout.text_left() - TEXT_PADDING + 4.)) })
                            .on_click(cx.listener(move |view: &mut V, _, window, cx| settle(view, Outcome::OpenFold(id), false, window, cx)))
                            .child(common::icon(gpui_kit::assets::IconName::ChevronRight))
                            .child(format!("{count} unchanged lines"))
                            .into_any_element(),
                    );
                }
                RowTarget::Filler => {}
                target @ (RowTarget::Line(_) | RowTarget::Other { .. }) => {
                    let (source, line, own) = match target {
                        RowTarget::Other { pane: p, line } if p < self.buffers.len() && line < self.buffers[p].line_count() => (p, line, false),
                        RowTarget::Line(line) => (pane, line, true),
                        _ => continue,
                    };
                    let buffer = &self.buffers[source];
                    let highlighter = self.highlighters[source].as_ref();
                    let line_range = buffer.line_range(line);
                    let text = buffer.line(line);
                    let syntax: Vec<(Range<usize>, HighlightStyle)> = highlighter
                        .map(|h| {
                            h.styles(&line_range, &*theme)
                                .into_iter()
                                .filter(|(_, st)| *st != HighlightStyle::default())
                                .map(|(r, st)| (r.start - line_range.start..r.end - line_range.start, st))
                                .collect()
                        })
                        .unwrap_or_default();
                    let mut backgrounds = look.words;
                    if let Some(sel) = selection.as_ref().filter(|_| own) {
                        let (a, b) = (sel.start.max(line_range.start), sel.end.min(line_range.end));
                        if a < b {
                            backgrounds.push((a - line_range.start..b - line_range.start, selection_color));
                        }
                    }
                    let wraps = self.row_wraps(pane, ix);
                    let text_el = if wraps.len() > 1 {
                        // Soft-wrapped: one visual line per part, each with its
                        // share of the colors.
                        let clip = |start: usize, end: usize, r: &Range<usize>| r.start.max(start).min(end) - start..r.end.min(end).max(start) - start;
                        let indent = wrap_indent(text, self.wrap_cols[pane]) as f32 * char_width;
                        let mut parts = gpui_kit::component::v_flex().flex_1().min_w_0();
                        for (i, &start) in wraps.iter().enumerate() {
                            let end = wraps.get(i + 1).copied().unwrap_or(text.len());
                            let syntax: Vec<_> = syntax.iter().map(|(r, st)| (clip(start, end, r), *st)).filter(|(r, _)| !r.is_empty()).collect();
                            let backgrounds: Vec<_> = backgrounds.iter().map(|(r, c)| (clip(start, end, r), *c)).filter(|(r, _)| !r.is_empty()).collect();
                            parts = parts.child(
                                div()
                                    .h(px(line_height()))
                                    .flex()
                                    .when(i > 0, |el| el.pl(px(indent)))
                                    .child(pane_text(&text[start..end], &syntax, &backgrounds, 0., self.show_whitespace, palette)),
                            );
                        }
                        parts.into_any_element()
                    } else {
                        pane_text(text, &syntax, &backgrounds, scroll_x, self.show_whitespace, palette).into_any_element()
                    };
                    if self.indent_guides {
                        let text_left = layout.text_left() - scroll_x;
                        let indent = guide_indent(buffer, line);
                        let mut column = unit;
                        while column < indent {
                            let x = text_left + column as f32 * char_width;
                            if x >= layout.text_left() - TEXT_PADDING {
                                guides.push(div().absolute().top(px(top)).h(px(line_height())).left(px(x)).w(px(1.)).bg(palette.border).into_any_element());
                            }
                            column += unit;
                        }
                    }
                    let number = look.number.unwrap_or_else(|| {
                        div()
                            .w(px(layout.numbers_width()))
                            .h_full()
                            .flex_shrink_0()
                            .text_right()
                            .map(|el| if layout.mirrored { el.pr_1() } else { el.pr(px(4. + layout.inline_buttons)) })
                            .text_color(palette.text_disabled)
                            .when(!layout.hide_numbers, |el| el.child((line + 1).to_string()))
                            .into_any_element()
                    });
                    let marker = div().w(px(MARKER_WIDTH)).h_full().flex_shrink_0().when_some(look.marker, |el, c| el.bg(c));
                    let spacer = div().w(px(layout.buttons)).flex_shrink_0();
                    // The change color covers the text only, not the gutter.
                    let text_el = div().flex_1().min_w_0().h_full().flex().when_some(look.background, |el, bg| el.bg(bg)).child(text_el);
                    for (edge, at_top) in [(look.top, true), (look.bottom, false)] {
                        if let Some(color) = edge {
                            let y = if at_top { top } else { top + height - 1. };
                            edges.push(div().absolute().left_0().right_0().top(px(y)).h(px(1.)).bg(color).into_any_element());
                        }
                    }
                    children.push(
                        row.flex()
                            .when_some(look.gutter, |el, bg| el.bg(bg))
                            .map(|el| {
                                if layout.mirrored {
                                    el.child(text_el).child(spacer).child(number).child(marker)
                                } else {
                                    el.child(marker).child(number).child(spacer).child(text_el)
                                }
                            })
                            .into_any_element(),
                    );
                }
            }
        }
        children.extend(guides);
        children.extend(edges);
        // The line between the text and the gutter.
        let gutter = layout.gutter_width();
        children.push(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .w(px(1.))
                .bg(palette.border)
                .map(|el| if layout.mirrored { el.right(px(gutter)) } else { el.left(px(gutter)) })
                .into_any_element(),
        );
        children.extend(content.overlays);

        children.extend(self.pane_hscrollbar(pane, layout, scroll_x, palette, cx));

        children.extend(self.pane_caret(pane, focused, layout, scroll_x, palette, window, cx));

        let weight = self.weights.get(pane).copied().unwrap_or(1.);
        div()
            .id(("text-pane", pane))
            .relative()
            .flex_basis(px(0.))
            .flex_shrink(1.)
            .map(|mut el| {
                el.style().flex_grow = Some(weight);
                el
            })
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .font_family(cx.theme().mono_font_family.clone())
            .text_size(px(font_size()))
            .cursor(gpui_kit::CursorStyle::IBeam)
            .on_scroll_wheel(cx.listener(move |view: &mut V, e: &ScrollWheelEvent, _, cx| {
                view.panes().on_wheel(pane, e);
                cx.stop_propagation();
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view: &mut V, e: &MouseDownEvent, window, cx| {
                    let outcome = view.panes().mouse_down(e, window, cx);
                    settle(view, outcome, false, window, cx);
                }),
            )
            .on_mouse_up(MouseButton::Left, cx.listener(|view: &mut V, _, _, _| view.panes().selecting = false))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|view: &mut V, e: &MouseDownEvent, window, cx| {
                    let outcome = view.panes().right_down(e, window, cx);
                    settle(view, outcome, false, window, cx);
                }),
            )
            .children(children)
            .into_any_element()
    }

    /// Measures the pane; takes text input while it has the caret; lets a
    /// drag selection follow the mouse anywhere in the window.
    fn pane_input_canvas<V: PaneHost<Extra = T>>(&self, pane: usize, focused: bool, entity: &gpui_kit::Entity<V>) -> AnyElement {
        let bounds_cell = self.bounds.clone();
        let measured = self.view_height.clone();
        let notify = entity.clone();
        let input = (focused && self.caret.is_some_and(|c| c.0 == pane)).then(|| (self.focus.clone(), entity.clone()));
        let primary = pane == self.primary;
        let wrapping = self.soft_wrap;
        let selecting = (primary && self.selecting).then(|| entity.clone());
        let dragging = (primary && self.drag.is_some()).then(|| entity.clone());
        canvas(
            move |bounds, _, cx| {
                let old = std::mem::replace(&mut bounds_cell.borrow_mut()[pane], bounds);
                let h = f32::from(bounds.size.height);
                if wrapping && (old.size.width - bounds.size.width).abs() > px(0.5) {
                    let notify = notify.clone();
                    cx.defer(move |cx| notify.update(cx, |_, cx| cx.notify()));
                }
                if primary && (measured.get() - h).abs() > 0.5 {
                    measured.set(h);
                    // Paint again with the rows the new height shows.
                    cx.defer(move |cx| notify.update(cx, |_, cx| cx.notify()));
                }
            },
            move |bounds, _, window, cx| {
                if let Some((focus, entity)) = input {
                    window.handle_input(&focus, ElementInputHandler::new(bounds, entity), cx);
                }
                if let Some(entity) = dragging {
                    let up = entity.clone();
                    window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                        if phase == DispatchPhase::Bubble {
                            entity.update(cx, |view, cx| {
                                if view.panes().drag_move(e) {
                                    cx.notify();
                                }
                            });
                        }
                    });
                    window.on_mouse_event(move |_: &gpui_kit::MouseUpEvent, phase, _, cx| {
                        if phase == DispatchPhase::Bubble {
                            up.update(cx, |view, cx| {
                                view.panes().drag = None;
                                cx.notify();
                            });
                        }
                    });
                }
                if let Some(entity) = selecting {
                    window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, cx| {
                        if phase == DispatchPhase::Bubble {
                            entity.update(cx, |view, cx| {
                                let outcome = view.panes().mouse_move(e, window, cx);
                                settle(view, outcome, false, window, cx);
                            });
                        }
                    });
                }
            },
        )
        .absolute()
        .size_full()
        .into_any_element()
    }

    /// The horizontal scrollbar under the text, when lines are wider.
    fn pane_hscrollbar<V: PaneHost<Extra = T>>(&self, pane: usize, layout: PaneLayout, scroll_x: f32, palette: &Palette, cx: &mut Context<V>) -> Option<AnyElement> {
        let (text_w, view_w) = self.text_extent(pane);
        if self.soft_wrap || view_w <= 0. || text_w <= view_w + 1. {
            return None;
        }
        let thumb = (view_w * view_w / text_w).max(30.);
        let at = scroll_x / (text_w - view_w) * (view_w - thumb);
        let track_left = if layout.mirrored { 0. } else { layout.gutter_width() };
        let thumb_color = palette.text_disabled.opacity(0.45);
        Some(
            div()
                .id(("pane-hscroll", pane))
                .absolute()
                .bottom_0()
                .left(px(track_left))
                .w(px(view_w))
                .h(px(HSCROLL_HEIGHT))
                .cursor_default()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |view: &mut V, e: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        let panes = view.panes();
                        let origin = f32::from(panes.bounds.borrow()[pane].origin.x) + track_left;
                        let x = f32::from(e.position.x) - origin;
                        let (sx, sy) = panes.scroll[pane];
                        // On the track: page towards the click; on the thumb: drag.
                        let target = if x < at { sx - view_w } else if x > at + thumb { sx + view_w } else { sx };
                        panes.scroll_to(pane, target, sy);
                        panes.drag = Some(Drag::HScroll { pane, start_x: f32::from(e.position.x), start_scroll: panes.scroll[pane].0 });
                        cx.notify();
                    }),
                )
                .child(div().absolute().top(px(2.)).left(px(at)).w(px(thumb)).h(px(HSCROLL_HEIGHT - 4.)).rounded_full().bg(thumb_color))
                .into_any_element(),
        )
    }

    /// The caret, while this pane has focus and it is in view.
    fn pane_caret<V: PaneHost<Extra = T>>(&self, pane: usize, focused: bool, layout: PaneLayout, scroll_x: f32, palette: &Palette, window: &mut Window, cx: &mut Context<V>) -> Option<AnyElement> {
        let (_, sel) = self.caret.filter(|c| c.0 == pane && focused)?;
        let (caret_x, caret_y) = self.caret_point(pane, sel.head, window, cx)?;
        let x = layout.text_left() + caret_x - scroll_x;
        let left_edge = layout.text_left() - TEXT_PADDING;
        if x < left_edge {
            return None;
        }
        let y = caret_y - self.scroll[pane].1;
        Some(div().absolute().top(px(y)).left(px(x)).w(px(2.)).h(px(line_height())).bg(palette.text).into_any_element())
    }

    /// Error stripe: each mark's place in the whole pane, the visible part
    /// as a thumb; click or drag to scroll there.
    pub fn render_stripe<V: PaneHost<Extra = T>>(&self, pane: usize, marks: Vec<(Range<usize>, Hsla)>, thumb: Hsla, cx: &mut Context<V>) -> AnyElement {
        let content = self.content_height(pane);
        let marks: Vec<(f32, f32, Hsla)> =
            marks.into_iter().map(|(rows, c)| (self.row_y(pane, rows.start), self.row_y(pane, rows.end) - self.row_y(pane, rows.start), c)).collect();
        let view = self.view_height.get();
        let scroll = self.scroll[pane].1;
        let cell = self.stripe_bounds.clone();
        div()
            .id(("pane-stripe", pane))
            .w(px(STRIPE_WIDTH))
            .h_full()
            .flex_shrink_0()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view: &mut V, e: &MouseDownEvent, _, cx| {
                    let panes = view.panes();
                    panes.stripe_drag = Some(pane);
                    panes.stripe_seek(pane, e.position.y);
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(move |view: &mut V, e: &MouseMoveEvent, _, cx| {
                let panes = view.panes();
                if panes.stripe_drag == Some(pane) && e.pressed_button == Some(MouseButton::Left) {
                    panes.stripe_seek(pane, e.position.y);
                    cx.notify();
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|view: &mut V, _, _, _| view.panes().stripe_drag = None))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|view: &mut V, _, _, _| view.panes().stripe_drag = None))
            .child(
                canvas(
                    move |bounds, _, _| cell.borrow_mut()[pane] = bounds,
                    move |bounds, _, window, _| {
                        let h = f32::from(bounds.size.height);
                        let total = (content + view / 2.).max(h).max(1.);
                        let scale = h / total;
                        let x = bounds.origin.x + px(2.);
                        let w = bounds.size.width - px(4.);
                        for (y, h, color) in &marks {
                            let top = bounds.origin.y + px(y * scale);
                            let height = px((h * scale).max(2.));
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
            .into_any_element()
    }
}

/// Implements gpui's text input for a [`PaneHost`] by forwarding to its panes.
#[macro_export]
macro_rules! impl_pane_input {
    ($view:ty) => {
        impl gpui_kit::EntityInputHandler for $view {
            fn text_for_range(
                &mut self,
                range: std::ops::Range<usize>,
                adjusted: &mut Option<std::ops::Range<usize>>,
                _: &mut gpui_kit::Window,
                _: &mut gpui_kit::Context<Self>,
            ) -> Option<String> {
                use $crate::ui::text_panes::PaneHost as _;
                self.panes().text_for_range(range, adjusted)
            }

            fn selected_text_range(&mut self, _: bool, _: &mut gpui_kit::Window, _: &mut gpui_kit::Context<Self>) -> Option<gpui_kit::UTF16Selection> {
                use $crate::ui::text_panes::PaneHost as _;
                self.panes().selected_text_range()
            }

            fn marked_text_range(&self, _: &mut gpui_kit::Window, _: &mut gpui_kit::Context<Self>) -> Option<std::ops::Range<usize>> {
                use $crate::ui::text_panes::PaneHost as _;
                self.text_panes().marked_text_range()
            }

            fn unmark_text(&mut self, _: &mut gpui_kit::Window, _: &mut gpui_kit::Context<Self>) {
                use $crate::ui::text_panes::PaneHost as _;
                self.panes().unmark_text();
            }

            fn replace_text_in_range(&mut self, range: Option<std::ops::Range<usize>>, text: &str, window: &mut gpui_kit::Window, cx: &mut gpui_kit::Context<Self>) {
                use $crate::ui::text_panes::PaneHost as _;
                let outcome = self.panes().replace_text_in_range(range, text);
                $crate::ui::text_panes::settle(self, outcome, true, window, cx);
            }

            fn replace_and_mark_text_in_range(
                &mut self,
                range: Option<std::ops::Range<usize>>,
                text: &str,
                selected: Option<std::ops::Range<usize>>,
                window: &mut gpui_kit::Window,
                cx: &mut gpui_kit::Context<Self>,
            ) {
                use $crate::ui::text_panes::PaneHost as _;
                let outcome = self.panes().replace_and_mark_text_in_range(range, text, selected);
                $crate::ui::text_panes::settle(self, outcome, true, window, cx);
            }

            fn bounds_for_range(
                &mut self,
                range: std::ops::Range<usize>,
                _: gpui_kit::Bounds<gpui_kit::Pixels>,
                window: &mut gpui_kit::Window,
                cx: &mut gpui_kit::Context<Self>,
            ) -> Option<gpui_kit::Bounds<gpui_kit::Pixels>> {
                use $crate::ui::text_panes::PaneHost as _;
                self.text_panes().bounds_for_range(range, window, cx)
            }

            fn character_index_for_point(
                &mut self,
                position: gpui_kit::Point<gpui_kit::Pixels>,
                window: &mut gpui_kit::Window,
                cx: &mut gpui_kit::Context<Self>,
            ) -> Option<usize> {
                use $crate::ui::text_panes::PaneHost as _;
                self.text_panes().character_index_for_point(position, window, cx)
            }
        }
    };
}

impl<T: Clone + Default> TextPanes<T> {
    /// Takes the gear menu's view settings (line numbers, whitespace,
    /// indent guides).
    pub fn apply_settings(&mut self, cx: &App) {
        let settings = &crate::settings::Settings::get(cx).diff;
        self.show_whitespace = settings.show_whitespaces;
        self.indent_guides = settings.show_indent_guides;
        if self.soft_wrap != settings.soft_wrap {
            self.soft_wrap = settings.soft_wrap;
            self.layout_dirty = true;
        }
        for (pane, layout) in self.layouts.iter_mut().enumerate() {
            layout.hide_numbers = !settings.show_line_numbers;
            if !layout.double_numbers {
                layout.digits = self.buffers.get(pane).map_or(0, |b| b.line_count().max(1).to_string().len());
            }
        }
        self.relayout();
    }
}

/// The editor part of a pane's context menu: Cut, Copy, Paste, Select All.
pub fn edit_menu<V: PaneHost>(menu: gpui_kit::component::menu::PopupMenu, entity: &gpui_kit::Entity<V>, cx: &App) -> gpui_kit::component::menu::PopupMenu {
    use gpui_kit::component::menu::PopupMenuItem;
    let panes = entity.read(cx).text_panes();
    let selected = panes.caret.is_some_and(|c| !c.1.is_empty());
    let editable = panes.caret_editable();
    let run = |f: fn(&mut TextPanes<V::Extra>, &mut App) -> Outcome| {
        let entity = entity.clone();
        move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| {
            entity.update(cx, |view, cx| {
                let outcome = f(view.panes(), cx);
                settle(view, outcome, false, window, cx);
            })
        }
    };
    menu.item(PopupMenuItem::new("Cut").disabled(!(selected && editable)).on_click(run(|p, cx| p.copy(true, cx))))
        .item(PopupMenuItem::new("Copy").disabled(!selected).on_click(run(|p, cx| p.copy(false, cx))))
        .item(PopupMenuItem::new("Paste").disabled(!editable).on_click(run(|p, cx| p.paste(cx))))
        .item(PopupMenuItem::new("Select All").on_click(run(|p, _| p.select_all())))
}

/// The diff and merge viewers' gear menu. `align` offers Align Changes
/// (side-by-side only).
pub fn gear_menu(
    menu: gpui_kit::component::menu::PopupMenu,
    align: bool,
    window: &mut Window,
    cx: &mut Context<gpui_kit::component::menu::PopupMenu>,
) -> gpui_kit::component::menu::PopupMenu {
    use crate::settings::{DiffSettings, Settings};
    use gpui_kit::component::menu::PopupMenuItem;
    let settings = Settings::get(cx).diff.clone();
    let toggle = |label: &'static str, on: bool, set: fn(&mut DiffSettings, bool)| {
        PopupMenuItem::new(label).checked(on).on_click(move |_, _, cx| Settings::update(cx, |s| set(&mut s.diff, !on)))
    };
    let context = settings.context_lines;
    let mut menu = menu
        .submenu("Context Lines", window, cx, move |mut menu, _, _| {
            for n in [1, 2, 3, 4, 5, 8, 10, 15] {
                menu = menu.item(
                    PopupMenuItem::new(if n == 1 { "1 line".to_owned() } else { format!("{n} lines") })
                        .checked(context == n)
                        .on_click(move |_, _, cx| Settings::update(cx, |s| s.diff.context_lines = n)),
                );
            }
            menu
        })
        .separator()
        .item(toggle("Show Line Numbers", settings.show_line_numbers, |s, v| s.show_line_numbers = v))
        .item(toggle("Show Whitespaces", settings.show_whitespaces, |s, v| s.show_whitespaces = v))
        .item(toggle("Show Indent Guides", settings.show_indent_guides, |s, v| s.show_indent_guides = v))
        .item(toggle("Soft-Wrap", settings.soft_wrap, |s, v| s.soft_wrap = v));
    if align {
        menu = menu.separator().item(toggle("Align Changes", settings.align_changes, |s, v| s.align_changes = v));
    }
    menu
}

/// Expands tabs to spaces, moving ranges along with the text.
pub fn expand_tabs(text: &str, ranges: &[Range<usize>]) -> (String, Vec<Range<usize>>) {
    let (out, ranges, _) = display_text(text, ranges, false);
    (out, ranges)
}

/// The text as painted: tabs expanded and, with `whitespace`, spaces shown
/// as `·` and tabs as `→`. Returns the moved ranges and where the
/// whitespace marks are.
fn display_text(text: &str, ranges: &[Range<usize>], whitespace: bool) -> (String, Vec<Range<usize>>, Vec<Range<usize>>) {
    if !text.contains('\t') && !(whitespace && text.contains(' ')) {
        return (text.to_owned(), ranges.to_vec(), Vec::new());
    }
    let mut out = String::with_capacity(text.len() + 16);
    let mut map = Vec::with_capacity(text.len() + 1);
    let mut marks: Vec<Range<usize>> = Vec::new();
    let mut column = 0;
    let mut mark = |out: &mut String, s: &str| {
        let start = out.len();
        out.push_str(s);
        match marks.last_mut() {
            Some(last) if last.end == start => last.end = out.len(),
            _ => marks.push(start..out.len()),
        }
    };
    for ch in text.chars() {
        for _ in 0..ch.len_utf8() {
            map.push(out.len());
        }
        if ch == '\t' {
            let spaces = TAB_WIDTH - column % TAB_WIDTH;
            if whitespace {
                mark(&mut out, "→");
                out.extend(std::iter::repeat_n(' ', spaces - 1));
            } else {
                out.extend(std::iter::repeat_n(' ', spaces));
            }
            column += spaces;
        } else if ch == ' ' && whitespace {
            mark(&mut out, "·");
            column += 1;
        } else {
            out.push(ch);
            column += 1;
        }
    }
    map.push(out.len());
    let ranges = ranges.iter().map(|r| map[r.start.min(text.len())]..map[r.end.min(text.len())]).collect();
    (out, ranges, marks)
}

/// The indent (in columns) indent guides go up to: a blank line takes the
/// smaller of its neighbors', so guides run through it.
fn guide_indent(buffer: &Buffer, line: usize) -> usize {
    let columns = |l: usize| {
        let text = buffer.line(l);
        (!text.trim().is_empty()).then(|| display_offset(text, buffer.indent(l).len()))
    };
    if let Some(c) = columns(line) {
        return c;
    }
    let up = (line.saturating_sub(100)..line).rev().find_map(columns);
    let down = (line + 1..(line + 100).min(buffer.line_count())).find_map(columns);
    up.unwrap_or(0).min(down.unwrap_or(0))
}

/// Where soft wrap breaks a line `cols` columns wide: byte offsets of each
/// visual line's start (the first is 0). Breaks after a space when the
/// visual line has one, as IntelliJ does, else mid-word.
fn wrap_starts(text: &str, cols: usize) -> Vec<usize> {
    let mut starts = vec![0];
    let (mut col, mut space, mut limit) = (0, None, cols);
    for (i, c) in text.char_indices() {
        let width = if c == '\t' { TAB_WIDTH } else { 1 };
        if col + width > limit && col > 0 {
            // The wrapped parts keep the line's indent.
            limit = cols - wrap_indent(text, cols);
            let start = *starts.last().unwrap_or(&0);
            let at = space.filter(|s| *s > start).unwrap_or(i);
            starts.push(at);
            col = text[at..i].chars().map(|c| if c == '\t' { TAB_WIDTH } else { 1 }).sum();
            space = None;
        }
        col += width;
        if c == ' ' || c == '\t' {
            space = Some(i + c.len_utf8());
        }
    }
    starts
}

/// The columns a wrapped line's later parts are indented by: the line's own
/// indent (IntelliJ's "Use original line's indent for wrapped parts"), at
/// most half the width.
fn wrap_indent(text: &str, cols: usize) -> usize {
    let indent = text.len() - text.trim_start_matches([' ', '\t']).len();
    display_offset(text, indent).min(cols / 2)
}

/// Byte offset in a line to its offset in the tab-expanded display text.
pub fn display_offset(text: &str, byte: usize) -> usize {
    expand_tabs(text, &[0..byte]).1[0].end
}

/// The reverse: a display offset (from hit testing) back to a byte offset.
fn byte_offset(text: &str, display: usize) -> usize {
    let mut best = 0;
    for (i, _) in text.char_indices().chain(std::iter::once((text.len(), ' '))) {
        if display_offset(text, i) <= display {
            best = i;
        } else {
            break;
        }
    }
    best
}

fn shape(text: &str, window: &Window, cx: &App) -> gpui_kit::ShapedLine {
    let run = TextRun {
        len: text.len(),
        font: font(cx.theme().mono_font_family.clone()),
        color: gpui_kit::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window.text_system().shape_line(SharedString::from(text.to_owned()), px(font_size()), &[run], None)
}

/// Lays background ranges (later ones win) over syntax styles, giving the
/// sorted, non-overlapping runs StyledText needs.
pub fn merge_styles(syntax: &[(Range<usize>, HighlightStyle)], backgrounds: &[(Range<usize>, Hsla)], len: usize) -> Vec<(Range<usize>, HighlightStyle)> {
    let mut cuts: Vec<usize> = vec![0, len];
    for r in syntax.iter().map(|(r, _)| r).chain(backgrounds.iter().map(|(r, _)| r)) {
        cuts.extend([r.start.min(len), r.end.min(len)]);
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut out: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    for pair in cuts.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let mut style = syntax.iter().rev().find(|(r, _)| r.start <= a && b <= r.end).map(|(_, s)| *s).unwrap_or_default();
        if let Some((_, bg)) = backgrounds.iter().rev().find(|(r, _)| r.start <= a && b <= r.end) {
            style.background_color = Some(*bg);
        }
        if style == HighlightStyle::default() {
            continue;
        }
        match out.last_mut() {
            Some((r, last)) if r.end == a && *last == style => r.end = b,
            _ => out.push((a..b, style)),
        }
    }
    out
}

/// A line's text, scrolled horizontally, with syntax colors and backgrounds.
fn pane_text(
    text: &str,
    syntax: &[(Range<usize>, HighlightStyle)],
    backgrounds: &[(Range<usize>, Hsla)],
    scroll_x: f32,
    whitespace: bool,
    palette: &Palette,
) -> impl IntoElement {
    let all: Vec<Range<usize>> = syntax.iter().map(|(r, _)| r.clone()).chain(backgrounds.iter().map(|(r, _)| r.clone())).collect();
    let (display, mapped, marks) = display_text(text, &all, whitespace);
    let faint = HighlightStyle { color: Some(palette.text_disabled), ..Default::default() };
    let n = syntax.len();
    let syntax: Vec<_> = mapped[..n]
        .iter()
        .cloned()
        .zip(syntax.iter().map(|(_, st)| *st))
        .chain(marks.into_iter().map(|r| (r, faint)))
        .collect();
    let backgrounds: Vec<_> = mapped[n..].iter().cloned().zip(backgrounds.iter().map(|(_, c)| *c)).filter(|(r, _)| !r.is_empty()).collect();
    let highlights = merge_styles(&syntax, &backgrounds, display.len());
    div().flex_1().min_w_0().h_full().overflow_hidden().child(
        div()
            .relative()
            .left(px(-scroll_x))
            .pl(px(TEXT_PADDING))
            .whitespace_nowrap()
            .text_color(palette.text)
            .child(StyledText::new(display).with_highlights(highlights)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soft_wrap_breaks_after_spaces_and_keeps_indent() {
        let text = "    let value = call(alpha, beta, gamma);";
        let starts = wrap_starts(text, 20);
        let parts: Vec<&str> = starts.iter().zip(starts.iter().skip(1).chain([&text.len()])).map(|(a, b)| &text[*a..*b]).collect();
        assert_eq!(parts, ["    let value = ", "call(alpha, ", "beta, gamma);"]);
        // Later parts fit the width less the indent.
        assert!(parts[1..].iter().all(|p| p.len() <= 20 - 4));
        // A word longer than the width breaks mid-word.
        assert_eq!(wrap_starts(&"x".repeat(25), 10), [0, 10, 20]);
        assert_eq!(wrap_starts("short", 10), [0]);
    }

    #[test]
    fn tabs_expand_with_ranges() {
        let (text, ranges) = expand_tabs("\tab\tc", &[1..3, 4..5]);
        assert_eq!(text, "    ab  c");
        assert_eq!(&text[ranges[0].clone()], "ab");
        assert_eq!(&text[ranges[1].clone()], "c");
    }

    #[test]
    fn display_and_byte_offsets_round_trip() {
        let text = "\tab\tc";
        assert_eq!(display_offset(text, 1), 4);
        assert_eq!(byte_offset(text, 4), 1);
        assert_eq!(byte_offset(text, 5), 2);
        // Inside a tab's spaces: snaps to before the tab.
        assert_eq!(byte_offset(text, 2), 0);
    }

    #[test]
    fn backgrounds_split_syntax_runs() {
        let red = HighlightStyle { color: Some(gpui_kit::red()), ..Default::default() };
        let out = merge_styles(&[(0..6, red)], &[(2..4, gpui_kit::blue())], 8);
        assert_eq!(out.len(), 3);
        assert_eq!(out[1].0, 2..4);
        assert_eq!(out[1].1.background_color, Some(gpui_kit::blue()));
        assert_eq!(out[2].0, 4..6);
    }
}

pub type Highlighter = gpui_kit::component::highlighter::SyntaxHighlighter;

/// Highlighters built ahead, by language: building one compiles the
/// grammar's queries (~25 ms for Rust), more than parsing a file.
static SPARE_HIGHLIGHTERS: std::sync::Mutex<Vec<Highlighter>> = std::sync::Mutex::new(Vec::new());

/// A highlighter for `language`, a spare one when there is (and another
/// spare is built in the background for the next file).
fn take_highlighter(language: &str) -> Highlighter {
    let spare = {
        let mut spares = SPARE_HIGHLIGHTERS.lock().unwrap();
        spares.iter().position(|h| h.language().as_ref() == language).map(|ix| spares.swap_remove(ix))
    };
    let language = language.to_owned();
    let refill = language.clone();
    std::thread::spawn(move || {
        let highlighter = Highlighter::new(&refill);
        let mut spares = SPARE_HIGHLIGHTERS.lock().unwrap();
        // Two per language (a diff's two sides), a few languages.
        // (A language without a grammar comes back as another: not kept.)
        if highlighter.language().as_ref() == refill.as_str() && spares.iter().filter(|h| h.language().as_ref() == refill.as_str()).count() < 2 {
            if spares.len() >= 8 {
                spares.remove(0);
            }
            spares.push(highlighter);
        }
    });
    spare.unwrap_or_else(|| Highlighter::new(&language))
}

/// Parses each text for syntax colors, in parallel. Slow (tens of ms for a
/// big file), so run it off the main thread.
pub fn highlight_texts(texts: &[String], language: &str) -> Vec<Highlighter> {
    std::thread::scope(|scope| {
        let jobs: Vec<_> = texts
            .iter()
            .map(|text| {
                let mut highlighter = take_highlighter(language);
                scope.spawn(move || {
                    highlighter.update(None, &gpui_kit::component::Rope::from_str(text), Some(Duration::from_millis(500)));
                    highlighter
                })
            })
            .collect();
        jobs.into_iter().map(|job| job.join().unwrap()).collect()
    })
}
