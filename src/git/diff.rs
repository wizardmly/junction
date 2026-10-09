//! Line diff for the diff viewer, computed from both file versions so the
//! side-by-side view can align lines and highlight changed words, as
//! IntelliJ's diff viewer does.

use std::ops::Range;

use anyhow::Result;
use similar::{ChangeTag, DiffOp, TextDiff};

use super::Repository;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IgnoreWhitespace {
    #[default]
    None,
    /// Ignore whitespace at line starts and ends.
    Trim,
    /// Ignore all whitespace.
    All,
    /// Ignore all whitespace, and lines that are blank.
    AllAndEmptyLines,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HighlightMode {
    Words,
    Lines,
    /// Words, with a modified block split into a change per line pair.
    Split,
    Characters,
    None,
}

impl HighlightMode {
    /// Whether changed ranges inside lines are highlighted.
    pub fn inner(self) -> bool {
        matches!(self, HighlightMode::Words | HighlightMode::Split | HighlightMode::Characters)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiffOptions {
    pub ignore_whitespace: IgnoreWhitespace,
    pub highlight: HighlightMode,
    /// Fold unchanged runs, keeping this many context lines around changes.
    pub context: Option<usize>,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self { ignore_whitespace: IgnoreWhitespace::None, highlight: HighlightMode::Words, context: Some(4) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Equal,
    Deleted,
    Inserted,
    Modified,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Side {
    /// 1-based line number.
    pub line: usize,
    pub text: String,
    /// Byte ranges of changed words, for word highlighting.
    pub changed: Vec<Range<usize>>,
    /// What each changed range is: words only on this side are Inserted
    /// (new side) or Deleted (old side); words replaced are Modified.
    pub kinds: Vec<RowKind>,
    /// The whole line is one inner fragment (a line inserted or deleted
    /// inside a modified block): IntelliJ paints it in that fragment's color.
    pub whole: Option<RowKind>,
}

/// One row of the side-by-side view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffRow {
    Line { kind: RowKind, left: Option<Side>, right: Option<Side>, change: Option<usize> },
    /// A folded run of unchanged lines; `rows` are shown when expanded.
    Fold { id: usize, rows: Vec<DiffRow> },
}

/// The line ranges (0-based) of one change block on each side.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hunk {
    pub old: Range<usize>,
    pub new: Range<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct FileDiff {
    pub rows: Vec<DiffRow>,
    /// Indexed by change number.
    pub hunks: Vec<Hunk>,
    /// Number of change blocks, for "Next Difference".
    pub changes: usize,
    pub binary: bool,
    pub inserted: usize,
    pub deleted: usize,
}

pub fn normalize(line: &str, mode: IgnoreWhitespace) -> String {
    match mode {
        IgnoreWhitespace::None => line.to_owned(),
        IgnoreWhitespace::Trim => line.trim().to_owned(),
        IgnoreWhitespace::All | IgnoreWhitespace::AllAndEmptyLines => line.chars().filter(|c| !c.is_whitespace()).collect(),
    }
}

/// A step of the line diff, in whole-file line numbers.
enum Step {
    Equal { old: usize, new: usize, len: usize },
    /// Lines that differ only in what is ignored (blank lines): shown
    /// unhighlighted and not counted as a change.
    Ignored { old: Range<usize>, new: Range<usize> },
    Change { old: Range<usize>, new: Range<usize> },
}

fn steps(old_keys: &[String], new_keys: &[String], skip_blank: bool) -> Vec<Step> {
    let change = |old: Range<usize>, new: Range<usize>| Step::Change { old, new };
    if !skip_blank {
        let old: Vec<&str> = old_keys.iter().map(String::as_str).collect();
        let new: Vec<&str> = new_keys.iter().map(String::as_str).collect();
        return similar::capture_diff_slices(similar::Algorithm::Patience, &old, &new)
            .into_iter()
            .map(|op| match op {
                DiffOp::Equal { old_index, new_index, len } => Step::Equal { old: old_index, new: new_index, len },
                DiffOp::Delete { old_index, old_len, new_index } => change(old_index..old_index + old_len, new_index..new_index),
                DiffOp::Insert { old_index, new_index, new_len } => change(old_index..old_index, new_index..new_index + new_len),
                DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                    change(old_index..old_index + old_len, new_index..new_index + new_len)
                }
            })
            .collect();
    }
    // Diff only the non-blank lines, then put the blank ones back as ignored.
    let filled = |keys: &[String]| -> Vec<usize> { (0..keys.len()).filter(|i| !keys[*i].is_empty()).collect() };
    let (old_ix, new_ix) = (filled(old_keys), filled(new_keys));
    let old: Vec<&str> = old_ix.iter().map(|i| old_keys[*i].as_str()).collect();
    let new: Vec<&str> = new_ix.iter().map(|i| new_keys[*i].as_str()).collect();
    let mut out = Vec::new();
    let (mut oi, mut ni) = (0, 0);
    let ignored = |out: &mut Vec<Step>, old: Range<usize>, new: Range<usize>| {
        if !old.is_empty() || !new.is_empty() {
            out.push(Step::Ignored { old, new });
        }
    };
    for op in similar::capture_diff_slices(similar::Algorithm::Patience, &old, &new) {
        let (o, n) = (op.old_range(), op.new_range());
        if let DiffOp::Equal { .. } = op {
            for k in 0..o.len() {
                let (a, b) = (old_ix[o.start + k], new_ix[n.start + k]);
                ignored(&mut out, oi..a, ni..b);
                out.push(Step::Equal { old: a, new: b, len: 1 });
                (oi, ni) = (a + 1, b + 1);
            }
            continue;
        }
        let span = |ix: &[usize], r: Range<usize>, at: usize| if r.is_empty() { at..at } else { ix[r.start]..ix[r.end - 1] + 1 };
        let (os, ns) = (span(&old_ix, o, oi), span(&new_ix, n, ni));
        ignored(&mut out, oi..os.start, ni..ns.start);
        (oi, ni) = (os.end, ns.end);
        out.push(Step::Change { old: os, new: ns });
    }
    ignored(&mut out, oi..old_keys.len(), ni..new_keys.len());
    out
}

pub fn compute(old: &str, new: &str, options: DiffOptions) -> FileDiff {
    if old.contains('\0') || new.contains('\0') {
        return FileDiff { binary: true, ..Default::default() };
    }
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let old_keys: Vec<String> = old_lines.iter().map(|l| normalize(l, options.ignore_whitespace)).collect();
    let new_keys: Vec<String> = new_lines.iter().map(|l| normalize(l, options.ignore_whitespace)).collect();
    let skip_blank = options.ignore_whitespace == IgnoreWhitespace::AllAndEmptyLines;

    let side = |lines: &[&str], ix: usize| Side { line: ix + 1, text: lines[ix].to_owned(), changed: Vec::new(), kinds: Vec::new(), whole: None };
    let mut rows = Vec::new();
    let mut changes = 0;
    let mut hunks = Vec::new();
    let (mut inserted, mut deleted) = (0, 0);

    for step in steps(&old_keys, &new_keys, skip_blank) {
        match step {
            Step::Equal { old, new, len } => {
                for k in 0..len {
                    rows.push(DiffRow::Line {
                        kind: RowKind::Equal,
                        left: Some(side(&old_lines, old + k)),
                        right: Some(side(&new_lines, new + k)),
                        change: None,
                    });
                }
            }
            Step::Ignored { old, new } => {
                for k in 0..old.len().max(new.len()) {
                    let left = (k < old.len()).then(|| side(&old_lines, old.start + k));
                    let right = (k < new.len()).then(|| side(&new_lines, new.start + k));
                    rows.push(DiffRow::Line { kind: RowKind::Equal, left, right, change: None });
                }
            }
            Step::Change { old, new } => {
                // "Highlight split changes": a block of line pairs becomes a change per pair.
                let pieces: Vec<(Range<usize>, Range<usize>)> = if options.highlight == HighlightMode::Split && old.len() == new.len() && old.len() > 1 {
                    (0..old.len()).map(|k| (old.start + k..old.start + k + 1, new.start + k..new.start + k + 1)).collect()
                } else {
                    vec![(old, new)]
                };
                for (old, new) in pieces {
                    hunks.push(Hunk { old: old.clone(), new: new.clone() });
                    deleted += old.len();
                    inserted += new.len();
                    let mut lefts: Vec<Side> = old.clone().map(|i| side(&old_lines, i)).collect();
                    let mut rights: Vec<Side> = new.clone().map(|i| side(&new_lines, i)).collect();
                    if !lefts.is_empty() && !rights.is_empty() && options.highlight.inner() {
                        block_fragments(&mut lefts, &mut rights, options.highlight == HighlightMode::Characters);
                    }
                    let (mut lefts, mut rights) = (lefts.into_iter(), rights.into_iter());
                    for _ in 0..old.len().max(new.len()) {
                        let (left, right) = (lefts.next(), rights.next());
                        let kind = match (&left, &right) {
                            (Some(_), Some(_)) => RowKind::Modified,
                            (Some(_), None) => RowKind::Deleted,
                            _ => RowKind::Inserted,
                        };
                        rows.push(DiffRow::Line { kind, left, right, change: Some(changes) });
                    }
                    changes += 1;
                }
            }
        }
    }

    if let Some(context) = options.context {
        rows = fold(rows, context);
    }
    FileDiff { rows, hunks, changes, binary: false, inserted, deleted }
}

/// `base` with one hunk's lines replaced by the other side's, keeping each
/// line's own terminator. `to_new` takes the new side's lines into `old`;
/// otherwise the old side's lines go back into `new`.
pub fn splice_hunk(old: &str, new: &str, hunk: &Hunk, to_new: bool) -> String {
    let (base, base_range, source, source_range) = if to_new { (old, hunk.old.clone(), new, hunk.new.clone()) } else { (new, hunk.new.clone(), old, hunk.old.clone()) };
    let source: Vec<&str> = source.split_inclusive('\n').collect();
    splice_lines(base, base_range, &source[source_range.start.min(source.len())..source_range.end.min(source.len())])
}

/// `base` with the lines in `range` replaced by `lines` (each with its own
/// terminator, the last one maybe without).
fn splice_lines(base: &str, range: Range<usize>, lines: &[&str]) -> String {
    let base: Vec<&str> = base.split_inclusive('\n').collect();
    let pieces = base[..range.start.min(base.len())].iter().chain(lines.iter()).chain(base[range.end.min(base.len())..].iter());
    let mut out = String::new();
    for piece in pieces {
        // A last line without a newline that is no longer last gets one.
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(piece);
    }
    out
}

/// Identifies a change block by its content, so a partial-commit choice
/// survives reloads that shift line numbers.
pub fn hunk_signature(old: &str, new: &str, hunk: &Hunk) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    old_lines.get(hunk.old.clone()).hash(&mut hasher);
    0xffu8.hash(&mut hasher);
    new_lines.get(hunk.new.clone()).hash(&mut hasher);
    hasher.finish()
}

/// The change blocks partial commits work with (no whitespace ignoring).
pub fn commit_hunks(old: &str, new: &str) -> Vec<Hunk> {
    compute(old, new, DiffOptions { ignore_whitespace: IgnoreWhitespace::None, highlight: HighlightMode::None, context: None }).hunks
}

/// Identifies one line of a change block left out of a commit ("Exclude
/// Lines from Commit"): the block's signature, its side and its offset.
pub fn line_id(signature: u64, new_side: bool, offset: usize) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (signature, new_side, offset).hash(&mut hasher);
    hasher.finish()
}

/// Which lines of a block are left out of the commit, per side. A block
/// excluded as a whole has every line excluded.
pub fn excluded_lines(signature: u64, hunk: &Hunk, excluded: &std::collections::HashSet<u64>) -> (Vec<bool>, Vec<bool>) {
    let all = excluded.contains(&signature);
    let side = |new_side: bool, len: usize| (0..len).map(|k| all || excluded.contains(&line_id(signature, new_side, k))).collect();
    (side(false, hunk.old.len()), side(true, hunk.new.len()))
}

/// Partial commit: `old` with every change block of `new` applied except the
/// excluded ones; a block with some lines excluded keeps the deletions and
/// leaves out the insertions those lines are. `None` when nothing is excluded.
pub fn partial_content(old: &str, new: &str, excluded: &std::collections::HashSet<u64>) -> Option<String> {
    let old_lines: Vec<&str> = old.split_inclusive('\n').collect();
    let new_lines: Vec<&str> = new.split_inclusive('\n').collect();
    let mut partial = false;
    // Bottom-up, so earlier line ranges stay valid.
    let mut content = old.to_owned();
    for hunk in commit_hunks(old, new).iter().rev() {
        let (old_out, new_out) = excluded_lines(hunk_signature(old, new, hunk), hunk, excluded);
        if !old_out.contains(&true) && !new_out.contains(&true) {
            content = splice_hunk(&content, new, hunk, true);
            continue;
        }
        partial = true;
        let kept = hunk.old.clone().zip(&old_out).filter(|(_, out)| **out).map(|(i, _)| old_lines[i]);
        let added = hunk.new.clone().zip(&new_out).filter(|(_, out)| !**out).map(|(i, _)| new_lines[i]);
        let lines: Vec<&str> = kept.chain(added).collect();
        content = splice_lines(&content, hunk.old.clone(), &lines);
    }
    partial.then_some(content)
}

/// What a diff gutter arrow does with one change block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HunkAction {
    /// Rollback the change in the working tree.
    Revert,
    /// Move the change into the index (staging area).
    Stage,
    /// Take the change back out of the index.
    Unstage,
}

/// Applies a gutter action. `old` / `new` are the versions the diff showed.
pub fn apply_hunk(repository: &Repository, revisions: &Revisions, old: &str, new: &str, hunk: &Hunk, action: HunkAction) -> Result<()> {
    let path = match revisions {
        Revisions::WorkingTree { path } | Revisions::Unstaged { path } | Revisions::Staged { path } => path,
        _ => anyhow::bail!("changes in history can't be edited"),
    };
    match (action, revisions) {
        (HunkAction::Revert, Revisions::WorkingTree { .. } | Revisions::Unstaged { .. }) => {
            let content = splice_hunk(old, new, hunk, false);
            std::fs::write(repository.root().join(path), content)?;
        }
        (HunkAction::Stage, Revisions::Unstaged { .. }) => write_index(repository, path, &splice_hunk(old, new, hunk, true), true)?,
        (HunkAction::Unstage, Revisions::Staged { .. }) => write_index(repository, path, &splice_hunk(old, new, hunk, false), false)?,
        _ => anyhow::bail!("this action doesn't apply to this diff"),
    }
    Ok(())
}

/// Stages the work tree lines `lines` (a change block against HEAD, as the
/// editor's gutter shows it): the index-vs-work-tree blocks they touch move
/// into the index, other staged changes stay.
pub fn stage_lines(repository: &Repository, path: &str, work_tree: &str, lines: Range<usize>) -> Result<()> {
    let index = repository.run(["show", &format!(":{path}")]).unwrap_or_default();
    let touched: Vec<Hunk> = commit_hunks(&index, work_tree)
        .into_iter()
        .filter(|h| {
            if h.new.is_empty() || lines.is_empty() {
                h.new.start <= lines.end && lines.start <= h.new.end
            } else {
                h.new.start < lines.end && lines.start < h.new.end
            }
        })
        .collect();
    if touched.is_empty() {
        anyhow::bail!("these lines are already staged");
    }
    let mut content = index;
    // Bottom-up, so the index lines of the blocks above don't move.
    for hunk in touched.iter().rev() {
        content = splice_hunk(&content, work_tree, hunk, true);
    }
    write_index(repository, path, &content, true)
}

/// Replaces a file's staged content. `filter` runs git's clean filters
/// (line endings), as for work tree content.
fn write_index(repository: &Repository, path: &str, content: &str, filter: bool) -> Result<()> {
    let mut args = vec!["hash-object", "-w", "--stdin"];
    let path_arg = format!("--path={path}");
    if filter {
        args.push(&path_arg);
    } else {
        args.push("--no-filters");
    }
    let hash = repository.run_with_input(&args, Some(content))?;
    let mode = repository
        .run(["ls-files", "-s", "--", path])
        .ok()
        .and_then(|line| line.split_whitespace().next().map(str::to_owned))
        .unwrap_or_else(|| "100644".into());
    repository.run(["update-index", "--add", "--cacheinfo", &format!("{mode},{},{path}", hash.trim())])?;
    Ok(())
}

/// Changed words of a block's lines on each side, compared as one text
/// (the merge tool's highlighting of each side against the base), by word
/// or with `chars` by character.
pub fn line_fragments(old: &[&str], new: &[&str], chars: bool) -> (Vec<Vec<Range<usize>>>, Vec<Vec<Range<usize>>>) {
    let sides = |lines: &[&str]| -> Vec<Side> {
        lines.iter().enumerate().map(|(i, l)| Side { line: i + 1, text: (*l).to_owned(), changed: Vec::new(), kinds: Vec::new(), whole: None }).collect()
    };
    let (mut lefts, mut rights) = (sides(old), sides(new));
    if !lefts.is_empty() && !rights.is_empty() {
        block_fragments(&mut lefts, &mut rights, chars);
    }
    let take = |sides: Vec<Side>| sides.into_iter().map(|s| s.changed).collect();
    (take(lefts), take(rights))
}

/// Inner fragments of a modified block, IntelliJ's "Highlight words":
/// the block's lines are compared word by word as one text, so a line
/// inserted in the middle of a block shows as inserted, and each changed
/// range gets a kind. Fragments are split back onto the lines they cover.
fn block_fragments(lefts: &mut [Side], rights: &mut [Side], chars: bool) {
    let join = |sides: &[Side]| sides.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join("\n");
    let (old, new) = (join(lefts), join(rights));
    let mut config = TextDiff::configure();
    config.algorithm(similar::Algorithm::Patience);
    let diff = if chars { config.diff_chars(&old, &new) } else { config.diff_unicode_words(&old, &new) };
    // Runs of deletions / insertions between equal words; a run with both is
    // a modification of those words.
    let (mut lpos, mut rpos) = (0, 0);
    let mut fragments: Vec<(Range<usize>, Range<usize>)> = Vec::new();
    let mut run: Option<(Range<usize>, Range<usize>)> = None;
    for change in diff.iter_all_changes() {
        let len = change.value().len();
        match change.tag() {
            ChangeTag::Equal => {
                fragments.extend(run.take());
                lpos += len;
                rpos += len;
            }
            ChangeTag::Delete => {
                let r = run.get_or_insert((lpos..lpos, rpos..rpos));
                r.0.end = lpos + len;
                lpos += len;
            }
            ChangeTag::Insert => {
                let r = run.get_or_insert((lpos..lpos, rpos..rpos));
                r.1.end = rpos + len;
                rpos += len;
            }
        }
    }
    fragments.extend(run.take());
    for (left, right) in fragments {
        let kind = match (left.is_empty(), right.is_empty()) {
            (false, false) => RowKind::Modified,
            (false, true) => RowKind::Deleted,
            _ => RowKind::Inserted,
        };
        spread(lefts, left, kind);
        spread(rights, right, kind);
    }
    for side in lefts.iter_mut().chain(rights.iter_mut()) {
        // A fragment spanning the whole (non-blank) line colors the line.
        if let ([range], [kind]) = (side.changed.as_slice(), side.kinds.as_slice()) {
            if *kind != RowKind::Modified && range.start == 0 && range.end >= side.text.trim_end().len() && !side.text.trim().is_empty() {
                side.whole = Some(*kind);
            }
        }
    }
}

/// Adds a fragment of the joined block text to the lines it covers.
fn spread(sides: &mut [Side], range: Range<usize>, kind: RowKind) {
    if range.is_empty() {
        return;
    }
    let mut start = 0;
    for side in sides.iter_mut() {
        let end = start + side.text.len();
        let (from, to) = (range.start.max(start), range.end.min(end));
        if from < to {
            let local = from - start..to - start;
            match (side.changed.last_mut(), side.kinds.last()) {
                (Some(last), Some(k)) if last.end == local.start && *k == kind => last.end = local.end,
                _ => {
                    side.changed.push(local);
                    side.kinds.push(kind);
                }
            }
        }
        start = end + 1;
    }
}

/// Folds runs of equal rows longer than `2 * context + 1`.
fn fold(rows: Vec<DiffRow>, context: usize) -> Vec<DiffRow> {
    let is_equal = |row: &DiffRow| matches!(row, DiffRow::Line { kind: RowKind::Equal, .. });
    let mut out = Vec::new();
    let mut run: Vec<DiffRow> = Vec::new();
    let mut fold_id = 0;
    let total = rows.len();
    let mut flush = |run: &mut Vec<DiffRow>, out: &mut Vec<DiffRow>, at_start: bool, at_end: bool| {
        let keep_head = if at_start { 0 } else { context };
        let keep_tail = if at_end { 0 } else { context };
        if run.len() > keep_head + keep_tail + 1 {
            let tail = run.split_off(run.len() - keep_tail);
            let middle = run.split_off(keep_head);
            out.append(run);
            out.push(DiffRow::Fold { id: fold_id, rows: middle });
            fold_id += 1;
            out.extend(tail);
        } else {
            out.append(run);
        }
    };
    let mut seen_change = false;
    for (ix, row) in rows.into_iter().enumerate() {
        if is_equal(&row) {
            run.push(row);
            if ix + 1 == total {
                flush(&mut run, &mut out, !seen_change, true);
            }
        } else {
            flush(&mut run, &mut out, !seen_change, false);
            seen_change = true;
            out.push(row);
        }
    }
    out
}

/// Which two versions of a file to compare.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Revisions {
    /// A file as changed by a commit, against the commit's first parent.
    Commit { hash: String, path: String, old_path: Option<String> },
    /// The working tree file against HEAD.
    WorkingTree { path: String },
    /// Staged changes: HEAD against the index.
    Staged { path: String },
    /// Unstaged changes: the index against the working tree.
    Unstaged { path: String },
    /// Any two revisions, or a revision against the working tree (`new: None`).
    Between { old: String, new: Option<String>, path: String, old_path: Option<String> },
    /// Compare With…: a working tree file against any other file on disk.
    Files { path: String, other: std::path::PathBuf },
    /// Two texts given as they are.
    Texts { path: String, old: String, new: String, old_title: String, new_title: String },
    /// Compare with Clipboard: the clipboard's text against a working tree file.
    Clipboard { path: String, text: String },
}

/// Loads both versions; a missing side (added/deleted file) is empty.
pub fn load_versions(repository: &Repository, revisions: &Revisions) -> Result<(String, String, String, String)> {
    // A submodule's recorded commit isn't an object here; git diff shows it
    // as "Subproject commit <sha>", and so do we.
    let show = |spec: String| {
        repository.run(["show", &spec]).unwrap_or_else(|_| {
            repository
                .run(["rev-parse", "--verify", "-q", &spec])
                .ok()
                .filter(|sha| repository.run(["cat-file", "-e", sha.trim()]).is_err())
                .map(|sha| format!("Subproject commit {}\n", sha.trim()))
                .unwrap_or_default()
        })
    };
    Ok(match revisions {
        Revisions::Commit { hash, path, old_path } => {
            let old_path = old_path.as_ref().unwrap_or(path);
            let has_parent = repository.run(["rev-parse", "--verify", "-q", &format!("{hash}^")]).is_ok();
            let old = if has_parent { show(format!("{hash}^:{old_path}")) } else { String::new() };
            let new = show(format!("{hash}:{path}"));
            let short = &hash[..hash.len().min(8)];
            (old, new, format!("{}^", short), short.to_owned())
        }
        Revisions::WorkingTree { path } => {
            // IntelliJ titles the base with its revision number.
            let head = repository.run(["rev-parse", "--short=8", "HEAD"]).map(|h| h.trim().to_owned()).unwrap_or_else(|_| "HEAD".into());
            let base = rename_source(repository, path).unwrap_or_else(|| path.clone());
            (show(format!("HEAD:{base}")), read_work_tree(repository, path), head, "Current version".into())
        }
        Revisions::Staged { path } => {
            let base = rename_source(repository, path).unwrap_or_else(|| path.clone());
            (show(format!("HEAD:{base}")), show(format!(":{path}")), "HEAD".into(), "Staged".into())
        }
        Revisions::Unstaged { path } => {
            (show(format!(":{path}")), read_work_tree(repository, path), "Staged".into(), "Current version".into())
        }
        Revisions::Between { old, new, path, old_path } => {
            let old_text = show(format!("{old}:{}", old_path.as_ref().unwrap_or(path)));
            let (new_text, new_title) = match new {
                Some(new) => (show(format!("{new}:{path}")), revision_title(new)),
                None => (read_work_tree(repository, path), "Current version".to_owned()),
            };
            (old_text, new_text, revision_title(old), new_title)
        }
        Revisions::Texts { old, new, old_title, new_title, .. } => (old.clone(), new.clone(), old_title.clone(), new_title.clone()),
        Revisions::Clipboard { path, text } => (text.clone(), read_work_tree(repository, path), "Clipboard".into(), path.clone()),
        Revisions::Files { path, other } => {
            let other_text = std::fs::read(other).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
            (read_work_tree(repository, path), other_text, path.clone(), other.to_string_lossy().into_owned())
        }
    })
}

/// The HEAD path a staged rename came from, so its diff compares the old
/// file with the new one (IntelliJ's "moved from") instead of showing an add.
fn rename_source(repository: &Repository, path: &str) -> Option<String> {
    if repository.run(["cat-file", "-e", &format!("HEAD:{path}")]).is_ok() {
        return None;
    }
    let status = super::status::WorkingTreeStatus::load(repository).ok()?;
    status.entries.into_iter().find(|e| e.path == path).and_then(|e| e.old_path)
}

/// A hash is shortened; branch names stay as they are.
fn revision_title(revision: &str) -> String {
    if revision.len() == 40 && revision.bytes().all(|b| b.is_ascii_hexdigit()) { revision[..8].to_owned() } else { revision.to_owned() }
}

/// Files that differ between `old` and `new` (the working tree when `None`),
/// for Compare with Local and Show Diff with Working Tree.
pub fn changed_files(repository: &Repository, old: &str, new: Option<&str>) -> Result<Vec<super::log::FileChange>> {
    let mut args = vec!["diff", "--name-status", "-z", "-M", old];
    if let Some(new) = new {
        args.push(new);
    }
    args.push("--");
    Ok(super::log::parse_name_status(&repository.run(&args)?))
}

fn read_work_tree(repository: &Repository, path: &str) -> String {
    if repository.root().join(path).join(".git").exists() {
        if let Some(text) = super::submodule::work_tree_text(repository, path) {
            return text;
        }
    }
    std::fs::read(repository.root().join(path)).map(|bytes| String::from_utf8_lossy(&bytes).into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(diff: &FileDiff) -> Vec<RowKind> {
        diff.rows
            .iter()
            .filter_map(|r| match r {
                DiffRow::Line { kind, .. } => Some(*kind),
                DiffRow::Fold { .. } => None,
            })
            .collect()
    }

    #[test]
    fn ignores_empty_lines() {
        let old = "a\nb\n\nc\n";
        let new = "a\n\n b\nc\nd\n";
        let options = DiffOptions { ignore_whitespace: IgnoreWhitespace::AllAndEmptyLines, context: None, ..Default::default() };
        let diff = compute(old, new, options);
        assert_eq!(diff.hunks, vec![Hunk { old: 4..4, new: 4..5 }]);
        // Every line of both files is still shown.
        let (mut left, mut right) = (0, 0);
        for row in &diff.rows {
            if let DiffRow::Line { left: l, right: r, .. } = row {
                left += l.is_some() as usize;
                right += r.is_some() as usize;
            }
        }
        assert_eq!((left, right), (4, 5));
    }

    #[test]
    fn splits_line_pairs() {
        let options = DiffOptions { highlight: HighlightMode::Split, context: None, ..Default::default() };
        let diff = compute("a1\nb1\nc\n", "a2\nb2\nc\n", options);
        assert_eq!(diff.changes, 2);
        let options = DiffOptions { highlight: HighlightMode::Characters, context: None, ..Default::default() };
        let diff = compute("value\n", "valve\n", options);
        let DiffRow::Line { right: Some(side), .. } = &diff.rows[0] else { panic!() };
        assert_eq!(side.changed, vec![3..4]);
    }

    #[test]
    fn commits_only_included_hunks() {
        let old = "a\nb\nc\nd\ne\nf\ng\nh\n";
        let new = "A\nb\nc\nd\ne\nf\ng\nH\n";
        let hunks = commit_hunks(old, new);
        assert_eq!(hunks.len(), 2);
        let excluded: std::collections::HashSet<u64> = [hunk_signature(old, new, &hunks[1])].into();
        assert_eq!(partial_content(old, new, &excluded).unwrap(), "A\nb\nc\nd\ne\nf\ng\nh\n");
        assert_eq!(partial_content(old, new, &Default::default()), None);
    }

    #[test]
    fn commits_only_included_lines() {
        let old = "a\nb\nc\n";
        let new = "a\nB\nX\nc\n";
        let hunk = commit_hunks(old, new).remove(0);
        let signature = hunk_signature(old, new, &hunk);
        // Keep the deletion of "b" out, and the inserted "X".
        let excluded: std::collections::HashSet<u64> = [line_id(signature, false, 0), line_id(signature, true, 1)].into();
        assert_eq!(partial_content(old, new, &excluded).unwrap(), "a\nb\nB\nc\n");
        assert_eq!(excluded_lines(signature, &hunk, &excluded), (vec![true], vec![false, true]));
    }

    #[test]
    fn splices_hunks_both_ways() {
        let old = "a\nb\nc\n";
        let new = "a\nB\nB2\nc\nd";
        let diff = compute(old, new, DiffOptions { context: None, ..Default::default() });
        assert_eq!(diff.hunks.len(), 2);
        // Revert the first change in the new version.
        assert_eq!(splice_hunk(old, new, &diff.hunks[0], false), "a\nb\nc\nd");
        // Take only the second change into the old version.
        assert_eq!(splice_hunk(old, new, &diff.hunks[1], true), "a\nb\nc\nd");
        assert_eq!(splice_hunk(old, new, &diff.hunks[0], true), "a\nB\nB2\nc\n");
    }

    #[test]
    fn pairs_replacements_and_marks_words() {
        let diff = compute("a\nlet x = 1;\nc\n", "a\nlet x = 2;\nc\nd\n", DiffOptions { context: None, ..Default::default() });
        assert_eq!(kinds(&diff), vec![RowKind::Equal, RowKind::Modified, RowKind::Equal, RowKind::Inserted]);
        assert_eq!(diff.changes, 2);
        let DiffRow::Line { left: Some(l), right: Some(r), .. } = &diff.rows[1] else { panic!() };
        assert_eq!(&l.text[l.changed[0].clone()], "1");
        assert_eq!(&r.text[r.changed[0].clone()], "2");
        assert_eq!((l.kinds[0], r.kinds[0]), (RowKind::Modified, RowKind::Modified));
    }

    #[test]
    fn line_inserted_inside_a_block_is_whole() {
        let diff = compute("a\n  x = 1\nz\n", "a\ntry {\n  x = 2\nz\n", DiffOptions { context: None, ..Default::default() });
        let sides: Vec<&Side> = diff
            .rows
            .iter()
            .filter_map(|r| match r {
                DiffRow::Line { right: Some(s), change: Some(_), .. } => Some(s),
                _ => None,
            })
            .collect();
        assert_eq!(sides[0].text, "try {");
        assert_eq!(sides[0].whole, Some(RowKind::Inserted));
        assert_eq!(sides[1].whole, None);
        assert_eq!(&sides[1].text[sides[1].changed.last().unwrap().clone()], "2");
    }

    #[test]
    fn ignores_whitespace_when_asked() {
        let options = DiffOptions { ignore_whitespace: IgnoreWhitespace::All, context: None, ..Default::default() };
        let diff = compute("fn  main() {}\n", "fn main(){}\n", options);
        assert_eq!(diff.changes, 0);
    }

    #[test]
    fn folds_long_unchanged_runs() {
        let old: String = (0..30).map(|i| format!("line {i}\n")).collect();
        let new = old.replace("line 15\n", "changed\n");
        let diff = compute(&old, &new, DiffOptions { context: Some(3), ..Default::default() });
        let folds: Vec<usize> = diff
            .rows
            .iter()
            .filter_map(|r| if let DiffRow::Fold { rows, .. } = r { Some(rows.len()) } else { None })
            .collect();
        // 15 equal lines before (12 folded, 3 kept) and 14 after (11 folded, 3 kept).
        assert_eq!(folds, vec![12, 11]);
    }
}

#[cfg(test)]
mod stage_tests {
    use super::*;
    use crate::git::GitConsole;

    #[test]
    fn stages_only_the_gutter_block() {
        let dir = std::env::temp_dir().join(format!("junction-stage-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git").current_dir(&dir).args(args).output().unwrap();
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "T"]);
        std::fs::write(dir.join("a.txt"), "1\n2\n3\n4\n5\n6\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "init"]);
        let repository = Repository::discover(&dir, GitConsole::default()).unwrap();
        let work = "1\nTWO\n3\n4\n5\nSIX\n";
        std::fs::write(dir.join("a.txt"), work).unwrap();
        let hunks = commit_hunks("1\n2\n3\n4\n5\n6\n", work);
        assert_eq!(hunks.len(), 2);
        stage_lines(&repository, "a.txt", work, hunks[1].new.clone()).unwrap();
        assert_eq!(repository.run(["show", ":a.txt"]).unwrap(), "1\n2\n3\n4\n5\nSIX\n");
        // The first block joins it; the second is already there.
        stage_lines(&repository, "a.txt", work, hunks[0].new.clone()).unwrap();
        assert_eq!(repository.run(["show", ":a.txt"]).unwrap(), work);
        assert!(stage_lines(&repository, "a.txt", work, hunks[1].new.clone()).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
