//! Android Studio's "A newer version of … is available" check on Gradle
//! files: where build scripts and version catalogs pin a dependency or a
//! plugin version, how versions order (Gradle's rules: `1.0-alpha01` <
//! `1.0-rc01` < `1.0` < `1.0.1`), and which newer version to offer
//! (a stable one for a stable version; a preview may move to a newer
//! preview).

use std::cmp::Ordering;
use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

use super::version_catalog::{Catalog, Section};

/// Where a file pins a version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionSite {
    /// `group:artifact`, or a plugin's id.
    pub module: String,
    pub plugin: bool,
    pub version: String,
    /// The version's bytes in the file.
    pub range: Range<usize>,
}

/// Looks up the versions a repository has of a module. Implemented over
/// Google's Maven, Maven Central and the Gradle Plugin Portal.
pub trait VersionSource {
    /// `None` when it can't be found out (offline, unknown module).
    fn versions(&self, module: &str, plugin: bool) -> Option<Vec<String>>;
}

static COORDINATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"["']([\w.\-]+):([\w.\-]+):([\w.\-+]+)["']"#).unwrap());
static PLUGIN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"id\s*\(?\s*["']([\w.\-]+)["']\s*\)?\s*version\s*\(?\s*["']([\w.\-+]+)["']"#).unwrap());

/// The versions a build script pins: `"g:a:v"` dependencies and
/// `id("x") version "v"` plugins.
pub fn script_sites(text: &str) -> Vec<VersionSite> {
    let mut out: Vec<VersionSite> = COORDINATE
        .captures_iter(text)
        .map(|c| {
            let v = c.get(3).unwrap();
            VersionSite { module: format!("{}:{}", &c[1], &c[2]), plugin: false, version: v.as_str().to_owned(), range: v.range() }
        })
        .collect();
    out.extend(PLUGIN.captures_iter(text).map(|c| {
        let v = c.get(2).unwrap();
        VersionSite { module: c[1].to_owned(), plugin: true, version: v.as_str().to_owned(), range: v.range() }
    }));
    out.retain(|s| if s.plugin { s.module.contains('.') } else { s.version.starts_with(|c: char| c.is_ascii_digit()) });
    out.sort_by_key(|s| s.range.start);
    out
}

static TEMPLATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"["']([\w.\-]+):([\w.\-]+):\$(?:\{(?:\w+\.)*(\w+)\}|(\w+))["']"#).unwrap());
static CONSTANT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\b(?:val\s+(\w+)\s*(?::\s*String\s*)?|String\s+(\w+)\s*)=\s*"(\d[\w.\-+]*)""#).unwrap());

/// `buildSrc` versions kept in constants: a dependency string built from
/// one (`"g:a:${Versions.core}"`, `"g:a:$core"`) in any of the buildSrc
/// files `all` makes the constant's value in `text` (`const val core =
/// "1.12.0"`, Java's `String core = "1.12.0"`) a site for `g:a`, the first
/// module that uses it.
pub fn constant_sites(text: &str, all: &[&str]) -> Vec<VersionSite> {
    let mut users: Vec<(String, String)> = Vec::new();
    for file in all {
        for c in TEMPLATE.captures_iter(file) {
            let name = c.get(3).or(c.get(4)).map_or("", |m| m.as_str()).to_owned();
            if !users.iter().any(|(n, _)| *n == name) {
                users.push((name, format!("{}:{}", &c[1], &c[2])));
            }
        }
    }
    let mut out = Vec::new();
    for c in CONSTANT.captures_iter(text) {
        let Some(name) = c.get(1).or(c.get(2)) else { continue };
        let Some((_, module)) = users.iter().find(|(n, _)| n == name.as_str()) else { continue };
        let v = c.get(3).unwrap();
        out.push(VersionSite { module: module.clone(), plugin: false, version: v.as_str().to_owned(), range: v.range() });
    }
    out
}

/// The versions a catalog pins: a library's or plugin's own `version`,
/// and a `[versions]` entry for the first library or plugin that uses it.
pub fn catalog_sites(catalog: &Catalog) -> Vec<VersionSite> {
    let mut out: Vec<VersionSite> = Vec::new();
    for e in catalog.entries.iter().filter(|e| matches!(e.section, Section::Libraries | Section::Plugins)) {
        let Some(module) = e.module.clone() else { continue };
        let plugin = e.section == Section::Plugins;
        let pinned = match (&e.version, &e.version_ref) {
            (Some(v), _) => Some(v.clone()),
            (None, Some((key, _))) => catalog.by_key(Section::Versions, key).and_then(|v| v.version.clone()),
            _ => None,
        };
        let Some((version, range)) = pinned else { continue };
        if out.iter().any(|s| s.range == range) {
            continue;
        }
        out.push(VersionSite { module, plugin, version, range });
    }
    out.sort_by_key(|s| s.range.start);
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Part {
    Num(u64),
    Word(String),
}

/// A word's place among releases: previews before the release (`""`), a
/// service pack after. Unknown words sort before the release, by text.
fn rank(word: &str) -> i32 {
    match word {
        "dev" => 0,
        "alpha" | "a" | "eap" | "preview" | "pre" => 1,
        "beta" | "b" => 2,
        "milestone" | "m" => 3,
        "rc" | "cr" => 4,
        "snapshot" => 5,
        "" | "ga" | "final" | "release" => 6,
        "sp" => 7,
        _ => 1,
    }
}

fn parts(version: &str) -> Vec<Part> {
    let mut out = Vec::new();
    let mut current = String::new();
    let flush = |current: &mut String, out: &mut Vec<Part>| {
        if !current.is_empty() {
            out.push(match current.parse() {
                Ok(n) => Part::Num(n),
                Err(_) => Part::Word(current.to_lowercase()),
            });
            current.clear();
        }
    };
    for c in version.chars() {
        if !c.is_ascii_alphanumeric() {
            flush(&mut current, &mut out);
        } else if current.chars().last().is_some_and(|l| l.is_ascii_digit() != c.is_ascii_digit()) {
            flush(&mut current, &mut out);
            current.push(c);
        } else {
            current.push(c);
        }
    }
    flush(&mut current, &mut out);
    out
}

/// Orders versions as Gradle does.
pub fn compare(a: &str, b: &str) -> Ordering {
    let (a, b) = (parts(a), parts(b));
    for i in 0..a.len().max(b.len()) {
        let ordering = match (a.get(i), b.get(i)) {
            (Some(Part::Num(x)), Some(Part::Num(y))) => x.cmp(y),
            (Some(Part::Num(_)), Some(Part::Word(_))) => Ordering::Greater,
            (Some(Part::Word(_)), Some(Part::Num(_))) => Ordering::Less,
            (Some(Part::Word(x)), Some(Part::Word(y))) => rank(x).cmp(&rank(y)).then_with(|| x.cmp(y)),
            // `1.0` = `1.0.0`; `1.0` > `1.0-rc01`; `1.0` < `1.0-sp1`.
            (None, Some(Part::Num(y))) => 0.cmp(y),
            (Some(Part::Num(x)), None) => x.cmp(&0),
            (None, Some(Part::Word(y))) => rank("").cmp(&rank(y)),
            (Some(Part::Word(x)), None) => rank(x).cmp(&rank("")),
            (None, None) => Ordering::Equal,
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    Ordering::Equal
}

fn words(version: &str) -> Vec<String> {
    parts(version).into_iter().filter_map(|p| if let Part::Word(w) = p { Some(w) } else { None }).collect()
}

/// A release, not a preview (alpha, beta, rc, dev, snapshot, eap…).
pub fn is_stable(version: &str) -> bool {
    words(version).iter().all(|w| rank(w) >= 6 || !is_preview_word(w))
}

fn is_preview_word(word: &str) -> bool {
    matches!(word, "dev" | "alpha" | "a" | "eap" | "preview" | "pre" | "beta" | "b" | "milestone" | "m" | "rc" | "cr" | "snapshot")
}

/// The newest version to offer instead of `current`, if there is a newer
/// one: stable for a stable version; a preview may move to any newer one.
/// A version with a flavor word (Guava's `-jre` / `-android`) keeps it.
/// Dynamic versions (`1.+`, ranges, `latest.release`) are left alone.
pub fn newer(current: &str, available: &[String]) -> Option<String> {
    if current.is_empty() || current.contains(['+', '[', '(', '$']) || current.starts_with("latest.") {
        return None;
    }
    let stable = is_stable(current);
    let flavor: Vec<String> = words(current).into_iter().filter(|w| !is_preview_word(w) && rank(w) != 6).collect();
    available
        .iter()
        .filter(|v| !stable || is_stable(v))
        .filter(|v| {
            let theirs = words(v);
            flavor.iter().all(|f| theirs.contains(f)) && theirs.iter().filter(|w| !is_preview_word(w) && rank(w) != 6).all(|w| flavor.contains(w))
        })
        .max_by(|a, b| compare(a, b))
        .filter(|best| compare(best, current) == Ordering::Greater)
        .cloned()
}

/// Android Studio's message.
pub fn message(site: &VersionSite, newest: &str) -> String {
    format!("A newer version of {} than {} is available: {newest}", site.module, site.version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_like_gradle() {
        let mut v = vec!["1.0", "1.0-alpha02", "1.0.1", "1.0-rc01", "1.0-alpha10", "1.0-beta01", "0.9", "1.0-sp1", "2.0.0-Beta1", "1.10"];
        v.sort_by(|a, b| compare(a, b));
        assert_eq!(v, ["0.9", "1.0-alpha02", "1.0-alpha10", "1.0-beta01", "1.0-rc01", "1.0", "1.0-sp1", "1.0.1", "1.10", "2.0.0-Beta1"]);
        assert_eq!(compare("1.0", "1.0.0"), Ordering::Equal);
    }

    #[test]
    fn offers_stable_for_stable() {
        let available: Vec<String> = ["1.10.1", "1.12.0", "1.13.1", "1.14.0-alpha01"].iter().map(|s| s.to_string()).collect();
        assert_eq!(newer("1.10.1", &available).as_deref(), Some("1.13.1"));
        assert_eq!(newer("1.13.1", &available), None);
        assert_eq!(newer("1.13.0-beta01", &available).as_deref(), Some("1.14.0-alpha01"));
        assert_eq!(newer("1.+", &available), None);
        let guava: Vec<String> = ["32.0.0-jre", "33.2.1-android", "33.2.1-jre"].iter().map(|s| s.to_string()).collect();
        assert_eq!(newer("32.0.0-jre", &guava).as_deref(), Some("33.2.1-jre"));
    }

    #[test]
    fn finds_sites_in_scripts_and_catalogs() {
        let script = r#"plugins { id("com.android.application") version "8.1.0" }
dependencies { implementation("androidx.core:core-ktx:1.10.1"); implementation 'junit:junit:4.12' }"#;
        let sites = script_sites(script);
        assert_eq!(sites.len(), 3);
        assert_eq!((sites[0].module.as_str(), sites[0].plugin, &script[sites[0].range.clone()]), ("com.android.application", true, "8.1.0"));
        assert_eq!((sites[1].module.as_str(), &script[sites[1].range.clone()]), ("androidx.core:core-ktx", "1.10.1"));
        let toml = "[versions]\ncore = \"1.10.1\"\n[libraries]\na = { module = \"androidx.core:core\", version.ref = \"core\" }\nb = { module = \"androidx.core:core-ktx\", version.ref = \"core\" }\nc = \"junit:junit:4.12\"\n";
        let catalog = Catalog::parse("libs", "gradle/libs.versions.toml", toml);
        let sites = catalog_sites(&catalog);
        assert_eq!(sites.len(), 2);
        assert_eq!((sites[0].module.as_str(), &toml[sites[0].range.clone()]), ("androidx.core:core", "1.10.1"));
        assert_eq!(&toml[sites[1].range.clone()], "4.12");
    }
}
