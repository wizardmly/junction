//! The Profiler's numbers over adb: `ps`, `/proc`, `dumpsys meminfo`, and
//! `am dumpheap` and `am profile` pulled to the computer.

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
        self.settle(serial, &remote, "The heap dump")?;
        self.pull(serial, &remote, to)
    }

    fn start_method_trace(&self, serial: &str, pid: u32, sampling_us: Option<u32>) -> Result<(), String> {
        let sampling = sampling_us.map(|us| format!("--sampling {us} ")).unwrap_or_default();
        let out = self.shell(serial, &format!("rm -f {}; am profile start {sampling}{pid} {}", trace_file(pid), trace_file(pid)), Duration::from_secs(20))?;
        failure(&out).map_or(Ok(()), Err)
    }

    fn stop_method_trace(&self, serial: &str, pid: u32, to: &Path) -> Result<Vec<u8>, String> {
        let remote = trace_file(pid);
        let out = self.shell(serial, &format!("am profile stop {pid}"), Duration::from_secs(20))?;
        if let Some(error) = failure(&out) {
            return Err(error);
        }
        // The app writes the trace after the command returns.
        self.settle(serial, &remote, "The method trace")?;
        self.pull(serial, &remote, to)
    }

    fn start_system_trace(&self, serial: &str, package: &str) -> Result<u32, String> {
        let config = system_trace_config(crate::infra::adb_background_tasks::package_arg(package)?);
        // The config goes in on stdin: perfetto may not read files under /data/local/tmp.
        let out = self.shell(serial, &format!("rm -f {SYSTEM_TRACE}; echo '{config}' | perfetto --background --txt -c - -o {SYSTEM_TRACE}"), Duration::from_secs(20))?;
        // `--background` prints the recording's pid.
        out.lines().rev().find_map(|l| l.trim().parse::<u32>().ok()).ok_or_else(|| {
            let message = out.trim();
            if message.is_empty() { "perfetto didn't start".to_owned() } else { message.lines().last().unwrap_or(message).to_owned() }
        })
    }

    fn stop_system_trace(&self, serial: &str, id: u32, to: &Path) -> Result<Vec<u8>, String> {
        // perfetto writes the trace when it is told to stop, and exits.
        self.shell(serial, &format!("kill -TERM {id} 2>/dev/null; true"), Duration::from_secs(10))?;
        let started = Instant::now();
        while self.shell(serial, &format!("kill -0 {id} 2>/dev/null && echo running"), Duration::from_secs(5)).is_ok_and(|o| o.contains("running")) {
            if started.elapsed() > Duration::from_secs(60) {
                return Err("perfetto didn't stop in a minute".to_owned());
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        self.settle(serial, SYSTEM_TRACE, "The system trace")?;
        self.pull(serial, SYSTEM_TRACE, to)
    }
}

/// Where perfetto writes: the one directory it may write to on user builds.
const SYSTEM_TRACE: &str = "/data/misc/perfetto-traces/junction.pftrace";

/// Android Studio's System Trace configuration: the scheduler, CPU
/// frequencies, thread names and creation, and the app's atrace sections,
/// in perfetto's text format (no single quotes: it is passed through `echo`).
fn system_trace_config(package: &str) -> String {
    let events = ["sched/sched_switch", "sched/sched_waking", "sched/sched_wakeup_new", "task/task_newtask", "task/task_rename", "power/cpu_frequency", "ftrace/print"];
    let categories = ["am", "dalvik", "gfx", "input", "view", "wm", "sched", "freq"];
    let events: String = events.iter().map(|e| format!("ftrace_events: \"{e}\" ")).collect();
    let categories: String = categories.iter().map(|c| format!("atrace_categories: \"{c}\" ")).collect();
    format!(
        "buffers {{ size_kb: 65536 fill_policy: RING_BUFFER }} \
         buffers {{ size_kb: 4096 fill_policy: RING_BUFFER }} \
         data_sources {{ config {{ name: \"linux.ftrace\" target_buffer: 0 ftrace_config {{ {events}{categories}atrace_apps: \"{package}\" compact_sched {{ enabled: false }} }} }} }} \
         data_sources {{ config {{ name: \"linux.process_stats\" target_buffer: 1 process_stats_config {{ scan_all_processes_on_start: true }} }} }} \
         duration_ms: 600000"
    )
}

fn trace_file(pid: u32) -> String {
    format!("/data/local/tmp/junction-{pid}.trace")
}

fn failure(out: &str) -> Option<String> {
    out.lines().find(|l| l.contains("Error") || l.contains("Exception") || l.contains("not ")).map(|l| l.trim().to_owned())
}

impl AdbProfiler {
    /// Waits until the remote file exists and its size stops changing.
    fn settle(&self, serial: &str, remote: &str, what: &str) -> Result<(), String> {
        let started = Instant::now();
        let mut last = None;
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let size = self.shell(serial, &format!("stat -c %s {remote} 2>/dev/null"), Duration::from_secs(5)).ok().and_then(|s| s.trim().parse::<u64>().ok());
            if size.is_some_and(|s| s > 0) && size == last {
                return Ok(());
            }
            if started.elapsed() > Duration::from_secs(120) {
                return Err(format!("{what} didn't finish in 2 minutes"));
            }
            last = size;
        }
    }

    /// Copies the remote file to `to`, removes it from the device, and
    /// returns its bytes.
    fn pull(&self, serial: &str, remote: &str, to: &Path) -> Result<Vec<u8>, String> {
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let local = to.to_string_lossy().into_owned();
        let pulled = self.adb.run(&["-s", serial, "pull", remote, &local], Duration::from_secs(300)).map_err(|e| e.to_string())?;
        let _ = self.shell(serial, &format!("rm -f {remote}"), Duration::from_secs(5));
        if !pulled.success {
            return Err(pulled.message());
        }
        std::fs::read(to).map_err(|e| e.to_string())
    }
}
