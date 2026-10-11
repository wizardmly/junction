//! The project index: every source file's symbols and bridge sites, updated
//! incrementally (by size and modification time) and cached in the
//! repository's git directory between runs.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use super::bridge::{self, BridgeItem};
use super::libraries::ExternalIndex;
use super::lang::Lang;
use super::symbols::{self, Symbol};

/// Bumped whenever extraction or the cache format changes, so stale caches are rebuilt.
const CACHE_VERSION: u32 = 4;
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
    /// Where each symbol name and bridge key occurs: (file, position in
    /// its list). Files share one path string.
    /// Names also keep [`char_mask`] of themselves, so a search can skip
    /// most names without reading them.
    #[serde(skip)]
    names: HashMap<String, (u64, Vec<(Arc<str>, u32)>)>,
    #[serde(skip)]
    keys: HashMap<String, Vec<(Arc<str>, u32)>>,
    /// The symbols naming each supertype (types extending or implementing
    /// it, Rust trait methods), for the gutter's implementation markers.
    #[serde(skip)]
    subtypes: HashMap<String, Vec<(Arc<str>, u32)>>,
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

/// Which letters and digits (case-insensitive) a name contains, one bit
/// each: a query matches only names having all of its own.
pub fn char_mask(text: &str) -> u64 {
    text.bytes().fold(0, |mask, b| match b.to_ascii_lowercase() {
        c @ b'a'..=b'z' => mask | 1 << (c - b'a'),
        c @ b'0'..=b'9' => mask | 1 << (26 + c - b'0'),
        _ => mask,
    })
}

/// Files that changed since the index was taken, see [`ProjectIndex::scan`].
pub struct IndexChanges {
    all_files: Vec<String>,
    removed: Vec<String>,
    updated: Vec<(String, FileEntry)>,
}

/// Indexes files on every core, reporting (done, total).
fn index_files(root: &Path, files: &[String], progress: &(dyn Fn(usize, usize) + Sync)) -> Vec<(String, FileEntry)> {
    let total = files.len();
    if total == 0 {
        return Vec::new();
    }
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 16);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(rel) = files.get(i) else { break };
                        if let Some(entry) = index_file(root, rel) {
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
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    })
}

impl ProjectIndex {
    fn cache_path(root: &Path) -> Option<PathBuf> {
        let output = crate::git::git_process()
            .args(["rev-parse", "--absolute-git-dir"])
            .current_dir(root)
            .output()
            .ok()?;
        let dir = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        (!dir.is_empty()).then(|| PathBuf::from(dir).join("junction").join("index.bin"))
    }

    /// The cached index for a project, or an empty one.
    pub fn load(root: &Path) -> Self {
        let path = Self::cache_path(root);
        if let Some(path) = &path {
            // The JSON cache of earlier versions.
            std::fs::remove_file(path.with_extension("json")).ok();
        }
        let cached = path
            .and_then(|p| read_cache::<ProjectIndex>(&p, CACHE_VERSION))
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
        let tmp = path.with_extension("bin.tmp");
        let written = std::fs::File::create(&tmp).ok().is_some_and(|file| {
            let mut out = std::io::BufWriter::with_capacity(1 << 20, file);
            bincode::serialize_into(&mut out, self).is_ok() && std::io::Write::flush(&mut out).is_ok()
        });
        if written {
            std::fs::rename(&tmp, &path).ok();
        } else {
            std::fs::remove_file(&tmp).ok();
        }
    }

    /// Re-lists the project and re-indexes changed files in parallel,
    /// reporting (done, total) of the files that needed work. Returns
    /// whether anything changed.
    pub fn update(&mut self, progress: &(dyn Fn(usize, usize) + Sync)) -> bool {
        match self.scan(progress) {
            Some(changes) => {
                self.apply(changes);
                true
            }
            None => false,
        }
    }

    /// What changed on disk since the index was taken, with the changed
    /// files already parsed; `None` when nothing did. Only reads the
    /// index, so queries keep answering from it meanwhile.
    pub fn scan(&self, progress: &(dyn Fn(usize, usize) + Sync)) -> Option<IndexChanges> {
        let all = list_files(&self.root);
        let mut stale: Vec<String> = Vec::new();
        let mut kept = HashSet::new();
        for rel in &all {
            if Lang::from_path(rel).is_none() {
                continue;
            }
            let current = stat(&self.root.join(rel));
            match (self.files.get(rel), current) {
                (Some(entry), Some((size, mtime))) if entry.size == size && entry.mtime == mtime => {
                    kept.insert(rel.as_str());
                }
                (_, Some((size, _))) if size <= MAX_FILE_SIZE => stale.push(rel.clone()),
                _ => {}
            }
        }
        // Deleted, now ignored, too large, or about to be re-indexed.
        let removed: Vec<String> = self.files.keys().filter(|rel| !kept.contains(rel.as_str())).cloned().collect();
        if stale.is_empty() && removed.is_empty() && all == self.all_files {
            return None;
        }
        let updated = index_files(&self.root, &stale, progress);
        Some(IndexChanges { all_files: all, removed, updated })
    }

    /// Applies what [`scan`](Self::scan) found.
    pub fn apply(&mut self, changes: IndexChanges) {
        self.all_files = changes.all_files;
        // Many changes (a branch switch): rebuilding the lookups is quicker.
        let rebuild = changes.removed.len() + changes.updated.len() > 1000.max(self.files.len() / 8);
        for rel in &changes.removed {
            if let Some(old) = self.files.remove(rel) {
                if !rebuild {
                    self.unmap(rel, &old);
                }
            }
        }
        for (rel, entry) in changes.updated {
            if !rebuild {
                self.map(&rel, &entry);
            }
            self.files.insert(rel, entry);
        }
        if rebuild {
            self.rebuild_maps();
        }
    }

    /// Re-indexes one file now (after a save in the editor).
    pub fn refresh_file(&mut self, rel: &str) {
        if let Some(old) = self.files.remove(rel) {
            self.unmap(rel, &old);
        }
        if let Some(entry) = index_file(&self.root, rel) {
            self.map(rel, &entry);
            self.files.insert(rel.to_owned(), entry);
        }
    }

    fn map(&mut self, path: &str, entry: &FileEntry) {
        let path: Arc<str> = path.into();
        for (i, s) in entry.symbols.iter().enumerate() {
            self.names.entry(s.name.clone()).or_insert_with(|| (char_mask(&s.name), Vec::new())).1.push((path.clone(), i as u32));
            for parent in &s.supers {
                self.subtypes.entry(parent.clone()).or_default().push((path.clone(), i as u32));
            }
        }
        for (i, b) in entry.bridges.iter().enumerate() {
            self.keys.entry(b.key.clone()).or_default().push((path.clone(), i as u32));
        }
    }

    fn unmap(&mut self, path: &str, entry: &FileEntry) {
        for s in &entry.symbols {
            if let Some((_, list)) = self.names.get_mut(&s.name) {
                list.retain(|(p, _)| &**p != path);
                if list.is_empty() {
                    self.names.remove(&s.name);
                }
            }
            for parent in &s.supers {
                if let Some(list) = self.subtypes.get_mut(parent) {
                    list.retain(|(p, _)| &**p != path);
                    if list.is_empty() {
                        self.subtypes.remove(parent);
                    }
                }
            }
        }
        for b in &entry.bridges {
            if let Some(list) = self.keys.get_mut(&b.key) {
                list.retain(|(p, _)| &**p != path);
                if list.is_empty() {
                    self.keys.remove(&b.key);
                }
            }
        }
    }

    fn rebuild_maps(&mut self) {
        self.names.clear();
        self.keys.clear();
        self.subtypes.clear();
        let files = std::mem::take(&mut self.files);
        for (path, entry) in &files {
            self.map(path, entry);
        }
        self.files = files;
    }

    pub fn symbol_count(&self) -> usize {
        self.files.values().map(|f| f.symbols.len()).sum()
    }

    /// Symbols with this exact name: the project's, then the libraries'.
    pub fn symbols_named<'a>(&'a self, name: &str) -> impl Iterator<Item = (&'a str, &'a FileEntry, &'a Symbol)> + 'a {
        self.project_symbols_named(name).chain(self.external.symbols_named(name))
    }

    pub fn project_symbols_named<'a>(&'a self, name: &str) -> impl Iterator<Item = (&'a str, &'a FileEntry, &'a Symbol)> + 'a {
        self.names.get(name).into_iter().flat_map(|(_, at)| at).filter_map(|(path, i)| {
            let (path, entry) = self.files.get_key_value(&**path)?;
            Some((path.as_str(), entry, entry.symbols.get(*i as usize)?))
        })
    }

    /// The project's symbols that name `parent` as a supertype.
    pub fn subtypes_of<'a>(&'a self, parent: &str) -> impl Iterator<Item = (&'a str, &'a FileEntry, &'a Symbol)> + 'a {
        self.subtypes.get(parent).into_iter().flatten().filter_map(|(path, i)| {
            let (path, entry) = self.files.get_key_value(&**path)?;
            Some((path.as_str(), entry, entry.symbols.get(*i as usize)?))
        })
    }

    #[cfg(test)]
    pub fn insert_for_test(&mut self, path: &str, entry: FileEntry) {
        self.map(path, &entry);
        self.files.insert(path.to_owned(), entry);
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
            let (path, entry) = self.files.get_key_value(&**path)?;
            Some((path.as_str(), entry, entry.bridges.get(*i as usize)?))
        })
    }

    /// The project's symbol names, for Go to Symbol.
    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.names.keys()
    }

    /// The project's symbol names with their [`char_mask`].
    pub fn names_with_masks(&self) -> impl Iterator<Item = (&String, u64)> {
        self.names.iter().map(|(name, (mask, _))| (name, *mask))
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
    fn rejects_old_and_corrupt_caches() {
        let path = std::env::temp_dir().join(format!("junction-cache-{}.bin", std::process::id()));
        // A string's bytes where a length should be: read as a length they ask
        // for exabytes, which used to abort at startup.
        let mut bytes = CACHE_VERSION.to_le_bytes().to_vec();
        bytes.extend_from_slice(b"keystore/release.jks");
        std::fs::write(&path, &bytes).unwrap();
        assert!(read_cache::<ProjectIndex>(&path, CACHE_VERSION).is_none());
        bytes[..4].copy_from_slice(&3u32.to_le_bytes());
        std::fs::write(&path, &bytes).unwrap();
        assert!(read_cache::<ProjectIndex>(&path, CACHE_VERSION).is_none());
        std::fs::remove_file(&path).ok();
    }

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

/// Reads a bincode cache whose first field is its `version: u32`. A file
/// from another version is skipped before decoding: its layout differs, so a
/// string's bytes could be read as a length, and allocating that aborts the
/// process instead of failing. Decoding is also capped at the file's size so
/// a truncated or corrupt file fails cleanly.
pub fn read_cache<T: serde::de::DeserializeOwned>(path: &Path, version: u32) -> Option<T> {
    use bincode::Options;
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut head = [0u8; 4];
    file.read_exact(&mut head).ok()?;
    if u32::from_le_bytes(head) != version {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .allow_trailing_bytes()
        .with_limit(len)
        .deserialize_from(std::io::BufReader::with_capacity(1 << 20, file))
        .ok()
}
