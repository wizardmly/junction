//! Debug, as Android Studio's: line breakpoints (Ctrl+F8, a click on the
//! line numbers), kept across sessions; Debug on a test runs it through
//! Gradle with its JVM waiting for the debugger, attaches over JDWP and
//! sets the breakpoints; the Debug window shows the paused thread's frames
//! and variables, and the editors the execution point. Composition root of
//! the debug session use case over the JDWP client.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Global, SharedString, Task};

use crate::domain::debugger::{self, Frame, StepDepth, ThreadInfo, Value, Variable};
use crate::domain::testing::TestMark;
use crate::infra::breakpoint_store::{self, SavedBreakpoint};
use crate::infra::jdwp_client::JdwpVm;
use crate::ui::test_service::TestService;
use crate::usecase::debug_session::{DebugSession, LineBreakpoint, PauseReason, SessionEvent};

/// How long Debug waits for the test JVM to listen (Gradle builds first).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(300);

pub enum DebugServiceEvent {
    /// A debug session started: the Debug window shows it.
    Show,
    /// Paused: the editor goes to the execution point.
    Navigate { path: PathBuf, line: u32 },
    Error { title: String, message: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugStatus {
    /// Waiting for the VM (Gradle builds the tests).
    Connecting,
    Running,
    Paused,
    Ended,
}

/// What a root of the Variables tree is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Variable,
    /// A watch, by its index.
    Watch(usize),
    /// The evaluate field's result.
    Evaluation,
}

/// A row of the Variables tree: a variable, and its children once
/// expanded.
#[derive(Clone, Debug)]
pub struct VarNode {
    pub var: Variable,
    pub kind: NodeKind,
    /// After the value: its `toString()`, a collection's size.
    pub detail: Option<String>,
    /// A watch or evaluation that failed: why.
    pub error: Option<String>,
    pub expanded: bool,
    /// Loaded on first expansion.
    pub children: Option<Vec<VarNode>>,
}

impl VarNode {
    fn new((var, detail): (Variable, Option<String>)) -> Self {
        Self { var, kind: NodeKind::Variable, detail, error: None, expanded: false, children: None }
    }

    /// A watch's or an evaluation's row: its expression, and its value or
    /// error.
    fn expression(text: String, kind: NodeKind, result: Result<(Value, Option<String>), String>) -> Self {
        let (value, detail, error) = match result {
            Ok((value, detail)) => (value, detail, None),
            Err(e) => (Value::Void, None, Some(e)),
        };
        Self { var: Variable { name: text, value }, kind, detail, error, expanded: false, children: None }
    }
}

/// A breakpoint as the editor's gutter draws it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BreakpointMark {
    /// 0-based.
    pub line: u32,
    pub verified: bool,
    pub enabled: bool,
    pub conditional: bool,
}

/// The Debug window's session.
#[derive(Clone)]
pub struct DebugRun {
    /// "CalcTest.adds".
    pub name: String,
    pub status: DebugStatus,
    pub reason: Option<PauseReason>,
    pub threads: Vec<ThreadInfo>,
    pub thread: Option<u64>,
    pub frames: Vec<Frame>,
    /// The selected frame.
    pub frame: usize,
    /// The evaluation's result, the watches, then the frame's variables.
    pub variables: Vec<VarNode>,
    pub variables_loading: bool,
    /// What the session printed about itself: "Connected to the target VM…".
    pub console: Vec<SharedString>,
}

/// Finds a frame's source in the project: the path suffix
/// (`com/example/CalcTest.java`) to a file.
pub type SourceResolver = Rc<dyn Fn(&str, &App) -> Option<PathBuf>>;

#[derive(Default)]
pub struct DebugService {
    breakpoints: Vec<SavedBreakpoint>,
    /// The Watches' expressions.
    pub watches: Vec<String>,
    /// Breakpoints that stop somewhere in the running session.
    verified: HashSet<(String, u32)>,
    pub run: Option<DebugRun>,
    session: Option<Arc<DebugSession>>,
    resolver: Option<SourceResolver>,
    /// The session's generation: a new run makes the old one's tasks moot.
    generation: u64,
    _poll: Option<Task<()>>,
}

impl EventEmitter<DebugServiceEvent> for DebugService {}

struct GlobalDebug(Entity<DebugService>);
impl Global for GlobalDebug {}

/// A file's path as breakpoints key it.
fn key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

impl DebugService {
    pub fn global(cx: &mut App) -> Entity<Self> {
        if let Some(g) = cx.try_global::<GlobalDebug>() {
            return g.0.clone();
        }
        let service = cx.new(|_| Self { breakpoints: breakpoint_store::load(), watches: breakpoint_store::load_watches(), ..Default::default() });
        cx.set_global(GlobalDebug(service.clone()));
        service
    }

    pub fn set_resolver(&mut self, resolver: SourceResolver) {
        self.resolver = Some(resolver);
    }

    // --- Breakpoints ----------------------------------------------------------

    /// A file's breakpoints, as the gutter draws them.
    pub fn breakpoints_of(&self, file: &str) -> Vec<BreakpointMark> {
        self.breakpoints
            .iter()
            .filter(|b| b.file == file)
            .map(|b| BreakpointMark { line: b.line, verified: self.verified.contains(&(b.file.clone(), b.line)), enabled: b.enabled, conditional: b.condition.is_some() })
            .collect()
    }

    /// The breakpoint on a line, for its properties popup.
    pub fn breakpoint(&self, file: &str, line: u32) -> Option<SavedBreakpoint> {
        self.breakpoints.iter().find(|b| b.file == file && b.line == line).cloned()
    }

    /// The breakpoint popup's Enabled and Condition.
    pub fn set_breakpoint_properties(&mut self, file: &str, line: u32, enabled: bool, condition: Option<String>, cx: &mut Context<Self>) {
        let condition = condition.map(|c| c.trim().to_owned()).filter(|c| !c.is_empty());
        let Some(saved) = self.breakpoints.iter_mut().find(|b| b.file == file && b.line == line) else { return };
        let was_enabled = saved.enabled;
        saved.enabled = enabled;
        saved.condition = condition.clone();
        let breakpoint = saved.clone();
        breakpoint_store::save(&self.breakpoints);
        if let Some(session) = self.session.clone().filter(|s| !s.is_ended()) {
            let path = PathBuf::from(file);
            match (was_enabled, enabled) {
                (true, false) => {
                    self.verified.remove(&(file.to_owned(), line));
                    cx.background_spawn(async move { session.remove_breakpoint(&path, line + 1) }).detach();
                }
                (false, true) => self.place_in_session(session, breakpoint, cx),
                _ => session.set_condition(&path, line + 1, condition),
            }
        }
        cx.notify();
    }

    /// Sets a breakpoint in the running session; verified once it stops
    /// somewhere.
    fn place_in_session(&mut self, session: Arc<DebugSession>, saved: SavedBreakpoint, cx: &mut Context<Self>) {
        let generation = self.generation;
        let (file, line) = (saved.file.clone(), saved.line);
        let task = cx.background_spawn(async move { session.add_breakpoint(line_breakpoint(&saved)) });
        cx.spawn(async move |this, cx| {
            if let Ok(true) = task.await {
                this.update(cx, |this, cx| {
                    if this.generation == generation {
                        this.verified.insert((file, line));
                        cx.notify();
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    /// Toggle Line Breakpoint (Ctrl+F8, a click on the line number).
    pub fn toggle_breakpoint(&mut self, file: &str, line: u32, cx: &mut Context<Self>) {
        let session = self.session.clone().filter(|s| !s.is_ended());
        if let Some(ix) = self.breakpoints.iter().position(|b| b.file == file && b.line == line) {
            self.breakpoints.remove(ix);
            self.verified.remove(&(file.to_owned(), line));
            if let Some(session) = session {
                let path = PathBuf::from(file);
                cx.background_spawn(async move { session.remove_breakpoint(&path, line + 1) }).detach();
            }
        } else {
            let saved = SavedBreakpoint::new(file, line);
            self.breakpoints.push(saved.clone());
            if let Some(session) = session {
                self.place_in_session(session, saved, cx);
            }
        }
        breakpoint_store::save(&self.breakpoints);
        cx.notify();
    }

    /// Moves a file's breakpoints when lines above them are added or
    /// removed (an edit of `delta` lines at `at`).
    pub fn shift_breakpoints(&mut self, file: &str, at: u32, delta: i64, cx: &mut Context<Self>) {
        let mut changed = false;
        for b in self.breakpoints.iter_mut().filter(|b| b.file == file && b.line > at) {
            b.line = (b.line as i64 + delta).max(at as i64) as u32;
            changed = true;
        }
        if changed {
            breakpoint_store::save(&self.breakpoints);
            cx.notify();
        }
    }

    // --- Sessions -------------------------------------------------------------

    pub fn is_active(&self) -> bool {
        self.run.as_ref().is_some_and(|r| r.status != DebugStatus::Ended)
    }

    pub fn is_paused(&self) -> bool {
        self.run.as_ref().is_some_and(|r| r.status == DebugStatus::Paused)
    }

    /// Debug a test (the gutter popup's Debug, Ctrl+Shift+F9): runs it with
    /// its JVM waiting for the debugger, and attaches.
    pub fn debug_test(&mut self, path: PathBuf, marks: Vec<TestMark>, mark: &TestMark, cx: &mut Context<Self>) {
        self.end_session(cx);
        let Some(port) = free_port() else {
            cx.emit(DebugServiceEvent::Error { title: format!("Error running '{}'", mark.name()), message: "No free port for the debugger.".into() });
            return;
        };
        let started = TestService::global(cx).update(cx, |s, cx| s.run_mark(path, marks, mark, Some(port), cx));
        if !started {
            return;
        }
        let generation = self.begin(mark.name(), format!("Waiting for the test JVM on port {port}…"), cx);
        let address = format!("127.0.0.1:{port}");
        let tests = TestService::global(cx);
        self._poll = Some(cx.spawn(async move |this, cx| {
            // Gradle builds, then starts the test JVM: try until it listens,
            // the test run ends without it, or Stop.
            let started = Instant::now();
            let vm = loop {
                let attempt = address.clone();
                if let Ok(vm) = cx.background_spawn(async move { JdwpVm::connect(&attempt) }).await {
                    break Some(vm);
                }
                let gone = this.update(cx, |this, _| this.generation != generation).unwrap_or(true);
                let finished = cx.update(|cx| !tests.read(cx).is_running());
                if gone || finished || started.elapsed() > CONNECT_TIMEOUT {
                    break None;
                }
                cx.background_executor().timer(Duration::from_millis(300)).await;
            };
            match vm {
                Some(vm) => drive(this, cx, generation, vm, format!("address: '{address}', transport: 'socket'")).await,
                None => drop(this.update(cx, |this, cx| this.ended(generation, Some("The test JVM did not start."), cx))),
            }
        }));
    }

    /// Debug 'app' and Attach Debugger to Android Process: the process's
    /// JDWP through adb.
    pub fn attach_android(&mut self, name: String, adb: crate::android::adb::Adb, serial: String, pid: u32, cx: &mut Context<Self>) {
        self.end_session(cx);
        let generation = self.begin(name, format!("Connecting to process {pid} on {serial}…"), cx);
        self._poll = Some(cx.spawn(async move |this, cx| {
            let target = serial.clone();
            let vm = cx.background_spawn(async move { crate::android::jdwp::connect(&adb, &target, pid).and_then(JdwpVm::over) }).await;
            match vm {
                Ok(vm) => drive(this, cx, generation, vm, format!("pid {pid}, device '{serial}'")).await,
                Err(e) => drop(this.update(cx, |this, cx| {
                    this.ended(generation, Some(&format!("Unable to connect to process {pid}: {e}")), cx);
                    cx.emit(DebugServiceEvent::Error { title: "Debugger".into(), message: format!("Unable to attach to process {pid}: {e}") });
                })),
            }
        }));
    }

    /// A new session in the Debug window, waiting for its VM.
    fn begin(&mut self, name: String, first_line: String, cx: &mut Context<Self>) -> u64 {
        self.generation += 1;
        self.run = Some(DebugRun {
            name,
            status: DebugStatus::Connecting,
            reason: None,
            threads: Vec::new(),
            thread: None,
            frames: Vec::new(),
            frame: 0,
            variables: Vec::new(),
            variables_loading: false,
            console: vec![first_line.into()],
        });
        cx.emit(DebugServiceEvent::Show);
        cx.notify();
        self.generation
    }

    /// Applies what the session reported; whether to keep polling.
    fn apply(&mut self, generation: u64, events: Result<Vec<SessionEvent>, String>, cx: &mut Context<Self>) -> bool {
        if generation != self.generation {
            return false;
        }
        let events = match events {
            Ok(events) => events,
            Err(e) => {
                self.ended(generation, Some(&e), cx);
                return false;
            }
        };
        let mut go_on = true;
        for event in events {
            match event {
                SessionEvent::Ended => {
                    self.ended(generation, None, cx);
                    go_on = false;
                }
                other => self.show(other, cx),
            }
        }
        go_on
    }

    /// Shows a session event: paused, resumed, a verified breakpoint.
    fn show(&mut self, event: SessionEvent, cx: &mut Context<Self>) {
        match event {
            SessionEvent::Paused { thread, frames, threads, reason } => {
                let Some(run) = self.run.as_mut() else { return };
                run.status = DebugStatus::Paused;
                run.reason = Some(reason);
                run.thread = Some(thread);
                run.threads = threads;
                run.frames = frames;
                run.frame = 0;
                self.load_variables(cx);
                if let Some((path, line)) = self.execution_point(cx) {
                    cx.emit(DebugServiceEvent::Navigate { path, line });
                }
            }
            SessionEvent::Resumed => {
                if let Some(run) = self.run.as_mut() {
                    run.status = DebugStatus::Running;
                    run.reason = None;
                    run.frames.clear();
                    run.variables.clear();
                }
            }
            SessionEvent::Verified { path, line } => {
                self.verified.insert((key(&path), line - 1));
            }
            SessionEvent::Message(text) => {
                if let Some(run) = self.run.as_mut() {
                    run.console.push(text.clone().into());
                }
                cx.emit(DebugServiceEvent::Error { title: "Breakpoint condition".into(), message: text });
            }
            SessionEvent::Ended => {}
        }
        cx.notify();
    }

    fn console(&mut self, generation: u64, line: String) {
        if generation == self.generation {
            if let Some(run) = self.run.as_mut() {
                run.console.push(line.into());
            }
        }
    }

    fn ended(&mut self, generation: u64, error: Option<&str>, cx: &mut Context<Self>) {
        if generation != self.generation {
            return;
        }
        if let Some(run) = self.run.as_mut() {
            if run.status != DebugStatus::Ended {
                if let Some(e) = error {
                    run.console.push(e.to_owned().into());
                }
                run.console.push("Disconnected from the target VM".into());
                run.status = DebugStatus::Ended;
                run.frames.clear();
                run.variables.clear();
                run.threads.clear();
            }
        }
        self.session = None;
        self.verified.clear();
        cx.notify();
    }

    /// Ends the current session, if any, without waiting.
    fn end_session(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.session.take() {
            cx.background_spawn(async move { session.stop() }).detach();
        }
        let generation = self.generation;
        self.ended(generation, None, cx);
        self.generation += 1;
    }

    /// Runs a command on the session in the background, then shows its
    /// result.
    fn command(&mut self, f: impl FnOnce(&DebugSession) -> Result<SessionEvent, String> + Send + 'static, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else { return };
        let generation = self.generation;
        let task = cx.background_spawn(async move { f(&session) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(event) => this.show(event, cx),
                    Err(message) => cx.emit(DebugServiceEvent::Error { title: "Debugger".into(), message }),
                }
            })
            .ok();
        })
        .detach();
    }

    /// Resume Program (F9).
    pub fn resume(&mut self, cx: &mut Context<Self>) {
        if self.is_paused() {
            self.command(|s| s.resume(), cx);
        }
    }

    /// Pause Program.
    pub fn pause(&mut self, cx: &mut Context<Self>) {
        if self.run.as_ref().is_some_and(|r| r.status == DebugStatus::Running) {
            self.command(|s| s.pause(), cx);
        }
    }

    /// Step Over (F8), Step Into (F7), Step Out (Shift+F8).
    pub fn step(&mut self, depth: StepDepth, cx: &mut Context<Self>) {
        if self.is_paused() {
            self.command(move |s| s.step(depth), cx);
        }
    }

    /// Run to Cursor (Alt+F9).
    pub fn run_to_line(&mut self, file: &str, line: u32, cx: &mut Context<Self>) {
        if self.is_paused() {
            let target = line_breakpoint(&SavedBreakpoint::new(file, line));
            self.command(move |s| s.run_to_line(&target), cx);
        }
    }

    /// Stop (Ctrl+F2): ends the debugged JVM, and so the test run.
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if !self.is_active() {
            return;
        }
        let connecting = self.run.as_ref().is_some_and(|r| r.status == DebugStatus::Connecting);
        self.end_session(cx);
        if connecting {
            TestService::global(cx).update(cx, |s, cx| s.stop(cx));
        }
    }

    /// Selects a frame in the Frames view: its variables, its place in the
    /// editor.
    pub fn select_frame(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(run) = self.run.as_mut() else { return };
        if ix >= run.frames.len() || run.frame == ix {
            return;
        }
        run.frame = ix;
        self.load_variables(cx);
        if let Some((path, line)) = self.execution_point(cx) {
            cx.emit(DebugServiceEvent::Navigate { path, line });
        }
        cx.notify();
    }

    /// Picks another thread in the Frames view: its frames.
    pub fn select_thread(&mut self, thread: u64, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else { return };
        if !self.is_paused() || self.run.as_ref().and_then(|r| r.thread) == Some(thread) {
            return;
        }
        let generation = self.generation;
        let task = cx.background_spawn(async move { session.frames(thread) });
        cx.spawn(async move |this, cx| {
            let frames = task.await.unwrap_or_default();
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                let Some(run) = this.run.as_mut() else { return };
                run.thread = Some(thread);
                run.frames = frames;
                run.frame = 0;
                this.load_variables(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The selected frame's watches and variables, each object described
    /// (its `toString()`, a collection's size).
    fn load_variables(&mut self, cx: &mut Context<Self>) {
        self.reload_variables(false, cx);
    }

    /// The variables again; a changed watch list keeps the evaluation's
    /// result, a new pause or frame doesn't.
    fn reload_variables(&mut self, keep_evaluation: bool, cx: &mut Context<Self>) {
        let (Some(session), Some(run)) = (self.session.clone(), self.run.as_mut()) else { return };
        let (Some(thread), true) = (run.thread, run.frame < run.frames.len()) else { return };
        let depth = run.frame;
        run.variables.retain(|n| keep_evaluation && n.kind == NodeKind::Evaluation);
        run.variables_loading = true;
        let generation = self.generation;
        let selected = run.frame;
        let watches = self.watches.clone();
        let task = cx.background_spawn(async move {
            let described = |value: Value| {
                let detail = session.describe(thread, &value);
                (value, detail)
            };
            let watches: Vec<VarNode> = watches
                .into_iter()
                .enumerate()
                .map(|(ix, text)| {
                    let result = session.evaluate(thread, depth, &text).map(described);
                    VarNode::expression(text, NodeKind::Watch(ix), result)
                })
                .collect();
            let locals = session.locals(thread, depth).map(|vars| {
                vars.into_iter()
                    .map(|v| {
                        let detail = session.describe(thread, &v.value);
                        VarNode::new((v, detail))
                    })
                    .collect::<Vec<_>>()
            });
            (watches, locals)
        });
        cx.spawn(async move |this, cx| {
            let (watches, locals) = task.await;
            this.update(cx, |this, cx| {
                let Some(run) = this.run.as_mut().filter(|r| r.frame == selected) else { return };
                if this.generation != generation {
                    return;
                }
                run.variables_loading = false;
                // An evaluation's result stays on top.
                let evaluation: Vec<VarNode> = run.variables.drain(..).filter(|n| n.kind == NodeKind::Evaluation).collect();
                run.variables = evaluation;
                run.variables.extend(watches);
                match locals {
                    Ok(locals) => run.variables.extend(locals),
                    Err(e) => run.console.push(format!("Variables are not available: {e}").into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The evaluate field (Enter): the expression's value on top of the
    /// Variables view, in the selected frame.
    pub fn evaluate(&mut self, text: String, cx: &mut Context<Self>) {
        let (Some(session), Some(run)) = (self.session.clone(), self.run.as_ref()) else { return };
        let (Some(thread), true) = (run.thread.filter(|_| run.status == DebugStatus::Paused), run.frame < run.frames.len()) else { return };
        let depth = run.frame;
        if text.trim().is_empty() {
            return;
        }
        let generation = self.generation;
        let expression = text.clone();
        let task = cx.background_spawn(async move {
            session.evaluate(thread, depth, &expression).map(|value| {
                let detail = session.describe(thread, &value);
                (value, detail)
            })
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                let Some(run) = this.run.as_mut() else { return };
                run.variables.retain(|n| n.kind != NodeKind::Evaluation);
                run.variables.insert(0, VarNode::expression(text, NodeKind::Evaluation, result));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Add to Watches (Ctrl+Shift+Enter in the evaluate field, or New Watch).
    pub fn add_watch(&mut self, text: String, cx: &mut Context<Self>) {
        let text = text.trim().to_owned();
        if text.is_empty() || self.watches.contains(&text) {
            return;
        }
        self.watches.push(text);
        breakpoint_store::save_watches(&self.watches);
        self.reload_variables(true, cx);
        cx.notify();
    }

    /// Remove Watch.
    pub fn remove_watch(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.watches.len() {
            self.watches.remove(ix);
            breakpoint_store::save_watches(&self.watches);
            self.reload_variables(true, cx);
            cx.notify();
        }
    }

    /// Clears the evaluation's result row.
    pub fn clear_evaluation(&mut self, cx: &mut Context<Self>) {
        if let Some(run) = self.run.as_mut() {
            run.variables.retain(|n| n.kind != NodeKind::Evaluation);
            cx.notify();
        }
    }

    /// Expands or collapses a variable (its path of indexes in the tree);
    /// the first expansion loads its fields, elements or a collection's
    /// items, each described.
    pub fn toggle_variable(&mut self, path: &[usize], cx: &mut Context<Self>) {
        let session = self.session.clone();
        let generation = self.generation;
        let thread = self.run.as_ref().and_then(|r| r.thread);
        let Some(node) = self.run.as_mut().and_then(|r| node_at(&mut r.variables, path)) else { return };
        node.expanded = !node.expanded;
        if node.expanded && node.children.is_none() {
            let (Some(session), Some(thread)) = (session, thread) else { return };
            let value = node.var.value.clone();
            let task = cx.background_spawn(async move {
                session.children(thread, &value).map(|children| {
                    children
                        .into_iter()
                        .map(|v| {
                            let detail = session.describe(thread, &v.value);
                            VarNode::new((v, detail))
                        })
                        .collect::<Vec<_>>()
                })
            });
            let path = path.to_vec();
            cx.spawn(async move |this, cx| {
                let result = task.await;
                this.update(cx, |this, cx| {
                    if this.generation != generation {
                        return;
                    }
                    if let Some(node) = this.run.as_mut().and_then(|r| node_at(&mut r.variables, &path)) {
                        node.children = Some(result.unwrap_or_default());
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        cx.notify();
    }

    /// Where the selected frame is: a project file and 0-based line.
    pub fn execution_point(&self, cx: &App) -> Option<(PathBuf, u32)> {
        let run = self.run.as_ref().filter(|r| r.status == DebugStatus::Paused)?;
        let frame = run.frames.get(run.frame)?;
        let source = frame.source.as_deref()?;
        let line = frame.line?.checked_sub(1)?;
        let suffix = debugger::source_path(&frame.class, source);
        // A file with a breakpoint first, then the project's files.
        let path = self
            .breakpoints
            .iter()
            .map(|b| PathBuf::from(&b.file))
            .find(|p| p.to_string_lossy().replace('\\', "/").ends_with(&format!("/{suffix}")))
            .or_else(|| self.resolver.as_ref().and_then(|r| r(&suffix, cx)))?;
        Some((path, line))
    }
}

/// A connected session: sets the breakpoints, lets the VM run, and shows
/// what it reports until it ends.
async fn drive(this: gpui_kit::WeakEntity<DebugService>, cx: &mut gpui_kit::AsyncApp, generation: u64, vm: JdwpVm, target: String) {
    let session = Arc::new(DebugSession::new(Arc::new(vm)));
    let Ok(breakpoints) = this.update(cx, |this, cx| {
        this.session = Some(session.clone());
        if let Some(run) = this.run.as_mut() {
            run.status = DebugStatus::Running;
            run.console.push(format!("Connected to the target VM, {target}").into());
        }
        cx.notify();
        this.breakpoints.clone()
    }) else {
        return;
    };
    let setup = session.clone();
    let (verified, started) = cx
        .background_spawn(async move {
            let mut verified = Vec::new();
            for b in breakpoints.into_iter().filter(|b| b.enabled) {
                if let Ok(true) = setup.add_breakpoint(line_breakpoint(&b)) {
                    verified.push((b.file, b.line));
                }
            }
            let started = setup.start();
            (verified, started)
        })
        .await;
    this.update(cx, |this, cx| {
        this.verified.extend(verified);
        if let Err(e) = started {
            this.console(generation, format!("Could not start the VM: {e}"));
        }
        cx.notify();
    })
    .ok();
    loop {
        let poller = session.clone();
        let events = cx.background_spawn(async move { poller.poll(Duration::from_millis(250)) }).await;
        let Ok(go_on) = this.update(cx, |this, cx| this.apply(generation, events, cx)) else { break };
        if !go_on {
            break;
        }
    }
}

/// The variable at a path of indexes in the tree.
fn node_at<'a>(nodes: &'a mut [VarNode], path: &[usize]) -> Option<&'a mut VarNode> {
    let (first, rest) = path.split_first()?;
    let node = nodes.get_mut(*first)?;
    if rest.is_empty() {
        return Some(node);
    }
    node_at(node.children.as_mut()?, rest)
}

/// A saved breakpoint as the session sets it: the package from the file.
fn line_breakpoint(saved: &SavedBreakpoint) -> LineBreakpoint {
    let text = std::fs::read_to_string(&saved.file).unwrap_or_default();
    let mut breakpoint = LineBreakpoint::new(Path::new(&saved.file), debugger::package_of(&text), saved.line + 1);
    breakpoint.condition = saved.condition.clone();
    breakpoint
}

fn free_port() -> Option<u16> {
    std::net::TcpListener::bind("127.0.0.1:0").ok()?.local_addr().ok().map(|a| a.port())
}
