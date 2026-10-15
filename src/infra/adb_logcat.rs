//! The log port over adb: `adb logcat -v threadtime -v year`, the process
//! list from `ps`, and `logcat -c`.

use std::io::{BufRead as _, BufReader};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::android::adb::Adb;
use crate::android::process;
use crate::domain::logcat::{self, LogSource};

pub struct AdbLog {
    pub adb: Adb,
}

impl AdbLog {
    /// One `logcat` run; the number of lines it printed.
    fn stream_with(&self, serial: &str, args: &[&str], on_line: &mut dyn FnMut(&str), stop: &AtomicBool) -> Result<usize, String> {
        let mut child = process::command(&self.adb.path)
            .args(["-s", serial, "logcat"])
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        let stdout = child.stdout.take().ok_or("adb printed nothing")?;
        let child = std::sync::Mutex::new(child);
        let done = AtomicBool::new(false);
        let lines = std::thread::scope(|scope| {
            // Reading blocks while the device is quiet, so a watcher kills
            // the process once Logcat stops.
            scope.spawn(|| {
                while !done.load(Ordering::Relaxed) {
                    if stop.load(Ordering::Relaxed) {
                        if let Ok(mut c) = child.lock() {
                            let _ = c.kill();
                        }
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(150));
                }
            });
            let mut lines = 0;
            let mut reader = BufReader::new(stdout);
            let mut buf = Vec::new();
            while !stop.load(Ordering::Relaxed) {
                buf.clear();
                match reader.read_until(b'\n', &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        lines += 1;
                        on_line(&String::from_utf8_lossy(&buf));
                    }
                }
            }
            done.store(true, Ordering::Relaxed);
            lines
        });
        if let Ok(mut c) = child.lock() {
            let _ = c.kill();
            let _ = c.wait();
        }
        Ok(lines)
    }
}

impl LogSource for AdbLog {
    fn stream(&self, serial: &str, on_line: &mut dyn FnMut(&str), stop: &AtomicBool) -> Result<(), String> {
        // `-v year` came with Android 7; older logcat refuses the format.
        let lines = self.stream_with(serial, &["-v", "threadtime", "-v", "year"], on_line, stop)?;
        if lines == 0 && !stop.load(Ordering::Relaxed) {
            self.stream_with(serial, &["-v", "threadtime"], on_line, stop)?;
        }
        Ok(())
    }

    fn processes(&self, serial: &str) -> Vec<(u32, String)> {
        let out = self.adb.run(&["-s", serial, "shell", "ps -A -o PID,NAME"], Duration::from_secs(10)).map(|o| o.stdout).unwrap_or_default();
        let all = logcat::parse_processes(&out);
        if all.len() > 1 {
            return all;
        }
        // Before Android 8, `ps` lists every process and takes no options.
        let out = self.adb.run(&["-s", serial, "shell", "ps"], Duration::from_secs(10)).map(|o| o.stdout).unwrap_or_default();
        logcat::parse_processes(&out)
    }

    fn clear(&self, serial: &str) -> Result<(), String> {
        let out = self.adb.run(&["-s", serial, "logcat", "-c"], Duration::from_secs(5)).map_err(|e| e.to_string())?;
        if out.success { Ok(()) } else { Err(out.message()) }
    }
}
