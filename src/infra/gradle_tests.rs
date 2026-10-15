//! The test runner port over Gradle: an init script makes every `Test`
//! task print a line per test event (started, finished, output) as JSON,
//! which is turned into the domain's test events as Gradle runs; then the
//! JUnit XML reports the run wrote fill in what the events missed
//! (instrumented tests report only there).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Deserialize;

use super::gradle;
use crate::domain::run::BuildResult;
use crate::domain::testing::{comparison_in, Comparison, Outcome, OutputLine, TestCase, TestEvent, TestPlan, TestRunReport, TestRunner, TestSuite};
use crate::index::libraries::cache_dir;

/// The prefix of the init script's lines.
const PREFIX: &str = "JUNCTION_TEST ";

const INIT_SCRIPT: &str = r#"// Written by Junction for running tests.
import groovy.json.JsonOutput

def junctionEmit = { Map m -> println("JUNCTION_TEST " + JsonOutput.toJson(m)) }
def junctionName = { d ->
    try { return d.displayName ?: d.name } catch (Throwable ignored) { return d.name }
}

allprojects {
    tasks.withType(Test).configureEach { t ->
        // Run again even when nothing changed, as Android Studio does.
        t.outputs.upToDateWhen { false }
        // Debug: one test JVM, waiting for Junction's debugger.
        def junctionDebugPort = t.project.findProperty("junction.debugPort")
        if (junctionDebugPort) {
            t.maxParallelForks = 1
            t.jvmArgs("-agentlib:jdwp=transport=dt_socket,server=y,suspend=y,address=127.0.0.1:" + junctionDebugPort)
        }
        t.beforeTest { d ->
            junctionEmit(["event": "started", "class": d.className, "name": junctionName(d), "id": d.name, "composite": d.composite])
        }
        t.afterTest { d, r ->
            def m = ["event": "finished", "class": d.className, "name": junctionName(d), "id": d.name, "composite": d.composite,
                     "result": r.resultType.name(), "startTime": r.startTime, "endTime": r.endTime]
            def e = r.exception
            if (e != null) {
                def sw = new StringWriter()
                e.printStackTrace(new PrintWriter(sw))
                m["failure"] = sw.toString()
                m["assertion"] = e instanceof AssertionError
            }
            try {
                def f = r.failures.find { it.details != null }
                if (f != null) {
                    if (f.details.assertionFailure) m["assertion"] = true
                    if (f.details.expected != null) m["expected"] = f.details.expected
                    if (f.details.actual != null) m["actual"] = f.details.actual
                }
            } catch (Throwable ignored) {
                // Failure details came with Gradle 7.6.
            }
            junctionEmit(m)
        }
        t.onOutput { d, o ->
            junctionEmit(["event": "output", "class": d.className, "id": d.name, "composite": d.composite,
                          "stderr": o.destination.name() == "StdErr", "text": o.message])
        }
    }
}
"#;

pub struct GradleTests;

impl TestRunner for GradleTests {
    fn run_tests(&self, plan: &TestPlan, event: &mut dyn FnMut(TestEvent), line: &mut dyn FnMut(&str), cancelled: &dyn Fn() -> bool) -> TestRunReport {
        let started = SystemTime::now();
        let Some(script) = init_script() else {
            let message = "Could not write the test script to Junction's cache.";
            line(message);
            return TestRunReport { build: BuildResult::default(), reported: Vec::new() };
        };
        let mut command = gradle::gradle(&plan.build_root);
        command
            .arg("--init-script")
            .arg(&script)
            // The init script's listeners can't be cached.
            .arg("-Dorg.gradle.configuration-cache=false")
            .args(&plan.args);
        let mut dispatch = |l: &str| match parse_line(l) {
            Parsed::Events(events) => events.into_iter().for_each(&mut *event),
            Parsed::Lines(lines) => lines.iter().for_each(|l| line(l.as_str())),
        };
        let (succeeded, was_cancelled, output) = gradle::stream(command, &mut dispatch, cancelled);
        let since = started.checked_sub(Duration::from_secs(2)).unwrap_or(started);
        let reported = report_files(&plan.report_dir, since).iter().filter_map(|f| std::fs::read_to_string(f).ok()).flat_map(|xml| parse_report(&xml)).collect();
        TestRunReport { build: BuildResult { succeeded, cancelled: was_cancelled, problems: gradle::parse_output(&output) }, reported }
    }
}

fn init_script() -> Option<PathBuf> {
    let path = cache_dir()?.join("gradle-tests").join("junction-tests.gradle");
    if std::fs::read_to_string(&path).ok().as_deref() != Some(INIT_SCRIPT) {
        std::fs::create_dir_all(path.parent()?).ok()?;
        std::fs::write(&path, INIT_SCRIPT).ok()?;
    }
    Some(path)
}

// --- Event lines -------------------------------------------------------------

/// A line of the init script, as it prints it.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct RawEvent {
    event: String,
    class: Option<String>,
    name: Option<String>,
    id: Option<String>,
    composite: bool,
    result: Option<String>,
    start_time: Option<u64>,
    end_time: Option<u64>,
    failure: Option<String>,
    assertion: bool,
    expected: Option<String>,
    actual: Option<String>,
    stderr: bool,
    text: Option<String>,
}

/// What a line of Gradle's output carries.
#[derive(Debug, PartialEq)]
enum Parsed {
    Events(Vec<TestEvent>),
    /// Lines for the Gradle console (none for a line nobody shows).
    Lines(Vec<String>),
}

fn parse_line(line: &str) -> Parsed {
    let Some(json) = line.strip_prefix(PREFIX) else {
        return Parsed::Lines(vec![line.to_owned()]);
    };
    let Ok(raw) = serde_json::from_str::<RawEvent>(json) else {
        return Parsed::Lines(Vec::new());
    };
    let class = raw.class.filter(|c| !c.is_empty());
    let id = raw.id.unwrap_or_default();
    let name = raw.name.filter(|n| !n.is_empty()).unwrap_or_else(|| id.clone());
    match raw.event.as_str() {
        "output" => {
            let text = raw.text.unwrap_or_default();
            let lines = text.lines().map(str::to_owned);
            match class {
                // Printed by the class around its tests.
                Some(class) => {
                    let id = (!raw.composite).then_some(id);
                    Parsed::Events(lines.map(|text| TestEvent::Output { class: class.clone(), id: id.clone(), line: OutputLine { text, stderr: raw.stderr } }).collect())
                }
                // Printed by the test executor itself.
                None => Parsed::Lines(lines.collect()),
            }
        }
        _ if raw.composite => Parsed::Events(Vec::new()),
        "started" => Parsed::Events(class.map(|class| TestEvent::Started { class, name, id }).into_iter().collect()),
        "finished" => {
            let Some(class) = class else { return Parsed::Events(Vec::new()) };
            let outcome = match raw.result.as_deref() {
                Some("SUCCESS") => Outcome::Passed,
                Some("SKIPPED") => Outcome::Ignored,
                _ if raw.assertion => Outcome::Failed,
                _ => Outcome::Error,
            };
            let duration_ms = match (raw.start_time, raw.end_time) {
                (Some(start), Some(end)) => end.saturating_sub(start),
                _ => 0,
            };
            let comparison = match (raw.expected, raw.actual) {
                (Some(expected), Some(actual)) => Some(Comparison { expected, actual }),
                _ => None,
            };
            let failure = raw.failure.filter(|_| outcome.is_failure());
            Parsed::Events(vec![TestEvent::Finished { class, name, id, outcome, duration_ms, failure, comparison }])
        }
        _ => Parsed::Events(Vec::new()),
    }
}

// --- JUnit XML reports ---------------------------------------------------------

/// The XML files under a report directory written at or after `since`.
fn report_files(dir: &Path, since: SystemTime) -> Vec<PathBuf> {
    fn walk(dir: &Path, since: SystemTime, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, since, out);
            } else if path.extension().is_some_and(|e| e == "xml") && entry.metadata().and_then(|m| m.modified()).is_ok_and(|t| t >= since) {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, since, &mut out);
    out.sort();
    out
}

/// The test classes of a JUnit XML report (`<testsuite>`, or several in
/// `<testsuites>`), grouped by each test's class.
fn parse_report(xml: &str) -> Vec<TestSuite> {
    let Ok(doc) = roxmltree::Document::parse(xml) else { return Vec::new() };
    let mut suites: Vec<TestSuite> = Vec::new();
    fn suite<'a>(suites: &'a mut Vec<TestSuite>, class: &str) -> &'a mut TestSuite {
        match suites.iter().position(|s| s.class == class) {
            Some(ix) => &mut suites[ix],
            None => {
                suites.push(TestSuite { class: class.to_owned(), tests: Vec::new(), output: Vec::new() });
                suites.last_mut().unwrap()
            }
        }
    }
    let output_of = |node: roxmltree::Node| -> Vec<OutputLine> {
        let mut out = Vec::new();
        for child in node.children().filter(|c| c.is_element()) {
            let stderr = match child.tag_name().name() {
                "system-out" => false,
                "system-err" => true,
                _ => continue,
            };
            out.extend(child.text().unwrap_or_default().lines().map(|text| OutputLine { text: text.to_owned(), stderr }));
        }
        out
    };
    for testsuite in doc.descendants().filter(|n| n.has_tag_name("testsuite")) {
        let suite_name = testsuite.attribute("name").unwrap_or_default();
        let mut classes: Vec<String> = Vec::new();
        for case in testsuite.children().filter(|n| n.has_tag_name("testcase")) {
            let class = case.attribute("classname").filter(|c| !c.is_empty()).unwrap_or(suite_name).to_owned();
            let test = parse_case(case, output_of(case));
            suite(&mut suites, &class).tests.push(test);
            if !classes.contains(&class) {
                classes.push(class);
            }
        }
        let output = output_of(testsuite);
        let owner = classes.iter().find(|c| *c == suite_name).or(classes.first());
        if let (Some(owner), false) = (owner, output.is_empty()) {
            suite(&mut suites, owner).output.extend(output);
        }
    }
    suites
}

fn parse_case(case: roxmltree::Node, output: Vec<OutputLine>) -> TestCase {
    let name = case.attribute("name").unwrap_or_default().to_owned();
    let duration_ms = case.attribute("time").and_then(|t| t.parse::<f64>().ok()).map(|s| (s * 1000.0).round() as u64);
    let mut outcome = Outcome::Passed;
    let mut failure = None;
    for child in case.children().filter(|c| c.is_element()) {
        let kind = match child.tag_name().name() {
            "failure" => Outcome::Failed,
            "error" => Outcome::Error,
            "skipped" => {
                // A failure in the same case wins.
                if outcome == Outcome::Passed {
                    outcome = Outcome::Ignored;
                }
                continue;
            }
            _ => continue,
        };
        outcome = kind;
        let message = child.attribute("message").unwrap_or_default().trim();
        let text = child.text().unwrap_or_default().trim();
        failure = Some(match (message.is_empty(), text.is_empty()) {
            (_, true) => message.to_owned(),
            (false, false) if !text.contains(message) => format!("{message}\n{text}"),
            _ => text.to_owned(),
        });
    }
    let comparison = failure.as_deref().and_then(comparison_in);
    TestCase { id: name.clone(), name, outcome, duration_ms, output, failure, comparison }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(line: &str) -> Vec<TestEvent> {
        match parse_line(line) {
            Parsed::Events(e) => e,
            other => panic!("not events: {other:?}"),
        }
    }

    #[test]
    fn reads_event_lines() {
        assert_eq!(parse_line("> Task :app:testDebugUnitTest"), Parsed::Lines(vec!["> Task :app:testDebugUnitTest".into()]));
        assert_eq!(parse_line("JUNCTION_TEST {not json"), Parsed::Lines(vec![]));
        let started = events(r#"JUNCTION_TEST {"event":"started","class":"a.ATest","name":"adds two","id":"adds()","composite":false}"#);
        assert_eq!(started, vec![TestEvent::Started { class: "a.ATest".into(), name: "adds two".into(), id: "adds()".into() }]);
        // Executors and classes start too; the tree doesn't show them.
        assert!(events(r#"JUNCTION_TEST {"event":"started","class":"a.ATest","name":"a.ATest","id":"a.ATest","composite":true}"#).is_empty());
        let passed = events(r#"JUNCTION_TEST {"event":"finished","class":"a.ATest","name":"adds","id":"adds","composite":false,"result":"SUCCESS","startTime":1000,"endTime":1012}"#);
        assert_eq!(passed, vec![TestEvent::Finished { class: "a.ATest".into(), name: "adds".into(), id: "adds".into(), outcome: Outcome::Passed, duration_ms: 12, failure: None, comparison: None }]);
        let skipped = events(r#"JUNCTION_TEST {"event":"finished","class":"a.ATest","name":"later","id":"later","result":"SKIPPED","startTime":5,"endTime":5}"#);
        assert!(matches!(&skipped[0], TestEvent::Finished { outcome: Outcome::Ignored, .. }));
    }

    #[test]
    fn maps_failures() {
        let failed = events(
            r#"JUNCTION_TEST {"event":"finished","class":"a.ATest","name":"adds","id":"adds","result":"FAILURE","startTime":1,"endTime":4,"failure":"java.lang.AssertionError: expected:<4> but was:<5>\n\tat a.ATest.adds(ATest.kt:9)\n","assertion":true,"expected":"4","actual":"5"}"#,
        );
        let TestEvent::Finished { outcome, duration_ms, failure, comparison, .. } = &failed[0] else { panic!() };
        assert_eq!((*outcome, *duration_ms), (Outcome::Failed, 3));
        assert!(failure.as_deref().unwrap().starts_with("java.lang.AssertionError: expected:<4>"));
        assert_eq!(comparison, &Some(Comparison { expected: "4".into(), actual: "5".into() }));
        let error = events(r#"JUNCTION_TEST {"event":"finished","class":"a.ATest","name":"boom","id":"boom","result":"FAILURE","failure":"java.lang.IllegalStateException: boom","assertion":false}"#);
        let TestEvent::Finished { outcome, comparison, failure, .. } = &error[0] else { panic!() };
        assert_eq!((*outcome, comparison), (Outcome::Error, &None));
        assert_eq!(failure.as_deref(), Some("java.lang.IllegalStateException: boom"));
    }

    #[test]
    fn splits_output() {
        let out = events(r#"JUNCTION_TEST {"event":"output","class":"a.ATest","id":"adds","composite":false,"stderr":true,"text":"one\r\ntwo\n"}"#);
        assert_eq!(
            out,
            vec![
                TestEvent::Output { class: "a.ATest".into(), id: Some("adds".into()), line: OutputLine { text: "one".into(), stderr: true } },
                TestEvent::Output { class: "a.ATest".into(), id: Some("adds".into()), line: OutputLine { text: "two".into(), stderr: true } },
            ]
        );
        // The class's own output (@BeforeClass), and the executor's.
        let class = events(r#"JUNCTION_TEST {"event":"output","class":"a.ATest","id":"a.ATest","composite":true,"stderr":false,"text":"setup\n"}"#);
        assert_eq!(class, vec![TestEvent::Output { class: "a.ATest".into(), id: None, line: OutputLine { text: "setup".into(), stderr: false } }]);
        let executor = parse_line(r#"JUNCTION_TEST {"event":"output","class":null,"id":"Gradle Test Executor 1","composite":true,"text":"warning: x\nmore\n"}"#);
        assert_eq!(executor, Parsed::Lines(vec!["warning: x".into(), "more".into()]));
    }

    #[test]
    fn reads_xml_reports() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuite name="a.ATest" tests="4" skipped="1" failures="1" errors="1" timestamp="2026-10-15T10:00:00" hostname="h" time="0.05">
  <properties/>
  <testcase name="adds" classname="a.ATest" time="0.012"/>
  <testcase name="compares" classname="a.ATest" time="0.003">
    <failure message="org.junit.ComparisonFailure: expected:&lt;hel[lo]&gt; but was:&lt;hel[p]&gt;" type="org.junit.ComparisonFailure">org.junit.ComparisonFailure: expected:&lt;hel[lo]&gt; but was:&lt;hel[p]&gt;
	at a.ATest.compares(ATest.kt:12)</failure>
  </testcase>
  <testcase name="throws" classname="a.ATest" time="0.001">
    <error message="boom" type="java.lang.IllegalStateException"/>
  </testcase>
  <testcase name="later" classname="a.ATest$Inner" time="0.0">
    <skipped/>
  </testcase>
  <system-out><![CDATA[hello
world
]]></system-out>
  <system-err><![CDATA[]]></system-err>
</testsuite>"#;
        let suites = parse_report(xml);
        assert_eq!(suites.len(), 2);
        let a = &suites[0];
        assert_eq!(a.class, "a.ATest");
        assert_eq!(a.tests.iter().map(|t| (t.name.as_str(), t.outcome, t.duration_ms)).collect::<Vec<_>>(), [
            ("adds", Outcome::Passed, Some(12)),
            ("compares", Outcome::Failed, Some(3)),
            ("throws", Outcome::Error, Some(1)),
        ]);
        assert!(a.tests[1].failure.as_deref().unwrap().contains("at a.ATest.compares"));
        assert_eq!(a.tests[1].comparison, Some(Comparison { expected: "hello".into(), actual: "help".into() }));
        assert_eq!(a.tests[2].failure.as_deref(), Some("boom"));
        assert_eq!(a.output.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["hello", "world"]);
        assert_eq!((suites[1].class.as_str(), suites[1].tests[0].outcome), ("a.ATest$Inner", Outcome::Ignored));
        assert!(parse_report("<not xml").is_empty());
        let several = "<testsuites><testsuite name=\"x\"><testcase name=\"t\" classname=\"b.BTest\"/></testsuite></testsuites>";
        assert_eq!(parse_report(several)[0].class, "b.BTest");
    }

    #[test]
    fn finds_fresh_reports() {
        let dir = std::env::temp_dir().join(format!("junction-test-reports-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::write(dir.join("nested/TEST-a.ATest.xml"), "<testsuite/>").unwrap();
        std::fs::write(dir.join("binary.bin"), "").unwrap();
        let before = SystemTime::now() - Duration::from_secs(60);
        assert_eq!(report_files(&dir, before), vec![dir.join("nested/TEST-a.ATest.xml")]);
        assert!(report_files(&dir, SystemTime::now() + Duration::from_secs(60)).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
