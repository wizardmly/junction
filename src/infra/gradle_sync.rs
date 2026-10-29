//! The sync port over Gradle: an init script adds a `junctionSync` task to
//! every project that resolves its compile classpaths (main, unit test and
//! instrumented test, for Android the debug variant) and asks for their
//! sources, so Gradle downloads what is missing into its cache; the task
//! prints what it resolved. Records are kept in Junction's cache.

use std::path::{Path, PathBuf};

use crate::domain::run::{ModuleInfo, ModuleType};
use crate::domain::sync::{self, Coordinate, ProjectSync, SyncRecord, SyncReport};
use crate::index::libraries::{cache_dir, stable_hash};

const INIT_SCRIPT: &str = r#"// Written by Junction for Sync Project with Gradle Files.
import org.gradle.api.artifacts.component.ModuleComponentIdentifier
import org.gradle.api.artifacts.result.ResolvedArtifactResult
import org.gradle.api.artifacts.result.UnresolvedDependencyResult
import org.gradle.jvm.JvmLibrary
import org.gradle.language.base.artifact.SourcesArtifact

allprojects { p ->
    p.tasks.register("junctionSync") {
        doLast {
            def has = { id -> p.pluginManager.hasPlugin(id) }
            def androidLibrary = ["com.android.library", "com.android.dynamic-feature", "com.android.test", "com.android.kotlin.multiplatform.library"].any { has(it) }
            def kind = has("com.android.application") ? "application" : (androidLibrary ? "library" : (has("java") ? "jvm" : "other"))
            def variants = []
            if (kind == "application" || kind == "library") {
                variants = p.configurations.names.findAll { it.endsWith("RuntimeClasspath") }.collect { it - "RuntimeClasspath" }
                    .findAll { it && !(it ==~ /.*(UnitTest|AndroidTest|TestFixtures|ScreenshotTest)$/) && it != "test" }.sort()
            }
            def chosen = System.getProperty("junction.variants", "").split(";").findAll { it.contains("=") }.collectEntries { def kv = it.split("=", 2); [(kv[0]): kv[1]] }
            def variant = chosen[p.path] ?: (variants.contains("debug") ? "debug" : (variants.find { it.endsWith("Debug") } ?: (variants ? variants[0] : "debug")))
            println "JUNCTION_MODULE ${p.path}\t${kind}\t${variants.join(',')}\t${p.projectDir}"
            def names = [variant + "CompileClasspath", variant + "UnitTestCompileClasspath", variant + "AndroidTestCompileClasspath", "compileClasspath", "testCompileClasspath"]
            def ids = new LinkedHashSet()
            names.each { name ->
                def conf = p.configurations.findByName(name)
                if (conf == null || !conf.canBeResolved) return
                try {
                    def graph = conf.incoming.resolutionResult
                    graph.allComponents.each { c ->
                        if (c.id instanceof ModuleComponentIdentifier) ids << c.id
                    }
                    graph.allDependencies.each { d ->
                        if (d instanceof UnresolvedDependencyResult) println "JUNCTION_UNRESOLVED ${p.path} ${d.attempted.displayName}"
                    }
                } catch (Throwable e) {
                    println "JUNCTION_ERROR ${p.path} ${name}: ${e.message?.readLines()?.find { it }}"
                }
            }
            ids.each { println "JUNCTION_DEP ${p.path} ${it.group}:${it.module}:${it.version}" }
            if (ids.isEmpty()) return
            try {
                def result = p.dependencies.createArtifactResolutionQuery().forComponents(ids).withArtifacts(JvmLibrary, SourcesArtifact).execute()
                result.resolvedComponents.each { c ->
                    c.getArtifacts(SourcesArtifact).each { a ->
                        if (a instanceof ResolvedArtifactResult) println "JUNCTION_SOURCES ${c.id.group}:${c.id.module}:${c.id.version} ${a.file}"
                    }
                }
            } catch (Throwable e) {
                println "JUNCTION_ERROR ${p.path} sources: ${e.message?.readLines()?.find { it }}"
            }
        }
    }
}
"#;

pub struct GradleSync;

impl ProjectSync for GradleSync {
    fn build_roots(&self, root: &Path, files: &[String]) -> Vec<PathBuf> {
        let settings = |f: &&String| {
            let name = f.rsplit('/').next().unwrap_or(f);
            name == "settings.gradle" || name == "settings.gradle.kts"
        };
        let mut dirs: Vec<String> = files.iter().filter(settings).map(|f| f.rsplit_once('/').map_or(String::new(), |(d, _)| d.to_owned())).collect();
        if dirs.is_empty() && files.iter().any(|f| f == "build.gradle" || f == "build.gradle.kts") {
            dirs.push(String::new());
        }
        dirs.sort();
        dirs.dedup();
        // An included build (build-logic, buildSrc…) syncs with its parent.
        let nested = |d: &String| dirs.iter().any(|o| o != d && (o.is_empty() || d.starts_with(&format!("{o}/"))));
        dirs.iter().filter(|d| !nested(d)).map(|d| if d.is_empty() { root.to_path_buf() } else { root.join(d) }).collect()
    }

    fn stamp(&self, root: &Path, build_root: &Path, files: &[String]) -> u64 {
        let prefix = build_root.strip_prefix(root).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default();
        let mut text = String::new();
        for file in files.iter().filter(|f| sync::is_build_file(f) && (prefix.is_empty() || f.starts_with(&format!("{prefix}/")))) {
            let meta = std::fs::metadata(root.join(file)).ok();
            let mtime = meta.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
            text.push_str(&format!("{file}:{}:{mtime};", meta.map_or(0, |m| m.len())));
        }
        stable_hash(&text)
    }

    fn sync(&self, build_root: &Path, variants: &[(String, String)], line: &mut dyn FnMut(&str), cancelled: &dyn Fn() -> bool) -> SyncReport {
        let Some(script) = init_script() else {
            return SyncReport { output: "Could not write the sync script to Junction's cache.".into(), ..Default::default() };
        };
        let mut command = super::gradle::gradle(build_root);
        command
            .arg("--init-script")
            .arg(&script)
            .arg("junctionSync")
            .arg(format!("-Djunction.variants={}", variants.iter().map(|(m, v)| format!("{m}={v}")).collect::<Vec<_>>().join(";")))
            // The task reads configurations as it runs.
            .arg("-Dorg.gradle.configuration-cache=false");
        line(&super::gradle::environment(build_root));
        // The task's own lines (the classpath it reports) aren't for people.
        let (succeeded, was_cancelled, output) = super::gradle::stream(command, &mut |l| if !l.starts_with("JUNCTION_") { line(l) }, cancelled);
        let mut report = parse_output(&output);
        report.succeeded = succeeded;
        report.cancelled = was_cancelled;
        report.output = output;
        report
    }

    fn load(&self, build_root: &Path) -> Option<SyncRecord> {
        load(build_root)
    }

    fn save(&self, build_root: &Path, record: &SyncRecord) {
        let Some(path) = record_path(build_root) else { return };
        let mut text = format!("junction-sync 1\nstamp {}\n", record.stamp);
        for m in &record.modules {
            text.push_str(&format!("module\t{}\n", module_line(m)));
        }
        for c in &record.dependencies {
            text.push_str(&format!("{c}\n"));
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        // Whole or not at all: the index reads it meanwhile.
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, text).is_ok() {
            std::fs::rename(&tmp, &path).ok();
        }
    }
}

/// Where a build's last sync is kept.
pub fn record_path(build_root: &Path) -> Option<PathBuf> {
    let key = crate::domain::diagnostics::file_key(build_root);
    let name: String = build_root.file_name().map(|n| n.to_string_lossy().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(40).collect()).unwrap_or_default();
    Some(cache_dir()?.join("gradle-sync").join(format!("{name}-{:016x}.txt", stable_hash(&key))))
}

/// A build's last sync, if it was synced.
pub fn load(build_root: &Path) -> Option<SyncRecord> {
    let text = std::fs::read_to_string(record_path(build_root)?).ok()?;
    let mut lines = text.lines();
    if lines.next()? != "junction-sync 1" {
        return None;
    }
    let stamp = lines.next()?.strip_prefix("stamp ")?.parse().ok()?;
    let mut record = SyncRecord { stamp, ..Default::default() };
    for line in lines {
        match line.strip_prefix("module\t") {
            Some(m) => record.modules.extend(parse_module(m)),
            None => record.dependencies.extend(Coordinate::parse(line)),
        }
    }
    Some(record)
}

fn module_line(m: &ModuleInfo) -> String {
    format!("{}\t{}\t{}\t{}", m.path, m.kind.key(), m.variants.join(","), m.dir.display())
}

/// `:app<TAB>application<TAB>debug,release<TAB>/p/app`.
fn parse_module(text: &str) -> Option<ModuleInfo> {
    let mut parts = text.splitn(4, '\t');
    let (path, kind, variants, dir) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    Some(ModuleInfo {
        path: path.to_owned(),
        dir: PathBuf::from(dir.trim_end()),
        kind: ModuleType::from_key(kind),
        variants: variants.split(',').filter(|v| !v.is_empty()).map(str::to_owned).collect(),
    })
}

/// `id("com.android.application") apply false`, `alias(…) apply (false)`.
fn is_unapplied_plugin(line: &str) -> bool {
    let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    compact.contains("applyfalse") || compact.contains("apply(false)")
}

/// The modules of a build that was never synced: each directory with a
/// build file under the build root, typed by the plugins it names.
pub fn guess_modules(root: &Path, build_root: &Path, files: &[String]) -> Vec<ModuleInfo> {
    let mut out: Vec<ModuleInfo> = files
        .iter()
        .filter(|f| f.ends_with("build.gradle") || f.ends_with("build.gradle.kts"))
        .filter_map(|f| {
            let dir = root.join(f).parent()?.to_path_buf();
            if !dir.starts_with(build_root) {
                return None;
            }
            let text = std::fs::read_to_string(root.join(f)).unwrap_or_default();
            // A root build file declares the plugins for its modules with
            // `apply false`: those don't make it one.
            let text: String = text.lines().filter(|l| !is_unapplied_plugin(l)).collect::<Vec<_>>().join("\n");
            let kind = if text.contains("com.android.application") || text.contains("android.application") {
                ModuleType::Application
            } else if text.contains("com.android.") || text.contains("android.library") {
                ModuleType::AndroidLibrary
            } else if text.contains("kotlin(\"jvm\")") || text.contains("java-library") || text.contains("kotlin.jvm") || text.contains("`java`") || text.contains("id 'java'") {
                ModuleType::Jvm
            } else {
                ModuleType::Other
            };
            let variants = if kind.is_android() { crate::domain::modules::variant_names(&text, f.ends_with(".kts")) } else { Vec::new() };
            Some(ModuleInfo { path: crate::domain::build::project_path(build_root, &dir), dir, kind, variants })
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn init_script() -> Option<PathBuf> {
    let path = cache_dir()?.join("gradle-sync").join("junction-sync.gradle");
    if std::fs::read_to_string(&path).ok().as_deref() != Some(INIT_SCRIPT) {
        std::fs::create_dir_all(path.parent()?).ok()?;
        std::fs::write(&path, INIT_SCRIPT).ok()?;
    }
    Some(path)
}

/// The task's lines in Gradle's output.
pub fn parse_output(output: &str) -> SyncReport {
    let mut report = SyncReport::default();
    for line in output.lines().map(str::trim_end) {
        if let Some(rest) = line.strip_prefix("JUNCTION_MODULE ") {
            report.modules.extend(parse_module(rest));
        } else if let Some(rest) = line.strip_prefix("JUNCTION_DEP ") {
            if let Some(c) = rest.split_once(' ').and_then(|(_, c)| Coordinate::parse(c)) {
                report.dependencies.push(c);
            }
        } else if let Some(rest) = line.strip_prefix("JUNCTION_UNRESOLVED ") {
            let what = rest.split_once(' ').map_or(rest, |(_, w)| w);
            let message = format!("Failed to resolve: {what}");
            if !report.unresolved.contains(&message) {
                report.unresolved.push(message);
            }
        }
    }
    report.modules.sort_by(|a, b| a.path.cmp(&b.path));
    report.modules.dedup_by(|a, b| a.path == b.path);
    report.dependencies.sort();
    report.dependencies.dedup();
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_task_output() {
        let output = "\
> Task :app:junctionSync
JUNCTION_MODULE :app\tapplication\tdebug,release\t/p/app
JUNCTION_UNRESOLVED :app does.not:exist:1.0
JUNCTION_UNRESOLVED :app does.not:exist:1.0
JUNCTION_DEP :app com.squareup.okhttp3:okhttp:4.12.0
JUNCTION_DEP :lib com.squareup.okhttp3:okhttp:4.12.0
JUNCTION_DEP :lib com.google.code.gson:gson:2.11.0
JUNCTION_SOURCES com.google.code.gson:gson:2.11.0 /g/gson-2.11.0-sources.jar
BUILD SUCCESSFUL in 2s
";
        let r = parse_output(output);
        assert_eq!(r.dependencies.len(), 2);
        assert_eq!(r.dependencies[0].to_string(), "com.google.code.gson:gson:2.11.0");
        assert_eq!(r.unresolved, vec!["Failed to resolve: does.not:exist:1.0"]);
        assert_eq!(r.modules.len(), 1);
        assert_eq!((r.modules[0].kind, r.modules[0].variants.len()), (ModuleType::Application, 2));
        assert_eq!(parse_module(&module_line(&r.modules[0])).as_ref(), Some(&r.modules[0]));
    }

    #[test]
    fn finds_builds_and_stamps_their_files() {
        let files: Vec<String> = ["settings.gradle.kts", "app/build.gradle.kts", "build-logic/settings.gradle.kts", "tools/x/settings.gradle", "app/src/A.kt"].iter().map(|s| s.to_string()).collect();
        let root = Path::new("/p");
        assert_eq!(GradleSync.build_roots(root, &files), vec![PathBuf::from("/p")]);
        let files2: Vec<String> = files[3..].to_vec();
        assert_eq!(GradleSync.build_roots(root, &files2), vec![PathBuf::from("/p/tools/x")]);

        let dir = std::env::temp_dir().join(format!("junction-sync-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("app/src")).unwrap();
        std::fs::write(dir.join("settings.gradle.kts"), "").unwrap();
        std::fs::write(dir.join("app/build.gradle.kts"), "a").unwrap();
        std::fs::write(dir.join("app/src/A.kt"), "").unwrap();
        let files: Vec<String> = vec!["settings.gradle.kts".into(), "app/build.gradle.kts".into(), "app/src/A.kt".into()];
        let before = GradleSync.stamp(&dir, &dir, &files);
        std::fs::write(dir.join("app/src/A.kt"), "changed").unwrap();
        assert_eq!(GradleSync.stamp(&dir, &dir, &files), before);
        std::fs::write(dir.join("app/build.gradle.kts"), "changed").unwrap();
        assert_ne!(GradleSync.stamp(&dir, &dir, &files), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_root_declaring_plugins_is_no_app() {
        let dir = std::env::temp_dir().join(format!("junction-guess-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("app")).unwrap();
        std::fs::write(dir.join("build.gradle.kts"), "plugins {\n    alias(libs.plugins.android.application) apply false\n    id(\"com.android.library\") apply (false)\n}\n").unwrap();
        std::fs::write(dir.join("app/build.gradle.kts"), "plugins {\n    alias(libs.plugins.android.application)\n}\n").unwrap();
        let files: Vec<String> = vec!["build.gradle.kts".into(), "app/build.gradle.kts".into()];
        let modules = guess_modules(&dir, &dir, &files);
        let apps: Vec<&str> = modules.iter().filter(|m| m.kind == ModuleType::Application).map(|m| m.path.as_str()).collect();
        assert_eq!(apps, [":app"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
