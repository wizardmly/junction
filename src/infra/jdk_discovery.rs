//! The JDKs on this machine, as Android Studio's Gradle JDK list finds
//! them: the project's `GRADLE_LOCAL_JAVA_HOME`, `JAVA_HOME`, Android
//! Studio's bundled runtime wherever it's installed (JetBrains Toolbox
//! included), JDKs an IDE downloaded (`~/.jdks`), installed ones (Adoptium,
//! Zulu, Corretto, Microsoft, Oracle, `/usr/lib/jvm`, sdkman…), and the
//! `java` on the PATH.

use std::path::{Path, PathBuf};

use crate::domain::jdk::{parse_config_properties, parse_release, Jdk, JdkSource};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    Mac,
    Linux,
}

/// What discovery reads from the environment.
pub struct Env {
    pub os: Os,
    pub home: Option<PathBuf>,
    pub java_home: Option<PathBuf>,
    pub path: Vec<PathBuf>,
    /// Windows: `%ProgramFiles%`, `%LOCALAPPDATA%`.
    pub program_files: Vec<PathBuf>,
    pub local_app_data: Option<PathBuf>,
}

impl Env {
    pub fn current() -> Self {
        let var = |k: &str| std::env::var_os(k).map(PathBuf::from);
        let os = if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Mac
        } else {
            Os::Linux
        };
        let mut program_files: Vec<PathBuf> = ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"].iter().filter_map(|k| var(k)).collect();
        program_files.dedup();
        Env {
            os,
            home: var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }),
            java_home: var("JAVA_HOME"),
            path: std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default(),
            program_files,
            local_app_data: var("LOCALAPPDATA"),
        }
    }

    fn java(&self) -> &'static str {
        if self.os == Os::Windows { "java.exe" } else { "java" }
    }
}

/// The directories under `base` matching `pattern` (`*`, `prefix*` or a
/// plain name), one segment at a time.
fn expand(base: &Path, pattern: &[&str]) -> Vec<PathBuf> {
    let Some((first, rest)) = pattern.split_first() else { return vec![base.to_path_buf()] };
    let matches: Vec<PathBuf> = if let Some(prefix) = first.strip_suffix('*') {
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(base)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(prefix) && e.path().is_dir())
            .map(|e| e.path())
            .collect();
        dirs.sort();
        dirs
    } else {
        let dir = base.join(first);
        if dir.is_dir() { vec![dir] } else { Vec::new() }
    };
    matches.iter().flat_map(|m| expand(m, rest)).collect()
}

/// A JDK at `home` (or its macOS bundle's `Contents/Home`), if it has a
/// `java` to run.
fn jdk_at(env: &Env, home: &Path, source: JdkSource) -> Option<Jdk> {
    let home = [home.join("Contents/Home"), home.to_path_buf()].into_iter().find(|h| h.join("bin").join(env.java()).is_file())?;
    let (version, vendor) = std::fs::read_to_string(home.join("release")).map(|t| parse_release(&t)).unwrap_or_default();
    Some(Jdk { home, version, vendor, source })
}

/// Android Studio's (and IntelliJ's) installations: their `jbr`.
fn studio_runtimes(env: &Env) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut add = |base: &Path, pattern: &[&str]| out.extend(expand(base, pattern));
    match env.os {
        Os::Windows => {
            for pf in &env.program_files {
                add(pf, &["Android", "Android Studio*", "jbr"]);
                add(pf, &["Android Studio*", "jbr"]);
                add(pf, &["JetBrains", "*", "jbr"]);
            }
            if let Some(local) = &env.local_app_data {
                add(local, &["Programs", "Android Studio*", "jbr"]);
                add(local, &["JetBrains", "Toolbox", "apps", "AndroidStudio", "ch-*", "*", "jbr"]);
                add(local, &["JetBrains", "Toolbox", "apps", "android-studio*", "jbr"]);
                add(local, &["Programs", "IntelliJ IDEA*", "jbr"]);
            }
        }
        Os::Mac => {
            for apps in [PathBuf::from("/Applications"), env.home.clone().unwrap_or_default().join("Applications")] {
                add(&apps, &["Android Studio*", "Contents", "jbr"]);
                add(&apps, &["IntelliJ IDEA*", "Contents", "jbr"]);
            }
            if let Some(home) = &env.home {
                add(home, &["Library", "Application Support", "JetBrains", "Toolbox", "apps", "AndroidStudio", "ch-*", "*", "Android Studio*", "Contents", "jbr"]);
            }
        }
        Os::Linux => {
            add(Path::new("/opt"), &["android-studio*", "jbr"]);
            add(Path::new("/usr/local"), &["android-studio*", "jbr"]);
            add(Path::new("/snap"), &["android-studio", "current", "jbr"]);
            add(Path::new("/snap"), &["android-studio", "current", "android-studio", "jbr"]);
            if let Some(home) = &env.home {
                add(home, &["android-studio*", "jbr"]);
                add(home, &[".local", "share", "JetBrains", "Toolbox", "apps", "android-studio*", "jbr"]);
                add(home, &[".local", "share", "JetBrains", "Toolbox", "apps", "AndroidStudio", "ch-*", "*", "jbr"]);
            }
        }
    }
    out
}

/// Installed and downloaded JDKs.
fn installed(env: &Env) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut add = |base: &Path, pattern: &[&str]| out.extend(expand(base, pattern));
    if let Some(home) = &env.home {
        add(home, &[".jdks", "*"]);
        add(home, &[".sdkman", "candidates", "java", "*"]);
    }
    match env.os {
        Os::Windows => {
            for pf in &env.program_files {
                for vendor in ["Java", "Eclipse Adoptium", "Eclipse Foundation", "AdoptOpenJDK", "Microsoft", "Zulu", "Amazon Corretto", "BellSoft", "Semeru", "OpenJDK"] {
                    add(pf, &[vendor, "*"]);
                }
            }
        }
        Os::Mac => {
            add(Path::new("/Library/Java/JavaVirtualMachines"), &["*"]);
            if let Some(home) = &env.home {
                add(home, &["Library", "Java", "JavaVirtualMachines", "*"]);
            }
            add(Path::new("/opt/homebrew/opt"), &["openjdk*", "libexec", "openjdk.jdk"]);
        }
        Os::Linux => {
            add(Path::new("/usr/lib/jvm"), &["*"]);
            add(Path::new("/usr/java"), &["*"]);
            add(Path::new("/opt"), &["java*"]);
            add(Path::new("/opt"), &["jdk*"]);
        }
    }
    out
}

/// Every JDK found, the project's first; each home once.
pub fn discover(env: &Env, build_root: Option<&Path>) -> Vec<Jdk> {
    let mut found: Vec<Jdk> = Vec::new();
    let mut push = |jdk: Option<Jdk>| {
        let Some(jdk) = jdk else { return };
        let key = std::fs::canonicalize(&jdk.home).unwrap_or_else(|_| jdk.home.clone());
        if !found.iter().any(|j| std::fs::canonicalize(&j.home).unwrap_or_else(|_| j.home.clone()) == key) {
            found.push(jdk);
        }
    };
    if let Some(root) = build_root {
        let local = std::fs::read_to_string(root.join(".gradle").join("config.properties")).ok().and_then(|t| parse_config_properties(&t));
        push(local.and_then(|home| jdk_at(env, &home, JdkSource::ProjectLocal)));
    }
    push(env.java_home.as_deref().and_then(|home| jdk_at(env, home, JdkSource::JavaHome)));
    for home in studio_runtimes(env) {
        push(jdk_at(env, &home, JdkSource::AndroidStudio));
    }
    for home in installed(env) {
        push(jdk_at(env, &home, JdkSource::Installed));
    }
    // The `java` on the PATH: its JDK.
    for dir in &env.path {
        let java = dir.join(env.java());
        if java.is_file() {
            let real = std::fs::canonicalize(&java).unwrap_or(java);
            if let Some(home) = real.parent().and_then(Path::parent) {
                push(jdk_at(env, home, JdkSource::Installed));
            }
        }
    }
    found
}

/// A folder the user chose (Add JDK…): a JDK, or why not.
pub fn check(home: &Path) -> Result<Jdk, String> {
    jdk_at(&Env::current(), home, JdkSource::Installed).ok_or_else(|| format!("The selected directory is not a valid JDK home: {}", home.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_jdk(home: &Path, version: &str) {
        std::fs::create_dir_all(home.join("bin")).unwrap();
        std::fs::write(home.join("bin/java"), "").unwrap();
        std::fs::write(home.join("release"), format!("JAVA_VERSION=\"{version}\"\nIMPLEMENTOR=\"Vendor\"\n")).unwrap();
    }

    #[test]
    fn finds_jdks() {
        let root = std::env::temp_dir().join(format!("junction-jdks-{}", std::process::id()));
        let home = root.join("home");
        fake_jdk(&home.join(".jdks/temurin-17"), "17.0.9");
        fake_jdk(&home.join(".local/share/JetBrains/Toolbox/apps/AndroidStudio/ch-0/243.1/jbr"), "21.0.3");
        fake_jdk(&home.join("java-home"), "11.0.2");
        // A folder that isn't a JDK.
        std::fs::create_dir_all(home.join(".jdks/broken")).unwrap();
        let project = root.join("project");
        std::fs::create_dir_all(project.join(".gradle")).unwrap();
        std::fs::write(project.join(".gradle/config.properties"), format!("java.home={}\n", home.join(".jdks/temurin-17").display())).unwrap();
        let env = Env { os: Os::Linux, home: Some(home.clone()), java_home: Some(home.join("java-home")), path: vec![home.join("java-home/bin")], program_files: vec![], local_app_data: None };
        let found = discover(&env, Some(&project));
        let got: Vec<(JdkSource, Option<&str>)> = found.iter().map(|j| (j.source, j.version.as_deref())).collect();
        // The project's JDK once, though ~/.jdks has it too; JAVA_HOME's
        // once, though it's on the PATH.
        assert_eq!(got[..3], [(JdkSource::ProjectLocal, Some("17.0.9")), (JdkSource::JavaHome, Some("11.0.2")), (JdkSource::AndroidStudio, Some("21.0.3"))]);
        assert_eq!(found.iter().filter(|j| j.home.ends_with("temurin-17")).count(), 1);
        assert!(found.iter().all(|j| !j.home.ends_with("broken")));
        assert!(check(&home.join(".jdks/broken")).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
