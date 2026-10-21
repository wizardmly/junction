//! The Profiler's session: a sample of the app's CPU and memory per tick,
//! Capture heap dump read into its class list, and Record's method trace
//! read into call trees, or its System Trace read into CPU and thread
//! timelines.

use std::path::Path;

use crate::domain::hprof::{self, HeapSummary};
use crate::domain::method_trace::{self, MethodTrace};
use crate::domain::profiler::{self, CpuTicks, ProfilerSource, Sample};
use crate::domain::system_trace::{self, SystemTrace};

#[derive(Default)]
pub struct Sampler {
    last: Option<CpuTicks>,
}

impl Sampler {
    /// The next point; the CPU share needs the reading before it, so the
    /// first point has none.
    pub fn sample(&mut self, source: &dyn ProfilerSource, serial: &str, pid: u32, at: f32) -> Sample {
        let ticks = source.cpu(serial, pid).ok().and_then(|t| profiler::parse_cpu(&t));
        let cpu = match (&self.last, &ticks) {
            (Some(before), Some(now)) => Some(profiler::cpu_percent(before, now)),
            _ => None,
        };
        self.last = ticks;
        let memory = source.meminfo(serial, pid).ok().and_then(|t| profiler::parse_meminfo(&t));
        Sample { at, cpu, threads: ticks.map(|t| t.threads), memory }
    }
}

/// Dumps the heap to `to` and reads its classes.
pub fn capture_heap_dump(source: &dyn ProfilerSource, serial: &str, pid: u32, to: &Path) -> Result<HeapSummary, String> {
    let bytes = source.heap_dump(serial, pid, to)?;
    hprof::summarize(&bytes).ok_or_else(|| "The heap dump can't be read".to_owned())
}

/// How Record captures the app's methods, as Android Studio's CPU
/// recording configurations name them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceMode {
    /// Java/Kotlin Method Sample: the stacks every millisecond.
    Sample,
    /// Java/Kotlin Method Trace: every call (slows the app down).
    Trace,
    /// System Trace: Perfetto's view of the CPUs and the app's threads.
    System,
}

impl TraceMode {
    pub fn label(self) -> &'static str {
        match self {
            TraceMode::Sample => "Java/Kotlin Method Sample",
            TraceMode::Trace => "Java/Kotlin Method Trace",
            TraceMode::System => "System Trace",
        }
    }
}

pub fn start_method_trace(source: &dyn ProfilerSource, serial: &str, pid: u32, mode: TraceMode) -> Result<(), String> {
    source.start_method_trace(serial, pid, (mode == TraceMode::Sample).then_some(1000))
}

/// Stops the recording, saves it to `to` and reads its call trees.
pub fn stop_method_trace(source: &dyn ProfilerSource, serial: &str, pid: u32, to: &Path) -> Result<MethodTrace, String> {
    let bytes = source.stop_method_trace(serial, pid, to)?;
    method_trace::parse(&bytes).ok_or_else(|| "The method trace can't be read".to_owned())
}

/// Starts a System Trace of the process `process` (`com.example.app` or
/// `com.example.app:remote`); the recording's id.
pub fn start_system_trace(source: &dyn ProfilerSource, serial: &str, process: &str) -> Result<u32, String> {
    let package = process.split(':').next().unwrap_or(process);
    source.start_system_trace(serial, package)
}

/// Stops the recording, saves it to `to` and reads it for the process `pid`.
pub fn stop_system_trace(source: &dyn ProfilerSource, serial: &str, id: u32, pid: u32, to: &Path) -> Result<SystemTrace, String> {
    let bytes = source.stop_system_trace(serial, id, to)?;
    system_trace::parse(&bytes, pid).ok_or_else(|| "The system trace can't be read".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::profiler::ProfiledProcess;
    use std::cell::Cell;

    struct Fake(Cell<u64>);

    impl ProfilerSource for Fake {
        fn processes(&self, _: &str) -> Result<Vec<ProfiledProcess>, String> {
            Ok(Vec::new())
        }

        fn cpu(&self, _: &str, _: u32) -> Result<String, String> {
            let n = self.0.get();
            self.0.set(n + 1);
            Ok(format!("cpu  {} 0 0 0\n42 (app) S 1 2 3 4 5 6 7 8 9 10 {} 0 0 0 20 0 9 0\n", 1000 * (n + 1), 100 * (n + 1)))
        }

        fn meminfo(&self, _: &str, _: u32) -> Result<String, String> {
            Ok(" App Summary\n Java Heap: 10\n TOTAL PSS: 30\n".into())
        }

        fn heap_dump(&self, _: &str, _: u32, _: &Path) -> Result<Vec<u8>, String> {
            Ok(crate::domain::hprof::tests::sample())
        }

        fn start_method_trace(&self, _: &str, _: u32, sampling_us: Option<u32>) -> Result<(), String> {
            assert_eq!(sampling_us, Some(1000));
            Ok(())
        }

        fn stop_method_trace(&self, _: &str, _: u32, _: &Path) -> Result<Vec<u8>, String> {
            Ok(crate::domain::method_trace::tests::sample())
        }

        fn start_system_trace(&self, _: &str, package: &str) -> Result<u32, String> {
            assert_eq!(package, "com.example.app");
            Ok(7)
        }

        fn stop_system_trace(&self, _: &str, id: u32, _: &Path) -> Result<Vec<u8>, String> {
            assert_eq!(id, 7);
            Ok(crate::domain::system_trace::tests::sample())
        }
    }

    #[test]
    fn samples_and_heap_dump() {
        let fake = Fake(Cell::new(0));
        let mut sampler = Sampler::default();
        let first = sampler.sample(&fake, "s", 42, 0.);
        assert_eq!((first.cpu, first.threads), (None, Some(9)));
        let second = sampler.sample(&fake, "s", 42, 1.);
        assert_eq!(second.cpu, Some(10.));
        assert_eq!(second.memory.unwrap().total, 30);
        let heap = capture_heap_dump(&fake, "s", 42, Path::new("x.hprof")).unwrap();
        assert_eq!(heap.classes[0].name, "java.lang.String");
        start_method_trace(&fake, "s", 42, TraceMode::Sample).unwrap();
        let trace = stop_method_trace(&fake, "s", 42, Path::new("x.trace")).unwrap();
        assert_eq!(trace.threads[0].name, "main");
        let id = start_system_trace(&fake, "s", "com.example.app:remote").unwrap();
        let system = stop_system_trace(&fake, "s", id, 100, Path::new("x.pftrace")).unwrap();
        assert_eq!(system.threads[0].name, "com.example.app");
    }
}
