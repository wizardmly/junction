//! Tests, as Android Studio runs them through Gradle: the test classes and
//! methods of a source file (the gutter's run icons), the Gradle tasks that
//! run them, the results tree the Run window shows, and IntelliJ's status
//! line and durations.

use std::path::{Path, PathBuf};

use super::run::{capitalize, BuildResult, ModuleInfo};

// --- Test sources ---------------------------------------------------------

/// Where a test lives: `src/test*` runs on the JVM, `src/androidTest*` on a
/// device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TestKind {
    Unit,
    Instrumented,
}

/// The kind of tests a file holds, by its source set (`src/test`,
/// `src/testDebug`, `src/androidTest`…); none outside test source sets.
pub fn test_kind(path: &str) -> Option<TestKind> {
    let path = path.replace('\\', "/");
    let mut parts = path.split('/').peekable();
    while let Some(part) = parts.next() {
        if part != "src" {
            continue;
        }
        let Some(set) = parts.peek() else { break };
        let is = |prefix: &str| set.strip_prefix(prefix).is_some_and(|rest| rest.is_empty() || rest.starts_with(|c: char| c.is_ascii_uppercase()));
        if is("androidTest") {
            return Some(TestKind::Instrumented);
        }
        if is("test") {
            return Some(TestKind::Unit);
        }
    }
    None
}

/// A runnable test in a source file: a test class, or one of its test
/// methods. `class` is the JVM name (`com.example.OuterTest$Inner`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TestMark {
    /// 0-based line of the class or method name.
    pub line: u32,
    /// The 0-based line its body ends on.
    pub end: u32,
    pub class: String,
    pub method: Option<String>,
}

impl TestMark {
    /// The run configuration's name, as Android Studio gives it:
    /// `ExampleUnitTest`, `ExampleUnitTest.addition_isCorrect`.
    pub fn name(&self) -> String {
        config_name(&self.class, self.method.as_deref())
    }
}

pub fn config_name(class: &str, method: Option<&str>) -> String {
    let simple = class.rsplit('.').next().unwrap_or(class).replace('$', ".");
    match method {
        Some(m) => format!("{simple}.{m}"),
        None => simple,
    }
}

/// Annotations marking a test method (JUnit 4 and 5, kotlin.test).
const TEST_ANNOTATIONS: &[&str] = &["Test", "ParameterizedTest", "RepeatedTest", "TestFactory", "TestTemplate", "Theory"];

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Sym(char),
}

/// Identifiers (backquoted Kotlin names as one) and symbols, with their
/// 0-based lines; comments, strings and characters left out.
fn tokens(text: &str) -> Vec<(Tok, u32)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let (mut i, mut line) = (0usize, 0u32);
    let at = |i: usize| chars.get(i).copied().unwrap_or('\0');
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if c == '/' && at(i + 1) == '/' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && at(i + 1) == '*' {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && at(i + 1) == '/') {
                line += (chars[i] == '\n') as u32;
                i += 1;
            }
            i += 2;
        } else if c == '"' && at(i + 1) == '"' && at(i + 2) == '"' {
            // Kotlin raw strings, Java text blocks.
            i += 3;
            while i < chars.len() && !(chars[i] == '"' && at(i + 1) == '"' && at(i + 2) == '"') {
                line += (chars[i] == '\n') as u32;
                i += 1;
            }
            i += 3;
            while at(i) == '"' {
                i += 1;
            }
        } else if c == '"' || c == '\'' {
            i += 1;
            let mut template = 0usize;
            while i < chars.len() {
                let ch = chars[i];
                if ch == '\\' {
                    i += 2;
                    continue;
                }
                if ch == '\n' {
                    line += 1;
                    if template == 0 {
                        break;
                    }
                }
                if c == '"' && ch == '$' && at(i + 1) == '{' {
                    template += 1;
                    i += 2;
                    continue;
                }
                if template > 0 && ch == '{' {
                    template += 1;
                } else if template > 0 && ch == '}' {
                    template -= 1;
                } else if template == 0 && ch == c {
                    i += 1;
                    break;
                }
                i += 1;
            }
        } else if c == '`' {
            let start = i + 1;
            i = start;
            while i < chars.len() && chars[i] != '`' && chars[i] != '\n' {
                i += 1;
            }
            out.push((Tok::Ident(chars[start..i].iter().collect()), line));
            i += 1;
        } else if c.is_alphanumeric() || c == '_' || c == '$' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '$') {
                i += 1;
            }
            out.push((Tok::Ident(chars[start..i].iter().collect()), line));
        } else {
            out.push((Tok::Sym(c), line));
            i += 1;
        }
    }
    out
}

struct OpenClass {
    name: String,
    line: u32,
    /// The brace depth inside its body.
    depth: usize,
    runnable: bool,
    has_tests: bool,
    run_with: bool,
}

/// The test classes and methods of a Kotlin or Java file, in line order:
/// methods annotated `@Test` (and JUnit 5's other test annotations) in a
/// class that isn't abstract, and the classes holding them (or annotated
/// `@RunWith`), nested ones named `Outer$Inner`.
pub fn find_tests(text: &str) -> Vec<TestMark> {
    let toks = tokens(text);
    let ident = |i: usize| match toks.get(i) {
        Some((Tok::Ident(s), _)) => Some(s.as_str()),
        _ => None,
    };
    let sym = |i: usize| match toks.get(i) {
        Some((Tok::Sym(c), _)) => Some(*c),
        _ => None,
    };
    let mut package = String::new();
    let mut classes: Vec<OpenClass> = Vec::new();
    let mut marks: Vec<TestMark> = Vec::new();
    let (mut depth, mut parens) = (0usize, 0usize);
    // Since the last declaration: modifiers and annotations.
    let mut is_abstract = false;
    let mut test_annotation = false;
    let mut run_with = false;
    // A class whose body hasn't opened yet: name, line, runnable.
    let mut pending: Option<(String, u32, bool)> = None;
    // The test method just declared: its mark, and its body's depth once open.
    let mut method_body: Option<(usize, Option<usize>)> = None;
    let fqn = |classes: &[OpenClass], package: &str| {
        let names: Vec<&str> = classes.iter().map(|c| c.name.as_str()).collect();
        if package.is_empty() { names.join("$") } else { format!("{package}.{}", names.join("$")) }
    };
    let mut i = 0;
    while i < toks.len() {
        let line = toks[i].1;
        // An expression-bodied test method ends where the next declaration starts.
        if let Some((ix, None)) = method_body {
            let next_declaration = parens == 0
                && (matches!(&toks[i].0, Tok::Sym('@') | Tok::Sym(';') | Tok::Sym('}'))
                    || matches!(&toks[i].0, Tok::Ident(w) if matches!(w.as_str(), "fun" | "val" | "var" | "class" | "public" | "private" | "protected" | "internal")));
            if next_declaration && i > 0 && toks[i - 1].1 >= marks[ix].line {
                marks[ix].end = toks[i - 1].1;
                method_body = None;
            }
        }
        match &toks[i].0 {
            Tok::Sym('@') => {
                // An annotation: its (dotted) name, then any arguments.
                let mut j = i + 1;
                let mut last = "";
                while let Some(name) = ident(j) {
                    last = name;
                    if sym(j + 1) == Some('.') && ident(j + 2).is_some() {
                        j += 2;
                    } else if sym(j + 1) == Some(':') && sym(j + 2) != Some(':') && ident(j + 2).is_some() {
                        // Kotlin use-site targets: @get:Rule.
                        j += 2;
                    } else {
                        break;
                    }
                }
                if TEST_ANNOTATIONS.contains(&last) {
                    test_annotation = true;
                }
                if last == "RunWith" {
                    run_with = true;
                }
                j += 1;
                if sym(j) == Some('(') {
                    let mut nested = 0usize;
                    while j < toks.len() {
                        match sym(j) {
                            Some('(') => nested += 1,
                            Some(')') => {
                                nested -= 1;
                                if nested == 0 {
                                    j += 1;
                                    break;
                                }
                            }
                            _ => {}
                        }
                        j += 1;
                    }
                }
                i = j;
                continue;
            }
            Tok::Sym('(') => {
                if parens == 0 && test_annotation && pending.is_none() {
                    let name = i.checked_sub(1).and_then(ident).map(str::to_owned);
                    if let (Some(name), true) = (name, classes.last().is_some_and(|c| c.runnable)) {
                        let class = fqn(&classes, &package);
                        classes.last_mut().unwrap().has_tests = true;
                        let method_line = toks[i - 1].1;
                        marks.push(TestMark { line: method_line, end: method_line, class, method: Some(name) });
                        method_body = Some((marks.len() - 1, None));
                    }
                    test_annotation = false;
                }
                parens += 1;
            }
            Tok::Sym(')') => parens = parens.saturating_sub(1),
            Tok::Sym('{') if parens == 0 => {
                depth += 1;
                if let Some((ix, None)) = method_body {
                    method_body = Some((ix, Some(depth)));
                }
                if let Some((name, line, runnable)) = pending.take() {
                    classes.push(OpenClass { name, line, depth, runnable, has_tests: false, run_with });
                }
                (is_abstract, test_annotation, run_with) = (false, false, false);
            }
            Tok::Sym('}') if parens == 0 => {
                match method_body {
                    Some((ix, Some(d))) if d == depth => {
                        marks[ix].end = line;
                        method_body = None;
                    }
                    Some((ix, None)) => {
                        marks[ix].end = line;
                        method_body = None;
                    }
                    _ => {}
                }
                if classes.last().is_some_and(|c| c.depth == depth) {
                    let class = classes.pop().unwrap();
                    if class.runnable && (class.has_tests || class.run_with) {
                        let mut path: Vec<&str> = classes.iter().map(|c| c.name.as_str()).collect();
                        path.push(&class.name);
                        let name = if package.is_empty() { path.join("$") } else { format!("{package}.{}", path.join("$")) };
                        marks.push(TestMark { line: class.line, end: line, class: name, method: None });
                    }
                    if class.has_tests {
                        if let Some(parent) = classes.last_mut() {
                            parent.has_tests = true;
                        }
                    }
                }
                depth = depth.saturating_sub(1);
                pending = None;
                (is_abstract, test_annotation, run_with) = (false, false, false);
            }
            Tok::Sym(';') if parens == 0 => {
                pending = None;
                (is_abstract, test_annotation, run_with) = (false, false, false);
            }
            Tok::Ident(word) if parens == 0 => match word.as_str() {
                "package" if depth == 0 => {
                    let mut j = i + 1;
                    let mut parts = Vec::new();
                    while let Some(part) = ident(j) {
                        parts.push(part.to_owned());
                        if sym(j + 1) == Some('.') {
                            j += 2;
                        } else {
                            break;
                        }
                    }
                    package = parts.join(".");
                    i = j + 1;
                    continue;
                }
                "abstract" | "sealed" => is_abstract = true,
                "class" | "interface" | "object" | "enum" => {
                    // `Foo::class`, `Foo.class` are expressions.
                    let expression = matches!(i.checked_sub(1).and_then(sym), Some('.') | Some(':'));
                    if !expression {
                        if let Some(name) = ident(i + 1).filter(|n| *n != "class") {
                            let runnable = word == "class" && !is_abstract && classes.last().is_none_or(|c| c.runnable);
                            pending = Some((name.to_owned(), toks[i + 1].1, runnable));
                            i += 2;
                            continue;
                        } else if word == "object" {
                            // A companion or anonymous object.
                            pending = Some(("Companion".into(), line, false));
                        }
                    }
                }
                "fun" | "val" | "var" if pending.is_some() => pending = None,
                _ => {}
            },
            _ => {}
        }
        i += 1;
    }
    marks.sort_by(|a, b| a.line.cmp(&b.line).then(a.method.is_some().cmp(&b.method.is_some())));
    marks
}

/// The test at a line, as Run (Ctrl+Shift+F10) picks it: the method
/// whose declaration holds it, else the innermost class, else the file's
/// first class.
pub fn test_at(marks: &[TestMark], line: u32) -> Option<&TestMark> {
    let holds = |m: &&TestMark| m.line <= line && line <= m.end;
    marks
        .iter()
        .filter(holds)
        .filter(|m| m.method.is_some())
        .last()
        .or_else(|| marks.iter().filter(holds).max_by_key(|m| m.line))
        .or_else(|| marks.iter().find(|m| m.method.is_none()))
}

// --- Gradle --------------------------------------------------------------

/// What to run: a class, a method, a list of methods (Rerun Failed Tests),
/// or everything in the module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestRequest {
    pub kind: TestKind,
    /// `(class, method)` pairs; empty runs the whole module.
    pub filters: Vec<(String, Option<String>)>,
    /// The run configuration's name.
    pub name: String,
}

impl TestRequest {
    pub fn of(kind: TestKind, mark: &TestMark) -> Self {
        Self { kind, filters: vec![(mark.class.clone(), mark.method.clone())], name: mark.name() }
    }
}

/// The Gradle invocation running a request on a module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestPlan {
    pub build_root: PathBuf,
    /// Tasks and their options.
    pub args: Vec<String>,
    /// Where the task writes its JUnit XML reports.
    pub report_dir: PathBuf,
}

/// `:app:testDebugUnitTest --tests com.example.ExampleUnitTest.addition`
/// for unit tests (`:core:test` in a plain JVM module), and
/// `:app:connectedDebugAndroidTest` with the instrumentation runner's
/// `class` argument for the device's.
pub fn plan(build_root: &Path, module: &ModuleInfo, variant: &str, request: &TestRequest) -> TestPlan {
    let prefix = if module.path == ":" { String::new() } else { module.path.clone() };
    let mut args = Vec::new();
    let report_dir;
    match request.kind {
        TestKind::Unit => {
            let task = if module.kind.is_android() { format!("test{}UnitTest", capitalize(variant)) } else { "test".into() };
            args.push(format!("{prefix}:{task}"));
            for (class, method) in &request.filters {
                args.push("--tests".into());
                args.push(match method {
                    Some(m) => format!("{class}.{m}"),
                    None => class.clone(),
                });
            }
            report_dir = module.dir.join("build/test-results").join(task);
        }
        TestKind::Instrumented => {
            args.push(format!("{prefix}:connected{}AndroidTest", capitalize(variant)));
            if !request.filters.is_empty() {
                let classes: Vec<String> = request
                    .filters
                    .iter()
                    .map(|(class, method)| match method {
                        Some(m) => format!("{class}#{m}"),
                        None => class.clone(),
                    })
                    .collect();
                args.push(format!("-Pandroid.testInstrumentationRunnerArguments.class={}", classes.join(",")));
            }
            report_dir = module.dir.join("build/outputs/androidTest-results/connected");
        }
    }
    TestPlan { build_root: build_root.to_path_buf(), args, report_dir }
}

// --- Results ------------------------------------------------------------

/// A test's state, as the tree's icons show it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Outcome {
    Ignored,
    Passed,
    Running,
    /// An assertion failed.
    Failed,
    /// Something else was thrown.
    Error,
    /// Started, never finished: the run was stopped.
    Terminated,
}

impl Outcome {
    pub fn is_failure(self) -> bool {
        matches!(self, Outcome::Failed | Outcome::Error)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputLine {
    pub text: String,
    pub stderr: bool,
}

/// A failed assertion's two sides, for `<Click to see difference>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comparison {
    pub expected: String,
    pub actual: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestCase {
    /// As the framework reports it: `addition_isCorrect`, a JUnit 5
    /// display name, `[1] 2, 3`.
    pub name: String,
    /// The test's own name (`addition_isCorrect`, `twice(int)[1]`).
    pub id: String,
    pub outcome: Outcome,
    pub duration_ms: Option<u64>,
    pub output: Vec<OutputLine>,
    /// The exception's message and stack trace.
    pub failure: Option<String>,
    pub comparison: Option<Comparison>,
}

impl TestCase {
    /// The method the test runs (its name without JUnit 5's parameter
    /// list or invocation index): what `--tests` and the gutter go by.
    pub fn method(&self) -> &str {
        let end = self.id.find(['(', '[']).unwrap_or(self.id.len());
        self.id[..end].trim()
    }

    fn new(name: String, id: String) -> Self {
        Self { name, id, outcome: Outcome::Running, duration_ms: None, output: Vec::new(), failure: None, comparison: None }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestSuite {
    /// `com.example.ExampleUnitTest`.
    pub class: String,
    pub tests: Vec<TestCase>,
    /// What the class printed outside its tests.
    pub output: Vec<OutputLine>,
}

impl TestSuite {
    pub fn simple_name(&self) -> String {
        self.class.rsplit('.').next().unwrap_or(&self.class).to_owned()
    }

    /// The worst of its tests: failures first, then running, then passed;
    /// all ignored is ignored.
    pub fn outcome(&self) -> Outcome {
        self.tests.iter().map(|t| t.outcome).max().unwrap_or(Outcome::Passed)
    }

    pub fn duration_ms(&self) -> Option<u64> {
        let all: Vec<u64> = self.tests.iter().filter_map(|t| t.duration_ms).collect();
        (!all.is_empty()).then(|| all.iter().sum())
    }
}

/// What a test run reports as it goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TestEvent {
    /// `name` is what the tree shows, `id` the test's own name.
    Started { class: String, name: String, id: String },
    Finished { class: String, name: String, id: String, outcome: Outcome, duration_ms: u64, failure: Option<String>, comparison: Option<Comparison> },
    /// Printed by a test (`id`), or by the class around its tests.
    Output { class: String, id: Option<String>, line: OutputLine },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub passed: usize,
    pub failed: usize,
    pub ignored: usize,
    pub running: usize,
}

impl Counts {
    pub fn finished(&self) -> usize {
        self.passed + self.failed + self.ignored
    }
}

/// The results tree: test classes and their tests, in the order they ran.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TestResults {
    pub suites: Vec<TestSuite>,
}

impl TestResults {
    fn suite(&mut self, class: &str) -> &mut TestSuite {
        match self.suites.iter().position(|s| s.class == class) {
            Some(ix) => &mut self.suites[ix],
            None => {
                self.suites.push(TestSuite { class: class.to_owned(), tests: Vec::new(), output: Vec::new() });
                self.suites.last_mut().unwrap()
            }
        }
    }

    /// The test's latest row.
    fn case(&mut self, class: &str, id: &str, name: &str) -> &mut TestCase {
        let suite = self.suite(class);
        match suite.tests.iter().rposition(|t| t.id == id) {
            Some(ix) => &mut suite.tests[ix],
            None => {
                suite.tests.push(TestCase::new(name.to_owned(), id.to_owned()));
                suite.tests.last_mut().unwrap()
            }
        }
    }

    pub fn apply(&mut self, event: TestEvent) {
        match event {
            TestEvent::Started { class, name, id } => {
                // A test that runs again (repeated, retried) gets a new row.
                let suite = self.suite(&class);
                if suite.tests.iter().rposition(|t| t.id == id).is_none_or(|ix| suite.tests[ix].outcome != Outcome::Running) {
                    suite.tests.push(TestCase::new(name, id));
                }
            }
            TestEvent::Finished { class, name, id, outcome, duration_ms, failure, comparison } => {
                let case = self.case(&class, &id, &name);
                case.outcome = outcome;
                case.duration_ms = Some(duration_ms);
                case.comparison = comparison.or_else(|| failure.as_deref().and_then(comparison_in));
                case.failure = failure;
            }
            TestEvent::Output { class, id, line } => match id {
                Some(id) => self.case(&class, &id, &id).output.push(line),
                None => self.suite(&class).output.push(line),
            },
        }
    }

    /// The XML reports' results for classes the run didn't report as it
    /// went (instrumented tests report only there).
    pub fn merge(&mut self, reported: Vec<TestSuite>) {
        for suite in reported {
            match self.suites.iter_mut().find(|s| s.class == suite.class) {
                Some(mine) => {
                    for test in suite.tests {
                        if !mine.tests.iter().any(|t| t.id == test.id) {
                            mine.tests.push(test);
                        }
                    }
                }
                None => self.suites.push(suite),
            }
        }
    }

    /// Stopped: what still runs never finishes.
    pub fn terminate(&mut self) {
        for test in self.suites.iter_mut().flat_map(|s| s.tests.iter_mut()) {
            if test.outcome == Outcome::Running {
                test.outcome = Outcome::Terminated;
            }
        }
    }

    pub fn counts(&self) -> Counts {
        let mut c = Counts::default();
        for t in self.suites.iter().flat_map(|s| &s.tests) {
            match t.outcome {
                Outcome::Passed => c.passed += 1,
                Outcome::Failed | Outcome::Error => c.failed += 1,
                Outcome::Ignored => c.ignored += 1,
                Outcome::Running | Outcome::Terminated => c.running += 1,
            }
        }
        c
    }

    pub fn duration_ms(&self) -> Option<u64> {
        let all: Vec<u64> = self.suites.iter().filter_map(TestSuite::duration_ms).collect();
        (!all.is_empty()).then(|| all.iter().sum())
    }

    /// Rerun Failed Tests: each failed test's class and method, once.
    pub fn failed(&self) -> Vec<(String, Option<String>)> {
        let mut out: Vec<(String, Option<String>)> = Vec::new();
        for suite in &self.suites {
            for test in suite.tests.iter().filter(|t| t.outcome.is_failure()) {
                let filter = (suite.class.clone(), Some(test.method().to_owned()));
                if !out.contains(&filter) {
                    out.push(filter);
                }
            }
        }
        out
    }

    /// The outcome of a class, or one of its methods (all its runs), for
    /// the gutter.
    pub fn outcome_of(&self, class: &str, method: Option<&str>) -> Option<Outcome> {
        let suite = self.suites.iter().find(|s| s.class == class)?;
        match method {
            None => Some(suite.outcome()),
            Some(m) => suite.tests.iter().filter(|t| t.method() == m).map(|t| t.outcome).max(),
        }
    }
}

/// The two sides of a failed equality assertion, from its message:
/// JUnit 4 `expected:<4> but was:<5>`, JUnit 5 and kotlin.test
/// `expected: <4> but was: <5>`.
pub fn comparison_in(failure: &str) -> Option<Comparison> {
    let first = failure.lines().next()?;
    let start = first.find("expected:")?;
    let rest = &first[start + "expected:".len()..];
    let rest = rest.trim_start().strip_prefix('<')?;
    let split = rest.rfind("> but was:")?;
    let expected = &rest[..split];
    let actual = rest[split + "> but was:".len()..].trim_start().strip_prefix('<')?;
    let actual = actual.strip_suffix('>')?;
    // JUnit 4's ComparisonFailure brackets the part that differs.
    let unmark = |s: &str| if first.contains("ComparisonFailure") { s.replacen('[', "", 1).replacen(']', "", 1) } else { s.to_owned() };
    Some(Comparison { expected: unmark(expected), actual: unmark(actual) })
}

// --- Presentation ----------------------------------------------------------

/// A part of the status line, colored as IntelliJ's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Normal,
    Failed,
    Ignored,
}

/// IntelliJ's test status line: "3 tests passed" (failures in red,
/// ignored in grey), then "3 tests total, 1 sec 230 ms" in grey; while
/// running the count so far, and "Stopped." when stopped early.
pub fn status_line(counts: &Counts, finished: bool, duration_ms: Option<u64>) -> (Vec<(String, Tone)>, String) {
    let tests = |n: usize| format!("{n} {}", if n == 1 { "test" } else { "tests" });
    let done = counts.finished();
    let mut parts: Vec<(String, Tone)> = Vec::new();
    if done == 0 {
        return (parts, String::new());
    }
    let stopped = finished && counts.running > 0;
    if stopped {
        parts.push(("Stopped. ".into(), Tone::Normal));
    }
    let (failed, passed, ignored) = (counts.failed, counts.passed, counts.ignored);
    let f = |n| (format!("{} failed", tests(n)), Tone::Failed);
    let i = |n| (format!("{} ignored", tests(n)), Tone::Ignored);
    match (failed, passed, ignored) {
        (0, _, 0) => parts.push((format!("{} passed", tests(done)), Tone::Normal)),
        (_, 0, 0) => parts.push(f(failed)),
        (0, 0, _) => parts.push(i(ignored)),
        (_, _, 0) => parts.extend([(format!("{},", f(failed).0), Tone::Failed), (format!(" {passed} passed"), Tone::Normal)]),
        (_, 0, _) => parts.extend([(format!("{},", f(failed).0), Tone::Failed), (format!(" {ignored} ignored"), Tone::Ignored)]),
        (0, _, _) => parts.extend([(format!("{} passed, ", tests(passed)), Tone::Normal), (format!("{ignored} ignored"), Tone::Ignored)]),
        _ => parts.extend([(format!("{},", f(failed).0), Tone::Failed), (format!(" {passed} passed, "), Tone::Normal), (format!("{ignored} ignored"), Tone::Ignored)]),
    }
    let total = done + counts.running;
    let description = match (stopped, finished, duration_ms) {
        (true, _, Some(ms)) => format!("{done} / {}, {}", tests(total), duration_narrow(ms)),
        (true, _, None) => format!("{done} / {}", tests(total)),
        (false, true, Some(ms)) => format!("{} total, {}", tests(total), duration_narrow(ms)),
        _ => String::new(),
    };
    (parts, description)
}

/// IntelliJ's approximate narrow duration: two units at most, rounded,
/// a thin space before each unit ("25 ms", "1 sec 234 ms", "2 min 4 sec").
pub fn duration_narrow(ms: u64) -> String {
    const UNITS: [&str; 6] = ["ms", "sec", "min", "hr", "day", "week"];
    const MULTIPLIERS: [u64; 6] = [1, 1000, 60, 60, 24, 7];
    fn format(duration: u64) -> String {
        let mut values: Vec<(u64, usize)> = Vec::new();
        let mut count = duration;
        let mut i = 1;
        while i < UNITS.len() && count > 0 {
            let multiplier = MULTIPLIERS[i];
            if count < multiplier {
                break;
            }
            let remainder = count % multiplier;
            count /= multiplier;
            if remainder != 0 || !values.is_empty() {
                values.insert(0, (remainder, i - 1));
            }
            i += 1;
        }
        values.insert(0, (count, i - 1));
        if values.len() > 2 {
            let last_unit = values[1].1;
            let last_multiplier = MULTIPLIERS[last_unit];
            if values[2].0 > last_multiplier / 2 {
                let mut increment = last_multiplier - values[2].0;
                for unit in (1..last_unit).rev() {
                    increment *= MULTIPLIERS[unit];
                }
                return format(duration + increment);
            }
        }
        values.iter().take(2).map(|(v, u)| format!("{v}\u{2009}{}", UNITS[*u])).collect::<Vec<_>>().join(" ")
    }
    format(ms)
}

/// What a test run left.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TestRunReport {
    pub build: BuildResult,
    /// The JUnit XML reports written during the run.
    pub reported: Vec<TestSuite>,
}

/// Runs a test plan, reporting test events and Gradle's other output as
/// they come. Implemented over Gradle with an init script.
pub trait TestRunner {
    fn run_tests(&self, plan: &TestPlan, event: &mut dyn FnMut(TestEvent), line: &mut dyn FnMut(&str), cancelled: &dyn Fn() -> bool) -> TestRunReport;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::run::ModuleType;

    #[test]
    fn source_sets() {
        assert_eq!(test_kind("app/src/test/java/a/ATest.kt"), Some(TestKind::Unit));
        assert_eq!(test_kind("app/src/testDebug/java/a/ATest.kt"), Some(TestKind::Unit));
        assert_eq!(test_kind("C:\\p\\app\\src\\androidTest\\java\\A.kt"), Some(TestKind::Instrumented));
        assert_eq!(test_kind("app/src/main/java/a/Test.kt"), None);
        assert_eq!(test_kind("app/src/testing/A.kt"), None);
    }

    #[test]
    fn finds_kotlin_tests() {
        let text = r#"package com.example.calc

import org.junit.Assert.assertEquals
import org.junit.Test

/** Not a test: @Test fun nope() */
class ExampleUnitTest {
    private val label = "class Fake { @Test fun no() }"

    @Test
    fun addition_isCorrect() {
        assertEquals(4, 2 + 2)
    }

    @Test(expected = IllegalStateException::class)
    fun `throws on empty`() = Unit

    fun helper() {}

    @org.junit.jupiter.api.Nested
    inner class Inner {
        @ParameterizedTest @ValueSource(ints = [1, 2])
        fun <T> twice(x: Int) { val s = "${x}" }
    }
}

abstract class BaseTest {
    @Test fun inherited() {}
}
"#;
        let marks = find_tests(text);
        let got: Vec<(u32, String, Option<String>)> = marks.iter().map(|m| (m.line, m.class.clone(), m.method.clone())).collect();
        assert_eq!(
            got,
            vec![
                (6, "com.example.calc.ExampleUnitTest".into(), None),
                (10, "com.example.calc.ExampleUnitTest".into(), Some("addition_isCorrect".into())),
                (15, "com.example.calc.ExampleUnitTest".into(), Some("throws on empty".into())),
                (20, "com.example.calc.ExampleUnitTest$Inner".into(), None),
                (22, "com.example.calc.ExampleUnitTest$Inner".into(), Some("twice".into())),
            ]
        );
        assert_eq!(marks[1].name(), "ExampleUnitTest.addition_isCorrect");
        assert_eq!(marks[3].name(), "ExampleUnitTest.Inner");
        // The caret inside a method runs it; on the class line or between
        // methods, the class.
        assert_eq!(test_at(&marks, 12).unwrap().method.as_deref(), Some("addition_isCorrect"));
        assert_eq!(test_at(&marks, 15).unwrap().method.as_deref(), Some("throws on empty"));
        assert_eq!(test_at(&marks, 6).unwrap().method, None);
        assert_eq!(test_at(&marks, 13).unwrap().method, None);
        assert_eq!(test_at(&marks, 17).unwrap().method, None);
        assert_eq!(test_at(&marks, 22).unwrap().class, "com.example.calc.ExampleUnitTest$Inner");
        assert_eq!((marks[1].end, marks[2].end, marks[0].end), (12, 15, 24));
    }

    #[test]
    fn finds_java_tests() {
        let text = "package a.b;\n\nimport org.junit.Test;\n\n@RunWith(AndroidJUnit4.class)\npublic class MainTest {\n  @Test(timeout = 10)\n  public void opens() throws Exception {\n    Class<?> c = String.class;\n  }\n  @Ignore @Test public void later() {}\n  private void helper() {}\n}\n";
        let got: Vec<(u32, Option<String>)> = find_tests(text).into_iter().map(|m| (m.line, m.method)).collect();
        assert_eq!(got, vec![(5, None), (7, Some("opens".into())), (10, Some("later".into()))]);
    }

    #[test]
    fn plans_gradle_runs() {
        let app = ModuleInfo { path: ":app".into(), dir: "/p/app".into(), kind: ModuleType::Application, variants: vec!["demoDebug".into()] };
        let mark = TestMark { line: 0, end: 0, class: "a.ExampleTest".into(), method: Some("adds".into()) };
        let p = plan(Path::new("/p"), &app, "demoDebug", &TestRequest::of(TestKind::Unit, &mark));
        assert_eq!(p.args, [":app:testDemoDebugUnitTest", "--tests", "a.ExampleTest.adds"]);
        assert_eq!(p.report_dir, PathBuf::from("/p/app/build/test-results/testDemoDebugUnitTest"));
        let request = TestRequest { kind: TestKind::Instrumented, filters: vec![("a.ATest".into(), None), ("a.BTest".into(), Some("b".into()))], name: "x".into() };
        let p = plan(Path::new("/p"), &app, "demoDebug", &request);
        assert_eq!(p.args, [":app:connectedDemoDebugAndroidTest", "-Pandroid.testInstrumentationRunnerArguments.class=a.ATest,a.BTest#b"]);
        let core = ModuleInfo { path: ":".into(), dir: "/p".into(), kind: ModuleType::Jvm, variants: vec![] };
        let all = TestRequest { kind: TestKind::Unit, filters: vec![], name: "All".into() };
        assert_eq!(plan(Path::new("/p"), &core, "debug", &all).args, [":test"]);
    }

    fn finished(class: &str, name: &str, outcome: Outcome, ms: u64, failure: Option<&str>) -> TestEvent {
        TestEvent::Finished { class: class.into(), name: name.into(), id: name.into(), outcome, duration_ms: ms, failure: failure.map(Into::into), comparison: None }
    }

    fn started(class: &str, name: &str) -> TestEvent {
        TestEvent::Started { class: class.into(), name: name.into(), id: name.into() }
    }

    #[test]
    fn builds_the_results_tree() {
        let mut r = TestResults::default();
        r.apply(started("a.ATest", "one"));
        r.apply(TestEvent::Output { class: "a.ATest".into(), id: Some("one".into()), line: OutputLine { text: "hello".into(), stderr: false } });
        assert_eq!(r.counts(), Counts { running: 1, ..Default::default() });
        r.apply(finished("a.ATest", "one", Outcome::Passed, 12, None));
        r.apply(started("a.ATest", "two(int)[1]"));
        r.apply(finished("a.ATest", "two(int)[1]", Outcome::Failed, 3, Some("java.lang.AssertionError: expected:<4> but was:<5>\n\tat a.ATest.two(ATest.kt:9)")));
        r.apply(finished("a.BTest", "skip", Outcome::Ignored, 0, None));
        assert_eq!(r.counts(), Counts { passed: 1, failed: 1, ignored: 1, running: 0 });
        assert_eq!(r.suites[0].tests[0].output[0].text, "hello");
        assert_eq!(r.suites[0].outcome(), Outcome::Failed);
        assert_eq!(r.suites[0].duration_ms(), Some(15));
        assert_eq!(r.suites[0].tests[1].comparison, Some(Comparison { expected: "4".into(), actual: "5".into() }));
        assert_eq!(r.failed(), vec![("a.ATest".to_string(), Some("two".to_string()))]);
        assert_eq!(r.outcome_of("a.ATest", Some("two")), Some(Outcome::Failed));
        assert_eq!(r.outcome_of("a.ATest", Some("one")), Some(Outcome::Passed));
        assert_eq!(r.outcome_of("a.BTest", None), Some(Outcome::Ignored));
        // The XML reports add what the events missed, nothing twice.
        let one = r.suites[0].tests[0].clone();
        let extra = TestCase { name: "three".into(), id: "three".into(), ..one.clone() };
        r.merge(vec![TestSuite { class: "a.ATest".into(), tests: vec![one, extra], output: vec![] }]);
        assert_eq!(r.suites[0].tests.len(), 3);
    }

    #[test]
    fn comparisons() {
        let c = comparison_in("org.opentest4j.AssertionFailedError: expected: <hello> but was: <help>").unwrap();
        assert_eq!((c.expected.as_str(), c.actual.as_str()), ("hello", "help"));
        let c = comparison_in("org.junit.ComparisonFailure: expected:<hel[lo]> but was:<hel[p]>").unwrap();
        assert_eq!((c.expected.as_str(), c.actual.as_str()), ("hello", "help"));
        assert!(comparison_in("java.lang.IllegalStateException: boom").is_none());
    }

    #[test]
    fn status_lines_and_durations() {
        let c = |passed, failed, ignored, running| Counts { passed, failed, ignored, running };
        let text = |parts: Vec<(String, Tone)>| parts.into_iter().map(|(t, _)| t).collect::<String>();
        let (parts, desc) = status_line(&c(3, 0, 0, 0), true, Some(25));
        assert_eq!((text(parts), desc.as_str()), ("3 tests passed".into(), "3 tests total, 25\u{2009}ms"));
        let (parts, _) = status_line(&c(2, 1, 0, 0), true, Some(25));
        assert_eq!(parts[0], ("1 test failed,".into(), Tone::Failed));
        assert_eq!(text(parts), "1 test failed, 2 passed");
        assert_eq!(text(status_line(&c(1, 1, 1, 0), true, None).0), "1 test failed, 1 passed, 1 ignored");
        assert_eq!(text(status_line(&c(2, 0, 1, 0), true, None).0), "2 tests passed, 1 ignored");
        let (parts, desc) = status_line(&c(1, 0, 0, 2), true, Some(1500));
        assert_eq!((text(parts), desc.as_str()), ("Stopped. 1 test passed".into(), "1 / 3 tests, 1\u{2009}sec 500\u{2009}ms"));
        // Running: the count so far, no total yet.
        assert_eq!(status_line(&c(1, 0, 0, 1), false, Some(10)).1, "");
        assert_eq!(duration_narrow(0), "0\u{2009}ms");
        assert_eq!(duration_narrow(1234), "1\u{2009}sec 234\u{2009}ms");
        assert_eq!(duration_narrow(2000), "2\u{2009}sec");
        assert_eq!(duration_narrow(123_456), "2\u{2009}min 3\u{2009}sec");
        assert_eq!(duration_narrow(123_600), "2\u{2009}min 4\u{2009}sec");
    }
}
