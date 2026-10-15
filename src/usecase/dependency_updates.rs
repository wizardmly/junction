//! The newer-version check on a Gradle file (Android Studio's
//! GradleDependency / NewerVersionAvailable lint): each pinned dependency
//! or plugin version with a newer release gets a warning, and Alt+Enter
//! offers "Change to …".

use std::collections::HashMap;

use crate::domain::dependency_versions::{self as versions, VersionSite, VersionSource};
use crate::domain::diagnostics::{Fix, Origin, Pos, Problem, Severity};
use crate::domain::version_catalog::{self as catalog, Catalog};

/// The sites to check in a file: a catalog's pinned versions, or a build
/// script's. Other files have none.
pub fn sites(path: &str, text: &str) -> Vec<VersionSite> {
    let file_name = path.rsplit('/').next().unwrap_or(path);
    if let Some(name) = catalog::name_of(file_name) {
        return versions::catalog_sites(&Catalog::parse(name, path, text));
    }
    if catalog::is_build_script(path) {
        return versions::script_sites(text);
    }
    Vec::new()
}

/// Warnings for the sites with a newer version. Looks each module up once.
pub fn check(source: &(dyn VersionSource + Sync), text: &str, sites: &[VersionSite]) -> Vec<Problem> {
    // The lookups may go to the network: all modules at once, a few at a time.
    let mut modules: Vec<(String, bool)> = sites.iter().map(|s| (s.module.clone(), s.plugin)).collect();
    modules.sort();
    modules.dedup();
    let known: HashMap<(String, bool), Option<Vec<String>>> = std::thread::scope(|scope| {
        let chunk = modules.len().div_ceil(8).max(1);
        let workers: Vec<_> = modules.chunks(chunk).map(|part| scope.spawn(move || part.iter().map(|(m, p)| ((m.clone(), *p), source.versions(m, *p))).collect::<Vec<_>>())).collect();
        workers.into_iter().flat_map(|w| w.join().unwrap_or_default()).collect()
    });
    let mut out = Vec::new();
    for site in sites {
        let available = known.get(&(site.module.clone(), site.plugin)).cloned().flatten();
        let Some(newest) = available.as_deref().and_then(|a| versions::newer(&site.version, a)) else { continue };
        out.push(Problem {
            start: Pos::of(text, site.range.start),
            end: Pos::of(text, site.range.end),
            severity: Severity::Warning,
            message: versions::message(site, &newest),
            origin: Origin::Inspection,
            fix: Some(Fix { title: format!("Change to {newest}"), replacement: newest }),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake;
    impl VersionSource for Fake {
        fn versions(&self, module: &str, _: bool) -> Option<Vec<String>> {
            (module == "androidx.core:core-ktx").then(|| vec!["1.10.1".into(), "1.13.1".into(), "1.15.0-alpha01".into()])
        }
    }

    #[test]
    fn warns_with_a_fix() {
        let text = "dependencies {\n    implementation(\"androidx.core:core-ktx:1.10.1\")\n    implementation(\"x.y:z:1.0\")\n}\n";
        let sites = sites("app/build.gradle.kts", text);
        assert_eq!(sites.len(), 2);
        let problems = check(&Fake, text, &sites);
        assert_eq!(problems.len(), 1);
        let p = &problems[0];
        assert_eq!(p.message, "A newer version of androidx.core:core-ktx than 1.10.1 is available: 1.13.1");
        assert_eq!((p.start, p.end), (Pos::new(1, 43), Pos::new(1, 49)));
        assert_eq!(p.fix.as_ref().unwrap().title, "Change to 1.13.1");
        assert!(super::sites("src/Main.kt", text).is_empty());
    }
}
