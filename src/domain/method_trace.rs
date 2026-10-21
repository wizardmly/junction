//! An ART method trace (the `.trace` file `am profile` writes, Android
//! Studio's "Java/Kotlin Method Sample" and "Method Trace") read into a
//! call tree per thread: Top Down, its Bottom Up inversion and the Flame
//! Chart all come from it.
//!
//! The file is a text header (`*version`, `*threads`, `*methods`, `*end`)
//! followed by a binary `SLOW` block of enter / exit records with each
//! thread's CPU time and the wall time, in microseconds from the start.

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Method {
    pub class: String,
    pub name: String,
    pub signature: String,
    pub file: String,
}

impl Method {
    /// As Android Studio lists it: `onCreate (com.example.MainActivity)`.
    pub fn label(&self) -> String {
        if self.class.is_empty() { self.name.clone() } else { format!("{} ({})", self.name, self.class) }
    }
}

/// Which time a figure is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clock {
    Wall = 0,
    Thread = 1,
}

/// A call tree's node: one method under one call path. Times are
/// `[wall, thread]` microseconds.
#[derive(Clone, Debug, Default)]
pub struct Node {
    /// Index into `MethodTrace::methods`; the root has `ROOT`.
    pub method: u32,
    pub parent: u32,
    pub children: Vec<u32>,
    pub total: [u64; 2],
    pub own: [u64; 2],
    pub calls: u32,
}

pub const ROOT: u32 = u32::MAX;

/// Node 0 is the root, which stands for the whole thread.
#[derive(Clone, Debug, Default)]
pub struct CallTree {
    pub nodes: Vec<Node>,
}

impl CallTree {
    fn new() -> Self {
        Self { nodes: vec![Node { method: ROOT, parent: ROOT, ..Node::default() }] }
    }

    pub fn total(&self, clock: Clock) -> u64 {
        self.nodes[0].total[clock as usize]
    }

    /// A node's children, longest first.
    pub fn sorted_children(&self, node: u32, clock: Clock) -> Vec<u32> {
        let mut children = self.nodes[node as usize].children.clone();
        children.sort_by_key(|&c| std::cmp::Reverse(self.nodes[c as usize].total[clock as usize]));
        children
    }

    /// The nodes from the root down the longest child each time: the path
    /// Android Studio opens.
    pub fn heaviest_path(&self, clock: Clock) -> Vec<u32> {
        let mut path = vec![0];
        while let Some(&next) = self.sorted_children(*path.last().unwrap(), clock).first() {
            path.push(next);
        }
        path
    }

    fn child(&mut self, index: &mut HashMap<(u32, u32), u32>, parent: u32, method: u32) -> u32 {
        *index.entry((parent, method)).or_insert_with(|| {
            let id = self.nodes.len() as u32;
            self.nodes.push(Node { method, parent, ..Node::default() });
            self.nodes[parent as usize].children.push(id);
            id
        })
    }

    /// Bottom Up: each method at the top with all the time spent in it,
    /// and under it the callers it was reached through. A recursive call
    /// inside the same method isn't counted again in its total.
    pub fn bottom_up(&self) -> CallTree {
        let mut out = CallTree::new();
        let mut index = HashMap::new();
        for (id, node) in self.nodes.iter().enumerate().skip(1) {
            let mut nested = false;
            let mut up = node.parent;
            while up != 0 {
                nested |= self.nodes[up as usize].method == node.method;
                up = self.nodes[up as usize].parent;
            }
            let mut at = id as u32;
            let mut parent = 0;
            while at != 0 {
                let target = out.child(&mut index, parent, self.nodes[at as usize].method);
                let n = &mut out.nodes[target as usize];
                for c in 0..2 {
                    n.own[c] += node.own[c];
                    if !nested {
                        n.total[c] += node.total[c];
                    }
                }
                n.calls += node.calls;
                parent = target;
                at = self.nodes[at as usize].parent;
            }
        }
        out.nodes[0].total = self.nodes[0].total;
        out
    }
}

#[derive(Clone, Debug)]
pub struct TraceThread {
    pub id: u32,
    pub name: String,
    pub tree: CallTree,
    /// Every call as it happened, for the Call Chart.
    pub calls: Vec<Call>,
    /// The first and last record's `[wall, thread]` times.
    pub span: [[u64; 2]; 2],
}

/// One call on the Call Chart: `depth` 0 is a thread's outermost method.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Call {
    pub method: u32,
    pub depth: u32,
    pub start: [u64; 2],
    pub end: [u64; 2],
}

#[derive(Clone, Debug, Default)]
pub struct MethodTrace {
    pub methods: Vec<Method>,
    /// Busiest first (by wall time).
    pub threads: Vec<TraceThread>,
    /// Which clocks the trace has; a single-clock trace repeats its one
    /// time in both slots.
    pub wall: bool,
    pub thread_time: bool,
    pub elapsed_us: u64,
}

impl MethodTrace {
    pub fn label(&self, method: u32) -> String {
        self.methods.get(method as usize).map(Method::label).unwrap_or_else(|| "[root]".into())
    }
}

const MAGIC: u32 = 0x574f_4c53; // "SLOW"

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn hex(s: &str) -> Option<u32> {
    let s = s.trim();
    u32::from_str_radix(s.strip_prefix("0x").unwrap_or(s), 16).ok()
}

/// One record: thread, method (its id from the header), whether it is an
/// exit, and `[wall, thread]` times.
struct Record {
    thread: u32,
    method: u32,
    exit: bool,
    time: [u64; 2],
}

/// `None` when `b` isn't a non-streaming ART trace.
pub fn parse(b: &[u8]) -> Option<MethodTrace> {
    const END: &[u8] = b"*end\n";
    let end = b.windows(END.len()).position(|w| w == END)?;
    let text = std::str::from_utf8(&b[..end]).ok()?;
    if !text.starts_with("*version") {
        return None;
    }
    let mut section = "";
    let mut clock = "";
    let mut elapsed_us = 0;
    let mut names = HashMap::new();
    let mut methods = Vec::new();
    let mut method_ids = HashMap::new();
    for line in text.lines() {
        if let Some(s) = line.strip_prefix('*') {
            section = s.trim();
            continue;
        }
        match section {
            "version" => match line.split_once('=') {
                Some(("clock", v)) => clock = v.trim(),
                Some(("elapsed-time-usec", v)) => elapsed_us = v.trim().parse().unwrap_or(0),
                _ => {}
            },
            "threads" => {
                if let Some((id, name)) = line.split_once('\t') {
                    if let Ok(id) = id.trim().parse::<u32>() {
                        names.insert(id, name.to_owned());
                    }
                }
            }
            "methods" => {
                let mut parts = line.split('\t');
                let Some(id) = parts.next().and_then(hex) else { continue };
                let mut next = || parts.next().unwrap_or_default().to_owned();
                let method = Method { class: next(), name: next(), signature: next(), file: next() };
                method_ids.insert(id & !3, methods.len() as u32);
                methods.push(method);
            }
            _ => {}
        }
    }
    let data = &b[end + END.len()..];
    if u32_at(data, 0)? != MAGIC {
        return None;
    }
    let version = u16_at(data, 4)?;
    let start = u16_at(data, 6)? as usize;
    let (wall, thread_time) = match clock {
        "dual" => (true, true),
        "thread-cpu" => (false, true),
        _ => (true, false),
    };
    let dual = wall && thread_time;
    let size = match version {
        1 => 9,
        2 => 10,
        _ => u16_at(data, 16)? as usize,
    };
    let tid_len = if version == 1 { 1 } else { 2 };
    if size < tid_len + 4 + if dual { 8 } else { 4 } {
        return None;
    }
    let mut records = Vec::new();
    let mut at = start;
    while at + size <= data.len() {
        let thread = if tid_len == 1 { data[at] as u32 } else { u16_at(data, at)? as u32 };
        let value = u32_at(data, at + tid_len)?;
        let t = at + tid_len + 4;
        let time = if dual {
            [u32_at(data, t + 4)? as u64, u32_at(data, t)? as u64]
        } else {
            let one = u32_at(data, t)? as u64;
            [one, one]
        };
        let id = value & !3;
        let method = match method_ids.get(&id) {
            Some(&m) => m,
            None => {
                let m = methods.len() as u32;
                methods.push(Method { class: String::new(), name: format!("unknown method 0x{id:x}"), signature: String::new(), file: String::new() });
                method_ids.insert(id, m);
                m
            }
        };
        records.push(Record { thread, method, exit: value & 3 != 0, time });
        at += size;
    }
    let mut order = Vec::new();
    let mut by_thread: HashMap<u32, Vec<Record>> = HashMap::new();
    for r in records {
        if !by_thread.contains_key(&r.thread) {
            order.push(r.thread);
        }
        by_thread.entry(r.thread).or_default().push(r);
    }
    let mut threads: Vec<TraceThread> = order
        .into_iter()
        .map(|id| {
            let records = &by_thread[&id];
            let (tree, calls) = build(records);
            let span = [records[0].time, records[records.len() - 1].time];
            TraceThread { id, name: names.get(&id).cloned().unwrap_or_else(|| format!("Thread {id}")), tree, calls, span }
        })
        .collect();
    threads.sort_by_key(|t| std::cmp::Reverse(t.tree.total(Clock::Wall)));
    if elapsed_us == 0 {
        elapsed_us = threads.iter().map(|t| t.tree.total(Clock::Wall)).max().unwrap_or(0);
    }
    Some(MethodTrace { methods, threads, wall, thread_time, elapsed_us })
}

/// One thread's records as a Top Down tree. Methods already running when
/// the trace started (exits without an enter) start at its first record;
/// methods still running at its end stop at its last.
fn build(records: &[Record]) -> (CallTree, Vec<Call>) {
    let mut tree = CallTree::new();
    let mut calls = Vec::new();
    let Some(first) = records.first() else { return (tree, calls) };
    let last = records.last().unwrap().time;
    let mut open = Vec::new();
    let mut unmatched = Vec::new();
    for r in records {
        if !r.exit {
            open.push(r.method);
        } else if let Some(i) = open.iter().rposition(|&m| m == r.method) {
            open.truncate(i);
        } else if open.is_empty() {
            unmatched.push(r.method);
        }
    }
    struct Frame {
        node: u32,
        start: [u64; 2],
        children: [u64; 2],
    }
    let mut index = HashMap::new();
    let mut stack: Vec<Frame> = Vec::new();
    let enter = |tree: &mut CallTree, index: &mut HashMap<(u32, u32), u32>, stack: &mut Vec<Frame>, method: u32, time: [u64; 2]| {
        let parent = stack.last().map_or(0, |f| f.node);
        let node = tree.child(index, parent, method);
        tree.nodes[node as usize].calls += 1;
        stack.push(Frame { node, start: time, children: [0; 2] });
    };
    let close = |tree: &mut CallTree, calls: &mut Vec<Call>, stack: &mut Vec<Frame>, time: [u64; 2]| {
        let frame = stack.pop().unwrap();
        calls.push(Call { method: tree.nodes[frame.node as usize].method, depth: stack.len() as u32, start: frame.start, end: time });
        let node = &mut tree.nodes[frame.node as usize];
        for c in 0..2 {
            let duration = time[c].saturating_sub(frame.start[c]);
            node.total[c] += duration;
            node.own[c] += duration.saturating_sub(frame.children[c]);
            if let Some(parent) = stack.last_mut() {
                parent.children[c] += duration;
            }
        }
    };
    for &method in unmatched.iter().rev() {
        enter(&mut tree, &mut index, &mut stack, method, first.time);
    }
    for r in records {
        if !r.exit {
            enter(&mut tree, &mut index, &mut stack, r.method, r.time);
        } else if let Some(i) = stack.iter().rposition(|f| tree.nodes[f.node as usize].method == r.method) {
            while stack.len() > i {
                close(&mut tree, &mut calls, &mut stack, r.time);
            }
        }
    }
    while !stack.is_empty() {
        close(&mut tree, &mut calls, &mut stack, last);
    }
    let roots = tree.nodes[0].children.clone();
    for c in 0..2 {
        tree.nodes[0].total[c] = roots.iter().map(|&r| tree.nodes[r as usize].total[c]).sum();
    }
    calls.sort_by_key(|c| (c.start[0], c.depth));
    (tree, calls)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A dual-clock trace of thread 1 "main": `run` (already running when
    /// the trace starts) calls `a` twice, `a` calls `b` once; thread 2
    /// "worker" runs `b` alone.
    pub fn sample() -> Vec<u8> {
        let mut out = String::from("*version\n3\ndata-file-overflow=false\nclock=dual\nelapsed-time-usec=100\nvm=art\n*threads\n1\tmain\n2\tworker\n*methods\n");
        for (id, class, name) in [(0x10, "com.example.Main", "run"), (0x14, "com.example.Main", "a"), (0x18, "com.example.Util", "b")] {
            out.push_str(&format!("0x{id:x}\t{class}\t{name}\t()V\tMain.java\n"));
        }
        out.push_str("*end\n");
        let mut b = out.into_bytes();
        let mut header = Vec::new();
        header.extend(MAGIC.to_le_bytes());
        header.extend(3u16.to_le_bytes());
        header.extend(32u16.to_le_bytes());
        header.extend(0u64.to_le_bytes());
        header.extend(14u16.to_le_bytes());
        header.resize(32, 0);
        b.extend(header);
        // (thread, method value, thread time, wall time)
        let records: [(u16, u32, u32, u32); 9] = [
            (1, 0x14, 0, 0),
            (1, 0x18, 5, 10),
            (1, 0x19, 10, 20),
            (2, 0x18, 0, 5),
            (1, 0x15, 15, 30),
            (1, 0x14, 20, 40),
            (1, 0x15, 25, 50),
            (2, 0x19, 30, 35),
            (1, 0x11, 30, 60),
        ];
        for (thread, method, cpu, wall) in records {
            b.extend(thread.to_le_bytes());
            b.extend(method.to_le_bytes());
            b.extend(cpu.to_le_bytes());
            b.extend(wall.to_le_bytes());
        }
        b
    }

    #[test]
    fn builds_top_down_per_thread() {
        let trace = parse(&sample()).unwrap();
        assert!(trace.wall && trace.thread_time);
        assert_eq!(trace.elapsed_us, 100);
        let names: Vec<&str> = trace.threads.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["main", "worker"]);
        let main = &trace.threads[0].tree;
        assert_eq!(main.total(Clock::Wall), 60);
        assert_eq!(main.total(Clock::Thread), 30);
        let run = &main.nodes[main.nodes[0].children[0] as usize];
        assert_eq!(trace.label(run.method), "run (com.example.Main)");
        assert_eq!((run.total, run.own, run.calls), ([60, 30], [20, 10], 1));
        let a = &main.nodes[run.children[0] as usize];
        assert_eq!((a.total, a.own, a.calls), ([40, 20], [30, 15], 2));
        let b = &main.nodes[a.children[0] as usize];
        assert_eq!((trace.label(b.method).as_str(), b.total), ("b (com.example.Util)", [10, 5]));
        let path: Vec<u32> = main.heaviest_path(Clock::Wall).iter().map(|&n| main.nodes[n as usize].method).collect();
        assert_eq!(path, [ROOT, 0, 1, 2]);
        assert_eq!(trace.threads[1].tree.total(Clock::Wall), 30);
        let calls: Vec<(u32, u32, u64, u64)> = trace.threads[0].calls.iter().map(|c| (c.method, c.depth, c.start[0], c.end[0])).collect();
        assert_eq!(calls, [(0, 0, 0, 60), (1, 1, 0, 30), (2, 2, 10, 20), (1, 1, 40, 50)]);
        assert_eq!(trace.threads[0].span, [[0, 0], [60, 30]]);
    }

    #[test]
    fn bottom_up_lists_callers() {
        let trace = parse(&sample()).unwrap();
        let up = trace.threads[0].tree.bottom_up();
        let top: Vec<(String, [u64; 2], [u64; 2])> = up.sorted_children(0, Clock::Wall).iter().map(|&n| (trace.label(up.nodes[n as usize].method), up.nodes[n as usize].total, up.nodes[n as usize].own)).collect();
        assert_eq!(top[0], ("run (com.example.Main)".into(), [60, 30], [20, 10]));
        assert_eq!(top[1], ("a (com.example.Main)".into(), [40, 20], [30, 15]));
        assert_eq!(top[2], ("b (com.example.Util)".into(), [10, 5], [10, 5]));
        let b = up.sorted_children(0, Clock::Wall)[2];
        let callers: Vec<String> = up.nodes[b as usize].children.iter().map(|&n| trace.label(up.nodes[n as usize].method)).collect();
        assert_eq!(callers, ["a (com.example.Main)"]);
        let grand = up.nodes[up.nodes[b as usize].children[0] as usize].children[0];
        assert_eq!(trace.label(up.nodes[grand as usize].method), "run (com.example.Main)");
    }

    #[test]
    fn recursion_counts_once_in_bottom_up() {
        let mut tree = CallTree::new();
        let mut index = HashMap::new();
        let outer = tree.child(&mut index, 0, 7);
        let inner = tree.child(&mut index, outer, 7);
        tree.nodes[outer as usize].total = [10, 10];
        tree.nodes[outer as usize].own = [4, 4];
        tree.nodes[inner as usize].total = [6, 6];
        tree.nodes[inner as usize].own = [6, 6];
        let up = tree.bottom_up();
        let top = &up.nodes[up.nodes[0].children[0] as usize];
        assert_eq!((top.total, top.own), ([10, 10], [10, 10]));
    }

    #[test]
    fn rejects_other_files() {
        assert!(parse(b"PK\x03\x04").is_none());
        assert!(parse(b"*version\n3\n*end\nnope").is_none());
    }
}
