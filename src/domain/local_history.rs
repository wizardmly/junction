//! Local History, as IntelliJ keeps it: a copy of a file's text each time it
//! is saved (or changed outside), and labels the user puts on the project;
//! kept for the last five working days.

use std::collections::BTreeSet;

/// IntelliJ's default: `localHistory.daysToKeep`, counted in days that saw changes.
pub const WORKING_DAYS_TO_KEEP: usize = 5;
/// Bigger files are not kept, as IntelliJ skips large and binary content.
pub const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Saved in the editor.
    Saved,
    /// Its text before something else (a revert) replaced it.
    Before,
    /// A label put on the whole project.
    Label,
    /// Changed outside the editor (a Git operation, another program):
    /// IntelliJ's "External change".
    External,
}

impl Kind {
    pub fn key(self) -> &'static str {
        match self {
            Kind::Saved => "saved",
            Kind::Before => "before",
            Kind::Label => "label",
            Kind::External => "external",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "saved" => Kind::Saved,
            "before" => Kind::Before,
            "label" => Kind::Label,
            "external" => Kind::External,
            _ => return None,
        })
    }
}

/// One recorded change: a file's text (by content hash) or a label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Revision {
    /// Unix milliseconds.
    pub time: i64,
    pub kind: Kind,
    /// The file relative to the project; empty for a label.
    pub path: String,
    /// The text's hash; empty for a label.
    pub content: String,
    /// The label, or what made the change ("Reverted to …").
    pub label: String,
}

impl Revision {
    pub fn line(&self) -> String {
        format!("{}\t{}\t{}\t{}\t{}", self.time, self.kind.key(), self.path, self.content, self.label.replace(['\t', '\n'], " "))
    }

    pub fn parse(line: &str) -> Option<Self> {
        let mut f = line.splitn(5, '\t');
        Some(Self { time: f.next()?.parse().ok()?, kind: Kind::from_key(f.next()?)?, path: f.next()?.to_owned(), content: f.next()?.to_owned(), label: f.next().unwrap_or("").to_owned() })
    }
}

/// The storage behind Local History.
pub trait HistoryStore {
    fn revisions(&self) -> Vec<Revision>;
    fn append(&self, revision: &Revision) -> Result<(), String>;
    /// Keeps only `kept` (pruning) and drops content nothing refers to.
    fn rewrite(&self, kept: &[Revision]) -> Result<(), String>;
    fn put_content(&self, text: &str) -> Result<String, String>;
    fn content(&self, hash: &str) -> Option<String>;
}

/// What survives pruning: the revisions of the last `days` calendar days
/// that saw changes (`day` maps a time to its local day).
pub fn keep_working_days(revisions: &[Revision], days: usize, day: impl Fn(i64) -> i64) -> Vec<Revision> {
    let working: BTreeSet<i64> = revisions.iter().map(|r| day(r.time)).collect();
    let Some(&first_kept) = working.iter().rev().take(days).last() else { return Vec::new() };
    revisions.iter().filter(|r| day(r.time) >= first_kept).cloned().collect()
}

/// A file's history, newest first: its own revisions and the project's
/// labels in between.
pub fn file_history(revisions: &[Revision], path: &str) -> Vec<Revision> {
    let mut out: Vec<Revision> = revisions.iter().filter(|r| r.path == path || r.kind == Kind::Label).cloned().collect();
    out.sort_by_key(|r| std::cmp::Reverse(r.time));
    // Labels older than the file's first revision tell nothing about it.
    while out.last().is_some_and(|r| r.kind == Kind::Label) {
        out.pop();
    }
    out
}

/// Whether `path` is in the folder `dir` (empty: the project).
pub fn is_under(path: &str, dir: &str) -> bool {
    dir.is_empty() || path == dir || path.strip_prefix(dir).is_some_and(|rest| rest.starts_with('/'))
}

/// A folder's history (`dir` empty: the whole project's), newest first:
/// the versions of its files and the project's labels in between.
pub fn folder_history(revisions: &[Revision], dir: &str) -> Vec<Revision> {
    let mut out: Vec<Revision> = revisions.iter().filter(|r| r.kind == Kind::Label || is_under(&r.path, dir)).cloned().collect();
    out.sort_by_key(|r| std::cmp::Reverse(r.time));
    while out.last().is_some_and(|r| r.kind == Kind::Label) {
        out.pop();
    }
    out
}

/// The folder's files as they were at `time` that differ from their newest
/// version: (path, content hash then), by path. A file with no version
/// yet at that time is left out: its text then isn't known.
pub fn changed_since(revisions: &[Revision], dir: &str, time: i64) -> Vec<(String, String)> {
    use std::collections::BTreeMap;
    // Per file: (newest version at or before `time`, newest overall).
    let mut files: BTreeMap<&str, (Option<&Revision>, &Revision)> = BTreeMap::new();
    for r in revisions.iter().filter(|r| r.kind != Kind::Label && is_under(&r.path, dir)) {
        let entry = files.entry(r.path.as_str()).or_insert((None, r));
        if r.time >= entry.1.time {
            entry.1 = r;
        }
        if r.time <= time && entry.0.is_none_or(|then| r.time >= then.time) {
            entry.0 = Some(r);
        }
    }
    files
        .into_iter()
        .filter_map(|(path, (then, newest))| then.filter(|then| then.content != newest.content).map(|then| (path.to_owned(), then.content.clone())))
        .collect()
}

/// Show History for Selection, as IntelliJ's `Block.createPreviousBlock`:
/// where the lines `start..end` of `newer` were in `older`. A block edge
/// inside a change takes the whole change.
pub fn previous_block(newer: &str, start: usize, end: usize, older: &str) -> (usize, usize) {
    use similar::DiffOp;
    let diff = similar::TextDiff::from_lines(older, newer);
    let ops = diff.ops();
    // The old range of the change holding new line `line`, or its own line.
    let find = |line: usize| -> Option<(usize, usize)> {
        ops.iter().find_map(|op| match *op {
            DiffOp::Equal { old_index, new_index, len } if (new_index..new_index + len).contains(&line) => {
                let old = old_index + line - new_index;
                Some((old, old + 1))
            }
            DiffOp::Insert { old_index, new_index, new_len } if (new_index..new_index + new_len).contains(&line) => Some((old_index, old_index)),
            DiffOp::Replace { old_index, old_len, new_index, new_len } if (new_index..new_index + new_len).contains(&line) => Some((old_index, old_index + old_len)),
            _ => None,
        })
    };
    if start >= end {
        return find(start).map_or((0, 0), |(s, _)| (s, s));
    }
    let Some((old_start, _)) = find(start) else { return (0, 0) };
    let Some((_, old_end)) = find(end - 1) else { return (old_start, old_start) };
    (old_start, old_end.max(old_start))
}

/// The lines `start..end` of `text`, with their line ends.
pub fn block_text(text: &str, start: usize, end: usize) -> String {
    text.split_inclusive('\n').skip(start).take(end.saturating_sub(start)).collect()
}

/// `text` with its lines `start..end` replaced by `block` (Revert Selection).
pub fn replace_block(text: &str, start: usize, end: usize, block: &str) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut out: String = lines.iter().take(start).copied().collect();
    out.push_str(block);
    // A block from the file's last line has no line end before what follows.
    if !block.is_empty() && !block.ends_with('\n') && end < lines.len() {
        out.push('\n');
    }
    out.extend(lines.iter().skip(end).copied());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rev(time: i64, kind: Kind, path: &str) -> Revision {
        Revision { time, kind, path: path.into(), content: if kind == Kind::Label { String::new() } else { "h".into() }, label: String::new() }
    }

    #[test]
    fn follows_a_block_back() {
        let older = "a\nb\nc\nd\ne\n";
        // "x" inserted before the block; "c" changed to "C"; "d" kept.
        let newer = "a\nx\nb\nC\nd\ne\n";
        assert_eq!(previous_block(newer, 2, 5, older), (1, 4));
        assert_eq!(block_text(older, 1, 4), "b\nc\nd\n");
        // A block made of inserted lines was empty then.
        assert_eq!(previous_block(newer, 1, 2, older), (1, 1));
        // An edge inside a change takes all of it.
        let newer = "a\nB1\nB2\nc\nd\ne\n";
        assert_eq!(previous_block(newer, 2, 4, older), (1, 3));
        assert_eq!(replace_block("1\n2\n3\n", 1, 2, "two\nzwei\n"), "1\ntwo\nzwei\n3\n");
        assert_eq!(replace_block("1\n2\n3", 2, 3, "three"), "1\n2\nthree");
        assert_eq!(replace_block("1\n2\n3\n", 0, 1, "one"), "one\n2\n3\n");
    }

    #[test]
    fn round_trips() {
        let r = Revision { time: 5, kind: Kind::Label, path: String::new(), content: String::new(), label: "before\trefactor".into() };
        assert_eq!(Revision::parse(&r.line()).unwrap().label, "before refactor");
    }

    #[test]
    fn keeps_five_working_days() {
        // Days 1, 2, 10, 11, 12, 20: the last five working days start at day 2.
        let revs: Vec<Revision> = [1, 2, 10, 11, 12, 20].iter().map(|d| rev(*d, Kind::Saved, "a")).collect();
        let kept = keep_working_days(&revs, 5, |t| t);
        assert_eq!(kept.iter().map(|r| r.time).collect::<Vec<_>>(), vec![2, 10, 11, 12, 20]);
    }

    #[test]
    fn file_history_with_labels() {
        let revs = vec![rev(1, Kind::Label, ""), rev(2, Kind::Saved, "a"), rev(3, Kind::Saved, "b"), rev(4, Kind::Label, ""), rev(5, Kind::Saved, "a")];
        let h = file_history(&revs, "a");
        assert_eq!(h.iter().map(|r| r.time).collect::<Vec<_>>(), vec![5, 4, 2]);
    }

    #[test]
    fn folder_history_and_changes() {
        let mut revs = vec![rev(1, Kind::Saved, "src/a"), rev(2, Kind::Saved, "src/b"), rev(3, Kind::Label, ""), rev(4, Kind::Saved, "src/a"), rev(5, Kind::Saved, "srcx/c")];
        revs[3].content = "h2".into();
        assert_eq!(folder_history(&revs, "src").iter().map(|r| r.time).collect::<Vec<_>>(), vec![4, 3, 2, 1]);
        assert_eq!(folder_history(&revs, "").len(), 5);
        // At 3, src/a was "h"; it is "h2" now. src/b is unchanged.
        assert_eq!(changed_since(&revs, "src", 3), vec![("src/a".to_owned(), "h".to_owned())]);
        assert!(changed_since(&revs, "src", 4).is_empty());
        // Before src/b had a version: only src/a is known.
        assert_eq!(changed_since(&revs, "", 1).len(), 1);
        assert!(is_under("src/a", "src") && !is_under("srcx/c", "src") && is_under("x", ""));
    }
}
