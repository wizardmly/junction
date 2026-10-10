use anyhow::Result;

use super::Repository;

/// One row of the log, with only what the table and graph need.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub parents: Vec<String>,
    /// Shared between a person's commits.
    pub author_name: std::sync::Arc<str>,
    pub author_email: std::sync::Arc<str>,
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
    /// Authors (names or emails); a commit by any of them matches.
    pub authors: Vec<String>,
    /// Passed to `--since`, e.g. `7 days ago`.
    pub since: Option<String>,
    /// Passed to `--until` (a custom date range's end).
    pub until: Option<String>,
    pub paths: Vec<String>,
    /// History for Selection: 1-based inclusive line range in the single path (`git log -L`).
    pub lines: Option<(usize, usize)>,
    /// View Options › Sort by date (`--date-order`); set from the settings.
    pub date_order: bool,
}

const FIELD: char = '\u{1f}';
const RECORD: char = '\u{1e}';

/// How many commits the Log shows before the rest is loaded in the background.
pub const FIRST_PAGE: usize = 1000;

pub fn load_log(repository: &Repository, filter: &LogFilter, limit: Option<usize>) -> Result<Vec<Commit>> {
    load(repository, filter, limit, false)
}

/// The first page shown while the whole log loads. Sorting needs git to
/// walk the history up to the oldest ref first, which takes seconds in a
/// large repository unless git has a commit-graph; without one the page
/// comes in commit date order. Tags pointing deep into the history slow
/// the walk too, so the page leaves them out (it is replaced moments later).
pub fn load_first_page(repository: &Repository, filter: &LogFilter) -> Result<Vec<Commit>> {
    load(repository, filter, Some(FIRST_PAGE), true)
}

fn load(repository: &Repository, filter: &LogFilter, limit: Option<usize>, first_page: bool) -> Result<Vec<Commit>> {
    let sorted = !first_page || has_commit_graph(repository);
    let mut args: Vec<String> = vec!["log".into()];
    if sorted {
        // IntelliJ's default is IntelliSort; topological order keeps branches
        // contiguous in the graph the same way.
        args.push(if filter.date_order { "--date-order" } else { "--topo-order" }.into());
    }
    args.push("--no-color".into());
    args.push(format!("--format={RECORD}%H{FIELD}%P{FIELD}%an{FIELD}%ae{FIELD}%at{FIELD}%s"));
    if filter.branches.is_empty() && first_page {
        args.extend(["--branches".into(), "--remotes".into(), "HEAD".into()]);
    } else if filter.branches.is_empty() {
        // Stashes and shelves are listed separately, as in IntelliJ.
        // (`--exclude` only applies to the `--all` after it.)
        args.push("--exclude=refs/stash".into());
        args.push("--exclude=refs/junction/*".into());
        args.push("--all".into());
    }
    for author in &filter.authors {
        args.push(format!("--author={author}"));
    }
    if let Some(since) = &filter.since {
        args.push(format!("--since={since}"));
    }
    if let Some(until) = &filter.until {
        args.push(format!("--until={until}"));
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
    match (filter.lines, filter.paths.as_slice()) {
        // History for Selection: commits that touched these lines (`-s` drops the patches).
        (Some((start, end)), [path]) => {
            args.push(format!("-L{start},{end}:{path}"));
            args.push("-s".into());
        }
        _ => {
            // File History follows renames, like IntelliJ's.
            // (`--follow` only works for a single file, not a folder.)
            if let [path] = filter.paths.as_slice() {
                if !repository.root().join(path).is_dir() {
                    args.push("--follow".into());
                }
            }
            args.push("--".into());
            args.extend(filter.paths.iter().cloned());
        }
    }

    let mut commits = repository.run_streaming(&args, |out| {
        let mut commits = Vec::new();
        let mut record = Vec::new();
        let mut people = People::default();
        loop {
            record.clear();
            match out.read_until(RECORD as u8, &mut record) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            if record.last() == Some(&(RECORD as u8)) {
                record.pop();
            }
            if let Some(commit) = parse_record(&String::from_utf8_lossy(&record), &mut people) {
                commits.push(commit);
            }
        }
        commits.shrink_to_fit();
        commits
    })?;
    if hash_search {
        // A hex string may be a hash prefix or a word in a message; IntelliJ matches both.
        let needle = text.to_ascii_lowercase();
        commits.retain(|c| c.hash.starts_with(&needle) || c.subject.to_lowercase().contains(&needle));
    }
    Ok(commits)
}

fn objects_dir(repository: &Repository) -> Option<std::path::PathBuf> {
    // Linked worktrees keep their objects in the main repository.
    let git_dir = repository.git_dir();
    let common = std::fs::read_to_string(git_dir.join("commondir")).ok().map(|c| git_dir.join(c.trim()));
    Some(common.unwrap_or_else(|| git_dir.to_path_buf()).join("objects"))
}

/// Whether git keeps a commit-graph, which lets it sort the log without
/// reading the whole history first.
pub fn has_commit_graph(repository: &Repository) -> bool {
    objects_dir(repository).is_some_and(|objects| {
        let info = objects.join("info");
        info.join("commit-graph").is_file() || info.join("commit-graphs").join("commit-graph-chain").is_file()
    })
}

/// Writes or extends git's commit-graph (what `git gc` and `git maintenance`
/// do), so the next log loads sorted in moments. Adding a layer for new
/// commits is cheap; the first write of a large history takes a while.
pub fn write_commit_graph(repository: &Repository) {
    let disabled = repository.run(["config", "--bool", "core.commitGraph"]).is_ok_and(|v| v.trim() == "false");
    if disabled {
        return;
    }
    crate::git::git_process()
        .args(["commit-graph", "write", "--reachable", "--split", "--no-progress"])
        .current_dir(repository.root())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok();
}

/// Finds a commit's row by its hash without scanning the log.
#[derive(Debug, Default)]
pub struct CommitRows(std::collections::HashMap<u64, u32>);

impl CommitRows {
    pub fn build(commits: &[Commit]) -> Self {
        let mut map = std::collections::HashMap::with_capacity(commits.len());
        for (ix, commit) in commits.iter().enumerate() {
            if let Some(key) = row_key(&commit.hash) {
                map.entry(key).or_insert(ix as u32);
            }
        }
        Self(map)
    }

    pub fn get(&self, commits: &[Commit], hash: &str) -> Option<usize> {
        let ix = row_key(hash).and_then(|key| self.0.get(&key)).map(|&ix| ix as usize);
        match ix {
            Some(ix) if commits.get(ix).is_some_and(|c| c.hash == hash) => Some(ix),
            // Not a full hash, or two hashes sharing the first 16 digits.
            _ => (hash.len() < 40 || ix.is_some()).then(|| commits.iter().position(|c| c.hash == hash)).flatten(),
        }
    }
}

fn row_key(hash: &str) -> Option<u64> {
    u64::from_str_radix(hash.get(..16)?, 16).ok()
}

pub(crate) fn parse_log(output: &str) -> Vec<Commit> {
    let mut people = People::default();
    output.split(RECORD).filter_map(|r| parse_record(r, &mut people)).collect()
}

/// Author names and emails, each kept once.
#[derive(Default)]
struct People(std::collections::HashSet<std::sync::Arc<str>>);

impl People {
    fn get(&mut self, name: &str) -> std::sync::Arc<str> {
        if let Some(known) = self.0.get(name) {
            return known.clone();
        }
        let name: std::sync::Arc<str> = name.into();
        self.0.insert(name.clone());
        name
    }
}

fn parse_record(record: &str, people: &mut People) -> Option<Commit> {
    let record = record.trim_end_matches('\n');
    if record.is_empty() {
        return None;
    }
    let mut fields = record.splitn(6, FIELD);
    let hash = fields.next()?.to_owned();
    let parents = fields.next()?.split_whitespace().map(str::to_owned).collect();
    let author_name = people.get(fields.next()?);
    let author_email = people.get(fields.next()?);
    let author_time = fields.next()?.parse().unwrap_or(0);
    let subject = fields.next().unwrap_or_default().to_owned();
    Some(Commit { hash, parents, author_name, author_email, author_time, subject })
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
    pub signature: Option<Signature>,
}

/// A commit's GPG / SSH signature, as git verifies it (`%G?`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    pub status: char,
    pub signer: String,
    pub key: String,
}

impl Signature {
    pub fn is_good(&self) -> bool {
        self.status == 'G' || self.status == 'U'
    }

    /// The details pane wording, after IntelliJ's signature tooltip.
    pub fn describe(&self) -> String {
        let who = if self.signer.is_empty() { String::new() } else { format!(" by {}", self.signer) };
        let key = if self.key.is_empty() { String::new() } else { format!(" (key {})", self.key) };
        match self.status {
            'G' => format!("Verified signature{who}{key}"),
            'U' => format!("Good signature with unknown validity{who}{key}"),
            'X' => format!("Good signature, expired{who}{key}"),
            'Y' => format!("Good signature made by an expired key{who}{key}"),
            'R' => format!("Good signature made by a revoked key{who}{key}"),
            'B' => format!("Bad signature{who}{key}"),
            // `N` with a signature present: git couldn't check it (an SSH
            // signature without gpg.ssh.allowedSignersFile).
            'E' | 'N' => format!("Signed, but the signature can't be checked (missing key, gpg, or SSH allowed signers){key}"),
            _ => "Signed".to_owned(),
        }
    }
}

/// Verifies a commit's signature, only when it has one (verification runs gpg).
fn load_signature(repository: &Repository, hash: &str) -> Option<Signature> {
    let raw = repository.run(["cat-file", "commit", hash]).ok()?;
    let header = raw.split("\n\n").next().unwrap_or_default();
    if !header.lines().any(|l| l.starts_with("gpgsig")) {
        return None;
    }
    let output = repository
        .run(["-c", "log.showSignature=false", "show", "-s", &format!("--format=%G?{FIELD}%GS{FIELD}%GK"), hash])
        .unwrap_or_else(|_| "E".into());
    let mut fields = output.trim_end().splitn(3, FIELD);
    Some(Signature {
        status: fields.next().and_then(|f| f.chars().next()).unwrap_or('E'),
        signer: fields.next().unwrap_or_default().to_owned(),
        key: fields.next().unwrap_or_default().to_owned(),
    })
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
        // (`-m --first-parent` would list the diffs against every parent.)
        diff_args.push(&parents[0]);
    }
    diff_args.push(&hash);
    let changes = parse_name_status(&repository.run(&diff_args)?);

    let containing_branches = repository
        .run(["branch", "-a", "--contains", &hash, "--format=%(refname)"])
        .map(|output| {
            // Full names, so "(HEAD detached at …)" and origin/HEAD (short: "origin") drop out.
            output
                .lines()
                .filter(|l| l.starts_with("refs/") && !l.ends_with("/HEAD"))
                .map(|l| l.strip_prefix("refs/heads/").or_else(|| l.strip_prefix("refs/remotes/")).unwrap_or(l).to_owned())
                .collect()
        })
        .unwrap_or_default();
    let signature = load_signature(repository, &hash);

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
        signature,
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

/// IntelliJ's "Collapse Linear Branches": runs of commits with one parent
/// and one child (and no refs) are hidden, and the commit above each run
/// points straight past it. Returns the visible commits and, per commit
/// whose edges skip a run, how many commits it hides. Commits in `expanded`
/// keep their runs.
pub fn collapse_linear(
    commits: &[Commit],
    has_refs: impl Fn(&str) -> bool,
    expanded: &std::collections::HashSet<String>,
) -> (Vec<Commit>, std::collections::HashMap<String, usize>) {
    use std::collections::{HashMap, HashSet};
    let index: HashMap<&str, usize> = commits.iter().enumerate().map(|(ix, c)| (c.hash.as_str(), ix)).collect();
    let mut children: HashMap<&str, usize> = HashMap::new();
    for commit in commits {
        for parent in &commit.parents {
            *children.entry(parent.as_str()).or_default() += 1;
        }
    }
    let interior = |ix: usize| {
        let c = &commits[ix];
        c.parents.len() == 1
            && children.get(c.hash.as_str()) == Some(&1)
            && !has_refs(&c.hash)
            && index.contains_key(c.parents[0].as_str())
    };
    let mut hidden_set: HashSet<usize> = HashSet::new();
    let mut hidden_count: HashMap<String, usize> = HashMap::new();
    let mut rewritten: HashMap<usize, Vec<String>> = HashMap::new();
    for (ix, commit) in commits.iter().enumerate() {
        if interior(ix) || expanded.contains(&commit.hash) {
            continue;
        }
        let mut parents = commit.parents.clone();
        for parent in parents.iter_mut() {
            let mut chain = Vec::new();
            let mut cursor = index.get(parent.as_str()).copied();
            while let Some(p) = cursor.filter(|&p| interior(p)) {
                chain.push(p);
                cursor = index.get(commits[p].parents[0].as_str()).copied();
            }
            // A single commit isn't worth a fold.
            if chain.len() >= 2 {
                if let Some(end) = cursor {
                    *parent = commits[end].hash.clone();
                    *hidden_count.entry(commit.hash.clone()).or_default() += chain.len();
                    hidden_set.extend(chain);
                }
            }
        }
        if parents != commit.parents {
            rewritten.insert(ix, parents);
        }
    }
    let visible = commits
        .iter()
        .enumerate()
        .filter(|(ix, _)| !hidden_set.contains(ix))
        .map(|(ix, c)| match rewritten.get(&ix) {
            Some(parents) => Commit { parents: parents.clone(), ..c.clone() },
            None => c.clone(),
        })
        .collect();
    (visible, hidden_count)
}

#[cfg(test)]
mod collapse_tests {
    use super::*;

    fn commit(hash: &str, parents: &[&str]) -> Commit {
        Commit {
            hash: hash.into(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            author_name: "".into(),
            author_email: "".into(),
            author_time: 0,
            subject: hash.into(),
        }
    }

    #[test]
    fn collapses_runs_between_branch_points() {
        // m merges b into a; a1..a3 is a linear run under a; b has one commit.
        let commits = vec![
            commit("m", &["a", "b"]),
            commit("a", &["a1"]),
            commit("b", &["base"]),
            commit("a1", &["a2"]),
            commit("a2", &["a3"]),
            commit("a3", &["base"]),
            commit("base", &[]),
        ];
        let refs = |h: &str| h == "m";
        let (visible, hidden) = collapse_linear(&commits, refs, &Default::default());
        let hashes: Vec<&str> = visible.iter().map(|c| c.hash.as_str()).collect();
        // m's first parent run a..a3 (4 commits) folds, b (1 commit) stays.
        assert_eq!(hashes, vec!["m", "b", "base"]);
        assert_eq!(visible[0].parents, vec!["base".to_string(), "b".to_string()]);
        assert_eq!(hidden.get("m"), Some(&4));
        let expanded: std::collections::HashSet<String> = ["m".to_string()].into();
        let (all, _) = collapse_linear(&commits, refs, &expanded);
        assert_eq!(all.len(), commits.len());
    }
}
