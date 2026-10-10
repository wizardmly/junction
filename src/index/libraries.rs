//! External libraries, as IntelliJ's "External Libraries": the sources of
//! a project's dependencies and SDKs, found from its build files (Cargo.lock,
//! go.mod, package_config.json, Gradle files and the Gradle cache, Android
//! SDK, JDK, include paths, SwiftPM / CocoaPods, node_modules…). They are
//! indexed like project files, cached per library across projects, and open
//! read-only.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use regex::Regex;
use serde::{Deserialize, Serialize};

use super::lang::Lang;
use super::store::{self, FileEntry};

/// Bumped whenever extraction changes, so stale caches are rebuilt.
const CACHE_VERSION: u32 = 3;
/// Per library; bigger ones are cut (generated or vendored code).
const MAX_LIBRARY_FILES: usize = 12_000;
/// Headers reached through `#include` from the project.
const MAX_HEADERS: usize = 8_000;
/// Artifacts followed through Gradle dependency metadata.
const MAX_ARTIFACTS: usize = 600;

/// One library: a root and the source files under it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Library {
    /// "serde 1.0.219", "< jbr-21 >", "Gradle: androidx.core:core:1.13.1@aar"…
    pub name: String,
    /// The Project view's grouping: "Cargo", "Gradle", "JDK"…
    pub group: &'static str,
    pub root: PathBuf,
    /// The archive the sources stand for, shown as the library's root
    /// ("core-1.13.1.jar  library root"); files sit right under the library without it.
    pub root_label: Option<String>,
    /// Shown greyed after the name, as IntelliJ shows an SDK's home.
    pub location: Option<String>,
    /// Only these files; otherwise every source file under `root`.
    pub only: Option<Vec<PathBuf>>,
    /// Paths under `root` (with `/`) that are left out.
    pub exclude: Vec<&'static str>,
}

/// What the Project view and Go to File show of a library.
#[derive(Clone, Debug)]
pub struct LibraryInfo {
    pub name: String,
    pub group: &'static str,
    pub root: PathBuf,
    pub root_label: Option<String>,
    pub location: Option<String>,
    /// Absolute paths, sorted.
    pub files: Vec<String>,
}

/// Every library's index, keyed by absolute path.
#[derive(Default)]
pub struct ExternalIndex {
    pub libraries: Vec<LibraryInfo>,
    pub files: HashMap<String, FileEntry>,
    names: HashMap<String, Vec<(Arc<str>, u32)>>,
    /// What built each library, to reuse it when nothing changed.
    built: Vec<(Library, Vec<String>)>,
    /// Discovery ran (a project may have no libraries).
    pub scanned: bool,
    /// [`inputs_stamp`] when it ran.
    pub stamp: u64,
}

impl ExternalIndex {
    pub fn is_empty(&self) -> bool {
        self.libraries.is_empty()
    }

    pub fn symbol_count(&self) -> usize {
        self.files.values().map(|f| f.symbols.len()).sum()
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.names.keys()
    }

    pub fn symbols_named<'a>(&'a self, name: &str) -> impl Iterator<Item = (&'a str, &'a FileEntry, &'a super::symbols::Symbol)> + 'a {
        self.names.get(name).into_iter().flatten().filter_map(|(path, i)| {
            let (path, entry) = self.files.get_key_value(&**path)?;
            Some((path.as_str(), entry, entry.symbols.get(*i as usize)?))
        })
    }

    /// The library a file belongs to.
    pub fn library_of(&self, path: &str) -> Option<&LibraryInfo> {
        self.libraries.iter().find(|l| l.files.binary_search_by(|f| f.as_str().cmp(path)).is_ok())
    }

    fn rebuild_names(&mut self) {
        self.names.clear();
        for (path, entry) in &self.files {
            let path: Arc<str> = Arc::from(path.as_str());
            for (i, s) in entry.symbols.iter().enumerate() {
                self.names.entry(s.name.clone()).or_default().push((path.clone(), i as u32));
            }
        }
    }

    /// Indexes `libraries`, reusing `previous` for the ones that didn't
    /// change and the on-disk cache for the rest.
    pub fn build(libraries: Vec<Library>, stamp: u64, previous: Option<&ExternalIndex>, progress: &(dyn Fn(usize, usize) + Sync)) -> ExternalIndex {
        let mut out = ExternalIndex { scanned: true, stamp, ..Default::default() };
        let mut seen_files: HashSet<String> = HashSet::new();
        let listed: Vec<(Library, Vec<String>)> = libraries
            .into_iter()
            .map(|lib| {
                let files = list_library(&lib);
                (lib, files)
            })
            .collect();
        let total: usize = listed.iter().map(|(_, f)| f.len()).sum();
        let done = AtomicUsize::new(0);
        for (lib, files) in listed {
            let reused = previous.and_then(|p| p.built.iter().position(|(l, f)| *l == lib && *f == files)).map(|_| {
                let previous = previous.unwrap();
                files.iter().filter_map(|f| previous.files.get(f).map(|e| (f.clone(), e.clone()))).collect::<Vec<_>>()
            });
            let entries = match reused {
                Some(entries) => {
                    done.fetch_add(files.len(), Ordering::Relaxed);
                    entries
                }
                None => index_library(&lib, &files, &done, total, progress),
            };
            let entries = within_budget(entries);
            let mut indexed: Vec<String> = Vec::new();
            for (path, entry) in entries {
                if seen_files.insert(path.clone()) {
                    indexed.push(path.clone());
                    out.files.insert(path, entry);
                }
            }
            indexed.sort();
            if !indexed.is_empty() {
                out.libraries.push(LibraryInfo {
                    name: lib.name.clone(),
                    group: lib.group,
                    root: lib.root.clone(),
                    root_label: lib.root_label.clone(),
                    location: lib.location.clone(),
                    files: indexed,
                });
            }
            out.built.push((lib, files));
        }
        out.libraries.sort_by(|a, b| group_rank(a.group).cmp(&group_rank(b.group)).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        out.rebuild_names();
        out
    }
}

/// Symbols one library may keep in memory. Generated bindings (libc,
/// linux-raw-sys, platform SDK wrappers) go past it: they keep their types
/// and functions, then are cut.
const SYMBOL_BUDGET: usize = 60_000;

fn within_budget(mut entries: Vec<(String, FileEntry)>) -> Vec<(String, FileEntry)> {
    let total: usize = entries.iter().map(|(_, e)| e.symbols.len()).sum();
    if total <= SYMBOL_BUDGET {
        return entries;
    }
    use super::symbols::SymbolKind;
    let mut kept = 0;
    for (_, entry) in entries.iter_mut() {
        entry.symbols.retain(|s| {
            matches!(s.kind, SymbolKind::Function | SymbolKind::Method | SymbolKind::Constructor | SymbolKind::Macro | SymbolKind::Module) || s.kind.is_type()
        });
        if kept >= SYMBOL_BUDGET {
            entry.symbols.clear();
        }
        kept += entry.symbols.len();
    }
    entries.retain(|(_, e)| !e.symbols.is_empty());
    entries
}

/// SDKs before dependencies, as IntelliJ lists them.
fn group_rank(group: &str) -> usize {
    ["JDK", "Android SDK", "Rust", "Go SDK", "Dart SDK", "SDK", "C/C++"].iter().position(|g| *g == group).unwrap_or(10)
}

// ---------------------------------------------------------------------------
// Indexing

#[derive(Default, Serialize, Deserialize)]
struct LibraryCache {
    version: u32,
    files: BTreeMap<String, FileEntry>,
}

pub fn cache_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("JUNCTION_CACHE_DIR") {
        return Some(PathBuf::from(dir));
    }
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library/Caches"))
    } else {
        std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).or_else(|| home().map(|h| h.join(".cache")))
    }?;
    Some(base.join("Junction"))
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)
}

fn stable_hash(text: &str) -> u64 {
    // FNV-1a: stable across runs and builds, unlike std's hasher.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

fn cache_file(lib: &Library) -> Option<PathBuf> {
    let key = lib.root.to_string_lossy();
    let name: String = lib.name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).take(60).collect();
    Some(cache_dir()?.join("libraries").join(format!("{name}-{:016x}.bin", stable_hash(&key))))
}

/// The language of a library file: by extension, plus C++ standard
/// headers (no extension) and Swift module interfaces.
pub fn library_lang(path: &str) -> Option<Lang> {
    if let Some(lang) = Lang::from_path(path) {
        return Some(lang);
    }
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    if name.ends_with(".swiftinterface") {
        return Some(Lang::Swift);
    }
    (!name.contains('.')).then_some(Lang::Cpp)
}

fn index_library(lib: &Library, files: &[String], done: &AtomicUsize, total: usize, progress: &(dyn Fn(usize, usize) + Sync)) -> Vec<(String, FileEntry)> {
    let cache_path = cache_file(lib);
    if let Some(path) = &cache_path {
        // The JSON cache of earlier versions.
        std::fs::remove_file(path.with_extension("json")).ok();
    }
    let mut cache: LibraryCache = cache_path
        .as_ref()
        .and_then(|p| std::fs::File::open(p).ok())
        .and_then(|file| bincode::deserialize_from::<_, LibraryCache>(std::io::BufReader::with_capacity(1 << 20, file)).ok())
        .filter(|c| c.version == CACHE_VERSION)
        .unwrap_or_default();
    let mut stale: Vec<&String> = Vec::new();
    let mut out: Vec<(String, FileEntry)> = Vec::new();
    for path in files {
        let current = store::stat(Path::new(path));
        match (cache.files.remove(path), current) {
            (Some(entry), Some((size, mtime))) if entry.size == size && entry.mtime == mtime => {
                out.push((path.clone(), entry));
                done.fetch_add(1, Ordering::Relaxed);
            }
            (_, Some(_)) => stale.push(path),
            _ => {
                done.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    let changed = !stale.is_empty() || !cache.files.is_empty();
    if !stale.is_empty() {
        let next = AtomicUsize::new(0);
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 16);
        let results: Vec<Vec<(String, FileEntry)>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..threads)
                .map(|_| {
                    scope.spawn(|| {
                        let mut out = Vec::new();
                        loop {
                            let i = next.fetch_add(1, Ordering::Relaxed);
                            let Some(path) = stale.get(i) else { break };
                            if let Some(entry) = library_lang(path).and_then(|lang| store::index_path(Path::new(path.as_str()), lang, false)) {
                                out.push(((*path).clone(), entry));
                            }
                            let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                            if d % 128 == 0 {
                                progress(d, total);
                            }
                        }
                        out
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap_or_default()).collect()
        });
        out.extend(results.into_iter().flatten());
    }
    if changed {
        if let Some(path) = cache_path {
            let cache = LibraryCache { version: CACHE_VERSION, files: out.iter().cloned().collect() };
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).ok();
            }
            if let Ok(bytes) = bincode::serialize(&cache) {
                let tmp = path.with_extension("bin.tmp");
                if std::fs::write(&tmp, bytes).is_ok() {
                    std::fs::rename(&tmp, &path).ok();
                }
            }
        }
    }
    out
}

/// Directories no library's sources need.
const SKIP_DIRS: &[&str] = &[
    ".git", "tests", "test", "testdata", "testing", "benches", "benchmarks", "examples", "example", "__tests__", "fixtures", "target", ".build",
    "node_modules", "__pycache__", "docs", ".github",
];

fn list_library(lib: &Library) -> Vec<String> {
    let mut files: Vec<String> = match &lib.only {
        Some(only) => only.iter().map(|p| p.to_string_lossy().into_owned()).collect(),
        None => {
            let mut out = Vec::new();
            let mut stack = vec![lib.root.clone()];
            while let Some(dir) = stack.pop() {
                let Ok(entries) = std::fs::read_dir(&dir) else { continue };
                for entry in entries.flatten() {
                    let path = entry.path();
                    let Ok(kind) = entry.file_type() else { continue };
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if kind.is_dir() {
                        let rel = path.strip_prefix(&lib.root).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
                        if SKIP_DIRS.contains(&name.as_str()) || lib.exclude.iter().any(|e| rel == *e || rel.starts_with(&format!("{e}/"))) {
                            continue;
                        }
                        stack.push(path);
                    } else if kind.is_file() && is_library_source(&name) {
                        out.push(path.to_string_lossy().into_owned());
                        if out.len() >= MAX_LIBRARY_FILES {
                            stack.clear();
                            break;
                        }
                    }
                }
            }
            out
        }
    };
    files.sort();
    files.dedup();
    files
}

fn is_library_source(name: &str) -> bool {
    if name.ends_with("_test.go") || name.ends_with(".min.js") || name.ends_with("Test.java") || name.ends_with("Test.kt") {
        return false;
    }
    Lang::from_path(name).is_some()
}

/// Replaces attribute-like macros in system headers with spaces (offsets
/// and lines stay put), so tree-sitter sees plain declarations.
pub fn blank_macros(text: &str) -> String {
    static MACROS: OnceLock<Regex> = OnceLock::new();
    let re = regex(
        &MACROS,
        r"\b(?:__(?:[A-Z][A-Z0-9_]*|wur|nonnull|restrict|extension__|asm__|asm|attribute__|attribute_\w+|attr_\w+|fortified_\w+|always_inline|inline|leaf|nothrow)|_GLIBCXX\w*|_LIBCPP_\w+|NS_(?:[A-Z][A-Z_]*)|API_(?:AVAILABLE|UNAVAILABLE|DEPRECATED\w*)|CF_(?:[A-Z][A-Z_]*)|UI_APPEARANCE_SELECTOR|JNIEXPORT|JNICALL)\b(?:\s*\((?:[^()]|\((?:[^()]|\([^()]*\))*\))*\))?",
    );
    // Enum-defining Apple macros declare types; keep those.
    re.replace_all(text, |c: &regex::Captures| {
        let m = &c[0];
        if ["NS_ENUM", "NS_OPTIONS", "NS_CLOSED_ENUM", "NS_ERROR_ENUM", "NS_TYPED_ENUM", "NS_TYPED_EXTENSIBLE_ENUM", "CF_ENUM", "CF_OPTIONS"].iter().any(|k| m.starts_with(k)) {
            return m.to_owned();
        }
        m.chars().map(|ch| if ch == '\n' { "\n".to_owned() } else { " ".repeat(ch.len_utf8()) }).collect::<String>()
    })
    .into_owned()
}

// ---------------------------------------------------------------------------
// Discovery

/// Changes when anything discovery reads may have: build files, lock files
/// beside them, and the sources whose imports pick headers and modules.
pub fn inputs_stamp(root: &Path, files: &[String]) -> u64 {
    const MANIFESTS: &[&str] = &[
        "Cargo.toml", "go.mod", "pubspec.yaml", "build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts", "Package.swift",
        "Podfile", "Cartfile", "package.json", "CMakeLists.txt",
    ];
    let mut text = String::new();
    let mut add = |path: &Path| {
        if let Some((size, mtime)) = store::stat(path) {
            text.push_str(&format!("{}:{size}:{mtime};", path.display()));
        }
    };
    for file in files {
        let name = file.rsplit('/').next().unwrap_or(file);
        let path = root.join(file);
        let source = matches!(Lang::from_path(file), Some(Lang::C | Lang::Cpp | Lang::ObjC | Lang::Swift | Lang::Python | Lang::V));
        if MANIFESTS.contains(&name) || name.ends_with(".versions.toml") || source {
            add(&path);
        }
        if MANIFESTS.contains(&name) {
            let dir = path.parent().unwrap_or(root);
            for side in ["Cargo.lock", ".dart_tool/package_config.json", "local.properties", "node_modules", ".build/checkouts", "Pods", "go.sum"] {
                add(&dir.join(side));
            }
        }
    }
    add(&root.join("compile_commands.json"));
    add(&root.join("build/compile_commands.json"));
    stable_hash(&text) ^ files.len() as u64
}

/// The libraries a project uses. `files` are the project's files, relative.
pub fn discover(root: &Path, files: &[String]) -> Vec<Library> {
    let ctx = Project::new(root, files);
    let mut out = Vec::new();
    cargo(&ctx, &mut out);
    go(&ctx, &mut out);
    dart(&ctx, &mut out);
    let android = android_sdk(&ctx, &mut out);
    gradle(&ctx, &mut out);
    jdk(&ctx, &mut out);
    c_family(&ctx, android.as_deref(), &mut out);
    swift(&ctx, &mut out);
    npm(&ctx, &mut out);
    python(&ctx, &mut out);
    vlang(&ctx, &mut out);
    let mut seen = HashSet::new();
    out.retain(|l| seen.insert((l.root.clone(), l.only.is_some())));
    out
}

struct Project<'a> {
    root: &'a Path,
    files: &'a [String],
    langs: HashSet<Lang>,
}

impl<'a> Project<'a> {
    fn new(root: &'a Path, files: &'a [String]) -> Self {
        let langs = files.iter().filter_map(|f| Lang::from_path(f)).collect();
        Self { root, files, langs }
    }

    fn has(&self, langs: &[Lang]) -> bool {
        langs.iter().any(|l| self.langs.contains(l))
    }

    /// Project files with this name (any directory).
    fn named(&self, name: &str) -> Vec<PathBuf> {
        self.files.iter().filter(|f| f.rsplit('/').next() == Some(name)).map(|f| self.root.join(f)).collect()
    }

    fn with_suffix(&self, suffix: &str) -> Vec<PathBuf> {
        self.files.iter().filter(|f| f.ends_with(suffix)).map(|f| self.root.join(f)).collect()
    }

    fn sources(&self, langs: &[Lang]) -> impl Iterator<Item = PathBuf> + '_ {
        let langs = langs.to_vec();
        self.files.iter().filter(move |f| Lang::from_path(f).is_some_and(|l| langs.contains(&l))).map(|f| self.root.join(f))
    }
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    let exe = super::lsp::find_program(program)?;
    let mut command = Command::new(exe);
    command.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let output = command.output().ok()?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Some(text)
}

/// A tool's answer, asked once per run.
fn cached_run(slot: &'static OnceLock<Option<String>>, program: &str, args: &[&str]) -> Option<String> {
    slot.get_or_init(|| run(program, args).map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())).clone()
}

fn dir(path: PathBuf) -> Option<PathBuf> {
    path.is_dir().then_some(path)
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn regex(slot: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    slot.get_or_init(|| Regex::new(pattern).unwrap())
}

/// Orders "1.10.2" after "1.9", "2.0.0-alpha" before "2.0.0".
fn version_key(version: &str) -> (Vec<u64>, bool, String) {
    let version = version.split('+').next().unwrap_or(version);
    let (core, pre) = match version.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (version, None),
    };
    let numbers = core.split('.').map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0)).collect();
    (numbers, pre.is_none(), pre.unwrap_or("").to_owned())
}

fn newest_subdir(dir: &Path, prefix: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_prefix(prefix).map(|v| (version_key(v), e.path()))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, p)| p)
}

fn lib(name: impl Into<String>, group: &'static str, root: PathBuf) -> Library {
    Library { name: name.into(), group, root, root_label: None, location: None, only: None, exclude: Vec::new() }
}

// --- Rust ---------------------------------------------------------------

fn cargo(ctx: &Project, out: &mut Vec<Library>) {
    let manifests = ctx.named("Cargo.toml");
    if manifests.is_empty() {
        return;
    }
    // The lock file sits at the workspace root, often git-ignored.
    let mut locks: BTreeSet<PathBuf> = BTreeSet::new();
    for manifest in &manifests {
        let mut dir = manifest.parent();
        while let Some(d) = dir {
            if d.join("Cargo.lock").is_file() {
                locks.insert(d.join("Cargo.lock"));
                break;
            }
            if d == ctx.root {
                break;
            }
            dir = d.parent();
        }
    }
    let cargo_home = std::env::var_os("CARGO_HOME").map(PathBuf::from).or_else(|| home().map(|h| h.join(".cargo")));
    let Some(cargo_home) = cargo_home else { return };
    let registries: Vec<PathBuf> =
        std::fs::read_dir(cargo_home.join("registry/src")).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    // Outer workspaces first: a vendored or patched crate inside one is
    // part of it, not a project of its own.
    let mut locks: Vec<PathBuf> = locks.into_iter().collect();
    locks.sort_by_key(|l| l.components().count());
    let mut covered: Vec<PathBuf> = Vec::new();
    for lock in locks {
        let workspace = lock.parent().unwrap();
        if covered.iter().any(|c| c == workspace) {
            continue;
        }
        // Cargo knows which crates the host platform builds; the lock file
        // lists every platform's (windows-sys, objc2… on Linux).
        if let Some(crates) = cargo_metadata(workspace) {
            for (name, version, root) in crates {
                if root.starts_with(ctx.root) {
                    covered.push(root);
                } else {
                    out.push(lib(format!("{name} {version}"), "Cargo", root));
                }
            }
            continue;
        }
        for (name, version, source) in parse_cargo_lock(&read(&lock)) {
            if source.starts_with("registry+") || source.starts_with("sparse+") {
                if let Some(root) = registries.iter().map(|r| r.join(format!("{name}-{version}"))).find(|p| p.is_dir()) {
                    out.push(lib(format!("{name} {version}"), "Cargo", root));
                }
            } else if let Some((_, rev)) = source.split_once('#') {
                let short = &rev[..rev.len().min(7)];
                let checkouts = cargo_home.join("git/checkouts");
                for repo in std::fs::read_dir(&checkouts).into_iter().flatten().flatten() {
                    let checkout = repo.path().join(short);
                    if let Some(root) = find_crate(&checkout, &name, 3) {
                        out.push(lib(format!("{name} {version}"), "Cargo", root));
                        break;
                    }
                }
            }
        }
    }
    static SYSROOT: OnceLock<Option<String>> = OnceLock::new();
    if let Some(sysroot) = cached_run(&SYSROOT, "rustc", &["--print", "sysroot"]) {
        let library = PathBuf::from(sysroot).join("lib/rustlib/src/rust/library");
        for krate in ["core", "alloc", "std"] {
            if let Some(src) = dir(library.join(krate).join("src")) {
                out.push(Library { exclude: vec!["arch"], ..lib(format!("Rust {krate}"), "Rust", src) });
            }
        }
    }
}

/// The host's resolved crates: (name, version, root).
fn cargo_metadata(workspace: &Path) -> Option<Vec<(String, String, PathBuf)>> {
    static HOST: OnceLock<Option<String>> = OnceLock::new();
    let host = cached_run(&HOST, "rustc", &["-vV"])?.lines().find_map(|l| l.strip_prefix("host: ").map(str::to_owned))?;
    let exe = super::lsp::find_program("cargo")?;
    let mut command = Command::new(exe);
    command
        .args(["metadata", "--format-version", "1", "--offline", "--filter-platform", &host])
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let output = command.output().ok().filter(|o| o.status.success())?;
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    let resolved: HashSet<&str> = json["resolve"]["nodes"].as_array()?.iter().filter_map(|n| n["id"].as_str()).collect();
    Some(
        json["packages"]
            .as_array()?
            .iter()
            .filter(|p| p["id"].as_str().is_some_and(|id| resolved.contains(id)))
            .filter_map(|p| {
                let root = Path::new(p["manifest_path"].as_str()?).parent()?.to_path_buf();
                Some((p["name"].as_str()?.to_owned(), p["version"].as_str()?.to_owned(), root))
            })
            .collect(),
    )
}

fn parse_cargo_lock(text: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for block in text.split("[[package]]").skip(1) {
        let field = |key: &str| {
            block.lines().find_map(|l| {
                let (k, v) = l.split_once('=')?;
                (k.trim() == key).then(|| v.trim().trim_matches('"').to_owned())
            })
        };
        if let (Some(name), Some(version), Some(source)) = (field("name"), field("version"), field("source")) {
            out.push((name, version, source));
        }
    }
    out
}

fn find_crate(dir: &Path, name: &str, depth: usize) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    let manifest = read(&dir.join("Cargo.toml"));
    if manifest.lines().any(|l| l.replace(' ', "") == format!("name=\"{name}\"")) {
        return Some(dir.to_path_buf());
    }
    if depth == 0 {
        return None;
    }
    std::fs::read_dir(dir).ok()?.flatten().filter(|e| e.path().is_dir()).find_map(|e| find_crate(&e.path(), name, depth - 1))
}

// --- Go -----------------------------------------------------------------

fn go(ctx: &Project, out: &mut Vec<Library>) {
    let mods = ctx.named("go.mod");
    if mods.is_empty() {
        return;
    }
    static GOENV: OnceLock<Option<String>> = OnceLock::new();
    let env = cached_run(&GOENV, "go", &["env", "GOROOT", "GOMODCACHE"]).unwrap_or_default();
    let mut lines = env.lines();
    let goroot = lines.next().map(PathBuf::from).or_else(|| std::env::var_os("GOROOT").map(PathBuf::from));
    let modcache = lines
        .next()
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("GOMODCACHE").map(PathBuf::from))
        .or_else(|| home().map(|h| h.join("go/pkg/mod")));
    if let Some(modcache) = modcache {
        for file in mods {
            for (path, version) in parse_go_mod(&read(&file)) {
                if let Some(root) = dir(modcache.join(format!("{}@{}", go_escape(&path), go_escape(&version)))) {
                    out.push(lib(format!("{path} {version}"), "Go Modules", root));
                }
            }
        }
    }
    if let Some(src) = goroot.and_then(|g| dir(g.join("src"))) {
        let version = read(&src.parent().unwrap().join("VERSION")).lines().next().unwrap_or("").to_owned();
        let name = if version.is_empty() { "Go SDK".to_owned() } else { format!("Go SDK {}", version.trim_start_matches("go")) };
        out.push(Library { exclude: vec!["cmd", "vendor", "internal/types/testdata"], ..lib(name, "Go SDK", src) });
    }
}

fn parse_go_mod(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut in_block = false;
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        if line.starts_with("require (") || line == "require(" {
            in_block = true;
            continue;
        }
        if in_block && line == ")" {
            in_block = false;
            continue;
        }
        let spec = if in_block { Some(line) } else { line.strip_prefix("require ") };
        if let Some(spec) = spec {
            let mut parts = spec.split_whitespace();
            if let (Some(path), Some(version)) = (parts.next(), parts.next()) {
                out.push((path.to_owned(), version.to_owned()));
            }
        }
    }
    out
}

/// The module cache spells capitals as `!` + lowercase.
fn go_escape(path: &str) -> String {
    let mut out = String::new();
    for c in path.chars() {
        if c.is_ascii_uppercase() {
            out.push('!');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

// --- Dart / Flutter -------------------------------------------------------

fn uri_to_path(uri: &str, base: &Path) -> PathBuf {
    match uri.strip_prefix("file://") {
        Some(rest) => {
            let rest = percent_decode(rest);
            // file:///C:/x → C:/x on Windows.
            let rest = if cfg!(windows) { rest.trim_start_matches('/').to_owned() } else { rest };
            PathBuf::from(rest)
        }
        None => base.join(percent_decode(uri)),
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&text[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

fn dart(ctx: &Project, out: &mut Vec<Library>) {
    let pubspecs = ctx.named("pubspec.yaml");
    if pubspecs.is_empty() {
        return;
    }
    let mut sdk: Option<PathBuf> = None;
    for pubspec in pubspecs {
        let project = pubspec.parent().unwrap().to_path_buf();
        let tool = project.join(".dart_tool");
        let Ok(config) = serde_json::from_str::<serde_json::Value>(&read(&tool.join("package_config.json"))) else { continue };
        for package in config["packages"].as_array().into_iter().flatten() {
            let (Some(name), Some(root_uri)) = (package["name"].as_str(), package["rootUri"].as_str()) else { continue };
            let root = normalize(&uri_to_path(root_uri, &tool));
            if root == normalize(&project) || root.starts_with(ctx.root) {
                continue;
            }
            let lib_dir = root.join(package["packageUri"].as_str().unwrap_or("lib/"));
            let label = root.file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| n.contains('-')).unwrap_or_else(|| name.to_owned());
            if let Some(lib_dir) = dir(normalize(&lib_dir)) {
                out.push(lib(label, "Dart Packages", lib_dir));
            }
        }
        if sdk.is_none() {
            sdk = config["flutterRoot"].as_str().map(|r| uri_to_path(r, &tool).join("bin/cache/dart-sdk/lib")).filter(|p| p.is_dir());
        }
    }
    let sdk = sdk.or_else(|| {
        let dart = std::fs::canonicalize(super::lsp::find_program("dart")?).ok()?;
        dir(dart.parent()?.parent()?.join("lib"))
    });
    if let Some(sdk) = sdk {
        out.push(Library { exclude: vec!["_internal"], ..lib("Dart SDK", "Dart SDK", sdk) });
    }
}

// --- Java / Kotlin: Gradle, Android SDK, JDK ------------------------------

fn gradle_texts(ctx: &Project) -> Vec<(PathBuf, String)> {
    let mut paths = ctx.with_suffix("build.gradle");
    paths.extend(ctx.with_suffix("build.gradle.kts"));
    paths.extend(ctx.with_suffix(".versions.toml"));
    paths.into_iter().map(|p| {
        let text = read(&p);
        (p, text)
    }).collect()
}

fn is_android(texts: &[(PathBuf, String)]) -> bool {
    texts.iter().any(|(_, t)| t.contains("com.android.application") || t.contains("com.android.library") || t.contains("android-application") || t.contains("android {"))
}

fn sdk_dir(ctx: &Project) -> Option<PathBuf> {
    let mut props = vec![ctx.root.join("local.properties")];
    props.extend(ctx.named("settings.gradle").iter().chain(ctx.named("settings.gradle.kts").iter()).filter_map(|p| p.parent().map(|d| d.join("local.properties"))));
    for prop in props {
        for line in read(&prop).lines() {
            if let Some(value) = line.trim().strip_prefix("sdk.dir=") {
                let value = value.replace("\\:", ":").replace("\\\\", "\\");
                if Path::new(&value).is_dir() {
                    return Some(PathBuf::from(value));
                }
            }
        }
    }
    for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(d) = std::env::var_os(var).map(PathBuf::from).filter(|d| d.is_dir()) {
            return Some(d);
        }
    }
    let defaults = [
        home().map(|h| h.join("Android/Sdk")),
        home().map(|h| h.join("Library/Android/sdk")),
        std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Android/Sdk")),
    ];
    defaults.into_iter().flatten().find(|d| d.is_dir())
}

/// Adds the Android platform sources; returns the SDK for the NDK.
fn android_sdk(ctx: &Project, out: &mut Vec<Library>) -> Option<PathBuf> {
    let texts = gradle_texts(ctx);
    if texts.is_empty() || !is_android(&texts) {
        return None;
    }
    let sdk = sdk_dir(ctx)?;
    static COMPILE_SDK: OnceLock<Regex> = OnceLock::new();
    let re = regex(&COMPILE_SDK, r#"(?i)compileSdk(?:Version)?\s*(?:=|\(|\s)\s*"?(?:android-)?(\d+)"#);
    let wanted = texts.iter().flat_map(|(_, t)| re.captures_iter(t).filter_map(|c| c[1].parse::<u32>().ok()).collect::<Vec<_>>()).max();
    let sources = sdk.join("sources");
    let platform = wanted.map(|n| sources.join(format!("android-{n}"))).filter(|p| p.is_dir()).or_else(|| newest_subdir(&sources, "android-"));
    if let Some(platform) = platform {
        // Named after the compileSdk platform, as IntelliJ does, even when
        // only another level's sources are installed.
        let api = wanted.map(|n| n.to_string()).unwrap_or_else(|| platform.file_name().unwrap().to_string_lossy().trim_start_matches("android-").to_owned());
        out.push(Library {
            exclude: vec!["com/android/internal", "com/android/server", "com/android/systemui"],
            root_label: Some("android.jar".into()),
            location: Some(sdk.display().to_string()),
            ..lib(android_platform_name(&sdk, &api), "Android SDK", platform)
        });
    }
    Some(sdk)
}

/// "< Android API 34, extension level 7 Platform >", the SDK table name
/// Android Studio gives a platform.
fn android_platform_name(sdk: &Path, api: &str) -> String {
    let dir = sdk.join("platforms").join(format!("android-{api}"));
    static LEVEL: OnceLock<Regex> = OnceLock::new();
    let re = regex(&LEVEL, r"<extension-level>\s*(\d+)\s*</extension-level>");
    let level = re
        .captures(&read(&dir.join("package.xml")))
        .map(|c| c[1].to_owned())
        .or_else(|| read(&dir.join("source.properties")).lines().find_map(|l| l.strip_prefix("Platform.ExtensionLevel=").map(|v| v.trim().to_owned())));
    match level {
        Some(level) => format!("< Android API {api}, extension level {level} Platform >"),
        None => format!("< Android API {api} Platform >"),
    }
}

fn gradle(ctx: &Project, out: &mut Vec<Library>) {
    let texts = gradle_texts(ctx);
    if texts.is_empty() {
        return;
    }
    let Some(gradle_home) = std::env::var_os("GRADLE_USER_HOME").map(PathBuf::from).or_else(|| home().map(|h| h.join(".gradle"))) else { return };
    let files = gradle_home.join("caches/modules-2/files-2.1");
    if !files.is_dir() {
        return;
    }
    let mut queue: VecDeque<(String, String, Option<String>)> = declared_coordinates(&texts).into();
    if ctx.has(&[Lang::Kotlin]) {
        queue.push_back(("org.jetbrains.kotlin".into(), "kotlin-stdlib".into(), kotlin_version(&texts)));
    }
    let mut seen: HashSet<(String, String)> = HashSet::new();
    while let Some((group, artifact, version)) = queue.pop_front() {
        if seen.len() >= MAX_ARTIFACTS || !seen.insert((group.clone(), artifact.clone())) {
            continue;
        }
        let artifact_dir = files.join(&group).join(&artifact);
        let version_dir = version
            .as_ref()
            .map(|v| artifact_dir.join(v))
            .filter(|d| d.is_dir())
            .or_else(|| newest_subdir(&artifact_dir, ""));
        let Some(version_dir) = version_dir else { continue };
        let version = version_dir.file_name().unwrap().to_string_lossy().into_owned();
        let artifacts: Vec<PathBuf> = std::fs::read_dir(&version_dir)
            .into_iter()
            .flatten()
            .flatten()
            .flat_map(|hash| std::fs::read_dir(hash.path()).into_iter().flatten().flatten().map(|f| f.path()))
            .collect();
        let named = |suffix: &str| artifacts.iter().find(|p| p.to_string_lossy().ends_with(suffix)).cloned();
        if let Some(jar) = named("-sources.jar") {
            if let Some(root) = extract_sources(&jar, &format!("gradle/{group}/{artifact}-{version}"), &|name| {
                (name.ends_with(".java") || name.ends_with(".kt")).then(|| name.to_owned())
            }) {
                // Android libraries ship as .aar; IntelliJ names them "…@aar"
                // and shows their classes.jar as the root.
                let aar = named(".aar").is_some() || named(".pom").is_some_and(|pom| read(&pom).contains("<packaging>aar</packaging>"));
                let (suffix, jar) = if aar { ("@aar", "classes.jar".to_owned()) } else { ("", format!("{artifact}-{version}.jar")) };
                out.push(Library { root_label: Some(jar), ..lib(format!("Gradle: {group}:{artifact}:{version}{suffix}"), "Gradle", root) });
            }
        }
        // Follow the dependencies: Gradle module metadata, else the POM.
        let deps = match named(".module") {
            Some(module) => module_dependencies(&read(&module)),
            None => named(".pom").map(|pom| pom_dependencies(&read(&pom))).unwrap_or_default(),
        };
        queue.extend(deps);
    }
}

/// `group:artifact:version` strings in build files, and version-catalog libraries.
fn declared_coordinates(texts: &[(PathBuf, String)]) -> Vec<(String, String, Option<String>)> {
    static COORD: OnceLock<Regex> = OnceLock::new();
    let coord = regex(&COORD, r#"["']([\w.\-]+):([\w.\-]+)(?::([\w.\-+\[\],]+))?["']"#);
    let mut out = Vec::new();
    let mut versions: HashMap<String, String> = HashMap::new();
    for (path, text) in texts {
        if !path.to_string_lossy().ends_with(".toml") {
            continue;
        }
        let mut section = "";
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                section = if line.starts_with("[versions]") { "versions" } else if line.starts_with("[libraries]") { "libraries" } else { "" };
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            let (key, value) = (key.trim(), value.trim());
            if section == "versions" {
                let v = if value.starts_with('{') { toml_field(value, "strictly").or_else(|| toml_field(value, "require")) } else { Some(value.trim_matches('"').to_owned()) };
                if let Some(v) = v {
                    versions.insert(key.to_owned(), v);
                }
            }
        }
        for line in text.lines().map(str::trim) {
            let Some((_, value)) = line.split_once('=') else { continue };
            let value = value.trim();
            if !value.starts_with('{') {
                continue;
            }
            let module = toml_field(value, "module").and_then(|m| m.split_once(':').map(|(g, a)| (g.to_owned(), a.to_owned())));
            let module = module.or_else(|| Some((toml_field(value, "group")?, toml_field(value, "name")?)));
            let Some((group, artifact)) = module else { continue };
            let version = toml_field(value, "version").or_else(|| versions.get(&toml_field(value, "version.ref")?).cloned());
            out.push((group, artifact, version));
        }
    }
    for (path, text) in texts {
        for c in coord.captures_iter(text) {
            if path.to_string_lossy().ends_with(".toml") && c.get(3).is_none() {
                continue;
            }
            let version = c.get(3).map(|v| v.as_str().to_owned()).filter(|v| v.chars().next().is_some_and(|c| c.is_ascii_digit()));
            out.push((c[1].to_owned(), c[2].to_owned(), version));
        }
    }
    out.retain(|(g, a, _)| g.contains('.') && !a.is_empty());
    out
}

fn toml_field(inline: &str, key: &str) -> Option<String> {
    let start = inline.find(&format!("{key} ")).or_else(|| inline.find(&format!("{key}=")))?;
    // `version` must not match `version.ref`.
    let rest = &inline[start + key.len()..];
    let rest = rest.trim_start().strip_prefix('=')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    Some(rest[..rest.find('"')?].to_owned())
}

fn kotlin_version(texts: &[(PathBuf, String)]) -> Option<String> {
    static KOTLIN: OnceLock<Regex> = OnceLock::new();
    let re = regex(&KOTLIN, r#"(?:kotlin(?:-gradle-plugin:|\("[\w.]+"\)\s*version\s*"|\.(?:android|jvm|multiplatform)"\s*\)?\s*version\s*"|\s*=\s*")|kotlin_version\s*=\s*['"])(\d+\.\d+\.\d+)"#);
    texts.iter().find_map(|(_, t)| re.captures(t).map(|c| c[1].to_owned()))
}

fn pom_dependencies(pom: &str) -> Vec<(String, String, Option<String>)> {
    static DEP: OnceLock<Regex> = OnceLock::new();
    let dep = regex(&DEP, r"(?s)<dependency>(.*?)</dependency>");
    let tag = |block: &str, name: &str| {
        let open = format!("<{name}>");
        let start = block.find(&open)? + open.len();
        let end = block[start..].find("</")? + start;
        Some(block[start..end].trim().to_owned())
    };
    // Dependency management only pins versions; it adds nothing.
    let body = match (pom.find("<dependencyManagement>"), pom.find("</dependencyManagement>")) {
        (Some(a), Some(b)) => format!("{}{}", &pom[..a], &pom[b..]),
        _ => pom.to_owned(),
    };
    dep.captures_iter(&body)
        .filter_map(|c| {
            let block = &c[1];
            let scope = tag(block, "scope").unwrap_or_default();
            if matches!(scope.as_str(), "test" | "provided" | "system" | "import") || tag(block, "optional").as_deref() == Some("true") {
                return None;
            }
            let version = tag(block, "version").filter(|v| !v.contains("${") && !v.starts_with('['));
            Some((tag(block, "groupId")?, tag(block, "artifactId")?, version))
        })
        .collect()
}

/// The JVM / Android variants' dependencies (and where a multiplatform
/// library's JVM part lives).
fn module_dependencies(module: &str) -> Vec<(String, String, Option<String>)> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(module) else { return Vec::new() };
    let mut out = Vec::new();
    for variant in json["variants"].as_array().into_iter().flatten() {
        let attrs = &variant["attributes"];
        let platform = attrs["org.jetbrains.kotlin.platform.type"].as_str();
        let usage = attrs["org.gradle.usage"].as_str().unwrap_or("");
        if !matches!(platform, None | Some("jvm") | Some("androidJvm") | Some("common")) || !(usage.contains("api") || usage.contains("runtime")) {
            continue;
        }
        if let Some(at) = variant.get("available-at") {
            if let (Some(g), Some(m)) = (at["group"].as_str(), at["module"].as_str()) {
                out.push((g.to_owned(), m.to_owned(), at["version"].as_str().map(str::to_owned)));
            }
        }
        for dep in variant["dependencies"].as_array().into_iter().flatten() {
            if let (Some(g), Some(m)) = (dep["group"].as_str(), dep["module"].as_str()) {
                let version = dep["version"]["requires"].as_str().or_else(|| dep["version"]["strictly"].as_str()).map(str::to_owned);
                out.push((g.to_owned(), m.to_owned(), version));
            }
        }
    }
    out
}

/// Unpacks the source files of a jar / zip once, into the cache.
/// `map` picks entries and names their path inside the library.
fn extract_sources(archive: &Path, name: &str, map: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let dest = cache_dir()?.join("sources").join(name);
    let marker = dest.join(".junction-complete");
    let stamp = store::stat(archive).map(|(size, mtime)| format!("{size}:{mtime}"))?;
    if read(&marker) == stamp {
        return Some(dest);
    }
    std::fs::remove_dir_all(&dest).ok();
    let file = std::fs::File::open(archive).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;
    for i in 0..zip.len() {
        let Ok(mut entry) = zip.by_index(i) else { continue };
        if entry.is_dir() || entry.enclosed_name().is_none() {
            continue;
        }
        let Some(target) = map(entry.name()) else { continue };
        let path = dest.join(&target);
        if !path.starts_with(&dest) {
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        if let Ok(mut file) = std::fs::File::create(&path) {
            std::io::copy(&mut entry, &mut file).ok();
        }
    }
    std::fs::create_dir_all(&dest).ok();
    std::fs::write(&marker, stamp).ok();
    Some(dest)
}

fn java_homes() -> Vec<PathBuf> {
    let mut homes: Vec<PathBuf> = Vec::new();
    if let Some(h) = std::env::var_os("JAVA_HOME") {
        homes.push(PathBuf::from(h));
    }
    // Android Studio's bundled runtime.
    homes.extend(
        [
            "/opt/android-studio/jbr",
            "/usr/local/android-studio/jbr",
            "/Applications/Android Studio.app/Contents/jbr/Contents/Home",
            "C:\\Program Files\\Android\\Android Studio\\jbr",
        ]
        .iter()
        .map(PathBuf::from),
    );
    if let Some(home) = home() {
        homes.push(home.join("android-studio/jbr"));
    }
    if let Some(java) = super::lsp::find_program("java").and_then(|j| std::fs::canonicalize(j).ok()) {
        if let Some(h) = java.parent().and_then(Path::parent) {
            homes.push(h.to_path_buf());
        }
    }
    if cfg!(target_os = "macos") {
        static JAVA_HOME: OnceLock<Option<String>> = OnceLock::new();
        if let Some(h) = cached_run(&JAVA_HOME, "/usr/libexec/java_home", &[]) {
            homes.push(PathBuf::from(h));
        }
    }
    homes
}

fn jdk(ctx: &Project, out: &mut Vec<Library>) {
    if !ctx.has(&[Lang::Java, Lang::Kotlin]) {
        return;
    }
    for home in java_homes() {
        let Some(src) = [home.join("lib/src.zip"), home.join("src.zip")].into_iter().find(|p| p.is_file()) else { continue };
        let version = read(&home.join("release"))
            .lines()
            .find_map(|l| l.strip_prefix("JAVA_VERSION=").map(|v| v.trim_matches('"').to_owned()))
            .unwrap_or_else(|| "?".into());
        const MODULES: &[&str] = &["java.base", "java.logging", "java.sql", "java.net.http", "java.prefs", "java.compiler"];
        let map = |name: &str| -> Option<String> {
            if !name.ends_with(".java") {
                return None;
            }
            // JDK 9+: module/package/File.java; JDK 8: package/File.java.
            let path = match name.split_once('/') {
                Some((module, rest)) if module.contains('.') && !module.starts_with("java/") => MODULES.contains(&module).then_some(rest)?,
                _ => name,
            };
            (path.starts_with("java/") || path.starts_with("javax/")).then(|| path.to_owned())
        };
        let major = version.split(['.', '_']).next().unwrap_or("?").to_owned();
        if let Some(root) = extract_sources(&src, &format!("jdk/{}-{:08x}", version, stable_hash(&src.to_string_lossy()) as u32), &map) {
            // IntelliJ names a detected JDK "jbr-21", "corretto-17"…: its folder and version.
            let folder = home.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let name = if folder.is_empty() || folder.chars().any(|c| c.is_ascii_digit()) { folder } else { format!("{folder}-{major}") };
            let name = if name.is_empty() { format!("< JDK {major} >") } else { format!("< {name} >") };
            out.push(Library { location: Some(home.display().to_string()), ..lib(name, "JDK", root) });
        }
        return;
    }
}

// --- C / C++ / Objective-C ----------------------------------------------

fn c_family(ctx: &Project, android_sdk: Option<&Path>, out: &mut Vec<Library>) {
    const C_LANGS: &[Lang] = &[Lang::C, Lang::Cpp, Lang::ObjC];
    if !ctx.has(C_LANGS) {
        return;
    }
    let mut dirs: Vec<PathBuf> = Vec::new();
    // The NDK's sysroot for Android native code.
    if let Some(sdk) = android_sdk {
        if let Some(ndk) = newest_subdir(&sdk.join("ndk"), "").or_else(|| dir(sdk.join("ndk-bundle"))) {
            let prebuilt = ndk.join("toolchains/llvm/prebuilt");
            if let Some(host) = std::fs::read_dir(&prebuilt).ok().and_then(|mut d| d.next()).and_then(|e| e.ok()) {
                let include = host.path().join("sysroot/usr/include");
                dirs.push(include.join("c++/v1"));
                dirs.push(include.clone());
            }
        }
    }
    dirs.extend(compile_commands_includes(ctx.root));
    dirs.extend(system_includes());
    if let Some(sdk) = apple_sdk(false) {
        dirs.push(sdk.join("usr/include"));
    }
    let mut seen_dirs = HashSet::new();
    dirs.retain(|d| d.is_dir() && !d.starts_with(ctx.root) && seen_dirs.insert(d.clone()));
    let frameworks = apple_sdk(false).map(|s| s.join("System/Library/Frameworks"));
    if dirs.is_empty() && frameworks.is_none() {
        return;
    }

    // Follow #include from the project's sources into those directories.
    let mut found: BTreeMap<PathBuf, (usize, Vec<PathBuf>)> = BTreeMap::new();
    let mut queue: VecDeque<PathBuf> = ctx.sources(C_LANGS).collect();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut headers = 0;
    while let Some(file) = queue.pop_front() {
        let from_project = file.starts_with(ctx.root);
        for include in includes(&read(&file)) {
            // Quoted includes next to the file are the project's own.
            if from_project && file.parent().is_some_and(|d| d.join(&include).is_file()) {
                continue;
            }
            let hit = dirs.iter().enumerate().find_map(|(i, d)| {
                let p = d.join(&include);
                p.is_file().then(|| (d.clone(), p, i))
            });
            let hit = hit.or_else(|| {
                let (framework, header) = include.split_once('/')?;
                let root = frameworks.as_ref()?.join(format!("{framework}.framework/Headers"));
                let p = root.join(header);
                p.is_file().then(|| (root, p, usize::MAX))
            });
            let Some((root, path, rank)) = hit else { continue };
            if !seen.insert(path.clone()) {
                continue;
            }
            found.entry(root).or_insert_with(|| (rank, Vec::new())).1.push(path.clone());
            headers += 1;
            if headers >= MAX_HEADERS {
                queue.clear();
                break;
            }
            queue.push_back(path);
        }
    }
    let mut libs: Vec<(usize, Library)> = found
        .into_iter()
        .map(|(root, (rank, files))| {
            let name = match root.file_name().map(|n| n.to_string_lossy().into_owned()) {
                Some(n) if root.ends_with("Headers") => root.parent().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned()).unwrap_or(n),
                _ => root.to_string_lossy().into_owned(),
            };
            (rank, Library { only: Some(files), ..lib(name, "C/C++", root) })
        })
        .collect();
    libs.sort_by_key(|(rank, _)| *rank);
    out.extend(libs.into_iter().map(|(_, l)| l));
}

fn includes(text: &str) -> Vec<String> {
    static INCLUDE: OnceLock<Regex> = OnceLock::new();
    let re = regex(&INCLUDE, r#"(?m)^\s*#\s*(?:include|import|include_next)\s*[<"]([^>"]+)[>"]|^\s*@import\s+([\w.]+)\s*;"#);
    re.captures_iter(text)
        .filter_map(|c| match (c.get(1), c.get(2)) {
            (Some(path), _) => Some(path.as_str().to_owned()),
            // @import Foundation; → Foundation/Foundation.h
            (None, Some(module)) => {
                let m = module.as_str().split('.').next().unwrap_or("");
                Some(format!("{m}/{m}.h"))
            }
            _ => None,
        })
        .collect()
}

fn compile_commands_includes(root: &Path) -> Vec<PathBuf> {
    let mut candidates = vec![root.join("compile_commands.json"), root.join("build/compile_commands.json")];
    for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("cmake-build") || name == "out" {
            candidates.push(entry.path().join("compile_commands.json"));
        }
    }
    let mut out = Vec::new();
    for file in candidates {
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&read(&file)) else { continue };
        for unit in json.as_array().into_iter().flatten().take(2000) {
            let directory = PathBuf::from(unit["directory"].as_str().unwrap_or(""));
            let args: Vec<String> = match unit["arguments"].as_array() {
                Some(a) => a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect(),
                None => unit["command"].as_str().unwrap_or("").split_whitespace().map(str::to_owned).collect(),
            };
            let mut it = args.iter();
            while let Some(arg) = it.next() {
                let value = ["-I", "-isystem", "-iquote", "/I"].iter().find_map(|flag| {
                    let rest = arg.strip_prefix(flag)?;
                    if rest.is_empty() { it.clone().next().cloned() } else { Some(rest.to_owned()) }
                });
                if let Some(value) = value {
                    let p = Path::new(&value);
                    out.push(if p.is_absolute() { p.to_path_buf() } else { normalize(&directory.join(p)) });
                }
            }
        }
    }
    out
}

/// The compiler's own include search path.
fn system_includes() -> Vec<PathBuf> {
    if cfg!(windows) {
        return std::env::var_os("INCLUDE").map(|v| std::env::split_paths(&v).collect()).unwrap_or_default();
    }
    static SEARCH: OnceLock<Vec<PathBuf>> = OnceLock::new();
    SEARCH
        .get_or_init(|| {
            for compiler in ["c++", "clang++", "g++", "cc", "clang"] {
                let Some(text) = run(compiler, &["-E", "-x", "c++", "-", "-v"]) else { continue };
                let mut dirs = Vec::new();
                let mut inside = false;
                for line in text.lines() {
                    if line.starts_with("#include <...> search starts here") || line.starts_with("#include \"...\" search starts here") {
                        inside = true;
                    } else if line.starts_with("End of search list") {
                        break;
                    } else if inside {
                        let line = line.trim().trim_end_matches(" (framework directory)");
                        if !line.is_empty() {
                            dirs.push(normalize(Path::new(line)));
                        }
                    }
                }
                if !dirs.is_empty() {
                    return dirs;
                }
            }
            Vec::new()
        })
        .clone()
}

fn apple_sdk(ios: bool) -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    static MAC: OnceLock<Option<String>> = OnceLock::new();
    static IOS: OnceLock<Option<String>> = OnceLock::new();
    let path = if ios {
        cached_run(&IOS, "xcrun", &["--sdk", "iphoneos", "--show-sdk-path"])
    } else {
        cached_run(&MAC, "xcrun", &["--show-sdk-path"])
    }?;
    dir(PathBuf::from(path.lines().next()?))
}

// --- Swift ----------------------------------------------------------------

fn swift(ctx: &Project, out: &mut Vec<Library>) {
    for manifest in ctx.named("Package.swift") {
        let checkouts = manifest.parent().unwrap().join(".build/checkouts");
        for entry in std::fs::read_dir(&checkouts).into_iter().flatten().flatten() {
            out.push(lib(entry.file_name().to_string_lossy(), "Swift Packages", entry.path()));
        }
    }
    // Xcode keeps a project's packages in DerivedData.
    let xcode: HashSet<String> = ctx.files.iter().filter_map(|f| f.split('/').find_map(|p| p.strip_suffix(".xcodeproj").or(p.strip_suffix(".xcworkspace")))).map(str::to_owned).collect();
    if !xcode.is_empty() {
        if let Some(derived) = home().map(|h| h.join("Library/Developer/Xcode/DerivedData")) {
            for entry in std::fs::read_dir(&derived).into_iter().flatten().flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if xcode.iter().any(|x| name.starts_with(&format!("{x}-"))) {
                    for checkout in std::fs::read_dir(entry.path().join("SourcePackages/checkouts")).into_iter().flatten().flatten() {
                        out.push(lib(checkout.file_name().to_string_lossy(), "Swift Packages", checkout.path()));
                    }
                }
            }
        }
    }
    for podfile in ctx.named("Podfile") {
        let pods = podfile.parent().unwrap().join("Pods");
        for entry in std::fs::read_dir(&pods).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.path().is_dir() && !matches!(name.as_str(), "Target Support Files" | "Headers" | "Local Podspecs" | "Pods.xcodeproj") {
                out.push(lib(name, "CocoaPods", entry.path()));
            }
        }
    }
    for cartfile in ctx.named("Cartfile") {
        let checkouts = cartfile.parent().unwrap().join("Carthage/Checkouts");
        for entry in std::fs::read_dir(&checkouts).into_iter().flatten().flatten() {
            out.push(lib(entry.file_name().to_string_lossy(), "Carthage", entry.path()));
        }
    }
    // The SDK's Swift module interfaces for what the sources import.
    if !ctx.has(&[Lang::Swift]) {
        return;
    }
    static IMPORT: OnceLock<Regex> = OnceLock::new();
    let re = regex(&IMPORT, r"(?m)^\s*(?:@\w+\s+)*import\s+(?:class\s+|struct\s+|enum\s+|func\s+|protocol\s+)?(\w+)");
    let mut modules: BTreeSet<String> = BTreeSet::from(["Swift".to_owned()]);
    for file in ctx.sources(&[Lang::Swift]) {
        modules.extend(re.captures_iter(&read(&file)).map(|c| c[1].to_owned()));
    }
    let ios = modules.contains("UIKit") || modules.contains("SwiftUI");
    let Some(sdk) = apple_sdk(ios).or_else(|| apple_sdk(false)) else { return };
    for module in modules {
        let framework = sdk.join(format!("System/Library/Frameworks/{module}.framework"));
        let candidates = [framework.join(format!("Modules/{module}.swiftmodule")), sdk.join(format!("usr/lib/swift/{module}.swiftmodule"))];
        let mut files: Vec<PathBuf> = Vec::new();
        for dir in candidates {
            let mut interfaces: Vec<PathBuf> = std::fs::read_dir(&dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "swiftinterface") && !p.to_string_lossy().contains("private"))
                .collect();
            interfaces.sort_by_key(|p| !p.to_string_lossy().contains("arm64"));
            files.extend(interfaces.into_iter().take(1));
        }
        let headers = framework.join("Headers");
        files.extend(std::fs::read_dir(&headers).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "h")));
        if !files.is_empty() {
            let root = if framework.is_dir() { framework } else { sdk.join("usr/lib/swift") };
            out.push(Library { only: Some(files), ..lib(module, "SDK", root) });
        }
    }
}

// --- JavaScript / TypeScript ------------------------------------------------

fn npm(ctx: &Project, out: &mut Vec<Library>) {
    for manifest in ctx.named("package.json") {
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&read(&manifest)) else { continue };
        let modules = manifest.parent().unwrap().join("node_modules");
        if !modules.is_dir() {
            continue;
        }
        let mut names: BTreeSet<String> = BTreeSet::new();
        for key in ["dependencies", "devDependencies", "peerDependencies"] {
            names.extend(json[key].as_object().into_iter().flat_map(|o| o.keys().cloned()));
        }
        for name in names.clone() {
            let types = match name.strip_prefix('@') {
                Some(scoped) => format!("@types/{}", scoped.replace('/', "__")),
                None => format!("@types/{name}"),
            };
            names.insert(types);
        }
        for name in names {
            let root = modules.join(&name);
            if !root.is_dir() {
                continue;
            }
            let version = serde_json::from_str::<serde_json::Value>(&read(&root.join("package.json")))
                .ok()
                .and_then(|p| p["version"].as_str().map(str::to_owned))
                .unwrap_or_default();
            // Declarations when the package ships them, else its sources.
            let mut all = Vec::new();
            collect(&root, &mut all, 2000, &|name| is_library_source(name));
            let dts: Vec<PathBuf> = all.iter().filter(|p| p.to_string_lossy().ends_with(".d.ts")).cloned().collect();
            let files = if dts.is_empty() { all } else { dts };
            if !files.is_empty() {
                out.push(Library { only: Some(files), ..lib(format!("{name} {version}").trim().to_owned(), "npm", root) });
            }
        }
    }
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>, cap: usize, keep: &dyn Fn(&str) -> bool) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if out.len() >= cap {
            return;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        match entry.file_type() {
            Ok(t) if t.is_dir() && !SKIP_DIRS.contains(&name.as_str()) => collect(&path, out, cap, keep),
            Ok(t) if t.is_file() && keep(&name) => out.push(path),
            _ => {}
        }
    }
}

// --- Python -------------------------------------------------------------------

fn python(ctx: &Project, out: &mut Vec<Library>) {
    if !ctx.has(&[Lang::Python]) {
        return;
    }
    static IMPORT: OnceLock<Regex> = OnceLock::new();
    let re = regex(&IMPORT, r"(?m)^\s*(?:from|import)\s+([A-Za-z_]\w*)");
    let local: HashSet<String> = ctx.files.iter().filter_map(|f| f.split('/').next().map(|p| p.trim_end_matches(".py").to_owned())).collect();
    let mut imports: BTreeSet<String> = BTreeSet::new();
    for file in ctx.sources(&[Lang::Python]) {
        imports.extend(re.captures_iter(&read(&file)).map(|c| c[1].to_owned()).filter(|m| !local.contains(m)));
    }
    let mut sites: Vec<PathBuf> = Vec::new();
    for venv in [".venv", "venv", "env"] {
        let lib_dir = ctx.root.join(venv).join("lib");
        for entry in std::fs::read_dir(&lib_dir).into_iter().flatten().flatten() {
            sites.push(entry.path().join("site-packages"));
        }
        sites.push(ctx.root.join(venv).join("Lib/site-packages"));
    }
    static STDLIB: OnceLock<Option<String>> = OnceLock::new();
    let stdlib = cached_run(&STDLIB, "python3", &["-c", "import sysconfig;print(sysconfig.get_paths()['stdlib'])"])
        .or_else(|| run("python", &["-c", "import sysconfig;print(sysconfig.get_paths()['stdlib'])"]))
        .map(|s| PathBuf::from(s.lines().next().unwrap_or("").trim()));
    let mut stdlib_files = Vec::new();
    for module in &imports {
        if let Some(site) = sites.iter().find(|s| s.join(module).is_dir() || s.join(format!("{module}.py")).is_file()) {
            match dir(site.join(module)) {
                Some(root) => out.push(lib(module.clone(), "Python", root)),
                None => out.push(Library { only: Some(vec![site.join(format!("{module}.py"))]), ..lib(module.clone(), "Python", site.clone()) }),
            }
        } else if let Some(stdlib) = &stdlib {
            if let Some(root) = dir(stdlib.join(module)) {
                collect(&root, &mut stdlib_files, 1500, &|n| n.ends_with(".py"));
            } else if stdlib.join(format!("{module}.py")).is_file() {
                stdlib_files.push(stdlib.join(format!("{module}.py")));
            }
        }
    }
    if let (Some(stdlib), false) = (stdlib, stdlib_files.is_empty()) {
        out.push(Library { only: Some(stdlib_files), ..lib("Python stdlib", "SDK", stdlib) });
    }
}

// --- V ------------------------------------------------------------------------

fn vlang(ctx: &Project, out: &mut Vec<Library>) {
    if !ctx.has(&[Lang::V]) {
        return;
    }
    static IMPORT: OnceLock<Regex> = OnceLock::new();
    let re = regex(&IMPORT, r"(?m)^\s*import\s+([\w.]+)");
    let mut modules: BTreeSet<String> = BTreeSet::from(["builtin".to_owned()]);
    for file in ctx.sources(&[Lang::V]) {
        modules.extend(re.captures_iter(&read(&file)).map(|c| c[1].to_owned()));
    }
    let vlib = super::lsp::find_program("v").and_then(|v| std::fs::canonicalize(v).ok()).and_then(|v| dir(v.parent()?.join("vlib")));
    let vmodules = std::env::var_os("VMODULES").map(PathBuf::from).or_else(|| home().map(|h| h.join(".vmodules")));
    for module in modules {
        let rel = module.replace('.', "/");
        let root = vlib.as_ref().and_then(|v| dir(v.join(&rel))).or_else(|| vmodules.as_ref().and_then(|v| dir(v.join(&rel))));
        if let Some(root) = root {
            out.push(lib(module, "V Modules", root));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_manifests() {
        let lock = "[[package]]\nname = \"serde\"\nversion = \"1.0.1\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n\n[[package]]\nname = \"me\"\nversion = \"0.1.0\"\n";
        assert_eq!(parse_cargo_lock(lock), vec![("serde".into(), "1.0.1".into(), "registry+https://github.com/rust-lang/crates.io-index".into())]);
        let go_mod = "module x\n\nrequire github.com/A/b v1.2.0\nrequire (\n\tgolang.org/x/sys v0.1.0 // indirect\n)\n";
        assert_eq!(parse_go_mod(go_mod), vec![("github.com/A/b".into(), "v1.2.0".into()), ("golang.org/x/sys".into(), "v0.1.0".into())]);
        assert_eq!(go_escape("github.com/A/b"), "github.com/!a/b");
        assert!(version_key("1.10.0") > version_key("1.9.2"));
        assert!(version_key("2.0.0") > version_key("2.0.0-alpha"));
    }

    #[test]
    fn reads_gradle_coordinates() {
        let toml = "[versions]\nkotlin = \"2.0.0\"\nktor = \"2.3.1\"\n[libraries]\nktor-core = { module = \"io.ktor:ktor-client-core\", version.ref = \"ktor\" }\nokio = { group = \"com.squareup.okio\", name = \"okio\", version = \"3.9.0\" }\n";
        let gradle = "dependencies {\n implementation(\"androidx.core:core-ktx:1.13.1\")\n implementation 'com.google.code.gson:gson:2.11.0'\n}\nplugins { id(\"org.jetbrains.kotlin.android\") version \"1.9.24\" }\n";
        let texts = vec![(PathBuf::from("gradle/libs.versions.toml"), toml.to_owned()), (PathBuf::from("app/build.gradle.kts"), gradle.to_owned())];
        let coords = declared_coordinates(&texts);
        assert!(coords.contains(&("io.ktor".into(), "ktor-client-core".into(), Some("2.3.1".into()))));
        assert!(coords.contains(&("com.squareup.okio".into(), "okio".into(), Some("3.9.0".into()))));
        assert!(coords.contains(&("androidx.core".into(), "core-ktx".into(), Some("1.13.1".into()))));
        assert!(coords.contains(&("com.google.code.gson".into(), "gson".into(), Some("2.11.0".into()))));
        let pom = "<project><dependencies><dependency><groupId>a.b</groupId><artifactId>c</artifactId><version>1.0</version></dependency><dependency><groupId>t.t</groupId><artifactId>junit</artifactId><scope>test</scope></dependency></dependencies></project>";
        assert_eq!(pom_dependencies(pom), vec![("a.b".into(), "c".into(), Some("1.0".into()))]);
    }

    #[test]
    fn finds_includes() {
        let text = "#include <stdio.h>\n#  include \"local.h\"\n#import <Foundation/Foundation.h>\n@import UIKit;\n";
        assert_eq!(includes(text), vec!["stdio.h", "local.h", "Foundation/Foundation.h", "UIKit/UIKit.h"]);
        assert_eq!(library_lang("/usr/include/c++/13/vector"), Some(Lang::Cpp));
        assert_eq!(library_lang("/x/Foundation.swiftinterface"), Some(Lang::Swift));
        let header = "extern size_t strlen (const char *__s)\n     __THROW __attribute_pure__ __nonnull ((1));\n";
        let blanked = blank_macros(header);
        assert_eq!(blanked.len(), header.len());
        assert!(blanked.contains("strlen (const char *__s)") && !blanked.contains("__THROW") && !blanked.contains("nonnull"));
    }
}

#[cfg(test)]
mod probe {
    /// `JUNCTION_PROBE=/path cargo test probe_project -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn probe_project() {
        let root = std::path::PathBuf::from(std::env::var("JUNCTION_PROBE").unwrap());
        let files = super::store::list_files(&root);
        let t = std::time::Instant::now();
        let libs = super::discover(&root, &files);
        println!("discover: {} libraries in {:?}", libs.len(), t.elapsed());
        for l in libs.iter().take(400) {
            println!("  [{}] {} {}", l.group, l.name, l.root.display());
        }
        let t = std::time::Instant::now();
        let ext = super::ExternalIndex::build(libs, 0, None, &|_, _| {});
        println!("build: {} files, {} symbols in {:?}", ext.file_count(), ext.symbol_count(), t.elapsed());
        let mut per: Vec<(usize, String)> = ext.libraries.iter().map(|l| (l.files.iter().map(|f| ext.files[f].symbols.len()).sum(), l.name.clone())).collect();
        per.sort();
        for (n, name) in per.iter().rev().take(25) {
            println!("  {n:>9} {name}");
        }
        let status = std::fs::read_to_string("/proc/self/status").unwrap();
        println!("{}", status.lines().find(|l| l.starts_with("VmRSS")).unwrap());
    }
}

#[cfg(test)]
mod probe_nav {
    /// `JUNCTION_PROBE=/path JUNCTION_PROBE_AT=file:needle,... cargo test probe_jumps -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn probe_jumps() {
        let root = std::path::PathBuf::from(std::env::var("JUNCTION_PROBE").unwrap());
        let mut index = super::store::ProjectIndex::load(&root);
        index.update(&|_, _| {});
        let libs = super::discover(&root, &index.all_files);
        index.external = std::sync::Arc::new(super::ExternalIndex::build(libs, 0, None, &|_, _| {}));
        for spec in std::env::var("JUNCTION_PROBE_AT").unwrap().split(',') {
            let (file, needle) = spec.split_once(':').unwrap();
            let text = std::fs::read_to_string(root.join(file)).unwrap();
            let offset = text.find(needle).unwrap() + 1;
            let lang = super::super::service::CodeIndex::lang_of(file, &text).unwrap();
            let t = std::time::Instant::now();
            let targets = super::super::nav::definitions(&index, file, lang, &text, offset);
            println!("{needle} ({:?}): {} targets", t.elapsed(), targets.len());
            for t in targets.iter().take(4) {
                println!("    {}:{} {} {:?}", t.path, t.line + 1, t.label, t.container);
            }
        }
    }
}
