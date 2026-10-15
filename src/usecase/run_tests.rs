//! Run tests, as Android Studio runs a Gradle test configuration: runs the
//! plan's tasks, building the results tree as tests start and finish, then
//! adds what only the JUnit XML reports tell (instrumented tests). Gradle's
//! other output goes to the console.

use crate::domain::run::BuildResult;
use crate::domain::testing::{TestEvent, TestPlan, TestResults, TestRunner};

/// Runs a test plan. `on_event` sees each event once it is in the tree,
/// `line` gets Gradle's other output. A stopped run leaves the tests that
/// were still running terminated.
pub fn run(runner: &dyn TestRunner, plan: &TestPlan, on_event: &mut dyn FnMut(&TestEvent), line: &mut dyn FnMut(&str), cancelled: &dyn Fn() -> bool) -> (TestResults, BuildResult) {
    let mut results = TestResults::default();
    let report = runner.run_tests(
        plan,
        &mut |event| {
            results.apply(event.clone());
            on_event(&event);
        },
        line,
        cancelled,
    );
    results.merge(report.reported);
    if report.build.cancelled || cancelled() {
        results.terminate();
    }
    (results, report.build)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::testing::{Counts, Outcome, OutputLine, TestCase, TestRunReport, TestSuite};
    use std::cell::RefCell;
    use std::path::PathBuf;

    /// Replays events, then reports what the XML reports hold.
    struct Gradle {
        events: Vec<TestEvent>,
        reported: Vec<TestSuite>,
        cancelled: bool,
        args: RefCell<Vec<String>>,
    }
    impl TestRunner for Gradle {
        fn run_tests(&self, plan: &TestPlan, event: &mut dyn FnMut(TestEvent), line: &mut dyn FnMut(&str), _: &dyn Fn() -> bool) -> TestRunReport {
            self.args.borrow_mut().extend(plan.args.iter().cloned());
            line("> Task :app:testDebugUnitTest");
            self.events.iter().cloned().for_each(event);
            let build = BuildResult { succeeded: !self.cancelled, cancelled: self.cancelled, problems: Vec::new() };
            TestRunReport { build, reported: self.reported.clone() }
        }
    }

    fn plan() -> TestPlan {
        TestPlan { build_root: PathBuf::from("/p"), args: vec![":app:testDebugUnitTest".into()], report_dir: PathBuf::from("/p/app/build/test-results/testDebugUnitTest") }
    }

    fn started(name: &str) -> TestEvent {
        TestEvent::Started { class: "a.ATest".into(), name: name.into(), id: name.into() }
    }

    fn passed(name: &str) -> TestEvent {
        TestEvent::Finished { class: "a.ATest".into(), name: name.into(), id: name.into(), outcome: Outcome::Passed, duration_ms: 5, failure: None, comparison: None }
    }

    #[test]
    fn builds_the_tree_as_tests_run() {
        let output = TestEvent::Output { class: "a.ATest".into(), id: Some("one".into()), line: OutputLine { text: "hi".into(), stderr: false } };
        let reported = TestSuite {
            class: "a.ATest".into(),
            tests: vec![TestCase { name: "one".into(), id: "one".into(), outcome: Outcome::Passed, duration_ms: Some(5), output: vec![], failure: None, comparison: None }, TestCase {
                name: "two".into(),
                id: "two".into(),
                outcome: Outcome::Failed,
                duration_ms: Some(1),
                output: vec![],
                failure: Some("boom".into()),
                comparison: None,
            }],
            output: vec![],
        };
        let gradle = Gradle { events: vec![started("one"), output, passed("one")], reported: vec![reported], cancelled: false, args: RefCell::default() };
        let (mut seen, mut lines) = (Vec::new(), Vec::new());
        let (results, build) = run(&gradle, &plan(), &mut |e| seen.push(e.clone()), &mut |l| lines.push(l.to_owned()), &|| false);
        assert!(build.succeeded);
        assert_eq!(gradle.args.borrow().as_slice(), [":app:testDebugUnitTest"]);
        assert_eq!(seen.len(), 3);
        assert_eq!(lines, ["> Task :app:testDebugUnitTest"]);
        // The report adds the test the events missed, not the one they had.
        assert_eq!(results.counts(), Counts { passed: 1, failed: 1, ..Default::default() });
        assert_eq!(results.suites[0].tests[0].output[0].text, "hi");
    }

    #[test]
    fn a_stopped_run_terminates_what_still_runs() {
        let gradle = Gradle { events: vec![started("one"), passed("one"), started("two")], reported: vec![], cancelled: true, args: RefCell::default() };
        let (results, build) = run(&gradle, &plan(), &mut |_| {}, &mut |_| {}, &|| true);
        assert!(build.cancelled);
        assert_eq!(results.outcome_of("a.ATest", Some("two")), Some(Outcome::Terminated));
        assert_eq!(results.counts(), Counts { passed: 1, running: 1, ..Default::default() });
    }
}
