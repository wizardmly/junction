//! Sync Project with Gradle Files, app-wide: watches the project's build
//! files through the index, offers a sync when they changed since the last
//! one at open, syncs by itself when they change while open, and runs one
//! sync per build at a time. Composition root of the sync use case over the
//! Gradle adapter.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Global};

use crate::domain::sync::{self, ProjectSync as _};
use crate::infra::gradle_sync::GradleSync;
use crate::usecase::sync_project::{self, Next, Trigger};

pub enum SyncEvent {
    /// "Gradle files have changed since last project sync".
    Offer { root: PathBuf },
    /// A sync started or ended.
    Status,
    /// A sync finished; the libraries follow its record.
    Finished { build: String, seconds: u64, unresolved: Vec<String> },
    Failed { build: String, output: String },
}

#[derive(Default)]
pub struct SyncService {
    /// The build files' stamp when last looked at, by build.
    seen: HashMap<PathBuf, u64>,
    running: HashSet<PathBuf>,
    again: HashSet<PathBuf>,
    /// The project root and files the last look used.
    project: Option<(PathBuf, Vec<String>)>,
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

    pub fn status(&self) -> Option<String> {
        (!self.running.is_empty()).then(|| "Gradle sync running…".to_owned())
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
                            let next = sync_project::decide(GradleSync.load(&build).as_ref(), stamp, trigger);
                            (build, stamp, next)
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            this.update(cx, |this, cx| {
                for (build, stamp, next) in looks {
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
        cx.spawn(async move |this, cx| {
            let started = Instant::now();
            let b = build.clone();
            let (report, stamp) = cx
                .background_spawn(async move {
                    let report = sync_project::sync(&GradleSync, &root, &b, &files);
                    (report, GradleSync.stamp(&root, &b, &files))
                })
                .await;
            this.update(cx, |this, cx| {
                this.running.remove(&build);
                this.seen.insert(build.clone(), stamp);
                let name = build_name(&build);
                if report.succeeded {
                    cx.emit(SyncEvent::Finished { build: name, seconds: started.elapsed().as_secs(), unresolved: report.unresolved });
                } else {
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

fn build_name(build: &Path) -> String {
    build.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| build.display().to_string())
}
