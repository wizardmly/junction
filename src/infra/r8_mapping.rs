//! R8 / ProGuard mapping files: loading one for retrace (read lazily from
//! disk, class by class) and finding the project's own under
//! `<module>/build/outputs/mapping/<variant>/mapping.txt`, as Android
//! Studio's `LogcatR8MappingsToken` does.

use std::fs::File;
use std::io::{BufRead as _, BufReader, Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};

use crate::domain::retrace::{IndexBuilder, MappingSource, Retracer};

struct FileSource(PathBuf);

impl MappingSource for FileSource {
    fn read(&self, at: u64, len: u32) -> Option<String> {
        let mut file = File::open(&self.0).ok()?;
        file.seek(SeekFrom::Start(at)).ok()?;
        let mut bytes = Vec::with_capacity(len as usize);
        file.take(len as u64).read_to_end(&mut bytes).ok()?;
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }
}

/// Indexes a mapping file. Fails when it has no class mappings at all.
pub fn load(path: &Path) -> Result<Retracer, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut reader = BufReader::with_capacity(1 << 16, file);
    let mut builder = IndexBuilder::default();
    let mut line = Vec::new();
    let mut offset = 0u64;
    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        builder.line(String::from_utf8_lossy(&line).trim_end_matches(['\r', '\n']), offset);
        offset += n as u64;
    }
    let retracer = builder.finish(offset, Box::new(FileSource(path.to_path_buf())));
    if retracer.class_count() == 0 {
        return Err("The file has no class mappings.".into());
    }
    Ok(retracer)
}

/// The `# pg_map_id:` in a mapping's header comments.
pub fn map_id(path: &Path) -> Option<String> {
    let reader = BufReader::new(File::open(path).ok()?);
    for line in reader.lines().map_while(Result::ok) {
        if !line.starts_with('#') {
            return None;
        }
        if let Some(id) = line.strip_prefix("# pg_map_id: ") {
            return Some(id.trim().to_owned());
        }
    }
    None
}

/// The project's mapping files, newest first.
pub fn project_mappings(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        if let Ok(variants) = std::fs::read_dir(dir.join("build/outputs/mapping")) {
            out.extend(variants.flatten().map(|v| v.path().join("mapping.txt")).filter(|p| p.is_file()));
        }
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if depth < 4 && entry.file_type().is_ok_and(|t| t.is_dir()) && !name.starts_with('.') && !matches!(name.as_str(), "build" | "node_modules" | "target" | "out" | "src") {
                walk(&entry.path(), depth + 1, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, 0, &mut out);
    out.sort_by_key(|p| std::cmp::Reverse(std::fs::metadata(p).and_then(|m| m.modified()).ok()));
    out
}

/// The project's mapping file R8 wrote with this map id.
pub fn find_by_map_id(root: &Path, id: &str) -> Option<PathBuf> {
    project_mappings(root).into_iter().find(|p| map_id(p).as_deref() == Some(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_and_loads() {
        let root = std::env::temp_dir().join(format!("junction-r8-{}", std::process::id()));
        let dir = root.join("app/build/outputs/mapping/release");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(root.join("app/build.gradle.kts"), "").unwrap();
        let mapping = dir.join("mapping.txt");
        std::fs::write(&mapping, "# compiler: R8\n# pg_map_id: abc\ncom.example.Foo -> a.a:\n    1:2:void run():10:11 -> a\n").unwrap();
        assert_eq!(find_by_map_id(&root, "abc"), Some(mapping.clone()));
        assert_eq!(find_by_map_id(&root, "zzz"), None);
        let r = load(&mapping).unwrap();
        assert_eq!(r.retrace("at a.a.a(SourceFile:2)").unwrap(), vec!["at com.example.Foo.run(Foo.java:11)"]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
