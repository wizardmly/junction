//! The parts IntelliJ's side-by-side and merge viewers are built from:
//! independent panes (each shows only its own lines), the change blocks
//! that pair their rows, the divider painted between two panes, and the
//! synchronized scrolling that keeps paired rows level.

use std::collections::HashSet;
use std::ops::Range;

use gpui_kit::{Bounds, Hsla, PathBuilder, Pixels, Window, point, px};

use crate::git::diff::{DiffRow, RowKind, Side};

pub const LINE_HEIGHT: f32 = 20.;
/// The divider between two panes, where change blocks are connected.
pub const DIVIDER_WIDTH: f32 = 24.;

/// One displayed row of a pane.
#[derive(Clone, Debug)]
pub enum PaneRow {
    Line {
        side: Side,
        /// The block's kind on a changed row.
        kind: Option<RowKind>,
        change: Option<usize>,
        /// First row of its change on this pane: carries the gutter buttons.
        first: bool,
    },
    /// A collapsed run of unchanged lines (on both panes at once).
    Fold { id: usize, count: usize },
    /// Space that keeps a block level with the other pane (Align Changes).
    Filler,
}

/// Rows of two panes that belong together: an unchanged stretch (same
/// number of rows on both) or a change block (either side may be empty).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub left: Range<usize>,
    pub right: Range<usize>,
    pub change: Option<usize>,
    pub kind: RowKind,
}

#[derive(Clone, Debug, Default)]
pub struct TwoSide {
    pub left: Vec<PaneRow>,
    pub right: Vec<PaneRow>,
    pub segments: Vec<Segment>,
}

impl TwoSide {
    /// Splits aligned diff rows into the two panes' own rows.
    pub fn build(rows: &[DiffRow], expanded: &HashSet<usize>) -> Self {
        let mut out = TwoSide::default();
        out.push_rows(rows, expanded);
        out.close_block();
        out
    }

    fn push_rows(&mut self, rows: &[DiffRow], expanded: &HashSet<usize>) {
        for row in rows {
            match row {
                DiffRow::Fold { id, rows } if expanded.contains(id) => self.push_rows(rows, expanded),
                DiffRow::Fold { id, rows } => {
                    self.close_block();
                    self.push_equal(true, true);
                    self.left.push(PaneRow::Fold { id: *id, count: rows.len() });
                    self.right.push(PaneRow::Fold { id: *id, count: rows.len() });
                }
                DiffRow::Line { change: None, left, right, .. } => {
                    // One side only: a blank line the whitespace option ignores.
                    self.close_block();
                    self.push_equal(left.is_some(), right.is_some());
                    if let Some(l) = left {
                        self.left.push(PaneRow::Line { side: l.clone(), kind: None, change: None, first: false });
                    }
                    if let Some(r) = right {
                        self.right.push(PaneRow::Line { side: r.clone(), kind: None, change: None, first: false });
                    }
                }
                DiffRow::Line { change: Some(c), left, right, .. } => {
                    let open = matches!(self.segments.last(), Some(Segment { change: Some(last), .. }) if last == c);
                    if !open {
                        self.close_block();
                        let (l, r) = (self.left.len(), self.right.len());
                        self.segments.push(Segment { left: l..l, right: r..r, change: Some(*c), kind: RowKind::Modified });
                    }
                    let seg = self.segments.last_mut().unwrap();
                    if let Some(l) = left {
                        let first = seg.left.is_empty();
                        self.left.push(PaneRow::Line { side: l.clone(), kind: None, change: Some(*c), first });
                        seg.left.end += 1;
                    }
                    if let Some(r) = right {
                        let first = seg.right.is_empty();
                        self.right.push(PaneRow::Line { side: r.clone(), kind: None, change: Some(*c), first });
                        seg.right.end += 1;
                    }
                }
            }
        }
    }

    /// One more row of an unchanged stretch, on either pane or both.
    fn push_equal(&mut self, left: bool, right: bool) {
        let (l, r) = (self.left.len(), self.right.len());
        let (dl, dr) = (left as usize, right as usize);
        match self.segments.last_mut() {
            Some(seg) if seg.change.is_none() => {
                seg.left.end += dl;
                seg.right.end += dr;
            }
            _ => self.segments.push(Segment { left: l..l + dl, right: r..r + dr, change: None, kind: RowKind::Equal }),
        }
    }

    /// Settles the open change block's kind and stamps it on its rows.
    fn close_block(&mut self) {
        let Some(seg) = self.segments.last_mut() else { return };
        if seg.change.is_none() {
            return;
        }
        seg.kind = match (seg.left.is_empty(), seg.right.is_empty()) {
            (false, false) => RowKind::Modified,
            (false, true) => RowKind::Deleted,
            _ => RowKind::Inserted,
        };
        let (kind, left, right) = (seg.kind, seg.left.clone(), seg.right.clone());
        for row in &mut self.left[left] {
            if let PaneRow::Line { kind: k, .. } = row {
                *k = Some(kind);
            }
        }
        for row in &mut self.right[right] {
            if let PaneRow::Line { kind: k, .. } = row {
                *k = Some(kind);
            }
        }
    }

    /// Align Changes: pads the shorter side of every block with filler
    /// rows, so paired rows sit level and the divider bands are straight.
    pub fn align(&mut self) {
        let (mut left, mut right) = (Vec::new(), Vec::new());
        let mut segments = Vec::with_capacity(self.segments.len());
        for seg in &self.segments {
            let n = seg.left.len().max(seg.right.len());
            let (l, r) = (left.len(), right.len());
            left.extend(self.left[seg.left.clone()].iter().cloned());
            right.extend(self.right[seg.right.clone()].iter().cloned());
            left.resize(l + n, PaneRow::Filler);
            right.resize(r + n, PaneRow::Filler);
            segments.push(Segment { left: l..l + n, right: r..r + n, ..seg.clone() });
        }
        // Rows after the last segment (none today) keep their place.
        left.extend(self.left.drain(self.segments.last().map_or(0, |s| s.left.end)..));
        right.extend(self.right.drain(self.segments.last().map_or(0, |s| s.right.end)..));
        (self.left, self.right, self.segments) = (left, right, segments);
    }

    /// Whether a pane's rows hold no line (an empty side, maybe padded).
    pub fn no_lines(rows: &[PaneRow]) -> bool {
        rows.iter().all(|r| matches!(r, PaneRow::Filler))
    }

    pub fn change_segment(&self, change: usize) -> Option<&Segment> {
        self.segments.iter().find(|s| s.change == Some(change))
    }
}

/// Maps a (fractional) row of one pane to the other, IntelliJ's sync
/// scrolling: unchanged stretches move row for row, change blocks
/// proportionally.
pub fn map_row(segments: &[Segment], from_left: bool, row: f32) -> f32 {
    let pick = |s: &Segment| if from_left { (s.left.clone(), s.right.clone()) } else { (s.right.clone(), s.left.clone()) };
    let mut last = 0.0;
    for seg in segments {
        let (from, to) = pick(seg);
        if row < from.end as f32 || (from.is_empty() && row <= from.start as f32) {
            if from.is_empty() {
                return to.start as f32;
            }
            let t = ((row - from.start as f32) / from.len() as f32).clamp(0., 1.);
            return to.start as f32 + t * to.len() as f32;
        }
        last = to.end as f32 + (row - from.end as f32);
    }
    last.max(0.)
}

/// Fill and edge colors of a change block.
#[derive(Clone, Copy)]
pub struct BlockColors {
    pub fill: Hsla,
    pub border: Hsla,
}

/// One connector of the divider, in rows of each pane.
pub struct Connector {
    pub left: Range<usize>,
    pub right: Range<usize>,
    pub colors: BlockColors,
}

/// Paints the divider: each change block as a band joining its rows on the
/// left pane to its rows on the right, edges drawn, as `DiffDividerDrawUtil`.
pub fn paint_divider(
    bounds: Bounds<Pixels>,
    scroll: (f32, f32),
    tops: (&[f32], &[f32]),
    connectors: &[Connector],
    folds: &[(usize, usize)],
    fold_color: Hsla,
    window: &mut Window,
) {
    let top = f32::from(bounds.origin.y);
    let (x0, x1) = (bounds.origin.x, bounds.origin.x + bounds.size.width);
    let height = f32::from(bounds.size.height);
    // Row tops come from the panes: wrapped rows are taller.
    let y = |tops: &[f32], row: usize, offset: f32| {
        top + tops.get(row).or(tops.last()).copied().unwrap_or(row as f32 * LINE_HEIGHT) - offset
    };
    for c in connectors {
        let (l0, l1) = (y(tops.0, c.left.start, scroll.0), y(tops.0, c.left.end, scroll.0));
        let (r0, r1) = (y(tops.1, c.right.start, scroll.1), y(tops.1, c.right.end, scroll.1));
        if l1.max(r1) < top || l0.min(r0) > top + height {
            continue;
        }
        let mut fill = PathBuilder::fill();
        fill.add_polygon(&[point(x0, px(l0)), point(x1, px(r0)), point(x1, px(r1)), point(x0, px(l1))], true);
        if let Ok(path) = fill.build() {
            window.paint_path(path, c.colors.fill);
        }
        for (a, b) in [(l0, r0), (l1, r1)] {
            let mut edge = PathBuilder::stroke(px(1.));
            edge.move_to(point(x0, px(a)));
            edge.line_to(point(x1, px(b)));
            if let Ok(path) = edge.build() {
                window.paint_path(path, c.colors.border);
            }
        }
    }
    // Collapsed fragments are joined by a line between their middles.
    for (l, r) in folds {
        let (a, b) = (y(tops.0, *l, scroll.0) + LINE_HEIGHT / 2., y(tops.1, *r, scroll.1) + LINE_HEIGHT / 2.);
        let mut link = PathBuilder::stroke(px(1.));
        link.move_to(point(x0, px(a)));
        link.line_to(point(x1, px(b)));
        if let Ok(path) = link.build() {
            window.paint_path(path, fold_color);
        }
    }
}

/// Rows of each collapsed fragment on both panes.
pub fn fold_links(two: &TwoSide) -> Vec<(usize, usize)> {
    let rows = |pane: &[PaneRow]| -> Vec<usize> {
        pane.iter().enumerate().filter(|(_, r)| matches!(r, PaneRow::Fold { .. })).map(|(i, _)| i).collect()
    };
    rows(&two.left).into_iter().zip(rows(&two.right)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::diff::{self, DiffOptions};

    fn build(old: &str, new: &str) -> TwoSide {
        let d = diff::compute(old, new, DiffOptions { context: None, ..Default::default() });
        TwoSide::build(&d.rows, &HashSet::new())
    }

    #[test]
    fn panes_hold_only_their_own_lines() {
        let t = build("a\nb\nc\nd\n", "a\nX\nY\nc\nd\ne\n");
        assert_eq!((t.left.len(), t.right.len()), (4, 6));
        let kinds: Vec<_> = t.segments.iter().map(|s| (s.left.clone(), s.right.clone(), s.kind)).collect();
        assert_eq!(
            kinds,
            vec![
                (0..1, 0..1, RowKind::Equal),
                (1..2, 1..3, RowKind::Modified),
                (2..4, 3..5, RowKind::Equal),
                (4..4, 5..6, RowKind::Inserted),
            ]
        );
        assert!(matches!(t.right[1], PaneRow::Line { first: true, kind: Some(RowKind::Modified), .. }));
        assert!(matches!(t.right[2], PaneRow::Line { first: false, .. }));
    }

    #[test]
    fn maps_rows_through_changes() {
        let t = build("a\nb\nc\nd\n", "a\nX\nY\nc\nd\ne\n");
        assert_eq!(map_row(&t.segments, true, 0.5), 0.5);
        // Inside the block: proportional.
        assert_eq!(map_row(&t.segments, true, 1.5), 2.0);
        // After it: shifted by the extra row.
        assert_eq!(map_row(&t.segments, true, 3.0), 4.0);
        assert_eq!(map_row(&t.segments, false, 4.0), 3.0);
    }
}
