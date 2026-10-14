//! The build system port over the project's Gradle wrapper: finds the
//! module of a file, runs its compile task and reads the Kotlin and Java
//! compilers' messages out of the output.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use crate::domain::build::{self, BuildSystem, CompileReport, Module, ModuleKind};
use crate::domain::diagnostics::{FileProblem, Origin, Pos, Problem, Severity};

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
        let mut command = gradle_command(&module.build_root);
        command
            .arg(&task)
            .arg("--console=plain")
            .arg(format!("-Dorg.gradle.daemon.idletimeout={DAEMON_IDLE_MS}"))
            .current_dir(&module.build_root);
        if std::env::var_os("JAVA_HOME").is_none() {
            if let Some(jbr) = studio_jbr() {
                command.env("JAVA_HOME", jbr);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
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

/// Android Studio's bundled runtime, for machines with no JAVA_HOME: the
/// one AS itself runs Gradle with.
fn studio_jbr() -> Option<PathBuf> {
    let candidates: Vec<PathBuf> = if cfg!(windows) {
        let program_files = std::env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\Program Files"));
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
        let mut v = vec![program_files.join("Android\\Android Studio\\jbr")];
        v.extend(local.map(|l| l.join("Programs\\Android Studio\\jbr")));
        v
    } else if cfg!(target_os = "macos") {
        vec![PathBuf::from("/Applications/Android Studio.app/Contents/jbr/Contents/Home")]
    } else {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        vec![PathBuf::from("/opt/android-studio/jbr"), home.join("android-studio/jbr"), PathBuf::from("/usr/local/android-studio/jbr")]
    };
    // Only when `java` isn't on the PATH either.
    let on_path = std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(if cfg!(windows) { "java.exe" } else { "java" }).is_file()));
    if on_path {
        return None;
    }
    candidates.into_iter().find(|c| c.join("bin").is_dir())
}

/// The compilers' messages in Gradle's output:
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
    let lines: Vec<&str> = output.lines().collect();
    let mut out: Vec<FileProblem> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim_end();
        if let Some(c) = kotlin.captures(line).or_else(|| kotlin_old.captures(line)) {
            let severity = if &c[1] == "e" { Severity::Error } else { Severity::Warning };
            let (row, col) = (c[3].parse::<u32>().unwrap_or(1).saturating_sub(1), c[4].parse::<u32>().unwrap_or(1).saturating_sub(1));
            out.push(problem(file_path(&c[2]), row, col, severity, c[5].trim().to_owned()));
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
    FileProblem { file, problem: Problem { start: Pos::new(line, col), end: Pos::new(line, col + 1), severity, message, origin: Origin::Compiler } }
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

FAILURE: Build failed with an exception.
";
        let p = parse_output(output);
        assert_eq!(p.len(), 5, "{p:?}");
        assert_eq!(p[0].file, PathBuf::from("/home/j/app/src/main/java/a/Main.kt"));
        assert_eq!((p[0].problem.start.line, p[0].problem.start.col), (11, 4));
        assert_eq!(p[0].problem.message, "Unresolved reference 'foo'.");
        assert_eq!(p[1].problem.severity, Severity::Warning);
        assert_eq!((p[2].problem.start.line, p[2].problem.start.col), (6, 8));
        assert_eq!((p[3].problem.start.line, p[3].problem.start.col), (19, 8));
        assert_eq!(p[3].problem.message, "cannot find symbol\nsymbol:   method bar()\nlocation: class B");
        assert_eq!(p[4].file, PathBuf::from("C:/Users/j/app/A.kt"));
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
