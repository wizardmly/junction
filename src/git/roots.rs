//! VCS roots of a project (Settings › Version Control › Directory
//! Mappings): the project's repository, its initialized submodules, nested
//! repositories found under it, and roots the user added or removed.

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::Repository;

/// Folders never scanned for nested repositories.
const SKIP: [&str; 8] = [".git", "node_modules", "target", "build", ".gradle", ".idea", "Pods", ".dart_tool"];
const MAX_DEPTH: usize = 3;

/// User edits to the detected roots, kept in `<gitdir>/gitglass/roots` as
/// `+path` (added) and `-path` (removed) lines, paths relative to the project.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mappings {
    pub added: Vec<PathBuf>,
    pub removed: Vec<PathBuf>,
}

fn mappings_file(repository: &Repository) -> PathBuf {
    repository.git_dir().join("gitglass").join("roots")
}

pub fn load_mappings(repository: &Repository) -> Mappings {
    let text = std::fs::read_to_string(mappings_file(repository)).unwrap_or_default();
    let mut mappings = Mappings::default();
    for line in text.lines() {
        if let Some(p) = line.strip_prefix('+') {
            mappings.added.push(PathBuf::from(p));
        } else if let Some(p) = line.strip_prefix('-') {
            mappings.removed.push(PathBuf::from(p));
        }
    }
    mappings
}

pub fn save_mappings(repository: &Repository, mappings: &Mappings) -> Result<()> {
    let path = mappings_file(repository);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut text = String::new();
    for p in &mappings.added {
        text.push_str(&format!("+{}\n", p.display()));
    }
    for p in &mappings.removed {
        text.push_str(&format!("-{}\n", p.display()));
    }
    std::fs::write(path, text)?;
    Ok(())
}

/// Nested repositories under `root` (not `root` itself), shallowest first.
pub fn scan_nested(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut queue = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = queue.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut children: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter(|e| !SKIP.contains(&e.file_name().to_string_lossy().as_ref()))
            .map(|e| e.path())
            .collect();
        children.sort();
        for child in children {
            if child.join(".git").exists() {
                found.push(child.clone());
            }
            if depth + 1 < MAX_DEPTH {
                queue.push((child, depth + 1));
            }
        }
    }
    found.sort_by_key(|p| (p.components().count(), p.clone()));
    found
}

/// Every root of the project, the project's repository first.
pub fn detect(repository: &Repository) -> Vec<PathBuf> {
    let root = repository.root().to_path_buf();
    let mappings = load_mappings(repository);
    let mut roots = vec![root.clone()];
    let mut push = |p: PathBuf| {
        if !roots.contains(&p) && !mappings.removed.iter().any(|r| root.join(r) == p) {
            roots.push(p);
        }
    };
    for path in scan_nested(&root) {
        push(path);
    }
    for added in &mappings.added {
        let p = if added.is_absolute() { added.clone() } else { root.join(added) };
        if p.join(".git").exists() {
            push(p);
        }
    }
    roots
}

/// The short label IntelliJ shows for a root: its path relative to the
/// project, or the folder name for the project itself.
pub fn label(project: &Path, root: &Path) -> String {
    if root == project {
        return project.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| root.display().to_string());
    }
    root.strip_prefix(project).map(|p| p.display().to_string()).unwrap_or_else(|_| root.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_nested_repositories_and_applies_mappings() {
        let dir = std::env::temp_dir().join(format!("gitglass-test-roots-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["", "libs/a", "libs/b", "node_modules/x", "deep/one/two/three"] {
            let p = dir.join(sub);
            std::fs::create_dir_all(&p).unwrap();
            std::process::Command::new("git").args(["init", "-q"]).current_dir(&p).output().unwrap();
        }
        let nested = scan_nested(&dir);
        assert_eq!(nested, vec![dir.join("libs/a"), dir.join("libs/b")]);

        let repo = Repository::discover(&dir, crate::git::GitConsole::default()).unwrap();
        save_mappings(&repo, &Mappings { added: vec![PathBuf::from("deep/one/two/three")], removed: vec![PathBuf::from("libs/b")] }).unwrap();
        let roots = detect(&repo);
        assert_eq!(roots, vec![repo.root().to_path_buf(), repo.root().join("libs/a"), repo.root().join("deep/one/two/three")]);
        assert_eq!(label(repo.root(), &roots[1]), "libs/a");
    }
}
