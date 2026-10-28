//! Build and Run, as Android Studio's main toolbar drives them: the run
//! configuration and target device picked there, Make Project, Run 'app'
//! and Stop. Keeps what the Build and Run tool windows show. Composition
//! root of the run use case over Gradle and adb.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, SharedString, Task};

use crate::android::{Device, Kind};
use crate::domain::diagnostics::FileProblem;
use crate::domain::run::{self, BuildResult, RunConfiguration};
use crate::infra::adb_device::AdbDevice;
use crate::infra::gradle::GradleBuild;
use crate::settings::Settings;
use crate::ui::device_manager::DeviceManager;
use crate::ui::sync_service::{self, SyncService};
use crate::ui::test_service::TestService;
use crate::usecase::run_app::{self, ConsoleLine, RunOutcome};

pub enum RunEvent {
    /// A build started: the Build window shows.
    ShowBuild,
    /// A launch started: the Run window shows.
    ShowRun,
    /// "Error running 'app'": a balloon.
    Error { title: String, message: String },
    /// Generate Signed Bundle / APK finished: Android Studio's balloon,
    /// whose link shows the folder.
    Generated { title: String, message: String, folder: PathBuf },
    /// Debug 'app' started the app waiting for the debugger: attach.
    Attach { name: String, serial: String, pid: u32, adb: crate::android::adb::Adb },
    /// "Clear log before launch" cleared the device's log: its Logcat
    /// panels empty too (Android Studio's `ClearLogcatListener`).
    ClearLogcat { serial: String },
    /// "Show logcat automatically": the Logcat tab of the launched app
    /// (Android Studio's `ShowLogcatListener`).
    ShowLogcat { serial: String, application_id: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

/// What the Build window shows: the last build.
pub struct BuildLog {
    /// "Build" (Make Project) or "Run 'app'".
    pub title: String,
    pub lines: Vec<SharedString>,
    pub problems: Vec<FileProblem>,
    /// Gradle's "What went wrong" when it failed.
    pub failure: Option<String>,
    pub status: BuildStatus,
    pub started: chrono::DateTime<chrono::Local>,
    pub took: Option<Duration>,
    /// A sync's warnings: "Failed to resolve: g:a:v".
    pub warnings: Vec<String>,
}

/// What the Run window shows: the last launch.
pub struct Console {
    pub name: String,
    pub lines: Vec<ConsoleLine>,
    /// The devices it was launched on.
    pub serials: Vec<String>,
    pub application_id: Option<String>,
    /// Launching, or the app runs.
    pub active: bool,
}

enum Message {
    Build(String),
    Console(ConsoleLine),
    ClearLogcat(String),
}

/// The device port, telling the Logcat panels when the log was cleared.
struct LaunchBridge {
    adb: AdbDevice,
    tx: std::sync::mpsc::Sender<Message>,
}

impl run::DeviceBridge for LaunchBridge {
    fn install(&self, serial: &str, apks: &[PathBuf]) -> Result<Duration, String> {
        self.adb.install(serial, apks)
    }
    fn force_stop(&self, serial: &str, application_id: &str) {
        self.adb.force_stop(serial, application_id)
    }
    fn launcher_activity(&self, serial: &str, application_id: &str) -> Option<String> {
        self.adb.launcher_activity(serial, application_id)
    }
    fn start(&self, serial: &str, component: &str, debug: bool, flags: &str) -> Result<(), String> {
        self.adb.start(serial, component, debug, flags)
    }
    fn pid(&self, serial: &str, application_id: &str) -> Option<u32> {
        self.adb.pid(serial, application_id)
    }
    fn clear_logcat(&self, serial: &str) {
        self.adb.clear_logcat(serial);
        self.tx.send(Message::ClearLogcat(serial.to_owned())).ok();
        // The panels empty (they're drained ten times a second) before the
        // app logs anything.
        std::thread::sleep(Duration::from_millis(250));
    }
}

pub struct RunService {
    devices: Entity<DeviceManager>,
    /// The picked configuration's module path ("build root|:app").
    config: Option<String>,
    /// A temporary test configuration picked instead (its name).
    test: Option<String>,
    pub build: Option<BuildLog>,
    pub console: Option<Console>,
    cancel: Arc<AtomicBool>,
    /// A build or launch is going on.
    busy: bool,
    /// Lines the background work sends.
    receiver: Option<std::sync::mpsc::Receiver<Message>>,
    /// Devices whose log a launch cleared, for the Logcat panels.
    cleared: Vec<String>,
    _task: Option<Task<()>>,
}

impl EventEmitter<RunEvent> for RunService {}

impl RunService {
    pub fn new(devices: Entity<DeviceManager>) -> Self {
        Self { devices, config: None, test: None, build: None, console: None, cancel: Arc::default(), busy: false, receiver: None, cleared: Vec::new(), _task: None }
    }

    /// The Build window's Clear All: its output text (the problems stay).
    pub fn clear_build_output(&mut self, cx: &mut gpui_kit::Context<Self>) {
        if let Some(build) = self.build.as_mut() {
            build.lines.clear();
            cx.notify();
        }
    }

    /// The Run console's Clear All.
    pub fn clear_console(&mut self, cx: &mut gpui_kit::Context<Self>) {
        if let Some(console) = self.console.as_mut() {
            console.lines.clear();
            cx.notify();
        }
    }

    /// One "Android App" configuration per application module.
    pub fn configurations(&self, cx: &App) -> Vec<RunConfiguration> {
        SyncService::try_global(cx).map(|s| s.read(cx).builds().iter().flat_map(|(root, modules)| run::run_configurations(root, modules, &crate::infra::run_config_store::load(root))).collect()).unwrap_or_default()
    }

    pub fn configuration(&self, cx: &App) -> Option<RunConfiguration> {
        let all = self.configurations(cx);
        let picked = self.config.as_ref().and_then(|key| all.iter().find(|c| config_key(c) == *key).cloned());
        picked.or_else(|| all.into_iter().next())
    }

    /// Run › Edit Configurations…: the dialog over this project's
    /// configurations; OK saves them.
    pub fn edit_configurations(&mut self, window: &mut gpui_kit::Window, cx: &mut Context<Self>) {
        let builds = SyncService::try_global(cx).map(|s| s.read(cx).builds().to_vec()).unwrap_or_default();
        let modules: Vec<(std::path::PathBuf, run::ModuleInfo)> =
            builds.iter().flat_map(|(root, modules)| modules.iter().map(move |m| (root.clone(), m.clone()))).collect();
        let current: Vec<(std::path::PathBuf, run::SavedRunConfiguration)> = self
            .configurations(cx)
            .into_iter()
            .map(|c| (c.build_root.clone(), run::SavedRunConfiguration { name: c.name, module: if c.gradle.is_some() { String::new() } else { c.module.path }, gradle: c.gradle, junit: c.junit, options: c.options }))
            .collect();
        let selected = self.configuration(cx).map(|c| c.name);
        let this = cx.entity().downgrade();
        let roots: Vec<std::path::PathBuf> = builds.iter().map(|(root, _)| root.clone()).collect();
        crate::ui::run_config_dialog::open(roots, modules, current, selected, move |_, cx| {
            this.update(cx, |_, cx| cx.notify()).ok();
        }, window, cx);
    }

    /// The picked temporary configuration as a saved JUnit (or Android
    /// Instrumented Tests) one, when it can be: tests of a package, a
    /// class or a method.
    fn savable_test(&self, cx: &App) -> Option<(std::path::PathBuf, run::SavedRunConfiguration)> {
        let name = self.selected_test(cx)?;
        let (root, module, request) = TestService::try_global(cx)?.read(cx).temporary(&name)?;
        let instrumented = request.kind == crate::domain::testing::TestKind::Instrumented;
        let junit = run::JUnitTarget::from_filters(&request.filters, instrumented)?;
        let options = run::LaunchOptions { before_launch: Vec::new(), ..Default::default() };
        Some((root, run::SavedRunConfiguration { name, module: module.path, gradle: None, junit: Some(junit), options }))
    }

    /// The dropdown's Save '…' Configuration: its name, when the picked
    /// temporary configuration can be saved.
    pub fn save_test_label(&self, cx: &App) -> Option<String> {
        self.savable_test(cx).map(|(_, s)| format!("Save '{}' Configuration", s.name))
    }

    /// Save '…' Configuration: the temporary configuration becomes a saved
    /// JUnit (or Android Instrumented Tests) one, picked.
    pub fn save_test(&mut self, cx: &mut Context<Self>) {
        let Some((root, mut saved)) = self.savable_test(cx) else { return };
        let temporary = saved.name.clone();
        // A build on its defaults keeps them alongside.
        let mut list = crate::infra::run_config_store::load(&root);
        if list.is_empty() {
            list = self
                .configurations(cx)
                .into_iter()
                .filter(|c| c.build_root == root)
                .map(|c| run::SavedRunConfiguration { name: c.name, module: c.module.path, gradle: c.gradle, junit: c.junit, options: c.options })
                .collect();
        }
        let base = saved.name.clone();
        saved.name = (0..).map(|i| if i == 0 { base.clone() } else { format!("{base} ({i})") }).find(|n| !list.iter().any(|c| c.name == *n)).unwrap_or(base);
        list.push(saved.clone());
        crate::infra::run_config_store::save(&root, &list);
        TestService::global(cx).update(cx, |t, cx| t.forget_temporary(&temporary, cx));
        if let Some(config) = self.configurations(cx).into_iter().find(|c| c.build_root == root && c.name == saved.name) {
            self.pick_configuration(&config, cx);
        }
    }

    pub fn pick_configuration(&mut self, config: &RunConfiguration, cx: &mut Context<Self>) {
        self.config = Some(config_key(config));
        self.test = None;
        cx.notify();
    }

    /// The temporary test configurations the dropdown lists after the
    /// project's, oldest first.
    pub fn test_configurations(&self, cx: &App) -> Vec<String> {
        TestService::try_global(cx).map(|t| t.read(cx).temporary_names()).unwrap_or_default()
    }

    /// The picked temporary test configuration, while it still exists.
    pub fn selected_test(&self, cx: &App) -> Option<String> {
        self.test.clone().filter(|name| self.test_configurations(cx).contains(name))
    }

    pub fn pick_test(&mut self, name: String, cx: &mut Context<Self>) {
        self.test = Some(name);
        cx.notify();
    }

    /// What Run and Debug start: the picked configuration's name.
    pub fn selected_name(&self, cx: &App) -> Option<String> {
        self.selected_test(cx).or_else(|| self.configuration(cx).map(|c| c.name))
    }

    /// The devices a run can target: running ones, then the virtual
    /// devices that can be started.
    pub fn devices(&self, cx: &App) -> Vec<Device> {
        let mut out: Vec<Device> = self.devices.read(cx).devices().iter().filter(|d| d.wifi.is_none() && d.error.is_none() && (d.online || d.kind == Kind::Virtual)).cloned().collect();
        out.sort_by_key(|d| !d.online);
        out
    }

    /// The device Run targets; with several selected, the first.
    pub fn device(&self, cx: &App) -> Option<Device> {
        self.targets(cx).into_iter().next()
    }

    /// The devices Run targets: those picked with Select Multiple
    /// Devices… that are still there, else the one picked.
    pub fn targets(&self, cx: &App) -> Vec<Device> {
        let all = self.devices(cx);
        let several: Vec<Device> = Settings::get(cx).run_devices.iter().filter_map(|id| all.iter().find(|d| d.id == *id).cloned()).collect();
        if !several.is_empty() {
            return several;
        }
        let saved = Settings::get(cx).run_device.clone();
        all.iter().find(|d| d.id == saved).cloned().or_else(|| all.into_iter().find(|d| d.online)).into_iter().collect()
    }

    /// Several devices picked: how many (the toolbar's "Multiple Devices (2)").
    pub fn multiple(&self, cx: &App) -> Option<usize> {
        (!Settings::get(cx).run_devices.is_empty()).then(|| self.targets(cx).len()).filter(|n| *n > 0)
    }

    pub fn pick_device(&mut self, id: String, cx: &mut Context<Self>) {
        Settings::update(cx, |s| {
            s.run_device = id;
            s.run_devices.clear();
        });
        cx.notify();
    }

    /// Select Multiple Devices…: OK with the checked ones.
    pub fn pick_devices(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        Settings::update(cx, |s| s.run_devices = ids);
        cx.notify();
    }

    pub fn is_busy(&self) -> bool {
        self.busy
    }

    /// Something Stop can stop: a build, a launch or the app.
    pub fn can_stop(&self) -> bool {
        self.busy || self.console.as_ref().is_some_and(|c| c.active)
    }

    /// The toolbar's and Run menu's Stop: the app, or a test run.
    pub fn can_stop_any(&self, cx: &App) -> bool {
        self.can_stop() || TestService::try_global(cx).is_some_and(|t| t.read(cx).is_running())
    }

    /// Make Project: every module's selected variant.
    pub fn make_project(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let builds = SyncService::global(cx).read(cx).builds().to_vec();
        let Some((root, _)) = builds.first() else {
            cx.emit(RunEvent::Error { title: "Make Project".into(), message: "There is no Gradle build in this project.".into() });
            return;
        };
        let tasks: Vec<String> = builds
            .iter()
            .filter(|(b, _)| b == root)
            .flat_map(|(b, modules)| modules.iter().filter(|m| m.kind != run::ModuleType::Other).map(|m| m.assemble_task(&sync_service::variant_of(b, m, cx))).collect::<Vec<_>>())
            .collect();
        let root = root.clone();
        let (send, cancel) = self.start_build("Build".into(), tasks.clone(), cx);
        let r = root.clone();
        let work = cx.background_spawn(async move {
            run_app::make(&GradleBuild, &r, &tasks, &mut |l| drop(send.send(Message::Build(l.to_owned()))), &|| cancel.load(Ordering::Relaxed))
        });
        self.finish_build_when(root, work, cx);
    }

    /// Generate Signed Bundle / APK: the module's variant built and signed
    /// with the request's key.
    pub fn generate_signed(&mut self, root: PathBuf, module: run::ModuleInfo, variant: String, request: crate::domain::signing::SigningRequest, cx: &mut Context<Self>) {
        use crate::domain::signing;
        if self.busy {
            return;
        }
        let format = request.format;
        let task = signing::task(&module, &variant, format);
        let folder = signing::output_dir(&variant, &request.destination);
        let title = format.title();
        let (send, cancel) = self.start_build(title.into(), vec![task.clone()], cx);
        let r = root.clone();
        let work = cx.background_spawn(async move {
            run_app::build_signed(&GradleBuild, &r, &task, &request, &mut |l| drop(send.send(Message::Build(l.to_owned()))), &|| cancel.load(Ordering::Relaxed))
        });
        let what = match format {
            signing::PackageFormat::Bundle => "App bundle(s)",
            signing::PackageFormat::Apk => "APK(s)",
        };
        let message = format!("{what} generated successfully for module '{}' with 1 build variant: Build variant '{variant}'.", module.name());
        cx.spawn(async move |this, cx| {
            let result = work.await;
            this.update(cx, |this, cx| {
                this.drain();
                let succeeded = result.succeeded;
                this.end_build(&root, result, cx);
                if succeeded {
                    cx.emit(RunEvent::Generated { title: title.into(), message, folder });
                }
            })
            .ok();
        })
        .detach();
    }

    /// Run 'app': build, install, start; an AVD that isn't running starts first.
    pub fn run(&mut self, cx: &mut Context<Self>) {
        self.launch(false, cx);
    }

    /// Debug 'app' (Shift+F9): as Run, the app waiting for the debugger.
    pub fn debug(&mut self, cx: &mut Context<Self>) {
        self.launch(true, cx);
    }

    /// Run '…' with Coverage: for a test configuration, its tests with
    /// JaCoCo measuring them.
    pub fn run_with_coverage(&mut self, cx: &mut Context<Self>) {
        if let Some((config, mut request)) = self.selected_test(cx).is_none().then(|| self.junit_request(cx)).flatten() {
            request.coverage = true;
            TestService::global(cx).update(cx, |t, cx| t.run_configured(config.build_root, config.module, request, cx));
            return;
        }
        match self.selected_test(cx) {
            Some(name) => {
                TestService::global(cx).update(cx, |t, cx| t.run_temporary(&name, None, true, cx));
            }
            None => cx.emit(RunEvent::Error { title: "Run with Coverage".into(), message: "Coverage is measured for test configurations. Run a test from the editor's gutter first.".into() }),
        }
    }

    /// The picked configuration when it is a JUnit or Android Instrumented
    /// Tests one, and its tests.
    fn junit_request(&self, cx: &App) -> Option<(RunConfiguration, crate::domain::testing::TestRequest)> {
        use crate::domain::testing::TestKind;
        let config = self.configuration(cx)?;
        let target = config.junit.as_ref()?;
        let kind = if target.instrumented { TestKind::Instrumented } else { TestKind::Unit };
        let request = crate::domain::testing::TestRequest { kind, filters: target.filters(), name: config.name.clone(), debug_port: None, coverage: false, arguments: if target.instrumented { target.arguments.clone() } else { Vec::new() } };
        Some((config, request))
    }

    fn launch(&mut self, debug: bool, cx: &mut Context<Self>) {
        if let Some(name) = self.selected_test(cx) {
            if debug {
                crate::ui::debug_service::DebugService::global(cx).update(cx, |d, cx| d.debug_temporary(name, cx));
            } else {
                TestService::global(cx).update(cx, |t, cx| t.run_temporary(&name, None, false, cx));
            }
            return;
        }
        if self.busy {
            self.stop(cx);
        }
        let Some(config) = self.configuration(cx) else {
            cx.emit(RunEvent::Error { title: "Error running app".into(), message: "No run configuration: the project has no Android application module.".into() });
            return;
        };
        if let Some(problem) = config.junit.as_ref().and_then(|t| t.problem()) {
            cx.emit(RunEvent::Error { title: format!("Error running '{}'", config.name), message: problem.into() });
            return;
        }
        if let Some((config, request)) = self.junit_request(cx) {
            if debug && request.kind == crate::domain::testing::TestKind::Instrumented {
                cx.emit(RunEvent::Error { title: format!("Error running '{}'", config.name), message: "Debugging instrumented tests is not supported: the debugger attaches to a JVM, not to the device.".into() });
                return;
            }
            if debug {
                crate::ui::debug_service::DebugService::global(cx).update(cx, |d, cx| d.debug_configured(config.build_root, config.module, request, cx));
            } else {
                TestService::global(cx).update(cx, |t, cx| t.run_configured(config.build_root, config.module, request, cx));
            }
            return;
        }
        if config.gradle.is_some() {
            if debug {
                cx.emit(RunEvent::Error { title: format!("Error running '{}'", config.name), message: "Debugging Gradle configurations is not supported.".into() });
            } else {
                self.run_gradle(config, cx);
            }
            return;
        }
        let mut targets = self.targets(cx);
        if targets.is_empty() {
            cx.emit(RunEvent::Error { title: format!("Error running '{}'", config.name), message: "No target device found.".into() });
            return;
        }
        // The debugger attaches to one app.
        if debug {
            targets.truncate(1);
        }
        let variant = sync_service::variant_of(&config.build_root, &config.module, cx);
        let tasks = config.gradle_tasks(&variant);
        let (send, cancel) = self.start_build(format!("Run '{}'", config.name), tasks, cx);
        self.console = Some(Console { name: config.name.clone(), lines: Vec::new(), serials: targets.iter().filter_map(|d| d.serial.clone()).collect(), application_id: None, active: true });
        let on = if targets.len() > 1 { "Multiple Devices".to_owned() } else { targets[0].name.clone() };
        let launch_line = format!("{}: Launching '{}' on {on}.", chrono::Local::now().format("%m/%d %H:%M:%S"), config.name);
        let devices = self.devices.clone();
        let root = config.build_root.clone();
        for device in targets.iter().filter(|d| !d.online) {
            devices.update(cx, |d, cx| d.launch(&device.id, cx));
        }
        cx.spawn(async move |this, cx| {
            // Virtual devices that weren't running: wait for them to boot.
            let serial_of = |d: &Device, cx: &mut gpui_kit::AsyncApp| devices.read_with(cx, |m, _| m.devices().iter().find(|x| x.id == d.id && x.online).and_then(|x| x.serial.clone()));
            let mut serials: Vec<Option<String>> = targets.iter().map(|d| d.serial.clone().filter(|_| d.online)).collect();
            let deadline = Instant::now() + Duration::from_secs(300);
            while serials.iter().any(Option::is_none) && Instant::now() < deadline && !cancel.load(Ordering::Relaxed) {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                for (serial, d) in serials.iter_mut().zip(&targets) {
                    if serial.is_none() {
                        *serial = serial_of(d, cx);
                    }
                }
            }
            let adb = devices.read_with(cx, |d, _| d.adb());
            let ready: Option<Vec<String>> = serials.into_iter().collect();
            let (Some(serials), Some(adb)) = (ready, adb) else {
                this.update(cx, |this, cx| {
                    this.end_build(&root, BuildResult { cancelled: true, ..Default::default() }, cx);
                    let message = if cancel.load(Ordering::Relaxed) { "Cancelled." } else { "The device didn't come online." };
                    cx.emit(RunEvent::Error { title: format!("Error running '{}'", config.name), message: message.into() });
                })
                .ok();
                return;
            };
            this.update(cx, |this, _| {
                if let Some(console) = this.console.as_mut() {
                    console.serials = serials.clone();
                }
            })
            .ok();
            let names: Vec<String> = targets.iter().map(|d| d.name.clone()).collect();
            let c = cancel.clone();
            let cfg = config.clone();
            let launched = serials.clone();
            let (outcomes, build) = cx
                .background_spawn(async move {
                    let mut console_started = false;
                    let bridge = LaunchBridge { adb: AdbDevice { adb }, tx: send.clone() };
                    let console_tx = send.clone();
                    let several = serials.len() > 1;
                    let run_targets: Vec<run_app::Target> = serials.iter().zip(&names).map(|(serial, name)| run_app::Target { serial, name, debug }).collect();
                    run_app::run_on(
                        &GradleBuild,
                        &bridge,
                        &cfg,
                        &variant,
                        &run_targets,
                        &mut |l| drop(send.send(Message::Build(l.to_owned()))),
                        &mut |i, l| {
                            if !console_started {
                                console_started = true;
                                console_tx.send(Message::Console(ConsoleLine::Info(launch_line.clone()))).ok();
                            }
                            // Several devices: each line says which.
                            let l = match l {
                                _ if !several => l,
                                ConsoleLine::Info(t) => ConsoleLine::Info(format!("[{}] {t}", names[i])),
                                ConsoleLine::Command(t) => ConsoleLine::Command(format!("[{}] {t}", names[i])),
                                ConsoleLine::Error(t) => ConsoleLine::Error(format!("[{}] {t}", names[i])),
                            };
                            console_tx.send(Message::Console(l)).ok();
                        },
                        &|| c.load(Ordering::Relaxed),
                    )
                })
                .await;
            this.update(cx, |this, cx| {
                // Lines still in the channel come first.
                this.drain();
                let failed = outcomes.iter().all(|o| matches!(o, RunOutcome::BuildFailed));
                this.end_build(&root, build, cx);
                let running = outcomes.iter().zip(&launched).find_map(|(o, serial)| match o {
                    RunOutcome::Running { application_id, pid } => Some((serial.clone(), application_id.clone(), *pid)),
                    _ => None,
                });
                if let Some(console) = this.console.as_mut() {
                    match &running {
                        Some((_, application_id, _)) => console.application_id = Some(application_id.clone()),
                        None => console.active = false,
                    }
                }
                // After the Run window, so Logcat is the one showing.
                let show_logcat = match &running {
                    Some((serial, application_id, _)) if config.options.show_logcat => Some(RunEvent::ShowLogcat { serial: serial.clone(), application_id: application_id.clone() }),
                    _ => None,
                };
                match (&running, outcomes.iter().any(|o| matches!(o, RunOutcome::Failed))) {
                    (Some((serial, _, Some(pid))), _) if debug => {
                        cx.emit(RunEvent::ShowRun);
                        let adb = this.devices.read(cx).adb();
                        if let Some(adb) = adb {
                            cx.emit(RunEvent::Attach { name: config.name.clone(), serial: serial.clone(), pid: *pid, adb });
                        }
                    }
                    (Some(_), _) | (None, true) => cx.emit(RunEvent::ShowRun),
                    (None, false) if failed => {
                        cx.emit(RunEvent::Error { title: format!("Error running '{}'", config.name), message: "Gradle build failed. See the Build window.".into() })
                    }
                    _ => {}
                }
                if let Some(event) = show_logcat {
                    cx.emit(event);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Stop: cancels the build or launch, and stops the app.
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(tests) = TestService::try_global(cx).filter(|t| t.read(cx).is_running()) {
            tests.update(cx, |t, cx| t.stop(cx));
        }
        let Some(console) = self.console.as_mut().filter(|c| c.active) else { return };
        console.active = false;
        if let (Some(id), Some(adb)) = (console.application_id.clone(), self.devices.read(cx).adb()) {
            let serials = console.serials.clone();
            cx.background_spawn(async move {
                for serial in serials {
                    let _ = adb.run(&["-s", &serial, "shell", &format!("am force-stop {id}")], Duration::from_secs(10));
                }
            })
            .detach();
        }
        cx.notify();
    }

    // --- Internals ------------------------------------------------------

    /// A Gradle configuration: its Run line's tasks in the Run window,
    /// as IntelliJ's "Executing '…'" / "Execution finished '…'".
    fn run_gradle(&mut self, config: RunConfiguration, cx: &mut Context<Self>) {
        let run_line = config.gradle.clone().unwrap_or_default();
        let time = || chrono::Local::now().format("%H:%M:%S").to_string();
        self.console = Some(Console { name: config.name.clone(), lines: vec![ConsoleLine::Info(format!("{}: Executing '{run_line}'…", time()))], serials: Vec::new(), application_id: None, active: true });
        cx.emit(RunEvent::ShowRun);
        let (send, cancel) = self.open_channel(cx);
        let variant = sync_service::variant_of(&config.build_root, &config.module, cx);
        let cfg = config.clone();
        let work = cx.background_spawn(async move {
            run_app::run_gradle(&GradleBuild, &cfg, &variant, &mut |l| drop(send.send(Message::Console(ConsoleLine::Info(l.to_owned())))), &|| cancel.load(Ordering::Relaxed))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            this.update(cx, |this, cx| {
                this.drain();
                this.busy = false;
                if let Some(console) = this.console.as_mut() {
                    console.active = false;
                    let end = if result.cancelled { "Execution cancelled" } else { "Execution finished" };
                    console.lines.push(ConsoleLine::Info(format!("{}: {end} '{run_line}'.", time())));
                }
                if !result.cancelled {
                    crate::ui::compile_service::CompileService::global(cx).update(cx, |c, cx| c.set_build_problems(&config.build_root, result.problems, cx));
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn start_build(&mut self, title: String, tasks: Vec<String>, cx: &mut Context<Self>) -> (std::sync::mpsc::Sender<Message>, Arc<AtomicBool>) {
        // Android Studio's first line.
        let first = format!("Executing tasks: [{}]", tasks.join(", "));
        self.build = Some(BuildLog { title, lines: vec![first.into()], problems: Vec::new(), failure: None, status: BuildStatus::Running, started: chrono::Local::now(), took: None, warnings: Vec::new() });
        cx.emit(RunEvent::ShowBuild);
        self.open_channel(cx)
    }

    /// Background work's lines in, ten times a second; Stop's flag.
    fn open_channel(&mut self, cx: &mut Context<Self>) -> (std::sync::mpsc::Sender<Message>, Arc<AtomicBool>) {
        self.cancel = Arc::default();
        self.busy = true;
        cx.notify();
        let (tx, rx) = std::sync::mpsc::channel();
        self.receiver = Some(rx);
        // What the background work sends goes into the logs ten times a second.
        self._task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                let Ok(more) = this.update(cx, |this, cx| {
                    if this.drain() {
                        cx.notify();
                    }
                    for serial in std::mem::take(&mut this.cleared) {
                        cx.emit(RunEvent::ClearLogcat { serial });
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
        (tx, self.cancel.clone())
    }

    /// Appends what arrived; whether anything did.
    fn drain(&mut self) -> bool {
        let Some(rx) = &self.receiver else { return false };
        let mut any = false;
        loop {
            match rx.try_recv() {
                Ok(Message::Build(line)) => {
                    any = true;
                    if let Some(b) = self.build.as_mut() {
                        b.lines.push(line.into());
                    }
                }
                Ok(Message::ClearLogcat(serial)) => self.cleared.push(serial),
                Ok(Message::Console(line)) => {
                    any = true;
                    if let Some(c) = self.console.as_mut() {
                        c.lines.push(line);
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

    fn finish_build_when(&mut self, root: PathBuf, work: Task<BuildResult>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let result = work.await;
            this.update(cx, |this, cx| {
                this.drain();
                this.end_build(&root, result, cx);
            })
            .ok();
        })
        .detach();
    }

    fn end_build(&mut self, root: &std::path::Path, result: BuildResult, cx: &mut Context<Self>) {
        self.busy = false;
        if let Some(b) = self.build.as_mut() {
            b.status = if result.cancelled {
                BuildStatus::Cancelled
            } else if result.succeeded {
                BuildStatus::Succeeded
            } else {
                BuildStatus::Failed
            };
            b.took = Some((chrono::Local::now() - b.started).to_std().unwrap_or_default());
            b.problems = result.problems.clone();
            b.failure = result.failure.clone();
        }
        // The editors mark the build's errors, as after a compile on save.
        if !result.cancelled {
            crate::ui::compile_service::CompileService::global(cx).update(cx, |c, cx| c.set_build_problems(root, result.problems, cx));
        }
        cx.notify();
    }
}

/// A configuration by its build, module and name (several can share a module).
fn config_key(config: &RunConfiguration) -> String {
    format!("{}#{}", sync_service::variant_key(&config.build_root, &config.module.path), config.name)
}
