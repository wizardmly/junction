//! The project's resources from its `res` folders on disk.

use std::path::{Path, PathBuf};

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

/// 0-based line and character column of a byte offset.
fn position(text: &str, offset: usize) -> (u32, u32) {
    let before = &text[..offset.min(text.len())];
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    (before.matches('\n').count() as u32, before[line_start..].chars().count() as u32)
}

impl ResourceSource for ResFolders {
    fn scan(&self) -> Vec<Declaration> {
        let mut out = Vec::new();
        for dir in self.res_dirs() {
            let Ok(folders) = std::fs::read_dir(&dir) else { continue };
            for folder in folders.flatten() {
                let Ok(files) = std::fs::read_dir(folder.path()) else { continue };
                for file in files.flatten() {
                    let path = file.path();
                    let rel = self.rel(&path);
                    if let Some(folder) = res::values_folder(&rel) {
                        let Ok(text) = std::fs::read_to_string(&path) else { continue };
                        for (resource, at, value) in res::parse_values(&text) {
                            let (line, col) = position(&text, at);
                            out.push(Declaration { resource, path: rel.clone(), folder: folder.clone(), line, col, value });
                        }
                    } else if let Some((resource, folder)) = res::file_resource(&rel) {
                        // Layouts, menus and graphs declare ids too.
                        if rel.ends_with(".xml") && matches!(resource.kind.as_str(), "layout" | "menu" | "navigation" | "xml" | "drawable" | "transition") {
                            if let Ok(text) = std::fs::read_to_string(&path) {
                                for (id, at) in res::parse_new_ids(&text) {
                                    let (line, col) = position(&text, at);
                                    out.push(Declaration { resource: id, path: rel.clone(), folder: folder.clone(), line, col, value: None });
                                }
                            }
                        }
                        out.push(Declaration { resource, path: rel, folder, line: 0, col: 0, value: None });
                    }
                }
            }
        }
        out
    }
}
