//! Lays out the commit graph drawn in the Log's first column.
//!
//! Each commit occupies one row. A row is split at its vertical middle, where
//! the commit's node sits: `Top` segments run from the row's top edge to the
//! middle, `Bottom` segments from the middle to the bottom edge. Lanes are
//! columns; a lane carries an edge downwards until it reaches the parent it is
//! waiting for.

use super::Commit;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Half {
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphLine {
    pub half: Half,
    pub from_lane: usize,
    pub to_lane: usize,
    pub color: usize,
}

/// Where a hidden long edge is cut: a short arrow in `lane`, pointing down
/// (the edge goes on below) or up; clicking it goes to row `target`, the
/// edge's other end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EdgeArrow {
    pub lane: usize,
    pub down: bool,
    pub color: usize,
    pub target: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GraphRow {
    pub node_lane: usize,
    pub node_color: usize,
    pub lines: Vec<GraphLine>,
    pub arrows: Vec<EdgeArrow>,
    /// Number of lanes this row spans, for sizing the graph column.
    pub width: usize,
}

/// An edge passing at least this many rows is long: IntelliJ hides all but
/// its ends unless View Options › Show Long Edges is on.
pub const LONG_EDGE: usize = 30;

/// The laid-out graph, kept compact: per row only the node and the edges
/// that start or end there. Edges passing straight through a row are
/// implied by which lanes are open, recovered from a snapshot of the lanes
/// taken every [`CHECKPOINT`] rows. Rows are drawn on demand with
/// [`GraphLayout::rows`], so a log of millions of commits stays small.
#[derive(Clone, Debug, Default)]
pub struct GraphLayout {
    nodes: Vec<Node>,
    events: Vec<Event>,
    /// Each lane occupation (an edge between a commit and its parent).
    segments: Vec<Segment>,
    /// Open lanes at the top of every `CHECKPOINT`th row: (lane, segment).
    checkpoints: Vec<u32>,
    checkpoint_lanes: Vec<(u32, u32)>,
}

const CHECKPOINT: usize = 64;
const NO_SEGMENT: u32 = u32::MAX;

#[derive(Clone, Copy, Debug)]
struct Node {
    lane: u32,
    color: u32,
    width: u32,
    /// End of this row's events in `events`.
    events_end: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EventKind {
    /// A lane waiting for this row's commit ends at its node.
    Close,
    /// An edge leaving the node downwards; `segment` is set when it opens a lane.
    Parent,
    /// A lane waiting for the same parent as the node folds into the node's lane.
    Moved,
}

#[derive(Clone, Copy, Debug)]
struct Event {
    kind: EventKind,
    from: u32,
    to: u32,
    color: u32,
    segment: u32,
}

#[derive(Clone, Copy, Debug)]
struct Segment {
    color: u32,
    /// The row where the lane opens (at its bottom half) and the row where
    /// it ends; `u32::MAX` while it runs past the last loaded row.
    start: u32,
    end: u32,
}

impl Segment {
    /// The rows it passes straight through, when that run is long.
    fn long_run(&self, rows: usize) -> Option<(usize, usize)> {
        let first = self.start as usize + 1;
        let last = if self.end == u32::MAX { rows.checked_sub(1)? } else { (self.end as usize).checked_sub(1)? };
        (last + 1 >= first + LONG_EDGE).then_some((first, last))
    }
}

struct Lane<'a> {
    waiting_for: &'a str,
    segment: u32,
}

impl GraphLayout {
    pub fn build(commits: &[Commit]) -> Self {
        let mut this = Self { nodes: Vec::with_capacity(commits.len()), ..Default::default() };
        let mut lanes: Vec<Option<Lane>> = Vec::new();
        // Which lane waits for a hash; each hash has at most one.
        let mut waiting: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        let mut next_color = 0u32;
        let mut new_color = || {
            let color = next_color;
            next_color += 1;
            color
        };

        for (row, commit) in commits.iter().enumerate() {
            if row % CHECKPOINT == 0 {
                this.checkpoints.push(this.checkpoint_lanes.len() as u32);
                for (ix, lane) in lanes.iter().enumerate() {
                    if let Some(lane) = lane {
                        this.checkpoint_lanes.push((ix as u32, lane.segment));
                    }
                }
            }
            let hash = commit.hash.as_str();
            let width_before = lanes.len();
            let open = |this: &mut Self, color: u32| {
                this.segments.push(Segment { color, start: row as u32, end: u32::MAX });
                (this.segments.len() - 1) as u32
            };
            let close = |this: &mut Self, segment: u32| this.segments[segment as usize].end = row as u32;

            let existing = waiting.remove(hash);
            let (node_lane, node_color) = match existing {
                Some(ix) => {
                    let lane = lanes[ix].take().unwrap();
                    let color = this.segments[lane.segment as usize].color;
                    close(&mut this, lane.segment);
                    this.events.push(Event { kind: EventKind::Close, from: ix as u32, to: ix as u32, color, segment: lane.segment });
                    (ix, color)
                }
                None => (free_slot(&mut lanes), new_color()),
            };

            for (parent_ix, parent) in commit.parents.iter().enumerate() {
                let parent = parent.as_str();
                if let Some(&ix) = waiting.get(parent) {
                    let color = this.segments[lanes[ix].as_ref().unwrap().segment as usize].color;
                    if parent_ix == 0 && ix > node_lane && lanes[node_lane].is_none() {
                        // The first parent continues straight down; pull the
                        // lane that was already waiting for it over into ours.
                        // Keep this commit's color going down; the folded lane's
                        // edge keeps its own color as it converges.
                        let lane = lanes[ix].take().unwrap();
                        close(&mut this, lane.segment);
                        this.events.push(Event { kind: EventKind::Moved, from: ix as u32, to: node_lane as u32, color, segment: lane.segment });
                        let segment = open(&mut this, node_color);
                        this.events.push(Event { kind: EventKind::Parent, from: node_lane as u32, to: node_lane as u32, color: node_color, segment });
                        lanes[node_lane] = Some(Lane { waiting_for: lane.waiting_for, segment });
                        waiting.insert(lane.waiting_for, node_lane);
                    } else {
                        this.events.push(Event { kind: EventKind::Parent, from: node_lane as u32, to: ix as u32, color, segment: NO_SEGMENT });
                    }
                    continue;
                }
                let (ix, color) = if parent_ix == 0 && lanes[node_lane].is_none() {
                    (node_lane, node_color)
                } else {
                    (free_slot(&mut lanes), new_color())
                };
                let segment = open(&mut this, color);
                lanes[ix] = Some(Lane { waiting_for: parent, segment });
                waiting.insert(parent, ix);
                this.events.push(Event { kind: EventKind::Parent, from: node_lane as u32, to: ix as u32, color, segment });
            }

            let width = width_before.max(lanes.len()).max(node_lane + 1);
            while lanes.last().is_some_and(Option::is_none) {
                lanes.pop();
            }
            this.nodes.push(Node { lane: node_lane as u32, color: node_color, width: width as u32, events_end: this.events.len() as u32 });
        }
        this
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    fn events(&self, row: usize) -> &[Event] {
        let start = if row == 0 { 0 } else { self.nodes[row - 1].events_end as usize };
        &self.events[start..self.nodes[row].events_end as usize]
    }

    /// The rows in `range`, with long edges cut into arrows unless
    /// `long_edges` (View Options › Show Long Edges).
    pub fn rows(&self, range: std::ops::Range<usize>, long_edges: bool) -> Vec<GraphRow> {
        let range = range.start.min(self.len())..range.end.min(self.len());
        if range.is_empty() {
            return Vec::new();
        }
        // Lanes open at the top of the row: their segments.
        let checkpoint = range.start / CHECKPOINT;
        let from = self.checkpoints[checkpoint] as usize;
        let to = self.checkpoints.get(checkpoint + 1).map_or(self.checkpoint_lanes.len(), |&e| e as usize);
        let mut lanes: Vec<u32> = Vec::new();
        for &(lane, segment) in &self.checkpoint_lanes[from..to] {
            set_lane(&mut lanes, lane as usize, segment);
        }
        let mut out = Vec::with_capacity(range.len());
        for row in checkpoint * CHECKPOINT..range.end {
            if row >= range.start {
                out.push(self.draw(row, &lanes, long_edges));
            }
            for event in self.events(row) {
                if matches!(event.kind, EventKind::Close | EventKind::Moved) {
                    lanes[event.from as usize] = NO_SEGMENT;
                }
            }
            for event in self.events(row) {
                if event.kind == EventKind::Parent && event.segment != NO_SEGMENT {
                    set_lane(&mut lanes, event.to as usize, event.segment);
                }
            }
            while lanes.last() == Some(&NO_SEGMENT) {
                lanes.pop();
            }
        }
        out
    }

    fn draw(&self, row: usize, lanes: &[u32], long_edges: bool) -> GraphRow {
        let node = self.nodes[row];
        let events = self.events(row);
        let node_lane = node.lane as usize;
        let mut lines = Vec::new();
        let mut arrows = Vec::new();
        // Lanes passing straight through, and whether each half is drawn.
        let mut passing: Vec<(usize, u32, bool)> = Vec::new();
        let mut cut = false;
        for (ix, &segment) in lanes.iter().enumerate() {
            if segment == NO_SEGMENT {
                continue;
            }
            let color = self.segments[segment as usize].color as usize;
            if let Some(close) = events.iter().find(|e| e.kind == EventKind::Close && e.from as usize == ix) {
                lines.push(GraphLine { half: Half::Top, from_lane: ix, to_lane: node_lane, color: close.color as usize });
                continue;
            }
            let (mut top, mut bottom) = (true, true);
            let moved = events.iter().any(|e| e.kind == EventKind::Moved && e.from as usize == ix);
            if !long_edges && !moved {
                if let Some((first, last)) = self.segments[segment as usize].long_run(self.len()) {
                    if (first..=last).contains(&row) {
                        cut = true;
                        top = row == first;
                        bottom = row == last;
                        if row == first {
                            arrows.push(EdgeArrow { lane: ix, down: true, color, target: (last + 1).min(self.len() - 1) });
                        }
                        if row == last {
                            arrows.push(EdgeArrow { lane: ix, down: false, color, target: first.saturating_sub(1) });
                        }
                    }
                }
            }
            if top {
                lines.push(GraphLine { half: Half::Top, from_lane: ix, to_lane: ix, color });
            }
            passing.push((ix, segment, bottom));
        }
        for event in events.iter().filter(|e| e.kind == EventKind::Parent) {
            lines.push(GraphLine { half: Half::Bottom, from_lane: node_lane, to_lane: event.to as usize, color: event.color as usize });
        }
        for (ix, segment, bottom) in passing {
            match events.iter().find(|e| e.kind == EventKind::Moved && e.from as usize == ix) {
                Some(moved) => lines.push(GraphLine { half: Half::Bottom, from_lane: ix, to_lane: moved.to as usize, color: moved.color as usize }),
                None if bottom => {
                    let color = self.segments[segment as usize].color as usize;
                    lines.push(GraphLine { half: Half::Bottom, from_lane: ix, to_lane: ix, color });
                }
                None => {}
            }
        }
        let mut width = node.width as usize;
        if cut {
            // Cut edges free their lanes: the row is as wide as what is drawn.
            let lines_max = lines.iter().flat_map(|l| [l.from_lane, l.to_lane]);
            width = lines_max.chain(arrows.iter().map(|a| a.lane)).chain([node_lane]).max().unwrap_or(0) + 1;
        }
        GraphRow { node_lane, node_color: node.color as usize, lines, arrows, width }
    }
}

fn set_lane(lanes: &mut Vec<u32>, lane: usize, segment: u32) {
    if lanes.len() <= lane {
        lanes.resize(lane + 1, NO_SEGMENT);
    }
    lanes[lane] = segment;
}

fn free_slot(lanes: &mut Vec<Option<Lane>>) -> usize {
    match lanes.iter().position(Option::is_none) {
        Some(ix) => ix,
        None => {
            lanes.push(None);
            lanes.len() - 1
        }
    }
}

/// The previous, eager layout: the reference the compact one must match.
#[cfg(test)]
pub(crate) mod reference {
    use super::*;

    #[derive(Clone, Debug, Default)]
    pub struct Reference {
        pub rows: Vec<GraphRow>,
    }

    #[derive(Clone)]
    struct OldLane<'a> {
        waiting_for: &'a str,
        color: usize,
    }

    fn old_free_slot(lanes: &mut Vec<Option<OldLane>>) -> usize {
        match lanes.iter().position(Option::is_none) {
            Some(ix) => ix,
            None => {
                lanes.push(None);
                lanes.len() - 1
            }
        }
    }

    impl Reference {
        pub fn build(commits: &[Commit]) -> Self {
            let mut lanes: Vec<Option<OldLane>> = Vec::new();
            let mut next_color = 0;
            let mut new_color = || {
                let color = next_color;
                next_color += 1;
                color
            };
            let mut rows = Vec::with_capacity(commits.len());

            for commit in commits {
                let hash = commit.hash.as_str();
                let mut lines = Vec::new();
                let width_before = lanes.len();

                let existing = lanes.iter().position(|lane| lane.as_ref().is_some_and(|l| l.waiting_for == hash));
                let (node_lane, node_color) = match existing {
                    Some(ix) => (ix, lanes[ix].as_ref().unwrap().color),
                    None => (old_free_slot(&mut lanes), new_color()),
                };

                // Edges arriving from above: lanes waiting for this commit merge into
                // the node, every other lane passes straight through.
                let mut passing = Vec::new();
                for (ix, lane) in lanes.iter_mut().enumerate() {
                    let Some(l) = lane else { continue };
                    if l.waiting_for == hash {
                        lines.push(GraphLine { half: Half::Top, from_lane: ix, to_lane: node_lane, color: l.color });
                        *lane = None;
                    } else {
                        lines.push(GraphLine { half: Half::Top, from_lane: ix, to_lane: ix, color: l.color });
                        passing.push(ix);
                    }
                }

                // Edges leaving downwards, one per parent.
                let mut moved: Vec<(usize, usize, usize)> = Vec::new();
                for (parent_ix, parent) in commit.parents.iter().enumerate() {
                    let parent = parent.as_str();
                    if let Some(ix) = lanes.iter().position(|lane| lane.as_ref().is_some_and(|l| l.waiting_for == parent)) {
                        if parent_ix == 0 && ix > node_lane && lanes[node_lane].is_none() {
                            // The first parent continues straight down; pull the
                            // lane that was already waiting for it over into ours.
                            // Keep this commit's color going down; the folded lane's
                            // edge keeps its own color as it converges.
                            let lane = lanes[ix].take().unwrap();
                            lines.push(GraphLine { half: Half::Bottom, from_lane: node_lane, to_lane: node_lane, color: node_color });
                            moved.push((ix, node_lane, lane.color));
                            lanes[node_lane] = Some(OldLane { waiting_for: lane.waiting_for, color: node_color });
                        } else {
                            let color = lanes[ix].as_ref().unwrap().color;
                            lines.push(GraphLine { half: Half::Bottom, from_lane: node_lane, to_lane: ix, color });
                        }
                        continue;
                    }
                    let (ix, color) = if parent_ix == 0 && lanes[node_lane].is_none() {
                        (node_lane, node_color)
                    } else {
                        (old_free_slot(&mut lanes), new_color())
                    };
                    lanes[ix] = Some(OldLane { waiting_for: parent, color });
                    lines.push(GraphLine { half: Half::Bottom, from_lane: node_lane, to_lane: ix, color });
                }

                for ix in passing {
                    let (to_lane, color) = match moved.iter().find(|(from, _, _)| *from == ix) {
                        Some(&(_, to, color)) => (to, color),
                        None => (ix, lanes[ix].as_ref().unwrap().color),
                    };
                    lines.push(GraphLine { half: Half::Bottom, from_lane: ix, to_lane, color });
                }

                let width = width_before.max(lanes.len()).max(node_lane + 1);
                while lanes.last().is_some_and(Option::is_none) {
                    lanes.pop();
                }
                rows.push(GraphRow { node_lane, node_color, lines, arrows: Vec::new(), width });
            }

            Self { rows }
        }

        /// The layout with long edges cut, as IntelliJ draws them by default:
        /// an edge passing [`LONG_EDGE`] rows or more keeps one row at each
        /// end, which ends in an arrow, and frees its lane in between.
        pub fn hide_long_edges(&self) -> Self {
            let mut rows = self.rows.clone();
            let lanes = rows.iter().map(|r| r.width).max().unwrap_or(0);
            let mut changed = vec![false; rows.len()];
            for lane in 0..lanes {
                let mut r = 0;
                while r < rows.len() {
                    if !passes(&rows[r], lane) {
                        r += 1;
                        continue;
                    }
                    let start = r;
                    while r < rows.len() && passes(&rows[r], lane) {
                        r += 1;
                    }
                    let end = r - 1;
                    if end + 1 - start < LONG_EDGE {
                        continue;
                    }
                    let straight = |half| GraphLine { half, from_lane: lane, to_lane: lane, color: 0 };
                    let color = rows[start].lines.iter().find(|l| l.from_lane == lane && l.to_lane == lane).map_or(0, |l| l.color);
                    let is = |line: &GraphLine, half| GraphLine { color: 0, ..*line } == straight(half);
                    rows[start].lines.retain(|l| !is(l, Half::Bottom));
                    rows[start].arrows.push(EdgeArrow { lane, down: true, color, target: (end + 1).min(self.rows.len() - 1) });
                    for row in &mut rows[start + 1..end] {
                        row.lines.retain(|l| !is(l, Half::Top) && !is(l, Half::Bottom));
                    }
                    rows[end].lines.retain(|l| !is(l, Half::Top));
                    rows[end].arrows.push(EdgeArrow { lane, down: false, color, target: start.saturating_sub(1) });
                    changed[start..=end].iter_mut().for_each(|c| *c = true);
                }
            }
            for (row, changed) in rows.iter_mut().zip(changed) {
                if changed {
                    let lines = row.lines.iter().flat_map(|l| [l.from_lane, l.to_lane]);
                    row.width = lines.chain(row.arrows.iter().map(|a| a.lane)).chain([row.node_lane]).max().unwrap_or(0) + 1;
                }
            }
            Self { rows }
        }
    }

    /// Whether an edge passes straight through `lane` in this row, without
    /// meeting the row's commit.
    fn passes(row: &GraphRow, lane: usize) -> bool {
        let straight = |half| row.lines.iter().any(|l| l.half == half && l.from_lane == lane && l.to_lane == lane);
        row.node_lane != lane && straight(Half::Top) && straight(Half::Bottom)
    }


    /// Both layouts, row by row, with and without long edges.
    pub fn assert_same(commits: &[Commit]) {
        let reference = Reference::build(commits);
        let hidden = reference.hide_long_edges();
        let layout = GraphLayout::build(commits);
        assert_eq!(layout.len(), reference.rows.len());
        for start in (0..commits.len()).step_by(97) {
            let end = (start + 150).min(commits.len());
            assert_eq!(layout.rows(start..end, true), reference.rows[start..end], "rows {start}..{end}");
            assert_eq!(layout.rows(start..end, false), hidden.rows[start..end], "rows {start}..{end} without long edges");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(hash: &str, parents: &[&str]) -> Commit {
        Commit {
            hash: hash.into(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            author_name: "".into(),
            author_email: "".into(),
            author_time: 0,
            subject: String::new(),
        }
    }

    #[test]
    fn linear_history_stays_in_one_lane() {
        let layout = all(&[commit("c", &["b"]), commit("b", &["a"]), commit("a", &[])]);
        assert!(layout.rows.iter().all(|row| row.node_lane == 0));
        assert!(layout.rows[2].lines.iter().all(|l| l.half == Half::Top));
    }

    #[test]
    fn merge_opens_and_closes_a_lane() {
        // m merges feature (f) into main (b); both come from a.
        let layout = all(&[
            commit("m", &["b", "f"]),
            commit("f", &["a"]),
            commit("b", &["a"]),
            commit("a", &[]),
        ]);
        let rows = &layout.rows;
        assert_eq!(rows[0].node_lane, 0);
        assert_eq!(rows[1].node_lane, 1, "feature commit gets the second lane");
        assert_eq!(rows[2].node_lane, 0);
        // Below `b`, the feature lane (also waiting for `a`) folds back into lane 0.
        assert!(rows[2].lines.contains(&GraphLine { half: Half::Bottom, from_lane: 1, to_lane: 0, color: 1 }));
        // Main keeps its color all the way down.
        assert!(rows.iter().filter(|r| r.node_lane == 0).all(|r| r.node_color == 0));
        assert_eq!(rows[3].node_lane, 0);
        assert_eq!(rows[3].width, 1);
    }

    /// `c0`, then a side tip `s` forking from `base` far below, then the
    /// main line `c1..c{n}` down to `base`.
    fn fork_far_below(n: usize) -> Vec<Commit> {
        let names: Vec<String> = (0..=n).map(|i| format!("c{i}")).collect();
        let mut commits = vec![commit("c0", &["c1"]), commit("s", &["base"])];
        for i in 1..=n {
            let parent = if i == n { "base".to_owned() } else { names[i + 1].clone() };
            commits.push(commit(&names[i], &[parent.as_str()]));
        }
        commits.push(commit("base", &[]));
        commits
    }

    #[test]
    fn long_edges_are_cut_into_arrows() {
        let commits = fork_far_below(40);
        let layout = all(&commits);
        assert_eq!(layout.rows[1].node_lane, 1, "the side tip gets the second lane");
        let hidden = Rows { rows: GraphLayout::build(&commits).rows(0..commits.len(), false) };
        // The side edge folds into the main lane on the last main commit.
        let join = hidden.rows.len() - 2;
        let on_lane = |row: &GraphRow| row.lines.iter().any(|l| l.from_lane == 1 || l.to_lane == 1);
        // The ends keep a stub with an arrow pointing at the other end.
        assert_eq!(hidden.rows[2].arrows, vec![EdgeArrow { lane: 1, down: true, color: 1, target: join }]);
        assert_eq!(hidden.rows[join - 1].arrows, vec![EdgeArrow { lane: 1, down: false, color: 1, target: 1 }]);
        assert!(on_lane(&hidden.rows[2]) && on_lane(&hidden.rows[join - 1]));
        // In between the lane is free and the graph narrower.
        assert!(hidden.rows[3..join - 1].iter().all(|r| !on_lane(r) && r.arrows.is_empty() && r.width == 1));
        // The fork and the join are drawn as before.
        assert_eq!(hidden.rows[1], layout.rows[1]);
        assert_eq!(hidden.rows[join], layout.rows[join]);
    }

    #[test]
    fn short_edges_stay_whole() {
        let commits = fork_far_below(LONG_EDGE - 2);
        let layout = GraphLayout::build(&commits);
        assert_eq!(layout.rows(0..commits.len(), false), layout.rows(0..commits.len(), true));
        reference::assert_same(&commits);
    }

    struct Rows {
        rows: Vec<GraphRow>,
    }

    fn all(commits: &[Commit]) -> Rows {
        reference::assert_same(commits);
        Rows { rows: GraphLayout::build(commits).rows(0..commits.len(), true) }
    }

    /// Random histories: merges, octopus merges, branches forking far
    /// below and tips whose parents aren't loaded.
    #[test]
    fn matches_the_eager_layout() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        for round in 0..40 {
            let n = 50 + next(400);
            // Commit i's parents come later in the list (topological order).
            let mut commits = Vec::new();
            for i in 0..n {
                let mut parents = Vec::new();
                let count = match next(10) { 0 => 0, 1 | 2 => 2, 3 if round % 3 == 0 => 3, _ => 1 };
                for _ in 0..count {
                    let span = if next(6) == 0 { 1 + next(120) } else { 1 + next(4) };
                    let p = i + span;
                    // Some parents point past the loaded rows, like a first page.
                    let name = if p < n || next(3) == 0 { format!("c{p}") } else { continue };
                    if !parents.contains(&name) {
                        parents.push(name);
                    }
                }
                let refs: Vec<&str> = parents.iter().map(String::as_str).collect();
                commits.push(commit(&format!("c{i}"), &refs));
            }
            reference::assert_same(&commits);
        }
    }
}
