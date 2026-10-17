//! The build system port over the project's Gradle wrapper: finds the
//! module of a file, runs its compile task and reads the Kotlin and Java
//! compilers' messages out of the output.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use crate::domain::build::{self, BuildSystem, CompileReport, Module, ModuleKind};
use crate::domain::diagnostics::{FileProblem, Origin, Pos, Problem, Severity};
use crate::domain::run::{ApkOutput, BuildResult, ModuleInfo, TaskRunner};

/// How long an idle Gradle daemon stays up (IntelliJ leaves Gradle's 3
/// hours; Junction lets it go sooner to give the memory back).
const DAEMON_IDLE_MS: u32 = 15 * 60 * 1000;

pub struct GradleBuild;

impl BuildSystem for GradleBuild {
    fn module_of(&self, file: &Path) -> Option<Module> {
        let dir = file.ancestors().skip(1).find(|d| has_any(d, &["build.gradle.kts", "build.gradle"]))?.to_path_buf();
        let build_root = dir.ancestors().find(|d| has_any(d, &["settings.gradle.kts", "settings.gradle"])).unwrap_or(&dir).to_path_buf();
        let kind = if is_android(&dir) { ModuleKind::Android } else { ModuleKind::Jvm };
        Some(Module { path: build::project_path(&build_root, &dir), build_root, dir, kind })
    }

    fn compile(&self, module: &Module, variant: &str) -> CompileReport {
        let task = module.compile_task(variant);
        let mut command = gradle(&module.build_root);
        command.arg(&task);
        match command.output() {
            Ok(out) => {
                let mut output = String::from_utf8_lossy(&out.stdout).into_owned();
                output.push_str(&String::from_utf8_lossy(&out.stderr));
                CompileReport { succeeded: out.status.success(), problems: parse_output(&output), output }
            }
            Err(e) => CompileReport { succeeded: false, problems: Vec::new(), output: format!("Could not run Gradle: {e}") },
        }
    }
}

impl TaskRunner for GradleBuild {
    fn run_tasks(&self, build_root: &Path, tasks: &[String], line: &mut dyn FnMut(&str), cancelled: &dyn Fn() -> bool) -> BuildResult {
        line(&environment(build_root));
        let mut command = gradle(build_root);
        command.args(tasks);
        let (succeeded, was_cancelled, output) = stream(command, line, cancelled);
        if let Some(hint) = build::missing_dll(&output) {
            line(&hint);
        }
        BuildResult { succeeded, cancelled: was_cancelled, problems: parse_output(&output) }
    }

    fn apk_output(&self, module: &ModuleInfo, variant: &str) -> Option<ApkOutput> {
        apk_output(&module.dir.join("build/outputs/apk"), variant)
    }
}

/// Runs Gradle, passing its output (both pipes, in the order lines
/// arrive) a line at a time; stops it when `cancelled` says so. Whether it
/// succeeded, whether it was stopped, and the whole output.
pub fn stream(mut command: Command, line: &mut dyn FnMut(&str), cancelled: &dyn Fn() -> bool) -> (bool, bool, String) {
    use std::io::BufRead as _;
    command.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            let message = format!("Could not run Gradle: {e}");
            line(&message);
            return (false, false, message);
        }
    };
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let pipes: Vec<Box<dyn std::io::Read + Send>> = vec![Box::new(child.stdout.take().unwrap()), Box::new(child.stderr.take().unwrap())];
    for pipe in pipes {
        let tx = tx.clone();
        std::thread::spawn(move || {
            for l in std::io::BufReader::new(pipe).lines().map_while(Result::ok) {
                if tx.send(l).is_err() {
                    break;
                }
            }
        });
    }
    drop(tx);
    let mut output = String::new();
    let mut stopped_at: Option<std::time::Instant> = None;
    loop {
        match rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(l) => {
                line(&l);
                output.push_str(&l);
                output.push('\n');
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
        // Stopping the client cancels the build in the daemon.
        if stopped_at.is_none() && cancelled() {
            stopped_at = Some(std::time::Instant::now());
            kill_tree(&mut child);
        }
        // Something it started may still hold the pipes.
        if stopped_at.is_some_and(|t| t.elapsed() > std::time::Duration::from_secs(3)) {
            break;
        }
    }
    let was_cancelled = stopped_at.is_some();
    let succeeded = child.wait().is_ok_and(|s| s.success()) && !was_cancelled;
    (succeeded, was_cancelled, output)
}

/// Kills Gradle's client: on Windows `cmd` and the wrapper's java under it.
fn kill_tree(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        let _ = Command::new("taskkill").args(["/T", "/F", "/PID", &child.id().to_string()]).creation_flags(0x0800_0000).output();
    }
    let _ = child.kill();
}

/// The variant's `output-metadata.json` under `build/outputs/apk` (AGP
/// 4.1+): its application ID and APK files.
pub fn apk_output(apk_dir: &Path, variant: &str) -> Option<ApkOutput> {
    fn find(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() && depth < 3 {
                find(&path, depth + 1, out);
            } else if path.file_name().is_some_and(|n| n == "output-metadata.json") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    find(apk_dir, 0, &mut files);
    files.into_iter().find_map(|file| {
        let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).ok()?).ok()?;
        if json["variantName"].as_str() != Some(variant) {
            return None;
        }
        let dir = file.parent()?;
        let apks: Vec<PathBuf> = json["elements"].as_array()?.iter().filter_map(|e| e["outputFile"].as_str()).map(|f| dir.join(f)).filter(|p| p.is_file()).collect();
        (!apks.is_empty()).then(|| ApkOutput { application_id: json["applicationId"].as_str().unwrap_or_default().to_owned(), apks })
    })
}

/// The build and module of a file from the build files alone, for before
/// the first sync: the nearest directory with a build script holds the
/// module, the nearest with a settings script above it is the build.
pub fn module_from_files(file: &Path) -> Option<(PathBuf, crate::domain::run::ModuleInfo)> {
    use crate::domain::run::{ModuleInfo, ModuleType};
    const BUILD: &[&str] = &["build.gradle.kts", "build.gradle"];
    const SETTINGS: &[&str] = &["settings.gradle.kts", "settings.gradle"];
    let module_dir = file.ancestors().skip(1).find(|d| has_any(d, BUILD) || has_any(d, SETTINGS))?;
    let root = module_dir.ancestors().find(|d| has_any(d, SETTINGS)).unwrap_or(module_dir);
    let rel = module_dir.strip_prefix(root).ok()?;
    let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let path = format!(":{}", parts.join(":"));
    let kind = if !is_android(module_dir) {
        ModuleType::Jvm
    } else if ["build.gradle.kts", "build.gradle"].iter().filter_map(|n| std::fs::read_to_string(module_dir.join(n)).ok()).any(|t| t.contains("com.android.application") || t.contains("android.application")) {
        ModuleType::Application
    } else {
        ModuleType::AndroidLibrary
    };
    Some((root.to_path_buf(), ModuleInfo { path, dir: module_dir.to_path_buf(), kind, variants: Vec::new() }))
}

fn has_any(dir: &Path, names: &[&str]) -> bool {
    names.iter().any(|n| dir.join(n).is_file())
}

/// An Android module: a manifest in its main source set, or the Android
/// Gradle plugin applied in its build file.
fn is_android(dir: &Path) -> bool {
    if dir.join("src/main/AndroidManifest.xml").is_file() {
        return true;
    }
    let text = ["build.gradle.kts", "build.gradle"].iter().find_map(|n| std::fs::read_to_string(dir.join(n)).ok()).unwrap_or_default();
    text.contains("com.android.") || text.contains("plugins.android.") || text.contains("android {")
}

/// Each build's Gradle JDK setting (Settings › Build Tools › Gradle), by
/// build root key; the UI keeps it current.
static JDK_SETTINGS: std::sync::RwLock<std::collections::BTreeMap<String, String>> = std::sync::RwLock::new(std::collections::BTreeMap::new());

pub fn set_jdk_settings(settings: std::collections::BTreeMap<String, String>) {
    *JDK_SETTINGS.write().unwrap() = settings;
}

/// The JDK a build's Gradle runs on: its setting, else Android Studio's
/// choice (see `domain::jdk::pick`).
pub fn jdk_for(build_root: &Path) -> Option<crate::domain::jdk::Jdk> {
    use crate::domain::jdk::{pick, GradleJdk};
    let key = crate::domain::diagnostics::file_key(build_root);
    let setting = JDK_SETTINGS.read().unwrap().get(&key).map(|k| GradleJdk::from_key(k)).unwrap_or_default();
    let found = super::jdk_discovery::discover(&super::jdk_discovery::Env::current(), Some(build_root));
    pick(&setting, &found).cloned()
}

/// The build's second line: the JDK Gradle runs on and, on Windows, the
/// system's version, for reading a failure.
fn environment(build_root: &Path) -> String {
    let jdk = jdk_for(build_root).map(|j| j.label()).unwrap_or_else(|| "none found (Gradle uses java on the PATH)".into());
    match os_version() {
        Some(os) => format!("Gradle JDK: {jdk} · {os}"),
        None => format!("Gradle JDK: {jdk}"),
    }
}

/// `Windows 10.0.19045`, as the system reports it (`RtlGetVersion`, which
/// unlike `GetVersionEx` doesn't depend on the program's manifest).
#[cfg(windows)]
fn os_version() -> Option<String> {
    #[repr(C)]
    struct OsVersionInfo {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform: u32,
        service_pack: [u16; 128],
    }
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(info: *mut OsVersionInfo) -> i32;
    }
    let mut info = OsVersionInfo { size: std::mem::size_of::<OsVersionInfo>() as u32, major: 0, minor: 0, build: 0, platform: 0, service_pack: [0; 128] };
    (unsafe { RtlGetVersion(&mut info) } == 0).then(|| format!("Windows {}.{}.{}", info.major, info.minor, info.build))
}

#[cfg(not(windows))]
fn os_version() -> Option<String> {
    None
}

/// Gradle for a build: its wrapper, plain output, the daemon's idle
/// timeout, the build's Gradle JDK, no console window on Windows.
pub fn gradle(build_root: &Path) -> Command {
    let mut command = gradle_command(build_root);
    command.arg("--console=plain").arg(format!("-Dorg.gradle.daemon.idletimeout={DAEMON_IDLE_MS}")).current_dir(build_root);
    if let Some(jdk) = jdk_for(build_root) {
        command.env("JAVA_HOME", &jdk.home);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
}

/// The wrapper in the build root, else `gradle` on the PATH.
fn gradle_command(root: &Path) -> Command {
    let wrapper = root.join(if cfg!(windows) { "gradlew.bat" } else { "gradlew" });
    if wrapper.is_file() {
        if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.arg("/C").arg(wrapper);
            return c;
        }
        return Command::new(wrapper);
    }
    Command::new(if cfg!(windows) { "gradle.bat" } else { "gradle" })
}

/// The compilers' messages in Gradle's output:
/// - AAPT: `ERROR: /path/res/layout/a.xml:10: AAPT: error: message`,
/// - Kotlin 2: `e: file:///path/A.kt:12:5 message` (`w:` for warnings),
/// - Kotlin 1: `e: /path/A.kt: (12, 5): message`,
/// - javac: `/path/A.java:12: error: message`, then the source line and a
///   caret line marking the column, then detail lines (`symbol: …`).
pub fn parse_output(output: &str) -> Vec<FileProblem> {
    static KOTLIN: OnceLock<regex::Regex> = OnceLock::new();
    static KOTLIN_OLD: OnceLock<regex::Regex> = OnceLock::new();
    static JAVA: OnceLock<regex::Regex> = OnceLock::new();
    let kotlin = KOTLIN.get_or_init(|| regex::Regex::new(r"^([ew]): (?:file://)?(.+?\.kts?):(\d+):(\d+) (.*)$").unwrap());
    let kotlin_old = KOTLIN_OLD.get_or_init(|| regex::Regex::new(r"^([ew]): (.+?\.kts?): \((\d+), (\d+)\): (.*)$").unwrap());
    let java = JAVA.get_or_init(|| regex::Regex::new(r"^(.+?\.java):(\d+): (error|warning): (.*)$").unwrap());
    static AAPT: OnceLock<regex::Regex> = OnceLock::new();
    let aapt = AAPT.get_or_init(|| regex::Regex::new(r"^(?:ERROR|WARNING):\s*(.+?\.xml):(\d+):\s*AAPT: (error|warning): (.*)$").unwrap());
    let lines: Vec<&str> = output.lines().collect();
    let mut out: Vec<FileProblem> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim_end();
        if let Some(c) = kotlin.captures(line).or_else(|| kotlin_old.captures(line)) {
            let severity = if &c[1] == "e" { Severity::Error } else { Severity::Warning };
            let (row, col) = (c[3].parse::<u32>().unwrap_or(1).saturating_sub(1), c[4].parse::<u32>().unwrap_or(1).saturating_sub(1));
            out.push(problem(file_path(&c[2]), row, col, severity, c[5].trim().to_owned()));
        } else if let Some(c) = aapt.captures(line) {
            let severity = if &c[3] == "error" { Severity::Error } else { Severity::Warning };
            let row = c[2].parse::<u32>().unwrap_or(1).saturating_sub(1);
            out.push(problem(file_path(&c[1]), row, 0, severity, c[4].trim().to_owned()));
        } else if let Some(c) = java.captures(line) {
            let severity = if &c[3] == "error" { Severity::Error } else { Severity::Warning };
            let row = c[2].parse::<u32>().unwrap_or(1).saturating_sub(1);
            let mut message = c[4].trim().to_owned();
            // The source line, then the caret under the column.
            let caret = lines.get(i + 2).filter(|l| l.trim() == "^");
            let col = caret.map_or(0, |l| l.chars().take_while(|ch| *ch != '^').count() as u32);
            let mut j = i + if caret.is_some() { 3 } else { 1 };
            while let Some(detail) = lines.get(j).filter(|l| l.starts_with("  ") && !java.is_match(l)) {
                message.push('\n');
                message.push_str(detail.trim());
                j += 1;
            }
            out.push(problem(file_path(&c[1]), row, col, severity, message));
            i = j;
            continue;
        }
        i += 1;
    }
    out.dedup();
    out
}

fn problem(file: PathBuf, line: u32, col: u32, severity: Severity, message: String) -> FileProblem {
    FileProblem { file, problem: Problem { start: Pos::new(line, col), end: Pos::new(line, col + 1), severity, message, origin: Origin::Compiler, fix: None } }
}

/// A path from a `file://` URI or plain: `/C:/x` is `C:/x` on Windows.
fn file_path(raw: &str) -> PathBuf {
    let raw = raw.replace("%20", " ");
    let bytes = raw.as_bytes();
    if bytes.len() > 2 && bytes[0] == b'/' && bytes[2] == b':' {
        return PathBuf::from(&raw[1..]);
    }
    PathBuf::from(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_module_from_build_files() {
        let dir = std::env::temp_dir().join(format!("junction-module-files-{}", std::process::id()));
        let test = dir.join("feature/login/src/test/java/a");
        std::fs::create_dir_all(&test).unwrap();
        std::fs::write(dir.join("settings.gradle.kts"), "include(\":feature:login\")").unwrap();
        std::fs::write(dir.join("feature/login/build.gradle.kts"), "plugins { id(\"com.android.library\") }").unwrap();
        let (root, module) = module_from_files(&test.join("LoginTest.kt")).unwrap();
        assert_eq!((root, module.path.as_str(), module.kind), (dir.clone(), ":feature:login", crate::domain::run::ModuleType::AndroidLibrary));
        // A single-project build: the root module.
        std::fs::remove_file(dir.join("feature/login/build.gradle.kts")).unwrap();
        assert_eq!(module_from_files(&test.join("LoginTest.kt")).unwrap().1.path, ":");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_kotlin_and_javac_messages() {
        let output = "\
> Task :app:compileDebugKotlin FAILED
e: file:///home/j/app/src/main/java/a/Main.kt:12:5 Unresolved reference 'foo'.
w: file:///home/j/app/src/main/java/a/Main.kt:3:1 Parameter 'x' is never used
e: /home/j/app/src/main/java/a/Old.kt: (7, 9): Type mismatch: inferred type is String but Int was expected
/home/j/app/src/main/java/a/B.java:20: error: cannot find symbol
        bar();
        ^
  symbol:   method bar()
  location: class B
e: file:///C:/Users/j/app/A.kt:1:2 x
ERROR: /home/j/app/src/main/res/layout/main.xml:10: AAPT: error: attribute android:foo not found.

FAILURE: Build failed with an exception.
";
        let p = parse_output(output);
        assert_eq!(p.len(), 6, "{p:?}");
        assert_eq!(p[0].file, PathBuf::from("/home/j/app/src/main/java/a/Main.kt"));
        assert_eq!((p[0].problem.start.line, p[0].problem.start.col), (11, 4));
        assert_eq!(p[0].problem.message, "Unresolved reference 'foo'.");
        assert_eq!(p[1].problem.severity, Severity::Warning);
        assert_eq!((p[2].problem.start.line, p[2].problem.start.col), (6, 8));
        assert_eq!((p[3].problem.start.line, p[3].problem.start.col), (19, 8));
        assert_eq!(p[3].problem.message, "cannot find symbol\nsymbol:   method bar()\nlocation: class B");
        assert_eq!(p[4].file, PathBuf::from("C:/Users/j/app/A.kt"));
        assert_eq!((p[5].problem.start.line, p[5].problem.message.as_str()), (9, "attribute android:foo not found."));
    }

    #[test]
    fn finds_the_module() {
        let dir = std::env::temp_dir().join(format!("junction-gradle-{}", std::process::id()));
        let src = dir.join("feature/home/src/main/java/a");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(dir.join("settings.gradle.kts"), "include(\":feature:home\")").unwrap();
        std::fs::write(dir.join("feature/home/build.gradle.kts"), "plugins { id(\"com.android.library\") }").unwrap();
        let file = src.join("Home.kt");
        std::fs::write(&file, "").unwrap();
        let m = GradleBuild.module_of(&file).unwrap();
        assert_eq!(m.path, ":feature:home");
        assert_eq!(m.kind, ModuleKind::Android);
        assert_eq!(m.build_root, dir);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
