//! Sync Project with Gradle Files, app-wide: watches the project's build
//! files through the index, offers a sync when they changed since the last
//! one at open, syncs by itself when they change while open, and runs one
//! sync per build at a time. Composition root of the sync use case over the
//! Gradle adapter.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Global, Task};

use crate::ui::run_service::{BuildLog, BuildStatus};

use crate::domain::run::ModuleInfo;
use crate::domain::sync::{self, ProjectSync as _};
use crate::infra::gradle_sync::{self as gradle_sync, GradleSync};
use crate::usecase::sync_project::{self, Next, Trigger};

pub enum SyncEvent {
    /// "Gradle files have changed since last project sync".
    Offer { root: PathBuf },
    /// A sync started or ended.
    Status,
    /// The builds' modules changed (a sync, the project opened).
    Modules,
    /// A sync finished; the libraries follow its record.
    Finished { build: String, seconds: u64, unresolved: Vec<String> },
    Failed { build: String, output: String },
    /// A sync started: the Build window shows its Sync tab.
    Started,
}

#[derive(Default)]
pub struct SyncService {
    /// The build files' stamp when last looked at, by build.
    seen: HashMap<PathBuf, u64>,
    running: HashSet<PathBuf>,
    again: HashSet<PathBuf>,
    /// The project root and files the last look used.
    project: Option<(PathBuf, Vec<String>)>,
    /// Each build and its modules: from its last sync, else guessed from
    /// the build files.
    builds: Vec<(PathBuf, Vec<ModuleInfo>)>,
    /// The Build window's Sync tab: the last sync's tree and output.
    pub log: Option<BuildLog>,
    /// Stops the running syncs.
    cancel: Arc<AtomicBool>,
    /// Output lines the running syncs send, drained ten times a second.
    lines: Option<std::sync::mpsc::Receiver<String>>,
    sender: Option<std::sync::mpsc::Sender<String>>,
    _drain: Option<Task<()>>,
}

impl EventEmitter<SyncEvent> for SyncService {}

struct GlobalSync(Entity<SyncService>);
impl Global for GlobalSync {}

impl SyncService {
    pub fn global(cx: &mut App) -> Entity<Self> {
        if let Some(g) = cx.try_global::<GlobalSync>() {
            return g.0.clone();
        }
        let service = cx.new(|_| Self::default());
        cx.set_global(GlobalSync(service.clone()));
        service
    }

    pub fn try_global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalSync>().map(|g| g.0.clone())
    }

    /// The Sync tab's Clear All: its output text.
    pub fn clear_output(&mut self, cx: &mut Context<Self>) {
        if let Some(log) = self.log.as_mut() {
            log.lines.clear();
            cx.notify();
        }
    }

    /// The Sync tab's Stop.
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if !self.running.is_empty() {
            self.cancel.store(true, Ordering::Relaxed);
            self.again.clear();
            cx.notify();
        }
    }

    /// A sync starts: with none running, the Sync tab begins anew, as
    /// Android Studio's "<project>: syncing…" node.
    fn start_log(&mut self, build: &Path, cx: &mut Context<Self>) -> std::sync::mpsc::Sender<String> {
        // Another build of the project is syncing too: the same tab.
        if self.running.len() > 1
            && let Some(sender) = &self.sender
        {
            return sender.clone();
        }
        self.cancel = Arc::default();
        let name = self.project.as_ref().map(|(root, _)| build_name(root)).unwrap_or_else(|| build_name(build));
        self.log = Some(BuildLog { title: name, lines: Vec::new(), problems: Vec::new(), failure: None, status: BuildStatus::Running, started: chrono::Local::now(), took: None, warnings: Vec::new() });
        let (tx, rx) = std::sync::mpsc::channel();
        self.lines = Some(rx);
        self.sender = Some(tx.clone());
        self._drain = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                let Ok(more) = this.update(cx, |this, cx| {
                    if this.drain() {
                        cx.notify();
                    }
                    this.lines.is_some()
                }) else {
                    break;
                };
                if !more {
                    break;
                }
            }
        }));
        cx.emit(SyncEvent::Started);
        tx
    }

    fn drain(&mut self) -> bool {
        let Some(rx) = &self.lines else { return false };
        let mut any = false;
        loop {
            match rx.try_recv() {
                Ok(line) => {
                    any = true;
                    if let Some(log) = self.log.as_mut() {
                        log.lines.push(line.into());
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.lines = None;
                    break;
                }
            }
        }
        any
    }

    /// A build's sync ended: with none left running, the tab's root node
    /// shows how it went.
    fn end_log(&mut self, report: &crate::domain::sync::SyncReport, cx: &mut Context<Self>) {
        self.drain();
        let Some(log) = self.log.as_mut() else { return };
        log.warnings.extend(report.unresolved.iter().cloned());
        if report.cancelled {
            log.status = BuildStatus::Cancelled;
        } else if !report.succeeded {
            log.status = BuildStatus::Failed;
            if log.failure.is_none() {
                log.failure = crate::domain::build::failure_summary(&report.output).or_else(|| Some(report.output.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("Gradle sync failed").to_owned()));
            }
        }
        if self.running.is_empty() {
            if log.status == BuildStatus::Running {
                log.status = BuildStatus::Succeeded;
            }
            log.took = Some((chrono::Local::now() - log.started).to_std().unwrap_or_default());
            // The senders go: the drain loop ends once it has read the rest.
            self.sender = None;
        }
        cx.notify();
    }

    pub fn status(&self) -> Option<String> {
        (!self.running.is_empty()).then(|| "Gradle sync running…".to_owned())
    }

    pub fn builds(&self) -> &[(PathBuf, Vec<ModuleInfo>)] {
        &self.builds
    }

    /// The build and module a module directory belongs to.
    pub fn module_at(&self, dir: &Path) -> Option<(&Path, &ModuleInfo)> {
        let key = crate::domain::diagnostics::file_key(dir);
        self.builds.iter().find_map(|(b, modules)| modules.iter().find(|m| crate::domain::diagnostics::file_key(&m.dir) == key).map(|m| (b.as_path(), m)))
    }

    /// Whether the project has a Gradle build to sync.
    pub fn has_builds(&self) -> bool {
        self.project.as_ref().is_some_and(|(_, files)| files.iter().any(|f| sync::is_build_file(f)))
    }

    /// The project's files changed (or were first listed): looks at its
    /// builds' files. Only build files matter; the rest is ignored.
    pub fn files_changed(&mut self, root: PathBuf, files: Vec<String>, cx: &mut Context<Self>) {
        let build_files: Vec<String> = files.into_iter().filter(|f| sync::is_build_file(f)).collect();
        if self.project.as_ref().is_some_and(|(r, _)| *r != root) {
            self.seen.clear();
        }
        self.project = Some((root.clone(), build_files.clone()));
        let seen = self.seen.clone();
        cx.spawn(async move |this, cx| {
            let looks = cx
                .background_spawn(async move {
                    GradleSync
                        .build_roots(&root, &build_files)
                        .into_iter()
                        .map(|build| {
                            let stamp = GradleSync.stamp(&root, &build, &build_files);
                            let trigger = seen.get(&build).map_or(Trigger::Opened, |s| Trigger::Seen(*s));
                            let record = GradleSync.load(&build);
                            let next = sync_project::decide(record.as_ref(), stamp, trigger);
                            let modules = record.map(|r| r.modules).filter(|m| !m.is_empty()).unwrap_or_else(|| gradle_sync::guess_modules(&root, &build, &build_files));
                            (build, stamp, next, modules)
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            this.update(cx, |this, cx| {
                let builds: Vec<(PathBuf, Vec<ModuleInfo>)> = looks.iter().map(|(b, _, _, m)| (b.clone(), m.clone())).collect();
                if builds != this.builds {
                    this.builds = builds;
                    cx.emit(SyncEvent::Modules);
                }
                for (build, stamp, next, _) in looks {
                    this.seen.insert(build.clone(), stamp);
                    match next {
                        Next::Nothing => {}
                        Next::Offer => cx.emit(SyncEvent::Offer { root: build }),
                        Next::Sync => this.sync(build, cx),
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Sync Project with Gradle Files: every build of the project.
    pub fn sync_all(&mut self, cx: &mut Context<Self>) {
        let Some((root, files)) = self.project.clone() else { return };
        for build in GradleSync.build_roots(&root, &files) {
            self.sync(build, cx);
        }
    }

    pub fn sync(&mut self, build: PathBuf, cx: &mut Context<Self>) {
        let Some((root, files)) = self.project.clone() else { return };
        if !self.running.insert(build.clone()) {
            self.again.insert(build);
            return;
        }
        cx.emit(SyncEvent::Status);
        let send = self.start_log(&build, cx);
        let cancel = self.cancel.clone();
        let variants = self.variants(&build, cx);
        cx.spawn(async move |this, cx| {
            let started = Instant::now();
            let b = build.clone();
            let (report, stamp) = cx
                .background_spawn(async move {
                    let report = sync_project::sync(&GradleSync, &root, &b, &files, &variants, &mut |l| drop(send.send(l.to_owned())), &|| cancel.load(Ordering::Relaxed));
                    (report, GradleSync.stamp(&root, &b, &files))
                })
                .await;
            this.update(cx, |this, cx| {
                this.running.remove(&build);
                this.end_log(&report, cx);
                this.seen.insert(build.clone(), stamp);
                let name = build_name(&build);
                if report.succeeded {
                    if let Some(entry) = this.builds.iter_mut().find(|(b, _)| *b == build) {
                        if !report.modules.is_empty() && entry.1 != report.modules {
                            entry.1 = report.modules.clone();
                            cx.emit(SyncEvent::Modules);
                        }
                    }
                    cx.emit(SyncEvent::Finished { build: name, seconds: started.elapsed().as_secs(), unresolved: report.unresolved });
                } else if !report.cancelled {
                    // What went wrong, without Gradle's "* Try:" advice.
                    let output = report.output.split("* Try:").next().unwrap_or_default();
                    let lines: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty() && !l.starts_with("JUNCTION_")).collect();
                    let tail = lines[lines.len().saturating_sub(12)..].join("\n");
                    cx.emit(SyncEvent::Failed { build: name, output: tail });
                }
                cx.emit(SyncEvent::Status);
                if this.again.remove(&build) {
                    this.sync(build, cx);
                }
            })
            .ok();
        })
        .detach();
    }
}

impl SyncService {
    /// The variants picked in Build Variants for a build's modules.
    fn variants(&self, build: &Path, cx: &App) -> Vec<(String, String)> {
        let modules = self.builds.iter().find(|(b, _)| b == build).map(|(_, m)| m.as_slice()).unwrap_or_default();
        modules.iter().filter(|m| m.kind.is_android()).map(|m| (m.path.clone(), variant_of(build, m, cx))).collect()
    }
}

/// The variant each Android module of the project builds, by its folder.
pub fn active_variants(cx: &mut App) -> Vec<(PathBuf, String)> {
    let service = SyncService::global(cx);
    let service = service.read(cx);
    service.builds.iter().flat_map(|(build, modules)| modules.iter().filter(|m| m.kind.is_android()).map(|m| (m.dir.clone(), variant_of(build, m, cx)))).collect()
}

/// The variant a module builds: the one picked in Build Variants, else its
/// default (the first debug one).
pub fn variant_of(build: &Path, module: &ModuleInfo, cx: &App) -> String {
    let picked = crate::settings::Settings::get(cx).build_variants.get(&variant_key(build, &module.path)).cloned();
    picked.filter(|v| module.variants.is_empty() || module.variants.contains(v)).unwrap_or_else(|| module.default_variant())
}

pub fn variant_key(build: &Path, module: &str) -> String {
    format!("{}|{module}", crate::domain::diagnostics::file_key(build))
}

fn build_name(build: &Path) -> String {
    build.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| build.display().to_string())
}
