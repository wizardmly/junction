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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HighlightMode {
    Words,
    Lines,
    None,
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

fn normalize(line: &str, mode: IgnoreWhitespace) -> String {
    match mode {
        IgnoreWhitespace::None => line.to_owned(),
        IgnoreWhitespace::Trim => line.trim().to_owned(),
        IgnoreWhitespace::All => line.chars().filter(|c| !c.is_whitespace()).collect(),
    }
}

pub fn compute(old: &str, new: &str, options: DiffOptions) -> FileDiff {
    if old.contains('\0') || new.contains('\0') {
        return FileDiff { binary: true, ..Default::default() };
    }
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let old_keys: Vec<String> = old_lines.iter().map(|l| normalize(l, options.ignore_whitespace)).collect();
    let new_keys: Vec<String> = new_lines.iter().map(|l| normalize(l, options.ignore_whitespace)).collect();
    let old_refs: Vec<&str> = old_keys.iter().map(String::as_str).collect();
    let new_refs: Vec<&str> = new_keys.iter().map(String::as_str).collect();
    let ops = similar::capture_diff_slices(similar::Algorithm::Patience, &old_refs, &new_refs);

    let side = |lines: &[&str], ix: usize| Side { line: ix + 1, text: lines[ix].to_owned(), changed: Vec::new() };
    let mut rows = Vec::new();
    let mut changes = 0;
    let mut hunks = Vec::new();
    let (mut inserted, mut deleted) = (0, 0);

    for op in ops {
        match op {
            DiffOp::Equal { old_index, new_index, len } => {
                for k in 0..len {
                    rows.push(DiffRow::Line {
                        kind: RowKind::Equal,
                        left: Some(side(&old_lines, old_index + k)),
                        right: Some(side(&new_lines, new_index + k)),
                        change: None,
                    });
                }
            }
            DiffOp::Delete { old_index, old_len, new_index } => {
                hunks.push(Hunk { old: old_index..old_index + old_len, new: new_index..new_index });
                deleted += old_len;
                for k in 0..old_len {
                    rows.push(DiffRow::Line { kind: RowKind::Deleted, left: Some(side(&old_lines, old_index + k)), right: None, change: Some(changes) });
                }
                changes += 1;
            }
            DiffOp::Insert { old_index, new_index, new_len } => {
                hunks.push(Hunk { old: old_index..old_index, new: new_index..new_index + new_len });
                inserted += new_len;
                for k in 0..new_len {
                    rows.push(DiffRow::Line { kind: RowKind::Inserted, left: None, right: Some(side(&new_lines, new_index + k)), change: Some(changes) });
                }
                changes += 1;
            }
            DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                hunks.push(Hunk { old: old_index..old_index + old_len, new: new_index..new_index + new_len });
                deleted += old_len;
                inserted += new_len;
                for k in 0..old_len.max(new_len) {
                    let mut left = (k < old_len).then(|| side(&old_lines, old_index + k));
                    let mut right = (k < new_len).then(|| side(&new_lines, new_index + k));
                    let kind = match (&left, &right) {
                        (Some(_), Some(_)) => RowKind::Modified,
                        (Some(_), None) => RowKind::Deleted,
                        _ => RowKind::Inserted,
                    };
                    if let (Some(l), Some(r), HighlightMode::Words) = (&mut left, &mut right, options.highlight) {
                        let (lc, rc) = word_changes(&l.text, &r.text);
                        l.changed = lc;
                        r.changed = rc;
                    }
                    rows.push(DiffRow::Line { kind, left, right, change: Some(changes) });
                }
                changes += 1;
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
    let old_lines: Vec<&str> = old.split_inclusive('\n').collect();
    let new_lines: Vec<&str> = new.split_inclusive('\n').collect();
    let (base, base_range, source, source_range) = if to_new {
        (&old_lines, hunk.old.clone(), &new_lines, hunk.new.clone())
    } else {
        (&new_lines, hunk.new.clone(), &old_lines, hunk.old.clone())
    };
    let pieces = base[..base_range.start.min(base.len())]
        .iter()
        .chain(source[source_range.start.min(source.len())..source_range.end.min(source.len())].iter())
        .chain(base[base_range.end.min(base.len())..].iter());
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

/// Partial commit: `old` with every change block of `new` applied except the
/// excluded ones. `None` when nothing is excluded.
pub fn partial_content(old: &str, new: &str, excluded: &std::collections::HashSet<u64>) -> Option<String> {
    let hunks = commit_hunks(old, new);
    let keep: Vec<&Hunk> = hunks.iter().filter(|h| !excluded.contains(&hunk_signature(old, new, h))).collect();
    if keep.len() == hunks.len() {
        return None;
    }
    // Bottom-up, so earlier line ranges stay valid.
    let mut content = old.to_owned();
    for hunk in keep.into_iter().rev() {
        content = splice_hunk(&content, new, hunk, true);
    }
    Some(content)
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

/// Byte ranges that differ between two lines, word by word.
fn word_changes(old: &str, new: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let diff = TextDiff::configure().algorithm(similar::Algorithm::Patience).diff_unicode_words(old, new);
    let (mut left, mut right) = (Vec::new(), Vec::new());
    let (mut lpos, mut rpos) = (0, 0);
    for change in diff.iter_all_changes() {
        let len = change.value().len();
        match change.tag() {
            ChangeTag::Equal => {
                lpos += len;
                rpos += len;
            }
            ChangeTag::Delete => {
                push_range(&mut left, lpos..lpos + len);
                lpos += len;
            }
            ChangeTag::Insert => {
                push_range(&mut right, rpos..rpos + len);
                rpos += len;
            }
        }
    }
    (left, right)
}

fn push_range(ranges: &mut Vec<Range<usize>>, range: Range<usize>) {
    match ranges.last_mut() {
        Some(last) if last.end == range.start => last.end = range.end,
        _ => ranges.push(range),
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
}

/// Loads both versions; a missing side (added/deleted file) is empty.
pub fn load_versions(repository: &Repository, revisions: &Revisions) -> Result<(String, String, String, String)> {
    let show = |spec: String| repository.run(["show", &spec]).unwrap_or_default();
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
            (show(format!("HEAD:{path}")), read_work_tree(repository, path), "HEAD".into(), "Your version".into())
        }
        Revisions::Staged { path } => {
            (show(format!("HEAD:{path}")), show(format!(":{path}")), "HEAD".into(), "Staged".into())
        }
        Revisions::Unstaged { path } => {
            (show(format!(":{path}")), read_work_tree(repository, path), "Staged".into(), "Your version".into())
        }
        Revisions::Between { old, new, path, old_path } => {
            let old_text = show(format!("{old}:{}", old_path.as_ref().unwrap_or(path)));
            let (new_text, new_title) = match new {
                Some(new) => (show(format!("{new}:{path}")), revision_title(new)),
                None => (read_work_tree(repository, path), "Your version".to_owned()),
            };
            (old_text, new_text, revision_title(old), new_title)
        }
    })
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
