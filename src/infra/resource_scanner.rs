//! The project's resources from its `res` folders on disk.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::domain::android_resources::{self as res, Declaration, ResourceSource};

pub struct ResFolders {
    pub root: PathBuf,
}

/// Folders never holding a module's sources.
const SKIP: [&str; 9] = [".git", "build", ".gradle", ".idea", "node_modules", "target", "Pods", ".dart_tool", ".cxx"];

impl ResFolders {
    fn res_dirs(&self) -> Vec<PathBuf> {
        fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else { return };
            for entry in entries.flatten() {
                if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                if SKIP.contains(&name.as_str()) || name.starts_with('.') {
                    continue;
                }
                let path = entry.path();
                // `<module>/src/<source set>/res`.
                if name == "res" && path.parent().and_then(|p| p.parent()).and_then(|p| p.file_name()).is_some_and(|n| n == "src") {
                    out.push(path);
                } else if depth < 8 {
                    walk(&path, depth + 1, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.root, 0, &mut out);
        out
    }

    fn rel(&self, path: &Path) -> String {
        path.strip_prefix(&self.root).unwrap_or(path).to_string_lossy().replace('\\', "/")
    }
}

/// 0-based line and character column of increasing byte offsets, counted
/// on from the previous one (a 700 KB strings.xml has thousands).
struct Lines<'a> {
    text: &'a str,
    at: usize,
    line: u32,
    line_start: usize,
}

impl<'a> Lines<'a> {
    fn new(text: &'a str) -> Self {
        Self { text, at: 0, line: 0, line_start: 0 }
    }

    fn position(&mut self, offset: usize) -> (u32, u32) {
        let offset = offset.min(self.text.len());
        if offset < self.at {
            *self = Self::new(self.text);
        }
        for (i, b) in self.text.as_bytes()[self.at..offset].iter().enumerate() {
            if *b == b'\n' {
                self.line += 1;
                self.line_start = self.at + i + 1;
            }
        }
        self.at = offset;
        (self.line, self.text[self.line_start..offset].chars().count() as u32)
    }
}

impl ResFolders {
    /// One file's declarations.
    fn scan_file(&self, path: &Path, folder_arc: &Arc<str>, names: &mut Interner) -> Vec<Declaration> {
        let mut out = Vec::new();
        let rel = self.rel(path);
        if res::values_folder(&rel).is_some() {
            let Ok(text) = std::fs::read_to_string(path) else { return out };
            let rel: Arc<str> = rel.into();
            // Values only from the default folder (what hover and previews
            // show); a translated string's text is not needed.
            let keep = &**folder_arc == "values";
            let mut lines = Lines::new(&text);
            for (mut resource, at, value) in res::parse_values(&text) {
                let (line, col) = lines.position(at);
                names.share(&mut resource);
                let value = value.filter(|_| keep || &*resource.kind != "string");
                out.push(Declaration { resource, path: rel.clone(), folder: folder_arc.clone(), line, col, value });
            }
        } else if let Some((resource, _)) = res::file_resource(&rel) {
            let ids = rel.ends_with(".xml") && matches!(&*resource.kind, "layout" | "menu" | "navigation" | "xml" | "drawable" | "transition");
            let rel: Arc<str> = rel.into();
            // Layouts, menus and graphs declare ids too.
            if ids {
                if let Ok(text) = std::fs::read_to_string(path) {
                    let mut lines = Lines::new(&text);
                    for (mut id, at) in res::parse_new_ids(&text) {
                        let (line, col) = lines.position(at);
                        names.share(&mut id);
                        out.push(Declaration { resource: id, path: rel.clone(), folder: folder_arc.clone(), line, col, value: None });
                    }
                }
            }
            out.push(Declaration { resource, path: rel, folder: folder_arc.clone(), line: 0, col: 0, value: None });
        }
        out
    }
}

impl ResFolders {
    /// Changes when a res file is added, removed or edited: the table is
    /// rebuilt only then.
    pub fn stamp(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for dir in self.res_dirs() {
            for folder in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                for file in std::fs::read_dir(folder.path()).into_iter().flatten().flatten() {
                    let Ok(meta) = file.metadata() else { continue };
                    file.file_name().hash(&mut hasher);
                    folder.file_name().hash(&mut hasher);
                    meta.len().hash(&mut hasher);
                    meta.modified().ok().hash(&mut hasher);
                }
            }
            dir.hash(&mut hasher);
        }
        hasher.finish()
    }
}

impl ResourceSource for ResFolders {
    /// Reads the files on every core: a big app has hundreds of
    /// translated values files.
    fn scan(&self) -> Vec<Declaration> {
        let mut files: Vec<(PathBuf, Arc<str>)> = Vec::new();
        for dir in self.res_dirs() {
            let Ok(folders) = std::fs::read_dir(&dir) else { continue };
            for folder in folders.flatten() {
                let name: Arc<str> = folder.file_name().to_string_lossy().into();
                let Ok(entries) = std::fs::read_dir(folder.path()) else { continue };
                files.extend(entries.flatten().map(|f| (f.path(), name.clone())));
            }
        }
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
        let chunk = files.len().div_ceil(threads).max(1);
        std::thread::scope(|scope| {
            let workers: Vec<_> = files.chunks(chunk).map(|part| scope.spawn(move || {
                let mut names = Interner::default();
                part.iter().flat_map(|(path, folder)| self.scan_file(path, folder, &mut names)).collect::<Vec<_>>()
            })).collect();
            workers.into_iter().flat_map(|w| w.join().unwrap_or_default()).collect()
        })
    }
}

/// One copy of each kind and name per worker: thousands of translations
/// declare the same names, and keeping a copy each fragments the heap.
#[derive(Default)]
struct Interner(std::collections::HashSet<Arc<str>>);

impl Interner {
    fn get(&mut self, s: &Arc<str>) -> Arc<str> {
        match self.0.get(&**s) {
            Some(shared) => shared.clone(),
            None => {
                self.0.insert(s.clone());
                s.clone()
            }
        }
    }

    fn share(&mut self, r: &mut res::ResourceRef) {
        r.kind = self.get(&r.kind);
        r.name = self.get(&r.name);
    }
}
