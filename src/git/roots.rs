//! VCS roots of a project (Settings › Version Control › Directory
//! Mappings): the project's repository, its initialized submodules, nested
//! repositories found under it, and roots the user added or removed.

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::Repository;

/// Folders never scanned for nested repositories.
pub const SKIP: [&str; 8] = [".git", "node_modules", "target", "build", ".gradle", ".idea", "Pods", ".dart_tool"];
const MAX_DEPTH: usize = 3;

/// The project's Directory Mappings, kept in `<gitdir>/junction/roots` as
/// `+path` (registered) and `-path` (ignored, not offered again) lines,
/// paths relative to the project.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mappings {
    pub added: Vec<PathBuf>,
    pub removed: Vec<PathBuf>,
}

/// Where a project's mappings live: in its repository, or (a folder that
/// only holds repositories) in Junction's config folder, keyed by its path.
pub(crate) fn mappings_file(project: &Path) -> Option<PathBuf> {
    match Repository::discover(project, Default::default()) {
        Ok(repository) if repository.root() == project => Some(repository.git_dir().join("junction").join("roots")),
        _ => {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            project.hash(&mut hasher);
            let name = project.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            Some(crate::settings::config_dir()?.join("folder-roots").join(format!("{name}-{:016x}", hasher.finish())))
        }
    }
}

pub fn load_mappings(project: &Path) -> Mappings {
    let text = mappings_file(project).and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
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

pub fn save_mappings(project: &Path, mappings: &Mappings) -> Result<()> {
    let path = mappings_file(project).ok_or_else(|| anyhow::anyhow!("No folder to keep the mappings in"))?;
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

/// Every registered root of the project: the project's own repository
/// first (unless the project is a folder that only holds repositories),
/// then the mapped ones. As in IntelliJ, a nested repository is used only
/// once registered (see `unregistered`); a folder of repositories opened
/// for the first time registers all of them.
pub fn detect(project: &Path) -> Vec<PathBuf> {
    let root = project.to_path_buf();
    let mut mappings = load_mappings(project);
    if !root.join(".git").exists() && mappings_file(project).is_some_and(|f| !f.exists()) {
        mappings.added = scan_nested(&root).iter().filter_map(|p| p.strip_prefix(&root).ok().map(PathBuf::from)).collect();
        let _ = save_mappings(project, &mappings);
    }
    let mut roots = if root.join(".git").exists() { vec![root.clone()] } else { Vec::new() };
    for added in &mappings.added {
        let p = if added.is_absolute() { added.clone() } else { root.join(added) };
        if p.join(".git").exists() && !roots.contains(&p) {
            roots.push(p);
        }
    }
    roots
}

/// Repositories under the project that are neither registered nor ignored:
/// IntelliJ's "Unregistered VCS roots detected" balloon offers to add them.
pub fn unregistered(project: &Path) -> Vec<PathBuf> {
    let mappings = load_mappings(project);
    let known = |p: &Path| mappings.added.iter().chain(&mappings.removed).any(|r| project.join(r) == p);
    scan_nested(project).into_iter().filter(|p| !known(p)).collect()
}

/// Registers roots (Add Roots), or stops offering them (Ignore).
pub fn register(project: &Path, roots: &[PathBuf], add: bool) -> Result<()> {
    let mut mappings = load_mappings(project);
    for root in roots {
        let relative = root.strip_prefix(project).map(PathBuf::from).unwrap_or_else(|_| root.clone());
        mappings.added.retain(|p| *p != relative);
        mappings.removed.retain(|p| *p != relative);
        if add { mappings.added.push(relative) } else { mappings.removed.push(relative) }
    }
    save_mappings(project, &mappings)
}

/// The short label IntelliJ shows for a root: its path relative to the
/// project, or the folder name for the project itself.
pub fn label(project: &Path, root: &Path) -> String {
    if root == project {
        return project.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| root.display().to_string());
    }
    root.strip_prefix(project).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_else(|_| root.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_nested_repositories_and_applies_mappings() {
        let dir = std::env::temp_dir().join(format!("junction-test-roots-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["", "libs/a", "libs/b", "node_modules/x", "deep/one/two/three"] {
            let p = dir.join(sub);
            std::fs::create_dir_all(&p).unwrap();
            std::process::Command::new("git").args(["init", "-q"]).current_dir(&p).output().unwrap();
        }
        let nested = scan_nested(&dir);
        assert_eq!(nested, vec![dir.join("libs/a"), dir.join("libs/b")]);

        let repo = Repository::discover(&dir, crate::git::GitConsole::default()).unwrap();
        save_mappings(repo.root(), &Mappings { added: vec![PathBuf::from("deep/one/two/three")], removed: vec![PathBuf::from("libs/b")] }).unwrap();
        // Nested repositories are used once registered; the rest are offered.
        assert_eq!(detect(repo.root()), vec![repo.root().to_path_buf(), repo.root().join("deep/one/two/three")]);
        assert_eq!(unregistered(repo.root()), vec![repo.root().join("libs/a")]);
        register(repo.root(), &[repo.root().join("libs/a")], true).unwrap();
        let roots = detect(repo.root());
        assert_eq!(roots, vec![repo.root().to_path_buf(), repo.root().join("deep/one/two/three"), repo.root().join("libs/a")]);
        assert!(unregistered(repo.root()).is_empty());
        assert_eq!(label(repo.root(), &roots[2]), "libs/a");
    }

    #[test]
    fn a_folder_of_repositories_has_only_the_nested_roots() {
        let dir = std::env::temp_dir().join(format!("junction-test-folder-roots-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["app", "lib", "docs/site"] {
            let p = dir.join(sub);
            std::fs::create_dir_all(&p).unwrap();
            std::process::Command::new("git").args(["init", "-q"]).current_dir(&p).output().unwrap();
        }
        std::fs::create_dir_all(dir.join("plain")).unwrap();
        // Opened the first time: every repository is registered.
        assert_eq!(detect(&dir), vec![dir.join("app"), dir.join("lib"), dir.join("docs/site")]);
        register(&dir, &[dir.join("lib")], false).unwrap();
        assert_eq!(detect(&dir), vec![dir.join("app"), dir.join("docs/site")]);
        assert!(unregistered(&dir).is_empty());
        let _ = std::fs::remove_file(mappings_file(&dir).unwrap());
        assert_eq!(label(&dir, &dir.join("docs/site")), "docs/site");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
