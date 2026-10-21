//! The Profiler's samples, as Android Studio's CPU and Memory timelines show
//! them: the app's share of the CPU from `/proc` (its ticks against the
//! whole machine's), its thread count, and the memory categories of
//! `dumpsys meminfo`'s App Summary.

/// The memory categories of the App Summary, in KB (Proportional Set Size).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Memory {
    pub java: u64,
    pub native: u64,
    pub graphics: u64,
    pub stack: u64,
    pub code: u64,
    /// Private Other and System.
    pub others: u64,
    pub total: u64,
}

impl Memory {
    /// Android Studio's order and names, bottom to top of its stacked chart.
    pub fn categories(&self) -> [(&'static str, u64); 6] {
        [("Java", self.java), ("Native", self.native), ("Graphics", self.graphics), ("Stack", self.stack), ("Code", self.code), ("Others", self.others)]
    }
}

fn kb(line: &str) -> Option<u64> {
    line.split(':').nth(1)?.split_whitespace().next()?.parse().ok()
}

/// The App Summary of `dumpsys meminfo <pid>`.
pub fn parse_meminfo(text: &str) -> Option<Memory> {
    let summary = &text[text.find("App Summary")?..];
    let mut memory = Memory::default();
    let mut private_other = 0;
    let mut system = 0;
    for line in summary.lines() {
        let t = line.trim_start();
        let value = || kb(t).unwrap_or(0);
        if t.starts_with("Java Heap:") {
            memory.java = value();
        } else if t.starts_with("Native Heap:") {
            memory.native = value();
        } else if t.starts_with("Code:") {
            memory.code = value();
        } else if t.starts_with("Stack:") {
            memory.stack = value();
        } else if t.starts_with("Graphics:") {
            memory.graphics = value();
        } else if t.starts_with("Private Other:") {
            private_other = value();
        } else if t.starts_with("System:") {
            system = value();
        } else if t.starts_with("TOTAL PSS:") || t.starts_with("TOTAL:") {
            memory.total = value();
            break;
        }
    }
    memory.others = private_other + system;
    if memory.total == 0 {
        memory.total = memory.java + memory.native + memory.graphics + memory.stack + memory.code + memory.others;
    }
    (memory.total > 0).then_some(memory)
}

/// The jiffies counters the CPU share is computed from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuTicks {
    /// All CPUs' time, busy or idle.
    pub machine: u64,
    /// The process's user and system time.
    pub process: u64,
    pub threads: u32,
}

/// `cat /proc/stat /proc/<pid>/stat`: the first line's total, and the
/// process's utime + stime and thread count.
pub fn parse_cpu(text: &str) -> Option<CpuTicks> {
    let machine: u64 = text.lines().find(|l| l.starts_with("cpu "))?.split_whitespace().skip(1).filter_map(|v| v.parse::<u64>().ok()).sum();
    // `<pid> (<name>) S …`: the name may hold spaces, so fields count from its `)`.
    let line = text.lines().find(|l| l.split_whitespace().next().is_some_and(|f| f.chars().all(|c| c.is_ascii_digit())) && l.contains(") "))?;
    let fields: Vec<&str> = line[line.rfind(") ")? + 2..].split_whitespace().collect();
    // After the name: state is field 3, so utime (14) is index 11.
    let number = |i: usize| fields.get(i).and_then(|v| v.parse::<u64>().ok());
    Some(CpuTicks { machine, process: number(11)? + number(12)?, threads: number(17)? as u32 })
}

/// The app's share of the whole CPU between two readings, in percent.
pub fn cpu_percent(before: &CpuTicks, after: &CpuTicks) -> f32 {
    let machine = after.machine.saturating_sub(before.machine);
    if machine == 0 {
        return 0.;
    }
    (after.process.saturating_sub(before.process) as f32 * 100. / machine as f32).clamp(0., 100.)
}

/// A process the Profiler can pick: `ps -A -o PID,NAME`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfiledProcess {
    pub pid: u32,
    pub name: String,
}

/// The app processes (`com.example.app`, `com.example.app:remote`).
pub fn parse_processes(text: &str) -> Vec<ProfiledProcess> {
    let mut out: Vec<ProfiledProcess> = text
        .lines()
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            let pid = parts.next()?.parse().ok()?;
            let name = parts.next()?.to_owned();
            (name.contains('.') && !name.starts_with('/') && !name.starts_with('[')).then_some(ProfiledProcess { pid, name })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.pid.cmp(&b.pid)));
    out
}

/// One point of the timelines.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sample {
    /// Seconds since the session started.
    pub at: f32,
    pub cpu: Option<f32>,
    pub threads: Option<u32>,
    pub memory: Option<Memory>,
}

/// Where the Profiler reads its numbers.
pub trait ProfilerSource {
    fn processes(&self, serial: &str) -> Result<Vec<ProfiledProcess>, String>;
    /// `cat /proc/stat /proc/<pid>/stat`.
    fn cpu(&self, serial: &str, pid: u32) -> Result<String, String>;
    /// `dumpsys meminfo <pid>`.
    fn meminfo(&self, serial: &str, pid: u32) -> Result<String, String>;
    /// Dumps the Java heap into the file `to` on the computer; its bytes.
    fn heap_dump(&self, serial: &str, pid: u32, to: &std::path::Path) -> Result<Vec<u8>, String>;
    /// `am profile start`: samples the app's stacks every `sampling_us`,
    /// or records every method enter and exit when `None`.
    fn start_method_trace(&self, serial: &str, pid: u32, sampling_us: Option<u32>) -> Result<(), String>;
    /// `am profile stop`; the trace pulled into the file `to`, its bytes.
    fn stop_method_trace(&self, serial: &str, pid: u32, to: &std::path::Path) -> Result<Vec<u8>, String>;
    /// Starts Perfetto recording the scheduler, CPU frequencies and the
    /// `package`'s atrace sections; the recording's id.
    fn start_system_trace(&self, serial: &str, package: &str) -> Result<u32, String>;
    /// Stops the recording `id`; the trace pulled into the file `to`, its bytes.
    fn stop_system_trace(&self, serial: &str, id: u32, to: &std::path::Path) -> Result<Vec<u8>, String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEMINFO: &str = "Applications Memory Usage (in Kilobytes):
Uptime: 4479648 Realtime: 4479648

** MEMINFO in pid 12345 [com.example.app] **
                   Pss  Private  Private  SwapPss      Rss     Heap     Heap     Heap
                 Total    Dirty    Clean    Dirty    Total     Size    Alloc     Free
 App Summary
                       Pss(KB)                        Rss(KB)
                        ------                         ------
           Java Heap:     5300                          17388
         Native Heap:     2520                           4044
                Code:     4688                          33856
               Stack:      564                            572
            Graphics:     1132                           1132
       Private Other:      612
              System:     2690
             Unknown:                                    1724

           TOTAL PSS:    17506            TOTAL RSS:    58716      TOTAL SWAP PSS:       85
";

    #[test]
    fn meminfo_summary() {
        let m = parse_meminfo(MEMINFO).unwrap();
        assert_eq!(m, Memory { java: 5300, native: 2520, graphics: 1132, stack: 564, code: 4688, others: 3302, total: 17506 });
        assert!(parse_meminfo("No process found for: 1").is_none());
    }

    #[test]
    fn cpu_share() {
        let a = parse_cpu("cpu  100 0 100 800 0 0 0 0 0 0\ncpu0 1 2 3\n12345 (com.example app) S 1 2 3 4 5 6 7 8 9 10 30 20 0 0 20 0 17 0\n").unwrap();
        assert_eq!(a, CpuTicks { machine: 1000, process: 50, threads: 17 });
        let b = CpuTicks { machine: 1400, process: 90, threads: 17 };
        assert_eq!(cpu_percent(&a, &b), 10.);
    }

    #[test]
    fn app_processes() {
        let ps = "  PID NAME\n    1 init\n  612 /system/bin/surfaceflinger\n 9001 com.example.app\n 9002 com.example.app:remote\n 700 [kworker/0:1]\n";
        let names: Vec<String> = parse_processes(ps).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["com.example.app", "com.example.app:remote"]);
    }
}
