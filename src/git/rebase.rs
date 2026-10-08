//! Interactive rebase driven from our own editor: we write the todo list,
//! and git runs Junction as `GIT_SEQUENCE_EDITOR` to pick it up.

use std::path::PathBuf;

use anyhow::{Context as _, Result};

use super::log::{Commit, parse_log};
use super::Repository;

/// Set when git runs Junction as the sequence editor: our todo file's path.
pub const TODO_ENV: &str = "JUNCTION_REBASE_TODO";

/// When started as the sequence editor, copies our todo over git's and exits.
pub fn handle_sequence_editor() -> bool {
    let Some(todo) = std::env::var_os(TODO_ENV) else { return false };
    let Some(target) = std::env::args_os().nth(1) else { return false };
    let ok = std::fs::copy(&todo, &target).is_ok();
    std::process::exit(if ok { 0 } else { 1 });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Pick,
    Reword,
    Edit,
    Squash,
    Fixup,
    Drop,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::Pick => "Pick",
            Action::Reword => "Reword",
            Action::Edit => "Edit",
            Action::Squash => "Squash",
            Action::Fixup => "Fixup",
            Action::Drop => "Drop",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub commit: Commit,
    pub action: Action,
    /// New message for Reword (and for the result of a Squash).
    pub message: Option<String>,
}

/// Commits a rebase from `base` would replay, oldest first.
pub fn commits_since(repository: &Repository, base: &str) -> Result<Vec<Commit>> {
    let range = if base == "--root" { "HEAD".to_owned() } else { format!("{base}..HEAD") };
    let output = repository.run(["log", "--reverse", "--no-merges", super::ops::LOG_FORMAT, &range])?;
    Ok(parse_log(&output))
}

/// Plans the rebase like `--autosquash`: each `fixup! <subject>` / `squash! <subject>`
/// commit moves after the commit it names, as Fixup / Squash. Oldest first.
pub fn autosquash(commits: Vec<Commit>) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::with_capacity(commits.len());
    for commit in commits {
        let target = [("fixup! ", Action::Fixup), ("squash! ", Action::Squash)]
            .into_iter()
            .find_map(|(prefix, action)| commit.subject.strip_prefix(prefix).map(|s| (s.to_owned(), action)));
        let anchor = target.as_ref().and_then(|(subject, _)| {
            entries.iter().position(|e| e.commit.subject == *subject || e.commit.hash.starts_with(subject.as_str()))
        });
        match (target, anchor) {
            (Some((_, action)), Some(anchor)) => {
                // After the target and any fixups already attached to it.
                let mut at = anchor + 1;
                while at < entries.len() && matches!(entries[at].action, Action::Fixup | Action::Squash) {
                    at += 1;
                }
                entries.insert(at, Entry { commit, action, message: None });
            }
            _ => entries.push(Entry { commit, action: Action::Pick, message: None }),
        }
    }
    entries
}

/// Whether `hash` is on the current branch, so history from it can be rewritten.
pub fn is_on_current_branch(repository: &Repository, hash: &str) -> bool {
    repository.run(["merge-base", "--is-ancestor", hash, "HEAD"]).is_ok()
}

/// The parent to rebase onto when rewriting from `hash`, or `--root`.
pub fn base_of(repository: &Repository, hash: &str) -> String {
    let parent = format!("{hash}^");
    if repository.run(["rev-parse", "--verify", "-q", &parent]).is_ok() { parent } else { "--root".into() }
}

/// The full message of a commit, for Reword's editor.
pub fn message_of(repository: &Repository, hash: &str) -> String {
    repository.run(["log", "-1", "--format=%B", hash]).map(|m| m.trim_end().to_owned()).unwrap_or_default()
}

fn temp_file(name: &str, content: &str) -> Result<PathBuf> {
    let path = std::env::temp_dir().join(format!("junction-{}-{name}", std::process::id()));
    std::fs::write(&path, content).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// Builds git's todo list. A new message is applied with an `exec` that
/// amends, so git never needs to open an editor.
pub fn todo(entries: &[Entry]) -> Result<String> {
    let mut out = String::new();
    // The message for the commit being built: a pick plus any squashes/fixups after it.
    let mut group_message: Option<&String> = None;
    for (ix, entry) in entries.iter().enumerate() {
        let verb = match entry.action {
            Action::Pick | Action::Reword => "pick",
            Action::Edit => "edit",
            Action::Squash => "squash",
            Action::Fixup => "fixup",
            Action::Drop => "drop",
        };
        out.push_str(&format!("{verb} {} {}\n", entry.commit.hash, entry.commit.subject));
        match entry.action {
            Action::Drop => continue,
            Action::Squash | Action::Fixup => group_message = entry.message.as_ref().or(group_message),
            _ => group_message = entry.message.as_ref(),
        }
        let next_folds = entries[ix + 1..]
            .iter()
            .find(|n| n.action != Action::Drop)
            .is_some_and(|n| matches!(n.action, Action::Squash | Action::Fixup));
        if let Some(message) = group_message.filter(|_| !next_folds) {
            let file = temp_file(&format!("msg-{ix}"), message)?;
            let path = file.to_string_lossy().replace('\\', "/");
            out.push_str(&format!("exec git commit --amend --no-verify --allow-empty -q -F \"{path}\"\n"));
        }
    }
    Ok(out)
}

/// Runs `git rebase -i` with our todo list.
pub fn run_interactive(repository: &Repository, base: &str, entries: &[Entry]) -> Result<String> {
    let exe = std::env::current_exe()?.to_string_lossy().into_owned();
    run_with_editor(repository, base, entries, &format!("\"{exe}\""))
}

fn run_with_editor(repository: &Repository, base: &str, entries: &[Entry], editor: &str) -> Result<String> {
    // Like IntelliJ, refuse to flatten merges silently.
    let range = if base == "--root" { "HEAD".to_owned() } else { format!("{base}..HEAD") };
    if !repository.run(["rev-list", "--merges", &range])?.trim().is_empty() {
        anyhow::bail!("The commits to rebase contain merge commits; interactive rebase would flatten them");
    }
    let todo_file = temp_file("todo", &todo(entries)?)?;
    let todo_path = todo_file.to_string_lossy().into_owned();
    let result = repository.run_with_env(
        ["rebase", "-i", "--autostash", base],
        &[("GIT_SEQUENCE_EDITOR", editor), (TODO_ENV, todo_path.as_str())],
    );
    let _ = std::fs::remove_file(&todo_file);
    result?;
    Ok(match repository.state() {
        super::RepositoryState::Rebasing => "Rebase stopped for editing; Continue when ready".into(),
        _ => format!("Rebased {} commits", entries.iter().filter(|e| e.action != Action::Drop).count()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(hash: &str, subject: &str) -> Commit {
        Commit {
            hash: hash.into(),
            parents: vec![],
            author_name: String::new(),
            author_email: String::new(),
            author_time: 0,
            subject: subject.into(),
        }
    }

    #[test]
    fn todo_amends_new_messages_after_squash_groups() {
        let entries = vec![
            Entry { commit: commit("a1", "one"), action: Action::Pick, message: Some("first".into()) },
            Entry { commit: commit("b2", "two"), action: Action::Squash, message: None },
            Entry { commit: commit("c3", "three"), action: Action::Drop, message: Some("ignored".into()) },
            Entry { commit: commit("d4", "four"), action: Action::Reword, message: Some("better".into()) },
        ];
        let todo = todo(&entries).unwrap();
        let lines: Vec<&str> = todo.lines().map(|l| l.split(" -F").next().unwrap()).collect();
        assert_eq!(
            lines,
            vec![
                "pick a1 one",
                "squash b2 two",
                "exec git commit --amend --no-verify --allow-empty -q",
                "drop c3 three",
                "pick d4 four",
                "exec git commit --amend --no-verify --allow-empty -q",
            ]
        );
    }

    #[test]
    fn autosquash_moves_fixups_after_their_target() {
        let commits = vec![commit("a", "one"), commit("b", "two"), commit("c", "fixup! one"), commit("d", "squash! one")];
        let plan: Vec<(String, Action)> = autosquash(commits).into_iter().map(|e| (e.commit.hash, e.action)).collect();
        assert_eq!(
            plan,
            vec![("a".into(), Action::Pick), ("c".into(), Action::Fixup), ("d".into(), Action::Squash), ("b".into(), Action::Pick)]
        );
    }

    #[cfg(unix)]
    #[test]
    fn squashes_drops_and_rewords() {
        use crate::git::GitConsole;
        let dir = std::env::temp_dir().join(format!("junction-test-rebase-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git").arg("-C").arg(&dir).args(args).output().unwrap();
            assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.name", "T"]);
        git(&["config", "user.email", "t@x"]);
        for name in ["base", "one", "two", "three"] {
            std::fs::write(dir.join(format!("{name}.txt")), name).unwrap();
            git(&["add", "."]);
            git(&["commit", "-qm", name]);
        }
        let repo = Repository::discover(&dir, GitConsole::default()).unwrap();
        let commits = commits_since(&repo, "HEAD~3").unwrap();
        assert_eq!(commits.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(), vec!["one", "two", "three"]);
        let entries = vec![
            Entry { commit: commits[0].clone(), action: Action::Reword, message: Some("one, reworded".into()) },
            Entry { commit: commits[1].clone(), action: Action::Fixup, message: None },
            Entry { commit: commits[2].clone(), action: Action::Drop, message: None },
        ];
        run_with_editor(&repo, "HEAD~3", &entries, "cp \"$JUNCTION_REBASE_TODO\"").unwrap();
        let log = repo.run(["log", "--format=%s"]).unwrap();
        assert_eq!(log.lines().collect::<Vec<_>>(), vec!["one, reworded", "base"]);
        assert!(dir.join("two.txt").exists() && !dir.join("three.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
