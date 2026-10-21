//! A Perfetto system trace (Android Studio's "System Trace" recording) read
//! into what the Profiler shows: each CPU's scheduled threads and clock
//! frequency, and the app's threads with their states (Running, Runnable,
//! Sleeping, Waiting on I/O) and their `android.os.Trace` sections.
//!
//! The file is a protobuf `Trace`: `TracePacket`s carrying ftrace event
//! bundles (`sched_switch`, `sched_waking`, `task_newtask`, `task_rename`,
//! `cpu_frequency`, and `print` for atrace's `B|pid|name` / `E|pid`) and
//! the process tree scanned when recording started.

use std::collections::HashMap;

/// A protobuf field's value.
enum Value<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
    Fixed,
}

/// The fields of one protobuf message, in order; stops at the first
/// malformed one.
struct Fields<'a> {
    buf: &'a [u8],
    pos: usize,
}

fn fields(buf: &[u8]) -> Fields<'_> {
    Fields { buf, pos: 0 }
}

impl<'a> Fields<'a> {
    fn varint(&mut self) -> Option<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = *self.buf.get(self.pos)?;
            self.pos += 1;
            value |= ((byte & 0x7f) as u64) << shift;
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }
}

impl<'a> Iterator for Fields<'a> {
    type Item = (u32, Value<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.buf.len() {
            return None;
        }
        let key = self.varint()?;
        let value = match key & 7 {
            0 => Value::Varint(self.varint()?),
            1 | 5 => {
                let n = if key & 7 == 1 { 8 } else { 4 };
                if self.pos + n > self.buf.len() {
                    self.pos = self.buf.len();
                    return None;
                }
                self.pos += n;
                Value::Fixed
            }
            2 => {
                let len = self.varint()? as usize;
                let end = self.pos.checked_add(len).filter(|&e| e <= self.buf.len());
                let Some(end) = end else {
                    self.pos = self.buf.len();
                    return None;
                };
                let bytes = &self.buf[self.pos..end];
                self.pos = end;
                Value::Bytes(bytes)
            }
            _ => {
                self.pos = self.buf.len();
                return None;
            }
        };
        Some(((key >> 3) as u32, value))
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A thread's state, as `sched_switch` leaves it and a wakeup changes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadState {
    Running,
    Runnable,
    Sleeping,
    /// Uninterruptible sleep: usually waiting on I/O.
    Waiting,
    Dead,
}

impl ThreadState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::Runnable => "Runnable",
            Self::Sleeping => "Sleeping",
            Self::Waiting => "Waiting (uninterruptible)",
            Self::Dead => "Dead",
        }
    }

    /// The kernel's `prev_state` of a switched-out thread.
    fn from_prev_state(state: i64) -> Self {
        let low = state & 0xff;
        if low == 0 {
            Self::Runnable
        } else if low & 0x02 != 0 {
            Self::Waiting
        } else if low & 0x01 != 0 {
            Self::Sleeping
        } else if low & (0x10 | 0x20) != 0 {
            Self::Dead
        } else {
            // Stopped, traced, parked or idle.
            Self::Sleeping
        }
    }
}

/// Times are nanoseconds on the trace's (boot) clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateSpan {
    pub start: u64,
    pub end: u64,
    pub state: ThreadState,
    /// The CPU while Running.
    pub cpu: Option<u32>,
}

/// An `android.os.Trace` section (`Trace.beginSection` / `endSection`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub start: u64,
    pub end: u64,
    pub depth: u32,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TraceThread {
    pub tid: u32,
    pub name: String,
    pub states: Vec<StateSpan>,
    pub sections: Vec<Section>,
}

impl TraceThread {
    /// Nanoseconds in each state between `from` and `to`.
    pub fn time_in(&self, from: u64, to: u64) -> Vec<(ThreadState, u64)> {
        let mut out: Vec<(ThreadState, u64)> = Vec::new();
        for s in &self.states {
            let d = s.end.min(to).saturating_sub(s.start.max(from));
            if d == 0 {
                continue;
            }
            match out.iter_mut().find(|(state, _)| *state == s.state) {
                Some((_, total)) => *total += d,
                None => out.push((s.state, d)),
            }
        }
        out.sort_by(|a, b| b.1.cmp(&a.1));
        out
    }
}

/// A thread on a CPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuSlice {
    pub start: u64,
    pub end: u64,
    pub tid: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cpu {
    pub slices: Vec<CpuSlice>,
    /// (from, kHz).
    pub frequency: Vec<(u64, u32)>,
}

impl Cpu {
    pub fn frequency_at(&self, at: u64) -> Option<u32> {
        let i = self.frequency.partition_point(|&(t, _)| t <= at);
        i.checked_sub(1).map(|i| self.frequency[i].1)
    }
}

#[derive(Clone, Debug, Default)]
pub struct SystemTrace {
    pub start: u64,
    pub end: u64,
    pub cpus: Vec<Cpu>,
    /// The recorded app's process.
    pub pid: u32,
    pub process: String,
    /// The app's threads: the main thread first, then by time on a CPU.
    pub threads: Vec<TraceThread>,
    /// Every thread's name, for the CPU lanes.
    pub names: HashMap<u32, String>,
    /// Every thread's process.
    pub tgids: HashMap<u32, u32>,
}

impl SystemTrace {
    pub fn is_app_thread(&self, tid: u32) -> bool {
        self.tgids.get(&tid) == Some(&self.pid)
    }

    /// `name (tid)`, or the process's name for another process's thread.
    pub fn thread_label(&self, tid: u32) -> String {
        let name = self.names.get(&tid).map(String::as_str).unwrap_or("<unknown>");
        match self.tgids.get(&tid) {
            Some(&pid) if pid != self.pid && pid != tid => match self.names.get(&pid) {
                Some(process) => format!("{name} ({tid}, {process})"),
                None => format!("{name} ({tid})"),
            },
            _ => format!("{name} ({tid})"),
        }
    }
}

/// An ftrace event the reading needs, with its time.
enum Event {
    Switch { cpu: u32, prev: u32, prev_state: i64, next: u32 },
    Waking { tid: u32 },
    NewTask { tid: u32 },
    Print { tid: u32, buf: String },
    Frequency { cpu: u32, khz: u32 },
}

/// `CLONE_THREAD`: the new task is a thread of the creator's process.
const CLONE_THREAD: u64 = 0x10000;

/// What the first pass learns: each thread's process and name.
#[derive(Default)]
struct Tasks {
    tgids: HashMap<u32, u32>,
    names: HashMap<u32, String>,
}

/// Reads the trace for the app process `pid`; `None` if it isn't a trace.
pub fn parse(bytes: &[u8], pid: u32) -> Option<SystemTrace> {
    let mut tasks = Tasks::default();
    let mut events: Vec<(u64, Event)> = Vec::new();
    let mut packets = 0;
    for (field, value) in fields(bytes) {
        let (1, Value::Bytes(packet)) = (field, value) else { return None };
        packets += 1;
        for (field, value) in fields(packet) {
            match (field, value) {
                (1, Value::Bytes(bundle)) => read_bundle(bundle, &mut tasks, &mut events),
                (2, Value::Bytes(tree)) => read_process_tree(tree, &mut tasks),
                _ => {}
            }
        }
    }
    if packets == 0 {
        return None;
    }
    // Bundles are per CPU: put them in one order.
    events.sort_by_key(|(at, _)| *at);
    // Threads that started during the recording.
    for (_, event) in &events {
        if let Event::Print { tid, buf } = event {
            // `B|<tgid>|…`: the writer's process.
            if let Some(tgid) = buf.split('|').nth(1).and_then(|p| p.trim().parse::<u32>().ok()) {
                tasks.tgids.entry(*tid).or_insert(tgid);
            }
        }
    }
    Some(build(events, tasks, pid))
}

fn read_process_tree(tree: &[u8], tasks: &mut Tasks) {
    for (field, value) in fields(tree) {
        let Value::Bytes(entry) = value else { continue };
        let (mut id, mut tgid, mut name) = (None, None, None);
        for (f, v) in fields(entry) {
            match (field, f, v) {
                // Process: pid 1, cmdline 3 (the first one is its name).
                (1, 1, Value::Varint(p)) => id = Some(p as u32),
                (1, 3, Value::Bytes(b)) if name.is_none() => name = Some(text(b)),
                // Thread: tid 1, name 2, tgid 3.
                (2, 1, Value::Varint(t)) => id = Some(t as u32),
                (2, 2, Value::Bytes(b)) => name = Some(text(b)),
                (2, 3, Value::Varint(g)) => tgid = Some(g as u32),
                _ => {}
            }
        }
        let Some(id) = id else { continue };
        let tgid = if field == 1 { id } else { tgid.unwrap_or(id) };
        tasks.tgids.insert(id, tgid);
        if let Some(name) = name.filter(|n| !n.is_empty()) {
            // A process's cmdline names its main thread unless the thread says otherwise.
            if field == 2 || !tasks.names.contains_key(&id) {
                let name = if field == 1 { name.rsplit('/').next().unwrap_or(&name).to_owned() } else { name };
                tasks.names.insert(id, name);
            }
        }
    }
}

fn read_bundle(bundle: &[u8], tasks: &mut Tasks, events: &mut Vec<(u64, Event)>) {
    let mut cpu = 0;
    let mut raw = Vec::new();
    for (field, value) in fields(bundle) {
        match (field, value) {
            (1, Value::Varint(c)) => cpu = c as u32,
            (2, Value::Bytes(event)) => raw.push(event),
            _ => {}
        }
    }
    for event in raw {
        let (mut at, mut tid) = (0u64, 0u32);
        let mut body = None;
        for (field, value) in fields(event) {
            match (field, value) {
                (1, Value::Varint(t)) => at = t,
                (2, Value::Varint(p)) => tid = p as u32,
                (3 | 4 | 11 | 20 | 235 | 236, Value::Bytes(b)) => body = Some((field, b)),
                _ => {}
            }
        }
        let Some((kind, body)) = body else { continue };
        let mut ints: HashMap<u32, u64> = HashMap::new();
        let mut strings: HashMap<u32, &[u8]> = HashMap::new();
        for (field, value) in fields(body) {
            match value {
                Value::Varint(v) => {
                    ints.insert(field, v);
                }
                Value::Bytes(b) => {
                    strings.insert(field, b);
                }
                Value::Fixed => {}
            }
        }
        let int = |f: u32| ints.get(&f).copied().unwrap_or(0);
        let mut name = |id: u32, f: u32| {
            if let Some(b) = strings.get(&f).filter(|b| !b.is_empty()) {
                tasks.names.entry(id).or_insert_with(|| text(b));
            }
        };
        let event = match kind {
            // PrintFtraceEvent: buf 2.
            3 => Event::Print { tid, buf: strings.get(&2).map(|b| text(b)).unwrap_or_default() },
            // SchedSwitch: prev_comm 1, prev_pid 2, prev_state 4, next_comm 5, next_pid 6.
            4 => {
                let (prev, next) = (int(2) as u32, int(6) as u32);
                name(prev, 1);
                name(next, 5);
                Event::Switch { cpu, prev, prev_state: int(4) as i64, next }
            }
            // CpuFrequency: state 1 (kHz), cpu_id 2.
            11 => Event::Frequency { cpu: int(2) as u32, khz: int(1) as u32 },
            // SchedWaking: comm 1, pid 2.
            20 => {
                name(int(2) as u32, 1);
                Event::Waking { tid: int(2) as u32 }
            }
            // TaskNewtask: pid 1, comm 2, clone_flags 3; the creator is the event's pid.
            235 => {
                let new = int(1) as u32;
                let tgid = if int(3) & CLONE_THREAD != 0 { tasks.tgids.get(&tid).copied().unwrap_or(tid) } else { new };
                tasks.tgids.insert(new, tgid);
                if let Some(b) = strings.get(&2) {
                    tasks.names.insert(new, text(b));
                }
                Event::NewTask { tid: new }
            }
            // TaskRename: pid 1, newcomm 3.
            236 => {
                if let Some(b) = strings.get(&3) {
                    tasks.names.insert(int(1) as u32, text(b));
                }
                continue;
            }
            _ => continue,
        };
        events.push((at, event));
    }
}

/// A thread being followed through the events.
#[derive(Default)]
struct Following {
    thread: TraceThread,
    state: Option<(ThreadState, u64, Option<u32>)>,
    open: Vec<(u64, String)>,
}

impl Following {
    fn set(&mut self, state: ThreadState, at: u64, cpu: Option<u32>) {
        if let Some((s, since, c)) = self.state {
            if (s, c) == (state, cpu) {
                return;
            }
            if at > since {
                self.thread.states.push(StateSpan { start: since, end: at, state: s, cpu: c });
            }
        }
        self.state = Some((state, at, cpu));
    }
}

fn build(events: Vec<(u64, Event)>, tasks: Tasks, pid: u32) -> SystemTrace {
    let start = events.first().map(|e| e.0).unwrap_or(0);
    let end = events.last().map(|e| e.0).unwrap_or(0);
    let mut cpus: Vec<Cpu> = Vec::new();
    let mut on_cpu: Vec<Option<(u32, u64)>> = Vec::new();
    let mut followed: HashMap<u32, Following> = HashMap::new();
    let ours = |tid: u32| tid != 0 && tasks.tgids.get(&tid) == Some(&pid);
    for (at, event) in events {
        match event {
            Event::Switch { cpu, prev, prev_state, next } => {
                let c = cpu as usize;
                if cpus.len() <= c {
                    cpus.resize_with(c + 1, Cpu::default);
                    on_cpu.resize(c + 1, None);
                }
                if let Some((tid, since)) = on_cpu[c].take() {
                    cpus[c].slices.push(CpuSlice { start: since, end: at, tid });
                }
                if prev != 0 && ours(prev) {
                    followed.entry(prev).or_default().set(ThreadState::from_prev_state(prev_state), at, None);
                }
                if next != 0 {
                    on_cpu[c] = Some((next, at));
                    if ours(next) {
                        followed.entry(next).or_default().set(ThreadState::Running, at, Some(cpu));
                    }
                }
            }
            Event::Waking { tid } | Event::NewTask { tid } if ours(tid) => {
                let f = followed.entry(tid).or_default();
                if !matches!(f.state, Some((ThreadState::Running | ThreadState::Runnable, _, _))) {
                    f.set(ThreadState::Runnable, at, None);
                }
            }
            Event::Print { tid, buf } if ours(tid) => {
                let f = followed.entry(tid).or_default();
                for line in buf.lines() {
                    let mut parts = line.splitn(3, '|');
                    match (parts.next(), parts.next(), parts.next()) {
                        (Some("B"), _, Some(name)) => f.open.push((at, name.trim_end().to_owned())),
                        (Some("E"), _, _) => {
                            if let Some((begin, name)) = f.open.pop() {
                                f.thread.sections.push(Section { start: begin, end: at, depth: f.open.len() as u32, name });
                            }
                        }
                        _ => {}
                    }
                }
            }
            Event::Frequency { cpu, khz } => {
                let c = cpu as usize;
                if cpus.len() <= c {
                    cpus.resize_with(c + 1, Cpu::default);
                    on_cpu.resize(c + 1, None);
                }
                cpus[c].frequency.push((at, khz));
            }
            _ => {}
        }
    }
    // Close what is still open at the end.
    for (c, running) in on_cpu.into_iter().enumerate() {
        if let Some((tid, since)) = running {
            cpus[c].slices.push(CpuSlice { start: since, end, tid });
        }
    }
    let mut threads: Vec<TraceThread> = followed
        .into_iter()
        .map(|(tid, mut f)| {
            if let Some((state, since, cpu)) = f.state.take() {
                if end > since {
                    f.thread.states.push(StateSpan { start: since, end, state, cpu });
                }
            }
            while let Some((begin, name)) = f.open.pop() {
                f.thread.sections.push(Section { start: begin, end, depth: f.open.len() as u32, name });
            }
            f.thread.sections.sort_by_key(|s| (s.start, s.depth));
            f.thread.tid = tid;
            f.thread.name = tasks.names.get(&tid).cloned().unwrap_or_default();
            f.thread
        })
        .collect();
    let running = |t: &TraceThread| t.states.iter().filter(|s| s.state == ThreadState::Running).map(|s| s.end - s.start).sum::<u64>();
    threads.sort_by_cached_key(|t| (t.tid != pid, std::cmp::Reverse(running(t)), t.tid));
    SystemTrace { start, end, cpus, pid, process: tasks.names.get(&pid).cloned().unwrap_or_default(), threads, names: tasks.names, tgids: tasks.tgids }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A tiny protobuf writer for building traces.
    #[derive(Default)]
    pub(crate) struct Message(pub Vec<u8>);

    impl Message {
        fn key(&mut self, field: u32, wire: u32) {
            self.varint_raw(((field << 3) | wire) as u64);
        }

        fn varint_raw(&mut self, mut v: u64) {
            loop {
                let byte = (v & 0x7f) as u8;
                v >>= 7;
                if v == 0 {
                    self.0.push(byte);
                    return;
                }
                self.0.push(byte | 0x80);
            }
        }

        pub fn int(mut self, field: u32, v: u64) -> Self {
            self.key(field, 0);
            self.varint_raw(v);
            self
        }

        pub fn bytes(mut self, field: u32, b: &[u8]) -> Self {
            self.key(field, 2);
            self.varint_raw(b.len() as u64);
            self.0.extend_from_slice(b);
            self
        }

        pub fn str(self, field: u32, s: &str) -> Self {
            self.bytes(field, s.as_bytes())
        }

        pub fn msg(self, field: u32, m: Message) -> Self {
            self.bytes(field, &m.0)
        }
    }

    fn event(at: u64, pid: u64, kind: u32, body: Message) -> Message {
        Message::default().int(1, at).int(2, pid).msg(kind, body)
    }

    fn switch(at: u64, prev: (&str, u64, u64), next: (&str, u64)) -> Message {
        event(at, prev.1, 4, Message::default().str(1, prev.0).int(2, prev.1).int(4, prev.2).str(5, next.0).int(6, next.1))
    }

    fn print(at: u64, tid: u64, buf: &str) -> Message {
        event(at, tid, 3, Message::default().int(1, 0).str(2, buf))
    }

    /// The app 100 (main thread 100, RenderThread 101, a worker 102 created
    /// during the recording) and system_server's 200, on two CPUs.
    pub(crate) fn sample() -> Vec<u8> {
        let tree = Message::default()
            .msg(1, Message::default().int(1, 100).int(2, 1).str(3, "com.example.app"))
            .msg(1, Message::default().int(1, 200).int(2, 1).str(3, "system_server"))
            .msg(2, Message::default().int(1, 101).str(2, "RenderThread").int(3, 100));
        let ms = 1_000_000u64;
        let cpu0 = Message::default()
            .int(1, 0)
            .msg(2, switch(10 * ms, ("swapper/0", 0, 0), ("com.example.app", 100)))
            .msg(2, print(11 * ms, 100, "B|100|Choreographer#doFrame"))
            .msg(2, print(12 * ms, 100, "B|100|inflate"))
            .msg(2, event(13 * ms, 100, 235, Message::default().int(1, 102).str(2, "worker").int(3, CLONE_THREAD)))
            .msg(2, print(14 * ms, 100, "E|100"))
            .msg(2, print(15 * ms, 100, "E|100"))
            .msg(2, switch(16 * ms, ("com.example.app", 100, 1), ("system_server", 200)))
            .msg(2, event(17 * ms, 101, 20, Message::default().str(1, "com.example.app").int(2, 100).int(5, 0)))
            .msg(2, switch(18 * ms, ("system_server", 200, 0), ("com.example.app", 100)))
            .msg(2, event(19 * ms, 0, 11, Message::default().int(1, 1_800_000).int(2, 0)));
        let cpu1 = Message::default()
            .int(1, 1)
            .msg(2, event(9 * ms, 0, 11, Message::default().int(1, 300_000).int(2, 1)))
            .msg(2, switch(12 * ms, ("swapper/1", 0, 0), ("RenderThread", 101)))
            .msg(2, print(12 * ms + 1, 101, "B|100|DrawFrame"))
            .msg(2, switch(14 * ms, ("RenderThread", 101, 2), ("worker", 102)))
            .msg(2, switch(20 * ms, ("worker", 102, 0), ("swapper/1", 0)));
        let packets = Message::default()
            .msg(1, Message::default().msg(2, tree).int(8, 9 * ms))
            .msg(1, Message::default().msg(1, cpu1).int(10, 1))
            .msg(1, Message::default().msg(1, cpu0).int(10, 1));
        packets.0
    }

    #[test]
    fn reads_cpus_threads_and_sections() {
        let ms = 1_000_000u64;
        let trace = parse(&sample(), 100).unwrap();
        assert_eq!((trace.start, trace.end, trace.process.as_str()), (9 * ms, 20 * ms, "com.example.app"));
        assert_eq!(trace.cpus.len(), 2);
        let lane0: Vec<(u64, u64, u32)> = trace.cpus[0].slices.iter().map(|s| (s.start / ms, s.end / ms, s.tid)).collect();
        assert_eq!(lane0, [(10, 16, 100), (16, 18, 200), (18, 20, 100)]);
        let lane1: Vec<(u64, u64, u32)> = trace.cpus[1].slices.iter().map(|s| (s.start / ms, s.end / ms, s.tid)).collect();
        assert_eq!(lane1, [(12, 14, 101), (14, 20, 102)]);
        assert_eq!((trace.cpus[1].frequency_at(15 * ms), trace.cpus[0].frequency_at(15 * ms)), (Some(300_000), None));
        // Main thread first, then by time running; system_server isn't ours.
        let names: Vec<&str> = trace.threads.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["com.example.app", "worker", "RenderThread"]);
        let main = &trace.threads[0];
        let states: Vec<(u64, u64, ThreadState)> = main.states.iter().map(|s| (s.start / ms, s.end / ms, s.state)).collect();
        assert_eq!(states, [(10, 16, ThreadState::Running), (16, 17, ThreadState::Sleeping), (17, 18, ThreadState::Runnable), (18, 20, ThreadState::Running)]);
        let sections: Vec<(&str, u32, u64, u64)> = main.sections.iter().map(|s| (s.name.as_str(), s.depth, s.start / ms, s.end / ms)).collect();
        assert_eq!(sections, [("Choreographer#doFrame", 0, 11, 15), ("inflate", 1, 12, 14)]);
        let render = &trace.threads[2];
        assert_eq!(render.states.last().map(|s| s.state), Some(ThreadState::Waiting));
        // An open section ends with the trace.
        assert_eq!(render.sections[0].end, 20 * ms);
        let worker = &trace.threads[1];
        assert_eq!(worker.states.first().map(|s| (s.start / ms, s.state)), Some((13, ThreadState::Runnable)));
        assert_eq!(main.time_in(0, u64::MAX)[0], (ThreadState::Running, 8 * ms));
        assert_eq!(trace.thread_label(200), "system_server (200)");
        assert!(trace.is_app_thread(102) && !trace.is_app_thread(200));
    }

    #[test]
    fn rejects_other_files() {
        assert!(parse(b"*version\n3\n", 1).is_none());
        assert!(parse(&[], 1).is_none());
    }
}
