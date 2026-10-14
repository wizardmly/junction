//! Builds: the Gradle module a file belongs to, the task that compiles it,
//! and the build system port the use cases run tasks through.

use std::path::{Path, PathBuf};

use super::diagnostics::FileProblem;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModuleKind {
    /// An Android application or library module (AGP).
    Android,
    /// A plain JVM (Kotlin / Java) module.
    Jvm,
}

/// A Gradle module: `path` is its project path (":app", ":core:data", or
/// ":" for the root project).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Module {
    pub build_root: PathBuf,
    pub dir: PathBuf,
    pub path: String,
    pub kind: ModuleKind,
}

impl Module {
    /// The task compiling the module's main sources for a variant, without
    /// packaging: what IntelliJ's "Compile" runs, and the fastest way to
    /// get the compiler's errors.
    pub fn compile_task(&self, variant: &str) -> String {
        let prefix = if self.path == ":" { String::new() } else { self.path.clone() };
        match self.kind {
            ModuleKind::Android => format!("{prefix}:compile{}JavaWithJavac", capitalize(variant)),
            ModuleKind::Jvm => format!("{prefix}:classes"),
        }
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// The Gradle project path of a module directory under the build root.
pub fn project_path(build_root: &Path, dir: &Path) -> String {
    let rel = dir.strip_prefix(build_root).unwrap_or(dir);
    let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    format!(":{}", parts.join(":"))
}

/// What a compile left: whether it succeeded, the compiler's problems and
/// the output (for the Build tool window, or to explain a failure without
/// problems, such as a broken build script).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompileReport {
    pub succeeded: bool,
    pub problems: Vec<FileProblem>,
    pub output: String,
}

/// Runs builds. Implemented over the Gradle wrapper.
pub trait BuildSystem {
    /// The module building `file`, if any.
    fn module_of(&self, file: &Path) -> Option<Module>;
    /// Compiles the module's sources for the variant.
    fn compile(&self, module: &Module, variant: &str) -> CompileReport;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tasks_and_paths() {
        let root = Path::new("/p");
        assert_eq!(project_path(root, Path::new("/p/core/data")), ":core:data");
        assert_eq!(project_path(root, root), ":");
        let m = |path: &str, kind| Module { build_root: root.into(), dir: root.into(), path: path.into(), kind };
        assert_eq!(m(":app", ModuleKind::Android).compile_task("debug"), ":app:compileDebugJavaWithJavac");
        assert_eq!(m(":", ModuleKind::Jvm).compile_task("debug"), ":classes");
    }
}
