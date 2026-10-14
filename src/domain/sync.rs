//! Project sync, as Android Studio's "Sync Project with Gradle Files": the
//! build resolves every module's compile classpath (downloading what is
//! missing, sources included) and the IDE takes the exact dependency list
//! from it. Junction's is lightweight: no project model is rebuilt, only
//! the libraries change, and it runs when the build files change or on
//! request, never just because a project was opened.

use std::fmt;
use std::path::{Path, PathBuf};

/// A resolved external module: `group:artifact:version`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Coordinate {
    pub group: String,
    pub artifact: String,
    pub version: String,
}

impl Coordinate {
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.trim().splitn(3, ':');
        let (group, artifact, version) = (parts.next()?, parts.next()?, parts.next()?);
        if group.is_empty() || artifact.is_empty() || version.is_empty() || version.contains(char::is_whitespace) {
            return None;
        }
        Some(Self { group: group.into(), artifact: artifact.into(), version: version.into() })
    }
}

impl fmt::Display for Coordinate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.group, self.artifact, self.version)
    }
}

/// What a sync of one build left.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// The build configured; a failed one keeps the previous record.
    pub succeeded: bool,
    /// Every external module on the modules' compile classpaths, sorted.
    pub dependencies: Vec<Coordinate>,
    /// "Failed to resolve: x:y:1.0", once each.
    pub unresolved: Vec<String>,
    /// The build's output, to explain a failure.
    pub output: String,
}

/// The last successful sync of a build, kept across sessions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncRecord {
    /// [`ProjectSync::stamp`] of the build files it read.
    pub stamp: u64,
    pub dependencies: Vec<Coordinate>,
}

/// Syncs builds and keeps their records. Implemented over Gradle.
pub trait ProjectSync {
    /// The builds of a project (directories holding a settings file),
    /// from its files (relative, with `/`).
    fn build_roots(&self, root: &Path, files: &[String]) -> Vec<PathBuf>;
    /// Changes whenever a build file of `build_root` does.
    fn stamp(&self, root: &Path, build_root: &Path, files: &[String]) -> u64;
    fn sync(&self, build_root: &Path) -> SyncReport;
    fn load(&self, build_root: &Path) -> Option<SyncRecord>;
    fn save(&self, build_root: &Path, record: &SyncRecord);
}

/// Files a Gradle sync reads: build scripts, settings, version catalogs,
/// properties and the wrapper's version.
pub fn is_build_file(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(name, "build.gradle" | "build.gradle.kts" | "settings.gradle" | "settings.gradle.kts" | "gradle.properties" | "gradle-wrapper.properties")
        || name.ends_with(".versions.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinates_and_build_files() {
        let c = Coordinate::parse("androidx.core:core-ktx:1.13.1").unwrap();
        assert_eq!(c.artifact, "core-ktx");
        assert_eq!(c.to_string(), "androidx.core:core-ktx:1.13.1");
        assert!(Coordinate::parse("a:b").is_none());
        assert!(is_build_file("app/build.gradle.kts"));
        assert!(is_build_file("gradle/libs.versions.toml"));
        assert!(is_build_file("gradle/wrapper/gradle-wrapper.properties"));
        assert!(!is_build_file("app/src/main/AndroidManifest.xml"));
    }
}
