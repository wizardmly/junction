//! HotSwap's ports over the disk: the class files Gradle wrote since a
//! time, and the SDK's d8 turning them into the dex an Android VM takes.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::domain::hotswap::{class_name, ClassFiles, Dexer};

pub struct DiskClassFiles;

impl ClassFiles for DiskClassFiles {
    fn changed_since(&self, roots: &[PathBuf], since: SystemTime) -> Vec<(String, PathBuf)> {
        let mut found = Vec::new();
        for root in roots {
            let mut dirs = vec![root.clone()];
            while let Some(dir) = dirs.pop() {
                let Ok(entries) = std::fs::read_dir(&dir) else { continue };
                for entry in entries.flatten() {
                    let path = entry.path();
                    let Ok(kind) = entry.file_type() else { continue };
                    if kind.is_dir() {
                        dirs.push(path);
                    } else if path.extension().is_some_and(|e| e == "class") && entry.metadata().and_then(|m| m.modified()).is_ok_and(|t| t > since) {
                        if let Some(name) = path.strip_prefix(root).ok().and_then(class_name) {
                            found.push((name, path));
                        }
                    }
                }
            }
        }
        found.sort();
        found.dedup_by(|a, b| a.0 == b.0);
        found
    }

    fn read(&self, file: &Path) -> Result<Vec<u8>, String> {
        std::fs::read(file).map_err(|e| e.to_string())
    }
}

/// The SDK's newest build-tools d8, with the newest platform's android.jar
/// as the library.
pub struct D8 {
    d8: PathBuf,
    android_jar: Option<PathBuf>,
}

impl D8 {
    pub fn locate(sdk: &Path) -> Option<Self> {
        let name = if cfg!(windows) { "d8.bat" } else { "d8" };
        let d8 = newest(&sdk.join("build-tools")).map(|d| d.join(name)).filter(|p| p.is_file())?;
        let android_jar = newest(&sdk.join("platforms")).map(|d| d.join("android.jar")).filter(|p| p.is_file());
        Some(Self { d8, android_jar })
    }
}

/// The subfolder with the highest version (`34.0.0`, `android-34`).
fn newest(dir: &Path) -> Option<PathBuf> {
    let version = |p: &Path| -> Vec<u32> { p.file_name().map(|n| n.to_string_lossy().split(|c: char| !c.is_ascii_digit()).filter_map(|s| s.parse().ok()).collect()).unwrap_or_default() };
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_dir()).max_by_key(|p| version(p))
}

impl Dexer for D8 {
    fn dex(&self, class_files: &[PathBuf]) -> Result<Vec<Vec<u8>>, String> {
        static RUN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let out = std::env::temp_dir().join(format!("junction-hotswap-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        // One dex per class, synthetic ones (lambdas) apart: ART redefines
        // a class from a dex holding just it.
        let mut command = std::process::Command::new(&self.d8);
        command.args(["--debug", "--min-api", "26", "--file-per-class", "--output"]).arg(&out);
        if let Some(jar) = &self.android_jar {
            command.arg("--lib").arg(jar);
        }
        command.args(class_files);
        let result = command.output().map_err(|e| format!("d8: {e}"));
        let dexes = result.and_then(|output| {
            if !output.status.success() {
                return Err(format!("d8 failed: {}", String::from_utf8_lossy(&output.stderr).trim()));
            }
            class_files.iter().map(|file| read_dex(&out, file)).collect()
        });
        let _ = std::fs::remove_dir_all(&out);
        dexes
    }
}

/// d8's dex of a class file: its class's path with `.dex`. The class's
/// name is in the class file itself; its path under the output root
/// matches it for what Gradle writes.
fn read_dex(out: &Path, class_file: &Path) -> Result<Vec<u8>, String> {
    let bytes = std::fs::read(class_file).map_err(|e| e.to_string())?;
    let name = this_class(&bytes).ok_or_else(|| format!("{}: not a class file", class_file.display()))?;
    std::fs::read(out.join(format!("{name}.dex"))).map_err(|e| format!("d8 wrote no dex for {name}: {e}"))
}

/// A class file's own name (`com/example/Calc$Inner`), from its constant
/// pool.
fn this_class(bytes: &[u8]) -> Option<String> {
    let u16_at = |at: usize| -> Option<usize> { Some(u16::from_be_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]) as usize) };
    if bytes.get(..4)? != [0xCA, 0xFE, 0xBA, 0xBE] {
        return None;
    }
    let count = u16_at(8)?;
    let mut at = 10;
    // Each entry's offset, by index (1-based; longs and doubles take two).
    let mut entries = vec![0usize; count];
    let mut i = 1;
    while i < count {
        entries[i] = at;
        let tag = *bytes.get(at)?;
        at += match tag {
            1 => 3 + u16_at(at + 1)?,
            3 | 4 | 9 | 10 | 11 | 12 | 17 | 18 => 5,
            5 | 6 => {
                i += 1;
                9
            }
            7 | 8 | 16 | 19 | 20 => 3,
            15 => 4,
            _ => return None,
        };
        i += 1;
    }
    // access_flags, then this_class: a Class entry naming a Utf8 entry.
    let class = u16_at(at + 2)?;
    let utf8 = u16_at(*entries.get(class)? + 1)?;
    let start = *entries.get(utf8)?;
    let len = u16_at(start + 1)?;
    String::from_utf8(bytes.get(start + 3..start + 3 + len)?.to_vec()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_class_files_written_since() {
        let dir = std::env::temp_dir().join(format!("junction-class-files-{}", std::process::id()));
        let classes = dir.join("classes");
        std::fs::create_dir_all(classes.join("com/example")).unwrap();
        std::fs::write(classes.join("com/example/Old.class"), b"").unwrap();
        let since = SystemTime::now();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(classes.join("com/example/Calc$Inner.class"), b"").unwrap();
        std::fs::write(classes.join("com/example/notes.txt"), b"").unwrap();
        let found = DiskClassFiles.changed_since(&[classes.clone(), dir.join("missing")], since);
        assert_eq!(found, vec![("com.example.Calc$Inner".to_owned(), classes.join("com/example/Calc$Inner.class"))]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reads_a_class_files_name() {
        // A minimal class file: Utf8 "a/B", Class #1, then access flags and
        // this_class #2.
        let mut bytes = vec![0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 52, 0, 3];
        bytes.extend_from_slice(&[1, 0, 3, b'a', b'/', b'B']);
        bytes.extend_from_slice(&[7, 0, 1]);
        bytes.extend_from_slice(&[0, 0x21, 0, 2]);
        assert_eq!(this_class(&bytes).as_deref(), Some("a/B"));
        assert_eq!(this_class(b"nope"), None);
    }
}
