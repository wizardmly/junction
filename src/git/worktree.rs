//! Git worktrees: `git worktree list / add / remove`, as IntelliJ's
//! Worktrees tab shows them.

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::Repository;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    /// HEAD commit; empty for a bare entry.
    pub head: String,
    /// Short branch name, `None` when detached.
    pub branch: Option<String>,
    /// The repository's main working tree (listed first by git).
    pub main: bool,
    pub locked: bool,
    pub prunable: bool,
}

/// Parses `git worktree list --porcelain`.
pub fn parse(output: &str) -> Vec<Worktree> {
    let mut worktrees = Vec::new();
    for block in output.split("\n\n") {
        let mut entry: Option<Worktree> = None;
        for line in block.lines() {
            let (key, value) = line.split_once(' ').unwrap_or((line, ""));
            match key {
                "worktree" => {
                    entry = Some(Worktree {
                        path: PathBuf::from(value),
                        head: String::new(),
                        branch: None,
                        main: worktrees.is_empty(),
                        locked: false,
                        prunable: false,
                    })
                }
                "HEAD" => {
                    if let Some(e) = entry.as_mut() {
                        e.head = value.to_owned();
                    }
                }
                "branch" => {
                    if let Some(e) = entry.as_mut() {
                        e.branch = Some(value.strip_prefix("refs/heads/").unwrap_or(value).to_owned());
                    }
                }
                "locked" => {
                    if let Some(e) = entry.as_mut() {
                        e.locked = true;
                    }
                }
                "prunable" => {
                    if let Some(e) = entry.as_mut() {
                        e.prunable = true;
                    }
                }
                _ => {}
            }
        }
        if let Some(e) = entry {
            worktrees.push(e);
        }
    }
    worktrees
}

pub fn list(repository: &Repository) -> Result<Vec<Worktree>> {
    Ok(parse(&repository.run(["worktree", "list", "--porcelain"])?))
}

/// New Worktree: checks out `branch` at `path`, creating the branch from
/// `start` when `new_branch` is set.
pub fn add(repository: &Repository, path: &Path, branch: &str, new_branch: bool, start: Option<&str>) -> Result<()> {
    let path = path.to_string_lossy().into_owned();
    let mut args: Vec<String> = vec!["worktree".into(), "add".into()];
    if new_branch {
        args.extend(["-b".into(), branch.to_owned(), path]);
        if let Some(start) = start.filter(|s| !s.is_empty()) {
            args.push(start.to_owned());
        }
    } else {
        args.extend([path, branch.to_owned()]);
    }
    repository.run(args)?;
    Ok(())
}

pub fn remove(repository: &Repository, path: &Path, force: bool) -> Result<()> {
    let path = path.to_string_lossy().into_owned();
    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.push(&path);
    repository.run(args)?;
    Ok(())
}

pub fn prune(repository: &Repository) -> Result<()> {
    repository.run(["worktree", "prune"])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain() {
        let output = "worktree /src/app\nHEAD 1111\nbranch refs/heads/main\n\nworktree /src/app-fix\nHEAD 2222\ndetached\nlocked\n\nworktree /src/gone\nHEAD 3333\nbranch refs/heads/feature/x\nprunable gitdir file points to non-existent location\n";
        let list = parse(output);
        assert_eq!(list.len(), 3);
        assert!(list[0].main && list[0].branch.as_deref() == Some("main"));
        assert!(!list[1].main && list[1].branch.is_none() && list[1].locked);
        assert_eq!(list[2].branch.as_deref(), Some("feature/x"));
        assert!(list[2].prunable);
    }

    #[test]
    fn add_and_remove() {
        use crate::git::GitConsole;
        use std::process::Command;
        let dir = std::env::temp_dir().join(format!("junction-test-worktree-{}", std::process::id()));
        let other = dir.with_file_name(format!("junction-test-worktree-b-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&other);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            let out = Command::new("git").arg("-C").arg(&dir).args(args).env("GIT_AUTHOR_NAME", "T").env("GIT_AUTHOR_EMAIL", "t@x").env("GIT_COMMITTER_NAME", "T").env("GIT_COMMITTER_EMAIL", "t@x").output().unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        };
        git(&["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("a"), "1").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "c"]);
        let repo = Repository::discover(&dir, GitConsole::default()).unwrap();
        add(&repo, &other, "fix", true, Some("main")).unwrap();
        let list = list(&repo).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[1].branch.as_deref(), Some("fix"));
        remove(&repo, &list[1].path, false).unwrap();
        assert_eq!(super::list(&repo).unwrap().len(), 1);
    }
}
