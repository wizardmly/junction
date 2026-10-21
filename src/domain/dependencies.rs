//! Project Structure › Dependencies (Android Studio's PSD): a module's
//! declared dependencies, read from its build script's top-level
//! `dependencies { … }` block, and the edits the page makes to them: the
//! requested version (in the script, or in the version catalog for a
//! `libs.x` reference), the configuration, removing one and adding a
//! library or module dependency.

use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

use super::catalog_migration;
use super::intentions::matching;
use super::version_catalog::{Catalog, Section};

/// What a dependency is of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notation {
    /// `group:artifact`.
    Library(String),
    /// Another module: `:lib`.
    Module(String),
    /// `files(…)`, `fileTree(…)`.
    Files,
    Other,
}

/// Where a requested version is written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VersionSite {
    Script(Range<usize>),
    /// In the catalog at this index of those passed in.
    Catalog(usize, Range<usize>),
}

/// A declared dependency: one statement of the block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declared {
    /// `implementation`.
    pub configuration: String,
    pub notation: Notation,
    /// As written: `libs.okhttp`, `"g:a:v"`, `project(":lib")`.
    pub written: String,
    /// What the list shows, as AS does: `g:a:v` (resolved through the
    /// catalog), `:lib`, or what's written.
    pub text: String,
    pub version: Option<String>,
    pub version_site: Option<VersionSite>,
    configuration_range: Range<usize>,
    /// The statement's whole lines.
    statement: Range<usize>,
}

/// The configurations the page offers, as AS's PSD lists them.
pub const CONFIGURATIONS: &[&str] =
    &["implementation", "api", "compileOnly", "runtimeOnly", "testImplementation", "androidTestImplementation", "debugImplementation", "releaseImplementation", "annotationProcessor", "kapt", "ksp"];

static COORDINATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^["']([\w.\-]+):([\w.\-]+)(?::([^"':@]+))?(?:@\w+)?["']$"#).unwrap());
static PROJECT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^project\s*\(\s*(?:path\s*[:=]\s*)?["']([^"']+)["']"#).unwrap());
static WRAPPER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:platform|enforcedPlatform|testFixtures)\s*\((.*)\)$").unwrap());
static STATEMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([A-Za-z_]\w*)\s*(\(?)\s*").unwrap());

/// The top-level `dependencies { … }` block: the bytes between its
/// braces.
fn block(script: &str) -> Option<Range<usize>> {
    static NAMED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\w+)\s*$").unwrap());
    let mut at = 0;
    while let Some(open) = next_open(script, at) {
        let close = matching(script, open)?;
        if NAMED.captures(&script[..open]).is_some_and(|c| &c[1] == "dependencies") {
            return Some(open + 1..close);
        }
        at = close + 1;
    }
    None
}

/// The next `{` from `from`, past strings and comments.
pub(super) fn next_open(text: &str, from: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        let rest = &text[i..];
        match bytes[i] {
            _ if rest.starts_with("//") => i += rest.find('\n').unwrap_or(rest.len()),
            _ if rest.starts_with("/*") => i += rest.find("*/").map_or(rest.len(), |e| e + 2),
            q @ (b'"' | b'\'') => {
                let mut j = i + 1;
                while j < bytes.len() && bytes[j] != q && bytes[j] != b'\n' {
                    j += if bytes[j] == b'\\' { 2 } else { 1 };
                }
                i = j + 1;
            }
            b'{' => return Some(i),
            _ => i += 1,
        }
    }
    None
}

/// The module's declared dependencies, in the order written.
pub fn declared(script: &str, catalogs: &[Catalog]) -> Vec<Declared> {
    let Some(body) = block(script) else { return Vec::new() };
    let mut out = Vec::new();
    let mut at = body.start;
    while at < body.end {
        let line_end = script[at..body.end].find('\n').map_or(body.end, |i| at + i + 1);
        let line = &script[at..line_end];
        let lead = line.len() - line.trim_start().len();
        let start = at + lead;
        // A statement with a closure (`{ exclude … }`) runs to its brace.
        let open = next_open(script, start).filter(|&o| o < line_end);
        let end = match open.and_then(|o| matching(script, o)) {
            Some(close) => script[close..body.end].find('\n').map_or(body.end, |i| close + i + 1),
            None => line_end,
        };
        let statement = at..end;
        at = end;
        let Some(c) = STATEMENT.captures(&script[start..line_end]) else { continue };
        let configuration = c[1].to_owned();
        let mut rest = script[start + c.get(0).unwrap().end()..open.unwrap_or(line_end)].trim();
        if let Some(cut) = rest.find("//") {
            rest = rest[..cut].trim_end();
        }
        if !c[2].is_empty() {
            rest = rest.strip_suffix(')').unwrap_or(rest).trim_end();
        }
        // `constraints { … }` and other nested blocks aren't dependencies.
        if rest.is_empty() {
            continue;
        }
        let rest_start = start + script[start..].find(rest).unwrap_or(0);
        let configuration_range = start..start + configuration.len();
        out.push(read_notation(configuration, rest, rest_start, configuration_range, statement, catalogs));
    }
    out
}

fn read_notation(configuration: String, written: &str, offset: usize, configuration_range: Range<usize>, statement: Range<usize>, catalogs: &[Catalog]) -> Declared {
    let mut d = Declared { configuration, notation: Notation::Other, written: written.to_owned(), text: written.to_owned(), version: None, version_site: None, configuration_range, statement };
    // `platform(…)` and the like wrap what they're of.
    let (inner, inner_offset) = match WRAPPER.captures(written) {
        Some(c) => (c.get(1).unwrap().as_str().trim(), offset + c.get(1).unwrap().start() + (c[1].len() - c[1].trim_start().len())),
        None => (written, offset),
    };
    if let Some(c) = COORDINATE.captures(inner) {
        d.notation = Notation::Library(format!("{}:{}", &c[1], &c[2]));
        d.text = inner[1..inner.len() - 1].to_owned();
        if let Some(v) = c.get(3) {
            d.version = Some(v.as_str().to_owned());
            d.version_site = Some(VersionSite::Script(inner_offset + v.start()..inner_offset + v.end()));
        }
    } else if let Some(c) = PROJECT.captures(inner) {
        d.notation = Notation::Module(c[1].to_owned());
        d.text = c[1].to_owned();
    } else if inner.starts_with("files(") || inner.starts_with("fileTree(") {
        d.notation = Notation::Files;
    } else if let Some((name, path)) = inner.split_once('.') {
        let path = path.trim_end_matches(".get()");
        let found = catalogs.iter().enumerate().filter(|(_, c)| c.name == name).find_map(|(ix, c)| c.resolve(path).filter(|e| e.section == Section::Libraries).map(|e| (ix, c, e)));
        if let Some((ix, catalog, entry)) = found {
            let module = entry.module.clone().unwrap_or_default();
            d.notation = Notation::Library(module.clone());
            d.text = catalog.describe(entry);
            let site = match (&entry.version, &entry.version_ref) {
                (Some((v, r)), _) => Some((v.clone(), r.clone())),
                (None, Some((key, _))) => catalog.by_key(Section::Versions, key).and_then(|v| v.version.clone()),
                _ => None,
            };
            if let Some((v, r)) = site {
                d.version = Some(v);
                d.version_site = Some(VersionSite::Catalog(ix, r));
            }
        }
    }
    d
}

/// An edit of a declared dependency.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Version(String),
    Configuration(String),
    Remove,
}

fn splice(text: &str, range: &Range<usize>, with: &str) -> String {
    format!("{}{}{}", &text[..range.start], with, &text[range.end..])
}

/// The script and catalogs' texts with a change made to `dep` (one of
/// `declared(script, …)`); `catalog_texts` matches the catalogs.
pub fn change(script: &str, catalog_texts: &[String], dep: &Declared, change: &Change) -> (String, Vec<String>) {
    let mut catalogs = catalog_texts.to_vec();
    let script = match change {
        Change::Configuration(name) => splice(script, &dep.configuration_range, name.trim()),
        Change::Remove => splice(script, &dep.statement, ""),
        Change::Version(v) => match &dep.version_site {
            Some(VersionSite::Script(r)) => splice(script, r, v.trim()),
            Some(VersionSite::Catalog(ix, r)) => {
                if let Some(text) = catalogs.get_mut(*ix) {
                    *text = splice(text, r, v.trim());
                }
                script.to_owned()
            }
            None => script.to_owned(),
        },
    };
    (script, catalogs)
}

/// Why a requested version or configuration can't be written.
pub fn check(change: &Change) -> Result<(), String> {
    let ok = |s: &str| !s.trim().is_empty() && !s.trim().contains(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ':'));
    match change {
        Change::Version(v) if !ok(v) => Err(format!("'{}' is not a version.", v.trim())),
        Change::Configuration(c) if !c.trim().chars().all(|ch| ch.is_alphanumeric() || ch == '_') || c.trim().is_empty() => Err(format!("'{}' is not a configuration.", c.trim())),
        _ => Ok(()),
    }
}

/// A dependency to add.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Addition {
    /// `group:artifact:version`.
    Library(String),
    /// `:lib`.
    Module(String),
}

/// The script (and catalogs) with a dependency added at the end of the
/// block (a new block when there's none). A library goes into the first
/// catalog when the build has one, as AS's PSD adds it there.
pub fn add(script: &str, kotlin: bool, catalogs: &[Catalog], catalog_texts: &[String], configuration: &str, addition: &Addition) -> Result<(String, Vec<String>), String> {
    let configuration = configuration.trim();
    check(&Change::Configuration(configuration.to_owned()))?;
    let notation = match addition {
        Addition::Library(coordinate) => {
            let parts: Vec<&str> = coordinate.trim().split(':').collect();
            if parts.len() != 3 || parts.iter().any(|p| p.is_empty() || p.contains(char::is_whitespace)) {
                return Err(format!("'{}' is not group:artifact:version.", coordinate.trim()));
            }
            format!("\"{}\"", coordinate.trim())
        }
        Addition::Module(path) => format!("project(\"{path}\")"),
    };
    let line = if kotlin { format!("{configuration}({notation})") } else { format!("{configuration} {}", notation.replace('"', "'")) };
    let (mut text, at) = match block(script) {
        Some(body) => {
            let inside = &script[body.clone()];
            let indent = inside.lines().map(|l| &l[..l.len() - l.trim_start().len()]).find(|i| !i.is_empty()).unwrap_or("    ").to_owned();
            // After the last line with something on it.
            let last = inside.trim_end().len();
            let at = body.start + last;
            let insert = format!("\n{indent}{line}");
            let mut text = script.to_owned();
            text.insert_str(at, &insert);
            if !text[at + insert.len()..].starts_with('\n') {
                text.insert(at + insert.len(), '\n');
            }
            (text, at + 1 + indent.len())
        }
        None => {
            let mut text = script.trim_end().to_owned();
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            let at = text.len() + "dependencies {\n    ".len();
            text.push_str(&format!("dependencies {{\n    {line}\n}}\n"));
            (text, at)
        }
    };
    let mut catalog_texts = catalog_texts.to_vec();
    if let (Addition::Library(_), Some(catalog), Some(catalog_text)) = (addition, catalogs.first(), catalog_texts.first_mut()) {
        if let Some(m) = catalog_migration::move_to_catalog(&text, at + configuration.len() + 1, catalog, catalog_text) {
            text = splice(&text, &m.range, &m.replacement);
            *catalog_text = m.catalog_text;
        }
    }
    Ok((text, catalog_texts))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CATALOG: &str = "[versions]\ncoreKtx = \"1.13.1\"\n\n[libraries]\nandroidx-core-ktx = { group = \"androidx.core\", name = \"core-ktx\", version.ref = \"coreKtx\" }\n";
    const KTS: &str = "plugins {\n    id(\"com.android.application\")\n}\n\nbuildscript {\n    dependencies {\n        classpath(\"x:y:1\")\n    }\n}\n\ndependencies {\n    implementation(libs.androidx.core.ktx)\n    implementation(\"com.squareup.okhttp3:okhttp:4.12.0\") // net\n    implementation(project(\":lib\"))\n    implementation(platform(\"androidx.compose:compose-bom:2024.06.00\"))\n    testImplementation(\"junit:junit:4.13.2\") {\n        exclude(group = \"org.hamcrest\")\n    }\n    implementation(fileTree(mapOf(\"dir\" to \"libs\")))\n}\n";

    fn catalogs() -> (Vec<Catalog>, Vec<String>) {
        (vec![Catalog::parse("libs", "gradle/libs.versions.toml", CATALOG)], vec![CATALOG.to_owned()])
    }

    #[test]
    fn reads_the_declared_dependencies() {
        let (cats, _) = catalogs();
        let deps = declared(KTS, &cats);
        let rows: Vec<(&str, &str)> = deps.iter().map(|d| (d.configuration.as_str(), d.text.as_str())).collect();
        assert_eq!(
            rows,
            [
                ("implementation", "androidx.core:core-ktx:1.13.1"),
                ("implementation", "com.squareup.okhttp3:okhttp:4.12.0"),
                ("implementation", ":lib"),
                ("implementation", "androidx.compose:compose-bom:2024.06.00"),
                ("testImplementation", "junit:junit:4.13.2"),
                ("implementation", "fileTree(mapOf(\"dir\" to \"libs\"))"),
            ]
        );
        assert_eq!(deps[0].version_site, Some(VersionSite::Catalog(0, CATALOG.find("1.13.1").unwrap()..CATALOG.find("1.13.1").unwrap() + 6)));
        assert_eq!(deps[2].notation, Notation::Module(":lib".into()));
        assert_eq!(deps[5].notation, Notation::Files);
        let groovy = "dependencies {\n    implementation 'androidx.appcompat:appcompat:1.7.0'\n    api project(':core')\n}\n";
        let deps = declared(groovy, &[]);
        assert_eq!(deps.iter().map(|d| d.text.as_str()).collect::<Vec<_>>(), ["androidx.appcompat:appcompat:1.7.0", ":core"]);
    }

    #[test]
    fn edits_versions_configurations_and_removes() {
        let (cats, texts) = catalogs();
        let deps = declared(KTS, &cats);
        let (script, _) = change(KTS, &texts, &deps[1], &Change::Version("4.12.1".into()));
        assert!(script.contains("\"com.squareup.okhttp3:okhttp:4.12.1\") // net"));
        let (script, catalog) = change(KTS, &texts, &deps[0], &Change::Version("1.15.0".into()));
        assert_eq!((script.as_str(), catalog[0].as_str()), (KTS, CATALOG.replace("1.13.1", "1.15.0").as_str()));
        let (script, _) = change(KTS, &texts, &deps[2], &Change::Configuration("api".into()));
        assert!(script.contains("    api(project(\":lib\"))\n"));
        let (script, _) = change(KTS, &texts, &deps[4], &Change::Remove);
        assert!(!script.contains("junit") && !script.contains("hamcrest"));
        assert!(script.contains("compose-bom:2024.06.00\"))\n    implementation(fileTree"));
        assert!(check(&Change::Version("1 2".into())).is_err());
    }

    #[test]
    fn adds_libraries_and_modules() {
        let (cats, texts) = catalogs();
        let (script, catalog) = add(KTS, true, &cats, &texts, "implementation", &Addition::Library("io.coil-kt:coil:2.6.0".into())).unwrap();
        assert!(script.contains("    implementation(fileTree(mapOf(\"dir\" to \"libs\")))\n    implementation(libs.coil)\n}\n"));
        assert!(catalog[0].contains("coil = \"2.6.0\"") && catalog[0].contains("coil = { group = \"io.coil-kt\", name = \"coil\", version.ref = \"coil\" }"));
        let (script, _) = add("android {\n}\n", false, &[], &[], "api", &Addition::Module(":lib".into())).unwrap();
        assert_eq!(script, "android {\n}\n\ndependencies {\n    api project(':lib')\n}\n");
        let (script, _) = add("dependencies {\n    implementation 'a:b:1'\n}\n", false, &[], &[], "testImplementation", &Addition::Library("junit:junit:4.13.2".into())).unwrap();
        assert_eq!(script, "dependencies {\n    implementation 'a:b:1'\n    testImplementation 'junit:junit:4.13.2'\n}\n");
        assert!(add(KTS, true, &cats, &texts, "implementation", &Addition::Library("nope".into())).is_err());
    }
}
