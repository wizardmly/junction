//! Excluded folders on disk, next to the project's Directory Mappings: in
//! its repository's `junction` folder, or Junction's config folder for a
//! folder of repositories.

use std::path::{Path, PathBuf};

use crate::domain::excluded_folders::{ExcludedFolders, ExcludedStore};

pub struct DiskExcluded {
    file: PathBuf,
}

impl DiskExcluded {
    pub fn for_project(project: &Path) -> Option<Self> {
        let roots = crate::git::roots::mappings_file(project)?;
        let name = format!("{}.excluded", roots.file_name()?.to_string_lossy());
        Some(DiskExcluded { file: roots.with_file_name(name) })
    }
}

impl ExcludedStore for DiskExcluded {
    fn load(&self) -> ExcludedFolders {
        ExcludedFolders::parse(&std::fs::read_to_string(&self.file).unwrap_or_default())
    }

    fn save(&self, folders: &ExcludedFolders) -> Result<(), String> {
        if folders.is_empty() {
            return match std::fs::remove_file(&self.file) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            };
        }
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&self.file, folders.text()).map_err(|e| e.to_string())
    }
}

/// The project's excluded folders (none without a place to keep them).
pub fn load(project: &Path) -> ExcludedFolders {
    DiskExcluded::for_project(project).map(|s| s.load()).unwrap_or_default()
}
