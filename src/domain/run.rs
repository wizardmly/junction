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

/// An "Android App" run configuration: one per application module unless
/// the user made their own (Edit Configurations).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunConfiguration {
    pub name: String,
    pub build_root: PathBuf,
    pub module: ModuleInfo,
    pub options: LaunchOptions,
    /// A Gradle configuration: its Run line (tasks and arguments,
    /// `clean :app:lint --info`); none for an Android App one.
    pub gradle: Option<String>,
}

/// What Launch starts, as the configuration's Launch Options say.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LaunchTarget {
    /// The launcher activity.
    #[default]
    DefaultActivity,
    /// An activity by class name (`.SettingsActivity` or the full name).
    Activity(String),
    /// Install only.
    Nothing,
}

/// The configuration's Installation and Launch Options.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LaunchOptions {
    /// Deploy: "Default APK"; off, "Nothing".
    #[serde(default = "yes")]
    pub deploy: bool,
    #[serde(default)]
    pub launch: LaunchTarget,
    /// Launch Flags: added to `am start` (`--es key value`, `-W`).
    #[serde(default)]
    pub flags: String,
    /// Miscellaneous › Logcat: "Show logcat automatically" (Android
    /// Studio's `SHOW_LOGCAT_AUTOMATICALLY`): Run and Debug open the app's
    /// Logcat tab.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub show_logcat: bool,
    /// "Clear log before launch" (`CLEAR_LOGCAT`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub clear_logcat: bool,
    /// Before launch: what runs, in order, before the APK is installed.
    #[serde(default = "make_only", skip_serializing_if = "is_make_only")]
    pub before_launch: Vec<BeforeLaunch>,
}

/// A Before launch task of a run configuration.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BeforeLaunch {
    /// Gradle-aware Make: the module's assemble task for the variant.
    Make,
    /// Run Gradle task: task names as typed (`clean :app:lint`).
    Gradle(String),
    /// Run External tool: a command line, run in the build's directory.
    External(String),
}

impl BeforeLaunch {
    /// The row in the Before launch list, as Android Studio words it.
    pub fn label(&self) -> String {
        match self {
            BeforeLaunch::Make => "Gradle-aware Make".into(),
            BeforeLaunch::Gradle(tasks) => format!("Run Gradle task '{}'", tasks.trim()),
            BeforeLaunch::External(command) => format!("Run External tool '{}'", command.trim()),
        }
    }
}

fn make_only() -> Vec<BeforeLaunch> {
    vec![BeforeLaunch::Make]
}

fn is_make_only(tasks: &Vec<BeforeLaunch>) -> bool {
    *tasks == make_only()
}

/// The words of a command line: split at spaces, except inside double
/// quotes (which are dropped).
pub fn split_command_line(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let (mut quoted, mut any) = (false, false);
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    words.push(std::mem::take(&mut word));
                    any = false;
                }
            }
            c => {
                word.push(c);
                any = true;
            }
        }
    }
    if any {
        words.push(word);
    }
    words
}

impl RunConfiguration {
    /// The Gradle tasks its Before launch list runs, in order: the first
    /// line of the Build window.
    pub fn gradle_tasks(&self, variant: &str) -> Vec<String> {
        self.options
            .before_launch
            .iter()
            .flat_map(|task| match task {
                BeforeLaunch::Make => vec![self.module.assemble_task(variant)],
                BeforeLaunch::Gradle(tasks) => split_command_line(tasks),
                BeforeLaunch::External(_) => Vec::new(),
            })
            .collect()
    }
}

fn yes() -> bool {
    true
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self { deploy: true, launch: LaunchTarget::DefaultActivity, flags: String::new(), show_logcat: false, clear_logcat: false, before_launch: make_only() }
    }
}

/// A configuration as the project keeps it: its module by Gradle path.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SavedRunConfiguration {
    pub name: String,
    /// ":app".
    pub module: String,
    /// A Gradle configuration's Run line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradle: Option<String>,
    #[serde(flatten)]
    pub options: LaunchOptions,
}

/// The `am start` Android Studio runs: the component, as a launcher
/// intent, waiting for the debugger with `-D`, then the Launch Flags.
pub fn start_command(component: &str, debug: bool, flags: &str) -> String {
    let wait = if debug { " -D" } else { "" };
    let flags = if flags.trim().is_empty() { String::new() } else { format!(" {}", flags.trim()) };
    format!("am start -n \"{component}\" -a android.intent.action.MAIN -c android.intent.category.LAUNCHER{wait}{flags}")
}

/// `pkg/.Activity` for a Specified Activity: a name starting with `.` is
/// the package's, a name with `/` is a component already.
pub fn activity_component(application_id: &str, activity: &str) -> String {
    let activity = activity.trim();
    if activity.contains('/') {
        activity.to_owned()
    } else {
        format!("{application_id}/{activity}")
    }
}

/// What a build left.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BuildResult {
    pub succeeded: bool,
    /// Stopped by the user.
    pub cancelled: bool,
    pub problems: Vec<FileProblem>,
    /// Gradle's "What went wrong" when the build failed: Android Studio's
    /// failure node in the Build window.
    pub failure: Option<String>,
}

/// The APKs a variant's build produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApkOutput {
    pub application_id: String,
    pub apks: Vec<PathBuf>,
}

/// Runs Gradle tasks, streaming the output a line at a time.
pub trait TaskRunner {
    /// Runs the tasks with Gradle properties (`-Pname=value`) set.
    fn run_tasks(&self, build_root: &std::path::Path, tasks: &[String], properties: &[(String, String)], line: &mut dyn FnMut(&str), cancelled: &dyn Fn() -> bool) -> BuildResult;
    /// Run External tool: runs `command` (program, then arguments) in
    /// `dir`, streaming its output.
    fn run_program(&self, dir: &std::path::Path, command: &[String], line: &mut dyn FnMut(&str), cancelled: &dyn Fn() -> bool) -> BuildResult;
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
    /// `flags`: the configuration's Launch Flags, added to the command.
    fn start(&self, serial: &str, component: &str, debug: bool, flags: &str) -> Result<(), String>;
    fn pid(&self, serial: &str, application_id: &str) -> Option<u32>;
    /// `adb logcat -c`: Clear log before launch.
    fn clear_logcat(&self, serial: &str);
}

/// The run configurations of a build: the saved ones (whose module is
/// still there), else one per application module by module path.
pub fn run_configurations(build_root: &std::path::Path, modules: &[ModuleInfo], saved: &[SavedRunConfiguration]) -> Vec<RunConfiguration> {
    if !saved.is_empty() {
        return saved
            .iter()
            .filter_map(|s| {
                // A Gradle configuration runs in the build's root project.
                let module = match &s.gradle {
                    Some(_) => ModuleInfo { path: ":".into(), dir: build_root.to_path_buf(), kind: ModuleType::Other, variants: Vec::new() },
                    None => modules.iter().find(|m| m.path == s.module && m.kind == ModuleType::Application)?.clone(),
                };
                Some(RunConfiguration { name: s.name.clone(), build_root: build_root.to_path_buf(), module, options: s.options.clone(), gradle: s.gradle.clone() })
            })
            .collect();
    }
    let mut out: Vec<RunConfiguration> = modules
        .iter()
        .filter(|m| m.kind == ModuleType::Application)
        .map(|m| RunConfiguration { name: m.name(), build_root: build_root.to_path_buf(), module: m.clone(), options: LaunchOptions::default(), gradle: None })
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
        let configs = run_configurations(std::path::Path::new("/p"), &[flavored, m(":lib", ModuleType::AndroidLibrary, &["debug"]), app], &[]);
        assert_eq!(configs.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["app", "tv"]);
        // Saved ones instead, those whose module is gone left out.
        let mut settings = LaunchOptions { launch: LaunchTarget::Activity(".SettingsActivity".into()), ..Default::default() };
        settings.flags = "-W".into();
        let saved = [
            SavedRunConfiguration { name: "Settings".into(), module: ":app".into(), gradle: None, options: settings.clone() },
            SavedRunConfiguration { name: "Old".into(), module: ":gone".into(), gradle: None, options: LaunchOptions::default() },
            SavedRunConfiguration { name: "lint".into(), module: String::new(), gradle: Some(":app:lint".into()), options: LaunchOptions { before_launch: Vec::new(), ..Default::default() } },
        ];
        let configs = run_configurations(std::path::Path::new("/p"), &[m(":app", ModuleType::Application, &["debug"])], &saved);
        assert_eq!((configs.len(), configs[0].name.as_str(), &configs[0].options), (2, "Settings", &settings));
        assert_eq!((configs[1].gradle.as_deref(), configs[1].module.dir.as_path()), (Some(":app:lint"), std::path::Path::new("/p")));
        assert_eq!(activity_component("com.ex", ".SettingsActivity"), "com.ex/.SettingsActivity");
        assert_eq!(activity_component("com.ex", "com.lib.Scan"), "com.ex/com.lib.Scan");
        assert_eq!(activity_component("com.ex", "other/.Main"), "other/.Main");
        // Saved before the options existed: deploy, default activity.
        let read: SavedRunConfiguration = serde_json::from_str(r#"{"name":"a","module":":app"}"#).unwrap();
        assert_eq!(read.options, LaunchOptions::default());
        assert_eq!(duration_label(Duration::from_millis(1230)), "1 s 230 ms");
        assert_eq!(duration_label(Duration::from_millis(95)), "95 ms");
    }

    #[test]
    fn before_launch_tasks() {
        assert_eq!(split_command_line(r#"clean  :app:lint "a b"c"#), ["clean", ":app:lint", "a bc"]);
        assert_eq!(split_command_line(r#"x "" y"#), ["x", "", "y"]);
        let module = ModuleInfo { path: ":app".into(), dir: "/p/app".into(), kind: ModuleType::Application, variants: vec!["debug".into()] };
        let mut config = RunConfiguration { name: "app".into(), build_root: "/p".into(), module, options: LaunchOptions::default(), gradle: None };
        assert_eq!(config.gradle_tasks("debug"), [":app:assembleDebug"]);
        config.options.before_launch = vec![BeforeLaunch::Gradle("clean".into()), BeforeLaunch::External("echo hi".into()), BeforeLaunch::Make];
        assert_eq!(config.gradle_tasks("debug"), ["clean", ":app:assembleDebug"]);
        assert_eq!(BeforeLaunch::Gradle(":app:lint".into()).label(), "Run Gradle task ':app:lint'");
        // Only a changed list is saved.
        let saved = |options: LaunchOptions| serde_json::to_string(&SavedRunConfiguration { name: "a".into(), module: ":app".into(), gradle: None, options }).unwrap();
        assert!(!saved(LaunchOptions::default()).contains("before_launch"));
        let json = saved(config.options.clone());
        let read: SavedRunConfiguration = serde_json::from_str(&json).unwrap();
        assert_eq!(read.options.before_launch, config.options.before_launch);
    }
}
