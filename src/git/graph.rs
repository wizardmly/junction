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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GraphRow {
    pub node_lane: usize,
    pub node_color: usize,
    pub lines: Vec<GraphLine>,
    /// Number of lanes this row spans, for sizing the graph column.
    pub width: usize,
}

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
            rows.push(GraphRow { node_lane, node_color, lines, width });
        }

        Self { rows }
    }
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
}
