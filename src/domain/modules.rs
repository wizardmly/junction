//! The project's Gradle modules and source sets, as Android Studio's
//! project model sees them for resolving names: a file sees its own
//! module and the modules it depends on (`project(":core")`,
//! `projects.core`), and within a module `main` doesn't see `test` or
//! `androidTest`, while tests see `main` (and `testFixtures`). Other
//! modules' tests are never visible.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;

/// A folder with a Gradle build file and the modules it depends on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Module {
    /// Project-relative, `/`-separated; "" for the root.
    dir: String,
    /// Indices into the graph's modules.
    deps: Vec<usize>,
    /// An Android module's build types and product flavors (with their
    /// dimension): the names its variant source sets are made of.
    build_types: Vec<String>,
    flavors: Vec<(String, String)>,
    /// The variant picked in Build Variants; else the default one.
    picked: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ModuleGraph {
    modules: Vec<Module>,
}

/// The group of a build type among a variant's parts; a flavor's is its
/// dimension.
const BUILD_TYPE: &str = "#build-type";

/// What a source set holds, for which other sets see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SetKind {
    Production,
    Test,
    AndroidTest,
    Fixtures,
}

fn set_kind(set: &str) -> SetKind {
    if set == "testFixtures" || set.starts_with("testFixtures") {
        SetKind::Fixtures
    } else if set.starts_with("androidTest") || set.starts_with("androidInstrumentedTest") {
        SetKind::AndroidTest
    } else if set.starts_with("test") || set.ends_with("Test") {
        SetKind::Test
    } else {
        SetKind::Production
    }
}

/// The modules a build file names: `project(":core:data")` → `core/data`,
/// and the type-safe accessors `projects.core.data` (as written, matched
/// against folder names later).
pub fn module_dependencies(text: &str) -> Vec<String> {
    static PROJECT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"project\(\s*(?:path\s*[:=]\s*)?["'](:[\w:.-]+)["']"#).unwrap());
    PROJECT.captures_iter(text).map(|c| c[1].trim_start_matches(':').replace(':', "/")).collect()
}

fn accessor_dependencies(text: &str) -> Vec<String> {
    static ACCESSOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bprojects((?:\.\w+)+)").unwrap());
    ACCESSOR.captures_iter(text).map(|c| c[1].trim_start_matches('.').replace('.', "/")).collect()
}

/// An Android module's build types (debug and release, and the ones its
/// build file adds) and product flavors with their dimension; None when
/// the build file isn't an Android module's.
fn variant_parts_of(script: &str, kotlin: bool) -> Option<(Vec<String>, Vec<(String, String)>)> {
    let (types, flavors) = super::project_structure::variants(script, &[], kotlin);
    if types.is_empty() && flavors.is_empty() && !script.contains("com.android.") && !script.contains("android {") {
        return None;
    }
    let mut names = vec!["debug".to_owned(), "release".to_owned()];
    names.extend(types.into_iter().map(|t| t.name).filter(|n| n != "debug" && n != "release"));
    let flavors = flavors
        .into_iter()
        .map(|f| {
            let dimension = f.properties.iter().find(|p| p.label == "Dimension").map(|p| p.value.trim_matches(['"', '\'']).to_owned()).unwrap_or_default();
            (f.name, dimension)
        })
        .collect();
    Some((names, flavors))
}

/// The build variants an Android module's build file makes, as Gradle
/// names them (`freeDebug`: a flavor of each dimension, then the build
/// type), sorted; for Build Variants before the project is synced.
pub fn variant_names(script: &str, kotlin: bool) -> Vec<String> {
    let Some((types, flavors)) = variant_parts_of(script, kotlin) else { return Vec::new() };
    let capitalized = |s: &str| {
        let mut chars = s.chars();
        chars.next().map(|c| c.to_uppercase().chain(chars).collect::<String>()).unwrap_or_default()
    };
    // Dimensions in the order they first show up.
    let mut dimensions: Vec<&str> = Vec::new();
    for (_, d) in &flavors {
        if !dimensions.contains(&d.as_str()) {
            dimensions.push(d);
        }
    }
    let mut prefixes = vec![String::new()];
    for d in dimensions {
        let names: Vec<&str> = flavors.iter().filter(|(_, fd)| fd == d).map(|(f, _)| f.as_str()).collect();
        prefixes = prefixes.iter().flat_map(|p| names.iter().map(move |f| if p.is_empty() { f.to_string() } else { format!("{p}{}", capitalized(f)) })).collect();
    }
    let mut out: Vec<String> = prefixes.iter().flat_map(|p| types.iter().map(move |t| if p.is_empty() { t.clone() } else { format!("{p}{}", capitalized(t)) })).collect();
    out.sort();
    out
}

/// A folder or accessor name compared loosely: `core-data`, `coreData` and
/// `core_data` are one.
fn loose(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric() || *c == '/').flat_map(char::to_lowercase).collect()
}

impl ModuleGraph {
    /// The Gradle modules among the project's `files`, with their build
    /// files read by `read` (project-relative paths).
    pub fn build(files: &[String], read: &dyn Fn(&str) -> Option<String>) -> Self {
        let mut builds: Vec<(String, String)> = files
            .iter()
            .filter_map(|f| {
                let (dir, name) = f.rsplit_once('/').unwrap_or(("", f));
                matches!(name, "build.gradle" | "build.gradle.kts").then(|| (dir.to_owned(), f.clone()))
            })
            .collect();
        builds.sort();
        builds.dedup_by(|a, b| a.0 == b.0);
        let mut modules: Vec<Module> = builds.iter().map(|(dir, _)| Module { dir: dir.clone(), ..Default::default() }).collect();
        for (i, (_, file)) in builds.iter().enumerate() {
            let Some(text) = read(file) else { continue };
            if let Some((types, flavors)) = variant_parts_of(&text, file.ends_with(".kts")) {
                modules[i].build_types = types;
                modules[i].flavors = flavors;
            }
            let named = module_dependencies(&text).into_iter().chain(accessor_dependencies(&text));
            let mut deps = Vec::new();
            for name in named {
                let wanted = loose(&name);
                // The folder the path names, else one ending in it (a
                // module moved with `projectDir`, or under a parent folder).
                let found = modules.iter().position(|m| loose(&m.dir) == wanted).or_else(|| {
                    let last = wanted.rsplit('/').next().unwrap_or(&wanted).to_owned();
                    modules.iter().position(|m| !m.dir.is_empty() && loose(m.dir.rsplit('/').next().unwrap_or(&m.dir)) == last)
                });
                if let Some(j) = found.filter(|j| *j != i && !deps.contains(j)) {
                    deps.push(j);
                }
            }
            modules[i].deps = deps;
        }
        ModuleGraph { modules }
    }

    /// Makes `variant` the one module `dir` builds (project-relative), as
    /// picked in Build Variants.
    pub fn pick(&mut self, dir: &str, variant: &str) {
        if let Some(m) = self.modules.iter_mut().find(|m| m.dir == dir) {
            m.picked = Some(variant.to_owned());
        }
    }

    /// The build types and flavors a variant source set's name is made of
    /// (`freeDebug` → free, debug), each with its group: [`BUILD_TYPE`],
    /// else the flavor's dimension. None for any other set (`main`,
    /// `test`, `commonMain`…).
    fn variant_parts(&self, module: usize, set: &str) -> Option<Vec<(String, String)>> {
        let m = &self.modules[module];
        if m.build_types.is_empty() {
            return None;
        }
        let names: Vec<(&str, &str)> = m.build_types.iter().map(|t| (t.as_str(), BUILD_TYPE)).chain(m.flavors.iter().map(|(f, d)| (f.as_str(), d.as_str()))).collect();
        fn split(rest: &str, first: bool, names: &[(&str, &str)], out: &mut Vec<(String, String)>) -> bool {
            if rest.is_empty() {
                return !out.is_empty();
            }
            for (name, group) in names {
                let mut word = name.to_string();
                if !first {
                    let mut chars = word.chars();
                    word = chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default();
                }
                if let Some(next) = rest.strip_prefix(word.as_str()) {
                    out.push((name.to_string(), group.to_string()));
                    if split(next, false, names, out) {
                        return true;
                    }
                    out.pop();
                }
            }
            false
        }
        let mut out = Vec::new();
        split(set, true, &names, &mut out).then_some(out)
    }

    /// The build type and flavors of the variant a module builds: the
    /// picked one, else Android Studio's default (debug, and each
    /// dimension's first flavor by name).
    fn active_parts(&self, module: usize) -> Vec<(String, String)> {
        let m = &self.modules[module];
        // A variant named before the module was synced may leave out
        // flavors ("debug"): those dimensions take their default.
        let mut parts = m.picked.as_deref().and_then(|v| self.variant_parts(module, v)).unwrap_or_default();
        if !parts.iter().any(|(_, g)| g == BUILD_TYPE) {
            parts.push(("debug".to_owned(), BUILD_TYPE.to_owned()));
        }
        let mut flavors = m.flavors.clone();
        flavors.sort();
        for (name, dimension) in flavors {
            if !parts.iter().any(|(_, d)| *d == dimension) {
                parts.push((name, dimension));
            }
        }
        parts
    }

    /// Whether code in `from_set` (of module `a`) sees the variant source
    /// set `to_set` of module `b`: one of the variant it builds, or for
    /// code in a variant set itself, one of the variants that set is in
    /// (`release` code sees `release` and the active flavors, not `debug`).
    fn variant_visible(&self, a: usize, from_set: &str, b: usize, to_set: &str) -> bool {
        let Some(to) = self.variant_parts(b, to_set) else { return true };
        let from = if a == b { self.variant_parts(a, from_set).unwrap_or_default() } else { Vec::new() };
        let active = self.active_parts(b);
        to.iter().all(|(name, group)| {
            if from.iter().any(|(n, _)| n == name) {
                return true;
            }
            // Another of the same build type or dimension as `from`'s.
            if from.iter().any(|(_, g)| g == group) {
                return false;
            }
            active.iter().any(|(n, _)| n == name)
        })
    }

    /// The innermost module holding `path`.
    fn module_of(&self, path: &str) -> Option<usize> {
        self.modules
            .iter()
            .enumerate()
            .filter(|(_, m)| m.dir.is_empty() || path.starts_with(&format!("{}/", m.dir)))
            .max_by_key(|(_, m)| m.dir.len())
            .map(|(i, _)| i)
    }

    /// The source set of a file under `<module>/src/<set>/`.
    fn source_set<'p>(&self, module: usize, path: &'p str) -> Option<&'p str> {
        let dir = &self.modules[module].dir;
        let rest = if dir.is_empty() { path } else { &path[dir.len() + 1..] };
        let rest = rest.strip_prefix("src/")?;
        let (set, inner) = rest.split_once('/')?;
        (!inner.is_empty()).then_some(set)
    }

    /// Whether code in `from` can name what's declared in `to` (both
    /// project-relative); anything outside a module's source sets (a
    /// library, a build script, a folder that isn't a Gradle module)
    /// counts as visible.
    pub fn visible(&self, from: &str, to: &str) -> bool {
        let (Some(a), Some(b)) = (self.module_of(from), self.module_of(to)) else { return true };
        let (Some(from_set), Some(to_set)) = (self.source_set(a, from), self.source_set(b, to)) else { return true };
        let (from_kind, to_kind) = (set_kind(from_set), set_kind(to_set));
        if to_kind == SetKind::Production && !self.variant_visible(a, from_set, b, to_set) {
            return false;
        }
        if a == b {
            return match from_kind {
                SetKind::Production => to_kind == SetKind::Production,
                SetKind::Fixtures => matches!(to_kind, SetKind::Production | SetKind::Fixtures),
                SetKind::Test => to_kind != SetKind::AndroidTest,
                SetKind::AndroidTest => to_kind != SetKind::Test,
            };
        }
        // Another module: its production code (and test fixtures), when a
        // dependency, direct or through others.
        if !matches!(to_kind, SetKind::Production | SetKind::Fixtures) {
            return false;
        }
        let mut seen = HashSet::new();
        let mut queue = vec![a];
        while let Some(m) = queue.pop() {
            if !seen.insert(m) {
                continue;
            }
            if m == b {
                return true;
            }
            queue.extend(self.modules[m].deps.iter().copied());
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(builds: &[(&str, &str)]) -> ModuleGraph {
        let files: Vec<String> = builds.iter().map(|(p, _)| p.to_string()).collect();
        let read = |p: &str| builds.iter().find(|(f, _)| *f == p).map(|(_, t)| t.to_string());
        ModuleGraph::build(&files, &read)
    }

    #[test]
    fn sees_own_module_and_dependencies() {
        let g = graph(&[
            ("build.gradle.kts", ""),
            ("app/build.gradle.kts", "dependencies {\n    implementation(project(\":core:data\"))\n    implementation(projects.feature.login)\n}\n"),
            ("core/data/build.gradle", "dependencies { api project(':core:model') }\n"),
            ("core/model/build.gradle.kts", ""),
            ("feature/login/build.gradle.kts", ""),
            ("other/build.gradle.kts", ""),
        ]);
        let app = "app/src/main/java/a/App.kt";
        assert!(g.visible(app, "app/src/main/java/a/Other.kt"));
        assert!(g.visible(app, "core/data/src/main/java/d/Repo.kt"));
        // Through core:data's api dependency.
        assert!(g.visible(app, "core/model/src/main/kotlin/m/User.kt"));
        assert!(g.visible(app, "feature/login/src/main/java/l/Login.kt"));
        assert!(!g.visible(app, "other/src/main/java/o/Other.kt"));
        assert!(!g.visible("core/data/src/main/java/d/Repo.kt", app));
        // Another module's tests, never.
        assert!(!g.visible(app, "core/data/src/test/java/d/RepoTest.kt"));
        // Not in a source set: a build script, a library, the root's own folders.
        assert!(g.visible("app/build.gradle.kts", "other/src/main/java/o/Other.kt"));
        assert!(g.visible(app, "/home/u/.gradle/caches/x/Lib.java"));
    }

    #[test]
    fn source_sets_see_main_but_not_the_other_way() {
        let g = graph(&[("app/build.gradle.kts", ""), ("lib/build.gradle.kts", "dependencies { testFixturesApi(project(\":x\")) }")]);
        let main = "app/src/main/java/a/A.kt";
        let test = "app/src/test/java/a/ATest.kt";
        let android = "app/src/androidTest/java/a/AUiTest.kt";
        let debug = "app/src/debug/java/a/D.kt";
        assert!(g.visible(test, main));
        assert!(g.visible(android, main));
        assert!(g.visible(main, debug) && g.visible(debug, main));
        assert!(!g.visible(main, test));
        assert!(!g.visible(main, android));
        assert!(!g.visible(test, android) && !g.visible(android, test));
        assert!(g.visible(test, "app/src/testFixtures/java/a/Fake.kt"));
        // Kotlin Multiplatform's sets.
        assert!(g.visible("app/src/commonTest/kotlin/T.kt", "app/src/commonMain/kotlin/M.kt"));
        assert!(!g.visible("app/src/commonMain/kotlin/M.kt", "app/src/commonTest/kotlin/T.kt"));
    }

    #[test]
    fn sees_the_active_variants_sets() {
        let script = "plugins { id(\"com.android.application\") }\nandroid {\n    buildTypes {\n        create(\"staging\") { }\n    }\n    productFlavors {\n        create(\"paid\") {\n            dimension = \"tier\"\n        }\n        create(\"free\") {\n            dimension = \"tier\"\n        }\n    }\n}\ndependencies { implementation(project(\":lib\")) }\n";
        let mut g = graph(&[("app/build.gradle.kts", script), ("lib/build.gradle.kts", "android { }\n")]);
        let main = "app/src/main/java/a/A.kt";
        let at = |set: &str| format!("app/src/{set}/java/a/Config.kt");
        // freeDebug by default: the first flavor by name.
        assert!(g.visible(main, &at("debug")) && g.visible(main, &at("free")) && g.visible(main, &at("freeDebug")));
        assert!(!g.visible(main, &at("release")) && !g.visible(main, &at("paid")) && !g.visible(main, &at("paidDebug")));
        assert!(!g.visible(main, &at("staging")));
        // Release code sees release and the active flavor, not debug.
        assert!(g.visible(&at("release"), &at("free")) && g.visible(&at("release"), &at("freeRelease")));
        assert!(!g.visible(&at("release"), &at("debug")));
        // Not variant sets: always there.
        assert!(g.visible(main, "app/src/androidMain/kotlin/a/P.kt"));
        // Another module's: its own default variant.
        assert!(g.visible(main, "lib/src/debug/java/l/L.kt") && !g.visible(main, "lib/src/release/java/l/L.kt"));
        // A variant without the flavors: the default one's.
        g.pick("app", "release");
        assert!(g.visible(main, &at("release")) && g.visible(main, &at("free")) && !g.visible(main, &at("debug")));
        g.pick("app", "paidStaging");
        assert!(g.visible(main, &at("paid")) && g.visible(main, &at("staging")) && g.visible(main, &at("paidStaging")));
        assert!(!g.visible(main, &at("debug")) && !g.visible(main, &at("free")));
        assert_eq!(variant_names(script, true), ["freeDebug", "freeRelease", "freeStaging", "paidDebug", "paidRelease", "paidStaging"]);
        assert_eq!(variant_names("android { }\n", true), ["debug", "release"]);
        assert!(variant_names("plugins { kotlin(\"jvm\") }\n", true).is_empty());
    }
}
