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
use crate::usecase::run_app::{self, ConsoleLine, RunOutcome};

pub enum RunEvent {
    /// A build started: the Build window shows.
    ShowBuild,
    /// A launch started: the Run window shows.
    ShowRun,
    /// "Error running 'app'": a balloon.
    Error { title: String, message: String },
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
    pub status: BuildStatus,
    pub started: chrono::DateTime<chrono::Local>,
    pub took: Option<Duration>,
}

/// What the Run window shows: the last launch.
pub struct Console {
    pub name: String,
    pub lines: Vec<ConsoleLine>,
    pub serial: String,
    pub application_id: Option<String>,
    /// Launching, or the app runs.
    pub active: bool,
}

enum Message {
    Build(String),
    Console(ConsoleLine),
}

pub struct RunService {
    devices: Entity<DeviceManager>,
    /// The picked configuration's module path ("build root|:app").
    config: Option<String>,
    pub build: Option<BuildLog>,
    pub console: Option<Console>,
    cancel: Arc<AtomicBool>,
    /// A build or launch is going on.
    busy: bool,
    /// Lines the background work sends.
    receiver: Option<std::sync::mpsc::Receiver<Message>>,
    _task: Option<Task<()>>,
}

impl EventEmitter<RunEvent> for RunService {}

impl RunService {
    pub fn new(devices: Entity<DeviceManager>) -> Self {
        Self { devices, config: None, build: None, console: None, cancel: Arc::default(), busy: false, receiver: None, _task: None }
    }

    /// One "Android App" configuration per application module.
    pub fn configurations(&self, cx: &App) -> Vec<RunConfiguration> {
        SyncService::try_global(cx).map(|s| s.read(cx).builds().iter().flat_map(|(root, modules)| run::run_configurations(root, modules)).collect()).unwrap_or_default()
    }

    pub fn configuration(&self, cx: &App) -> Option<RunConfiguration> {
        let all = self.configurations(cx);
        let picked = self.config.as_ref().and_then(|key| all.iter().find(|c| config_key(c) == *key).cloned());
        picked.or_else(|| all.into_iter().next())
    }

    pub fn pick_configuration(&mut self, config: &RunConfiguration, cx: &mut Context<Self>) {
        self.config = Some(config_key(config));
        cx.notify();
    }

    /// The devices a run can target: running ones, then the virtual
    /// devices that can be started.
    pub fn devices(&self, cx: &App) -> Vec<Device> {
        let mut out: Vec<Device> = self.devices.read(cx).devices().iter().filter(|d| d.wifi.is_none() && d.error.is_none() && (d.online || d.kind == Kind::Virtual)).cloned().collect();
        out.sort_by_key(|d| !d.online);
        out
    }

    pub fn device(&self, cx: &App) -> Option<Device> {
        let all = self.devices(cx);
        let saved = Settings::get(cx).run_device.clone();
        all.iter().find(|d| d.id == saved).cloned().or_else(|| all.into_iter().find(|d| d.online))
    }

    pub fn pick_device(&mut self, id: String, cx: &mut Context<Self>) {
        Settings::update(cx, |s| s.run_device = id);
        cx.notify();
    }

    pub fn is_busy(&self) -> bool {
        self.busy
    }

    /// Something Stop can stop: a build, a launch or the app.
    pub fn can_stop(&self) -> bool {
        self.busy || self.console.as_ref().is_some_and(|c| c.active)
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

    /// Run 'app': build, install, start; an AVD that isn't running starts first.
    pub fn run(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            self.stop(cx);
        }
        let Some(config) = self.configuration(cx) else {
            cx.emit(RunEvent::Error { title: "Error running app".into(), message: "No run configuration: the project has no Android application module.".into() });
            return;
        };
        let Some(device) = self.device(cx) else {
            cx.emit(RunEvent::Error { title: format!("Error running '{}'", config.name), message: "No target device found.".into() });
            return;
        };
        let variant = sync_service::variant_of(&config.build_root, &config.module, cx);
        let task = config.module.assemble_task(&variant);
        let (send, cancel) = self.start_build(format!("Run '{}'", config.name), vec![task], cx);
        self.console = Some(Console { name: config.name.clone(), lines: Vec::new(), serial: device.serial.clone().unwrap_or_default(), application_id: None, active: true });
        let launch_line = format!("{}: Launching '{}' on {}.", chrono::Local::now().format("%m/%d %H:%M:%S"), config.name, device.name);
        let devices = self.devices.clone();
        let root = config.build_root.clone();
        let start_avd = !device.online;
        if start_avd {
            devices.update(cx, |d, cx| d.launch(&device.id, cx));
        }
        let id = device.id.clone();
        cx.spawn(async move |this, cx| {
            // A virtual device that wasn't running: wait for it to boot.
            let mut serial = device.serial.clone().filter(|_| device.online);
            let deadline = Instant::now() + Duration::from_secs(300);
            while serial.is_none() && Instant::now() < deadline && !cancel.load(Ordering::Relaxed) {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                serial = devices.read_with(cx, |d, _| d.devices().iter().find(|d| d.id == id && d.online).and_then(|d| d.serial.clone()));
            }
            let adb = devices.read_with(cx, |d, _| d.adb());
            let (Some(serial), Some(adb)) = (serial, adb) else {
                this.update(cx, |this, cx| {
                    this.end_build(&root, BuildResult { cancelled: true, ..Default::default() }, cx);
                    let message = if cancel.load(Ordering::Relaxed) { "Cancelled." } else { "The device didn't come online." };
                    cx.emit(RunEvent::Error { title: format!("Error running '{}'", config.name), message: message.into() });
                })
                .ok();
                return;
            };
            let name = device.name.clone();
            let c = cancel.clone();
            let cfg = config.clone();
            let (outcome, build) = cx
                .background_spawn(async move {
                    let mut console_started = false;
                    let bridge = AdbDevice { adb };
                    let console_tx = send.clone();
                    run_app::run(
                        &GradleBuild,
                        &bridge,
                        &cfg,
                        &variant,
                        run_app::Target { serial: &serial, name: &name },
                        &mut |l| drop(send.send(Message::Build(l.to_owned()))),
                        &mut |l| {
                            if !console_started {
                                console_started = true;
                                console_tx.send(Message::Console(ConsoleLine::Info(launch_line.clone()))).ok();
                            }
                            console_tx.send(Message::Console(l)).ok();
                        },
                        &|| c.load(Ordering::Relaxed),
                    )
                })
                .await;
            this.update(cx, |this, cx| {
                // Lines still in the channel come first.
                this.drain();
                let failed = matches!(outcome, RunOutcome::BuildFailed);
                this.end_build(&root, build, cx);
                if let Some(console) = this.console.as_mut() {
                    match &outcome {
                        RunOutcome::Running { application_id, .. } => console.application_id = Some(application_id.clone()),
                        _ => console.active = false,
                    }
                }
                match outcome {
                    RunOutcome::Running { .. } | RunOutcome::Failed => cx.emit(RunEvent::ShowRun),
                    RunOutcome::BuildFailed if failed => {
                        cx.emit(RunEvent::Error { title: format!("Error running '{}'", config.name), message: "Gradle build failed. See the Build window.".into() })
                    }
                    _ => {}
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
        let Some(console) = self.console.as_mut().filter(|c| c.active) else { return };
        console.active = false;
        if let (Some(id), Some(adb)) = (console.application_id.clone(), self.devices.read(cx).adb()) {
            let serial = console.serial.clone();
            cx.background_spawn(async move {
                let _ = adb.run(&["-s", &serial, "shell", &format!("am force-stop {id}")], Duration::from_secs(10));
            })
            .detach();
        }
        cx.notify();
    }

    // --- Internals ------------------------------------------------------

    fn start_build(&mut self, title: String, tasks: Vec<String>, cx: &mut Context<Self>) -> (std::sync::mpsc::Sender<Message>, Arc<AtomicBool>) {
        self.cancel = Arc::default();
        self.busy = true;
        // Android Studio's first line.
        let first = format!("Executing tasks: [{}]", tasks.join(", "));
        self.build = Some(BuildLog { title, lines: vec![first.into()], problems: Vec::new(), status: BuildStatus::Running, started: chrono::Local::now(), took: None });
        cx.emit(RunEvent::ShowBuild);
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
        }
        // The editors mark the build's errors, as after a compile on save.
        if !result.cancelled {
            crate::ui::compile_service::CompileService::global(cx).update(cx, |c, cx| c.set_build_problems(root, result.problems, cx));
        }
        cx.notify();
    }
}

fn config_key(config: &RunConfiguration) -> String {
    sync_service::variant_key(&config.build_root, &config.module.path)
}
