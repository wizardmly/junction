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

#[derive(Clone, Debug, Default)]
pub struct GraphLayout {
    pub rows: Vec<GraphRow>,
}

#[derive(Clone)]
struct Lane<'a> {
    waiting_for: &'a str,
    color: usize,
}

impl GraphLayout {
    pub fn build(commits: &[Commit]) -> Self {
        let mut lanes: Vec<Option<Lane>> = Vec::new();
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
                None => (free_slot(&mut lanes), new_color()),
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
                        lanes[node_lane] = Some(Lane { waiting_for: lane.waiting_for, color: node_color });
                    } else {
                        let color = lanes[ix].as_ref().unwrap().color;
                        lines.push(GraphLine { half: Half::Bottom, from_lane: node_lane, to_lane: ix, color });
                    }
                    continue;
                }
                let (ix, color) = if parent_ix == 0 && lanes[node_lane].is_none() {
                    (node_lane, node_color)
                } else {
                    (free_slot(&mut lanes), new_color())
                };
                lanes[ix] = Some(Lane { waiting_for: parent, color });
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

fn free_slot(lanes: &mut Vec<Option<Lane>>) -> usize {
    match lanes.iter().position(Option::is_none) {
        Some(ix) => ix,
        None => {
            lanes.push(None);
            lanes.len() - 1
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
            author_name: String::new(),
            author_email: String::new(),
            author_time: 0,
            subject: String::new(),
        }
    }

    #[test]
    fn linear_history_stays_in_one_lane() {
        let layout = GraphLayout::build(&[commit("c", &["b"]), commit("b", &["a"]), commit("a", &[])]);
        assert!(layout.rows.iter().all(|row| row.node_lane == 0));
        assert!(layout.rows[2].lines.iter().all(|l| l.half == Half::Top));
    }

    #[test]
    fn merge_opens_and_closes_a_lane() {
        // m merges feature (f) into main (b); both come from a.
        let layout = GraphLayout::build(&[
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
        let layout = GraphLayout::build(&fork_far_below(40));
        assert_eq!(layout.rows[1].node_lane, 1, "the side tip gets the second lane");
        let hidden = layout.hide_long_edges();
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
        let layout = GraphLayout::build(&fork_far_below(LONG_EDGE - 2));
        assert_eq!(layout.hide_long_edges().rows, layout.rows);
    }
}
