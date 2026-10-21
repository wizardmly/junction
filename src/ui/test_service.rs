//! Tests, as Android Studio runs them: a test class or method from the
//! editor's gutter (or Ctrl+Shift+F10 at the caret) becomes a Gradle test
//! run on its module; the results tree fills in as tests start and finish,
//! and the Run window shows it. Rerun and Rerun Failed Tests repeat the
//! last run. Remembers each test's last outcome for the gutter icons.
//! Composition root of the run-tests use case over Gradle.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Global, SharedString, Task};

use crate::domain::coverage::CoverageReport;
use crate::domain::run::{BuildResult, ModuleInfo};
use crate::domain::testing::{self, Outcome, TestEvent, TestMark, TestRequest, TestResults};
use crate::infra::gradle_tests::GradleTests;
use crate::ui::sync_service::{self, SyncService};
use crate::usecase::run_tests;

pub enum TestServiceEvent {
    /// A test run started: the Run window shows its tab.
    Show,
    /// "Error running 'ExampleUnitTest'": a balloon.
    Error { title: String, message: String },
    /// A test run from the gutter or the Project window made (or reused)
    /// its temporary run configuration: the toolbar selects it.
    Temporary(String),
    /// Run with Coverage finished with a report (the Coverage window shows
    /// it), or Hide Coverage dropped it.
    Coverage,
}

const TEMPORARY_LIMIT: usize = 5;

/// What a test run is of, kept for Rerun.
#[derive(Clone)]
struct Launch {
    build_root: PathBuf,
    module: ModuleInfo,
    request: TestRequest,
    /// The file the run came from and its tests, to go to a test's source.
    source: Option<(PathBuf, Vec<TestMark>)>,
}

/// The Run window's test tab: the last run.
pub struct TestRun {
    /// The run configuration's name: "ExampleUnitTest.addition_isCorrect".
    pub name: String,
    pub results: TestResults,
    /// Gradle's other output, for the root's console.
    pub lines: Vec<SharedString>,
    pub running: bool,
    pub started: Instant,
    pub took: Option<Duration>,
    /// The build itself failed (a compile error): no tests ran.
    pub build_failed: bool,
    pub cancelled: bool,
}

enum Message {
    Event(TestEvent),
    Line(String),
}

#[derive(Default)]
pub struct TestService {
    pub run: Option<TestRun>,
    last: Option<Launch>,
    /// Each test's (and class's) last outcome, for the gutter.
    history: HashMap<(String, Option<String>), Outcome>,
    /// Temporary run configurations, as IntelliJ makes one for each test
    /// run from the gutter or the Project window: newest last, at most
    /// five (its default "Temporary configurations limit").
    temporary: Vec<Launch>,
    /// The last Run with Coverage's report, until Hide Coverage: the
    /// Coverage window's table and the editors' gutter stripes.
    pub coverage: Option<Arc<CoverageReport>>,
    /// The covered module's folder, where its classes' sources are.
    pub coverage_root: Option<PathBuf>,
    cancel: Arc<AtomicBool>,
    receiver: Option<std::sync::mpsc::Receiver<Message>>,
    _task: Option<Task<()>>,
}

impl EventEmitter<TestServiceEvent> for TestService {}

struct GlobalTests(Entity<TestService>);
impl Global for GlobalTests {}

impl TestService {
    pub fn global(cx: &mut App) -> Entity<Self> {
        if let Some(g) = cx.try_global::<GlobalTests>() {
            return g.0.clone();
        }
        let service = cx.new(|_| Self::default());
        cx.set_global(GlobalTests(service.clone()));
        service
    }

    pub fn try_global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalTests>().map(|g| g.0.clone())
    }

    pub fn is_running(&self) -> bool {
        self.run.as_ref().is_some_and(|r| r.running)
    }

    /// The last outcome of a test class or method, as the gutter shows it.
    pub fn outcome_of(&self, class: &str, method: Option<&str>) -> Option<Outcome> {
        if let Some(outcome) = self.run.as_ref().and_then(|r| r.results.outcome_of(class, method)) {
            return Some(outcome);
        }
        self.history.get(&(class.to_owned(), method.map(str::to_owned))).copied()
    }

    /// Where a test's source is: its file and 0-based line, when the run
    /// came from that file.
    pub fn source_of(&self, class: &str, method: Option<&str>) -> Option<(PathBuf, u32)> {
        let (path, marks) = self.last.as_ref()?.source.as_ref()?;
        let mark = marks
            .iter()
            .find(|m| m.class == class && method.is_some() && m.method.as_deref() == method)
            .or_else(|| marks.iter().find(|m| m.class == class && m.method.is_none()))?;
        Some((path.clone(), mark.line))
    }

    /// Runs one of a file's tests (a gutter icon, Ctrl+Shift+F10); with a
    /// port, its test JVM waits there for the debugger.
    pub fn run_mark(&mut self, path: PathBuf, marks: Vec<TestMark>, mark: &TestMark, debug_port: Option<u16>, cx: &mut Context<Self>) -> bool {
        self.launch_mark(path, marks, mark, |r| r.debug_port = debug_port, cx)
    }

    /// Run '…' with Coverage, from the gutter or the Project window.
    pub fn cover_mark(&mut self, path: PathBuf, marks: Vec<TestMark>, mark: &TestMark, cx: &mut Context<Self>) -> bool {
        self.launch_mark(path, marks, mark, |r| r.coverage = true, cx)
    }

    fn launch_mark(&mut self, path: PathBuf, marks: Vec<TestMark>, mark: &TestMark, set: impl FnOnce(&mut TestRequest), cx: &mut Context<Self>) -> bool {
        let name = mark.name();
        let Some(kind) = testing::test_kind(&path.to_string_lossy()) else {
            cx.emit(TestServiceEvent::Error { title: format!("Error running '{name}'"), message: "The file is not in a test source set.".into() });
            return false;
        };
        let mut request = TestRequest::of(kind, mark);
        set(&mut request);
        if request.coverage && kind == testing::TestKind::Instrumented {
            cx.emit(TestServiceEvent::Error { title: format!("Error running '{name}'"), message: "Coverage is measured for unit tests only.".into() });
            return false;
        }
        self.launch_request(path, marks, request, cx)
    }

    fn launch_request(&mut self, path: PathBuf, marks: Vec<TestMark>, request: TestRequest, cx: &mut Context<Self>) -> bool {
        let name = request.name.clone();
        let builds = SyncService::try_global(cx).map(|s| s.read(cx).builds().to_vec()).unwrap_or_default();
        // Before the first sync, the build files say where the module is.
        let found = testing::module_of(&path, &builds).map(|(b, m)| (b.to_path_buf(), m.clone())).or_else(|| crate::infra::gradle::module_from_files(&path));
        let Some((build_root, module)) = found else {
            cx.emit(TestServiceEvent::Error { title: format!("Error running '{name}'"), message: "The file is not in a Gradle module.".into() });
            return false;
        };
        let launch = Launch { build_root, module, request, source: Some((path, marks)) };
        self.remember(&launch, cx);
        self.start(launch, cx);
        true
    }

    /// What Run on a Project window folder runs there (`Tests in
    /// 'com.example'`), with the build and module; none outside tests.
    pub fn folder_tests(dir: &std::path::Path, cx: &App) -> Option<(PathBuf, crate::domain::run::ModuleInfo, TestRequest)> {
        let builds = SyncService::try_global(cx).map(|s| s.read(cx).builds().to_vec()).unwrap_or_default();
        let (build_root, module) = testing::module_of(dir, &builds).map(|(b, m)| (b.to_path_buf(), m.clone())).or_else(|| crate::infra::gradle::module_from_files(&dir.join("x")))?;
        let rel = dir.strip_prefix(&module.dir).ok()?.to_string_lossy().into_owned();
        let request = testing::folder_request(&rel, &module)?;
        // `src` and the module's folder offer it only with unit tests there.
        if rel.is_empty() || rel == "src" {
            let sets = std::fs::read_dir(module.dir.join("src")).ok()?;
            if !sets.flatten().any(|e| testing::test_kind(&format!("src/{}/", e.file_name().to_string_lossy())) == Some(testing::TestKind::Unit)) {
                return None;
            }
        }
        Some((build_root, module, request))
    }

    /// Runs a folder's tests (Project window › Run 'Tests in …').
    pub fn run_folder(&mut self, build_root: PathBuf, module: crate::domain::run::ModuleInfo, mut request: TestRequest, debug_port: Option<u16>, cx: &mut Context<Self>) {
        request.debug_port = debug_port;
        if request.coverage && request.kind == testing::TestKind::Instrumented {
            cx.emit(TestServiceEvent::Error { title: format!("Error running '{}'", request.name), message: "Coverage is measured for unit tests only.".into() });
            return;
        }
        let launch = Launch { build_root, module, request, source: None };
        self.remember(&launch, cx);
        self.start(launch, cx);
    }

    /// The temporary configurations' names, oldest first.
    pub fn temporary_names(&self) -> Vec<String> {
        self.temporary.iter().map(|l| l.request.name.clone()).collect()
    }

    /// Runs a temporary configuration again (the toolbar's Run or Debug
    /// with it selected); whether it still exists.
    pub fn run_temporary(&mut self, name: &str, debug_port: Option<u16>, coverage: bool, cx: &mut Context<Self>) -> bool {
        let Some(mut launch) = self.temporary.iter().find(|l| l.request.name == name).cloned() else { return false };
        launch.request.debug_port = debug_port;
        launch.request.coverage = coverage;
        if coverage && launch.request.kind == testing::TestKind::Instrumented {
            cx.emit(TestServiceEvent::Error { title: format!("Error running '{name}'"), message: "Coverage is measured for unit tests only.".into() });
            return false;
        }
        self.start(launch, cx);
        true
    }

    /// Rerun: the last run again.
    pub fn rerun(&mut self, cx: &mut Context<Self>) {
        if let Some(mut launch) = self.last.clone() {
            launch.request.debug_port = None;
            self.start(launch, cx);
        }
    }

    /// Rerun Failed Tests: the last run's failed tests.
    pub fn rerun_failed(&mut self, cx: &mut Context<Self>) {
        let (Some(mut launch), Some(run)) = (self.last.clone(), self.run.as_ref()) else { return };
        launch.request.debug_port = None;
        let failed = run.results.failed();
        if failed.is_empty() {
            return;
        }
        launch.request.filters = failed;
        self.start(launch, cx);
    }

    pub fn has_failed(&self) -> bool {
        self.run.as_ref().is_some_and(|r| !r.running && r.results.counts().failed > 0)
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.cancel.store(true, Ordering::Relaxed);
        cx.notify();
    }

    // --- Internals ------------------------------------------------------

    fn remember(&mut self, launch: &Launch, cx: &mut Context<Self>) {
        let name = launch.request.name.clone();
        self.temporary.retain(|l| l.request.name != name);
        let mut kept = launch.clone();
        kept.request.debug_port = None;
        kept.request.coverage = false;
        self.temporary.push(kept);
        if self.temporary.len() > TEMPORARY_LIMIT {
            self.temporary.remove(0);
        }
        cx.emit(TestServiceEvent::Temporary(name));
    }

    fn start(&mut self, launch: Launch, cx: &mut Context<Self>) {
        if self.is_running() {
            self.stop(cx);
        }
        let variant = sync_service::variant_of(&launch.build_root, &launch.module, cx);
        let plan = testing::plan(&launch.build_root, &launch.module, &variant, &launch.request);
        self.cancel = Arc::new(AtomicBool::new(false));
        let first = format!("Executing tasks: [{}]", plan.args.iter().filter(|a| a.starts_with(':')).cloned().collect::<Vec<_>>().join(", "));
        self.run = Some(TestRun {
            name: launch.request.name.clone(),
            results: TestResults::default(),
            lines: vec![first.into()],
            running: true,
            started: Instant::now(),
            took: None,
            build_failed: false,
            cancelled: false,
        });
        self.last = Some(launch.clone());
        cx.emit(TestServiceEvent::Show);
        cx.notify();

        let (tx, rx) = std::sync::mpsc::channel();
        self.receiver = Some(rx);
        // What the run sends goes into the tree ten times a second.
        self._task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                let Ok(more) = this.update(cx, |this, cx| {
                    if this.drain() {
                        cx.notify();
                    }
                    this.receiver.is_some()
                }) else {
                    break;
                };
                if !more {
                    break;
                }
            }
        }));
        let cancel = self.cancel.clone();
        let root = launch.build_root.clone();
        let work = cx.background_spawn(async move {
            let events = tx.clone();
            run_tests::run(&GradleTests, &plan, &mut |e| drop(events.send(Message::Event(e.clone()))), &mut |l| drop(tx.send(Message::Line(l.to_owned()))), &|| cancel.load(Ordering::Relaxed))
        });
        cx.spawn(async move |this, cx| {
            let (results, build, coverage) = work.await;
            this.update(cx, |this, cx| this.finish(&root, results, build, coverage, cx)).ok();
        })
        .detach();
    }

    /// Applies what arrived; whether anything did.
    fn drain(&mut self) -> bool {
        let Some(rx) = &self.receiver else { return false };
        let mut any = false;
        loop {
            match rx.try_recv() {
                Ok(message) => {
                    any = true;
                    let Some(run) = self.run.as_mut() else { continue };
                    match message {
                        Message::Event(event) => run.results.apply(event),
                        Message::Line(line) => run.lines.push(line.into()),
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.receiver = None;
                    break;
                }
            }
        }
        any
    }

    /// Hide Coverage: the Coverage window empties, the stripes go.
    pub fn hide_coverage(&mut self, cx: &mut Context<Self>) {
        self.coverage = None;
        cx.emit(TestServiceEvent::Coverage);
        cx.notify();
    }

    fn finish(&mut self, root: &Path, results: TestResults, build: BuildResult, coverage: Option<CoverageReport>, cx: &mut Context<Self>) {
        self.drain();
        self.receiver = None;
        let covered = self.last.as_ref().is_some_and(|l| l.request.coverage);
        let Some(run) = self.run.as_mut() else { return };
        // The use case's tree has the XML reports merged in and the
        // stopped tests terminated.
        run.results = results;
        run.running = false;
        run.cancelled = build.cancelled;
        run.took = Some(run.started.elapsed());
        let counts = run.results.counts();
        run.build_failed = !build.succeeded && !build.cancelled && counts.failed == 0;
        for suite in &run.results.suites {
            self.history.insert((suite.class.clone(), None), suite.outcome());
            for test in &suite.tests {
                if let Some(outcome) = run.results.outcome_of(&suite.class, Some(test.method())) {
                    self.history.insert((suite.class.clone(), Some(test.method().to_owned())), outcome);
                }
            }
        }
        // The editors mark a failed build's errors, as after Make Project.
        if run.build_failed {
            let name = run.name.clone();
            crate::ui::compile_service::CompileService::global(cx).update(cx, |c, cx| c.set_build_problems(root, build.problems, cx));
            cx.emit(TestServiceEvent::Error { title: format!("Error running '{name}'"), message: "Gradle build failed. See the Run window.".into() });
        } else if covered && !build.cancelled {
            match coverage {
                Some(mut report) => {
                    report.name = run.name.clone();
                    self.coverage = Some(Arc::new(report));
                    self.coverage_root = self.last.as_ref().map(|l| l.module.dir.clone());
                    cx.emit(TestServiceEvent::Coverage);
                }
                None => {
                    let name = run.name.clone();
                    cx.emit(TestServiceEvent::Error { title: format!("Coverage of '{name}'"), message: "JaCoCo wrote no coverage report. See the Run window.".into() });
                }
            }
        }
        cx.notify();
    }
}
