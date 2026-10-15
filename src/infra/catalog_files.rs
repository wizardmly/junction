//! The project's version catalogs on disk: each build's
//! `gradle/<name>.versions.toml` (Gradle imports `libs` by itself), and
//! the ones a settings script creates from a file
//! (`create("tools") { from(files("../tools.versions.toml")) }`).

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use crate::domain::version_catalog::{self as catalog, Catalog};

static FROM_FILES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"create\s*\(?\s*["'](\w+)["']\s*\)?\s*\{[^}]*?from\s*\(?\s*files\s*\(\s*["']([^"']+)["']"#).unwrap());

/// `files`: the project's files, relative to `root`.
pub fn load(root: &Path, files: &[String]) -> Vec<Catalog> {
    let mut out: Vec<Catalog> = Vec::new();
    let mut add = |name: &str, rel: String| {
        if out.iter().any(|c| c.path == rel) {
            return;
        }
        if let Ok(text) = std::fs::read_to_string(root.join(&rel)) {
            out.push(Catalog::parse(name, &rel, &text));
        }
    };
    for rel in files {
        let file_name = rel.rsplit('/').next().unwrap_or(rel);
        if file_name.starts_with("settings.gradle") {
            let Ok(text) = std::fs::read_to_string(root.join(rel)) else { continue };
            let dir = rel.rsplit_once('/').map_or("", |(d, _)| d);
            for c in FROM_FILES.captures_iter(&text) {
                let joined = if dir.is_empty() { c[2].to_owned() } else { format!("{dir}/{}", &c[2]) };
                add(&c[1], normalize(&joined));
            }
        }
    }
    for rel in files {
        let (dir, file_name) = rel.rsplit_once('/').unwrap_or(("", rel));
        if let (Some(name), true) = (catalog::name_of(file_name), dir == "gradle" || dir.ends_with("/gradle")) {
            add(name, rel.clone());
        }
    }
    out
}

/// `a/b/../c` → `a/c`.
fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

#[cfg(test)]
mod tests {
    #[test]
    fn normalizes() {
        assert_eq!(super::normalize("build-logic/../gradle/tools.versions.toml"), "gradle/tools.versions.toml");
        let settings = r#"dependencyResolutionManagement { versionCatalogs { create("tools") { from(files("gradle/tools.versions.toml")) } } }"#;
        let c = super::FROM_FILES.captures(settings).unwrap();
        assert_eq!((&c[1], &c[2]), ("tools", "gradle/tools.versions.toml"));
    }
}
