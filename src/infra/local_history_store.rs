//! Local History on disk: `<cache>/local-history/<project>-<hash>/` with an
//! `index` of revisions (one line each) and each text once under `content/`.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::domain::local_history::{HistoryStore, Revision};
use crate::index::libraries::{cache_dir, stable_hash};

pub struct DiskHistory {
    dir: PathBuf,
}

impl DiskHistory {
    pub fn for_project(root: &Path) -> Option<Self> {
        let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let dir = cache_dir()?.join("local-history").join(format!("{name}-{:016x}", stable_hash(&root.to_string_lossy())));
        Some(Self { dir })
    }

    #[cfg(test)]
    pub fn at(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn index(&self) -> PathBuf {
        self.dir.join("index")
    }
}

impl HistoryStore for DiskHistory {
    fn revisions(&self) -> Vec<Revision> {
        std::fs::read_to_string(self.index()).map(|t| t.lines().filter_map(Revision::parse).collect()).unwrap_or_default()
    }

    fn append(&self, revision: &Revision) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(self.index()).map_err(|e| e.to_string())?;
        writeln!(file, "{}", revision.line()).map_err(|e| e.to_string())
    }

    fn rewrite(&self, kept: &[Revision]) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let text: String = kept.iter().map(|r| r.line() + "\n").collect();
        let tmp = self.dir.join("index.tmp");
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, self.index()).map_err(|e| e.to_string())?;
        let used: std::collections::HashSet<&str> = kept.iter().map(|r| r.content.as_str()).collect();
        if let Ok(entries) = std::fs::read_dir(self.dir.join("content")) {
            for entry in entries.flatten() {
                if !used.contains(entry.file_name().to_string_lossy().as_ref()) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        Ok(())
    }

    fn put_content(&self, text: &str) -> Result<String, String> {
        let hash = format!("{:016x}{:08x}", stable_hash(text), text.len());
        let dir = self.dir.join("content");
        let path = dir.join(&hash);
        if !path.exists() {
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            std::fs::write(&path, text).map_err(|e| e.to_string())?;
        }
        Ok(hash)
    }

    fn content(&self, hash: &str) -> Option<String> {
        std::fs::read_to_string(self.dir.join("content").join(hash)).ok()
    }
}
