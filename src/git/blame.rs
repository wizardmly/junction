//! `git blame --porcelain`, for Annotate with Git Blame.

use std::collections::HashMap;

use anyhow::Result;

use super::Repository;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlameCommit {
    pub hash: String,
    pub author: String,
    pub author_email: String,
    /// Unix seconds.
    pub author_time: i64,
    pub summary: String,
    /// The file's path in that commit (it may have been renamed since).
    pub path: String,
    /// The commit's parent and the file's path there, when the file
    /// existed before it: "Annotate Previous Revision" (none for the commit
    /// that added the file, or a root commit).
    pub previous: Option<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlameLine {
    /// Index into `Blame::commits`.
    pub commit: usize,
    pub text: String,
    /// Line number in the commit that last changed it (for "Annotate Previous Revision").
    pub original_line: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Blame {
    pub commits: Vec<BlameCommit>,
    pub lines: Vec<BlameLine>,
}

impl Blame {
    #[cfg(test)]
    pub fn commit_of(&self, line: usize) -> Option<&BlameCommit> {
        self.lines.get(line).and_then(|l| self.commits.get(l.commit))
    }
}

/// Uncommitted lines are reported with an all-zero hash.
pub fn is_uncommitted(hash: &str) -> bool {
    hash.bytes().all(|b| b == b'0')
}

/// Blames `path` at `revision` (the work tree when `None`).
pub fn blame(repository: &Repository, path: &str, revision: Option<&str>) -> Result<Blame> {
    let mut args = vec!["blame", "--porcelain"];
    if let Some(revision) = revision {
        args.push(revision);
    }
    args.extend(["--", path]);
    Ok(parse(&repository.run(&args)?))
}

/// Blames the editor's text of the working-tree `path` (`--contents`), so
/// the annotations follow unsaved edits; new lines are "Not Committed Yet".
pub fn blame_contents(repository: &Repository, path: &str, contents: &str) -> Result<Blame> {
    Ok(parse(&repository.run_with_input(["blame", "--porcelain", "--contents", "-", "--", path], Some(contents))?))
}

pub(crate) fn parse(output: &str) -> Blame {
    let mut blame = Blame::default();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut lines = output.lines();
    while let Some(header) = lines.next() {
        // "<hash> <original line> <final line> [<group size>]"
        let mut parts = header.split(' ');
        let (Some(hash), Some(original)) = (parts.next(), parts.next()) else { continue };
        if hash.len() < 40 {
            continue;
        }
        let commit = *index.entry(hash.to_owned()).or_insert_with(|| {
            blame.commits.push(BlameCommit {
                hash: hash.to_owned(),
                author: String::new(),
                author_email: String::new(),
                author_time: 0,
                summary: String::new(),
                path: String::new(),
                previous: None,
            });
            blame.commits.len() - 1
        });
        // Metadata lines (only the first time a commit appears), then the content line.
        for line in lines.by_ref() {
            if let Some(text) = line.strip_prefix('\t') {
                blame.lines.push(BlameLine { commit, text: text.to_owned(), original_line: original.parse().unwrap_or(0) });
                break;
            }
            let entry = &mut blame.commits[commit];
            if let Some(v) = line.strip_prefix("author ") {
                entry.author = v.to_owned();
            } else if let Some(v) = line.strip_prefix("author-mail ") {
                entry.author_email = v.trim_matches(|c| c == '<' || c == '>').to_owned();
            } else if let Some(v) = line.strip_prefix("author-time ") {
                entry.author_time = v.parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("summary ") {
                entry.summary = v.to_owned();
            } else if let Some(v) = line.strip_prefix("filename ") {
                entry.path = v.to_owned();
            } else if let Some((hash, path)) = line.strip_prefix("previous ").and_then(|v| v.split_once(' ')) {
                entry.previous = Some((hash.to_owned(), path.to_owned()));
            }
        }
    }
    blame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_with_repeated_commits() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let output = format!(
            "{a} 1 1 2\nauthor Alice\nauthor-mail <alice@x>\nauthor-time 100\nsummary First\nfilename f\n\tline one\n\
             {a} 2 2\n\tline two\n\
             {b} 1 3 1\nauthor Bob\nauthor-mail <bob@x>\nauthor-time 200\nsummary Second\nprevious {a} old\nfilename f\n\tline three\n"
        );
        let blame = parse(&output);
        assert_eq!(blame.commits.len(), 2);
        assert_eq!(blame.lines.len(), 3);
        assert_eq!(blame.commit_of(1).unwrap().author, "Alice");
        assert_eq!(blame.commit_of(2).unwrap().summary, "Second");
        assert_eq!(blame.lines[2].text, "line three");
        assert_eq!(blame.commits[0].author_email, "alice@x");
        assert_eq!(blame.commits[1].path, "f");
        assert_eq!(blame.commits[0].previous, None);
        assert_eq!(blame.commits[1].previous, Some((a.clone(), "old".to_owned())));
    }
}
