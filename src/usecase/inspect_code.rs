//! Analyze › Inspect Code…: the inspections Junction has, run over a scope
//! of the project, as IntelliJ's Inspection Results: syntax errors from
//! the parser, unused imports in Java and Kotlin, and Android Lint (its
//! Gradle task) on Android modules.

use crate::domain::diagnostics::{Origin, Pos, Problem, Severity, SyntaxChecker};
use crate::domain::imports::{self, ImportStyle};
use crate::domain::project_files::ProjectFiles;

/// One inspection's finding in a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// IntelliJ's inspection name, the group in the results.
    pub inspection: &'static str,
    pub problem: Problem,
}

pub const SYNTAX: &str = "Syntax error";
pub const UNUSED_KOTLIN: &str = "Unused import directive";
pub const UNUSED_JAVA: &str = "Unused import";

/// The files of the selection (project-relative paths and whether each is
/// a folder; the empty path is the project) `inspectable` accepts, in order.
pub fn targets(files: &dyn ProjectFiles, items: &[(String, bool)], inspectable: &dyn Fn(&str) -> bool) -> Vec<String> {
    let listed = items.iter().any(|(_, dir)| *dir).then(|| files.list()).unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    for (path, dir) in items {
        if *dir {
            let prefix = if path.is_empty() { String::new() } else { format!("{}/", path.trim_end_matches('/')) };
            out.extend(listed.iter().filter(|p| p.starts_with(&prefix) && inspectable(p)).cloned());
        } else if inspectable(path) {
            out.push(path.clone());
        }
    }
    out.sort();
    out.dedup();
    out
}

/// A file's findings, in order; `syntax` checks its language's parse.
pub fn inspect(path: &str, text: &str, syntax: Option<&dyn SyntaxChecker>) -> Vec<Finding> {
    let mut out: Vec<Finding> = syntax.map(|s| s.check(text)).unwrap_or_default().into_iter().map(|problem| Finding { inspection: SYNTAX, problem }).collect();
    let style = if path.ends_with(".kt") || path.ends_with(".kts") {
        Some((ImportStyle::Kotlin, UNUSED_KOTLIN))
    } else if path.ends_with(".java") {
        Some((ImportStyle::Java, UNUSED_JAVA))
    } else {
        None
    };
    if let Some((style, inspection)) = style {
        for (line, path) in imports::unused_imports(style, text) {
            let col = text.lines().nth(line as usize).map_or(0, |l| (l.len() - l.trim_start().len()) as u32);
            let message = match style {
                ImportStyle::Kotlin => format!("Unused import directive '{path}'"),
                ImportStyle::Java => format!("Unused import '{path}'"),
            };
            let problem = Problem { start: Pos::new(line, col), end: Pos::new(line, col), severity: Severity::Warning, message, origin: Origin::Inspection, fix: None };
            out.push(Finding { inspection, problem });
        }
    }
    out.sort_by_key(|f| (f.problem.start, f.problem.end));
    out
}

/// An Android module Inspect Code runs Lint on.
#[derive(Clone, Debug)]
pub struct LintModule {
    /// ":app".
    pub path: String,
    pub dir: std::path::PathBuf,
    pub variant: String,
}

/// Android Lint on the modules of one build, in one Gradle run; the issues
/// as findings by file. Lint fails its task on errors but still writes the
/// report, so a failed run only counts when no report came out.
pub fn run_lint(
    runner: &dyn crate::domain::run::TaskRunner,
    build_root: &std::path::Path,
    modules: &[LintModule],
    read: &dyn Fn(&std::path::Path) -> Option<String>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<(std::path::PathBuf, Finding)>, String> {
    use crate::domain::lint;
    let tasks: Vec<String> = std::iter::once("--continue".to_owned()).chain(modules.iter().map(|m| lint::task(&m.path, &m.variant))).collect();
    let result = runner.run_tasks(build_root, &tasks, &[], &mut |_| {}, cancelled);
    let mut out = Vec::new();
    let mut reports = 0;
    for m in modules {
        let Some(xml) = read(&lint::report(&m.dir, &m.variant)) else { continue };
        reports += 1;
        for issue in lint::parse(&xml, &m.dir) {
            let at = Pos::new(issue.line, issue.col);
            let message = format!("{} [{}]", issue.message, issue.id);
            let problem = Problem { start: at, end: at, severity: issue.severity, message, origin: Origin::Inspection, fix: None };
            out.push((issue.file, Finding { inspection: LINT, problem }));
        }
    }
    if reports == 0 && !result.succeeded {
        return Err(result.failure.unwrap_or_else(|| "Android Lint did not run".to_owned()));
    }
    Ok(out)
}

pub const LINT: &str = "Android Lint";

#[cfg(test)]
mod tests {
    use super::*;

    struct NoErrors;
    impl SyntaxChecker for NoErrors {
        fn check(&self, _: &str) -> Vec<Problem> {
            Vec::new()
        }
    }

    struct FakeGradle;
    impl crate::domain::run::TaskRunner for FakeGradle {
        fn run_tasks(&self, _: &std::path::Path, tasks: &[String], _: &[(String, String)], _: &mut dyn FnMut(&str), _: &dyn Fn() -> bool) -> crate::domain::run::BuildResult {
            assert_eq!(tasks, ["--continue", ":app:lintDebug"]);
            crate::domain::run::BuildResult { succeeded: false, cancelled: false, problems: Vec::new(), failure: Some("Lint found errors".into()) }
        }
        fn run_program(&self, _: &std::path::Path, _: &[String], _: &mut dyn FnMut(&str), _: &dyn Fn() -> bool) -> crate::domain::run::BuildResult {
            unreachable!()
        }
        fn apk_output(&self, _: &crate::domain::run::ModuleInfo, _: &str) -> Option<crate::domain::run::ApkOutput> {
            None
        }
    }

    #[test]
    fn reads_lint_reports_even_when_lint_fails() {
        let modules = [LintModule { path: ":app".into(), dir: "/p/app".into(), variant: "debug".into() }];
        let report = |_: &std::path::Path| Some(r#"<issues><issue id="X" severity="Error" message="Bad"><location file="A.kt" line="2"/></issue></issues>"#.to_owned());
        let found = run_lint(&FakeGradle, std::path::Path::new("/p"), &modules, &report, &|| false).unwrap();
        assert_eq!(found[0].0, std::path::PathBuf::from("/p/app/A.kt"));
        assert_eq!(found[0].1.problem.message, "Bad [X]");
        let none = |_: &std::path::Path| None;
        assert_eq!(run_lint(&FakeGradle, std::path::Path::new("/p"), &modules, &none, &|| false).unwrap_err(), "Lint found errors");
    }

    #[test]
    fn finds_unused_imports() {
        let text = "package p\n\nimport a.B\n\nfun f() = 1\n";
        let found = inspect("app/F.kt", text, Some(&NoErrors));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].inspection, UNUSED_KOTLIN);
        assert_eq!(found[0].problem.start, Pos::new(2, 0));
    }
}
