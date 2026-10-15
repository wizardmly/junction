//! Build and Run, as Android Studio's: Gradle modules and their build
//! variants, the "Android App" run configuration of each application
//! module, and the ports a run goes through (the build and the device).

use std::path::PathBuf;
use std::time::Duration;

use super::diagnostics::FileProblem;

/// What a Gradle module builds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModuleType {
    /// `com.android.application`: installable, gets a run configuration.
    Application,
    /// `com.android.library` and the other Android plugins.
    AndroidLibrary,
    /// Plain Kotlin / Java.
    Jvm,
    Other,
}

impl ModuleType {
    pub fn key(self) -> &'static str {
        match self {
            ModuleType::Application => "application",
            ModuleType::AndroidLibrary => "library",
            ModuleType::Jvm => "jvm",
            ModuleType::Other => "other",
        }
    }

    pub fn from_key(key: &str) -> Self {
        match key {
            "application" => ModuleType::Application,
            "library" => ModuleType::AndroidLibrary,
            "jvm" => ModuleType::Jvm,
            _ => ModuleType::Other,
        }
    }

    pub fn is_android(self) -> bool {
        matches!(self, ModuleType::Application | ModuleType::AndroidLibrary)
    }
}

/// A module of a Gradle build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleInfo {
    /// ":app", or ":" for the root project.
    pub path: String,
    pub dir: PathBuf,
    pub kind: ModuleType,
    /// Android build variants ("debug", "release", "demoDebug"…), sorted.
    pub variants: Vec<String>,
}

impl ModuleInfo {
    /// The name Android Studio gives the module's run configuration and
    /// shows in Build Variants: the last segment of its path.
    pub fn name(&self) -> String {
        match self.path.rsplit(':').next() {
            Some(name) if !name.is_empty() => name.to_owned(),
            _ => self.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "app".into()),
        }
    }

    /// The variant a module builds when none was picked: as Android
    /// Studio, the first debug one.
    pub fn default_variant(&self) -> String {
        self.variants
            .iter()
            .find(|v| *v == "debug")
            .or_else(|| self.variants.iter().find(|v| v.ends_with("Debug")))
            .or_else(|| self.variants.first())
            .cloned()
            .unwrap_or_else(|| "debug".into())
    }

    /// The task building it: `:app:assembleDebug`, `:lib:assemble`.
    pub fn assemble_task(&self, variant: &str) -> String {
        let prefix = if self.path == ":" { String::new() } else { self.path.clone() };
        if self.kind.is_android() {
            format!("{prefix}:assemble{}", capitalize(variant))
        } else {
            format!("{prefix}:assemble")
        }
    }
}

pub fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// An "Android App" run configuration: one per application module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunConfiguration {
    pub name: String,
    pub build_root: PathBuf,
    pub module: ModuleInfo,
}

/// What a build left.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BuildResult {
    pub succeeded: bool,
    /// Stopped by the user.
    pub cancelled: bool,
    pub problems: Vec<FileProblem>,
}

/// The APKs a variant's build produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApkOutput {
    pub application_id: String,
    pub apks: Vec<PathBuf>,
}

/// Runs Gradle tasks, streaming the output a line at a time.
pub trait TaskRunner {
    fn run_tasks(&self, build_root: &std::path::Path, tasks: &[String], line: &mut dyn FnMut(&str), cancelled: &dyn Fn() -> bool) -> BuildResult;
    /// The APKs of a module's variant, after it was assembled.
    fn apk_output(&self, module: &ModuleInfo, variant: &str) -> Option<ApkOutput>;
}

/// A connected device, as a run uses it.
pub trait DeviceBridge {
    /// `adb install`: how long it took, or the reason it failed
    /// (`INSTALL_FAILED_…`).
    fn install(&self, serial: &str, apks: &[PathBuf]) -> Result<Duration, String>;
    fn force_stop(&self, serial: &str, application_id: &str);
    /// The launcher activity, `pkg/.MainActivity` (TV apps: the Leanback one).
    fn launcher_activity(&self, serial: &str, application_id: &str) -> Option<String>;
    /// `am start`: the command as shown, or why it failed.
    /// With `debug`, the app waits for a debugger (`am start -D`).
    fn start(&self, serial: &str, component: &str, debug: bool) -> Result<(), String>;
    fn pid(&self, serial: &str, application_id: &str) -> Option<u32>;
}

/// The run configurations of a project's modules, by module path.
pub fn run_configurations(build_root: &std::path::Path, modules: &[ModuleInfo]) -> Vec<RunConfiguration> {
    let mut out: Vec<RunConfiguration> = modules
        .iter()
        .filter(|m| m.kind == ModuleType::Application)
        .map(|m| RunConfiguration { name: m.name(), build_root: build_root.to_path_buf(), module: m.clone() })
        .collect();
    out.sort_by(|a, b| a.module.path.cmp(&b.module.path));
    out
}

/// "Install successfully finished in 1 s 230 ms.": Android Studio's
/// duration format.
pub fn duration_label(d: Duration) -> String {
    let ms = d.as_millis();
    let (s, ms) = (ms / 1000, ms % 1000);
    match (s / 60, s % 60) {
        (0, 0) => format!("{ms} ms"),
        (0, s) => format!("{s} s {ms} ms"),
        (m, s) => format!("{m} m {s} s {ms} ms"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modules_and_configurations() {
        let m = |path: &str, kind, variants: &[&str]| ModuleInfo { path: path.into(), dir: PathBuf::from("/p").join(path.trim_start_matches(':')), kind, variants: variants.iter().map(|v| v.to_string()).collect() };
        let app = m(":app", ModuleType::Application, &["debug", "release"]);
        assert_eq!(app.assemble_task("debug"), ":app:assembleDebug");
        assert_eq!(app.default_variant(), "debug");
        let flavored = m(":tv", ModuleType::Application, &["demoDebug", "demoRelease", "fullDebug"]);
        assert_eq!(flavored.default_variant(), "demoDebug");
        assert_eq!(m(":core", ModuleType::Jvm, &[]).assemble_task("debug"), ":core:assemble");
        let configs = run_configurations(std::path::Path::new("/p"), &[flavored, m(":lib", ModuleType::AndroidLibrary, &["debug"]), app]);
        assert_eq!(configs.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["app", "tv"]);
        assert_eq!(duration_label(Duration::from_millis(1230)), "1 s 230 ms");
        assert_eq!(duration_label(Duration::from_millis(95)), "95 ms");
    }
}
