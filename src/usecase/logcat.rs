//! Logcat: follows a device's log, naming each line's app by its process
//! and marking where an app's process starts and ends, as Android Studio's
//! Logcat does.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::domain::logcat::{self, Entry, LogSource};

/// Names a process takes between the fork and the app's start.
const TENTATIVE: [&str; 5] = ["<pre-initialized>", "zygote", "zygote64", "usap32", "usap64"];

/// How often the process list is read again: when an unknown pid logs, and
/// at least this often while lines come.
const RECHECK_UNKNOWN: Duration = Duration::from_millis(250);
const RECHECK: Duration = Duration::from_secs(2);

/// The device's processes by pid.
#[derive(Default)]
pub struct Processes {
    names: HashMap<u32, String>,
}

impl Processes {
    /// The process's name, if known for good.
    pub fn name(&self, pid: u32) -> Option<&str> {
        self.names.get(&pid).map(String::as_str).filter(|n| !TENTATIVE.contains(n))
    }

    /// Takes a new process list; the PROCESS STARTED / ENDED separators of
    /// the apps that came and went. `first` only records.
    pub fn update(&mut self, list: Vec<(u32, String)>, millis: i64, first: bool) -> Vec<Entry> {
        let new: HashMap<u32, String> = list.into_iter().collect();
        let mut out = Vec::new();
        if !first {
            let mut ended: Vec<(&u32, &String)> = self.names.iter().filter(|(pid, name)| logcat::is_app_process(name) && new.get(pid) != Some(name)).collect();
            ended.sort();
            out.extend(ended.into_iter().map(|(pid, name)| logcat::separator(false, *pid, name, millis)));
            let mut started: Vec<(&u32, &String)> = new.iter().filter(|(pid, name)| logcat::is_app_process(name) && self.names.get(pid) != Some(name)).collect();
            started.sort();
            out.extend(started.into_iter().map(|(pid, name)| logcat::separator(true, *pid, name, millis)));
        }
        self.names = new;
        out
    }
}

/// Follows `serial`'s log until `stop`, handing each entry (and separator)
/// to `emit` in order.
pub fn follow(source: &dyn LogSource, serial: &str, stop: &AtomicBool, emit: &mut dyn FnMut(Entry)) -> Result<(), String> {
    let mut processes = Processes::default();
    processes.update(source.processes(serial), 0, true);
    let mut checked = Instant::now();
    let mut on_line = |line: &str| {
        let Some(mut entry) = logcat::parse_threadtime(line) else { return };
        let unknown = processes.name(entry.pid).is_none();
        if (unknown && checked.elapsed() >= RECHECK_UNKNOWN) || checked.elapsed() >= RECHECK {
            for separator in processes.update(source.processes(serial), entry.millis, false) {
                emit(separator);
            }
            checked = Instant::now();
        }
        entry.package = match processes.name(entry.pid) {
            Some(name) => name.to_owned(),
            None => format!("pid-{}", entry.pid),
        };
        // ActivityManager tells of a death before the next look would.
        let died = entry.tag == "ActivityManager" && (entry.message.contains("has died") || entry.message.starts_with("Killing "));
        let millis = entry.millis;
        emit(entry);
        if died {
            for separator in processes.update(source.processes(serial), millis, false) {
                emit(separator);
            }
            checked = Instant::now();
        }
    };
    let result = source.stream(serial, &mut on_line, stop);
    if stop.load(Ordering::Relaxed) { Ok(()) } else { result }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Fake {
        lines: Vec<&'static str>,
        /// The process lists, one per call.
        ps: Mutex<Vec<Vec<(u32, String)>>>,
    }

    impl LogSource for Fake {
        fn stream(&self, _: &str, on_line: &mut dyn FnMut(&str), _: &AtomicBool) -> Result<(), String> {
            for line in &self.lines {
                on_line(line);
                std::thread::sleep(Duration::from_millis(300));
            }
            Ok(())
        }
        fn processes(&self, _: &str) -> Vec<(u32, String)> {
            let mut ps = self.ps.lock().unwrap();
            if ps.len() > 1 { ps.remove(0) } else { ps[0].clone() }
        }
        fn clear(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn names_processes_and_marks_starts() {
        let system = (600, "system_server".to_owned());
        let app = (4321, "com.example".to_owned());
        let fake = Fake {
            lines: vec![
                "2026-10-15 02:05:48.123   600   610 I ActivityManager: Start proc 4321:com.example",
                "2026-10-15 02:05:48.200  4321  4321 D MainActivity: onCreate",
                "2026-10-15 02:05:49.000   600   610 I ActivityManager: Process com.example (pid 4321) has died",
            ],
            ps: Mutex::new(vec![vec![system.clone()], vec![system.clone(), app.clone()], vec![system.clone()]]),
        };
        let mut got = Vec::new();
        follow(&fake, "serial", &AtomicBool::new(false), &mut |e| got.push(e)).unwrap();
        let text: Vec<String> = got.iter().map(|e| format!("{}|{}", e.package, e.message)).collect();
        assert_eq!(text[0], "system_server|Start proc 4321:com.example");
        assert!(got[1].system && got[1].message.contains("PROCESS STARTED (4321) for package com.example"), "{text:?}");
        assert_eq!(text[2], "com.example|onCreate");
        assert_eq!(text[3], "system_server|Process com.example (pid 4321) has died");
        assert!(got[4].system && got[4].message.contains("PROCESS ENDED (4321)"), "{text:?}");
    }

    #[test]
    fn waits_out_the_fork_name() {
        let mut p = Processes::default();
        p.update(vec![(7, "<pre-initialized>".into())], 0, true);
        assert_eq!(p.name(7), None);
        let marks = p.update(vec![(7, "com.example".into())], 0, false);
        assert_eq!(p.name(7), Some("com.example"));
        assert_eq!(marks.len(), 1);
    }
}
