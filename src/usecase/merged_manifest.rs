//! What the Merged Manifest merges for a module's manifest, as the Gradle
//! plugin finds it: the build type's overlay (`src/debug`), the main
//! manifest, the manifests of the library modules it depends on
//! (`project(":lib")`, and theirs), and its build file's values.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

use crate::domain::manifest_merge::{self, BuildValues, ManifestSource, Merged};

/// The module a manifest belongs to: the folder holding its `src`.
pub fn module_of(manifest: &Path) -> Option<(PathBuf, String)> {
    let set_dir = manifest.parent()?;
    let src = set_dir.parent()?;
    (src.file_name()? == "src").then(|| (src.parent().map(Path::to_path_buf).unwrap_or_default(), set_dir.file_name().unwrap_or_default().to_string_lossy().into_owned()))
}

fn capture(re: &Regex, text: &str) -> Option<String> {
    re.captures(text).map(|c| c[1].to_owned())
}

/// The build file's values (Kotlin DSL or Groovy).
pub fn build_values(text: &str) -> BuildValues {
    static APPLICATION_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\bapplicationId\s*=?\s*["']([\w.]+)["']"#).unwrap());
    static NAMESPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\bnamespace\s*=?\s*["']([\w.]+)["']"#).unwrap());
    static MIN_SDK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bminSdk(?:Version)?\s*(?:=|\(|\s)\s*(\d+)").unwrap());
    static TARGET_SDK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\btargetSdk(?:Version)?\s*(?:=|\(|\s)\s*(\d+)").unwrap());
    static VERSION_CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bversionCode\s*=?\s*(\d+)").unwrap());
    static VERSION_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\bversionName\s*=?\s*["']([^"']+)["']"#).unwrap());
    // `manifestPlaceholders["key"] = "value"`, `manifestPlaceholders += mapOf("key" to "value")`, Groovy's `[key: "value"]`.
    static INDEXED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"manifestPlaceholders\[\s*["'](\w+)["']\s*\]\s*=\s*["']([^"']*)["']"#).unwrap());
    static MAP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"manifestPlaceholders[^\n]*?(?:mapOf\(|\[)([^\])\n]*)"#).unwrap());
    static PAIR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"["']?(\w+)["']?\s*(?:to|:)\s*["']([^"']*)["']"#).unwrap());
    let mut placeholders: Vec<(String, String)> = INDEXED.captures_iter(text).map(|c| (c[1].to_owned(), c[2].to_owned())).collect();
    for m in MAP.captures_iter(text) {
        placeholders.extend(PAIR.captures_iter(&m[1]).map(|c| (c[1].to_owned(), c[2].to_owned())));
    }
    BuildValues {
        application_id: capture(&APPLICATION_ID, text),
        namespace: capture(&NAMESPACE, text),
        min_sdk: capture(&MIN_SDK, text),
        target_sdk: capture(&TARGET_SDK, text),
        version_code: capture(&VERSION_CODE, text),
        version_name: capture(&VERSION_NAME, text),
        placeholders,
    }
}

/// The library modules a build file depends on: `project(":lib")` → `lib`.
pub fn module_dependencies(text: &str) -> Vec<String> {
    crate::domain::modules::module_dependencies(text)
}

/// Merges the module's manifests; `read` reads a file (none when missing).
/// `variant_sets` are the source sets the picked build variant adds to
/// main, highest priority first (`freeDebug`, `debug`, `free`); none
/// takes debug's.
pub fn merged_manifest(root: &Path, manifest: &Path, variant_sets: &[String], read: &dyn Fn(&Path) -> Option<String>) -> Result<Merged, String> {
    let (module, set) = module_of(manifest).ok_or("Not a module's manifest (src/<source set>/AndroidManifest.xml)")?;
    let label = |p: &Path| p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/");
    let build_file = |dir: &Path| [dir.join("build.gradle.kts"), dir.join("build.gradle")].into_iter().find_map(|p| read(&p).map(|t| (p, t)));
    let mut sources = Vec::new();
    let add = |path: PathBuf, sources: &mut Vec<ManifestSource>| {
        if let Some(text) = read(&path) {
            if !sources.iter().any(|s: &ManifestSource| s.label == label(&path)) {
                sources.push(ManifestSource { label: label(&path), text });
            }
        }
    };
    // The variant's overlays above the main manifest; another set's own
    // manifest when that's the one open.
    let overlays: Vec<String> = if variant_sets.is_empty() {
        vec![if set == "main" { "debug".to_owned() } else { set }]
    } else if set == "main" || variant_sets.contains(&set) {
        variant_sets.to_vec()
    } else {
        vec![set]
    };
    for overlay in overlays {
        add(module.join("src").join(&overlay).join("AndroidManifest.xml"), &mut sources);
    }
    add(module.join("src/main/AndroidManifest.xml"), &mut sources);
    // The library modules, breadth first.
    let (build_path, build_text) = build_file(&module).unwrap_or_default();
    let mut queue = module_dependencies(&build_text);
    let mut seen = Vec::new();
    while let Some(dep) = queue.first().cloned() {
        queue.remove(0);
        if seen.contains(&dep) {
            continue;
        }
        seen.push(dep.clone());
        let dir = root.join(&dep);
        add(dir.join("src/main/AndroidManifest.xml"), &mut sources);
        if let Some((_, text)) = build_file(&dir) {
            queue.extend(module_dependencies(&text));
        }
    }
    let build_label = if build_text.is_empty() { "build.gradle".into() } else { label(&build_path) };
    // The main manifest: after the overlay when there is one.
    let main = sources.iter().position(|s| s.label == label(&module.join("src/main/AndroidManifest.xml"))).ok_or("The module has no src/main/AndroidManifest.xml")?;
    manifest_merge::merge(&sources, main, &build_values(&build_text), &build_label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn reads_build_files() {
        let kts = r#"android {
    namespace = "com.example.app"
    defaultConfig {
        applicationId = "com.example.app"
        minSdk = 24
        targetSdk = 35
        versionCode = 7
        versionName = "1.2"
        manifestPlaceholders["apiKey"] = "abc"
        manifestPlaceholders += mapOf("host" to "example.com")
    }
}
dependencies { implementation(project(":core:data")) }"#;
        let v = build_values(kts);
        assert_eq!((v.application_id.as_deref(), v.min_sdk.as_deref(), v.target_sdk.as_deref(), v.version_code.as_deref(), v.version_name.as_deref()), (Some("com.example.app"), Some("24"), Some("35"), Some("7"), Some("1.2")));
        assert_eq!(v.placeholders, [("apiKey".to_owned(), "abc".to_owned()), ("host".into(), "example.com".into())]);
        assert_eq!(module_dependencies(kts), ["core/data"]);
        let groovy = "defaultConfig {\n minSdkVersion 21\n manifestPlaceholders = [scheme: \"app\"]\n}\ndependencies { implementation project(':lib') }";
        let v = build_values(groovy);
        assert_eq!((v.min_sdk.as_deref(), v.placeholders.clone()), (Some("21"), vec![("scheme".to_owned(), "app".to_owned())]));
        assert_eq!(module_dependencies(groovy), ["lib"]);
    }

    #[test]
    fn finds_the_sources() {
        let ns = r#"xmlns:android="http://schemas.android.com/apk/res/android""#;
        let files: HashMap<PathBuf, String> = HashMap::from([
            (PathBuf::from("/p/app/src/main/AndroidManifest.xml"), format!("<manifest {ns}><application android:label=\"A\"/></manifest>")),
            (PathBuf::from("/p/app/src/debug/AndroidManifest.xml"), format!("<manifest {ns}><uses-permission android:name=\"D\"/></manifest>")),
            (PathBuf::from("/p/app/build.gradle.kts"), "minSdk = 26\nimplementation(project(\":lib\"))".into()),
            (PathBuf::from("/p/lib/src/main/AndroidManifest.xml"), format!("<manifest {ns}><uses-permission android:name=\"L\"/></manifest>")),
            (PathBuf::from("/p/lib/build.gradle"), "implementation project(':app')".into()),
        ]);
        let read = |p: &Path| files.get(p).cloned();
        let merged = merged_manifest(Path::new("/p"), Path::new("/p/app/src/main/AndroidManifest.xml"), &[], &read).unwrap();
        assert_eq!(merged.sources, ["app/src/debug/AndroidManifest.xml", "app/src/main/AndroidManifest.xml", "lib/src/main/AndroidManifest.xml", "app/build.gradle.kts"]);
        assert!(merged.lines.iter().any(|l| l.text.contains("minSdkVersion=\"26\"")));
        // The picked variant's: freeRelease has no manifest of its own here.
        let mut files = files;
        files.insert(PathBuf::from("/p/app/src/free/AndroidManifest.xml"), format!("<manifest {ns}><uses-permission android:name=\"F\"/></manifest>"));
        files.insert(PathBuf::from("/p/app/src/release/AndroidManifest.xml"), format!("<manifest {ns}><uses-permission android:name=\"R\"/></manifest>"));
        let read = |p: &Path| files.get(p).cloned();
        let sets = ["freeRelease", "release", "free"].map(String::from);
        let merged = merged_manifest(Path::new("/p"), Path::new("/p/app/src/main/AndroidManifest.xml"), &sets, &read).unwrap();
        assert_eq!(merged.sources, ["app/src/release/AndroidManifest.xml", "app/src/free/AndroidManifest.xml", "app/src/main/AndroidManifest.xml", "lib/src/main/AndroidManifest.xml", "app/build.gradle.kts"]);
        assert!(!merged.lines.iter().any(|l| l.text.contains("\"D\"")));
        assert_eq!(module_of(Path::new("/p/app/src/main/AndroidManifest.xml")), Some((PathBuf::from("/p/app"), "main".into())));
    }
}
