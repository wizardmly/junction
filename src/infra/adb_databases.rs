//! An app's databases over adb: `run-as` reads its private `databases`
//! folder, which works for debuggable apps, as Android Studio's Database
//! Inspector needs too.

use std::time::Duration;

use crate::android::adb::Adb;
use crate::android::process;
use crate::domain::sqlite_file::{self, DatabaseSource};

pub struct AdbDatabases {
    pub adb: Adb,
}

/// `'…'` for the device's shell.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

impl AdbDatabases {
    fn cat(&self, serial: &str, package: &str, file: &str) -> Result<Vec<u8>, String> {
        let command = format!("run-as {} cat {}", quote(package), quote(&format!("databases/{file}")));
        let (_, out, err) = process::run_bytes(&self.adb.path, &["-s", serial, "exec-out", &command], Duration::from_secs(120)).map_err(|e| e.to_string())?;
        if out.is_empty() && !err.trim().is_empty() {
            return Err(err.trim().to_owned());
        }
        Ok(out)
    }
}

impl DatabaseSource for AdbDatabases {
    fn databases(&self, serial: &str, package: &str) -> Result<Vec<String>, String> {
        let command = format!("run-as {} ls -1 databases", quote(package));
        let out = self.adb.run(&["-s", serial, "shell", &command], Duration::from_secs(10)).map_err(|e| e.to_string())?;
        let text = format!("{}{}", out.stdout, out.stderr);
        if text.contains("run-as:") {
            return Err(text.lines().find(|l| l.contains("run-as:")).unwrap_or_default().trim().to_owned());
        }
        // No `databases` folder: no databases yet.
        if text.contains("No such file") {
            return Ok(Vec::new());
        }
        Ok(sqlite_file::database_files(&out.stdout))
    }

    fn read(&self, serial: &str, package: &str, name: &str) -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
        let data = self.cat(serial, package, name)?;
        if !data.starts_with(b"SQLite format 3\0") {
            let text = String::from_utf8_lossy(&data[..data.len().min(200)]).trim().to_owned();
            return Err(if text.is_empty() { format!("{name} is empty") } else { text });
        }
        // A WAL starts with its magic number; anything else (an error, nothing) means none.
        let wal = self.cat(serial, package, &format!("{name}-wal")).ok().filter(|w| w.len() >= 32 && matches!(w[..4], [0x37, 0x7f, 0x06, 0x82 | 0x83]));
        Ok((data, wal))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shell_quoting() {
        assert_eq!(super::quote("it's.db"), "'it'\\''s.db'");
    }
}
