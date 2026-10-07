use anyhow::Result;

use super::Repository;

/// One row of the log, with only what the table and graph need.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    /// Unix seconds.
    pub author_time: i64,
    pub subject: String,
}

impl Commit {
    pub fn short_hash(&self) -> &str {
        &self.hash[..self.hash.len().min(8)]
    }

}

/// What the Log's filter bar selects.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogFilter {
    /// Branch or tag names; empty means all refs (`--all`).
    pub branches: Vec<String>,
    /// Matches the commit message or a hash prefix.
    pub text: String,
    pub regex: bool,
    pub match_case: bool,
    pub author: Option<String>,
    /// Passed to `--since`, e.g. `7 days ago`.
    pub since: Option<String>,
    pub paths: Vec<String>,
}

const FIELD: char = '\u{1f}';
const RECORD: char = '\u{1e}';

/// How many commits the Log shows before the rest is loaded in the background.
pub const FIRST_PAGE: usize = 1000;

pub fn load_log(repository: &Repository, filter: &LogFilter, limit: Option<usize>) -> Result<Vec<Commit>> {
    let mut args: Vec<String> = vec![
        "log".into(),
        // IntelliJ's default is IntelliSort; topological order keeps branches
        // contiguous in the graph the same way.
        "--topo-order".into(),
        "--no-color".into(),
        format!("--format={RECORD}%H{FIELD}%P{FIELD}%an{FIELD}%ae{FIELD}%at{FIELD}%s"),
    ];
    if filter.branches.is_empty() {
        args.push("--all".into());
        // Stashes are listed separately, as in IntelliJ.
        args.push("--exclude=refs/stash".into());
    }
    if let Some(author) = &filter.author {
        args.push(format!("--author={author}"));
    }
    if let Some(since) = &filter.since {
        args.push(format!("--since={since}"));
    }
    let text = filter.text.trim();
    let hash_search = !text.is_empty() && text.len() >= 4 && text.chars().all(|c| c.is_ascii_hexdigit());
    if let Some(limit) = limit.filter(|_| !hash_search) {
        args.push(format!("--max-count={limit}"));
    }
    if !text.is_empty() && !hash_search {
        args.push(format!("--grep={text}"));
        if !filter.regex {
            args.push("--fixed-strings".into());
        }
        if !filter.match_case {
            args.push("--regexp-ignore-case".into());
        }
    }
    args.extend(filter.branches.iter().cloned());
    // An empty repository has no HEAD; log of nothing is an empty list.
    if filter.branches.is_empty() && repository.run(["rev-parse", "--verify", "-q", "HEAD"]).is_err()
        && repository.run(["for-each-ref", "--count=1"]).map(|o| o.trim().is_empty()).unwrap_or(true)
    {
        return Ok(Vec::new());
    }
    args.push("--".into());
    args.extend(filter.paths.iter().cloned());

    let output = repository.run(&args)?;
    let mut commits = parse_log(&output);
    if hash_search {
        // A hex string may be a hash prefix or a word in a message; IntelliJ matches both.
        let needle = text.to_ascii_lowercase();
        commits.retain(|c| c.hash.starts_with(&needle) || c.subject.to_lowercase().contains(&needle));
    }
    Ok(commits)
}

pub(crate) fn parse_log(output: &str) -> Vec<Commit> {
    output
        .split(RECORD)
        .filter_map(|record| {
            let record = record.trim_end_matches('\n');
            if record.is_empty() {
                return None;
            }
            let mut fields = record.splitn(6, FIELD);
            let hash = fields.next()?.to_owned();
            let parents = fields.next()?.split_whitespace().map(str::to_owned).collect();
            let author_name = fields.next()?.to_owned();
            let author_email = fields.next()?.to_owned();
            let author_time = fields.next()?.parse().unwrap_or(0);
            let subject = fields.next().unwrap_or_default().to_owned();
            Some(Commit { hash, parents, author_name, author_email, author_time, subject })
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FileChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
}

impl FileChangeKind {
    fn from_status(status: &str) -> Self {
        match status.chars().next() {
            Some('A') => Self::Added,
            Some('D') => Self::Deleted,
            Some('R') => Self::Renamed,
            Some('C') => Self::Copied,
            Some('T') => Self::TypeChanged,
            _ => Self::Modified,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    pub kind: FileChangeKind,
    pub path: String,
    /// The source path for renames and copies.
    pub old_path: Option<String>,
}

/// Everything the details pane shows for one selected commit.
#[derive(Clone, Debug)]
pub struct CommitDetails {
    pub hash: String,
    pub author_name: String,
    pub author_email: String,
    pub author_time: i64,
    pub committer_name: String,
    pub committer_email: String,
    pub committer_time: i64,
    pub message: String,
    pub changes: Vec<FileChange>,
    /// Branches that contain this commit ("In 3 branches: main, …").
    pub containing_branches: Vec<String>,
}

pub fn load_details(repository: &Repository, hash: &str) -> Result<CommitDetails> {
    let header = repository.run([
        "show",
        "-s",
        &format!("--format=%H{FIELD}%P{FIELD}%an{FIELD}%ae{FIELD}%at{FIELD}%cn{FIELD}%ce{FIELD}%ct{FIELD}%B"),
        hash,
    ])?;
    let mut fields = header.splitn(9, FIELD);
    let mut next = || fields.next().unwrap_or_default().to_owned();
    let hash = next();
    let parents: Vec<String> = next().split_whitespace().map(str::to_owned).collect();
    let author_name = next();
    let author_email = next();
    let author_time = next().parse().unwrap_or(0);
    let committer_name = next();
    let committer_email = next();
    let committer_time = next().parse().unwrap_or(0);
    let message = next().trim_end().to_owned();

    // Like IntelliJ, a merge commit shows its changes against the first parent.
    let mut diff_args = vec!["diff-tree", "--no-commit-id", "-r", "-M", "--name-status", "-z"];
    if parents.is_empty() {
        diff_args.push("--root");
    } else if parents.len() > 1 {
        diff_args.extend(["-m", "--first-parent"]);
    }
    diff_args.push(&hash);
    let changes = parse_name_status(&repository.run(&diff_args)?);

    let containing_branches = repository
        .run(["branch", "-a", "--contains", &hash, "--format=%(refname:short)"])
        .map(|output| output.lines().filter(|l| !l.ends_with("/HEAD")).map(str::to_owned).collect())
        .unwrap_or_default();

    Ok(CommitDetails {
        hash,
        author_name,
        author_email,
        author_time,
        committer_name,
        committer_email,
        committer_time,
        message,
        changes,
        containing_branches,
    })
}

/// Parses `git diff-tree --name-status -z` output.
pub(crate) fn parse_name_status(output: &str) -> Vec<FileChange> {
    let mut parts = output.split('\0').filter(|p| !p.is_empty());
    let mut changes = Vec::new();
    while let Some(status) = parts.next() {
        let kind = FileChangeKind::from_status(status);
        let (old_path, path) = if matches!(kind, FileChangeKind::Renamed | FileChangeKind::Copied) {
            let old = parts.next().map(str::to_owned);
            (old, parts.next())
        } else {
            (None, parts.next())
        };
        if let Some(path) = path {
            changes.push(FileChange { kind, path: path.to_owned(), old_path });
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_log_records() {
        let output = format!(
            "{RECORD}abc{FIELD}p1 p2{FIELD}Jane{FIELD}j@x{FIELD}100{FIELD}Merge branch 'x'\n\n{RECORD}p1{FIELD}{FIELD}Jane{FIELD}j@x{FIELD}50{FIELD}Initial\n"
        );
        let commits = parse_log(&output);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].parents, vec!["p1", "p2"]);
        assert!(commits[1].parents.is_empty());
        assert_eq!(commits[1].subject, "Initial");
    }

    #[test]
    fn parses_renames() {
        let changes = parse_name_status("M\0a.rs\0R100\0old.rs\0new.rs\0A\0b.rs\0");
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[1].kind, FileChangeKind::Renamed);
        assert_eq!(changes[1].old_path.as_deref(), Some("old.rs"));
        assert_eq!(changes[1].path, "new.rs");
    }
}
