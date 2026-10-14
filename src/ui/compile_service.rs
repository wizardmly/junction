//! Compile on save, app-wide: one compile per module at a time (a save
//! during one queues another), the compiler's problems by file for the
//! editors, and the status for the status bar. Composition root of the
//! compile-on-save use case over the Gradle adapter.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Global};

use crate::domain::build::Module;
use crate::domain::diagnostics::{Problem, ProblemStore};
use crate::infra::gradle::GradleBuild;
use crate::usecase::compile_on_save;

/// The variant compiled; Build Variants will pick it.
const VARIANT: &str = "debug";

pub enum CompileEvent {
    /// The compiler's problems changed (any file).
    ProblemsChanged,
    /// A compile started or ended.
    Status,
    /// A compile failed without compiler messages (a broken build script, no
    /// JDK…): the end of Gradle's output says why.
    Failed { module: String, output: String },
}

#[derive(Default)]
pub struct CompileService {
    store: ProblemStore,
    /// Module directories compiling now, and those saved again meanwhile.
    running: HashSet<PathBuf>,
    again: HashSet<PathBuf>,
}

impl EventEmitter<CompileEvent> for CompileService {}

struct GlobalCompile(Entity<CompileService>);
impl Global for GlobalCompile {}

impl CompileService {
    /// The app's service, created on first use.
    pub fn global(cx: &mut App) -> Entity<Self> {
        if let Some(g) = cx.try_global::<GlobalCompile>() {
            return g.0.clone();
        }
        let service = cx.new(|_| Self::default());
        cx.set_global(GlobalCompile(service.clone()));
        service
    }

    pub fn problems(&self, file: &Path) -> Vec<Problem> {
        self.store.problems(file)
    }

    /// "Compiling :app…" while a compile runs.
    pub fn status(&self) -> Option<String> {
        (!self.running.is_empty()).then(|| "Compiling…".to_owned())
    }

    /// A file was saved: compile its module when it is a Kotlin or Java
    /// source of a Gradle module.
    pub fn file_saved(&mut self, file: PathBuf, cx: &mut Context<Self>) {
        if !compile_on_save::compiles(&file) || !crate::settings::Settings::get(cx).compile_on_save {
            return;
        }
        cx.spawn(async move |this, cx| {
            let module = cx.background_spawn(async move { compile_on_save::module_to_compile(&GradleBuild, &file) }).await;
            if let Some(module) = module {
                this.update(cx, |this, cx| this.compile(module, cx)).ok();
            }
        })
        .detach();
    }

    fn compile(&mut self, module: Module, cx: &mut Context<Self>) {
        if !self.running.insert(module.dir.clone()) {
            self.again.insert(module.dir.clone());
            return;
        }
        cx.emit(CompileEvent::Status);
        cx.spawn(async move |this, cx| {
            let m = module.clone();
            let report = cx.background_spawn(async move { compile_on_save::compile(&GradleBuild, &m, VARIANT) }).await;
            this.update(cx, |this, cx| {
                this.running.remove(&module.dir);
                let failed_quietly = !report.succeeded && report.problems.is_empty();
                this.store.replace_under(&module.dir, report.problems);
                if failed_quietly {
                    let tail: Vec<&str> = report.output.lines().filter(|l| !l.trim().is_empty()).collect();
                    let tail = tail[tail.len().saturating_sub(12)..].join("\n");
                    cx.emit(CompileEvent::Failed { module: module.path.clone(), output: tail });
                }
                cx.emit(CompileEvent::ProblemsChanged);
                cx.emit(CompileEvent::Status);
                if this.again.remove(&module.dir) {
                    this.compile(module, cx);
                }
            })
            .ok();
        })
        .detach();
    }
}
