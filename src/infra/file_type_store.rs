//! File type overrides on disk, in Junction's config folder.

use std::path::PathBuf;

use crate::domain::file_type_overrides::{FileTypeOverrides, FileTypeStore};

pub struct DiskFileTypes {
    file: PathBuf,
}

impl DiskFileTypes {
    pub fn new() -> Option<Self> {
        Some(DiskFileTypes { file: crate::settings::config_dir()?.join("file-type-overrides") })
    }
}

impl FileTypeStore for DiskFileTypes {
    fn load(&self) -> FileTypeOverrides {
        FileTypeOverrides::parse(&std::fs::read_to_string(&self.file).unwrap_or_default())
    }

    fn save(&self, overrides: &FileTypeOverrides) -> Result<(), String> {
        if overrides.is_empty() {
            return match std::fs::remove_file(&self.file) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            };
        }
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&self.file, overrides.text()).map_err(|e| e.to_string())
    }
}

/// The editor language `file` (an absolute path) is overridden to.
pub fn language_of(file: &std::path::Path) -> Option<String> {
    let key = file.to_string_lossy().replace('\\', "/");
    DiskFileTypes::new()?.load().get(&key).map(str::to_owned)
}
