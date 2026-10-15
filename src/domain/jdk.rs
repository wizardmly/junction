//! The JDK Gradle runs on, as Android Studio's Gradle JDK setting
//! (Settings › Build, Execution, Deployment › Build Tools › Gradle): the
//! project's `GRADLE_LOCAL_JAVA_HOME` (`java.home` in
//! `.gradle/config.properties`, which Android Studio writes), `JAVA_HOME`,
//! Android Studio's bundled JetBrains Runtime, or any JDK found on the
//! machine; and how one is picked when none was.

use std::path::{Path, PathBuf};

/// Where a JDK was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum JdkSource {
    /// `java.home` in the project's `.gradle/config.properties`.
    ProjectLocal,
    JavaHome,
    /// Android Studio's (or IntelliJ's) bundled JetBrains Runtime.
    AndroidStudio,
    /// Installed, downloaded by an IDE (`~/.jdks`), or on the PATH.
    Installed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Jdk {
    pub home: PathBuf,
    /// `21.0.3`, from the JDK's `release` file.
    pub version: Option<String>,
    /// `JetBrains s.r.o.`, `Eclipse Adoptium`.
    pub vendor: Option<String>,
    pub source: JdkSource,
}

impl Jdk {
    pub fn major(&self) -> Option<u32> {
        self.version.as_deref().and_then(major)
    }

    /// The name the dropdown's button shows: `jbr-21 (21.0.3)`.
    pub fn short_label(&self) -> String {
        let label = self.label();
        let name = label.split("  ").next().unwrap_or_default().to_owned();
        match &self.version {
            Some(v) => format!("{name} ({v})"),
            None => name,
        }
    }

    /// The dropdown's entry, as Android Studio writes it: `jbr-21
    /// JetBrains Runtime version 21.0.3 /path`.
    pub fn label(&self) -> String {
        let kind = match self.source {
            JdkSource::ProjectLocal => "GRADLE_LOCAL_JAVA_HOME".to_owned(),
            JdkSource::JavaHome => "JAVA_HOME".to_owned(),
            JdkSource::AndroidStudio => format!("jbr-{}", self.major().map(|m| m.to_string()).unwrap_or_default()),
            JdkSource::Installed => self.home.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        };
        let vendor = match (self.source, self.vendor.as_deref()) {
            (JdkSource::AndroidStudio, _) => Some("JetBrains Runtime"),
            (_, v) => v,
        };
        let version = self.version.as_deref().map(|v| format!("version {v}"));
        let detail: Vec<&str> = [vendor, version.as_deref()].into_iter().flatten().collect();
        format!("{kind}  {}  {}", detail.join(" "), self.home.display()).replace("    ", "  ")
    }
}

/// The setting for a build, as Android Studio's `gradleJvm` keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum GradleJdk {
    /// Not set: picked by `pick`.
    #[default]
    Auto,
    ProjectLocal,
    JavaHome,
    Path(PathBuf),
}

impl GradleJdk {
    pub fn key(&self) -> String {
        match self {
            GradleJdk::Auto => String::new(),
            GradleJdk::ProjectLocal => "#GRADLE_LOCAL_JAVA_HOME".into(),
            GradleJdk::JavaHome => "#JAVA_HOME".into(),
            GradleJdk::Path(p) => p.to_string_lossy().into_owned(),
        }
    }

    pub fn from_key(key: &str) -> Self {
        match key {
            "" => GradleJdk::Auto,
            "#GRADLE_LOCAL_JAVA_HOME" => GradleJdk::ProjectLocal,
            "#JAVA_HOME" => GradleJdk::JavaHome,
            path => GradleJdk::Path(PathBuf::from(path)),
        }
    }
}

/// `21.0.3` → 21, `1.8.0_392` → 8, `17` → 17.
pub fn major(version: &str) -> Option<u32> {
    let mut parts = version.split(['.', '_', '-', '+']);
    let first: u32 = parts.next()?.parse().ok()?;
    if first == 1 { parts.next()?.parse().ok() } else { Some(first) }
}

/// A JDK's `release` file: its version and vendor.
pub fn parse_release(text: &str) -> (Option<String>, Option<String>) {
    let value = |key: &str| {
        text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('=').map(|v| v.trim().trim_matches('"').to_owned())).filter(|v| !v.is_empty())
    };
    (value("JAVA_VERSION"), value("IMPLEMENTOR"))
}

/// `java.home` of a `.gradle/config.properties` (Java properties
/// escaping: `C\:\\Program Files\\Android\\jbr`).
pub fn parse_config_properties(text: &str) -> Option<PathBuf> {
    for line in text.lines() {
        let line = line.trim_start();
        if line.starts_with('#') || line.starts_with('!') {
            continue;
        }
        let Some(rest) = line.strip_prefix("java.home") else { continue };
        let rest = rest.trim_start();
        let Some(value) = rest.strip_prefix('=').or_else(|| rest.strip_prefix(':')) else { continue };
        let mut out = String::new();
        let mut chars = value.trim().chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('t') => out.push('\t'),
                    Some('n') => out.push('\n'),
                    Some(other) => out.push(other),
                    None => {}
                }
            } else {
                out.push(c);
            }
        }
        return (!out.is_empty()).then(|| PathBuf::from(out));
    }
    None
}

/// Paths compared as the OS does (case-insensitive on Windows).
fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        let s = p.to_string_lossy().replace('\\', "/");
        let s = s.trim_end_matches('/').to_owned();
        if cfg!(windows) { s.to_lowercase() } else { s }
    };
    norm(a) == norm(b)
}

/// The JDK a build's Gradle runs on. Not set: the project's own
/// (`GRADLE_LOCAL_JAVA_HOME`), `JAVA_HOME`, Android Studio's runtime, then
/// the newest JDK 17 or later (what the Android Gradle plugin needs), then
/// the newest of any.
pub fn pick<'a>(setting: &GradleJdk, found: &'a [Jdk]) -> Option<&'a Jdk> {
    let of = |source: JdkSource| found.iter().find(|j| j.source == source);
    match setting {
        GradleJdk::ProjectLocal => of(JdkSource::ProjectLocal),
        GradleJdk::JavaHome => of(JdkSource::JavaHome),
        GradleJdk::Path(path) => found.iter().find(|j| same_path(&j.home, path)),
        GradleJdk::Auto => of(JdkSource::ProjectLocal)
            .or_else(|| of(JdkSource::JavaHome))
            .or_else(|| found.iter().filter(|j| j.source == JdkSource::AndroidStudio).max_by_key(|j| j.major()))
            .or_else(|| found.iter().filter(|j| j.major().is_some_and(|m| m >= 17)).max_by_key(|j| j.major()))
            .or_else(|| found.iter().max_by_key(|j| j.major())),
    }
}

/// Whether Gradle's output says it found no usable Java: Android Studio's
/// "Invalid Gradle JDK configuration found".
pub fn is_missing_jdk(output: &str) -> bool {
    const SIGNS: &[&str] = &[
        "JAVA_HOME is not set and no 'java' command could be found",
        "JAVA_HOME is set to an invalid directory",
        "ERROR: JAVA_HOME is not set",
        "Could not determine java version",
        "Android Gradle plugin requires Java 17",
        "Android Gradle plugin requires Java 11",
        "Unsupported class file major version",
    ];
    SIGNS.iter().any(|s| output.contains(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jdk(home: &str, version: &str, source: JdkSource) -> Jdk {
        Jdk { home: home.into(), version: Some(version.into()), vendor: None, source }
    }

    #[test]
    fn versions_and_release_files() {
        assert_eq!(major("21.0.3"), Some(21));
        assert_eq!(major("1.8.0_392"), Some(8));
        assert_eq!(major("17"), Some(17));
        assert_eq!(major("x"), None);
        let release = "IMPLEMENTOR=\"JetBrains s.r.o.\"\nJAVA_VERSION=\"21.0.3\"\nOS_NAME=\"Linux\"\n";
        assert_eq!(parse_release(release), (Some("21.0.3".into()), Some("JetBrains s.r.o.".into())));
        assert_eq!(parse_release(""), (None, None));
    }

    #[test]
    fn reads_gradle_local_java_home() {
        assert_eq!(parse_config_properties("#Thu Oct 15\njava.home=C\\:\\\\Program Files\\\\Android\\\\Android Studio\\\\jbr\n"), Some(PathBuf::from("C:\\Program Files\\Android\\Android Studio\\jbr")));
        assert_eq!(parse_config_properties("java.home = /opt/jbr"), Some(PathBuf::from("/opt/jbr")));
        assert_eq!(parse_config_properties("# java.home=/x\nother=1"), None);
    }

    #[test]
    fn picks_a_jdk() {
        let found = vec![
            jdk("/usr/lib/jvm/java-11", "11.0.2", JdkSource::Installed),
            jdk("/usr/lib/jvm/java-21", "21.0.1", JdkSource::Installed),
            jdk("/opt/android-studio/jbr", "21.0.3", JdkSource::AndroidStudio),
        ];
        // No project or JAVA_HOME: Android Studio's runtime.
        assert_eq!(pick(&GradleJdk::Auto, &found).unwrap().home, PathBuf::from("/opt/android-studio/jbr"));
        // Without Android Studio: the newest 17+.
        assert_eq!(pick(&GradleJdk::Auto, &found[..2]).unwrap().home, PathBuf::from("/usr/lib/jvm/java-21"));
        let mut with_home = found.clone();
        with_home.push(jdk("/jdk17", "17.0.9", JdkSource::JavaHome));
        assert_eq!(pick(&GradleJdk::Auto, &with_home).unwrap().home, PathBuf::from("/jdk17"));
        with_home.push(jdk("/project/jbr", "21", JdkSource::ProjectLocal));
        assert_eq!(pick(&GradleJdk::Auto, &with_home).unwrap().home, PathBuf::from("/project/jbr"));
        assert_eq!(pick(&GradleJdk::Path("/usr/lib/jvm/java-11/".into()), &found).unwrap().version.as_deref(), Some("11.0.2"));
        assert_eq!(pick(&GradleJdk::JavaHome, &found), None);
        assert_eq!(pick(&GradleJdk::Auto, &[]), None);
        assert_eq!(GradleJdk::from_key(&GradleJdk::ProjectLocal.key()), GradleJdk::ProjectLocal);
        assert_eq!(GradleJdk::from_key("/x/jdk"), GradleJdk::Path("/x/jdk".into()));
    }

    #[test]
    fn labels_and_failures() {
        let jbr = Jdk { home: "/opt/as/jbr".into(), version: Some("21.0.3".into()), vendor: Some("JetBrains s.r.o.".into()), source: JdkSource::AndroidStudio };
        assert_eq!(jbr.label(), "jbr-21  JetBrains Runtime version 21.0.3  /opt/as/jbr");
        assert_eq!(jbr.short_label(), "jbr-21 (21.0.3)");
        let home = Jdk { home: "/jdk".into(), version: Some("17.0.9".into()), vendor: Some("Eclipse Adoptium".into()), source: JdkSource::JavaHome };
        assert_eq!(home.label(), "JAVA_HOME  Eclipse Adoptium version 17.0.9  /jdk");
        assert!(is_missing_jdk("\nERROR: JAVA_HOME is not set and no 'java' command could be found in your PATH.\n"));
        assert!(!is_missing_jdk("BUILD FAILED"));
    }
}
