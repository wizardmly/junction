//! Debug, as Android Studio's debugger session: line breakpoints become
//! breakpoint requests in each class compiled from their file, now or when
//! the class loads; a hit or a finished step pauses the session on its
//! thread (all threads suspended, IntelliJ's default); Resume, Pause, Step
//! Over / Into / Out, Run to Cursor and Stop drive the VM. The UI polls the
//! session from a background thread and gets what it should show.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::domain::debugger::{in_package, line_locations, package_pattern, DebugEvent, DebugVm, Frame, StepDepth, Suspend, ThreadInfo, Value, Variable};
use crate::usecase::evaluate::{self, Scope};

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
    /// Stops only when this holds (IntelliJ's Condition).
    pub condition: Option<String>,
    pub actions: BreakpointActions,
}

impl LineBreakpoint {
    pub fn new(path: &Path, package: String, line: u32) -> Self {
        let source_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Self { path: path.to_path_buf(), source_name, package, line, condition: None, actions: BreakpointActions::default() }
    }
}

/// What a hit does, as IntelliJ's breakpoint properties: Suspend
/// execution, and logging to the console ("Breakpoint hit" message, Stack
/// trace, Evaluate and log).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BreakpointActions {
    pub suspend: bool,
    pub log_message: bool,
    pub log_stack: bool,
    pub log_expression: Option<String>,
}

impl Default for BreakpointActions {
    fn default() -> Self {
        Self { suspend: true, log_message: false, log_stack: false, log_expression: None }
    }
}

impl BreakpointActions {
    pub fn logs(&self) -> bool {
        self.log_message || self.log_stack || self.log_expression.is_some()
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
    /// A line for the Debug window's console: a condition that failed.
    Message(String),
    /// What a logging breakpoint printed, for the console only.
    Log(String),
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
    /// Evaluations in progress: what their calls hit doesn't pause.
    evaluating: AtomicUsize,
}

impl DebugSession {
    pub fn new(vm: Arc<dyn DebugVm>) -> Self {
        Self { vm, state: Mutex::default(), evaluating: AtomicUsize::new(0) }
    }

    /// Changes a breakpoint's condition (none: always stop).
    pub fn set_condition(&self, path: &Path, line: u32, condition: Option<String>) {
        let mut state = self.state.lock().unwrap();
        if let Some(placed) = state.breakpoints.iter_mut().find(|p| p.breakpoint.path == path && p.breakpoint.line == line) {
            placed.breakpoint.condition = condition;
        }
    }

    /// Changes what a breakpoint's hit does.
    pub fn set_actions(&self, path: &Path, line: u32, actions: BreakpointActions) {
        let mut state = self.state.lock().unwrap();
        if let Some(placed) = state.breakpoints.iter_mut().find(|p| p.breakpoint.path == path && p.breakpoint.line == line) {
            placed.breakpoint.actions = actions;
        }
    }

    /// A frame of the paused thread by its depth, fetched now: a method
    /// call on the thread (an evaluation, a `toString()`) invalidates the
    /// frames' IDs taken before it.
    fn frame_at(&self, thread: u64, depth: usize) -> Result<Frame, String> {
        self.vm.frames(thread)?.into_iter().nth(depth).ok_or_else(|| "Frame is not available".to_string())
    }

    /// Evaluates an expression in a frame (by depth) of the paused thread
    /// (Evaluate Expression, Watches).
    pub fn evaluate(&self, thread: u64, depth: usize, text: &str) -> Result<Value, String> {
        let frame = self.frame_at(thread, depth)?;
        let locals = self.vm.locals(thread, &frame)?;
        self.evaluating.fetch_add(1, Ordering::SeqCst);
        let result = evaluate::evaluate(&Scope { vm: &*self.vm, thread, frame: &frame, locals: &locals }, text);
        self.evaluating.fetch_sub(1, Ordering::SeqCst);
        result
    }

    /// What the Variables view writes after an object: its `toString()`,
    /// a collection's size.
    pub fn describe(&self, thread: u64, value: &Value) -> Option<String> {
        self.evaluating.fetch_add(1, Ordering::SeqCst);
        let text = evaluate::describe(&*self.vm, thread, value);
        self.evaluating.fetch_sub(1, Ordering::SeqCst);
        text
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

    /// Detach: the program runs on without the debugger, its breakpoints
    /// gone (`VirtualMachine.Dispose`).
    pub fn detach(&self) {
        let mut state = self.state.lock().unwrap();
        state.ended = true;
        state.paused = None;
        drop(state);
        self.vm.dispose();
    }

    /// Another suspended thread's frames (the Frames view's thread list).
    pub fn frames(&self, thread: u64) -> Result<Vec<Frame>, String> {
        self.vm.frames(thread)
    }

    /// The Variables view of a frame (by depth).
    pub fn locals(&self, thread: u64, depth: usize) -> Result<Vec<Variable>, String> {
        let frame = self.frame_at(thread, depth)?;
        self.vm.locals(thread, &frame)
    }

    /// An expanded value's fields or elements, a collection's items
    /// (IntelliJ shows the first 100).
    pub fn children(&self, thread: u64, value: &Value) -> Result<Vec<Variable>, String> {
        self.evaluating.fetch_add(1, Ordering::SeqCst);
        let children = evaluate::children(&*self.vm, thread, value, 100);
        self.evaluating.fetch_sub(1, Ordering::SeqCst);
        children
    }

    /// Waits a while for the VM, and handles what it reports: what the UI
    /// should show.
    pub fn poll(&self, timeout: Duration) -> Result<Vec<SessionEvent>, String> {
        let Some(set) = self.vm.next_events(timeout)? else { return Ok(Vec::new()) };
        // A method an evaluation calls hit a breakpoint or a step ended in
        // it: let it run on, as IntelliJ does.
        let nested = set.events.iter().all(|e| matches!(e, DebugEvent::Breakpoint { .. } | DebugEvent::Step { .. }));
        if nested && !set.events.is_empty() && self.evaluating.load(Ordering::SeqCst) > 0 {
            self.vm.resume()?;
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        let mut state = self.state.lock().unwrap();
        let mut resume = false;
        let mut paused: Option<(u64, PauseReason)> = None;
        let mut hit: Option<LineBreakpoint> = None;
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
                    if reason == PauseReason::Breakpoint {
                        hit = state.breakpoints.iter().find(|p| p.requests.contains(&request)).map(|p| p.breakpoint.clone());
                    }
                    paused = Some((thread, reason));
                }
                DebugEvent::Step { thread, .. } => paused = Some((thread, PauseReason::Step)),
                // A VM launched to wait starts once the session calls start().
                DebugEvent::VmStart { .. } | DebugEvent::ThreadStart { .. } | DebugEvent::ThreadDeath { .. } => {}
            }
        }
        if let (Some((thread, PauseReason::Breakpoint)), Some(breakpoint)) = (paused, hit) {
            // The condition, in the hit frame: false runs on.
            drop(state);
            let frames = self.vm.frames(thread)?;
            let mut stop = breakpoint.actions.suspend;
            if let Some(text) = breakpoint.condition.as_deref() {
                match self.in_frame(thread, frames.first(), |scope| evaluate::condition_holds(scope, text)) {
                    Some(Ok(false)) => {
                        self.vm.resume()?;
                        return Ok(out);
                    }
                    Some(Ok(true)) | None => {}
                    // IntelliJ stops, and says why.
                    Some(Err(e)) => {
                        out.push(SessionEvent::Message(format!("Failed to evaluate breakpoint condition '{text}'. Reason: {e}")));
                        stop = true;
                    }
                }
            }
            if breakpoint.actions.logs() {
                out.push(SessionEvent::Log(self.hit_log(thread, &frames, &breakpoint)));
            }
            if !stop {
                self.vm.resume()?;
                return Ok(out);
            }
            state = self.state.lock().unwrap();
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

    /// Runs an evaluation in a frame of the thread a hit suspended; none
    /// without frames.
    fn in_frame<T>(&self, thread: u64, frame: Option<&Frame>, f: impl FnOnce(&Scope) -> Result<T, String>) -> Option<Result<T, String>> {
        let frame = frame?;
        let locals = match self.vm.locals(thread, frame) {
            Ok(locals) => locals,
            Err(e) => return Some(Err(e)),
        };
        self.evaluating.fetch_add(1, Ordering::SeqCst);
        let result = f(&Scope { vm: &*self.vm, thread, frame, locals: &locals });
        self.evaluating.fetch_sub(1, Ordering::SeqCst);
        Some(result)
    }

    /// What a logging breakpoint prints, in IntelliJ's words: "Breakpoint
    /// reached at a.CalcTest.adds(CalcTest.java:12)", the stack, the
    /// expression's value.
    fn hit_log(&self, thread: u64, frames: &[Frame], breakpoint: &LineBreakpoint) -> String {
        let location = |frame: &Frame| {
            let source = frame.source.clone().unwrap_or_else(|| "Unknown Source".into());
            match frame.line {
                Some(line) => format!("{}.{}({source}:{line})", frame.class, frame.method),
                None => format!("{}.{}({source})", frame.class, frame.method),
            }
        };
        let actions = &breakpoint.actions;
        let mut lines = Vec::new();
        if actions.log_message {
            let at = frames.first().map_or_else(|| format!("{}:{}", breakpoint.source_name, breakpoint.line), location);
            lines.push(format!("Breakpoint reached at {at}"));
        }
        if actions.log_stack {
            let mut trace = String::from("Breakpoint reached");
            for frame in frames {
                trace.push_str(&format!("\n\tat {}", location(frame)));
            }
            lines.push(trace);
        }
        if let Some(text) = actions.log_expression.as_deref() {
            // Frames taken before the condition's calls are stale.
            let frame = self.frame_at(thread, 0).ok();
            let value = self
                .in_frame(thread, frame.as_ref(), |scope| evaluate::evaluate(scope, text).and_then(|v| evaluate::as_string(scope.vm, thread, &v)))
                .unwrap_or_else(|| Err("Frame is not available".into()));
            lines.push(value.unwrap_or_else(|e| format!("Unable to evaluate the expression \"{text}\" : {e}")));
        }
        lines.join("\n")
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
        fn find_class(&self, _: &str) -> Result<Option<ClassRef>, String> {
            Ok(None)
        }
        fn class_of(&self, _: u64) -> Result<ClassRef, String> {
            Ok(ClassRef { id: 10, name: "a.CalcTest".into() })
        }
        fn class_name(&self, _: u64) -> Result<String, String> {
            Ok("a.CalcTest".into())
        }
        fn superclass(&self, _: u64) -> Result<u64, String> {
            Ok(0)
        }
        fn field(&self, _: u64, _: Option<u64>, _: &str) -> Result<Option<Value>, String> {
            Ok(None)
        }
        fn invoke(&self, _: u64, _: u64, _: Option<u64>, _: u64, _: &[Value]) -> Result<Value, String> {
            Err("not here".into())
        }
        fn create_string(&self, text: &str) -> Result<Value, String> {
            Ok(Value::String { id: 1, text: text.into() })
        }
        fn array_element(&self, _: u64, _: usize) -> Result<Value, String> {
            Ok(Value::Null)
        }
        fn next_events(&self, _: Duration) -> Result<Option<EventSet>, String> {
            Ok(self.events.lock().unwrap().pop_front())
        }
        fn exit(&self, code: i32) {
            self.log(format!("exit {code}"));
        }
        fn dispose(&self) {
            self.log("dispose".into());
        }
    }

    fn vm() -> Arc<FakeVm> {
        let method = |id, lines: &[(u64, u32)]| MethodInfo { id, name: format!("m{id}"), signature: "()V".into(), is_static: false, lines: lines.to_vec() };
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
    fn conditions_decide_whether_to_stop() {
        let vm = vm();
        vm.loaded.lock().unwrap().push(ClassRef { id: 10, name: "a.CalcTest".into() });
        let session = DebugSession::new(vm.clone());
        let mut breakpoint = breakpoint(5);
        breakpoint.condition = Some("x > 5".into());
        session.add_breakpoint(breakpoint.clone()).unwrap();
        let request = *vm.next.lock().unwrap();
        vm.take_log();
        let hit = || vec![DebugEvent::Breakpoint { thread: 1, location: Location { class: 10, method: 1, index: 4 }, request }];
        // x is 4: runs on.
        vm.send(Suspend::All, hit());
        assert_eq!(session.poll(Duration::ZERO), Ok(vec![]));
        assert_eq!(vm.take_log(), ["resume"]);
        // Changed in the popup: stops.
        session.set_condition(&breakpoint.path, 5, Some("x == 4 && x + 1 > 4".into()));
        vm.send(Suspend::All, hit());
        assert!(matches!(session.poll(Duration::ZERO).unwrap()[..], [SessionEvent::Paused { reason: PauseReason::Breakpoint, .. }]));
        // A condition that fails stops too, and says why.
        session.set_condition(&breakpoint.path, 5, Some("missing > 1".into()));
        vm.send(Suspend::All, hit());
        let events = session.poll(Duration::ZERO).unwrap();
        assert!(matches!(&events[0], SessionEvent::Message(m) if m.contains("'missing > 1'") && m.contains("Cannot find local variable 'missing'")), "{events:?}");
        assert!(matches!(events[1], SessionEvent::Paused { .. }));
        // An evaluation's own breakpoint hits don't pause.
        session.evaluating.fetch_add(1, Ordering::SeqCst);
        vm.take_log();
        vm.send(Suspend::All, hit());
        assert_eq!(session.poll(Duration::ZERO), Ok(vec![]));
        assert_eq!(vm.take_log(), ["resume"]);
    }

    #[test]
    fn logging_breakpoints_print_and_may_run_on() {
        let vm = vm();
        vm.loaded.lock().unwrap().push(ClassRef { id: 10, name: "a.CalcTest".into() });
        let session = DebugSession::new(vm.clone());
        let mut breakpoint = breakpoint(5);
        breakpoint.actions = BreakpointActions { suspend: false, log_message: true, log_stack: true, log_expression: Some("x + 1".into()) };
        session.add_breakpoint(breakpoint.clone()).unwrap();
        let request = *vm.next.lock().unwrap();
        vm.take_log();
        let hit = || vec![DebugEvent::Breakpoint { thread: 1, location: Location { class: 10, method: 1, index: 4 }, request }];
        // Not suspending: printed in IntelliJ's words, then runs on.
        vm.send(Suspend::All, hit());
        assert_eq!(
            session.poll(Duration::ZERO),
            Ok(vec![SessionEvent::Log("Breakpoint reached at a.CalcTest.on1(CalcTest.java:5)\nBreakpoint reached\n\tat a.CalcTest.on1(CalcTest.java:5)\n5".into())])
        );
        assert_eq!(vm.take_log(), ["resume"]);
        // An expression that fails says so; suspending again, it pauses.
        session.set_actions(&breakpoint.path, 5, BreakpointActions { log_expression: Some("missing".into()), ..BreakpointActions::default() });
        vm.send(Suspend::All, hit());
        let events = session.poll(Duration::ZERO).unwrap();
        assert!(matches!(&events[0], SessionEvent::Log(m) if m.starts_with("Unable to evaluate the expression \"missing\" : ")), "{events:?}");
        assert!(matches!(events[1], SessionEvent::Paused { reason: PauseReason::Breakpoint, .. }));
        // A false condition neither logs nor stops.
        session.set_condition(&breakpoint.path, 5, Some("x > 5".into()));
        vm.take_log();
        vm.send(Suspend::All, hit());
        assert_eq!(session.poll(Duration::ZERO), Ok(vec![]));
        // Detach lets the program run on.
        session.detach();
        assert!(session.is_ended());
        assert_eq!(vm.take_log(), ["resume", "dispose"]);
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
        assert_eq!(session.locals(1, 0).unwrap()[0].name, "x");
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
