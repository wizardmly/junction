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
}

#[derive(Clone, Debug, Default)]
pub struct ModuleGraph {
    modules: Vec<Module>,
}

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
        let mut modules: Vec<Module> = builds.iter().map(|(dir, _)| Module { dir: dir.clone(), deps: Vec::new() }).collect();
        for (i, (_, file)) in builds.iter().enumerate() {
            let Some(text) = read(file) else { continue };
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
}
