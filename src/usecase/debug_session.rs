//! Debug, as Android Studio's debugger session: line breakpoints become
//! breakpoint requests in each class compiled from their file, now or when
//! the class loads; a hit or a finished step pauses the session on its
//! thread (all threads suspended, IntelliJ's default); Resume, Pause, Step
//! Over / Into / Out, Run to Cursor and Stop drive the VM. The UI polls the
//! session from a background thread and gets what it should show.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::domain::debugger::{in_package, line_locations, package_pattern, DebugEvent, DebugVm, Frame, StepDepth, Suspend, ThreadInfo, Variable};

/// A line breakpoint, as the editor's gutter sets it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineBreakpoint {
    pub path: PathBuf,
    /// The file's name: what the classes' `SourceFile` says.
    pub source_name: String,
    /// The package the file declares.
    pub package: String,
    /// 1-based.
    pub line: u32,
}

impl LineBreakpoint {
    pub fn new(path: &Path, package: String, line: u32) -> Self {
        let source_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Self { path: path.to_path_buf(), source_name, package, line }
    }
}

/// Why the session paused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseReason {
    Breakpoint,
    Step,
    /// Pause Program, or Run to Cursor's line.
    Paused,
}

/// What the UI shows of the session.
#[derive(Clone, Debug, PartialEq)]
pub enum SessionEvent {
    /// Paused on a thread: its frames (the top one is the execution point)
    /// and all threads, for the Frames view's list.
    Paused { thread: u64, frames: Vec<Frame>, threads: Vec<ThreadInfo>, reason: PauseReason },
    Resumed,
    /// A breakpoint now stops somewhere (IntelliJ's verified breakpoint).
    Verified { path: PathBuf, line: u32 },
    /// The program ended, or was stopped or detached.
    Ended,
}

struct Placed {
    breakpoint: LineBreakpoint,
    requests: Vec<u32>,
}

#[derive(Default)]
struct State {
    breakpoints: Vec<Placed>,
    /// Class prepare watches, by package pattern.
    watched: HashMap<String, u32>,
    step: Option<u32>,
    /// Run to Cursor's breakpoint, removed at the next pause.
    run_to: Vec<u32>,
    paused: Option<u64>,
    ended: bool,
}

pub struct DebugSession {
    vm: Arc<dyn DebugVm>,
    state: Mutex<State>,
}

impl DebugSession {
    pub fn new(vm: Arc<dyn DebugVm>) -> Self {
        Self { vm, state: Mutex::default() }
    }

    pub fn is_ended(&self) -> bool {
        self.state.lock().unwrap().ended
    }

    /// Lets a VM launched to wait for the debugger run, once the
    /// breakpoints are set.
    pub fn start(&self) -> Result<(), String> {
        self.vm.resume()
    }

    /// Sets a line breakpoint: in the classes from its file loaded now, and
    /// in those that load later. Whether it stops somewhere already.
    pub fn add_breakpoint(&self, breakpoint: LineBreakpoint) -> Result<bool, String> {
        let mut state = self.state.lock().unwrap();
        if state.breakpoints.iter().any(|p| p.breakpoint.path == breakpoint.path && p.breakpoint.line == breakpoint.line) {
            return Ok(state.breakpoints.iter().any(|p| p.breakpoint == breakpoint && !p.requests.is_empty()));
        }
        let pattern = package_pattern(&breakpoint.package);
        if !state.watched.contains_key(&pattern) {
            let request = self.vm.watch_class_prepare(&pattern)?;
            state.watched.insert(pattern, request);
        }
        let mut requests = Vec::new();
        for class in self.vm.all_classes()?.into_iter().filter(|c| in_package(&c.name, &breakpoint.package)) {
            requests.extend(self.place(class.id, &breakpoint)?);
        }
        let verified = !requests.is_empty();
        state.breakpoints.push(Placed { breakpoint, requests });
        Ok(verified)
    }

    pub fn remove_breakpoint(&self, path: &Path, line: u32) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        let Some(ix) = state.breakpoints.iter().position(|p| p.breakpoint.path == path && p.breakpoint.line == line) else { return Ok(()) };
        let placed = state.breakpoints.remove(ix);
        for request in placed.requests {
            self.vm.clear_breakpoint(request)?;
        }
        Ok(())
    }

    /// Breakpoint requests for a class, when it was compiled from the
    /// breakpoint's file and has code on its line.
    fn place(&self, class: u64, breakpoint: &LineBreakpoint) -> Result<Vec<u32>, String> {
        if self.vm.source_file(class)?.as_deref() != Some(breakpoint.source_name.as_str()) {
            return Ok(Vec::new());
        }
        let methods = self.vm.methods(class)?;
        line_locations(class, &methods, breakpoint.line).into_iter().map(|location| self.vm.set_breakpoint(location)).collect()
    }

    /// Resume Program (F9).
    pub fn resume(&self) -> Result<SessionEvent, String> {
        let mut state = self.state.lock().unwrap();
        state.paused = None;
        self.vm.resume()?;
        Ok(SessionEvent::Resumed)
    }

    /// Pause Program: all threads stop; the frames are the main thread's
    /// (else the first one's), as IntelliJ picks.
    pub fn pause(&self) -> Result<SessionEvent, String> {
        self.vm.suspend()?;
        let threads = self.vm.threads()?;
        let thread = threads.iter().find(|t| t.name == "main").or(threads.first()).map(|t| t.id).ok_or("No threads")?;
        self.state.lock().unwrap().paused = Some(thread);
        let frames = self.vm.frames(thread)?;
        Ok(SessionEvent::Paused { thread, frames, threads, reason: PauseReason::Paused })
    }

    /// Step Over (F8), Step Into (F7), Step Out (Shift+F8) on the paused
    /// thread.
    pub fn step(&self, depth: StepDepth) -> Result<SessionEvent, String> {
        let mut state = self.state.lock().unwrap();
        let thread = state.paused.ok_or("Not paused")?;
        if let Some(old) = state.step.take() {
            self.vm.clear_step(old)?;
        }
        state.step = Some(self.vm.step(thread, depth)?);
        state.paused = None;
        self.vm.resume()?;
        Ok(SessionEvent::Resumed)
    }

    /// Run to Cursor (Alt+F9): a breakpoint at the line for the next pause
    /// only, then resume.
    pub fn run_to_line(&self, target: &LineBreakpoint) -> Result<SessionEvent, String> {
        let mut requests = Vec::new();
        for class in self.vm.all_classes()?.into_iter().filter(|c| in_package(&c.name, &target.package)) {
            requests.extend(self.place(class.id, target)?);
        }
        if requests.is_empty() {
            return Err(format!("No executable code found at line {} in {}", target.line, target.source_name));
        }
        let mut state = self.state.lock().unwrap();
        state.run_to.extend(requests);
        state.paused = None;
        self.vm.resume()?;
        Ok(SessionEvent::Resumed)
    }

    /// Stop (Ctrl+F2): ends the program.
    pub fn stop(&self) {
        self.state.lock().unwrap().ended = true;
        self.vm.exit(1);
    }

    /// Another suspended thread's frames (the Frames view's thread list).
    pub fn frames(&self, thread: u64) -> Result<Vec<Frame>, String> {
        self.vm.frames(thread)
    }

    /// The Variables view of a frame.
    pub fn locals(&self, thread: u64, frame: &Frame) -> Result<Vec<Variable>, String> {
        self.vm.locals(thread, frame)
    }

    /// An expanded value's fields or elements (IntelliJ shows the first 100).
    pub fn children(&self, object: u64) -> Result<Vec<Variable>, String> {
        self.vm.children(object, 100)
    }

    /// Waits a while for the VM, and handles what it reports: what the UI
    /// should show.
    pub fn poll(&self, timeout: Duration) -> Result<Vec<SessionEvent>, String> {
        let Some(set) = self.vm.next_events(timeout)? else { return Ok(Vec::new()) };
        let mut out = Vec::new();
        let mut state = self.state.lock().unwrap();
        let mut resume = false;
        let mut paused: Option<(u64, PauseReason)> = None;
        for event in set.events {
            match event {
                DebugEvent::VmDeath => {
                    state.ended = true;
                    state.paused = None;
                    out.push(SessionEvent::Ended);
                    return Ok(out);
                }
                DebugEvent::ClassPrepare { class, .. } => {
                    for ix in 0..state.breakpoints.len() {
                        if !in_package(&class.name, &state.breakpoints[ix].breakpoint.package) {
                            continue;
                        }
                        let placed = self.place(class.id, &state.breakpoints[ix].breakpoint)?;
                        let entry = &mut state.breakpoints[ix];
                        if entry.requests.is_empty() && !placed.is_empty() {
                            out.push(SessionEvent::Verified { path: entry.breakpoint.path.clone(), line: entry.breakpoint.line });
                        }
                        entry.requests.extend(placed);
                    }
                    resume = set.suspended != Suspend::None;
                }
                DebugEvent::Breakpoint { thread, request, .. } => {
                    let reason = if state.run_to.contains(&request) { PauseReason::Paused } else { PauseReason::Breakpoint };
                    paused = Some((thread, reason));
                }
                DebugEvent::Step { thread, .. } => paused = Some((thread, PauseReason::Step)),
                // A VM launched to wait starts once the session calls start().
                DebugEvent::VmStart { .. } | DebugEvent::ThreadStart { .. } | DebugEvent::ThreadDeath { .. } => {}
            }
        }
        if let Some((thread, reason)) = paused {
            // A pause ends a step in progress and Run to Cursor's breakpoint.
            if let Some(step) = state.step.take() {
                self.vm.clear_step(step)?;
            }
            for request in std::mem::take(&mut state.run_to) {
                self.vm.clear_breakpoint(request)?;
            }
            state.paused = Some(thread);
            drop(state);
            let frames = self.vm.frames(thread)?;
            let threads = self.vm.threads()?;
            out.push(SessionEvent::Paused { thread, frames, threads, reason });
        } else if resume {
            self.vm.resume()?;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::debugger::{ClassRef, EventSet, Location, MethodInfo, ThreadStatus, Value};
    use std::collections::VecDeque;

    /// A VM with a script of events and classes that load when told.
    #[derive(Default)]
    struct FakeVm {
        loaded: Mutex<Vec<ClassRef>>,
        sources: HashMap<u64, String>,
        methods: HashMap<u64, Vec<MethodInfo>>,
        events: Mutex<VecDeque<EventSet>>,
        log: Mutex<Vec<String>>,
        next: Mutex<u32>,
    }

    impl FakeVm {
        fn log(&self, entry: String) -> u32 {
            self.log.lock().unwrap().push(entry);
            let mut next = self.next.lock().unwrap();
            *next += 1;
            *next
        }

        fn take_log(&self) -> Vec<String> {
            std::mem::take(&mut self.log.lock().unwrap())
        }

        fn send(&self, suspended: Suspend, events: Vec<DebugEvent>) {
            self.events.lock().unwrap().push_back(EventSet { suspended, events });
        }
    }

    impl DebugVm for FakeVm {
        fn all_classes(&self) -> Result<Vec<ClassRef>, String> {
            Ok(self.loaded.lock().unwrap().clone())
        }
        fn source_file(&self, class: u64) -> Result<Option<String>, String> {
            Ok(self.sources.get(&class).cloned())
        }
        fn methods(&self, class: u64) -> Result<Vec<MethodInfo>, String> {
            Ok(self.methods.get(&class).cloned().unwrap_or_default())
        }
        fn watch_class_prepare(&self, pattern: &str) -> Result<u32, String> {
            Ok(self.log(format!("watch {pattern}")))
        }
        fn set_breakpoint(&self, l: Location) -> Result<u32, String> {
            Ok(self.log(format!("break {}:{}:{}", l.class, l.method, l.index)))
        }
        fn clear_breakpoint(&self, request: u32) -> Result<(), String> {
            self.log(format!("clear {request}"));
            Ok(())
        }
        fn step(&self, thread: u64, depth: StepDepth) -> Result<u32, String> {
            Ok(self.log(format!("step {thread} {depth:?}")))
        }
        fn clear_step(&self, request: u32) -> Result<(), String> {
            self.log(format!("clear step {request}"));
            Ok(())
        }
        fn suspend(&self) -> Result<(), String> {
            self.log("suspend".into());
            Ok(())
        }
        fn resume(&self) -> Result<(), String> {
            self.log("resume".into());
            Ok(())
        }
        fn threads(&self) -> Result<Vec<ThreadInfo>, String> {
            Ok(vec![
                ThreadInfo { id: 2, name: "Signal Dispatcher".into(), status: ThreadStatus::Running, suspended: true },
                ThreadInfo { id: 1, name: "main".into(), status: ThreadStatus::Running, suspended: true },
            ])
        }
        fn frames(&self, thread: u64) -> Result<Vec<Frame>, String> {
            Ok(vec![Frame { id: 9, location: Location { class: 10, method: 1, index: 4 }, class: "a.CalcTest".into(), method: format!("on{thread}"), line: Some(5), source: Some("CalcTest.java".into()) }])
        }
        fn locals(&self, _: u64, _: &Frame) -> Result<Vec<Variable>, String> {
            Ok(vec![Variable { name: "x".into(), value: Value::Int(4) }])
        }
        fn children(&self, _: u64, _: usize) -> Result<Vec<Variable>, String> {
            Ok(Vec::new())
        }
        fn next_events(&self, _: Duration) -> Result<Option<EventSet>, String> {
            Ok(self.events.lock().unwrap().pop_front())
        }
        fn exit(&self, code: i32) {
            self.log(format!("exit {code}"));
        }
    }

    fn vm() -> Arc<FakeVm> {
        let method = |id, lines: &[(u64, u32)]| MethodInfo { id, name: format!("m{id}"), signature: "()V".into(), lines: lines.to_vec() };
        let mut fake = FakeVm::default();
        // a.CalcTest from CalcTest.java; a.Other from Other.java; b.CalcTest elsewhere.
        fake.sources = HashMap::from([(10, "CalcTest.java".into()), (11, "Other.java".into()), (12, "CalcTest.java".into()), (13, "CalcTest.java".into())]);
        fake.methods = HashMap::from([(10, vec![method(1, &[(0, 4), (4, 5)])]), (11, vec![method(2, &[(0, 5)])]), (12, vec![method(3, &[(0, 5)])]), (13, vec![method(4, &[(2, 9)])])]);
        Arc::new(fake)
    }

    fn breakpoint(line: u32) -> LineBreakpoint {
        LineBreakpoint::new(Path::new("/p/src/test/java/a/CalcTest.java"), "a".into(), line)
    }

    #[test]
    fn breakpoints_wait_for_their_classes() {
        let vm = vm();
        let session = DebugSession::new(vm.clone());
        // Nothing loaded yet: watched, not verified.
        assert_eq!(session.add_breakpoint(breakpoint(5)), Ok(false));
        assert_eq!(vm.take_log(), ["watch a.*"]);
        // Another in the same package needs no second watch.
        assert_eq!(session.add_breakpoint(breakpoint(9)), Ok(false));
        assert!(vm.take_log().is_empty());
        session.start().unwrap();
        vm.take_log();
        // a.Other loads (wrong file), then a.CalcTest and its lambda class.
        let prepare = |id, name: &str| DebugEvent::ClassPrepare { thread: 1, class: ClassRef { id, name: name.into() }, request: 1 };
        vm.send(Suspend::All, vec![prepare(11, "a.Other")]);
        assert_eq!(session.poll(Duration::ZERO), Ok(vec![]));
        assert_eq!(vm.take_log(), ["resume"]);
        vm.send(Suspend::All, vec![prepare(10, "a.CalcTest"), prepare(13, "a.CalcTest$1")]);
        let events = session.poll(Duration::ZERO).unwrap();
        assert_eq!(events, vec![
            SessionEvent::Verified { path: "/p/src/test/java/a/CalcTest.java".into(), line: 5 },
            SessionEvent::Verified { path: "/p/src/test/java/a/CalcTest.java".into(), line: 9 },
        ]);
        assert_eq!(vm.take_log(), ["break 10:1:4", "break 13:4:2", "resume"]);
        // b.CalcTest isn't in the package.
        vm.send(Suspend::All, vec![prepare(12, "b.CalcTest")]);
        session.poll(Duration::ZERO).unwrap();
        assert_eq!(vm.take_log(), ["resume"]);
        // Removing clears its requests.
        session.remove_breakpoint(Path::new("/p/src/test/java/a/CalcTest.java"), 9).unwrap();
        assert_eq!(vm.take_log().len(), 1);
    }

    #[test]
    fn breakpoints_in_loaded_classes_are_set_at_once() {
        let vm = vm();
        vm.loaded.lock().unwrap().extend([ClassRef { id: 10, name: "a.CalcTest".into() }, ClassRef { id: 12, name: "b.CalcTest".into() }]);
        let session = DebugSession::new(vm.clone());
        assert_eq!(session.add_breakpoint(breakpoint(5)), Ok(true));
        assert_eq!(vm.take_log(), ["watch a.*", "break 10:1:4"]);
        // A line without code isn't verified.
        assert_eq!(session.add_breakpoint(breakpoint(7)), Ok(false));
    }

    #[test]
    fn pauses_steps_and_ends() {
        let vm = vm();
        vm.loaded.lock().unwrap().push(ClassRef { id: 10, name: "a.CalcTest".into() });
        let session = DebugSession::new(vm.clone());
        session.add_breakpoint(breakpoint(5)).unwrap();
        vm.take_log();
        let hit = DebugEvent::Breakpoint { thread: 1, location: Location { class: 10, method: 1, index: 4 }, request: 2 };
        vm.send(Suspend::All, vec![hit]);
        let events = session.poll(Duration::ZERO).unwrap();
        let [SessionEvent::Paused { thread: 1, frames, threads, reason: PauseReason::Breakpoint }] = events.as_slice() else { panic!("{events:?}") };
        assert_eq!((frames[0].line, threads.len()), (Some(5), 2));
        assert_eq!(session.locals(1, &frames[0]).unwrap()[0].name, "x");
        // Step Over: a step request, resumed; its end pauses again and clears it.
        assert_eq!(session.step(StepDepth::Over), Ok(SessionEvent::Resumed));
        assert_eq!(vm.take_log(), ["step 1 Over", "resume"]);
        let step = *vm.next.lock().unwrap() - 1;
        vm.send(Suspend::All, vec![DebugEvent::Step { thread: 1, location: Location { class: 10, method: 1, index: 6 }, request: step }]);
        assert!(matches!(session.poll(Duration::ZERO).unwrap()[..], [SessionEvent::Paused { reason: PauseReason::Step, .. }]));
        assert_eq!(vm.take_log(), [format!("clear step {step}")]);
        // Pause Program picks main.
        let SessionEvent::Paused { thread, reason, .. } = session.pause().unwrap() else { panic!() };
        assert_eq!((thread, reason), (1, PauseReason::Paused));
        // Run to Cursor: a one-time breakpoint, cleared at the pause.
        vm.take_log();
        session.run_to_line(&breakpoint(4)).unwrap();
        assert_eq!(vm.take_log(), ["break 10:1:0", "resume"]);
        let once = *vm.next.lock().unwrap() - 1;
        vm.send(Suspend::All, vec![DebugEvent::Breakpoint { thread: 1, location: Location { class: 10, method: 1, index: 0 }, request: once }]);
        assert!(matches!(session.poll(Duration::ZERO).unwrap()[..], [SessionEvent::Paused { reason: PauseReason::Paused, .. }]));
        assert_eq!(vm.take_log(), [format!("clear {once}")]);
        assert!(session.run_to_line(&breakpoint(30)).is_err());
        // The program ends.
        vm.send(Suspend::None, vec![DebugEvent::VmDeath]);
        assert_eq!(session.poll(Duration::ZERO), Ok(vec![SessionEvent::Ended]));
        assert!(session.is_ended());
    }
}
