//! Android Lint, as Inspect Code runs it on Android modules: the Gradle
//! task of a module's variant, where it writes its XML report, and the
//! issues read back from the report.

use std::path::{Path, PathBuf};

use super::diagnostics::Severity;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LintIssue {
    pub id: String,
    pub severity: Severity,
    pub category: String,
    pub message: String,
    pub file: PathBuf,
    /// 0-based; a file-level issue is on line 0.
    pub line: u32,
    pub col: u32,
}

fn capitalized(variant: &str) -> String {
    let mut chars = variant.chars();
    chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

/// `:app` and `debug` → `:app:lintDebug` (`lintDebug` for the root project).
pub fn task(module: &str, variant: &str) -> String {
    let module = module.trim_end_matches(':');
    format!("{module}:lint{}", capitalized(variant))
}

/// The report AGP writes for the variant: `build/reports/lint-results-debug.xml`.
pub fn report(module_dir: &Path, variant: &str) -> PathBuf {
    module_dir.join("build/reports").join(format!("lint-results-{variant}.xml"))
}

/// The report's issues; a location's relative file is under `module_dir`.
pub fn parse(xml: &str, module_dir: &Path) -> Vec<LintIssue> {
    let Ok(doc) = roxmltree::Document::parse(xml) else { return Vec::new() };
    let mut out = Vec::new();
    for issue in doc.descendants().filter(|n| n.has_tag_name("issue")) {
        let Some(location) = issue.children().find(|n| n.has_tag_name("location")) else { continue };
        let Some(file) = location.attribute("file") else { continue };
        let number = |name: &str| location.attribute(name).and_then(|v| v.parse::<u32>().ok()).map_or(0, |n| n.saturating_sub(1));
        let severity = match issue.attribute("severity").unwrap_or("Warning") {
            "Fatal" | "Error" => Severity::Error,
            _ => Severity::Warning,
        };
        let path = Path::new(file);
        out.push(LintIssue {
            id: issue.attribute("id").unwrap_or_default().to_owned(),
            severity,
            category: issue.attribute("category").unwrap_or_default().to_owned(),
            message: issue.attribute("message").unwrap_or_default().to_owned(),
            file: if path.is_absolute() { path.to_path_buf() } else { module_dir.join(path) },
            line: number("line"),
            col: number("column"),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_report() {
        assert_eq!(task(":app", "demoDebug"), ":app:lintDemoDebug");
        assert_eq!(report(Path::new("/p/app"), "debug"), PathBuf::from("/p/app/build/reports/lint-results-debug.xml"));
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<issues format="6" by="lint 8.5.0">
    <issue id="HardcodedText" severity="Warning" message="Hardcoded string &quot;Hi&quot;" category="Internationalization" priority="5">
        <location file="src/main/res/layout/main.xml" line="12" column="9"/>
    </issue>
    <issue id="MissingPermission" severity="Error" message="Call requires permission" category="Correctness">
        <location file="/abs/A.kt" line="3"/>
    </issue>
    <issue id="UnusedResources" severity="Warning" message="Unused" category="Performance"/>
</issues>"#;
        let issues = parse(xml, Path::new("/p/app"));
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].file, PathBuf::from("/p/app/src/main/res/layout/main.xml"));
        assert_eq!((issues[0].line, issues[0].col, issues[0].message.as_str()), (11, 8, "Hardcoded string \"Hi\""));
        assert_eq!((issues[1].severity, issues[1].line, issues[1].col), (Severity::Error, 2, 0));
    }
}
