//! Debug, as Android Studio's debugger session: line breakpoints become
//! breakpoint requests in each class compiled from their file, now or when
//! the class loads; a hit or a finished step pauses the session on its
//! thread (all threads suspended, IntelliJ's default); Resume, Pause, Step
//! Over / Into / Out, Run to Cursor and Stop drive the VM. The UI polls the
//! session from a background thread and gets what it should show.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::domain::debugger::{in_package, line_locations, method_entries, FieldWatch, MethodHeader, package_pattern, DebugEvent, DebugVm, Frame, StepDepth, Suspend, ThreadInfo, Value, Variable};
use crate::domain::hotswap::{ClassFiles, Dexer, HotSwapOutcome};
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
    /// A method breakpoint: it stops on entering the method declared on
    /// `line`.
    pub method: Option<MethodHeader>,
    /// A method breakpoint's Method entry (on by default) and Method exit
    /// (IntelliJ's Watch options).
    pub method_entry: bool,
    pub method_exit: bool,
    /// A field watchpoint: it stops where the field declared on `line` is
    /// read or written.
    pub field: Option<FieldWatch>,
    /// An interface / abstract method breakpoint's implementations: it
    /// stops entering (or leaving) each of them, as IntelliJ's.
    pub implementations: Vec<Implementation>,
}

/// A method implementing an abstract one: its file's name, its package and
/// the method as declared there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Implementation {
    pub source_name: String,
    pub package: String,
    pub header: MethodHeader,
}

impl LineBreakpoint {
    /// The packages whose classes may hold its code.
    fn packages(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.package.as_str()).chain(self.implementations.iter().map(|i| i.package.as_str()))
    }

    fn in_packages(&self, class: &str) -> bool {
        self.packages().any(|p| in_package(class, p))
    }
}

impl LineBreakpoint {
    pub fn new(path: &Path, package: String, line: u32) -> Self {
        let source_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Self { path: path.to_path_buf(), source_name, package, line, condition: None, actions: BreakpointActions::default(), method: None, method_entry: true, method_exit: false, field: None, implementations: Vec::new() }
    }
}

/// What a hit does, as IntelliJ's breakpoint properties: Suspend
/// execution, and logging to the console ("Breakpoint hit" message, Stack
/// trace, Evaluate and log).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BreakpointActions {
    pub suspend: bool,
    /// Remove once hit: the first hit whose condition holds removes it.
    pub remove_once_hit: bool,
    pub log_message: bool,
    pub log_stack: bool,
    pub log_expression: Option<String>,
    /// IntelliJ's "Disable until hitting the following breakpoint": the
    /// breakpoint (file, 1-based line) whose hit arms this one; until then
    /// its hits run on.
    pub master: Option<(PathBuf, u32)>,
    /// After its own hit: stays armed ("Leave enabled") rather than waiting
    /// for the master again ("Disable again").
    pub leave_enabled: bool,
}

impl Default for BreakpointActions {
    fn default() -> Self {
        Self { suspend: true, remove_once_hit: false, log_message: false, log_stack: false, log_expression: None, master: None, leave_enabled: false }
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
    /// An exception breakpoint: the console says which exception.
    Exception,
}

/// IntelliJ's Java Exception Breakpoint: a class's exceptions (its
/// subclasses' too), or any exception (`class` empty), caught, uncaught
/// or both.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExceptionBreakpoint {
    pub class: String,
    pub caught: bool,
    pub uncaught: bool,
    /// Stops only when this holds, with the exception as `this`.
    pub condition: Option<String>,
    /// IntelliJ's Class filters, on the class it's thrown in: patterns
    /// with `*`, `-` before one that excludes.
    pub filters: Vec<String>,
    /// IntelliJ's Catch class filters, the same on the class catching it;
    /// an uncaught exception passes.
    pub catch_filters: Vec<String>,
}

/// Whether a class passes class filters: no include pattern or one that
/// matches, and no exclude pattern that matches.
pub fn passes_filters(filters: &[String], class: &str) -> bool {
    let (excludes, includes): (Vec<&str>, Vec<&str>) = filters.iter().map(|f| f.trim()).filter(|f| !f.is_empty()).partition(|f| f.starts_with('-'));
    (includes.is_empty() || includes.iter().any(|p| glob(p, class))) && !excludes.iter().any(|p| glob(&p[1..], class))
}

/// A pattern where `*` is any run of characters.
fn glob(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = text.strip_prefix(first) else { return false };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else { return rest.is_empty() };
    for part in middle {
        let Some(at) = rest.find(part) else { return false };
        rest = &rest[at + part.len()..];
    }
    rest.ends_with(last)
}

struct PlacedException {
    breakpoint: ExceptionBreakpoint,
    /// None until its class loads.
    request: Option<u32>,
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
    /// An exception breakpoint stopped: which exception.
    Exception(String),
    /// A Remove once hit breakpoint was hit, and is gone from the session.
    Removed { path: PathBuf, line: u32 },
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
    exceptions: Vec<PlacedException>,
    /// Class prepare watches, by package pattern.
    watched: HashMap<String, u32>,
    step: Option<u32>,
    /// Run to Cursor's breakpoint, removed at the next pause.
    run_to: Vec<u32>,
    paused: Option<u64>,
    /// The exception an exception breakpoint stopped at, and its thread.
    thrown: Option<(u64, Value)>,
    /// Dependent breakpoints whose master has been hit (file, line).
    armed: std::collections::HashSet<(PathBuf, u32)>,
    ended: bool,
}

pub struct DebugSession {
    vm: Arc<dyn DebugVm>,
    state: Mutex<State>,
    /// Evaluations in progress: what their calls hit doesn't pause.
    evaluating: AtomicUsize,
    /// Mute Breakpoints: hits run on (Run to Cursor and steps still stop).
    muted: AtomicBool,
    /// Method exit requests, with the methods whose exits stop.
    exits: Mutex<HashMap<u32, Vec<u64>>>,
}

impl DebugSession {
    pub fn new(vm: Arc<dyn DebugVm>) -> Self {
        Self { vm, state: Mutex::default(), evaluating: AtomicUsize::new(0), muted: AtomicBool::new(false), exits: Mutex::default() }
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

    /// Mute Breakpoints, as IntelliJ's: line, method, field and exception
    /// breakpoints stay set but don't stop or log.
    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::SeqCst);
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
        for package in breakpoint.packages() {
            let pattern = package_pattern(package);
            if !state.watched.contains_key(&pattern) {
                let request = self.vm.watch_class_prepare(&pattern)?;
                state.watched.insert(pattern, request);
            }
        }
        let mut requests = Vec::new();
        for class in self.vm.all_classes()?.into_iter().filter(|c| breakpoint.in_packages(&c.name)) {
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
            if placed.breakpoint.field.is_some() {
                // Either kind: clearing the other one does nothing.
                self.vm.clear_field_watch(request, false)?;
                self.vm.clear_field_watch(request, true)?;
            } else {
                self.clear_request(request)?;
            }
        }
        Ok(())
    }

    /// Clears a line or method breakpoint's request.
    fn clear_request(&self, request: u32) -> Result<(), String> {
        if self.exits.lock().unwrap().remove(&request).is_some() {
            self.vm.clear_method_exit(request)
        } else {
            self.vm.clear_breakpoint(request)
        }
    }

    /// Sets an exception breakpoint (replacing the class's): now, when
    /// its class is loaded or it's for any exception; else once it loads.
    /// Whether it's set already.
    pub fn add_exception_breakpoint(&self, breakpoint: ExceptionBreakpoint) -> Result<bool, String> {
        self.remove_exception_breakpoint(&breakpoint.class)?;
        let mut state = self.state.lock().unwrap();
        let request = if breakpoint.class.is_empty() {
            Some(self.vm.set_exception_breakpoint(None, breakpoint.caught, breakpoint.uncaught)?)
        } else {
            match self.vm.find_class(&breakpoint.class)? {
                Some(class) => Some(self.vm.set_exception_breakpoint(Some(class.id), breakpoint.caught, breakpoint.uncaught)?),
                None => {
                    if !state.watched.contains_key(&breakpoint.class) {
                        let watch = self.vm.watch_class_prepare(&breakpoint.class)?;
                        state.watched.insert(breakpoint.class.clone(), watch);
                    }
                    None
                }
            }
        };
        state.exceptions.push(PlacedException { breakpoint, request });
        Ok(request.is_some())
    }

    pub fn remove_exception_breakpoint(&self, class: &str) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        let Some(ix) = state.exceptions.iter().position(|p| p.breakpoint.class == class) else { return Ok(()) };
        if let Some(request) = state.exceptions.remove(ix).request {
            self.vm.clear_exception_breakpoint(request)?;
        }
        Ok(())
    }

    /// Breakpoint requests for a class, when it was compiled from the
    /// breakpoint's file and has code on its line.
    fn place(&self, class: u64, breakpoint: &LineBreakpoint) -> Result<Vec<u32>, String> {
        let source = self.vm.source_file(class)?;
        // An implementation of the abstract method in this class.
        let mut requests = Vec::new();
        for implementation in breakpoint.implementations.iter().filter(|i| source.as_deref() == Some(i.source_name.as_str())) {
            let methods = self.vm.methods(class)?;
            requests.extend(self.place_method(class, &methods, &implementation.header, breakpoint)?);
        }
        if source.as_deref() != Some(breakpoint.source_name.as_str()) {
            return Ok(requests);
        }
        if let Some(watch) = &breakpoint.field {
            let Some(field) = self.vm.field_id(class, &watch.name)? else { return Ok(Vec::new()) };
            let mut requests = Vec::new();
            for (on, access) in [(watch.modification, false), (watch.access, true)] {
                if on {
                    requests.push(self.vm.set_field_watch(class, field, access)?);
                }
            }
            return Ok(requests);
        }
        let methods = self.vm.methods(class)?;
        if let Some(header) = &breakpoint.method {
            requests.extend(self.place_method(class, &methods, header, breakpoint)?);
            return Ok(requests);
        }
        for location in line_locations(class, &methods, breakpoint.line) {
            requests.push(self.vm.set_breakpoint(location)?);
        }
        Ok(requests)
    }

    /// A method breakpoint's requests for the method `header` declares in
    /// a class.
    fn place_method(&self, class: u64, methods: &[crate::domain::debugger::MethodInfo], header: &MethodHeader, breakpoint: &LineBreakpoint) -> Result<Vec<u32>, String> {
        let entries = method_entries(class, methods, header);
        let mut requests = Vec::new();
        // Exits come as events from the class's methods, kept for these
        // (IntelliJ's non-emulated method breakpoint).
        if breakpoint.method_exit && !entries.is_empty() {
            let request = self.vm.set_method_exit(class)?;
            self.exits.lock().unwrap().insert(request, entries.iter().map(|l| l.method).collect());
            requests.push(request);
        }
        if breakpoint.method_entry {
            for location in entries {
                requests.push(self.vm.set_breakpoint(location)?);
            }
        }
        Ok(requests)
    }

    /// Resume Program (F9).
    pub fn resume(&self) -> Result<SessionEvent, String> {
        let mut state = self.state.lock().unwrap();
        state.paused = None;
        state.thrown = None;
        self.vm.resume()?;
        Ok(SessionEvent::Resumed)
    }

    /// Pause Program: all threads stop; the frames are the main thread's
    /// (else the first one's), as IntelliJ picks.
    pub fn pause(&self) -> Result<SessionEvent, String> {
        self.vm.suspend()?;
        let threads = self.vm.threads()?;
        let thread = threads.iter().find(|t| t.name == "main").or(threads.first()).map(|t| t.id).ok_or("No threads")?;
        {
            let mut state = self.state.lock().unwrap();
            state.paused = Some(thread);
            state.thrown = None;
        }
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
        state.thrown = None;
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
        state.thrown = None;
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
        state.thrown = None;
        drop(state);
        self.vm.dispose();
    }

    /// HotSwap (IntelliJ's `ReloadClassesWorker`): the changed classes the
    /// VM has loaded get the new code (dexed first on Android); the line
    /// breakpoints in them are placed again, as the old ones went with the
    /// old code.
    pub fn hot_swap(&self, changed: &[(String, PathBuf)], files: &dyn ClassFiles, dexer: Option<&dyn Dexer>) -> HotSwapOutcome {
        match self.redefine(changed, files, dexer) {
            Ok(0) => HotSwapOutcome::UpToDate,
            Ok(n) => HotSwapOutcome::Reloaded(n),
            Err(e) => HotSwapOutcome::Failed(e),
        }
    }

    fn redefine(&self, changed: &[(String, PathBuf)], files: &dyn ClassFiles, dexer: Option<&dyn Dexer>) -> Result<usize, String> {
        let mut loaded = Vec::new();
        for (name, file) in changed {
            if let Some(class) = self.vm.find_class(name)? {
                loaded.push((class.id, name.as_str(), file.clone()));
            }
        }
        if loaded.is_empty() {
            return Ok(0);
        }
        if !self.vm.can_redefine()? {
            return Err(crate::domain::hotswap::redefine_error(99));
        }
        let paths: Vec<PathBuf> = loaded.iter().map(|(_, _, f)| f.clone()).collect();
        let bytes = match dexer {
            Some(dexer) => dexer.dex(&paths)?,
            None => paths.iter().map(|f| files.read(f).map_err(|e| format!("{} : {e}", f.display()))).collect::<Result<_, _>>()?,
        };
        let classes: Vec<(u64, Vec<u8>)> = loaded.iter().map(|(id, _, _)| *id).zip(bytes).collect();
        self.vm.redefine_classes(&classes)?;
        // Breakpoints whose file compiled to a reloaded class.
        let mut sources = std::collections::HashSet::new();
        for (id, _, _) in &loaded {
            if let Some(source) = self.vm.source_file(*id)? {
                sources.insert(source);
            }
        }
        let mut state = self.state.lock().unwrap();
        let classes = self.vm.all_classes()?;
        for placed in state.breakpoints.iter_mut().filter(|p| p.breakpoint.field.is_none() && (sources.contains(&p.breakpoint.source_name) || p.breakpoint.implementations.iter().any(|i| sources.contains(&i.source_name)))) {
            for request in placed.requests.drain(..) {
                let _ = self.clear_request(request);
            }
            for class in classes.iter().filter(|c| placed.breakpoint.in_packages(&c.name)) {
                placed.requests.extend(self.place(class.id, &placed.breakpoint)?);
            }
        }
        Ok(loaded.len())
    }

    /// Another suspended thread's frames (the Frames view's thread list).
    pub fn frames(&self, thread: u64) -> Result<Vec<Frame>, String> {
        self.vm.frames(thread)
    }

    /// The Variables view of a frame (by depth).
    pub fn locals(&self, thread: u64, depth: usize) -> Result<Vec<Variable>, String> {
        let frame = self.frame_at(thread, depth)?;
        let mut locals = self.vm.locals(thread, &frame)?;
        // The top frame of a thread stopped at an exception shows it first,
        // as IntelliJ's "Exception" node.
        let thrown = self.state.lock().unwrap().thrown.clone();
        if let Some((_, value)) = thrown.filter(|(t, _)| *t == thread && depth == 0) {
            locals.insert(0, Variable { name: "Exception".into(), value });
        }
        Ok(locals)
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
        let nested = set.events.iter().all(|e| matches!(e, DebugEvent::Breakpoint { .. } | DebugEvent::Field { .. } | DebugEvent::MethodExit { .. } | DebugEvent::Step { .. } | DebugEvent::Exception { .. }));
        if nested && !set.events.is_empty() && self.evaluating.load(Ordering::SeqCst) > 0 {
            self.vm.resume()?;
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        let mut state = self.state.lock().unwrap();
        let mut resume = false;
        let mut paused: Option<(u64, PauseReason)> = None;
        let mut hit: Option<LineBreakpoint> = None;
        let mut field_hit: Option<FieldHit> = None;
        // An exception breakpoint's hit: thread, exception, its class name,
        // the class catching it, the class it's thrown in, and the
        // breakpoint.
        let mut thrown: Option<(u64, u64, String, Option<u64>, u64, Option<ExceptionBreakpoint>)> = None;
        let muted = self.muted.load(Ordering::SeqCst);
        for event in set.events {
            match event {
                DebugEvent::VmDeath => {
                    state.ended = true;
                    state.paused = None;
                    state.thrown = None;
                    out.push(SessionEvent::Ended);
                    return Ok(out);
                }
                DebugEvent::ClassPrepare { class, .. } => {
                    for ix in 0..state.breakpoints.len() {
                        if !state.breakpoints[ix].breakpoint.in_packages(&class.name) {
                            continue;
                        }
                        let placed = self.place(class.id, &state.breakpoints[ix].breakpoint)?;
                        let entry = &mut state.breakpoints[ix];
                        if entry.requests.is_empty() && !placed.is_empty() {
                            out.push(SessionEvent::Verified { path: entry.breakpoint.path.clone(), line: entry.breakpoint.line });
                        }
                        entry.requests.extend(placed);
                    }
                    // An exception breakpoint waiting for its class.
                    for ix in 0..state.exceptions.len() {
                        let e = &state.exceptions[ix];
                        if e.request.is_none() && e.breakpoint.class == class.name {
                            let request = self.vm.set_exception_breakpoint(Some(class.id), e.breakpoint.caught, e.breakpoint.uncaught)?;
                            state.exceptions[ix].request = Some(request);
                        }
                    }
                    resume = set.suspended != Suspend::None;
                }
                DebugEvent::Breakpoint { request, .. } | DebugEvent::Field { request, .. } if muted && !state.run_to.contains(&request) => {
                    resume = set.suspended != Suspend::None;
                }
                // Another method of the class returning, or muted: run on.
                DebugEvent::MethodExit { request, location, .. } if muted || !self.exits.lock().unwrap().get(&request).is_some_and(|m| m.contains(&location.method)) => {
                    resume = set.suspended != Suspend::None;
                }
                DebugEvent::Exception { .. } if muted => resume = set.suspended != Suspend::None,
                DebugEvent::Breakpoint { thread, request, .. } => {
                    let reason = if state.run_to.contains(&request) { PauseReason::Paused } else { PauseReason::Breakpoint };
                    if reason == PauseReason::Breakpoint {
                        hit = state.breakpoints.iter().find(|p| p.requests.contains(&request)).map(|p| p.breakpoint.clone());
                    }
                    paused = Some((thread, reason));
                }
                DebugEvent::Field { thread, request, access, class, object, to_be, .. } => {
                    hit = state.breakpoints.iter().find(|p| p.requests.contains(&request)).map(|p| p.breakpoint.clone());
                    field_hit = Some(FieldHit { access, class, object, to_be });
                    paused = Some((thread, PauseReason::Breakpoint));
                }
                DebugEvent::MethodExit { thread, request, .. } => {
                    hit = state.breakpoints.iter().find(|p| p.requests.contains(&request)).map(|p| p.breakpoint.clone());
                    paused = Some((thread, PauseReason::Breakpoint));
                }
                DebugEvent::Step { thread, .. } => paused = Some((thread, PauseReason::Step)),
                DebugEvent::Exception { thread, location, exception, catch, request } => {
                    let caught = catch.map(|c| c.class);
                    let name = self.vm.class_of(exception).map(|c| c.name).unwrap_or_else(|_| "exception".into());
                    let breakpoint = state.exceptions.iter().find(|e| e.request == Some(request)).map(|e| e.breakpoint.clone());
                    thrown = Some((thread, exception, name, caught, location.class, breakpoint));
                    paused = Some((thread, PauseReason::Exception));
                }
                // A VM launched to wait starts once the session calls start().
                DebugEvent::VmStart { .. } | DebugEvent::ThreadStart { .. } | DebugEvent::ThreadDeath { .. } => {}
            }
        }
        if let (Some((_, PauseReason::Exception)), Some((thread, exception, name, caught, class, breakpoint))) = (paused, thrown) {
            let value = Value::Object { id: exception, class: name.clone() };
            if let Some(breakpoint) = breakpoint.filter(|b| b.condition.is_some() || !b.filters.is_empty() || !b.catch_filters.is_empty()) {
                drop(state);
                let catching = match caught {
                    Some(catcher) if !breakpoint.catch_filters.is_empty() => passes_filters(&breakpoint.catch_filters, &self.vm.class_name(catcher)?),
                    _ => true,
                };
                if !catching || !breakpoint.filters.is_empty() && !passes_filters(&breakpoint.filters, &self.vm.class_name(class)?) {
                    self.vm.resume()?;
                    return Ok(out);
                }
                if let Some(text) = breakpoint.condition.as_deref() {
                    let frames = self.vm.frames(thread)?;
                    match self.in_frame_as(thread, frames.first(), Some(value.clone()), |scope| evaluate::condition_holds(scope, text)) {
                        Some(Ok(false)) => {
                            self.vm.resume()?;
                            return Ok(out);
                        }
                        Some(Ok(true)) | None => {}
                        Some(Err(e)) => out.push(SessionEvent::Message(format!("Failed to evaluate breakpoint condition '{text}'. Reason: {e}"))),
                    }
                }
                state = self.state.lock().unwrap();
            }
            state.thrown = Some((thread, value));
            out.push(SessionEvent::Exception(format!("Exception '{name}' {}", if caught.is_some() { "occurred" } else { "was not caught" })));
        }
        if let (Some((thread, PauseReason::Breakpoint)), Some(breakpoint)) = (paused, hit) {
            let key = (breakpoint.path.clone(), breakpoint.line);
            // A dependent breakpoint before its master's hit: runs on.
            if breakpoint.actions.master.is_some() && !state.armed.contains(&key) {
                drop(state);
                self.vm.resume()?;
                return Ok(out);
            }
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
            // A hit that counts: arms the breakpoints waiting for it, and
            // a dependent one waits for its master again.
            {
                let mut state = self.state.lock().unwrap();
                let dependents: Vec<(PathBuf, u32)> = state.breakpoints.iter().filter(|p| p.breakpoint.actions.master.as_ref() == Some(&key)).map(|p| (p.breakpoint.path.clone(), p.breakpoint.line)).collect();
                if breakpoint.actions.master.is_some() && !breakpoint.actions.leave_enabled {
                    state.armed.remove(&key);
                }
                state.armed.extend(dependents);
            }
            if breakpoint.actions.remove_once_hit {
                self.remove_breakpoint(&breakpoint.path, breakpoint.line)?;
                out.push(SessionEvent::Removed { path: breakpoint.path.clone(), line: breakpoint.line });
            }
            if breakpoint.actions.logs() {
                out.push(SessionEvent::Log(self.hit_log(thread, &frames, &breakpoint, field_hit.as_ref())));
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
        self.in_frame_as(thread, frame, None, f)
    }

    /// [`Self::in_frame`], with `this` standing for another value: the
    /// exception, for an exception breakpoint's condition.
    fn in_frame_as<T>(&self, thread: u64, frame: Option<&Frame>, this: Option<Value>, f: impl FnOnce(&Scope) -> Result<T, String>) -> Option<Result<T, String>> {
        let frame = frame?;
        let mut locals = match self.vm.locals(thread, frame) {
            Ok(locals) => locals,
            Err(e) => return Some(Err(e)),
        };
        if let Some(this) = this {
            locals.retain(|v| v.name != "this");
            locals.insert(0, Variable { name: "this".into(), value: this });
        }
        self.evaluating.fetch_add(1, Ordering::SeqCst);
        let result = f(&Scope { vm: &*self.vm, thread, frame, locals: &locals });
        self.evaluating.fetch_sub(1, Ordering::SeqCst);
        Some(result)
    }

    /// What a logging breakpoint prints, in IntelliJ's words: "Breakpoint
    /// reached at a.CalcTest.adds(CalcTest.java:12)", the stack, the
    /// expression's value.
    /// A field watchpoint's message says what the field is and, for a write, what
    /// it will be: "Field watchpoint reached at …: 'calls' changes from 0
    /// to 1".
    fn hit_log(&self, thread: u64, frames: &[Frame], breakpoint: &LineBreakpoint, field: Option<&FieldHit>) -> String {
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
            match (field, &breakpoint.field) {
                (Some(hit), Some(watch)) => {
                    let now = match self.vm.field(hit.class, hit.object, &watch.name) {
                        Ok(Some(v)) => self.shown(thread, &v),
                        Ok(None) => "?".into(),
                        Err(e) => e,
                    };
                    let name = &watch.name;
                    lines.push(match &hit.to_be {
                        Some(to_be) if !hit.access => format!("Field watchpoint reached at {at}: '{name}' changes from {now} to {}", self.shown(thread, to_be)),
                        _ => format!("Field watchpoint reached at {at}: '{name}' is read ({now})"),
                    });
                }
                _ => lines.push(format!("Breakpoint reached at {at}")),
            }
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

    /// A value as a log shows it: a string quoted, an object's
    /// `toString()`.
    fn shown(&self, thread: u64, value: &Value) -> String {
        let string = match value {
            Value::String { .. } => true,
            Value::Object { id, class } if class.is_empty() => self.vm.class_of(*id).is_ok_and(|c| c.name == "java.lang.String"),
            _ => false,
        };
        match evaluate::as_string(&*self.vm, thread, value) {
            Ok(text) if string => format!("\"{text}\""),
            Ok(text) => text,
            Err(e) => e,
        }
    }
}

/// A field watchpoint's event: read or written, the field's class and
/// object (none: static), the value to be.
struct FieldHit {
    access: bool,
    class: u64,
    object: Option<u64>,
    to_be: Option<Value>,
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
        fn set_exception_breakpoint(&self, class: Option<u64>, caught: bool, uncaught: bool) -> Result<u32, String> {
            Ok(self.log(format!("exception {class:?} {caught} {uncaught}")))
        }
        fn field_id(&self, _: u64, name: &str) -> Result<Option<u64>, String> {
            Ok((name == "total").then_some(77))
        }
        fn set_field_watch(&self, class: u64, field: u64, access: bool) -> Result<u32, String> {
            Ok(self.log(format!("watch {class} {field} {access}")))
        }
        fn clear_field_watch(&self, request: u32, access: bool) -> Result<(), String> {
            self.log(format!("unwatch {request} {access}"));
            Ok(())
        }
        fn set_method_exit(&self, class: u64) -> Result<u32, String> {
            Ok(self.log(format!("exit {class}")))
        }
        fn clear_method_exit(&self, request: u32) -> Result<(), String> {
            self.log(format!("clear exit {request}"));
            Ok(())
        }
        fn clear_exception_breakpoint(&self, request: u32) -> Result<(), String> {
            self.log(format!("clear exception {request}"));
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
        fn find_class(&self, name: &str) -> Result<Option<ClassRef>, String> {
            Ok(self.loaded.lock().unwrap().iter().find(|c| c.name == name).cloned())
        }
        fn class_of(&self, _: u64) -> Result<ClassRef, String> {
            Ok(ClassRef { id: 10, name: "a.CalcTest".into() })
        }
        fn class_name(&self, class: u64) -> Result<String, String> {
            Ok(if class == 11 { "b.Handler" } else { "a.CalcTest" }.into())
        }
        fn superclass(&self, _: u64) -> Result<u64, String> {
            Ok(0)
        }
        fn field(&self, _: u64, _: Option<u64>, name: &str) -> Result<Option<Value>, String> {
            Ok((name == "total").then_some(Value::Int(4)))
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
        fn new_instance(&self, _: u64, _: u64, _: u64, _: &[Value]) -> Result<Value, String> {
            Err("not here".into())
        }
        fn interfaces(&self, _: u64) -> Result<Vec<u64>, String> {
            Ok(Vec::new())
        }
        fn class_object(&self, _: u64) -> Result<Value, String> {
            Ok(Value::Null)
        }
        fn variable_type(&self, _: u64, _: &Frame, _: &str) -> Result<Option<String>, String> {
            Ok(None)
        }
        fn set_variable(&self, _: u64, _: &Frame, _: &str, _: &Value) -> Result<(), String> {
            Err("not here".into())
        }
        fn field_type(&self, _: u64, _: &str) -> Result<Option<String>, String> {
            Ok(None)
        }
        fn set_field(&self, _: u64, _: Option<u64>, _: &str, _: &Value) -> Result<(), String> {
            Err("not here".into())
        }
        fn set_array_element(&self, _: u64, _: usize, _: &Value) -> Result<(), String> {
            Err("not here".into())
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
        fn can_redefine(&self) -> Result<bool, String> {
            Ok(true)
        }
        fn redefine_classes(&self, classes: &[(u64, Vec<u8>)]) -> Result<(), String> {
            self.log(format!("redefine {:?}", classes.iter().map(|(c, b)| (*c, b.len())).collect::<Vec<_>>()));
            Ok(())
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

    struct Files;
    impl ClassFiles for Files {
        fn changed_since(&self, _: &[PathBuf], _: std::time::SystemTime) -> Vec<(String, PathBuf)> {
            Vec::new()
        }
        fn read(&self, file: &Path) -> Result<Vec<u8>, String> {
            Ok(file.to_string_lossy().as_bytes().to_vec())
        }
    }

    #[test]
    fn hot_swap_reloads_loaded_classes_and_places_their_breakpoints_again() {
        let vm = vm();
        vm.loaded.lock().unwrap().push(ClassRef { id: 10, name: "a.CalcTest".into() });
        let session = DebugSession::new(vm.clone());
        assert_eq!(session.add_breakpoint(breakpoint(5)), Ok(true));
        vm.take_log();
        let changed = [("a.CalcTest".to_owned(), PathBuf::from("a/CalcTest.class")), ("a.NotLoaded".to_owned(), PathBuf::from("a/NotLoaded.class"))];
        assert_eq!(session.hot_swap(&changed, &Files, None), HotSwapOutcome::Reloaded(1));
        assert_eq!(vm.take_log(), ["redefine [(10, 16)]", "clear 2", "break 10:1:4"]);
        // Nothing of it loaded: up to date.
        assert_eq!(session.hot_swap(&changed[1..], &Files, None), HotSwapOutcome::UpToDate);
        assert!(vm.take_log().is_empty());
    }

    #[test]
    fn abstract_method_breakpoints_stop_in_implementations() {
        let vm = vm();
        let session = DebugSession::new(vm.clone());
        let mut iface = LineBreakpoint::new(Path::new("/p/src/main/java/b/Shape.java"), "b".into(), 3);
        iface.method = Some(MethodHeader { name: "m1".into(), line: 3, end: 3 });
        iface.implementations = vec![Implementation { source_name: "CalcTest.java".into(), package: "a".into(), header: MethodHeader { name: "m1".into(), line: 4, end: 5 } }];
        // Both packages watched.
        assert_eq!(session.add_breakpoint(iface), Ok(false));
        assert_eq!(vm.take_log(), ["watch b.*", "watch a.*"]);
        session.start().unwrap();
        vm.take_log();
        let prepare = |id, name: &str| DebugEvent::ClassPrepare { thread: 1, class: ClassRef { id, name: name.into() }, request: 1 };
        vm.send(Suspend::All, vec![prepare(10, "a.CalcTest")]);
        assert_eq!(session.poll(Duration::ZERO), Ok(vec![SessionEvent::Verified { path: "/p/src/main/java/b/Shape.java".into(), line: 3 }]));
        assert_eq!(vm.take_log(), ["break 10:1:0", "resume"]);
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
    fn method_exit_stops_only_when_its_method_returns() {
        let vm = vm();
        vm.loaded.lock().unwrap().push(ClassRef { id: 10, name: "a.CalcTest".into() });
        let session = DebugSession::new(vm.clone());
        let mut exit = breakpoint(4);
        exit.method = Some(MethodHeader { name: "m1".into(), line: 4, end: 5 });
        (exit.method_entry, exit.method_exit) = (false, true);
        assert_eq!(session.add_breakpoint(exit), Ok(true));
        let log = vm.take_log();
        assert_eq!(&log[1..], ["exit 10"]);
        let request = vm.log("probe".into()) - 1;
        vm.take_log();
        let returning = |method| vec![DebugEvent::MethodExit { thread: 1, location: Location { class: 10, method, index: 9 }, request }];
        // Another method of the class returns: it runs on.
        vm.send(Suspend::All, returning(2));
        assert_eq!(session.poll(Duration::ZERO), Ok(vec![]));
        assert_eq!(vm.take_log(), ["resume"]);
        vm.send(Suspend::All, returning(1));
        let events = session.poll(Duration::ZERO).unwrap();
        assert!(matches!(events.last(), Some(SessionEvent::Paused { reason: PauseReason::Breakpoint, .. })), "{events:?}");
        vm.take_log();
        session.remove_breakpoint(Path::new("/p/src/test/java/a/CalcTest.java"), 4).unwrap();
        assert_eq!(vm.take_log(), [format!("clear exit {request}")]);
    }

    #[test]
    fn field_watchpoints_stop_where_the_field_changes() {
        let vm = vm();
        vm.loaded.lock().unwrap().push(ClassRef { id: 10, name: "a.CalcTest".into() });
        let session = DebugSession::new(vm.clone());
        let mut watch = breakpoint(3);
        watch.field = Some(FieldWatch { name: "total".into(), access: true, modification: true });
        assert_eq!(session.add_breakpoint(watch), Ok(true));
        let log = vm.take_log();
        assert_eq!(&log[1..], ["watch 10 77 false", "watch 10 77 true"]);
        let request = vm.log("probe".into()) - 1;
        vm.take_log();
        let field = |access, to_be| vec![DebugEvent::Field { thread: 1, location: Location { class: 10, method: 1, index: 4 }, access, class: 10, object: Some(5), to_be, request }];
        vm.send(Suspend::All, field(false, Some(Value::Int(9))));
        let events = session.poll(Duration::ZERO).unwrap();
        assert!(matches!(events.last(), Some(SessionEvent::Paused { reason: PauseReason::Breakpoint, .. })), "{events:?}");
        // Logging: the value now and the value to be.
        let mut logging = breakpoint(3);
        logging.field = Some(FieldWatch { name: "total".into(), access: true, modification: true });
        logging.actions = BreakpointActions { suspend: false, log_message: true, ..BreakpointActions::default() };
        session.remove_breakpoint(Path::new("/p/src/test/java/a/CalcTest.java"), 3).unwrap();
        session.add_breakpoint(logging).unwrap();
        let request = vm.log("probe".into()) - 1;
        vm.take_log();
        let logged = |events: Vec<SessionEvent>| events.into_iter().find_map(|e| if let SessionEvent::Log(l) = e { Some(l) } else { None }).unwrap_or_default();
        let field = |access, to_be| vec![DebugEvent::Field { thread: 1, location: Location { class: 10, method: 1, index: 4 }, access, class: 10, object: None, to_be, request }];
        vm.send(Suspend::All, field(false, Some(Value::Int(9))));
        assert!(logged(session.poll(Duration::ZERO).unwrap()).ends_with(": 'total' changes from 4 to 9"));
        vm.send(Suspend::All, field(true, None));
        assert!(logged(session.poll(Duration::ZERO).unwrap()).ends_with(": 'total' is read (4)"));
        session.remove_breakpoint(Path::new("/p/src/test/java/a/CalcTest.java"), 3).unwrap();
        assert_eq!(vm.take_log().iter().filter(|l| l.starts_with("unwatch")).count(), 4);
    }

    #[test]
    fn muted_breakpoints_run_on() {
        let vm = vm();
        vm.loaded.lock().unwrap().push(ClassRef { id: 10, name: "a.CalcTest".into() });
        let session = DebugSession::new(vm.clone());
        session.add_breakpoint(breakpoint(5)).unwrap();
        let request = vm.log("probe".into()) - 1;
        vm.take_log();
        let hit = DebugEvent::Breakpoint { thread: 1, location: Location { class: 10, method: 1, index: 4 }, request };
        session.set_muted(true);
        vm.send(Suspend::All, vec![hit.clone()]);
        assert_eq!(session.poll(Duration::ZERO), Ok(vec![]));
        assert_eq!(vm.take_log(), ["resume"]);
        session.set_muted(false);
        vm.send(Suspend::All, vec![hit]);
        assert!(matches!(session.poll(Duration::ZERO).unwrap().last(), Some(SessionEvent::Paused { .. })));
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
        breakpoint.actions = BreakpointActions { suspend: false, log_message: true, log_stack: true, log_expression: Some("x + 1".into()), ..BreakpointActions::default() };
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
    fn remove_once_hit_goes_at_the_first_hit_that_counts() {
        let vm = vm();
        vm.loaded.lock().unwrap().push(ClassRef { id: 10, name: "a.CalcTest".into() });
        let session = DebugSession::new(vm.clone());
        let mut breakpoint = breakpoint(5);
        breakpoint.condition = Some("x > 5".into());
        breakpoint.actions.remove_once_hit = true;
        session.add_breakpoint(breakpoint.clone()).unwrap();
        let request = *vm.next.lock().unwrap();
        vm.take_log();
        let hit = || vec![DebugEvent::Breakpoint { thread: 1, location: Location { class: 10, method: 1, index: 4 }, request }];
        // A false condition isn't a hit: it stays.
        vm.send(Suspend::All, hit());
        assert_eq!(session.poll(Duration::ZERO), Ok(vec![]));
        assert_eq!(vm.take_log(), ["resume"]);
        // Holding, it stops once and is cleared.
        session.set_condition(&breakpoint.path, 5, None);
        vm.send(Suspend::All, hit());
        let events = session.poll(Duration::ZERO).unwrap();
        assert_eq!(events[0], SessionEvent::Removed { path: breakpoint.path.clone(), line: 5 });
        assert!(matches!(events[1], SessionEvent::Paused { reason: PauseReason::Breakpoint, .. }), "{events:?}");
        assert_eq!(vm.take_log()[0], format!("clear {request}"));
    }

    #[test]
    fn dependent_breakpoints_wait_for_their_master() {
        let vm = vm();
        vm.loaded.lock().unwrap().push(ClassRef { id: 10, name: "a.CalcTest".into() });
        let session = DebugSession::new(vm.clone());
        let master = breakpoint(4);
        session.add_breakpoint(master.clone()).unwrap();
        let master_request = *vm.next.lock().unwrap();
        let mut dependent = breakpoint(5);
        dependent.actions.master = Some((master.path.clone(), 4));
        session.add_breakpoint(dependent.clone()).unwrap();
        let dependent_request = *vm.next.lock().unwrap();
        vm.take_log();
        let hit = |index, request| vec![DebugEvent::Breakpoint { thread: 1, location: Location { class: 10, method: 1, index }, request }];
        let paused = |events: Vec<SessionEvent>| events.iter().any(|e| matches!(e, SessionEvent::Paused { .. }));
        // Before the master: runs on.
        vm.send(Suspend::All, hit(4, dependent_request));
        assert_eq!(session.poll(Duration::ZERO), Ok(vec![]));
        assert_eq!(vm.take_log(), ["resume"]);
        vm.send(Suspend::All, hit(0, master_request));
        assert!(paused(session.poll(Duration::ZERO).unwrap()));
        vm.send(Suspend::All, hit(4, dependent_request));
        assert!(paused(session.poll(Duration::ZERO).unwrap()));
        // Disable again: waits for the master once more.
        vm.take_log();
        vm.send(Suspend::All, hit(4, dependent_request));
        assert_eq!(session.poll(Duration::ZERO), Ok(vec![]));
        assert_eq!(vm.take_log(), ["resume"]);
        // Leave enabled: stays armed.
        dependent.actions.leave_enabled = true;
        session.set_actions(&dependent.path, 5, dependent.actions.clone());
        vm.send(Suspend::All, hit(0, master_request));
        session.poll(Duration::ZERO).unwrap();
        for _ in 0..2 {
            vm.send(Suspend::All, hit(4, dependent_request));
            assert!(paused(session.poll(Duration::ZERO).unwrap()));
        }
    }

    #[test]
    fn exception_breakpoints() {
        let vm = vm();
        let session = DebugSession::new(vm.clone());
        let any = ExceptionBreakpoint { class: String::new(), caught: true, uncaught: true, ..Default::default() };
        assert_eq!(session.add_exception_breakpoint(any), Ok(true));
        assert_eq!(vm.take_log(), ["exception None true true"]);
        // A class not loaded yet waits for it.
        let boom = ExceptionBreakpoint { class: "a.Boom".into(), caught: false, uncaught: true, ..Default::default() };
        assert_eq!(session.add_exception_breakpoint(boom), Ok(false));
        assert_eq!(vm.take_log(), ["watch a.Boom"]);
        vm.send(Suspend::All, vec![DebugEvent::ClassPrepare { thread: 1, class: ClassRef { id: 30, name: "a.Boom".into() }, request: 2 }]);
        session.poll(Duration::ZERO).unwrap();
        assert_eq!(vm.take_log(), ["exception Some(30) false true", "resume"]);
        // A hit: which exception, then paused.
        vm.send(Suspend::All, vec![DebugEvent::Exception { thread: 1, location: Location { class: 10, method: 1, index: 4 }, exception: 50, catch: None, request: 3 }]);
        let events = session.poll(Duration::ZERO).unwrap();
        assert_eq!(events[0], SessionEvent::Exception("Exception 'a.CalcTest' was not caught".into()));
        assert!(matches!(events[1], SessionEvent::Paused { reason: PauseReason::Exception, .. }));
        // The exception first in the top frame's variables, until resumed.
        assert_eq!(session.locals(1, 0).unwrap()[0], Variable { name: "Exception".into(), value: Value::Object { id: 50, class: "a.CalcTest".into() } });
        session.resume().unwrap();
        assert_eq!(session.locals(1, 0).unwrap()[0].name, "x");
        // Removed: its request cleared.
        session.remove_exception_breakpoint("").unwrap();
        assert_eq!(vm.take_log(), ["resume", "clear exception 1"]);
    }

    #[test]
    fn exception_breakpoints_filter_by_class_and_condition() {
        let vm = vm();
        let session = DebugSession::new(vm.clone());
        let any = ExceptionBreakpoint { caught: true, uncaught: true, filters: vec!["b.*".into()], ..Default::default() };
        session.add_exception_breakpoint(any.clone()).unwrap();
        let request = *vm.next.lock().unwrap();
        vm.take_log();
        let throw = || vec![DebugEvent::Exception { thread: 1, location: Location { class: 10, method: 1, index: 4 }, exception: 50, catch: Some(Location { class: 10, method: 1, index: 9 }), request }];
        // Thrown in a.CalcTest, not b.*: runs on.
        vm.send(Suspend::All, throw());
        assert_eq!(session.poll(Duration::ZERO), Ok(vec![]));
        assert_eq!(vm.take_log(), ["resume"]);
        // Allowed by the filters, but the condition doesn't hold.
        session.add_exception_breakpoint(ExceptionBreakpoint { filters: vec!["a.*".into(), "-a.Other".into()], condition: Some("x > 5".into()), ..any.clone() }).unwrap();
        let request = *vm.next.lock().unwrap();
        vm.take_log();
        let throw = || vec![DebugEvent::Exception { thread: 1, location: Location { class: 10, method: 1, index: 4 }, exception: 50, catch: Some(Location { class: 10, method: 1, index: 9 }), request }];
        vm.send(Suspend::All, throw());
        assert_eq!(session.poll(Duration::ZERO), Ok(vec![]));
        // `this` is the exception.
        session.add_exception_breakpoint(ExceptionBreakpoint { condition: Some("this != null".into()), filters: Vec::new(), ..any }).unwrap();
        let request = *vm.next.lock().unwrap();
        let throw = vec![DebugEvent::Exception { thread: 1, location: Location { class: 10, method: 1, index: 4 }, exception: 50, catch: Some(Location { class: 10, method: 1, index: 9 }), request }];
        vm.send(Suspend::All, throw);
        let events = session.poll(Duration::ZERO).unwrap();
        assert!(matches!(events[..], [SessionEvent::Exception(_), SessionEvent::Paused { reason: PauseReason::Exception, .. }]), "{events:?}");
    }

    #[test]
    fn exception_breakpoints_filter_by_the_catching_class() {
        let vm = vm();
        let session = DebugSession::new(vm.clone());
        // Caught in class 11 (b.Handler), thrown in 10 (a.CalcTest).
        let caught = |request| vec![DebugEvent::Exception { thread: 1, location: Location { class: 10, method: 1, index: 4 }, exception: 50, catch: Some(Location { class: 11, method: 2, index: 0 }), request }];
        let uncaught = |request| vec![DebugEvent::Exception { thread: 1, location: Location { class: 10, method: 1, index: 4 }, exception: 50, catch: None, request }];
        session.add_exception_breakpoint(ExceptionBreakpoint { caught: true, uncaught: true, catch_filters: vec!["a.*".into()], ..Default::default() }).unwrap();
        let request = *vm.next.lock().unwrap();
        vm.take_log();
        // Caught outside a.*: runs on.
        vm.send(Suspend::All, caught(request));
        assert_eq!(session.poll(Duration::ZERO), Ok(vec![]));
        assert_eq!(vm.take_log(), ["resume"]);
        // Uncaught: stops.
        vm.send(Suspend::All, uncaught(request));
        assert!(matches!(session.poll(Duration::ZERO).unwrap()[..], [SessionEvent::Exception(_), ..]));
        session.resume().unwrap();
        // Caught in b.*: stops.
        session.add_exception_breakpoint(ExceptionBreakpoint { caught: true, uncaught: true, catch_filters: vec!["b.*".into()], ..Default::default() }).unwrap();
        let request = *vm.next.lock().unwrap();
        vm.send(Suspend::All, caught(request));
        assert!(matches!(session.poll(Duration::ZERO).unwrap()[..], [SessionEvent::Exception(_), ..]));
    }

    #[test]
    fn class_filters() {
        let f = |list: &[&str], class| passes_filters(&list.iter().map(|s| s.to_string()).collect::<Vec<_>>(), class);
        assert!(f(&[], "a.B"));
        assert!(f(&["a.*"], "a.B") && !f(&["a.*"], "b.B"));
        assert!(f(&["*.B"], "x.y.B") && !f(&["*.B"], "x.y.C"));
        assert!(f(&["a.B"], "a.B") && !f(&["a.B"], "a.BC"));
        assert!(!f(&["a.*", "-a.Internal*"], "a.InternalX") && f(&["-a.Internal"], "b.C"));
        assert!(f(&["com.*.impl.*"], "com.x.impl.Y"));
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
