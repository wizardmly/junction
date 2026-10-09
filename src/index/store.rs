//! The project index: every source file's symbols and bridge sites, updated
//! incrementally (by size and modification time) and cached in the
//! repository's git directory between runs.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use super::bridge::{self, BridgeItem};
use super::libraries::ExternalIndex;
use super::lang::Lang;
use super::symbols::{self, Symbol};

/// Bumped whenever extraction changes, so stale caches are rebuilt.
const CACHE_VERSION: u32 = 1;
/// Larger files are generated or vendored; skipping them keeps indexing fast.
const MAX_FILE_SIZE: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileEntry {
    pub lang: Lang,
    pub mtime: u64,
    pub size: u64,
    pub symbols: Vec<Symbol>,
    pub bridges: Vec<BridgeItem>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ProjectIndex {
    version: u32,
    #[serde(skip)]
    pub root: PathBuf,
    /// Every file in the project (tracked and untracked, not ignored),
    /// relative with `/` separators — indexed or not, for Go to File.
    pub all_files: Vec<String>,
    pub files: BTreeMap<String, FileEntry>,
    #[serde(skip)]
    names: HashMap<String, Vec<(String, usize)>>,
    #[serde(skip)]
    keys: HashMap<String, Vec<(String, usize)>>,
    /// Dependencies and SDKs ("External Libraries"), keyed by absolute path.
    #[serde(skip)]
    pub external: std::sync::Arc<ExternalIndex>,
}

pub fn stat(path: &Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64;
    Some((meta.len(), mtime))
}

/// Files git would show: tracked plus untracked, minus ignored.
pub fn list_files(root: &Path) -> Vec<String> {
    let output = crate::git::git_process()
        .args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"])
        .current_dir(root)
        .output();
    let Ok(output) = output else { return Vec::new() };
    let mut files: Vec<String> = output
        .stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect();
    files.sort();
    files.dedup();
    files
}

pub fn index_file(root: &Path, rel: &str) -> Option<FileEntry> {
    index_path(&root.join(rel), Lang::from_path(rel)?, true)
}

/// Indexes one file; `.h` files get their language from their contents.
/// Library files skip bridges: those are the project's own boundaries.
pub fn index_path(path: &Path, mut lang: Lang, with_bridges: bool) -> Option<FileEntry> {
    let (size, mtime) = stat(path)?;
    if size > MAX_FILE_SIZE || !path.is_file() {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let mut text = String::from_utf8_lossy(&bytes);
    if path.extension().is_some_and(|e| e == "h") {
        lang = Lang::for_header(&text);
    }
    // System headers hide declarations behind attribute macros the
    // parser can't see through (glibc's __THROW, libstdc++'s _GLIBCXX_…).
    if !with_bridges && matches!(lang, Lang::C | Lang::Cpp | Lang::ObjC) {
        text = std::borrow::Cow::Owned(super::libraries::blank_macros(&text));
    }
    let symbols = symbols::extract(lang, &text);
    let rel = path.to_string_lossy();
    let bridges = if with_bridges { bridge::extract(lang, &rel, &text, &symbols) } else { Vec::new() };
    Some(FileEntry { lang, mtime, size, symbols, bridges })
}

impl ProjectIndex {
    fn cache_path(root: &Path) -> Option<PathBuf> {
        let output = crate::git::git_process()
            .args(["rev-parse", "--absolute-git-dir"])
            .current_dir(root)
            .output()
            .ok()?;
        let dir = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        (!dir.is_empty()).then(|| PathBuf::from(dir).join("junction").join("index.json"))
    }

    /// The cached index for a project, or an empty one.
    pub fn load(root: &Path) -> Self {
        let cached = Self::cache_path(root)
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice::<ProjectIndex>(&bytes).ok())
            .filter(|index| index.version == CACHE_VERSION);
        let mut index = cached.unwrap_or_default();
        index.version = CACHE_VERSION;
        index.root = root.to_path_buf();
        index.rebuild_maps();
        index
    }

    pub fn save(&self) {
        let Some(path) = Self::cache_path(&self.root) else { return };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        if let Ok(bytes) = serde_json::to_vec(self) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, bytes).is_ok() {
                std::fs::rename(&tmp, &path).ok();
            }
        }
    }

    /// Re-lists the project and re-indexes changed files in parallel,
    /// reporting (done, total) of the files that needed work. Returns
    /// whether anything changed.
    pub fn update(&mut self, progress: &(dyn Fn(usize, usize) + Sync)) -> bool {
        let all = list_files(&self.root);
        let mut changed = all.len() != self.all_files.len();
        let mut stale: Vec<String> = Vec::new();
        let mut keep: BTreeMap<String, FileEntry> = BTreeMap::new();
        for rel in &all {
            if Lang::from_path(rel).is_none() {
                continue;
            }
            let current = stat(&self.root.join(rel));
            match (self.files.remove(rel), current) {
                (Some(entry), Some((size, mtime))) if entry.size == size && entry.mtime == mtime => {
                    keep.insert(rel.clone(), entry);
                }
                (_, Some((size, _))) if size <= MAX_FILE_SIZE => stale.push(rel.clone()),
                _ => {}
            }
        }
        // Whatever is left in `files` was deleted or became ignored.
        changed |= !self.files.is_empty() || !stale.is_empty();
        self.files = keep;
        self.all_files = all;

        let total = stale.len();
        if total > 0 {
            let next = AtomicUsize::new(0);
            let done = AtomicUsize::new(0);
            let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 16);
            let root = self.root.clone();
            let results: Vec<Vec<(String, FileEntry)>> = std::thread::scope(|scope| {
                let handles: Vec<_> = (0..threads)
                    .map(|_| {
                        scope.spawn(|| {
                            let mut out = Vec::new();
                            loop {
                                let i = next.fetch_add(1, Ordering::Relaxed);
                                let Some(rel) = stale.get(i) else { break };
                                if let Some(entry) = index_file(&root, rel) {
                                    out.push((rel.clone(), entry));
                                }
                                let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                                if d % 64 == 0 || d == total {
                                    progress(d, total);
                                }
                            }
                            out
                        })
                    })
                    .collect();
                handles.into_iter().map(|h| h.join().unwrap_or_default()).collect()
            });
            for (rel, entry) in results.into_iter().flatten() {
                self.files.insert(rel, entry);
            }
        }
        if changed {
            self.rebuild_maps();
        }
        changed
    }

    /// Re-indexes one file now (after a save in the editor).
    pub fn refresh_file(&mut self, rel: &str) {
        match index_file(&self.root, rel) {
            Some(entry) => {
                self.files.insert(rel.to_owned(), entry);
            }
            None => {
                self.files.remove(rel);
            }
        }
        self.rebuild_maps();
    }

    fn rebuild_maps(&mut self) {
        self.names.clear();
        self.keys.clear();
        for (path, entry) in &self.files {
            for (i, s) in entry.symbols.iter().enumerate() {
                self.names.entry(s.name.clone()).or_default().push((path.clone(), i));
            }
            for (i, b) in entry.bridges.iter().enumerate() {
                self.keys.entry(b.key.clone()).or_default().push((path.clone(), i));
            }
        }
    }

    pub fn symbol_count(&self) -> usize {
        self.files.values().map(|f| f.symbols.len()).sum()
    }

    /// Symbols with this exact name: the project's, then the libraries'.
    pub fn symbols_named<'a>(&'a self, name: &str) -> impl Iterator<Item = (&'a str, &'a FileEntry, &'a Symbol)> + 'a {
        self.project_symbols_named(name).chain(self.external.symbols_named(name))
    }

    pub fn project_symbols_named<'a>(&'a self, name: &str) -> impl Iterator<Item = (&'a str, &'a FileEntry, &'a Symbol)> + 'a {
        self.names.get(name).into_iter().flatten().filter_map(|(path, i)| {
            let entry = self.files.get(path)?;
            Some((path.as_str(), entry, entry.symbols.get(*i)?))
        })
    }

    /// A project or library file's entry.
    pub fn file(&self, path: &str) -> Option<&FileEntry> {
        self.files.get(path).or_else(|| self.external.files.get(path))
    }

    /// Whether a path is a library file rather than the project's.
    pub fn is_external(path: &str) -> bool {
        Path::new(path).is_absolute()
    }

    /// Bridge sites with this key.
    pub fn bridges_keyed<'a>(&'a self, key: &str) -> impl Iterator<Item = (&'a str, &'a FileEntry, &'a BridgeItem)> + 'a {
        self.keys.get(key).into_iter().flatten().filter_map(|(path, i)| {
            let entry = self.files.get(path)?;
            Some((path.as_str(), entry, entry.bridges.get(*i)?))
        })
    }

    /// The project's symbol names, for Go to Symbol.
    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.names.keys()
    }

    /// Symbol names only libraries have.
    pub fn external_names(&self) -> impl Iterator<Item = &String> {
        self.external.names().filter(|n| !self.names.contains_key(*n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_incrementally() {
        let dir = std::env::temp_dir().join(format!("junction-index-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let git = |args: &[&str]| std::process::Command::new("git").args(args).current_dir(&dir).output().unwrap();
        git(&["init", "-q"]);
        std::fs::write(dir.join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
        std::fs::write(dir.join("api.h"), "int alpha(void);\n").unwrap();
        std::fs::write(dir.join(".gitignore"), "target/\n").unwrap();
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::write(dir.join("target/gen.rs"), "fn ignored() {}\n").unwrap();

        let mut index = ProjectIndex::load(&dir);
        assert!(index.update(&|_, _| {}));
        assert_eq!(index.files.len(), 2);
        assert_eq!(index.symbols_named("alpha").count(), 2);
        assert!(index.symbols_named("ignored").next().is_none());
        assert_eq!(index.bridges_keyed("c:alpha").count(), 1);
        index.save();

        let mut reloaded = ProjectIndex::load(&dir);
        assert_eq!(reloaded.files.len(), 2);
        assert!(!reloaded.update(&|_, _| panic!("nothing should be re-indexed")));

        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(dir.join("src/lib.rs"), "pub fn beta() {}\n").unwrap();
        assert!(reloaded.update(&|_, _| {}));
        assert!(reloaded.symbols_named("alpha").all(|(p, _, _)| p == "api.h"));
        assert_eq!(reloaded.symbols_named("beta").count(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }
}
