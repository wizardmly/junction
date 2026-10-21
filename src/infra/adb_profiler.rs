//! The Profiler's numbers over adb: `ps`, `/proc`, `dumpsys meminfo`, and
//! `am dumpheap` pulled to the computer.

use std::path::Path;
use std::time::{Duration, Instant};

use crate::android::adb::Adb;
use crate::domain::profiler::{self, ProfiledProcess, ProfilerSource};

pub struct AdbProfiler {
    pub adb: Adb,
}

impl AdbProfiler {
    fn shell(&self, serial: &str, command: &str, timeout: Duration) -> Result<String, String> {
        let out = self.adb.run(&["-s", serial, "shell", command], timeout).map_err(|e| e.to_string())?;
        if !out.success && out.stdout.trim().is_empty() {
            return Err(out.message());
        }
        Ok(out.stdout)
    }
}

impl ProfilerSource for AdbProfiler {
    fn processes(&self, serial: &str) -> Result<Vec<ProfiledProcess>, String> {
        Ok(profiler::parse_processes(&self.shell(serial, "ps -A -o PID,NAME", Duration::from_secs(10))?))
    }

    fn cpu(&self, serial: &str, pid: u32) -> Result<String, String> {
        self.shell(serial, &format!("cat /proc/stat /proc/{pid}/stat"), Duration::from_secs(5))
    }

    fn meminfo(&self, serial: &str, pid: u32) -> Result<String, String> {
        self.shell(serial, &format!("dumpsys meminfo {pid}"), Duration::from_secs(10))
    }

    fn heap_dump(&self, serial: &str, pid: u32, to: &Path) -> Result<Vec<u8>, String> {
        let remote = format!("/data/local/tmp/junction-{pid}.hprof");
        let out = self.shell(serial, &format!("rm -f {remote}; am dumpheap {pid} {remote}"), Duration::from_secs(120))?;
        if let Some(error) = out.lines().find(|l| l.contains("Error") || l.contains("Exception")) {
            return Err(error.trim().to_owned());
        }
        // Older Androids return before the dump is written: wait for its size to settle.
        let started = Instant::now();
        let mut last = None;
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let size = self.shell(serial, &format!("stat -c %s {remote} 2>/dev/null"), Duration::from_secs(5)).ok().and_then(|s| s.trim().parse::<u64>().ok());
            if size.is_some() && size == last {
                break;
            }
            if started.elapsed() > Duration::from_secs(120) {
                return Err("The heap dump didn't finish in 2 minutes".into());
            }
            last = size;
        }
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let local = to.to_string_lossy().into_owned();
        let pulled = self.adb.run(&["-s", serial, "pull", &remote, &local], Duration::from_secs(300)).map_err(|e| e.to_string())?;
        let _ = self.shell(serial, &format!("rm -f {remote}"), Duration::from_secs(5));
        if !pulled.success {
            return Err(pulled.message());
        }
        std::fs::read(to).map_err(|e| e.to_string())
    }
}
