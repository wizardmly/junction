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

/// Windows' STATUS_DLL_NOT_FOUND as exit values print it: a program the
/// build started couldn't load a DLL it imports (the loader's "The code
/// execution cannot proceed because … was not found").
const DLL_NOT_FOUND: &[&str] = &["-1073741515", "3221225781", "0xC0000135", "0xc0000135"];

/// What to say when a program the build ran exited for a missing DLL:
/// Gradle's `Process 'command '…'' finished with non-zero exit value
/// -1073741515` names it.
pub fn missing_dll(output: &str) -> Option<String> {
    let line = output.lines().find(|l| DLL_NOT_FOUND.iter().any(|c| l.contains(c)))?;
    let program = line.split_once("'command '").and_then(|(_, rest)| rest.split_once("''")).map(|(cmd, _)| cmd.to_owned());
    Some(match program {
        Some(program) => format!("{program} could not start: a DLL it needs is missing (STATUS_DLL_NOT_FOUND, 0xC0000135)."),
        None => "A program the build started could not start: a DLL it needs is missing (STATUS_DLL_NOT_FOUND, 0xC0000135). The line above names it.".to_owned(),
    })
}

/// Gradle's "* What went wrong:" section, its indent taken off; none when
/// the output has none.
pub fn failure_summary(output: &str) -> Option<String> {
    let (_, rest) = output.split_once("* What went wrong:")?;
    let section = rest.split("\n* ").next().unwrap_or(rest);
    let lines: Vec<&str> = section.lines().map(str::trim_end).skip_while(|l| l.trim().is_empty()).collect();
    let indent = lines.iter().filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    let text = lines.iter().map(|l| l.get(indent..).unwrap_or("").to_owned()).collect::<Vec<_>>().join("\n");
    let text = text.trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// The Gradle version a wrapper downloads: `distributionUrl` in
/// `gradle/wrapper/gradle-wrapper.properties`
/// (`…/gradle-8.14.3-bin.zip` is 8.14.3).
pub fn wrapper_version(properties: &str) -> Option<String> {
    let url = properties.lines().find_map(|l| l.trim().strip_prefix("distributionUrl")?.trim_start().strip_prefix('=').map(str::trim))?;
    let file = url.rsplit('/').next()?;
    let version = file.strip_prefix("gradle-")?.strip_suffix(".zip")?;
    let version = version.strip_suffix("-bin").or_else(|| version.strip_suffix("-all")).unwrap_or(version);
    (!version.is_empty()).then(|| version.to_owned())
}

/// How a build's Gradle is run (IntelliJ's "Distribution"): its wrapper,
/// or a Gradle installed in a folder ("Specified location").
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum GradleDistribution {
    #[default]
    Wrapper,
    Local(PathBuf),
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_the_wrappers_version() {
        let text = "distributionBase=GRADLE_USER_HOME\ndistributionUrl=https\\://services.gradle.org/distributions/gradle-8.14.3-bin.zip\n";
        assert_eq!(super::wrapper_version(text).as_deref(), Some("8.14.3"));
        assert_eq!(super::wrapper_version("distributionUrl = https://x/gradle-9.0-rc-1-all.zip").as_deref(), Some("9.0-rc-1"));
        assert_eq!(super::wrapper_version("zipStoreBase=GRADLE_USER_HOME"), None);
    }

    #[test]
    fn reads_what_went_wrong() {
        let out = "> Task :app:packageRelease FAILED\n\nFAILURE: Build failed with an exception.\n\n* What went wrong:\nA problem was found with the configuration of task ':app:packageRelease'.\n  - Property 'storeFile' specifies file 'D:\\app\\keystore.jks' which doesn't exist.\n    \n* Try:\n> Run with --stacktrace\n";
        assert_eq!(
            super::failure_summary(out).as_deref(),
            Some("A problem was found with the configuration of task ':app:packageRelease'.\n  - Property 'storeFile' specifies file 'D:\\app\\keystore.jks' which doesn't exist.")
        );
        assert_eq!(super::failure_summary("BUILD SUCCESSFUL"), None);
    }

    #[test]
    fn names_the_program_missing_a_dll() {
        let out = "> Task :app:mergeDebugResources FAILED\nProcess 'command 'C:\\sdk\\aapt2.exe'' finished with non-zero exit value -1073741515\n";
        assert_eq!(super::missing_dll(out).as_deref(), Some("C:\\sdk\\aapt2.exe could not start: a DLL it needs is missing (STATUS_DLL_NOT_FOUND, 0xC0000135)."));
        assert!(super::missing_dll("exit code 0xC0000135").unwrap().contains("The line above"));
        assert_eq!(super::missing_dll("BUILD FAILED"), None);
    }

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
