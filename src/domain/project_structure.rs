//! Project Structure (File › Project Structure…, Android Studio's PSD):
//! the values its pages show and edit, read from and written back into the
//! build scripts, the version catalog, `gradle-wrapper.properties` and
//! `local.properties`. A value written as a literal (or as a catalog
//! version, `libs.versions.minSdk.get().toInt()`) edits in place; any
//! other expression is shown and left alone.

use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

use super::version_catalog::{accessor, Catalog, Section};

/// Which file a value is written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// The module's (or for the AGP version, the root's) build script.
    Script,
    /// `settings.gradle(.kts)`: `pluginManagement { plugins { … } }`.
    Settings,
    /// The catalog at this index of those passed in.
    Catalog(usize),
}

/// How a value is written, to write a new one the same way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Number,
    /// Quoted with this character.
    Quoted(char),
    /// `JavaVersion.VERSION_17`, shown as `17`.
    JavaVersion,
    /// `true` / `false`.
    Boolean,
}

/// Where a value is and how to rewrite it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Site {
    pub place: Place,
    /// The written value's bytes (inside the quotes for a quoted one).
    pub range: Range<usize>,
    pub kind: Kind,
}

/// A row of a page: Android Studio's label, the value, and where to edit
/// it; `expression` is what's written when it can't be edited here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Property {
    pub label: &'static str,
    pub value: String,
    pub site: Option<Site>,
    pub expression: Option<String>,
}

/// Modules › Properties, then Default Config, as the PSD lists them.
pub const MODULE_PROPERTIES: &[(&str, &str)] = &[
    ("Compile Sdk Version", r"compileSdk(?:Version)?"),
    ("Build Tools Version", r"buildToolsVersion"),
    ("NDK Version", r"ndkVersion"),
    ("Source Compatibility", r"sourceCompatibility"),
    ("Target Compatibility", r"targetCompatibility"),
];

pub const DEFAULT_CONFIG: &[(&str, &str)] = &[
    ("Application ID", r"applicationId"),
    ("Min SDK Version", r"minSdk(?:Version)?"),
    ("Target SDK Version", r"targetSdk(?:Version)?"),
    ("Version Code", r"versionCode"),
    ("Version Name", r"versionName"),
];

static CATALOG_REF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\w+)\.versions\.([\w.]+?)(?:\.get\(\))?(?:\.toInt(?:eger)?\(\))?$").unwrap());

/// A property of a module's build script (Kotlin DSL or Groovy): `name =
/// value`, `name value` or `name(value)`; not set when absent.
pub fn property(label: &'static str, name: &str, script: &str, catalogs: &[(Catalog, String)]) -> Property {
    let re = Regex::new(&format!(r#"(?m)^[ \t]*{name}\b[ \t]*(=[ \t]*|\([ \t]*|[ \t]+)([^\n]*)"#)).unwrap();
    let not_set = Property { label, value: String::new(), site: None, expression: None };
    let Some(c) = re.captures(script) else { return not_set };
    let found = c.get(2).unwrap();
    // The value runs to the line's end, less a comment, a `)` closing
    // `name(value)` and trailing blanks.
    let mut text = found.as_str();
    if let Some(at) = text.find("//") {
        text = &text[..at];
    }
    text = text.trim_end();
    if c[1].starts_with('(') {
        text = text.strip_suffix(')').unwrap_or(text).trim_end();
    }
    if text.is_empty() {
        return not_set;
    }
    let start = found.start();
    let found = start..start + text.len();
    if let Some(quote) = text.chars().next().filter(|c| *c == '"' || *c == '\'') {
        if text.len() >= 2 && text.ends_with(quote) {
            let inner = start + 1..start + text.len() - 1;
            return Property { label, value: script[inner.clone()].to_owned(), site: Some(Site { place: Place::Script, range: inner, kind: Kind::Quoted(quote) }), expression: None };
        }
    }
    if text == "true" || text == "false" {
        return Property { label, value: text.to_owned(), site: Some(Site { place: Place::Script, range: found.clone(), kind: Kind::Boolean }), expression: None };
    }
    if text.chars().all(|c| c.is_ascii_digit()) {
        return Property { label, value: text.to_owned(), site: Some(Site { place: Place::Script, range: found.clone(), kind: Kind::Number }), expression: None };
    }
    if let Some(version) = text.strip_prefix("JavaVersion.VERSION_") {
        let range = start + "JavaVersion.VERSION_".len()..found.end;
        return Property { label, value: version.replace('_', "."), site: Some(Site { place: Place::Script, range, kind: Kind::JavaVersion }), expression: None };
    }
    if let Some(c) = CATALOG_REF.captures(text) {
        let (name, key) = (&c[1], &c[2]);
        let entry = catalogs.iter().enumerate().filter(|(_, (cat, _))| cat.name == name).flat_map(|(ix, (cat, _))| cat.entries.iter().map(move |e| (ix, e))).find(|(_, e)| e.section == Section::Versions && accessor(&e.key) == key);
        if let Some((ix, (value, range))) = entry.and_then(|(ix, e)| Some((ix, e.version.clone()?))) {
            return Property { label, value, site: Some(Site { place: Place::Catalog(ix), range, kind: Kind::Quoted('"') }), expression: Some(text.to_owned()) };
        }
    }
    Property { label, value: text.to_owned(), site: None, expression: Some(text.to_owned()) }
}

/// A module's Properties and Default Config.
pub fn module_properties(script: &str, catalogs: &[(Catalog, String)]) -> (Vec<Property>, Vec<Property>) {
    // Default Config's rows come from its block, as flavors set the same
    // names.
    let android = child_blocks(script, 0..script.len()).into_iter().find(|(n, _)| n == "android").map(|(_, r)| r);
    let default_config = android.and_then(|r| child_blocks(script, r).into_iter().find(|(n, _)| n == "defaultConfig")).map(|(_, r)| r).unwrap_or(0..script.len());
    let properties = MODULE_PROPERTIES.iter().map(|(label, name)| property(label, name, script, catalogs)).collect();
    let defaults = DEFAULT_CONFIG.iter().map(|(label, name)| property_in(label, name, script, default_config.clone(), catalogs)).collect();
    (properties, defaults)
}

/// Writes a new value at a site, the way the old one was written.
pub fn write(text: &str, site: &Site, value: &str) -> String {
    let value = match site.kind {
        Kind::Number | Kind::Boolean => value.trim().to_owned(),
        Kind::Quoted(q) => value.replace(q, &format!("\\{q}")),
        Kind::JavaVersion => value.trim().replace('.', "_"),
    };
    let mut out = String::with_capacity(text.len() + value.len());
    out.push_str(&text[..site.range.start]);
    out.push_str(&value);
    out.push_str(&text[site.range.end..]);
    out
}

/// Whether a new value suits its site: a number where one is written.
pub fn check(site: &Site, value: &str) -> Result<(), String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("A value is required.".into());
    }
    match site.kind {
        Kind::Number if !value.chars().all(|c| c.is_ascii_digit()) => Err(format!("'{value}' is not a whole number.")),
        Kind::JavaVersion if !value.chars().all(|c| c.is_ascii_digit() || c == '.') => Err(format!("'{value}' is not a Java version (17, 1.8).")),
        Kind::Boolean if value != "true" && value != "false" => Err(format!("'{value}' is not true or false.")),
        _ => Ok(()),
    }
}

/// Build Variants › Build Types, as the PSD lists a build type's rows
/// (Kotlin DSL's `isX` and Groovy's `x` alike).
pub const BUILD_TYPE: &[(&str, &str)] = &[
    ("Debuggable", r"(?:isDebuggable|debuggable)"),
    ("Jni Debuggable", r"(?:isJniDebuggable|jniDebuggable)"),
    ("Minify Enabled", r"(?:isMinifyEnabled|minifyEnabled)"),
    ("Shrink Resources", r"(?:isShrinkResources|shrinkResources)"),
    ("Multi Dex Enabled", r"multiDexEnabled"),
    ("Application Id Suffix", r"applicationIdSuffix"),
    ("Version Name Suffix", r"versionNameSuffix"),
];

/// Build Variants › Flavors.
pub const FLAVOR: &[(&str, &str)] = &[
    ("Dimension", r"dimension"),
    ("Application ID", r"applicationId"),
    ("Application Id Suffix", r"applicationIdSuffix"),
    ("Min SDK Version", r"minSdk(?:Version)?"),
    ("Target SDK Version", r"targetSdk(?:Version)?"),
    ("Version Code", r"versionCode"),
    ("Version Name", r"versionName"),
    ("Version Name Suffix", r"versionNameSuffix"),
];

/// A build type or product flavor: its name and rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variant {
    pub name: String,
    pub properties: Vec<Property>,
}

/// The blocks directly inside `range` of `text`: each name (`release`,
/// or `getByName("release")` / `create("staging")`'s argument) and the
/// bytes between its braces.
fn child_blocks(text: &str, range: std::ops::Range<usize>) -> Vec<(String, std::ops::Range<usize>)> {
    static NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?:(?:getByName|create|named|maybeCreate|register)\s*\(\s*["'](\w+)["']\s*\)|(\w+))\s*$"#).unwrap());
    let mut out = Vec::new();
    let mut at = range.start;
    while let Some(open) = super::dependencies::next_open(text, at).filter(|&o| o < range.end) {
        let Some(close) = super::intentions::matching(text, open) else { break };
        let line_start = text[..open].rfind('\n').map_or(0, |i| i + 1).max(range.start);
        if let Some(c) = NAME.captures(&text[line_start..open]) {
            out.push((c.get(1).or(c.get(2)).unwrap().as_str().to_owned(), open + 1..close));
        }
        at = close + 1;
    }
    out
}

/// A property read from `body` of the script, at the script's offsets.
fn property_in(label: &'static str, name: &str, script: &str, body: std::ops::Range<usize>, catalogs: &[(Catalog, String)]) -> Property {
    let mut p = property(label, name, &script[body.clone()], catalogs);
    if let Some(site) = p.site.as_mut().filter(|s| s.place == Place::Script) {
        site.range = site.range.start + body.start..site.range.end + body.start;
    }
    p
}

/// A module's build types and product flavors, from `android { buildTypes
/// { … } productFlavors { … } }`.
pub fn variants(script: &str, catalogs: &[(Catalog, String)]) -> (Vec<Variant>, Vec<Variant>) {
    let Some((_, android)) = child_blocks(script, 0..script.len()).into_iter().find(|(n, _)| n == "android") else { return Default::default() };
    let inside = child_blocks(script, android);
    let read = |block: &str, rows: &[(&'static str, &str)]| -> Vec<Variant> {
        let Some((_, range)) = inside.iter().find(|(n, _)| n == block) else { return Vec::new() };
        child_blocks(script, range.clone())
            .into_iter()
            .map(|(name, body)| {
                let properties = rows.iter().map(|(label, re)| property_in(label, re, script, body.clone(), catalogs)).collect();
                Variant { name, properties }
            })
            .collect()
    };
    (read("buildTypes", BUILD_TYPE), read("productFlavors", FLAVOR))
}

static AGP_PLUGIN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"id\s*\(?\s*["']com\.android\.[\w.]+["']\s*\)?\s*version\s*\(?\s*["']([\w.\-+]+)["']"#).unwrap());
static AGP_CLASSPATH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"["']com\.android\.tools\.build:gradle:([\w.\-+]+)["']"#).unwrap());

/// Project › Android Gradle Plugin Version: everywhere it's written (the
/// root script's `com.android.*` plugins or `buildscript` classpath, the
/// settings script's plugins, the catalog's plugins or their `[versions]`
/// entry); a new version goes to each, as AS sets every one.
pub fn agp_sites(root_script: &str, settings: &str, catalogs: &[(Catalog, String)]) -> Vec<(Site, String)> {
    let mut sites = Vec::new();
    for (place, text) in [(Place::Script, root_script), (Place::Settings, settings)] {
        for re in [&*AGP_PLUGIN, &*AGP_CLASSPATH] {
            for c in re.captures_iter(text) {
                let m = c.get(1).unwrap();
                sites.push((Site { place, range: m.range(), kind: Kind::Quoted('"') }, m.as_str().to_owned()));
            }
        }
    }
    for (ix, (catalog, _)) in catalogs.iter().enumerate() {
        for e in catalog.entries.iter().filter(|e| e.section == Section::Plugins && e.module.as_deref().is_some_and(|m| m.starts_with("com.android."))) {
            let quoted = match (&e.version, &e.version_ref) {
                (Some(v), _) => Some(v.clone()),
                (None, Some((key, _))) => catalog.entries.iter().find(|v| v.section == Section::Versions && v.key == *key).and_then(|v| v.version.clone()),
                _ => None,
            };
            if let Some((value, range)) = quoted {
                if !sites.iter().any(|(s, _): &(Site, String)| s.place == Place::Catalog(ix) && s.range == range) {
                    sites.push((Site { place: Place::Catalog(ix), range, kind: Kind::Quoted('"') }, value));
                }
            }
        }
    }
    sites
}

/// `gradle-wrapper.properties` with another Gradle version in its
/// `distributionUrl`; none when it has no such line.
pub fn set_wrapper_version(properties: &str, version: &str) -> Option<String> {
    static URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^(\s*distributionUrl\s*=.*gradle-)([^/\s]+?)(-(?:bin|all))?(\.zip)").unwrap());
    let c = URL.captures(properties)?;
    let m = c.get(2)?;
    Some(format!("{}{}{}", &properties[..m.start()], version.trim(), &properties[m.end()..]))
}

/// `sdk.dir` from `local.properties`, unescaped (`C\:\\Users` is
/// `C:\Users`).
pub fn sdk_dir(properties: &str) -> Option<String> {
    properties.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        if key.trim() != "sdk.dir" {
            return None;
        }
        let mut out = String::new();
        let mut chars = value.trim().chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                out.extend(chars.next());
            } else {
                out.push(c);
            }
        }
        (!out.is_empty()).then_some(out)
    })
}

/// `local.properties` with `sdk.dir` set as Android Studio writes it
/// (`C\:\\Users\\…`), or removed for an empty path.
pub fn set_sdk_dir(properties: &str, path: &str) -> String {
    let escaped: String = path.trim().chars().flat_map(|c| match c {
        '\\' => vec!['\\', '\\'],
        ':' => vec!['\\', ':'],
        c => vec![c],
    }).collect();
    let mut lines: Vec<String> = properties.lines().map(str::to_owned).collect();
    let at = lines.iter().position(|l| l.split_once('=').is_some_and(|(k, _)| k.trim() == "sdk.dir"));
    match (at, path.trim().is_empty()) {
        (Some(i), true) => drop(lines.remove(i)),
        (Some(i), false) => lines[i] = format!("sdk.dir={escaped}"),
        (None, false) => lines.push(format!("sdk.dir={escaped}")),
        (None, true) => {}
    }
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const KTS: &str = r#"plugins { id("com.android.application") }
android {
    namespace = "com.example.app"
    compileSdk = libs.versions.compileSdk.get().toInt()
    buildToolsVersion = "34.0.0"
    defaultConfig {
        applicationId = "com.example.app"
        minSdk = 24
        targetSdk = rootProject.extra["target"] as Int
        versionCode = 7
        versionName = "1.2"
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_17
    }
}
"#;

    fn catalog() -> Vec<(Catalog, String)> {
        let text = "[versions]\ncompileSdk = \"34\"\nagp = \"8.5.0\"\n[plugins]\nandroid-application = { id = \"com.android.application\", version.ref = \"agp\" }\nandroid-library = { id = \"com.android.library\", version.ref = \"agp\" }\n";
        vec![(Catalog::parse("libs", "gradle/libs.versions.toml", text), text.to_owned())]
    }

    #[test]
    fn reads_a_modules_values_as_written() {
        let (props, defaults) = module_properties(KTS, &catalog());
        let value = |list: &[Property], label: &str| list.iter().find(|p| p.label == label).unwrap().clone();
        let compile = value(&props, "Compile Sdk Version");
        assert_eq!((compile.value.as_str(), compile.site.as_ref().map(|s| s.place)), ("34", Some(Place::Catalog(0))));
        assert_eq!(value(&props, "Build Tools Version").value, "34.0.0");
        assert_eq!(value(&props, "NDK Version").site, None);
        assert_eq!(value(&props, "Source Compatibility").value, "1.8");
        assert_eq!(value(&defaults, "Min SDK Version").value, "24");
        let target = value(&defaults, "Target SDK Version");
        assert_eq!((target.site, target.expression.as_deref()), (None, Some("rootProject.extra[\"target\"] as Int")));
        assert_eq!(value(&defaults, "Version Name").value, "1.2");
    }

    #[test]
    fn writes_values_the_same_way() {
        let (props, defaults) = module_properties(KTS, &catalog());
        let site = |list: &[Property], label: &str| list.iter().find(|p| p.label == label).unwrap().site.clone().unwrap();
        let text = write(KTS, &site(&defaults, "Min SDK Version"), "26");
        assert!(text.contains("minSdk = 26\n"));
        let text = write(KTS, &site(&defaults, "Version Name"), "2.0");
        assert!(text.contains("versionName = \"2.0\""));
        let text = write(KTS, &site(&props, "Target Compatibility"), "21");
        assert!(text.contains("JavaVersion.VERSION_21"));
        let text = write(&catalog()[0].1, &site(&props, "Compile Sdk Version"), "35");
        assert!(text.contains("compileSdk = \"35\""));
        assert!(check(&site(&defaults, "Version Code"), "x").is_err());
    }

    #[test]
    fn reads_groovy() {
        let groovy = "android {\n    compileSdkVersion 33\n    defaultConfig {\n        minSdkVersion 21\n        applicationId 'com.example'\n    }\n}\n";
        let (props, defaults) = module_properties(groovy, &[]);
        assert_eq!(props[0].value, "33");
        assert_eq!(defaults[1].value, "21");
        assert_eq!(defaults[0].site.as_ref().map(|s| s.kind), Some(Kind::Quoted('\'')));
    }

    #[test]
    fn reads_build_types_and_flavors() {
        let kts = "android {\n    productFlavors { create(\"x\") { minSdk = 19 } }\n    defaultConfig { minSdk = 24 }\n    buildTypes {\n        release {\n            isMinifyEnabled = true\n        }\n        getByName(\"debug\") {\n            applicationIdSuffix = \".debug\"\n        }\n        create(\"staging\") {\n            initWith(getByName(\"debug\"))\n        }\n    }\n    flavorDimensions += \"tier\"\n    productFlavors {\n        create(\"free\") {\n            dimension = \"tier\"\n            minSdk = 21\n        }\n    }\n}\n";
        assert_eq!(module_properties(kts, &[]).1[1].value, "24");
        let kts = &kts.replace("    productFlavors { create(\"x\") { minSdk = 19 } }\n", "");
        let (types, flavors) = variants(kts, &[]);
        assert_eq!(types.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["release", "debug", "staging"]);
        let row = |v: &Variant, label: &str| v.properties.iter().find(|p| p.label == label).unwrap().clone();
        let minify = row(&types[0], "Minify Enabled");
        assert_eq!(minify.value, "true");
        assert_eq!(write(kts, minify.site.as_ref().unwrap(), "false").contains("isMinifyEnabled = false"), true);
        assert_eq!(row(&types[1], "Application Id Suffix").value, ".debug");
        assert_eq!(row(&types[2], "Debuggable").site, None);
        let min = row(&flavors[0], "Min SDK Version");
        assert_eq!((flavors[0].name.as_str(), min.value.as_str()), ("free", "21"));
        assert!(write(kts, min.site.as_ref().unwrap(), "23").contains("minSdk = 23\n        }"));
        assert!(check(minify.site.as_ref().unwrap(), "yes").is_err());
        let groovy = "android {\n    buildTypes {\n        release {\n            minifyEnabled false\n        }\n    }\n}\n";
        assert_eq!(variants(groovy, &[]).0[0].properties[2].value, "false");
    }

    #[test]
    fn finds_the_agp_version() {
        let sites = agp_sites("", "", &catalog());
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].1, "8.5.0");
        let root = "plugins {\n    id(\"com.android.application\") version \"8.4.0\" apply false\n    id(\"com.android.library\") version \"8.4.0\" apply false\n}\n";
        assert_eq!(agp_sites(root, "", &[]).len(), 2);
        let classpath = "buildscript { dependencies { classpath 'com.android.tools.build:gradle:7.4.2' } }";
        assert_eq!(agp_sites(classpath, "", &[])[0].1, "7.4.2");
    }

    #[test]
    fn edits_the_wrapper_and_sdk_location() {
        let wrapper = "distributionBase=GRADLE_USER_HOME\ndistributionUrl=https\\://services.gradle.org/distributions/gradle-8.7-bin.zip\n";
        assert_eq!(set_wrapper_version(wrapper, "8.14.3").unwrap(), wrapper.replace("8.7", "8.14.3"));
        assert_eq!(set_wrapper_version("x=1", "8.0"), None);
        assert_eq!(set_sdk_dir("a=1\nsdk.dir=/old\n", "C:\\Users\\me\\Sdk"), "a=1\nsdk.dir=C\\:\\\\Users\\\\me\\\\Sdk\n");
        assert_eq!(set_sdk_dir("", "/opt/sdk"), "sdk.dir=/opt/sdk\n");
        assert_eq!(set_sdk_dir("sdk.dir=/x\n", ""), "");
    }
}
