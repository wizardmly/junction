//! Gradle version catalogs (`gradle/libs.versions.toml`) as Android Studio
//! reads them: the `[versions]`, `[libraries]`, `[bundles]` and `[plugins]`
//! entries, the type-safe accessors build scripts use for them
//! (`libs.androidx.core.ktx`, `libs.plugins.android.application`,
//! `libs.versions.agp`), and the references inside the catalog itself
//! (`version.ref = "agp"`, a bundle's members).

use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    Versions,
    Libraries,
    Bundles,
    Plugins,
}

impl Section {
    fn from_header(header: &str) -> Option<Self> {
        match header {
            "versions" => Some(Self::Versions),
            "libraries" => Some(Self::Libraries),
            "bundles" => Some(Self::Bundles),
            "plugins" => Some(Self::Plugins),
            _ => None,
        }
    }

    /// What an accessor puts between the catalog's name and the entry:
    /// `libs.plugins.x`, `libs.x` for a library.
    pub fn accessor_prefix(self) -> &'static str {
        match self {
            Self::Versions => "versions.",
            Self::Libraries => "",
            Self::Bundles => "bundles.",
            Self::Plugins => "plugins.",
        }
    }
}

/// A quoted string in the catalog: its text and the byte range of the text.
pub type Quoted = (String, Range<usize>);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub section: Section,
    pub key: String,
    /// The key's bytes, and its 0-based line and character column.
    pub key_range: Range<usize>,
    pub line: u32,
    pub col: u32,
    /// `group:artifact` of a library, the id of a plugin.
    pub module: Option<String>,
    /// The version written in place: a `[versions]` entry's value, or a
    /// library's / plugin's `version = "…"` (or the one in `"g:a:v"`).
    pub version: Option<Quoted>,
    /// `version.ref = "…"`: the `[versions]` key.
    pub version_ref: Option<Quoted>,
    /// A bundle's libraries.
    pub members: Vec<Quoted>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalog {
    /// What build scripts call it: `libs`.
    pub name: String,
    /// Relative to the project root.
    pub path: String,
    pub entries: Vec<Entry>,
}

/// The accessor path of a key: Gradle turns `-`, `_` and `.` into `.`
/// (`androidx-core_ktx` is `androidx.core.ktx`).
pub fn accessor(key: &str) -> String {
    key.replace(['-', '_'], ".")
}

/// A catalog's name from its file: `libs.versions.toml` is `libs`.
pub fn name_of(file_name: &str) -> Option<&str> {
    file_name.strip_suffix(".versions.toml").filter(|n| !n.is_empty())
}

static KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^\s*([A-Za-z0-9_.\-]+|"[^"]+")\s*="#).unwrap());
static QUOTED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""([^"]*)""#).unwrap());

/// `name = "value"` inside an inline table: the value and its range in `line`.
fn field(line: &str, name: &str) -> Option<(String, Range<usize>)> {
    let re = Regex::new(&format!(r#"(?:^|[\s{{,])({})\s*=\s*"([^"]*)""#, regex::escape(name))).ok()?;
    let c = re.captures(line)?;
    let m = c.get(2)?;
    Some((m.as_str().to_owned(), m.range()))
}

/// A rich version (`{ strictly = "…" }`, `{ require = "…" }`, `{ prefer = "…" }`).
fn rich_version(table: &str) -> Option<(String, Range<usize>)> {
    ["strictly", "require", "prefer"].into_iter().find_map(|k| field(table, k))
}

fn shift((text, range): (String, Range<usize>), by: usize) -> Quoted {
    (text, range.start + by..range.end + by)
}

impl Catalog {
    pub fn parse(name: &str, path: &str, text: &str) -> Catalog {
        let mut entries = Vec::new();
        let mut section = None;
        let mut offset = 0;
        let mut lines = text.split_inclusive('\n').enumerate().peekable();
        while let Some((line_no, raw)) = lines.next() {
            let start = offset;
            offset += raw.len();
            let line = raw.trim_end_matches(['\n', '\r']);
            let trimmed = line.trim_start();
            if trimmed.starts_with('#') || trimmed.is_empty() {
                continue;
            }
            if let Some(header) = trimmed.strip_prefix('[').and_then(|h| h.split(']').next()) {
                section = Section::from_header(header.trim());
                continue;
            }
            let Some(section) = section else { continue };
            let Some(caps) = KEY.captures(line) else { continue };
            let key_match = caps.get(1).unwrap();
            let key = key_match.as_str().trim_matches('"').to_owned();
            let value_at = caps.get(0).unwrap().end();
            let value = &line[value_at..];
            let col = line[..key_match.start()].chars().count() as u32;
            let mut entry = Entry {
                section,
                key,
                key_range: start + key_match.start()..start + key_match.end(),
                line: line_no as u32,
                col,
                module: None,
                version: None,
                version_ref: None,
                members: Vec::new(),
            };
            let at = start + value_at;
            let value_trim = value.trim_start();
            match section {
                Section::Versions => {
                    entry.version = if value_trim.starts_with('{') { rich_version(value).map(|q| shift(q, at)) } else { QUOTED.captures(value).map(|c| shift((c[1].to_owned(), c.get(1).unwrap().range()), at)) };
                }
                Section::Libraries | Section::Plugins if value_trim.starts_with('"') => {
                    // `"group:artifact:version"`, `"plugin.id:version"`.
                    if let Some(c) = QUOTED.captures(value) {
                        let m = c.get(1).unwrap();
                        let s = m.as_str();
                        match s.rfind(':').filter(|_| section == Section::Plugins || s.matches(':').count() == 2) {
                            Some(colon) => {
                                entry.module = Some(s[..colon].to_owned());
                                entry.version = Some((s[colon + 1..].to_owned(), at + m.start() + colon + 1..at + m.end()));
                            }
                            None => entry.module = Some(s.to_owned()),
                        }
                    }
                }
                Section::Libraries | Section::Plugins => {
                    entry.module = match section {
                        Section::Plugins => field(value, "id").map(|q| q.0),
                        _ => field(value, "module").map(|q| q.0).or_else(|| Some(format!("{}:{}", field(value, "group")?.0, field(value, "name")?.0))),
                    };
                    entry.version_ref = field(value, "version.ref").map(|q| shift(q, at));
                    if entry.version_ref.is_none() {
                        entry.version = field(value, "version").map(|q| shift(q, at)).or_else(|| {
                            let rich = value.find("version")?;
                            let table = &value[rich..];
                            let table = &table[..table.find('}').map_or(table.len(), |e| e + 1)];
                            rich_version(table).map(|q| shift(q, at + rich))
                        });
                    }
                }
                Section::Bundles => {
                    // The array may go on over the next lines.
                    let mut collect = |s: &str, base: usize| {
                        for c in QUOTED.captures_iter(s) {
                            let m = c.get(1).unwrap();
                            entry.members.push((m.as_str().to_owned(), base + m.start()..base + m.end()));
                        }
                    };
                    collect(value, at);
                    let mut closed = value.contains(']');
                    while !closed {
                        let Some((_, next)) = lines.next() else { break };
                        let base = offset;
                        offset += next.len();
                        let body = next.split('#').next().unwrap_or_default();
                        collect(body, base);
                        closed = body.contains(']');
                    }
                }
            }
            entries.push(entry);
        }
        Catalog { name: name.to_owned(), path: path.to_owned(), entries }
    }

    /// The entry an accessor path names in a section.
    pub fn find(&self, section: Section, path: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.section == section && accessor(&e.key) == path)
    }

    /// The entry with this key, as references inside the catalog spell it.
    pub fn by_key(&self, section: Section, key: &str) -> Option<&Entry> {
        let path = accessor(key);
        self.find(section, &path)
    }

    /// The version an entry ends up with: its own or the one it refers to.
    pub fn version_of(&self, entry: &Entry) -> Option<String> {
        if let Some((v, _)) = &entry.version {
            return Some(v.clone());
        }
        let (key, _) = entry.version_ref.as_ref()?;
        self.by_key(Section::Versions, key)?.version.as_ref().map(|v| v.0.clone())
    }

    /// What hover shows: `androidx.core:core-ktx:1.13.1`, `com.android.application:8.5.0`, a bundle's members.
    pub fn describe(&self, entry: &Entry) -> String {
        match entry.section {
            Section::Versions => entry.version.as_ref().map(|v| v.0.clone()).unwrap_or_default(),
            Section::Bundles => entry.members.iter().filter_map(|(m, _)| self.by_key(Section::Libraries, m)).map(|e| self.describe(e)).collect::<Vec<_>>().join("\n"),
            Section::Libraries | Section::Plugins => {
                let module = entry.module.clone().unwrap_or_default();
                match self.version_of(entry) {
                    Some(v) => format!("{module}:{v}"),
                    None => module,
                }
            }
        }
    }

    /// The accessor of an entry: `libs.androidx.core.ktx`.
    pub fn accessor_of(&self, entry: &Entry) -> String {
        format!("{}.{}{}", self.name, entry.section.accessor_prefix(), accessor(&entry.key))
    }

    /// What is at `offset` of this catalog's own text: an entry's key (a
    /// declaration, `true`), or a reference to one (`version.ref`, a
    /// bundle member).
    pub fn reference_at(&self, offset: usize) -> Option<(&Entry, bool)> {
        let inside = |r: &Range<usize>| r.start <= offset && offset <= r.end;
        for e in &self.entries {
            if inside(&e.key_range) {
                return Some((e, true));
            }
            if let Some((key, r)) = &e.version_ref {
                if inside(r) {
                    return self.by_key(Section::Versions, key).map(|t| (t, false));
                }
            }
            for (key, r) in &e.members {
                if inside(r) {
                    return self.by_key(Section::Libraries, key).map(|t| (t, false));
                }
            }
        }
        None
    }

    /// The places in the catalog that refer to `target`: `version.ref`s of
    /// a version, bundle members of a library.
    pub fn references_to(&self, target: &Entry) -> Vec<Range<usize>> {
        let path = accessor(&target.key);
        let mut out = Vec::new();
        for e in &self.entries {
            match target.section {
                Section::Versions => out.extend(e.version_ref.iter().filter(|(k, _)| accessor(k) == path).map(|(_, r)| r.clone())),
                Section::Libraries => out.extend(e.members.iter().filter(|(k, _)| accessor(k) == path).map(|(_, r)| r.clone())),
                _ => {}
            }
        }
        out
    }

    /// Resolves an accessor path after the catalog's name
    /// (`androidx.core.ktx`, `plugins.android.application.get`): the
    /// longest entry it starts with, as trailing calls like `.get()` or
    /// `.asProvider()` don't change what it names.
    pub fn resolve(&self, path: &str) -> Option<&Entry> {
        let (section, rest) = [Section::Plugins, Section::Versions, Section::Bundles]
            .into_iter()
            .find_map(|s| path.strip_prefix(s.accessor_prefix()).map(|r| (s, r)))
            .unwrap_or((Section::Libraries, path));
        let parts: Vec<&str> = rest.split('.').filter(|p| !p.is_empty()).collect();
        (1..=parts.len()).rev().find_map(|n| self.find(section, &parts[..n].join(".")))
    }

    /// Completion after `libs.` + `typed`: the next path segments, each
    /// with the entry it completes (when it does).
    pub fn next_segments(&self, typed: &str) -> Vec<(String, Option<&Entry>)> {
        let mut out: Vec<(String, Option<&Entry>)> = Vec::new();
        for e in &self.entries {
            let full = format!("{}{}", e.section.accessor_prefix(), accessor(&e.key));
            let Some(rest) = full.strip_prefix(typed) else { continue };
            let (segment, entry) = match rest.split_once('.') {
                Some((segment, _)) => (segment, None),
                None => (rest, Some(e)),
            };
            match out.iter_mut().find(|(s, _)| s == segment) {
                Some(existing) => existing.1 = existing.1.or(entry),
                None => out.push((segment.to_owned(), entry)),
            }
        }
        out
    }
}

/// The catalog accessor at `offset` of a build script: the catalog's name,
/// the path after it, and the bytes of the whole chain.
pub fn accessor_at<'a>(text: &str, offset: usize, names: impl IntoIterator<Item = &'a str>) -> Option<(String, String, Range<usize>)> {
    let is_part = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '.';
    let offset = offset.min(text.len());
    let start = text[..offset].rfind(|c: char| !is_part(c)).map_or(0, |i| i + 1);
    let end = text[offset..].find(|c: char| !is_part(c)).map_or(text.len(), |i| offset + i);
    let chain = text[start..end].trim_end_matches('.');
    let (first, rest) = chain.split_once('.')?;
    let name = names.into_iter().find(|n| *n == first)?;
    Some((name.to_owned(), rest.to_owned(), start..start + chain.len()))
}

/// Completion in a build script: the catalog's name and what is typed
/// after it, when the text before `offset` is `libs.` + segments.
pub fn completion_prefix<'a>(text: &str, offset: usize, names: impl IntoIterator<Item = &'a str>) -> Option<(String, String)> {
    let before = &text[..offset.min(text.len())];
    let start = before.rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.')).map_or(0, |i| i + 1);
    let chain = &before[start..];
    let (first, rest) = chain.split_once('.')?;
    let name = names.into_iter().find(|n| *n == first)?;
    Some((name.to_owned(), rest.to_owned()))
}

/// A build script or catalog file Gradle reads.
pub fn is_build_script(path: &str) -> bool {
    path.ends_with(".gradle") || path.ends_with(".gradle.kts")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOML: &str = r#"[versions]
agp = "8.5.0"
coreKtx = "1.10.1"
compose = { strictly = "1.6.0" }

[libraries]
androidx-core-ktx = { group = "androidx.core", name = "core-ktx", version.ref = "coreKtx" }
androidx-core = { module = "androidx.core:core", version.ref = "coreKtx" }
okhttp = "com.squareup.okhttp3:okhttp:4.11.0"
junit = { module = "junit:junit", version = "4.13.2" }

[bundles]
core = [
    "androidx-core-ktx", # the KTX one
    "androidx-core",
]

[plugins]
android-application = { id = "com.android.application", version.ref = "agp" }
kotlin = "org.jetbrains.kotlin.android:1.9.0"
"#;

    fn catalog() -> Catalog {
        Catalog::parse("libs", "gradle/libs.versions.toml", TOML)
    }

    #[test]
    fn parses_every_notation() {
        let c = catalog();
        let core = c.find(Section::Libraries, "androidx.core.ktx").unwrap();
        assert_eq!(core.module.as_deref(), Some("androidx.core:core-ktx"));
        assert_eq!(c.version_of(core).as_deref(), Some("1.10.1"));
        assert_eq!((core.line, core.col), (6, 0));
        let okhttp = c.find(Section::Libraries, "okhttp").unwrap();
        assert_eq!(okhttp.module.as_deref(), Some("com.squareup.okhttp3:okhttp"));
        let (v, r) = okhttp.version.clone().unwrap();
        assert_eq!((v.as_str(), &TOML[r]), ("4.11.0", "4.11.0"));
        assert_eq!(c.find(Section::Versions, "compose").unwrap().version.as_ref().unwrap().0, "1.6.0");
        assert_eq!(c.find(Section::Libraries, "junit").unwrap().version.as_ref().unwrap().0, "4.13.2");
        let bundle = c.find(Section::Bundles, "core").unwrap();
        assert_eq!(bundle.members.iter().map(|m| m.0.as_str()).collect::<Vec<_>>(), ["androidx-core-ktx", "androidx-core"]);
        assert_eq!(&TOML[bundle.members[1].1.clone()], "androidx-core");
        let kotlin = c.find(Section::Plugins, "kotlin").unwrap();
        assert_eq!((kotlin.module.as_deref(), c.version_of(kotlin).as_deref()), (Some("org.jetbrains.kotlin.android"), Some("1.9.0")));
        assert_eq!(c.describe(c.find(Section::Plugins, "android.application").unwrap()), "com.android.application:8.5.0");
    }

    #[test]
    fn resolves_accessors_from_build_scripts() {
        let c = catalog();
        let script = "dependencies { implementation(libs.androidx.core.ktx) }\nplugins { alias(libs.plugins.android.application) }\nval v = libs.versions.agp.get()";
        let at = script.find("core.ktx").unwrap();
        let (name, path, range) = accessor_at(script, at, ["libs"]).unwrap();
        assert_eq!((name.as_str(), path.as_str(), &script[range]), ("libs", "androidx.core.ktx", "libs.androidx.core.ktx"));
        assert_eq!(c.resolve(&path).unwrap().key, "androidx-core-ktx");
        let (_, path, _) = accessor_at(script, script.find("application").unwrap(), ["libs"]).unwrap();
        assert_eq!(c.resolve(&path).unwrap().section, Section::Plugins);
        let (_, path, _) = accessor_at(script, script.find("agp").unwrap(), ["libs"]).unwrap();
        assert_eq!(c.resolve(&path).unwrap().key, "agp");
        // `libs.androidx.core` is its own library, not a prefix of the KTX one.
        assert_eq!(c.resolve("androidx.core").unwrap().key, "androidx-core");
        assert!(accessor_at("foo.libs.x", 6, ["libs"]).is_none());
    }

    #[test]
    fn references_inside_the_catalog() {
        let c = catalog();
        let at = TOML.find("version.ref = \"coreKtx\"").unwrap() + 15;
        let (target, declaration) = c.reference_at(at).unwrap();
        assert_eq!((target.key.as_str(), declaration), ("coreKtx", false));
        assert_eq!(c.references_to(target).len(), 2);
        let (target, declaration) = c.reference_at(TOML.find("okhttp =").unwrap() + 1).unwrap();
        assert_eq!((target.key.as_str(), declaration), ("okhttp", true));
        let member = TOML.find("\"androidx-core\",").unwrap() + 2;
        assert_eq!(c.reference_at(member).unwrap().0.key, "androidx-core");
    }

    #[test]
    fn completes_next_segments() {
        let c = catalog();
        let first: Vec<String> = c.next_segments("").into_iter().map(|s| s.0).collect();
        assert!(first.contains(&"androidx".to_owned()) && first.contains(&"plugins".to_owned()) && first.contains(&"versions".to_owned()));
        let core = c.next_segments("androidx.");
        assert_eq!(core.len(), 1);
        assert_eq!(core[0].0, "core");
        assert_eq!(core[0].1.unwrap().key, "androidx-core");
        assert_eq!(completion_prefix("implementation(libs.andr", 24, ["libs"]), Some(("libs".into(), "andr".into())));
    }
}
