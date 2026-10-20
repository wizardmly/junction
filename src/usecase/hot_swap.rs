//! Apply HotSwap (IntelliJ's floating "Code changed" toolbar, Run ›
//! Debugging Actions › Reload Changed Classes; Android Studio's Apply Code
//! Changes while debugging): the modules of the sources saved during the
//! session compile, then the classes they wrote since the last reload
//! replace the loaded ones.

use std::path::PathBuf;
use std::time::SystemTime;

use crate::domain::build::{BuildSystem, Module};
use crate::domain::diagnostics::FileProblem;
use crate::domain::hotswap::{self, ClassFiles, Dexer, HotSwapOutcome};
use crate::domain::run::TaskRunner;
use crate::usecase::debug_session::DebugSession;

/// What compiles: each changed source's module, main or test classes.
pub fn targets(build: &dyn BuildSystem, sources: &[PathBuf]) -> Vec<(Module, bool)> {
    let mut targets: Vec<(Module, bool)> = Vec::new();
    for source in sources {
        if let Some(module) = crate::usecase::compile_on_save::module_to_compile(build, source) {
            let target = (module, hotswap::is_test_source(source));
            if !targets.contains(&target) {
                targets.push(target);
            }
        }
    }
    targets
}

/// The outcome, and the compiler's problems when the compile failed.
pub struct Applied {
    pub outcome: HotSwapOutcome,
    pub problems: Vec<FileProblem>,
}

/// Compiles the targets (each with its module's variant), then reloads
/// what they wrote after `since`.
pub fn apply(runner: &dyn TaskRunner, targets: &[(Module, bool, String)], session: &DebugSession, files: &dyn ClassFiles, since: SystemTime, dexer: Option<&dyn Dexer>) -> Applied {
    let mut roots = Vec::new();
    let mut builds: Vec<(PathBuf, Vec<String>)> = Vec::new();
    for (module, test, variant) in targets {
        let task = hotswap::compile_task(module, variant, *test);
        match builds.iter_mut().find(|(root, _)| *root == module.build_root) {
            Some((_, tasks)) => tasks.push(task),
            None => builds.push((module.build_root.clone(), vec![task])),
        }
        roots.extend(hotswap::output_roots(module, variant));
    }
    for (root, tasks) in &builds {
        let result = runner.run_tasks(root, tasks, &[], &mut |_| {}, &|| false);
        if !result.succeeded {
            let reason = result.failure.unwrap_or_else(|| "Compilation failed".into());
            return Applied { outcome: HotSwapOutcome::Failed(format!("Compilation failed: {}", reason.lines().next().unwrap_or_default())), problems: result.problems };
        }
    }
    roots.dedup();
    let changed = files.changed_since(&roots, since);
    Applied { outcome: session.hot_swap(&changed, files, dexer), problems: Vec::new() }
}
