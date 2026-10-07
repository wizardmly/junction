//! Submodules: `git submodule status`, update, sync, and the
//! "Subproject commit" text git shows when a submodule pointer changes.

use anyhow::Result;

use super::Repository;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmoduleState {
    /// Checked out at the recorded commit.
    UpToDate,
    /// Checked out at another commit than the superproject records (`+`).
    Changed,
    /// Not cloned yet (`-`).
    Uninitialized,
    /// Merge conflict in the pointer (`U`).
    Conflict,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Submodule {
    pub path: String,
    /// The commit checked out (or recorded, when uninitialized).
    pub commit: String,
    pub state: SubmoduleState,
    /// `git describe` output git prints in parentheses, if any.
    pub describe: Option<String>,
    pub url: Option<String>,
}

/// Parses `git submodule status --recursive`.
pub fn parse_status(output: &str) -> Vec<Submodule> {
    output
        .lines()
        .filter(|l| l.len() > 41)
        .map(|line| {
            let state = match line.as_bytes()[0] {
                b'+' => SubmoduleState::Changed,
                b'-' => SubmoduleState::Uninitialized,
                b'U' => SubmoduleState::Conflict,
                _ => SubmoduleState::UpToDate,
            };
            let rest = &line[1..];
            let (commit, rest) = rest.split_once(' ').unwrap_or((rest, ""));
            let (path, describe) = match rest.rsplit_once(" (") {
                Some((path, d)) if d.ends_with(')') => (path, Some(d.trim_end_matches(')').to_owned())),
                _ => (rest, None),
            };
            Submodule { path: path.to_owned(), commit: commit.to_owned(), state, describe, url: None }
        })
        .collect()
}

pub fn list(repository: &Repository) -> Result<Vec<Submodule>> {
    if !repository.root().join(".gitmodules").exists() {
        return Ok(Vec::new());
    }
    let mut list = parse_status(&repository.run(["submodule", "status", "--recursive"])?);
    let urls = repository.run(["config", "--file", ".gitmodules", "--get-regexp", r"^submodule\..*\.url$"]).unwrap_or_default();
    let paths = repository.run(["config", "--file", ".gitmodules", "--get-regexp", r"^submodule\..*\.path$"]).unwrap_or_default();
    for line in paths.lines() {
        let Some((key, path)) = line.split_once(' ') else { continue };
        let name = key.trim_start_matches("submodule.").trim_end_matches(".path");
        let url = urls.lines().find_map(|l| {
            let (k, v) = l.split_once(' ')?;
            (k == format!("submodule.{name}.url")).then(|| v.to_owned())
        });
        if let Some(sub) = list.iter_mut().find(|s| s.path == path) {
            sub.url = url;
        }
    }
    Ok(list)
}

/// Update Submodules: `git submodule update --init --recursive`, limited
/// to `paths` when given.
pub fn update(repository: &Repository, paths: &[String]) -> Result<()> {
    let mut args: Vec<String> = vec!["submodule".into(), "update".into(), "--init".into(), "--recursive".into()];
    if !paths.is_empty() {
        args.push("--".into());
        args.extend(paths.iter().cloned());
    }
    repository.run(args)?;
    Ok(())
}

pub fn sync(repository: &Repository) -> Result<()> {
    repository.run(["submodule", "sync", "--recursive"])?;
    Ok(())
}

/// Paths that are submodules (gitlinks) in the index.
pub fn gitlink_paths(repository: &Repository) -> Vec<String> {
    repository
        .run(["ls-files", "--stage", "-z"])
        .unwrap_or_default()
        .split('\0')
        .filter_map(|entry| {
            let (meta, path) = entry.split_once('\t')?;
            meta.starts_with("160000 ").then(|| path.to_owned())
        })
        .collect()
}

/// What git shows for a submodule in a diff: "Subproject commit <sha>",
/// with "-dirty" when its working tree has changes.
pub fn work_tree_text(repository: &Repository, path: &str) -> Option<String> {
    let nested = repository.nested(path).ok()?;
    if nested.root() == repository.root() {
        return None;
    }
    let head = nested.run(["rev-parse", "HEAD"]).ok()?;
    let dirty = nested.run(["status", "--porcelain", "--ignore-submodules=none"]).map(|s| !s.trim().is_empty()).unwrap_or(false);
    Some(format!("Subproject commit {}{}\n", head.trim(), if dirty { "-dirty" } else { "" }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_submodule_status() {
        let out = " 1111111111111111111111111111111111111111 libs/a (v1.0)\n+2222222222222222222222222222222222222222 libs/b (heads/main)\n-3333333333333333333333333333333333333333 libs/c\n";
        let list = parse_status(out);
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].state, SubmoduleState::UpToDate);
        assert_eq!(list[0].describe.as_deref(), Some("v1.0"));
        assert_eq!(list[1].state, SubmoduleState::Changed);
        assert_eq!(list[1].path, "libs/b");
        assert_eq!(list[2].state, SubmoduleState::Uninitialized);
        assert_eq!(list[2].describe, None);
    }
}
