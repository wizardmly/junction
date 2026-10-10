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
    /// Whether the branch already tracks a remote branch; a push without
    /// one sets it (`--set-upstream`).
    pub has_upstream: bool,
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
    let new_branch = repository.run(["rev-parse", "--verify", "-q", &format!("refs/remotes/{remote}/{target}")]).is_err();
    Ok(PushPreview { branch, remotes, remote, target, new_branch, has_upstream: upstream.is_some() })
}

/// What pushing HEAD to `remote`/`target` would send, and whether that
/// creates the remote branch. The Push dialog asks again whenever the
/// target branch or remote changes.
pub fn push_commits(repository: &Repository, remote: &str, target: &str) -> Result<(bool, Vec<Commit>)> {
    let remote_ref = format!("refs/remotes/{remote}/{target}");
    let new_branch = target.is_empty() || repository.run(["rev-parse", "--verify", "-q", &remote_ref]).is_err();
    let commits = if new_branch {
        // Everything that remote doesn't have yet.
        parse_log(&repository.run(["log", LOG_FORMAT, "HEAD", "--not", &format!("--remotes={remote}")])?)
    } else {
        parse_log(&repository.run(["log", LOG_FORMAT, &format!("{remote_ref}..HEAD")])?)
    };
    Ok((new_branch, commits))
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
    /// Push All up to Here: push this commit of the branch, not its tip.
    pub up_to: Option<String>,
}

pub fn push(repository: &Repository, request: &PushRequest) -> Result<String> {
    let mut args = vec!["push".to_owned(), "--porcelain".into()];
    if request.force_with_lease {
        args.push("--force-with-lease".into());
    }
    // `--set-upstream` needs a branch as the source; a commit's push sets it below.
    if request.set_upstream && request.up_to.is_none() {
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
    let source = request.up_to.as_deref().unwrap_or(&request.branch);
    args.push(format!("{source}:refs/heads/{}", request.target));
    repository.run(&args)?;
    if let Some(hash) = &request.up_to {
        if request.set_upstream {
            let upstream = format!("--set-upstream-to={}/{}", request.remote, request.target);
            let _ = repository.run(["branch", upstream.as_str(), request.branch.as_str()]);
        }
        return Ok(format!("Pushed commits up to {} to {}/{}", &hash[..hash.len().min(8)], request.remote, request.target));
    }
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
        // Up to a commit: after a rebase that commit is no longer the one to push.
        Err(error) if is_rejected(&error) && !request.force_with_lease && request.up_to.is_none() => {
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
    let Some(upstream) = repository
        .run(["rev-parse", "--abbrev-ref", "@{upstream}"])
        .ok()
        .map(|u| u.trim().to_owned())
        .filter(|u| !u.is_empty())
    else {
        anyhow::bail!("the current branch has no tracked branch");
    };
    update_onto(repository, &upstream, rebase, clean)
}

/// The Push Rejected dialog's Merge / Rebase: fetch the push target,
/// update onto it, and push again.
pub fn update_after_rejected_push(repository: &Repository, request: &PushRequest, rebase: bool, clean: CleanWith) -> Result<String> {
    repository.run(["fetch", &request.remote])?;
    let onto = format!("{}/{}", request.remote, request.target);
    let updated = update_onto(repository, &onto, rebase, clean)?;
    let updated = updated.split('\u{1f}').next().unwrap_or_default().to_owned();
    let pushed = push(repository, request)?;
    Ok(format!("{pushed} ({updated})"))
}

/// Merges or rebases the current branch onto the already fetched `onto`,
/// keeping local changes out of the way with stash or a shelf.
fn update_onto(repository: &Repository, onto: &str, rebase: bool, clean: CleanWith) -> Result<String> {
    let upstream = onto;
    // IntelliJ merges or rebases onto the fetched tracked branch rather
    // than running `git pull`, so a merge reads "Merge remote-tracking
    // branch 'origin/main'".
    let command = if rebase { "rebase" } else { "merge" };
    let before = repository.run(["rev-parse", "HEAD"])?.trim().to_owned();
    let mut restore_note = String::new();
    match clean {
        CleanWith::Stash => {
            repository.run([command, "--autostash", upstream])?;
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
            let pulled = repository.run([command, upstream]);
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
    let incoming = repository.run(["rev-parse", upstream]).map(|o| o.trim().to_owned()).unwrap_or_else(|_| after.clone());
    Ok(format!("{}{restore_note}\u{1f}{}", updated_summary(repository, &before, &after, &incoming), updated_ranges(&before, &after, &incoming)))
}

/// "N files updated in M commits" for an update that moved HEAD from
/// `before` to `after` by merging or rebasing onto `incoming`. As in
/// IntelliJ, only the received commits count: local commits replayed by
/// a rebase and the merge commit itself are not "updated".
pub fn updated_summary(repository: &Repository, before: &str, after: &str, incoming: &str) -> String {
    let count = repository.run(["rev-list", "--count", &format!("{before}..{incoming}")]).unwrap_or_default();
    let count = count.trim();
    let files = repository.run(["diff", "--name-only", &format!("{before}..{after}")]).map(|o| o.lines().count()).unwrap_or(0);
    format!(
        "{files} file{} updated in {count} commit{}",
        if files == 1 { "" } else { "s" },
        if count == "1" { "" } else { "s" }
    )
}

/// The ranges an update notification carries after its unit separator:
/// the changed files (`before..after`), then the received commits
/// (`before..incoming`), separated by a space.
pub fn updated_ranges(before: &str, after: &str, incoming: &str) -> String {
    format!("{before}..{after} {before}..{incoming}")
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

/// Checks out a pull / merge request's branch: an existing local branch is
/// fast-forwarded to `head_sha` when it can be; otherwise `head_ref` (e.g.
/// `pull/7/head`) is fetched from `remote` into a new local `branch`.
pub fn checkout_pull_request(repository: &Repository, remote: &str, head_ref: &str, branch: &str, head_sha: &str) -> Result<()> {
    if repository.run(["rev-parse", "--verify", "-q", &format!("refs/heads/{branch}")]).is_ok() {
        repository.run(["checkout", branch])?;
        repository.run(["merge", "--ff-only", head_sha]).ok();
    } else {
        repository.run(["fetch", remote, &format!("{head_ref}:{branch}")])?;
        repository.run(["checkout", branch])?;
    }
    Ok(())
}

/// Runs a `git pull` command line and describes the result the way the
/// Pull dialog reports it: "Already up to date", or the update summary and
/// ranges joined by U+001F.
pub fn pull(repository: &Repository, args: &[String]) -> Result<String> {
    let before = repository.run(["rev-parse", "HEAD"]).unwrap_or_default();
    repository.run(args)?;
    let after = repository.run(["rev-parse", "HEAD"]).unwrap_or_default();
    if before == after {
        return Ok("Already up to date".into());
    }
    let (before, after) = (before.trim(), after.trim());
    // The fetched branch the pull merged or rebased onto.
    let incoming = repository.run(["rev-parse", "FETCH_HEAD"]).map(|o| o.trim().to_owned()).unwrap_or_else(|_| after.to_owned());
    Ok(format!("{}\u{1f}{}", updated_summary(repository, before, after, &incoming), updated_ranges(before, after, &incoming)))
}

/// Renames a file or folder: tracked paths move with `git mv` so the rename
/// shows as one change, anything else (or a failed `git mv`) is renamed on
/// disk. `path` is `from` relative to the repository root.
pub fn rename_path(repository: Option<&Repository>, path: &str, from: &std::path::Path, to: &std::path::Path) {
    let tracked = repository.is_some_and(|r| r.run(["ls-files", "--error-unmatch", "--", path]).is_ok());
    let moved = tracked && repository.is_some_and(|r| r.run(["mv", "--", from.to_string_lossy().as_ref(), to.to_string_lossy().as_ref()]).is_ok());
    if !moved {
        let _ = std::fs::rename(from, to);
    }
}

/// Cherry-picks, skipping commits whose changes are already in the
/// current branch (git stops on them, "now empty"), as IntelliJ does
/// rather than leaving a cherry-pick in progress.
pub fn cherry_pick_skipping_empty(repo: &Repository, args: &[String], picked: String) -> Result<String> {
    let total = args.iter().filter(|a| !a.starts_with('-')).count() - 1;
    let mut skipped = 0;
    let mut result = repo.run(args);
    while let Err(error) = result {
        let text = format!("{error:#}");
        if skipped >= total || !(text.contains("is now empty") || text.contains("nothing to commit")) {
            return Err(error);
        }
        skipped += 1;
        result = repo.run(["cherry-pick", "--skip"]);
    }
    Ok(match (skipped, total) {
        (0, _) => picked,
        (s, t) if s == t && t == 1 => "Nothing to cherry-pick: the commit's changes are already in the current branch".to_owned(),
        (s, t) if s == t => "Nothing to cherry-pick: the commits' changes are already in the current branch".to_owned(),
        (s, t) => format!("Cherry-picked {} of {t} commits; {s} skipped, their changes are already in the current branch", t - s),
    })
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
        let dir = std::env::temp_dir().join(format!("junction-test-{name}-{}", std::process::id()));
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
        let local = origin.with_file_name(format!("junction-test-update-local-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&local);
        git(&origin, &["clone", "-q", origin.to_str().unwrap(), local.to_str().unwrap()]);
        git(&local, &["config", "user.name", "T"]);
        git(&local, &["config", "user.email", "t@x"]);
        std::fs::write(origin.join("b.txt"), "b\n").unwrap();
        git(&origin, &["add", "."]);
        git(&origin, &["commit", "-qm", "second"]);
        std::fs::write(local.join("a.txt"), "local\n").unwrap();
        std::fs::write(local.join("c.txt"), "new\n").unwrap();
        git(&local, &["add", "c.txt"]);

        let repo = Repository::discover(&local, GitConsole::default()).unwrap();
        let message = update_project(&repo, true, CleanWith::Shelve).unwrap();
        assert!(message.starts_with("1 file updated in 1 commit\u{1f}"), "{message}");
        assert_eq!(std::fs::read_to_string(local.join("a.txt")).unwrap(), "local\n");
        assert!(local.join("b.txt").exists());
        // A file added to Git before the update comes back added, not unversioned.
        assert_eq!(repo.run(["status", "--porcelain", "--", "c.txt"]).unwrap().trim(), "A  c.txt");
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
        assert_eq!(preview.target, "main");
        let (new_branch, commits) = push_commits(&repo, &preview.remote, &preview.target).unwrap();
        assert!(new_branch);
        assert_eq!(commits.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn push_up_to_a_commit_sets_the_upstream() {
        let origin = temp_repo("upto-src");
        let bare = origin.with_extension("git");
        let _ = std::fs::remove_dir_all(&bare);
        git(&origin, &["init", "-q", "--bare", bare.to_str().unwrap()]);
        git(&origin, &["remote", "add", "origin", bare.to_str().unwrap()]);
        for name in ["one", "two"] {
            std::fs::write(origin.join(format!("{name}.txt")), name).unwrap();
            git(&origin, &["add", "."]);
            git(&origin, &["commit", "-qm", name]);
        }
        let repo = Repository::discover(&origin, GitConsole::default()).unwrap();
        let first = repo.run(["rev-parse", "HEAD~1"]).unwrap().trim().to_owned();
        let branch = repo.run(["branch", "--show-current"]).unwrap().trim().to_owned();
        let request = PushRequest {
            remote: "origin".into(),
            branch: branch.clone(),
            target: branch.clone(),
            force_with_lease: false,
            set_upstream: true,
            tags: PushTags::None,
            run_hooks: true,
            up_to: Some(first.clone()),
        };
        push(&repo, &request).unwrap();
        // Only the first commit went; the branch now tracks the pushed one.
        assert_eq!(repo.run(["rev-parse", &format!("origin/{branch}")]).unwrap().trim(), first);
        assert_eq!(repo.run(["rev-parse", "--abbrev-ref", "@{upstream}"]).unwrap().trim(), format!("origin/{branch}"));
        let _ = std::fs::remove_dir_all(&origin);
        let _ = std::fs::remove_dir_all(&bare);
    }

    #[test]
    fn rejected_push_updates_and_retries() {
        let origin = temp_repo("origin-src");
        let bare = origin.with_extension("git");
        let _ = std::fs::remove_dir_all(&bare);
        git(&origin, &["clone", "-q", "--bare", ".", bare.to_str().unwrap()]);
        let clone = |name: &str| {
            let dir = std::env::temp_dir().join(format!("junction-test-{name}-{}", std::process::id()));
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
            up_to: None,
        };
        assert!(is_rejected(&push(&repo, &request).unwrap_err()));
        push_with_auto_update(&repo, &request, true).unwrap();
        assert!(mine.join("b.txt").exists());
        for dir in [origin, bare, mine, theirs] {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn update_counts_only_received_commits() {
        for rebase in [true, false] {
            let name = if rebase { "count-rebase" } else { "count-merge" };
            let origin = temp_repo(&format!("{name}-origin"));
            let local = origin.with_file_name(format!("junction-test-{name}-local-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&local);
            git(&origin, &["clone", "-q", origin.to_str().unwrap(), local.to_str().unwrap()]);
            git(&local, &["config", "user.name", "T"]);
            git(&local, &["config", "user.email", "t@x"]);
            std::fs::write(origin.join("b.txt"), "b\n").unwrap();
            git(&origin, &["add", "."]);
            git(&origin, &["commit", "-qm", "remote"]);
            std::fs::write(local.join("c.txt"), "c\n").unwrap();
            git(&local, &["add", "."]);
            git(&local, &["commit", "-qm", "local"]);

            let repo = Repository::discover(&local, GitConsole::default()).unwrap();
            let message = update_project(&repo, rebase, CleanWith::Stash).unwrap();
            let (summary, ranges) = message.split_once('\u{1f}').unwrap();
            assert_eq!(summary, "1 file updated in 1 commit", "rebase={rebase}");
            let commits = ranges.split(' ').last().unwrap();
            assert_eq!(repo.run(["rev-list", "--count", commits]).unwrap().trim(), "1");
            if !rebase {
                let subject = repo.run(["log", "-1", "--format=%s"]).unwrap();
                assert_eq!(subject.trim(), "Merge remote-tracking branch 'origin/main'");
            }
            for dir in [origin, local] {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }

    #[test]
    fn checkout_pull_request_fetches_or_fast_forwards() {
        let t = crate::git::test_support::TestRepo::new("ops-checkout-pr");
        let bare = t.bare_remote("origin");
        t.git(&["remote", "add", "origin", bare.to_str().unwrap()]);
        t.git(&["push", "-q", "origin", "main"]);
        let first_pr = t.commit("f.txt", "1", "feature");
        t.git(&["push", "-q", "origin", "HEAD:refs/pull/7/head"]);
        t.git(&["reset", "-q", "--hard", "HEAD~1"]);
        checkout_pull_request(&t.repo, "origin", "pull/7/head", "feature", &first_pr).unwrap();
        assert_eq!(t.git(&["symbolic-ref", "--short", "HEAD"]).trim(), "feature");
        assert_eq!(t.git(&["rev-parse", "HEAD"]).trim(), first_pr);
        // The branch exists now: a newer head fast-forwards it.
        let newer = t.commit("f.txt", "2", "feature 2");
        t.git(&["checkout", "-q", "main"]);
        t.git(&["branch", "-q", "-f", "feature", &first_pr]);
        checkout_pull_request(&t.repo, "origin", "pull/7/head", "feature", &newer).unwrap();
        assert_eq!(t.git(&["rev-parse", "HEAD"]).trim(), newer);
    }

    #[test]
    fn pull_reports_up_to_date_or_summary() {
        let up = crate::git::test_support::TestRepo::new("ops-pull-up");
        let bare = up.bare_remote("origin");
        up.git(&["remote", "add", "origin", bare.to_str().unwrap()]);
        up.git(&["push", "-q", "origin", "main"]);
        let down = crate::git::test_support::TestRepo::new("ops-pull-down");
        down.git(&["remote", "add", "origin", bare.to_str().unwrap()]);
        down.git(&["fetch", "-q", "origin"]);
        down.git(&["reset", "-q", "--hard", "origin/main"]);
        let args: Vec<String> = ["pull", "--no-rebase", "origin", "main"].map(String::from).to_vec();
        assert_eq!(pull(&down.repo, &args).unwrap(), "Already up to date");
        let before = down.git(&["rev-parse", "HEAD"]).trim().to_owned();
        let after = up.commit("n.txt", "n", "new");
        up.git(&["push", "-q", "origin", "main"]);
        let message = pull(&down.repo, &args).unwrap();
        let (summary, ranges) = message.split_once('\u{1f}').unwrap();
        assert!(summary.contains('1'), "{summary}");
        assert_eq!(ranges, format!("{before}..{after} {before}..{after}"));
    }

    #[test]
    fn rename_path_uses_git_mv_for_tracked_files() {
        let t = crate::git::test_support::TestRepo::new("ops-rename");
        rename_path(Some(&t.repo), "a.txt", &t.dir.join("a.txt"), &t.dir.join("b.txt"));
        assert!(t.git(&["status", "--porcelain"]).starts_with("R  a.txt -> b.txt"));
        std::fs::write(t.dir.join("u.txt"), "u").unwrap();
        rename_path(Some(&t.repo), "u.txt", &t.dir.join("u.txt"), &t.dir.join("v.txt"));
        assert!(t.dir.join("v.txt").exists() && !t.dir.join("u.txt").exists());
        rename_path(None, "v.txt", &t.dir.join("v.txt"), &t.dir.join("w.txt"));
        assert!(t.dir.join("w.txt").exists());
    }

    #[test]
    fn cherry_pick_skips_commits_already_applied() {
        let t = crate::git::test_support::TestRepo::new("ops-cherry-pick");
        t.git(&["checkout", "-q", "-b", "topic"]);
        let a = t.commit("x.txt", "x", "add x");
        let b = t.commit("y.txt", "y", "add y");
        t.git(&["checkout", "-q", "main"]);
        t.commit("x.txt", "x", "same x");
        let args: Vec<String> = ["cherry-pick".to_owned(), a.clone(), b.clone()].to_vec();
        let message = cherry_pick_skipping_empty(&t.repo, &args, "Picked".into()).unwrap();
        assert_eq!(message, "Cherry-picked 1 of 2 commits; 1 skipped, their changes are already in the current branch");
        assert!(t.dir.join("y.txt").exists());
        let again: Vec<String> = ["cherry-pick".to_owned(), a].to_vec();
        let message = cherry_pick_skipping_empty(&t.repo, &again, "Picked".into()).unwrap();
        assert_eq!(message, "Nothing to cherry-pick: the commit's changes are already in the current branch");
        let one: Vec<String> = ["cherry-pick".to_owned(), t.git(&["rev-parse", "topic~1"]).trim().to_owned()].to_vec();
        assert!(cherry_pick_skipping_empty(&t.repo, &one, "Picked".into()).is_ok());
    }
}
