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
        append(&report);
        previous(info);
    }));
    #[cfg(windows)]
    windows::install();
}

fn append(report: &str) {
    for path in log_paths() {
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = file.write_all(report.as_bytes());
        }
    }
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

/// Crashes that are not panics: an access violation or other fault reaches
/// the unhandled-exception filter, and Rust's stack-overflow report goes to
/// stderr, which a GUI process does not have, so stderr is sent to
/// `stderr.log` in the settings folder.
#[cfg(windows)]
mod windows {
    use std::os::windows::io::IntoRawHandle;

    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::Console::{STD_ERROR_HANDLE, SetStdHandle};
    use windows_sys::Win32::System::Diagnostics::Debug::{EXCEPTION_POINTERS, SetUnhandledExceptionFilter};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;

    pub fn install() {
        if let Some(dir) = crate::settings::config_dir() {
            let _ = std::fs::create_dir_all(&dir);
            if let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("stderr.log")) {
                unsafe { SetStdHandle(STD_ERROR_HANDLE, file.into_raw_handle() as HANDLE) };
            }
        }
        unsafe { SetUnhandledExceptionFilter(Some(filter)) };
    }

    unsafe extern "system" fn filter(info: *const EXCEPTION_POINTERS) -> i32 {
        let base = unsafe { GetModuleHandleW(std::ptr::null()) } as usize;
        let (code, address) = unsafe {
            info.as_ref()
                .and_then(|info| info.ExceptionRecord.as_ref())
                .map(|record| (record.ExceptionCode as u32, record.ExceptionAddress as usize))
                .unwrap_or((0, 0))
        };
        super::append(&format!(
            "Junction Studio {} crashed: exception 0x{code:08x} at 0x{address:x} (junction.exe+0x{:x})\n\n{}\n",
            env!("CARGO_PKG_VERSION"),
            address.wrapping_sub(base),
            std::backtrace::Backtrace::force_capture(),
        ));
        0 // EXCEPTION_CONTINUE_SEARCH: let Windows end the process as before
    }
}
