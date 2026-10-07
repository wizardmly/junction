//! Conflict resolution: the three versions of a conflicted file, a diff3
//! split into chunks for the merge tool, and Accept Yours / Theirs.

use std::ops::Range;

use anyhow::Result;
use similar::DiffOp;

use super::{Repository, RepositoryState};

/// How a file conflicts, from the porcelain status code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictKind {
    BothModified,
    BothAdded,
    DeletedByUs,
    DeletedByThem,
    BothDeleted,
    AddedByUs,
    AddedByThem,
}

impl ConflictKind {
    pub fn from_codes(index: char, work_tree: char) -> Option<Self> {
        Some(match (index, work_tree) {
            ('U', 'U') => Self::BothModified,
            ('A', 'A') => Self::BothAdded,
            ('D', 'U') => Self::DeletedByUs,
            ('U', 'D') => Self::DeletedByThem,
            ('D', 'D') => Self::BothDeleted,
            ('A', 'U') => Self::AddedByUs,
            ('U', 'A') => Self::AddedByThem,
            _ => return None,
        })
    }

    /// IntelliJ's "Yours" / "Theirs" columns in the Conflicts dialog.
    pub fn sides(self) -> (&'static str, &'static str) {
        match self {
            Self::BothModified => ("Modified", "Modified"),
            Self::BothAdded => ("Added", "Added"),
            Self::DeletedByUs => ("Deleted", "Modified"),
            Self::DeletedByThem => ("Modified", "Deleted"),
            Self::BothDeleted => ("Deleted", "Deleted"),
            Self::AddedByUs => ("Added", "—"),
            Self::AddedByThem => ("—", "Added"),
        }
    }

    /// Whether the merge tool can show it (both sides have content).
    pub fn can_merge(self) -> bool {
        matches!(self, Self::BothModified | Self::BothAdded)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    pub path: String,
    pub kind: ConflictKind,
}

pub fn conflicts(status: &super::WorkingTreeStatus) -> Vec<Conflict> {
    status
        .entries
        .iter()
        .filter_map(|e| ConflictKind::from_codes(e.index, e.work_tree).map(|kind| Conflict { path: e.path.clone(), kind }))
        .collect()
}

/// Base, ours and theirs (index stages 1–3); a missing stage is empty.
pub struct MergeVersions {
    pub base: String,
    pub ours: String,
    pub theirs: String,
}

pub fn load_versions(repository: &Repository, path: &str) -> Result<MergeVersions> {
    let stage = |n: u8| repository.run(["show", &format!(":{n}:{path}")]).unwrap_or_default();
    Ok(MergeVersions { base: stage(1), ours: stage(2), theirs: stage(3) })
}

/// Titles for the merge tool's sides, as IntelliJ words them for each operation.
pub fn side_titles(state: RepositoryState) -> (&'static str, &'static str) {
    match state {
        // During a rebase "ours" is the branch being rebased onto.
        RepositoryState::Rebasing => ("Changes from upstream", "Your changes being rebased"),
        RepositoryState::CherryPicking => ("Your version", "Changes from cherry-picked commit"),
        RepositoryState::Reverting => ("Your version", "Changes from reverted commit"),
        _ => ("Your version", "Changes from the merged branch"),
    }
}

pub fn accept(repository: &Repository, conflict: &Conflict, ours: bool) -> Result<()> {
    let path = conflict.path.as_str();
    let deleted_on_chosen_side = match conflict.kind {
        ConflictKind::DeletedByUs | ConflictKind::AddedByThem => ours,
        ConflictKind::DeletedByThem | ConflictKind::AddedByUs => !ours,
        ConflictKind::BothDeleted => true,
        _ => false,
    };
    if deleted_on_chosen_side {
        repository.run(["rm", "-q", "--", path])?;
    } else {
        repository.run(["checkout", if ours { "--ours" } else { "--theirs" }, "--", path])?;
        repository.run(["add", "--", path])?;
    }
    Ok(())
}

/// Writes the merge tool's result and marks the file resolved.
pub fn resolve_with(repository: &Repository, path: &str, content: &str) -> Result<()> {
    std::fs::write(repository.root().join(path), content)?;
    repository.run(["add", "--", path])?;
    Ok(())
}

/// One region of a three-way merge, over line indexes of each version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeChunk {
    /// Unchanged on both sides.
    Equal { base: Range<usize> },
    /// Changed on one or both sides; a conflict when both changed differently.
    Change { base: Range<usize>, ours: Range<usize>, theirs: Range<usize>, ours_changed: bool, theirs_changed: bool, conflict: bool },
}

struct Edit {
    base: Range<usize>,
    side: Range<usize>,
}

fn edits(base: &[&str], side: &[&str]) -> Vec<Edit> {
    similar::capture_diff_slices(similar::Algorithm::Myers, base, side)
        .into_iter()
        .filter_map(|op| match op {
            DiffOp::Equal { .. } => None,
            DiffOp::Delete { old_index, old_len, new_index } => {
                Some(Edit { base: old_index..old_index + old_len, side: new_index..new_index })
            }
            DiffOp::Insert { old_index, new_index, new_len } => {
                Some(Edit { base: old_index..old_index, side: new_index..new_index + new_len })
            }
            DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                Some(Edit { base: old_index..old_index + old_len, side: new_index..new_index + new_len })
            }
        })
        .collect()
}

/// Maps a base range to the side's range, given the side's edits in it.
fn side_range(region: &Range<usize>, edits: &[&Edit], offset: isize) -> Range<usize> {
    match (edits.first(), edits.last()) {
        (Some(first), Some(last)) => {
            let start = first.side.start - (first.base.start - region.start);
            let end = last.side.end + (region.end - last.base.end);
            start..end
        }
        _ => {
            let start = (region.start as isize + offset) as usize;
            start..start + region.len()
        }
    }
}

/// Splits a three-way merge into chunks, like diff3: overlapping or touching
/// edits from both sides form one change, a conflict if their results differ.
pub fn chunks(base: &[&str], ours: &[&str], theirs: &[&str]) -> Vec<MergeChunk> {
    let ours_edits = edits(base, ours);
    let theirs_edits = edits(base, theirs);
    let (mut oi, mut ti) = (0, 0);
    // Line offset of each side relative to base, before the current position.
    let (mut ours_offset, mut theirs_offset) = (0isize, 0isize);
    let mut position = 0;
    let mut out = Vec::new();

    loop {
        let next_ours = ours_edits.get(oi).map(|e| e.base.start);
        let next_theirs = theirs_edits.get(ti).map(|e| e.base.start);
        let start = match (next_ours, next_theirs) {
            (None, None) => break,
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (Some(a), Some(b)) => a.min(b),
        };
        if start > position {
            out.push(MergeChunk::Equal { base: position..start });
        }
        // Grow the region while edits from either side overlap or touch it.
        let mut region = start..start;
        let (mut ours_in, mut theirs_in): (Vec<&Edit>, Vec<&Edit>) = (Vec::new(), Vec::new());
        loop {
            let mut grew = false;
            while let Some(edit) = ours_edits.get(oi).filter(|e| e.base.start <= region.end) {
                region.end = region.end.max(edit.base.end);
                ours_in.push(edit);
                oi += 1;
                grew = true;
            }
            while let Some(edit) = theirs_edits.get(ti).filter(|e| e.base.start <= region.end) {
                region.end = region.end.max(edit.base.end);
                theirs_in.push(edit);
                ti += 1;
                grew = true;
            }
            if !grew {
                break;
            }
        }
        let ours_range = side_range(&region, &ours_in, ours_offset);
        let theirs_range = side_range(&region, &theirs_in, theirs_offset);
        ours_offset = ours_range.end as isize - region.end as isize;
        theirs_offset = theirs_range.end as isize - region.end as isize;
        let (ours_changed, theirs_changed) = (!ours_in.is_empty(), !theirs_in.is_empty());
        let conflict = ours_changed && theirs_changed && ours[ours_range.clone()] != theirs[theirs_range.clone()];
        position = region.end;
        out.push(MergeChunk::Change { base: region, ours: ours_range, theirs: theirs_range, ours_changed, theirs_changed, conflict });
    }
    if position < base.len() {
        out.push(MergeChunk::Equal { base: position..base.len() });
    }
    out
}

/// What the merge result takes for a change chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// Not decided yet: the result shows the base text.
    Unresolved,
    Ours,
    Theirs,
    OursThenTheirs,
    TheirsThenOurs,
    /// Both sides' changes ignored: keep the base.
    Base,
}

/// Builds the result text from per-chunk resolutions (`None` for Equal chunks).
pub fn result_lines<'a>(
    chunks: &[MergeChunk],
    resolutions: &[Resolution],
    base: &[&'a str],
    ours: &[&'a str],
    theirs: &[&'a str],
) -> Vec<&'a str> {
    let mut out = Vec::new();
    for (chunk, resolution) in chunks.iter().zip(resolutions) {
        match chunk {
            MergeChunk::Equal { base: range } => out.extend_from_slice(&base[range.clone()]),
            MergeChunk::Change { base: b, ours: o, theirs: t, .. } => match resolution {
                Resolution::Unresolved | Resolution::Base => out.extend_from_slice(&base[b.clone()]),
                Resolution::Ours => out.extend_from_slice(&ours[o.clone()]),
                Resolution::Theirs => out.extend_from_slice(&theirs[t.clone()]),
                Resolution::OursThenTheirs => {
                    out.extend_from_slice(&ours[o.clone()]);
                    out.extend_from_slice(&theirs[t.clone()]);
                }
                Resolution::TheirsThenOurs => {
                    out.extend_from_slice(&theirs[t.clone()]);
                    out.extend_from_slice(&ours[o.clone()]);
                }
            },
        }
    }
    out
}

/// Resolution for a non-conflicting change ("Apply non-conflicting changes").
pub fn automatic(chunk: &MergeChunk) -> Option<Resolution> {
    match chunk {
        MergeChunk::Change { conflict: false, ours_changed: true, .. } => Some(Resolution::Ours),
        MergeChunk::Change { conflict: false, theirs_changed: true, .. } => Some(Resolution::Theirs),
        _ => None,
    }
}

/// Continue / Skip / Abort for the operation in progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationStep {
    Continue,
    Skip,
    Abort,
}

pub fn step(repository: &Repository, state: RepositoryState, step: OperationStep) -> Result<String> {
    let verb = match state {
        RepositoryState::Rebasing => "rebase",
        RepositoryState::Merging => "merge",
        RepositoryState::CherryPicking => "cherry-pick",
        RepositoryState::Reverting => "revert",
        RepositoryState::Normal => anyhow::bail!("no operation in progress"),
    };
    match (state, step) {
        // `merge --continue` would open an editor; commit with the prepared message.
        (RepositoryState::Merging, OperationStep::Continue) => {
            repository.run(["commit", "--no-edit"])?;
        }
        (RepositoryState::Merging, OperationStep::Skip) => anyhow::bail!("a merge can't be skipped"),
        (_, OperationStep::Continue) => {
            repository.run([verb, "--continue"])?;
        }
        (_, OperationStep::Skip) => {
            repository.run([verb, "--skip"])?;
        }
        (_, OperationStep::Abort) => {
            repository.run([verb, "--abort"])?;
        }
    }
    Ok(match step {
        OperationStep::Continue => format!("Continued {verb}"),
        OperationStep::Skip => format!("Skipped a commit in {verb}"),
        OperationStep::Abort => format!("Aborted {verb}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<&str> {
        s.lines().collect()
    }

    #[test]
    fn separates_conflicts_from_one_sided_changes() {
        let base = lines("a\nb\nc\nd\ne");
        let ours = lines("a\nB1\nc\nd\ne\nf");
        let theirs = lines("A\nb\nc\nD2\ne");
        let chunks = chunks(&base, &ours, &theirs);
        let changes: Vec<_> = chunks
            .iter()
            .filter_map(|c| match c {
                MergeChunk::Change { base, conflict, ours_changed, theirs_changed, .. } => {
                    Some((base.clone(), *conflict, *ours_changed, *theirs_changed))
                }
                _ => None,
            })
            .collect();
        // "a"→"A" (theirs) touches "b"→"B1" (ours): one conflicting chunk, like diff3.
        assert_eq!(changes, vec![(0..2, true, true, true), (3..4, false, false, true), (5..5, false, true, false)]);

        let auto: Vec<Resolution> =
            chunks.iter().map(|c| automatic(c).unwrap_or(Resolution::Unresolved)).collect();
        let result = result_lines(&chunks, &auto, &base, &ours, &theirs);
        assert_eq!(result, vec!["a", "b", "c", "D2", "e", "f"]);
    }

    #[test]
    fn same_change_on_both_sides_is_not_a_conflict() {
        let base = lines("x\ny");
        let both = lines("x\nY");
        let chunks = chunks(&base, &both, &both);
        assert!(matches!(chunks[1], MergeChunk::Change { conflict: false, .. }));
    }

    #[test]
    fn resolutions_pick_sides() {
        let base = lines("1\n2\n3");
        let ours = lines("1\nours\n3");
        let theirs = lines("1\ntheirs\n3");
        let chunks = chunks(&base, &ours, &theirs);
        let pick = |r: Resolution| {
            let rs: Vec<Resolution> =
                chunks.iter().map(|c| if matches!(c, MergeChunk::Change { .. }) { r } else { Resolution::Unresolved }).collect();
            result_lines(&chunks, &rs, &base, &ours, &theirs).join(",")
        };
        assert_eq!(pick(Resolution::Ours), "1,ours,3");
        assert_eq!(pick(Resolution::TheirsThenOurs), "1,theirs,ours,3");
        assert_eq!(pick(Resolution::Unresolved), "1,2,3");
    }
}
