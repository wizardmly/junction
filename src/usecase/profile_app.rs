//! The Profiler's session: a sample of the app's CPU and memory per tick,
//! and Capture heap dump read into its class list.

use std::path::Path;

use crate::domain::hprof::{self, HeapSummary};
use crate::domain::profiler::{self, CpuTicks, ProfilerSource, Sample};

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
    }
}
