//! Writes a panic's message, location and backtrace to `crash.log`, next to
//! the executable and in the settings folder: a Windows GUI build has no
//! console, so a crash would otherwise leave no trace.

use std::backtrace::Backtrace;
use std::io::Write;
use std::path::PathBuf;

pub fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let report = format!(
            "Junction Studio {} crashed at {} (unix time)\nthread '{}' {info}\n\n{}\n",
            env!("CARGO_PKG_VERSION"),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
            thread.name().unwrap_or("<unnamed>"),
            Backtrace::force_capture(),
        );
        for path in log_paths() {
            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                let _ = file.write_all(report.as_bytes());
            }
        }
        previous(info);
    }));
}

fn log_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(PathBuf::from)) {
        paths.push(dir.join("crash.log"));
    }
    if let Some(dir) = crate::settings::config_dir() {
        let _ = std::fs::create_dir_all(&dir);
        paths.push(dir.join("crash.log"));
    }
    paths
}
