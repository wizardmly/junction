//! A module's `res` folder on disk, for the Translations Editor.

use std::path::PathBuf;

use crate::usecase::translations::ResFiles;

pub struct DiskRes {
    pub res: PathBuf,
}

impl ResFiles for DiskRes {
    fn values_files(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let Ok(dirs) = std::fs::read_dir(&self.res) else { return out };
        for dir in dirs.flatten() {
            let folder = dir.file_name().to_string_lossy().into_owned();
            if !(folder == "values" || folder.starts_with("values-")) || !dir.path().is_dir() {
                continue;
            }
            let Ok(files) = std::fs::read_dir(dir.path()) else { continue };
            for f in files.flatten() {
                let name = f.file_name().to_string_lossy().into_owned();
                if name.ends_with(".xml") {
                    out.push((folder.clone(), name));
                }
            }
        }
        out.sort();
        out
    }

    fn read(&self, folder: &str, file: &str) -> Option<String> {
        std::fs::read_to_string(self.res.join(folder).join(file)).ok()
    }

    fn write(&self, folder: &str, file: &str, text: &str) -> Result<(), String> {
        let dir = self.res.join(folder);
        std::fs::create_dir_all(&dir).map_err(|e| format!("{folder}: {e}"))?;
        std::fs::write(dir.join(file), text).map_err(|e| format!("{folder}/{file}: {e}"))
    }
}
