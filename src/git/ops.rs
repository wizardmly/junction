//! Queries behind the Push, Update and Stash dialogs.

use anyhow::Result;

use super::log::{FileChange, parse_log, parse_name_status};
use super::{Commit, Repository};

/// What "Push" would send for the current branch.
#[derive(Clone, Debug, Default)]
pub struct PushPreview {
    pub branch: Option<String>,
    pub remotes: Vec<String>,
    pub remote: String,
    /// Remote branch name the push targets.
    pub target: String,
    /// True when the remote branch does not exist yet ("New" in IntelliJ).
    pub new_branch: bool,
    pub commits: Vec<Commit>,
}

pub(crate) const LOG_FORMAT: &str = "--format=\u{1e}%H\u{1f}%P\u{1f}%an\u{1f}%ae\u{1f}%at\u{1f}%s";

pub fn remotes(repository: &Repository) -> Vec<String> {
    repository
        .run(["remote"])
        .map(|o| o.lines().map(str::to_owned).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default()
}

pub fn push_preview(repository: &Repository) -> Result<PushPreview> {
    let branch = repository
        .run(["symbolic-ref", "-q", "--short", "HEAD"])
        .ok()
        .map(|b| b.trim().to_owned())
        .filter(|b| !b.is_empty());
    let remotes = remotes(repository);
    let upstream = repository
        .run(["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"])
        .ok()
        .map(|u| u.trim().to_owned())
        .filter(|u| !u.is_empty());

    let (remote, target) = match &upstream {
        Some(upstream) => {
            let (remote, target) = upstream.split_once('/').unwrap_or(("origin", upstream.as_str()));
            (remote.to_owned(), target.to_owned())
        }
        None => (
            remotes.iter().find(|r| *r == "origin").or(remotes.first()).cloned().unwrap_or_else(|| "origin".into()),
            branch.clone().unwrap_or_default(),
        ),
    };
    let remote_ref = format!("refs/remotes/{remote}/{target}");
    let new_branch = repository.run(["rev-parse", "--verify", "-q", &remote_ref]).is_err();
    let commits = if new_branch {
        // Everything not already on some remote.
        parse_log(&repository.run(["log", LOG_FORMAT, "HEAD", "--not", "--remotes"])?)
    } else {
        parse_log(&repository.run(["log", LOG_FORMAT, &format!("{remote_ref}..HEAD")])?)
    };
    Ok(PushPreview { branch, remotes, remote, target, new_branch, commits })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushTags {
    None,
    All,
    CurrentBranch,
}

#[derive(Clone, Debug)]
pub struct PushRequest {
    pub remote: String,
    pub branch: String,
    pub target: String,
    pub force_with_lease: bool,
    pub tags: PushTags,
    pub set_upstream: bool,
    pub run_hooks: bool,
}

pub fn push(repository: &Repository, request: &PushRequest) -> Result<String> {
    let mut args = vec!["push".to_owned(), "--porcelain".into()];
    if request.force_with_lease {
        args.push("--force-with-lease".into());
    }
    if request.set_upstream {
        args.push("--set-upstream".into());
    }
    if !request.run_hooks {
        args.push("--no-verify".into());
    }
    match request.tags {
        PushTags::None => {}
        PushTags::All => args.push("--tags".into()),
        PushTags::CurrentBranch => args.push("--follow-tags".into()),
    }
    args.push(request.remote.clone());
    args.push(format!("{}:refs/heads/{}", request.branch, request.target));
    repository.run(&args)?;
    Ok(format!("Pushed {} to {}/{}", request.branch, request.remote, request.target))
}

/// Whether a push failed because the remote has commits we don't
/// (IntelliJ's "Push Rejected" case), as opposed to auth or network errors.
pub fn is_rejected(error: &anyhow::Error) -> bool {
    let text = error.to_string();
    text.contains("[rejected]") || text.contains("Updates were rejected") || text.contains("non-fast-forward")
}

/// Push, and if rejected, update the branch (merge or rebase, stashing
/// local changes) and push once more: "Auto-update if push was rejected".
pub fn push_with_auto_update(repository: &Repository, request: &PushRequest, rebase: bool) -> Result<String> {
    match push(repository, request) {
        Err(error) if is_rejected(&error) && !request.force_with_lease => {
            repository.run([
                "pull",
                if rebase { "--rebase" } else { "--no-rebase" },
                "--autostash",
                &request.remote,
                &request.target,
            ])?;
            push(repository, request).map(|message| format!("{message} (after updating from {})", request.remote))
        }
        result => result,
    }
}

/// How Update Project keeps local changes out of the way while pulling.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CleanWith {
    #[default]
    Stash,
    Shelve,
}

/// Update Project: fetch, then merge or rebase the tracked branch, saving
/// local changes with stash or a shelf and restoring them afterwards.
/// The message carries the updated range after a unit separator.
pub fn update_project(repository: &Repository, rebase: bool, clean: CleanWith) -> Result<String> {
    repository.run(["fetch", "--all", "--prune"])?;
    if repository.run(["rev-parse", "--abbrev-ref", "@{upstream}"]).is_err() {
        anyhow::bail!("the current branch has no tracked branch");
    }
    let mode = if rebase { "--rebase" } else { "--no-rebase" };
    let before = repository.run(["rev-parse", "HEAD"])?.trim().to_owned();
    let mut restore_note = String::new();
    match clean {
        CleanWith::Stash => {
            repository.run(["pull", mode, "--autostash"])?;
        }
        CleanWith::Shelve => {
            let changed = repository.run(["diff", "--name-only", "-z", "HEAD"])?;
            let paths: Vec<String> = changed.split('\0').filter(|p| !p.is_empty()).map(str::to_owned).collect();
            let shelf = if paths.is_empty() {
                None
            } else {
                let name = format!("Uncommitted changes before Update at {}", chrono::Local::now().format("%Y-%m-%d %H:%M"));
                Some(super::patch::shelve(repository, &paths, &name, false)?)
            };
            let pulled = repository.run(["pull", mode]);
            if let Some(shelf) = &shelf {
                match super::patch::unshelve(repository, shelf, None, false) {
                    Ok(super::patch::ApplyOutcome::Conflicts) => {
                        restore_note = format!("; local changes restored from shelf \"{}\" with conflicts", shelf.name)
                    }
                    Ok(_) => {}
                    Err(error) => restore_note = format!("; local changes kept in shelf \"{}\": {error}", shelf.name),
                }
            }
            pulled?;
        }
    }
    let after = repository.run(["rev-parse", "HEAD"])?.trim().to_owned();
    if before == after {
        return Ok(format!("All files are up to date{restore_note}"));
    }
    let range = format!("{before}..{after}");
    // Submodules whose recorded commit the update moved follow along,
    // as `git pull --recurse-submodules` would.
    if repository.root().join(".gitmodules").exists() {
        let changed: Vec<String> = repository.run(["diff", "--name-only", &range]).unwrap_or_default().lines().map(str::to_owned).collect();
        let moved: Vec<String> = super::submodule::gitlink_paths(repository).into_iter().filter(|p| changed.contains(p)).collect();
        if !moved.is_empty() {
            match super::submodule::update(repository, &moved) {
                Ok(()) => restore_note.push_str(&format!("; {} submodule{} updated", moved.len(), if moved.len() == 1 { "" } else { "s" })),
                Err(error) => restore_note.push_str(&format!("; submodules not updated: {error}")),
            }
        }
    }
    let count = repository.run(["rev-list", "--count", &range]).unwrap_or_default();
    let count = count.trim();
    let files = repository.run(["diff", "--name-only", &range]).map(|o| o.lines().count()).unwrap_or(0);
    Ok(format!(
        "{files} file{} updated in {count} commit{}{restore_note}\u{1f}{range}",
        if files == 1 { "" } else { "s" },
        if count == "1" { "" } else { "s" }
    ))
}

/// One entry of `git stash list`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stash {
    /// `stash@{0}`.
    pub name: String,
    pub hash: String,
    pub message: String,
    pub time: i64,
    pub branch: Option<String>,
}

pub fn stash_list(repository: &Repository) -> Result<Vec<Stash>> {
    let output = repository.run(["stash", "list", "--format=%gd\u{1f}%H\u{1f}%gs\u{1f}%ct"])?;
    Ok(output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\u{1f}');
            let name = fields.next()?.to_owned();
            let hash = fields.next()?.to_owned();
            let subject = fields.next()?.to_owned();
            let time = fields.next()?.parse().unwrap_or(0);
            // "WIP on main: abc123 msg" / "On main: message"
            let branch = subject
                .strip_prefix("WIP on ")
                .or_else(|| subject.strip_prefix("On "))
                .and_then(|rest| rest.split(':').next())
                .map(str::to_owned);
            let message = subject.split_once(": ").map_or(subject.clone(), |(_, m)| m.to_owned());
            Some(Stash { name, hash, message, time, branch })
        })
        .collect())
}

pub fn stash_files(repository: &Repository, stash: &str) -> Result<Vec<FileChange>> {
    let mut changes = parse_name_status(&repository.run(["stash", "show", "--name-status", "-z", "-M", stash])?);
    // Untracked files saved with `-u` live in the stash's third parent.
    if let Ok(output) = repository.run(["show", "--name-status", "-z", "--format=", &format!("{stash}^3")]) {
        changes.extend(parse_name_status(&output));
    }
    Ok(changes)
}

#[derive(Clone, Debug, Default)]
pub struct StashRequest {
    pub message: String,
    pub keep_index: bool,
    pub include_untracked: bool,
}

pub fn stash_save(repository: &Repository, request: &StashRequest) -> Result<String> {
    let mut args = vec!["stash".to_owned(), "push".into()];
    if request.keep_index {
        args.push("--keep-index".into());
    }
    if request.include_untracked {
        args.push("--include-untracked".into());
    }
    if !request.message.trim().is_empty() {
        args.push("-m".into());
        args.push(request.message.trim().to_owned());
    }
    repository.run(&args)?;
    Ok("Local changes stashed".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::GitConsole;
    use std::process::Command;

    fn git(dir: &std::path::Path, args: &[&str]) {
        let status = Command::new("git").arg("-C").arg(dir).args(args).env("GIT_AUTHOR_NAME", "T").env("GIT_AUTHOR_EMAIL", "t@x").env("GIT_COMMITTER_NAME", "T").env("GIT_COMMITTER_EMAIL", "t@x").output().unwrap();
        assert!(status.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&status.stderr));
    }

    fn temp_repo(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gitglass-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("a.txt"), "1\n").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-qm", "first"]);
        dir
    }

    #[test]
    fn update_project_with_shelve_restores_local_changes() {
        let origin = temp_repo("update-origin");
        let local = origin.with_file_name(format!("gitglass-test-update-local-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&local);
        git(&origin, &["clone", "-q", origin.to_str().unwrap(), local.to_str().unwrap()]);
        git(&local, &["config", "user.name", "T"]);
        git(&local, &["config", "user.email", "t@x"]);
        std::fs::write(origin.join("b.txt"), "b\n").unwrap();
        git(&origin, &["add", "."]);
        git(&origin, &["commit", "-qm", "second"]);
        std::fs::write(local.join("a.txt"), "local\n").unwrap();

        let repo = Repository::discover(&local, GitConsole::default()).unwrap();
        let message = update_project(&repo, false, CleanWith::Shelve).unwrap();
        assert!(message.starts_with("1 file updated in 1 commit\u{1f}"), "{message}");
        assert_eq!(std::fs::read_to_string(local.join("a.txt")).unwrap(), "local\n");
        assert!(local.join("b.txt").exists());
        let shelves = crate::git::patch::shelves(&repo);
        assert_eq!(shelves.len(), 1);
        assert!(shelves[0].deleted && shelves[0].name.starts_with("Uncommitted changes before Update"));
        assert!(repo.run(["stash", "list"]).unwrap().is_empty());
    }

    #[test]
    fn stash_round_trip_and_push_preview() {
        let dir = temp_repo("stash");
        let repo = Repository::discover(&dir, GitConsole::default()).unwrap();
        std::fs::write(dir.join("a.txt"), "2\n").unwrap();
        std::fs::write(dir.join("new.txt"), "n\n").unwrap();
        stash_save(&repo, &StashRequest { message: "wip work".into(), keep_index: false, include_untracked: true }).unwrap();
        let stashes = stash_list(&repo).unwrap();
        assert_eq!(stashes.len(), 1);
        assert_eq!(stashes[0].message, "wip work");
        assert_eq!(stashes[0].branch.as_deref(), Some("main"));
        let files: Vec<_> = stash_files(&repo, "stash@{0}").unwrap().into_iter().map(|f| f.path).collect();
        assert!(files.contains(&"a.txt".to_owned()) && files.contains(&"new.txt".to_owned()), "{files:?}");

        // No remote: every commit is "new" and would be pushed.
        let preview = push_preview(&repo).unwrap();
        assert!(preview.new_branch);
        assert_eq!(preview.commits.len(), 1);
        assert_eq!(preview.target, "main");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejected_push_updates_and_retries() {
        let origin = temp_repo("origin-src");
        let bare = origin.with_extension("git");
        let _ = std::fs::remove_dir_all(&bare);
        git(&origin, &["clone", "-q", "--bare", ".", bare.to_str().unwrap()]);
        let clone = |name: &str| {
            let dir = std::env::temp_dir().join(format!("gitglass-test-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            git(&origin, &["clone", "-q", bare.to_str().unwrap(), dir.to_str().unwrap()]);
            git(&dir, &["config", "user.name", "T"]);
            git(&dir, &["config", "user.email", "t@x"]);
            dir
        };
        let (mine, theirs) = (clone("mine"), clone("theirs"));
        std::fs::write(theirs.join("b.txt"), "b\n").unwrap();
        git(&theirs, &["add", "."]);
        git(&theirs, &["commit", "-qm", "theirs"]);
        git(&theirs, &["push", "-q"]);
        std::fs::write(mine.join("c.txt"), "c\n").unwrap();
        git(&mine, &["add", "."]);
        git(&mine, &["commit", "-qm", "mine"]);

        let repo = Repository::discover(&mine, GitConsole::default()).unwrap();
        let request = PushRequest {
            remote: "origin".into(),
            branch: "main".into(),
            target: "main".into(),
            force_with_lease: false,
            set_upstream: false,
            tags: PushTags::None,
            run_hooks: true,
        };
        assert!(is_rejected(&push(&repo, &request).unwrap_err()));
        push_with_auto_update(&repo, &request, true).unwrap();
        assert!(mine.join("b.txt").exists());
        for dir in [origin, bare, mine, theirs] {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}
