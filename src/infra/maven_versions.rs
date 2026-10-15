//! The versions published of a module, as Android Studio looks them up for
//! its newer-version check: Google's Maven (`group-index.xml`) for Google's
//! groups, Maven Central's `maven-metadata.xml` for the rest, and the
//! Gradle Plugin Portal for plugins. Answers are kept on disk for a day
//! (a failed lookup for an hour), so files open without waiting on the
//! network.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::domain::dependency_versions::VersionSource;

const FRESH: Duration = Duration::from_secs(24 * 3600);
const RETRY: Duration = Duration::from_secs(3600);

pub struct MavenVersions {
    cache: Option<PathBuf>,
}

impl MavenVersions {
    pub fn new() -> Self {
        Self { cache: crate::index::libraries::cache_dir().map(|d| d.join("maven-versions")) }
    }

    fn cache_file(&self, module: &str, plugin: bool) -> Option<PathBuf> {
        let name: String = module.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '~' }).collect();
        Some(self.cache.as_ref()?.join(if plugin { format!("plugin~{name}") } else { name }))
    }

    /// The cached answer: `Some(None)` for a failed lookup still remembered.
    fn cached(&self, module: &str, plugin: bool) -> Option<Option<Vec<String>>> {
        let text = std::fs::read_to_string(self.cache_file(module, plugin)?).ok()?;
        let mut lines = text.lines();
        let stamp: u64 = lines.next()?.parse().ok()?;
        let age = now().saturating_sub(stamp);
        let versions: Vec<String> = lines.map(str::to_owned).collect();
        let fresh = if versions.is_empty() { RETRY } else { FRESH };
        (age < fresh.as_secs()).then(|| (!versions.is_empty()).then_some(versions))
    }

    fn store(&self, module: &str, plugin: bool, versions: &[String]) {
        let Some(file) = self.cache_file(module, plugin) else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut text = now().to_string();
        for v in versions {
            text.push('\n');
            text.push_str(v);
        }
        let _ = std::fs::write(file, text);
    }

    fn fetch(&self, module: &str, plugin: bool) -> Vec<String> {
        let (group, artifact) = if plugin { (module.to_owned(), format!("{module}.gradle.plugin")) } else {
            match module.split_once(':') {
                Some((g, a)) => (g.to_owned(), a.to_owned()),
                None => return Vec::new(),
            }
        };
        let path = group.replace('.', "/");
        if is_google(&group) {
            let found = get(&format!("https://dl.google.com/android/maven2/{path}/group-index.xml")).map(|xml| group_index_versions(&xml, &artifact)).unwrap_or_default();
            if !found.is_empty() {
                return found;
            }
        }
        let repository = if plugin { "https://plugins.gradle.org/m2" } else { "https://repo1.maven.org/maven2" };
        get(&format!("{repository}/{path}/{artifact}/maven-metadata.xml")).map(|xml| metadata_versions(&xml)).unwrap_or_default()
    }
}

impl VersionSource for MavenVersions {
    fn versions(&self, module: &str, plugin: bool) -> Option<Vec<String>> {
        if let Some(answer) = self.cached(module, plugin) {
            return answer;
        }
        let found = self.fetch(module, plugin);
        self.store(module, plugin, &found);
        (!found.is_empty()).then_some(found)
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Groups published on Google's Maven repository.
fn is_google(group: &str) -> bool {
    ["androidx.", "com.android.", "com.google.android.", "com.google.firebase", "com.google.gms", "com.google.mlkit", "com.google.ar"]
        .iter()
        .any(|p| group.starts_with(p) || group == p.trim_end_matches('.'))
}

fn get(url: &str) -> Option<String> {
    let response = crate::hosting::agent_for(url).get(url).timeout(Duration::from_secs(10)).call().ok()?;
    response.into_string().ok()
}

/// `<core-ktx versions="1.0.0,1.1.0"/>` in a group index.
fn group_index_versions(xml: &str, artifact: &str) -> Vec<String> {
    let open = format!("<{artifact} ");
    let Some(at) = xml.find(&open) else { return Vec::new() };
    let rest = &xml[at..];
    let Some(v) = rest.find("versions=\"") else { return Vec::new() };
    let list = &rest[v + 10..];
    list[..list.find('"').unwrap_or(0)].split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect()
}

/// The `<version>`s of a `maven-metadata.xml`.
fn metadata_versions(xml: &str) -> Vec<String> {
    xml.split("<version>").skip(1).filter_map(|s| s.split("</version>").next()).map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_both_formats() {
        let index = r#"<?xml version='1.0' encoding='UTF-8'?><androidx.core><core versions="1.0.0,1.1.0"/><core-ktx versions="1.0.0,1.13.1"/></androidx.core>"#;
        assert_eq!(group_index_versions(index, "core-ktx"), ["1.0.0", "1.13.1"]);
        assert_eq!(group_index_versions(index, "core"), ["1.0.0", "1.1.0"]);
        let meta = "<metadata><versioning><latest>4.12.0</latest><versions><version>4.11.0</version><version>4.12.0</version></versions></versioning></metadata>";
        assert_eq!(metadata_versions(meta), ["4.11.0", "4.12.0"]);
        assert!(is_google("androidx.core") && is_google("com.google.firebase") && !is_google("com.squareup.okhttp3"));
    }
}
