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
use std::time::{Duration, Instant, SystemTime};

use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Global, SharedString, Task};

use crate::domain::debugger::{self, Frame, StepDepth, ThreadInfo, Value, Variable};
use crate::domain::testing::TestMark;
use crate::infra::breakpoint_store::{self, SavedBreakpoint};
use crate::infra::jdwp_client::JdwpVm;
use crate::ui::test_service::TestService;
use crate::usecase::debug_session::{BreakpointActions, DebugSession, LineBreakpoint, PauseReason, SessionEvent};

/// How long Debug waits for the test JVM to listen (Gradle builds first).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(300);

pub enum DebugServiceEvent {
    /// A debug session started: the Debug window shows it.
    Show,
    /// Paused: the editor goes to the execution point.
    Navigate { path: PathBuf, line: u32 },
    Error { title: String, message: String },
    /// HotSwap finished: IntelliJ's notification.
    HotSwap { message: String, failed: bool },
}

/// The editor's HotSwap toolbar (IntelliJ's "Code changed:").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HotSwapStatus {
    /// Nothing changed since the last reload.
    #[default]
    Hidden,
    /// Sources were saved during the session: "Code changed: Apply HotSwap".
    Offered,
    Running,
    /// "Code has been reloaded", until the next change.
    Reloaded,
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
    pub(crate) fn expression(text: String, kind: NodeKind, result: Result<(Value, Option<String>), String>) -> Self {
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
    /// Off: a logging breakpoint, drawn yellow.
    pub suspend: bool,
    /// A method breakpoint (a diamond) or a field watchpoint (an eye).
    pub kind: crate::ui::breakpoints_dialog::MarkKind,
    /// Mute Breakpoints is on: drawn gray.
    pub muted: bool,
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
    /// "address: '127.0.0.1:5005', transport: 'socket'", once connected.
    pub target: Option<String>,
    /// Paused by an exception breakpoint: which exception.
    pub exception: Option<String>,
}

/// Finds a frame's source in the project: the path suffix
/// (`com/example/CalcTest.java`) to a file.
pub type SourceResolver = Rc<dyn Fn(&str, &App) -> Option<PathBuf>>;

#[derive(Default)]
pub struct DebugService {
    breakpoints: Vec<SavedBreakpoint>,
    /// Java Exception Breakpoints, "Any exception" first.
    exceptions: Vec<breakpoint_store::SavedExceptionBreakpoint>,
    /// The Watches' expressions.
    pub watches: Vec<String>,
    /// Evaluate Expression's history, the latest first.
    pub evaluated: Vec<String>,
    /// Breakpoints that stop somewhere in the running session.
    verified: HashSet<(String, u32)>,
    pub run: Option<DebugRun>,
    session: Option<Arc<DebugSession>>,
    resolver: Option<SourceResolver>,
    /// The session's generation: a new run makes the old one's tasks moot.
    generation: u64,
    /// Mute Breakpoints, kept for the next sessions too.
    muted: bool,
    /// HotSwap: the sources saved during the session, when classes were
    /// last reloaded (the session's start first), whether the VM is an
    /// Android app's (it takes dex).
    changed_sources: Vec<PathBuf>,
    swapped_at: Option<SystemTime>,
    android: bool,
    pub hot_swap: HotSwapStatus,
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
        let service = cx.new(|_| Self { breakpoints: breakpoint_store::load(), exceptions: breakpoint_store::load_exceptions(), watches: breakpoint_store::load_watches(), muted: breakpoint_store::load_muted(), ..Default::default() });
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
            .map(|b| BreakpointMark { line: b.line, verified: self.verified.contains(&(b.file.clone(), b.line)), enabled: b.enabled, conditional: b.condition.is_some(), suspend: b.suspend, kind: crate::ui::breakpoints_dialog::MarkKind::of(b), muted: self.muted })
            .collect()
    }

    /// The breakpoint on a line, for its properties popup.
    pub fn breakpoint(&self, file: &str, line: u32) -> Option<SavedBreakpoint> {
        self.breakpoints.iter().find(|b| b.file == file && b.line == line).cloned()
    }

    /// The breakpoint popup's properties: Enabled, Condition, Suspend
    /// execution and what a hit logs.
    pub fn set_breakpoint_properties(&mut self, updated: SavedBreakpoint, cx: &mut Context<Self>) {
        let trimmed = |text: Option<String>| text.map(|c| c.trim().to_owned()).filter(|c| !c.is_empty());
        let (file, line) = (updated.file.clone(), updated.line);
        let Some(saved) = self.breakpoints.iter_mut().find(|b| b.file == file && b.line == line) else { return };
        let was_enabled = saved.enabled;
        let before = saved.clone();
        *saved = SavedBreakpoint { condition: trimmed(updated.condition), log_expression: trimmed(updated.log_expression), ..updated };
        let breakpoint = saved.clone();
        breakpoint_store::save(&self.breakpoints);
        if let Some(session) = self.session.clone().filter(|s| !s.is_ended()) {
            let path = PathBuf::from(&file);
            match (was_enabled, breakpoint.enabled) {
                (true, false) => {
                    self.verified.remove(&(file, line));
                    cx.background_spawn(async move { session.remove_breakpoint(&path, line + 1) }).detach();
                }
                (false, true) => self.place_in_session(session, breakpoint, cx),
                // A watchpoint's access / modification or a method
                // breakpoint's entry / exit changed: set again.
                (true, true) if (before.field_access, before.field_modification, before.method_entry, before.method_exit) != (breakpoint.field_access, breakpoint.field_modification, breakpoint.method_entry, breakpoint.method_exit) => {
                    self.verified.remove(&(file, line));
                    let task = cx.background_spawn(async move { session.remove_breakpoint(&path, line + 1) });
                    let generation = self.generation;
                    cx.spawn(async move |this, cx| {
                        task.await.ok();
                        this.update(cx, |this, cx| {
                            if let Some(session) = this.session.clone().filter(|s| this.generation == generation && !s.is_ended()) {
                                this.place_in_session(session, breakpoint, cx);
                            }
                        })
                        .ok();
                    })
                    .detach();
                }
                _ => {
                    session.set_condition(&path, line + 1, breakpoint.condition.clone());
                    session.set_actions(&path, line + 1, actions(&breakpoint));
                }
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

    /// Every breakpoint, in the order they were set, and whether it stops
    /// somewhere in the running session (the Breakpoints dialog).
    pub fn all_breakpoints(&self) -> Vec<(SavedBreakpoint, bool)> {
        self.breakpoints.iter().map(|b| (b.clone(), self.verified.contains(&(b.file.clone(), b.line)))).collect()
    }

    /// The Java Exception Breakpoints, "Any exception" first.
    pub fn exception_breakpoints(&self) -> Vec<breakpoint_store::SavedExceptionBreakpoint> {
        self.exceptions.clone()
    }

    /// Adds or changes an exception breakpoint (by its class), in the
    /// running session too.
    pub fn set_exception_breakpoint(&mut self, updated: breakpoint_store::SavedExceptionBreakpoint, cx: &mut Context<Self>) {
        match self.exceptions.iter_mut().find(|e| e.class == updated.class) {
            Some(e) => *e = updated.clone(),
            None => self.exceptions.push(updated.clone()),
        }
        breakpoint_store::save_exceptions(&self.exceptions);
        if let Some(session) = self.session.clone().filter(|s| !s.is_ended()) {
            cx.background_spawn(async move {
                if updated.enabled {
                    let _ = session.add_exception_breakpoint(exception_breakpoint(&updated));
                } else {
                    let _ = session.remove_exception_breakpoint(&updated.class);
                }
            })
            .detach();
        }
        cx.notify();
    }

    /// Removes an exception breakpoint ("Any exception" stays).
    pub fn remove_exception_breakpoint(&mut self, class: &str, cx: &mut Context<Self>) {
        if class.is_empty() {
            return;
        }
        self.exceptions.retain(|e| e.class != class);
        breakpoint_store::save_exceptions(&self.exceptions);
        if let Some(session) = self.session.clone().filter(|s| !s.is_ended()) {
            let class = class.to_owned();
            cx.background_spawn(async move { session.remove_exception_breakpoint(&class) }).detach();
        }
        cx.notify();
    }

    pub fn is_muted(&self) -> bool {
        self.muted
    }

    /// Mute Breakpoints (the Debug toolbar's toggle): breakpoints stay but
    /// don't stop, in the running session and the next ones.
    pub fn toggle_mute(&mut self, cx: &mut Context<Self>) {
        self.muted = !self.muted;
        breakpoint_store::save_muted(self.muted);
        if let Some(session) = &self.session {
            session.set_muted(self.muted);
        }
        cx.notify();
    }

    /// The Breakpoints dialog's Remove.
    pub fn remove_breakpoint(&mut self, file: &str, line: u32, cx: &mut Context<Self>) {
        if self.breakpoint(file, line).is_some() {
            self.toggle(SavedBreakpoint::new(file, line), cx);
        }
    }

    /// The breakpoint a click on a line sets, as IntelliJ's: a method
    /// breakpoint on a method's declaration line with none of its code, a
    /// field watchpoint on a field's, else a line breakpoint.
    fn kind_on(file: &str, line: u32, text: Option<&str>) -> SavedBreakpoint {
        let new = SavedBreakpoint::new(file, line);
        let (Some(lang), Some(text)) = (crate::index::lang::Lang::from_path(file), text) else { return new };
        use crate::index::method_breakpoint::{field_at, method_header};
        SavedBreakpoint { method: method_header(lang, text, line).map(|h| h.name), field: field_at(lang, text, line), ..new }
    }

    /// Opens a breakpoint's line in the editor.
    pub fn show_source(&mut self, file: &str, line: u32, cx: &mut Context<Self>) {
        cx.emit(DebugServiceEvent::Navigate { path: PathBuf::from(file), line });
    }

    /// Toggle Line Breakpoint (Ctrl+F8, a click on the line number).
    /// On a method's declaration line (`text` is the file's), a method
    /// breakpoint.
    pub fn toggle_breakpoint(&mut self, file: &str, line: u32, text: Option<&str>, cx: &mut Context<Self>) {
        self.toggle(Self::kind_on(file, line, text), cx);
    }

    /// Shift+click on a line number, as IntelliJ's: a breakpoint that
    /// doesn't suspend and logs its hits ("Breakpoint hit" message).
    pub fn toggle_logging_breakpoint(&mut self, file: &str, line: u32, text: Option<&str>, cx: &mut Context<Self>) {
        self.toggle(SavedBreakpoint { suspend: false, log_message: true, ..Self::kind_on(file, line, text) }, cx);
    }

    fn toggle(&mut self, new: SavedBreakpoint, cx: &mut Context<Self>) {
        let (file, line) = (new.file.as_str(), new.line);
        let session = self.session.clone().filter(|s| !s.is_ended());
        if let Some(ix) = self.breakpoints.iter().position(|b| b.file == file && b.line == line) {
            self.breakpoints.remove(ix);
            self.verified.remove(&(file.to_owned(), line));
            if let Some(session) = session {
                let path = PathBuf::from(file);
                cx.background_spawn(async move { session.remove_breakpoint(&path, line + 1) }).detach();
            }
        } else {
            let saved = new.clone();
            self.breakpoints.push(saved.clone());
            if let Some(session) = session {
                self.place_in_session(session, saved, cx);
            }
        }
        breakpoint_store::save(&self.breakpoints);
        cx.notify();
    }

    /// Moves a file's breakpoints with an edit of its text, line by line
    /// (`debugger::follow_lines`).
    pub fn follow_edit(&mut self, file: &str, old: &str, new: &str, cx: &mut Context<Self>) {
        let lines: Vec<u32> = self.breakpoints.iter().filter(|b| b.file == file).map(|b| b.line).collect();
        if lines.is_empty() {
            return;
        }
        let moved = debugger::follow_lines(old, new, &lines);
        let mut changed = false;
        for (b, line) in self.breakpoints.iter_mut().filter(|b| b.file == file).zip(moved) {
            changed |= b.line != line;
            b.line = line;
        }
        // Two on one line: the first stays.
        let mut seen = HashSet::new();
        self.breakpoints.retain(|b| b.file != file || seen.insert(b.line));
        if changed {
            breakpoint_store::save(&self.breakpoints);
            cx.notify();
        }
    }

    // --- Sessions -------------------------------------------------------------

    /// A source was saved: during a session, IntelliJ's editor offers
    /// HotSwap.
    pub fn source_saved(&mut self, file: PathBuf, cx: &mut Context<Self>) {
        if self.session.is_none() || !crate::usecase::compile_on_save::compiles(&file) {
            return;
        }
        if !self.changed_sources.contains(&file) {
            self.changed_sources.push(file);
        }
        if self.hot_swap != HotSwapStatus::Running {
            self.hot_swap = HotSwapStatus::Offered;
        }
        cx.notify();
    }

    /// The toolbar's Hide: the changes stay to reload later.
    pub fn hide_hot_swap(&mut self, cx: &mut Context<Self>) {
        if self.hot_swap != HotSwapStatus::Running {
            self.hot_swap = HotSwapStatus::Hidden;
            cx.notify();
        }
    }

    /// Apply HotSwap / Reload Changed Classes / Apply Code Changes while
    /// debugging: compiles the saved sources' modules, then reloads the
    /// classes written since the last reload.
    pub fn apply_hot_swap(&mut self, cx: &mut Context<Self>) {
        let (Some(session), Some(since)) = (self.session.clone(), self.swapped_at) else {
            cx.emit(DebugServiceEvent::HotSwap { message: "No debug session is running.".into(), failed: true });
            return;
        };
        if self.hot_swap == HotSwapStatus::Running {
            return;
        }
        self.hot_swap = HotSwapStatus::Running;
        cx.notify();
        let sources = self.changed_sources.clone();
        let android = self.android;
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let found = sources.clone();
            let targets = cx.background_spawn(async move { crate::usecase::hot_swap::targets(&crate::infra::gradle::GradleBuild, &found) }).await;
            // Each module's variant, as Build Variants picked it.
            let targets: Vec<_> = cx.update(|cx| {
                let sync = crate::ui::sync_service::SyncService::global(cx);
                targets
                    .into_iter()
                    .map(|(module, test)| {
                        let variant = sync.read(cx).module_at(&module.dir).map(|(build, m)| crate::ui::sync_service::variant_of(build, m, cx)).unwrap_or_else(|| "debug".into());
                        (module, test, variant)
                    })
                    .collect()
            });
            let applied = cx
                .background_spawn(async move {
                    let dexer = if android {
                        let sdk = crate::android::sdk::locate(targets.first().map(|(m, _, _)| m.build_root.as_path()));
                        match sdk.as_deref().and_then(crate::infra::class_files::D8::locate) {
                            Some(d8) => Some(d8),
                            None => {
                                let outcome = crate::domain::hotswap::HotSwapOutcome::Failed("Android SDK build-tools not found: d8 turns the classes into the dex the app's VM reloads.".into());
                                return (crate::usecase::hot_swap::Applied { outcome, problems: Vec::new() }, SystemTime::now());
                            }
                        }
                    } else {
                        None
                    };
                    let files = crate::infra::class_files::DiskClassFiles;
                    let applied = crate::usecase::hot_swap::apply(&crate::infra::gradle::GradleBuild, &targets, &session, &files, since, dexer.as_ref().map(|d| d as &dyn crate::domain::hotswap::Dexer));
                    (applied, SystemTime::now())
                })
                .await;
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                let (applied, finished) = applied;
                let failed = matches!(applied.outcome, crate::domain::hotswap::HotSwapOutcome::Failed(_));
                if !failed {
                    this.swapped_at = Some(finished);
                    this.changed_sources.retain(|f| !sources.contains(f));
                }
                if !applied.problems.is_empty() {
                    use crate::domain::build::BuildSystem as _;
                    if let Some(root) = sources.first().and_then(|f| crate::infra::gradle::GradleBuild.module_of(f)).map(|m| m.build_root) {
                        crate::ui::compile_service::CompileService::global(cx).update(cx, |c, cx| c.set_build_problems(&root, applied.problems, cx));
                    }
                }
                let message = applied.outcome.message();
                if let Some(run) = this.run.as_mut() {
                    run.console.push(format!("HotSwap: {message}").into());
                }
                this.hot_swap = if failed { HotSwapStatus::Offered } else { HotSwapStatus::Reloaded };
                cx.emit(DebugServiceEvent::HotSwap { message, failed });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn is_active(&self) -> bool {
        self.run.as_ref().is_some_and(|r| r.status != DebugStatus::Ended)
    }

    pub fn is_paused(&self) -> bool {
        self.run.as_ref().is_some_and(|r| r.status == DebugStatus::Paused)
    }

    /// Debug a test (the gutter popup's Debug, Ctrl+Shift+F9): runs it with
    /// its JVM waiting for the debugger, and attaches.
    pub fn debug_test(&mut self, path: PathBuf, marks: Vec<TestMark>, mark: &TestMark, cx: &mut Context<Self>) {
        let mark = mark.clone();
        self.debug_tests(mark.name(), move |port, cx| TestService::global(cx).update(cx, |s, cx| s.run_mark(path, marks, &mark, Some(port), cx)), cx);
    }

    /// Debug 'Tests in …' on a Project window folder.
    pub fn debug_folder(&mut self, build_root: PathBuf, module: crate::domain::run::ModuleInfo, request: crate::domain::testing::TestRequest, cx: &mut Context<Self>) {
        let name = request.name.clone();
        self.debug_tests(name, move |port, cx| {
            TestService::global(cx).update(cx, |s, cx| s.run_folder(build_root, module, request, Some(port), cx));
            true
        }, cx);
    }

    /// Debug with a saved JUnit configuration selected in the toolbar.
    pub fn debug_configured(&mut self, build_root: PathBuf, module: crate::domain::run::ModuleInfo, mut request: crate::domain::testing::TestRequest, cx: &mut Context<Self>) {
        let name = request.name.clone();
        self.debug_tests(name, move |port, cx| {
            request.debug_port = Some(port);
            TestService::global(cx).update(cx, |s, cx| s.run_configured(build_root, module, request, cx));
            true
        }, cx);
    }

    /// Debug with a temporary test configuration selected in the toolbar.
    pub fn debug_temporary(&mut self, name: String, cx: &mut Context<Self>) {
        let n = name.clone();
        self.debug_tests(name, move |port, cx| TestService::global(cx).update(cx, |s, cx| s.run_temporary(&n, Some(port), false, cx)), cx);
    }

    /// Starts tests with their JVM waiting for the debugger on a free
    /// port (`start` says whether it did), and attaches.
    fn debug_tests(&mut self, name: String, start: impl FnOnce(u16, &mut Context<Self>) -> bool, cx: &mut Context<Self>) {
        self.end_session(cx);
        let Some(port) = free_port() else {
            cx.emit(DebugServiceEvent::Error { title: format!("Error running '{name}'"), message: "No free port for the debugger.".into() });
            return;
        };
        if !start(port, cx) {
            return;
        }
        let generation = self.begin(name, format!("Waiting for the test JVM on port {port}…"), cx);
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
        self.android = true;
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
        self.android = false;
        self.changed_sources.clear();
        self.swapped_at = None;
        self.hot_swap = HotSwapStatus::Hidden;
        self.run = Some(DebugRun {
            name,
            status: DebugStatus::Connecting,
            reason: None,
            exception: None,
            threads: Vec::new(),
            thread: None,
            frames: Vec::new(),
            frame: 0,
            variables: Vec::new(),
            variables_loading: false,
            console: vec![first_line.into()],
            target: None,
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
                    run.exception = None;
                    run.frames.clear();
                    run.variables.clear();
                }
            }
            SessionEvent::Verified { path, line } => {
                self.verified.insert((key(&path), line - 1));
            }
            SessionEvent::Exception(text) => {
                if let Some(run) = self.run.as_mut() {
                    run.console.push(text.clone().into());
                    run.exception = Some(text);
                }
            }
            SessionEvent::Message(text) => {
                if let Some(run) = self.run.as_mut() {
                    run.console.push(text.clone().into());
                }
                cx.emit(DebugServiceEvent::Error { title: "Breakpoint condition".into(), message: text });
            }
            SessionEvent::Log(text) => {
                if let Some(run) = self.run.as_mut() {
                    run.console.extend(text.lines().map(|l| SharedString::from(l.to_owned())));
                }
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
                let disconnected = match &run.target {
                    Some(target) => format!("Disconnected from the target VM, {target}"),
                    None => "Disconnected from the target VM".into(),
                };
                run.console.push(disconnected.into());
                run.status = DebugStatus::Ended;
                run.frames.clear();
                run.variables.clear();
                run.threads.clear();
            }
        }
        self.session = None;
        self.verified.clear();
        self.hot_swap = HotSwapStatus::Hidden;
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

    /// Detach: the debugger lets go; the program runs on without it.
    pub fn detach(&mut self, cx: &mut Context<Self>) {
        if !self.run.as_ref().is_some_and(|r| matches!(r.status, DebugStatus::Running | DebugStatus::Paused)) {
            return;
        }
        if let Some(session) = self.session.take() {
            cx.background_spawn(async move { session.detach() }).detach();
        }
        let generation = self.generation;
        self.ended(generation, None, cx);
        self.generation += 1;
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

    /// An expression's value and description in the selected frame, while
    /// paused (Evaluate Expression's Evaluate).
    pub fn evaluate_task(&self, text: &str, cx: &mut Context<Self>) -> Option<Task<Result<(Value, Option<String>), String>>> {
        let (session, run) = (self.session.clone()?, self.run.as_ref()?);
        let thread = run.thread.filter(|_| run.status == DebugStatus::Paused && run.frame < run.frames.len())?;
        let (depth, expression) = (run.frame, text.to_owned());
        Some(cx.background_spawn(async move {
            session.evaluate(thread, depth, &expression).map(|value| {
                let detail = session.describe(thread, &value);
                (value, detail)
            })
        }))
    }

    /// A value's fields, elements or items, each described: a tree row's
    /// children.
    pub fn children_task(&self, value: Value, cx: &mut Context<Self>) -> Option<Task<Vec<VarNode>>> {
        let (session, thread) = (self.session.clone()?, self.run.as_ref()?.thread?);
        Some(cx.background_spawn(async move {
            session
                .children(thread, &value)
                .map(|children| {
                    children
                        .into_iter()
                        .map(|v| {
                            let detail = session.describe(thread, &v.value);
                            VarNode::new((v, detail))
                        })
                        .collect()
                })
                .unwrap_or_default()
        }))
    }

    /// After an evaluation: one that assigned shows the Variables again,
    /// with the new values.
    pub fn evaluated(&mut self, text: &str, cx: &mut Context<Self>) {
        if crate::domain::expression::assigns(text) {
            self.reload_variables(true, cx);
        }
    }

    /// Remembers an evaluated expression, the latest first, as IntelliJ's
    /// history (20 of them).
    pub fn remember_evaluated(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.evaluated.retain(|e| e != text);
        self.evaluated.insert(0, text.to_owned());
        self.evaluated.truncate(20);
    }

    /// The evaluate field (Enter): the expression's value on top of the
    /// Variables view, in the selected frame.
    pub fn evaluate(&mut self, text: String, cx: &mut Context<Self>) {
        if text.trim().is_empty() {
            return;
        }
        let Some(task) = self.evaluate_task(&text, cx) else { return };
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                let Some(run) = this.run.as_mut() else { return };
                run.variables.retain(|n| n.kind != NodeKind::Evaluation);
                run.variables.insert(0, VarNode::expression(text.clone(), NodeKind::Evaluation, result));
                this.evaluated(&text, cx);
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
        let generation = self.generation;
        let Some(node) = self.run.as_mut().and_then(|r| node_at(&mut r.variables, path)) else { return };
        node.expanded = !node.expanded;
        let load = (node.expanded && node.children.is_none()).then(|| node.var.value.clone());
        if let Some(value) = load {
            let Some(task) = self.children_task(value, cx) else { return };
            let path = path.to_vec();
            cx.spawn(async move |this, cx| {
                let result = task.await;
                this.update(cx, |this, cx| {
                    if this.generation != generation {
                        return;
                    }
                    if let Some(node) = this.run.as_mut().and_then(|r| node_at(&mut r.variables, &path)) {
                        node.children = Some(result);
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
        // What was compiled before the VM started is what it loads.
        this.swapped_at = Some(SystemTime::now());
        session.set_muted(this.muted);
        if let Some(run) = this.run.as_mut() {
            run.status = DebugStatus::Running;
            run.console.push(format!("Connected to the target VM, {target}").into());
            run.target = Some(target.clone());
        }
        cx.notify();
        (this.breakpoints.clone(), this.exceptions.clone())
    }) else {
        return;
    };
    let (breakpoints, exceptions) = breakpoints;
    let setup = session.clone();
    let (verified, started) = cx
        .background_spawn(async move {
            let mut verified = Vec::new();
            for b in breakpoints.into_iter().filter(|b| b.enabled) {
                if let Ok(true) = setup.add_breakpoint(line_breakpoint(&b)) {
                    verified.push((b.file, b.line));
                }
            }
            for e in exceptions.iter().filter(|e| e.enabled) {
                let _ = setup.add_exception_breakpoint(exception_breakpoint(e));
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
pub(crate) fn node_at<'a>(nodes: &'a mut [VarNode], path: &[usize]) -> Option<&'a mut VarNode> {
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
    breakpoint.actions = actions(saved);
    // A method breakpoint: the method as the file declares it now.
    breakpoint.method = saved.method.as_ref().map(|name| {
        crate::index::lang::Lang::from_path(&saved.file)
            .and_then(|lang| crate::index::method_breakpoint::method_header(lang, &text, saved.line))
            .unwrap_or(debugger::MethodHeader { name: name.clone(), line: saved.line + 1, end: saved.line + 1 })
    });
    breakpoint.method_entry = saved.method_entry;
    breakpoint.method_exit = saved.method_exit;
    breakpoint.field = saved.field.as_ref().map(|name| debugger::FieldWatch { name: name.clone(), access: saved.field_access, modification: saved.field_modification });
    breakpoint
}

fn actions(saved: &SavedBreakpoint) -> BreakpointActions {
    BreakpointActions { suspend: saved.suspend, log_message: saved.log_message, log_stack: saved.log_stack, log_expression: saved.log_expression.clone() }
}

fn free_port() -> Option<u16> {
    std::net::TcpListener::bind("127.0.0.1:0").ok()?.local_addr().ok().map(|a| a.port())
}

/// A saved exception breakpoint as the session sets it.
fn exception_breakpoint(saved: &breakpoint_store::SavedExceptionBreakpoint) -> crate::usecase::debug_session::ExceptionBreakpoint {
    crate::usecase::debug_session::ExceptionBreakpoint { class: saved.class.clone(), caught: saved.caught, uncaught: saved.uncaught }
}
