//! Android Studio's "Use version catalog instead" fix (lint's
//! UseTomlInstead) on a build script: a dependency written as
//! `"group:artifact:version"`, or a plugin as `id("x") version "v"`, moves
//! into the version catalog. The script then refers to it
//! (`libs.okhttp`, `alias(libs.plugins.android.application)`). An entry
//! that's already in the catalog is reused ("Replace with existing version
//! catalog reference"). Otherwise one is added with a `[versions]` entry,
//! named as Android Studio names them ("Replace with new library catalog
//! declaration for okhttp"): `androidx-core-ktx` with version `coreKtx`,
//! plugins `android-application` with version `agp`.

use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

use super::version_catalog::{accessor, Catalog, Section};

/// The fix: the script's text to replace, and the catalog's new text
/// (unchanged when an entry is reused).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogMove {
    pub title: String,
    pub range: Range<usize>,
    pub replacement: String,
    pub catalog_text: String,
}

static COORDINATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"["']([\w.\-]+):([\w.\-]+):([\w.\-+]+)["']"#).unwrap());
static PLUGIN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"id\s*\(?\s*["']([\w.\-]+)["']\s*\)?\s*version\s*\(?\s*["']([\w.\-+]+)["'](?:\s*\))?"#).unwrap());

/// The fix for the dependency or plugin at `offset` of a build script,
/// with the catalog it goes to (`catalog_text` is its file).
pub fn move_to_catalog(script: &str, offset: usize, catalog: &Catalog, catalog_text: &str) -> Option<CatalogMove> {
    let hit = |m: &regex::Match| m.start() <= offset && offset <= m.end();
    if let Some(c) = COORDINATE.captures_iter(script).find(|c| hit(&c.get(0).unwrap())) {
        let (group, artifact, version) = (&c[1], &c[2], &c[3]);
        if !version.starts_with(|ch: char| ch.is_ascii_digit()) {
            return None;
        }
        return Some(library(catalog, catalog_text, group, artifact, version, c.get(0).unwrap().range()));
    }
    let c = PLUGIN.captures_iter(script).find(|c| hit(&c.get(0).unwrap()))?;
    let (id, version) = (&c[1], &c[2]);
    id.contains('.').then(|| plugin(catalog, catalog_text, id, version, c.get(0).unwrap().range()))
}

fn library(catalog: &Catalog, text: &str, group: &str, artifact: &str, version: &str, range: Range<usize>) -> CatalogMove {
    let module = format!("{group}:{artifact}");
    let reference = |key: &str| format!("{}.{}", catalog.name, accessor(key));
    if let Some(e) = catalog.entries.iter().find(|e| e.section == Section::Libraries && e.module.as_deref() == Some(module.as_str())) {
        return CatalogMove { title: format!("Replace with existing version catalog reference '{}'", e.key), range, replacement: reference(&e.key), catalog_text: text.to_owned() };
    }
    let key = library_key(catalog, group, artifact);
    let (version_key, text) = version_entry(catalog, text, &camel(artifact), version);
    let line = format!("{key} = {{ group = \"{group}\", name = \"{artifact}\", version.ref = \"{version_key}\" }}");
    CatalogMove {
        title: format!("Replace with new library catalog declaration for {key}"),
        range,
        replacement: reference(&key),
        catalog_text: append(&text, "libraries", &line),
    }
}

fn plugin(catalog: &Catalog, text: &str, id: &str, version: &str, range: Range<usize>) -> CatalogMove {
    let reference = |key: &str| format!("alias({}.plugins.{})", catalog.name, accessor(key));
    if let Some(e) = catalog.entries.iter().find(|e| e.section == Section::Plugins && e.module.as_deref() == Some(id)) {
        return CatalogMove { title: format!("Replace with existing version catalog reference '{}'", e.key), range, replacement: reference(&e.key), catalog_text: text.to_owned() };
    }
    let base = if let Some(rest) = id.strip_prefix("com.android.") {
        format!("android-{}", rest.replace('.', "-"))
    } else if let Some(rest) = id.strip_prefix("org.jetbrains.kotlin.") {
        format!("kotlin-{}", rest.replace('.', "-"))
    } else {
        id.replace('.', "-")
    };
    let key = unique(catalog, Section::Plugins, &base, None);
    let version_name = if id.starts_with("com.android.") {
        "agp".to_owned()
    } else if id.starts_with("org.jetbrains.kotlin.") {
        "kotlin".to_owned()
    } else {
        camel(id.rsplit('.').next().unwrap_or(id))
    };
    let (version_key, text) = version_entry(catalog, text, &version_name, version);
    let line = format!("{key} = {{ id = \"{id}\", version.ref = \"{version_key}\" }}");
    CatalogMove {
        title: format!("Replace with new plugin catalog declaration for {key}"),
        range,
        replacement: reference(&key),
        catalog_text: append(&text, "plugins", &line),
    }
}

/// A library's key: `androidx-<artifact>` for AndroidX, else the
/// artifact, else with its group's last part before it.
fn library_key(catalog: &Catalog, group: &str, artifact: &str) -> String {
    let base = if group.starts_with("androidx.") { format!("androidx-{artifact}") } else { artifact.to_owned() };
    let base = base.to_lowercase();
    let qualifier = group.rsplit('.').next().unwrap_or(group).to_lowercase();
    unique(catalog, Section::Libraries, &base, Some(&qualifier))
}

/// `base`, `qualifier-base`, then `base2`, `base3`… whichever no entry of
/// the section has (keys that read as the same accessor clash).
fn unique(catalog: &Catalog, section: Section, base: &str, qualifier: Option<&str>) -> String {
    let taken = |key: &str| catalog.entries.iter().any(|e| e.section == section && accessor(&e.key) == accessor(key));
    let mut candidates = vec![base.to_owned()];
    candidates.extend(qualifier.filter(|q| !base.starts_with(q)).map(|q| format!("{q}-{base}")));
    if let Some(free) = candidates.iter().find(|k| !taken(k)) {
        return free.clone();
    }
    (2..).map(|n| format!("{base}{n}")).find(|k| !taken(k)).unwrap()
}

/// The `[versions]` key for a version: one named so with that version
/// already, else a new one (the catalog's text with it).
fn version_entry(catalog: &Catalog, text: &str, name: &str, version: &str) -> (String, String) {
    let versions = || catalog.entries.iter().filter(|e| e.section == Section::Versions);
    let candidates = std::iter::once(name.to_owned()).chain((2..).map(|n| format!("{name}{n}")));
    for key in candidates {
        match versions().find(|e| accessor(&e.key) == accessor(&key)) {
            Some(e) if e.version.as_ref().is_some_and(|(v, _)| v == version) => return (key, text.to_owned()),
            Some(_) => continue,
            None => {
                let line = format!("{key} = \"{version}\"");
                return (key.clone(), append(text, "versions", &line));
            }
        }
    }
    unreachable!()
}

/// `core-ktx` → `coreKtx`.
fn camel(name: &str) -> String {
    let mut out = String::new();
    let mut upper = false;
    for c in name.chars() {
        if matches!(c, '-' | '_' | '.') {
            upper = !out.is_empty();
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// The catalog's text with a line added at the end of a section (after
/// its last entry), or a new section at the end of the file.
fn append(text: &str, section: &str, line: &str) -> String {
    let mut lines: Vec<&str> = text.lines().collect();
    let header = lines.iter().position(|l| l.trim().strip_prefix('[').and_then(|h| h.split(']').next()).is_some_and(|h| h.trim() == section));
    let out = match header {
        Some(at) => {
            let end = lines[at + 1..].iter().position(|l| l.trim_start().starts_with('[')).map_or(lines.len(), |p| at + 1 + p);
            // After the last non-blank line of the section.
            let last = (at + 1..end).rev().find(|&i| !lines[i].trim().is_empty()).map_or(at + 1, |i| i + 1);
            lines.insert(last, line);
            lines.join("\n")
        }
        None => {
            // In Gradle's order: versions, libraries, bundles, plugins.
            const ORDER: &[&str] = &["versions", "libraries", "bundles", "plugins"];
            let later = &ORDER[ORDER.iter().position(|s| *s == section).map_or(ORDER.len(), |p| p + 1)..];
            let next = lines.iter().position(|l| l.trim().strip_prefix('[').and_then(|h| h.split(']').next()).is_some_and(|h| later.contains(&h.trim())));
            match next {
                Some(at) => {
                    let block = format!("[{section}]\n{line}\n");
                    lines.insert(at, &block);
                    lines.join("\n").trim_end().to_owned()
                }
                None => {
                    let mut s = text.trim_end().to_owned();
                    if !s.is_empty() {
                        s.push_str("\n\n");
                    }
                    s.push_str(&format!("[{section}]\n{line}"));
                    s
                }
            }
        }
    };
    out + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    const CATALOG: &str = "[versions]\nagp = \"8.5.0\"\ncoreKtx = \"1.13.1\"\n\n[libraries]\nandroidx-core-ktx = { group = \"androidx.core\", name = \"core-ktx\", version.ref = \"coreKtx\" }\n\n[plugins]\nandroid-application = { id = \"com.android.application\", version.ref = \"agp\" }\n";

    fn fix(script: &str, at: &str) -> CatalogMove {
        let catalog = Catalog::parse("libs", "gradle/libs.versions.toml", CATALOG);
        move_to_catalog(script, script.find(at).unwrap(), &catalog, CATALOG).unwrap()
    }

    #[test]
    fn adds_a_library_and_its_version() {
        let script = "dependencies {\n    implementation(\"com.squareup.okhttp3:okhttp:4.12.0\")\n}\n";
        let f = fix(script, "okhttp:");
        assert_eq!(f.title, "Replace with new library catalog declaration for okhttp");
        assert_eq!(&script[f.range.clone()], "\"com.squareup.okhttp3:okhttp:4.12.0\"");
        assert_eq!(f.replacement, "libs.okhttp");
        assert_eq!(
            f.catalog_text,
            "[versions]\nagp = \"8.5.0\"\ncoreKtx = \"1.13.1\"\nokhttp = \"4.12.0\"\n\n[libraries]\nandroidx-core-ktx = { group = \"androidx.core\", name = \"core-ktx\", version.ref = \"coreKtx\" }\nokhttp = { group = \"com.squareup.okhttp3\", name = \"okhttp\", version.ref = \"okhttp\" }\n\n[plugins]\nandroid-application = { id = \"com.android.application\", version.ref = \"agp\" }\n"
        );
    }

    #[test]
    fn reuses_what_the_catalog_has() {
        let f = fix("dependencies {\n    implementation 'androidx.core:core-ktx:1.12.0'\n}\n", "core-ktx");
        assert_eq!(f.title, "Replace with existing version catalog reference 'androidx-core-ktx'");
        assert_eq!(f.replacement, "libs.androidx.core.ktx");
        assert_eq!(f.catalog_text, CATALOG);
        // AndroidX libraries get the `androidx-` prefix.
        let f = fix("implementation(\"androidx.appcompat:appcompat:1.13.1\")", "appcompat");
        assert_eq!(f.replacement, "libs.androidx.appcompat");
        assert!(f.catalog_text.contains("androidx-appcompat = { group = \"androidx.appcompat\", name = \"appcompat\", version.ref = \"appcompat\" }"));
        // Same artifact name elsewhere: a version key of its own.
        let f = fix("implementation(\"androidx.core:core-ktx2:2.0\")", "core-ktx2");
        assert!(f.catalog_text.contains("coreKtx2 = \"2.0\""), "{}", f.catalog_text);
    }

    #[test]
    fn moves_plugins() {
        let script = "plugins {\n    id(\"com.android.library\") version \"8.5.0\" apply false\n    id 'org.jetbrains.kotlin.android' version '2.0.0'\n}\n";
        let f = fix(script, "com.android.library");
        assert_eq!(f.title, "Replace with new plugin catalog declaration for android-library");
        assert_eq!(&script[f.range.clone()], "id(\"com.android.library\") version \"8.5.0\"");
        assert_eq!(f.replacement, "alias(libs.plugins.android.library)");
        assert!(f.catalog_text.contains("android-library = { id = \"com.android.library\", version.ref = \"agp\" }\n"));
        assert!(!f.catalog_text.contains("agp2"));
        let f = fix(script, "kotlin.android");
        assert!(f.catalog_text.contains("kotlin = \"2.0.0\"\n") && f.catalog_text.contains("kotlin-android = { id = \"org.jetbrains.kotlin.android\", version.ref = \"kotlin\" }"));
        let f = fix("id(\"com.android.application\") version \"8.5.0\"", "application");
        assert_eq!(f.title, "Replace with existing version catalog reference 'android-application'");
    }

    #[test]
    fn creates_missing_sections() {
        let catalog = Catalog::parse("libs", "gradle/libs.versions.toml", "");
        let f = move_to_catalog("implementation(\"a.b:c:1.0\")", 18, &catalog, "").unwrap();
        assert_eq!(f.catalog_text, "[versions]\nc = \"1.0\"\n\n[libraries]\nc = { group = \"a.b\", name = \"c\", version.ref = \"c\" }\n");
        assert!(move_to_catalog("implementation(\"a.b:c:$v\")", 18, &catalog, "").is_none());
        let text = "[versions]\nagp = \"8.5.0\"\n\n[plugins]\nandroid-application = { id = \"com.android.application\", version.ref = \"agp\" }\n";
        let catalog = Catalog::parse("libs", "gradle/libs.versions.toml", text);
        let f = move_to_catalog("implementation(\"a.b:c:1.0\")", 18, &catalog, text).unwrap();
        assert_eq!(f.catalog_text, "[versions]\nagp = \"8.5.0\"\nc = \"1.0\"\n\n[libraries]\nc = { group = \"a.b\", name = \"c\", version.ref = \"c\" }\n\n[plugins]\nandroid-application = { id = \"com.android.application\", version.ref = \"agp\" }\n");
        assert!(move_to_catalog("val x = 1", 3, &catalog, "").is_none());
    }
}
