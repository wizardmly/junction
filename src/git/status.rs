use anyhow::Result;

use super::Repository;

/// How a file differs from HEAD, as the Commit tool window colors it
/// (with the staging area off, index and work tree are shown combined).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum StatusKind {
    Conflicted,
    Added,
    Modified,
    Renamed,
    Deleted,
    Unversioned,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusEntry {
    pub kind: StatusKind,
    pub path: String,
    pub old_path: Option<String>,
    /// The raw two-letter porcelain code, kept for the staging-area view.
    pub index: char,
    pub work_tree: char,
}

#[derive(Clone, Debug, Default)]
pub struct WorkingTreeStatus {
    pub entries: Vec<StatusEntry>,
}

impl WorkingTreeStatus {
    pub fn load(repository: &Repository) -> Result<Self> {
        let output = repository.run(["status", "--porcelain=v1", "-z", "--untracked-files=all"])?;
        Ok(Self { entries: parse_porcelain(&output) })
    }

    pub fn changes(&self) -> impl Iterator<Item = &StatusEntry> {
        self.entries.iter().filter(|e| e.kind != StatusKind::Unversioned)
    }

    pub fn unversioned(&self) -> impl Iterator<Item = &StatusEntry> {
        self.entries.iter().filter(|e| e.kind == StatusKind::Unversioned)
    }

    /// Changes in the index (the Staged tree in staging-area mode).
    pub fn staged(&self) -> Vec<(String, StatusKind)> {
        self.entries
            .iter()
            .filter(|e| e.kind != StatusKind::Unversioned && e.index != ' ')
            .map(|e| (e.path.clone(), if e.kind == StatusKind::Conflicted { e.kind } else { kind_of(e.index) }))
            .filter(|(_, kind)| *kind != StatusKind::Conflicted)
            .collect()
    }

    /// Work tree changes not yet staged, conflicts included (the Unstaged tree).
    pub fn unstaged(&self) -> Vec<(String, StatusKind)> {
        self.entries
            .iter()
            .filter(|e| e.kind != StatusKind::Unversioned)
            .filter(|e| e.kind == StatusKind::Conflicted || e.work_tree != ' ')
            .map(|e| (e.path.clone(), if e.kind == StatusKind::Conflicted { e.kind } else { kind_of(e.work_tree) }))
            .collect()
    }
}

fn kind_of(code: char) -> StatusKind {
    match code {
        'A' => StatusKind::Added,
        'D' => StatusKind::Deleted,
        'R' | 'C' => StatusKind::Renamed,
        'U' => StatusKind::Conflicted,
        _ => StatusKind::Modified,
    }
}

/// `git add` for the given paths (Stage in staging-area mode).
pub fn stage(repository: &Repository, paths: &[String]) -> Result<()> {
    let mut args = vec!["add".to_owned(), "-A".to_owned(), "--".to_owned()];
    args.extend(paths.iter().cloned());
    repository.run(&args)?;
    Ok(())
}

/// Removes the given paths from the index, keeping work tree changes.
pub fn unstage(repository: &Repository, paths: &[String]) -> Result<()> {
    let has_head = repository.run(["rev-parse", "--verify", "-q", "HEAD"]).is_ok();
    let mut args: Vec<String> = if has_head {
        vec!["restore".into(), "--staged".into(), "--".into()]
    } else {
        vec!["rm".into(), "--cached".into(), "-r".into(), "-q".into(), "--".into()]
    };
    args.extend(paths.iter().cloned());
    repository.run(&args)?;
    Ok(())
}

pub(crate) fn parse_porcelain(output: &str) -> Vec<StatusEntry> {
    let mut parts = output.split('\0').filter(|p| !p.is_empty());
    let mut entries = Vec::new();
    while let Some(record) = parts.next() {
        if record.len() < 4 {
            continue;
        }
        let mut chars = record.chars();
        let index = chars.next().unwrap();
        let work_tree = chars.next().unwrap();
        let path = record[3..].to_owned();
        let old_path = (index == 'R' || index == 'C').then(|| parts.next().map(str::to_owned)).flatten();
        let kind = match (index, work_tree) {
            ('?', '?') => StatusKind::Unversioned,
            ('!', '!') => continue,
            ('U', _) | (_, 'U') | ('A', 'A') | ('D', 'D') => StatusKind::Conflicted,
            ('A', _) => StatusKind::Added,
            ('R', _) | ('C', _) => StatusKind::Renamed,
            ('D', _) | (_, 'D') => StatusKind::Deleted,
            _ => StatusKind::Modified,
        };
        entries.push(StatusEntry { kind, path, old_path, index, work_tree });
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    entries
}

/// Options from the Commit tool window.
#[derive(Clone, Debug, Default)]
pub struct CommitRequest {
    pub message: String,
    pub amend: bool,
    /// Files to commit; IntelliJ commits only the checked files.
    pub paths: Vec<String>,
    /// Unversioned files among `paths` are added first.
    pub unversioned: Vec<String>,
    pub sign_off: bool,
    /// Staging-area mode: commit what is in the index, ignoring `paths`.
    pub staged_only: bool,
    /// Commit options: "Author", "GPG-sign", "Run Git hooks", "Clean up commit message".
    pub author: Option<String>,
    pub gpg_sign: bool,
    pub run_hooks: bool,
    pub cleanup: bool,
}

pub fn commit(repository: &Repository, request: &CommitRequest) -> Result<String> {
    if !request.unversioned.is_empty() {
        let mut args = vec!["add".to_owned(), "--".to_owned()];
        args.extend(request.unversioned.iter().cloned());
        repository.run(&args)?;
    }
    // A merge can't be committed partially: stage the chosen files, then
    // commit the whole index, as IntelliJ does while merging.
    let merging = repository.state() == super::RepositoryState::Merging;
    if merging && !request.staged_only && !request.paths.is_empty() {
        let mut args = vec!["add".to_owned(), "-A".to_owned(), "--".to_owned()];
        args.extend(request.paths.iter().cloned());
        repository.run(&args)?;
    }
    let mut args = vec!["commit".to_owned(), "-F".to_owned(), "-".to_owned()];
    if request.amend {
        args.push("--amend".into());
    }
    if request.sign_off {
        args.push("--signoff".into());
    }
    if let Some(author) = request.author.as_ref().filter(|a| !a.trim().is_empty()) {
        args.push(format!("--author={}", author.trim()));
    }
    if request.gpg_sign {
        args.push("--gpg-sign".into());
    }
    if !request.run_hooks {
        args.push("--no-verify".into());
    }
    if request.cleanup {
        args.push("--cleanup=strip".into());
    }
    if request.staged_only || merging {
        // Commit the index as it is.
    } else if request.paths.is_empty() {
        // Amending only the message.
        args.push("--only".into());
    } else {
        args.push("--only".into());
        args.push("--".into());
        args.extend(request.paths.iter().cloned());
    }
    repository.run_with_input(&args, Some(&request.message))?;
    let hash = repository.run(["rev-parse", "HEAD"])?;
    Ok(hash.trim().to_owned())
}

/// Files among `paths` whose working tree content has CRLF line separators
/// that git would commit as they are (no `core.autocrlf`).
pub fn crlf_files(repository: &Repository, paths: &[String]) -> Vec<String> {
    let autocrlf = repository.run(["config", "--get", "core.autocrlf"]).map(|v| v.trim().to_owned()).unwrap_or_default();
    if autocrlf == "true" || autocrlf == "input" {
        return Vec::new();
    }
    paths
        .iter()
        .filter(|path| {
            let full = repository.root().join(path);
            match std::fs::metadata(&full) {
                Ok(meta) if meta.is_file() && meta.len() < 4 * 1024 * 1024 => std::fs::read(&full)
                    .map(|bytes| !bytes.contains(&0) && bytes.windows(2).any(|w| w == b"\r\n"))
                    .unwrap_or(false),
                _ => false,
            }
        })
        .cloned()
        .collect()
}

/// Files among `paths` larger than `limit` bytes (hosting services reject them).
pub fn large_files(repository: &Repository, paths: &[String], limit: u64) -> Vec<(String, u64)> {
    paths
        .iter()
        .filter_map(|path| {
            let size = std::fs::metadata(repository.root().join(path)).ok()?.len();
            (size > limit).then(|| (path.clone(), size))
        })
        .collect()
}

/// Whether commits are signed by default (`commit.gpgSign`).
pub fn gpg_sign_default(repository: &Repository) -> bool {
    repository.run(["config", "--bool", "--get", "commit.gpgsign"]).is_ok_and(|v| v.trim() == "true")
}

/// The message of HEAD, loaded into the editor when "Amend" is checked.
pub fn last_commit_message(repository: &Repository) -> Option<String> {
    repository.run(["log", "-1", "--format=%B"]).ok().map(|m| m.trim_end().to_owned())
}

/// Checks out a local branch, or creates a tracking branch for a remote one.
/// If local changes are in the way, does IntelliJ's "Smart Checkout": stash,
/// check out, and restore the changes.
pub fn checkout(repository: &Repository, reference: &super::RefName) -> Result<()> {
    match checkout_plain(repository, reference) {
        Err(error) if error.to_string().contains("would be overwritten by checkout") => {
            repository.run(["stash", "push", "--include-untracked", "-m", "GitGlass smart checkout"])?;
            let result = checkout_plain(repository, reference);
            // Restore even if the checkout failed; a conflict leaves the stash for the Stash tab.
            repository.run(["stash", "pop"])?;
            result
        }
        result => result,
    }
}

fn checkout_plain(repository: &Repository, reference: &super::RefName) -> Result<()> {
    match reference.kind {
        super::RefKind::RemoteBranch => {
            let local = reference.branch_without_remote();
            repository.run(["checkout", "-b", local, "--track", &reference.name])?;
        }
        _ => {
            repository.run(["checkout", &reference.name])?;
        }
    }
    Ok(())
}

pub fn create_branch(repository: &Repository, name: &str, start_point: &str, checkout: bool) -> Result<()> {
    if checkout {
        repository.run(["checkout", "-b", name, start_point])?;
    } else {
        repository.run(["branch", name, start_point])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_staged_and_unstaged() {
        let status = WorkingTreeStatus { entries: parse_porcelain("MM a.rs\0A  b.rs\0 D c.rs\0UU d.rs\0?? e.rs\0") };
        let staged: Vec<_> = status.staged().into_iter().map(|(p, _)| p).collect();
        let unstaged: Vec<_> = status.unstaged().into_iter().map(|(p, k)| (p, k)).collect();
        assert_eq!(staged, vec!["a.rs", "b.rs"]);
        assert_eq!(
            unstaged,
            vec![("a.rs".into(), StatusKind::Modified), ("c.rs".into(), StatusKind::Deleted), ("d.rs".into(), StatusKind::Conflicted)]
        );
    }

    #[test]
    fn parses_porcelain_v1() {
        let entries = parse_porcelain(" M src/a.rs\0R  new.rs\0old.rs\0?? notes.txt\0UU c.rs\0A  b.rs\0");
        let kinds: Vec<_> = entries.iter().map(|e| (e.path.as_str(), e.kind)).collect();
        assert_eq!(
            kinds,
            vec![
                ("b.rs", StatusKind::Added),
                ("c.rs", StatusKind::Conflicted),
                ("new.rs", StatusKind::Renamed),
                ("notes.txt", StatusKind::Unversioned),
                ("src/a.rs", StatusKind::Modified),
            ]
        );
        assert_eq!(entries[2].old_path.as_deref(), Some("old.rs"));
    }
}
