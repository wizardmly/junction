//! Analyze Dependencies…: what each Java / Kotlin file of a scope uses, as
//! IntelliJ's Dependency Viewer finds it: the project files its imports and
//! same-package names resolve to, and the libraries (packages) the rest
//! come from.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::domain::imports;

use super::store::ProjectIndex;

/// One file's dependencies.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDeps {
    pub file: String,
    /// Project files, project-relative.
    pub project: Vec<String>,
    /// Library packages ("androidx.compose.runtime").
    pub libraries: Vec<String>,
}

fn is_jvm(path: &str) -> bool {
    path.ends_with(".kt") || path.ends_with(".kts") || path.ends_with(".java")
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// The dependencies of `files` (project-relative), each read with `read`.
pub fn dependencies(index: &ProjectIndex, files: &[String], read: &dyn Fn(&str) -> Option<String>) -> Vec<FileDeps> {
    let mut packages: HashMap<String, String> = HashMap::new();
    let mut package_of = |path: &str| -> String {
        packages.entry(path.to_owned()).or_insert_with(|| read(path).map(|t| imports::header(&t).package).unwrap_or_default()).clone()
    };
    // Each folder's top-level names, for same-package and wildcard uses.
    let mut by_dir: HashMap<&str, Vec<(&str, &str)>> = HashMap::new();
    for (path, entry) in &index.files {
        if is_jvm(path) {
            let names = by_dir.entry(dir_of(path)).or_default();
            names.extend(entry.symbols.iter().filter(|s| s.container.is_none()).map(|s| (s.name.as_str(), path.as_str())));
        }
    }
    let mut out = Vec::new();
    for file in files.iter().filter(|f| is_jvm(f)) {
        let Some(text) = read(file) else { continue };
        let header = imports::header(&text);
        let words: HashSet<&str> = text
            .lines()
            .filter(|l| !(l.trim_start().starts_with("import ") || l.trim_start().starts_with("package ")))
            .flat_map(|l| l.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$')))
            .filter(|w| !w.is_empty())
            .collect();
        let mut project = BTreeSet::new();
        let mut libraries = BTreeSet::new();
        for (_, qualified) in &header.imports {
            let (owner, name) = qualified.rsplit_once('.').unwrap_or(("", qualified));
            let found: Vec<String> = index
                .project_symbols_named(name)
                .filter(|(_, _, s)| s.container.is_none())
                .map(|(p, _, _)| p.to_owned())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .filter(|p| package_of(p) == owner)
                .collect();
            if found.is_empty() {
                // An import of a class's member: its owner's package.
                let package = owner.rsplit_once('.').filter(|(_, last)| last.starts_with(char::is_uppercase)).map_or(owner, |(p, _)| p);
                libraries.insert(package.to_owned());
            } else {
                project.extend(found);
            }
        }
        // The file's own package (its folder) and its wildcard imports.
        for (package, own) in std::iter::once((header.package.clone(), true)).chain(header.wildcards.iter().map(|w| (w.clone(), false))) {
            let suffix = package.replace('.', "/");
            let mut scanned = false;
            let dirs: Vec<&str> = if own { vec![dir_of(file)] } else { by_dir.keys().copied().filter(|d| d.ends_with(&suffix)).collect() };
            for dir in dirs {
                for (name, path) in by_dir.get(dir).into_iter().flatten() {
                    if *path != file && words.contains(name) && (own || package_of(path) == package) {
                        project.insert(path.to_string());
                        scanned = true;
                    }
                }
            }
            if !own && !scanned {
                libraries.insert(package);
            }
        }
        out.push(FileDeps { file: file.clone(), project: project.into_iter().collect(), libraries: libraries.into_iter().collect() });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::lang::Lang;
    use crate::index::store::FileEntry;
    use crate::index::symbols;

    #[test]
    fn resolves_imports_and_same_package() {
        let texts: HashMap<&str, &str> = HashMap::from([
            ("app/src/a/Main.kt", "package a\n\nimport b.Helper\nimport androidx.compose.runtime.Composable\n\nfun main() { Helper(); Local() }\n"),
            ("app/src/a/Local.kt", "package a\n\nclass Local\n"),
            ("app/src/b/Helper.kt", "package b\n\nclass Helper\n"),
        ]);
        let mut index = ProjectIndex::default();
        for (path, text) in &texts {
            index.insert_for_test(path, FileEntry { lang: Lang::Kotlin, mtime: 0, size: 0, symbols: symbols::extract(Lang::Kotlin, text), bridges: Vec::new() });
        }
        let read = |p: &str| texts.get(p).map(|t| t.to_string());
        let deps = dependencies(&index, &["app/src/a/Main.kt".to_owned()], &read);
        assert_eq!(deps[0].project, vec!["app/src/a/Local.kt".to_owned(), "app/src/b/Helper.kt".to_owned()]);
        assert_eq!(deps[0].libraries, vec!["androidx.compose.runtime".to_owned()]);
    }
}

#[cfg(test)]
#[test]
#[ignore]
fn bench_dependencies() {
    let Ok(root) = std::env::var("JUNCTION_BENCH") else { return };
    let root = std::path::PathBuf::from(root);
    let index = ProjectIndex::load(&root);
    let files: Vec<String> = index.files.keys().filter(|p| p.starts_with("app/") && is_jvm(p)).cloned().collect();
    let read = |p: &str| std::fs::read_to_string(root.join(p)).ok();
    let start = std::time::Instant::now();
    let deps = dependencies(&index, &files, &read);
    let edges: usize = deps.iter().map(|d| d.project.len()).sum();
    eprintln!("{} files, {edges} project edges in {:?}", deps.len(), start.elapsed());
}
