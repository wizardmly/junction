//! The test runner tab of the Run window, as IntelliJ's: Rerun, Rerun
//! Failed Tests and Stop, Show Passed / Show Ignored and the sort toggles,
//! the status line ("1 test failed, 2 passed  3 tests total, 25 ms"), the
//! results tree (Test Results › classes › tests, with outcome icons and
//! durations) and the console of the selected node: the test's output and
//! failure, a class's tests, or all of Gradle's output for the root.
//! Double-clicking a test goes to its source.

use std::collections::HashSet;
use std::ops::Range;

use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, ClickEvent, Context, Entity, EventEmitter, FontWeight, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, UniformListScrollHandle, Window, div, prelude::FluentBuilder as _, px, uniform_list,
};

use crate::domain::testing::{self, Outcome, TestCase, TestResults, TestSuite, Tone};
use crate::theme::ActivePalette as _;
use crate::ui::as_icons::{self as icons, AsIcon};
use crate::ui::common::tool_button;
use crate::ui::run_views::{RunViewEvent, stop_label};
use crate::ui::test_service::{TestRun, TestService};

const ROW: f32 = 22.;
const LINE: f32 = 18.;

/// A node of the results tree.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Node {
    Root,
    /// A test class, by name.
    Suite(String),
    /// A test of a class: its index there.
    Test(String, usize),
}

struct Row {
    node: Node,
    depth: usize,
    label: SharedString,
    outcome: Option<Outcome>,
    duration_ms: Option<u64>,
    /// A class row: whether it's collapsed.
    collapsed: Option<bool>,
}

#[derive(Clone)]
struct ConsoleLine {
    text: SharedString,
    color: Hsla,
}

pub struct TestView {
    service: Entity<TestService>,
    selected: Node,
    collapsed: HashSet<String>,
    show_passed: bool,
    show_ignored: bool,
    sort_alphabetically: bool,
    sort_by_duration: bool,
    tree_scroll: UniformListScrollHandle,
    console_scroll: UniformListScrollHandle,
    /// The console as last rendered, for its list's rows.
    console: Vec<ConsoleLine>,
    shown_lines: usize,
    rows: Vec<Row>,
    _observe: Subscription,
}

impl EventEmitter<RunViewEvent> for TestView {}

/// IntelliJ's tree icon for an outcome.
fn outcome_icon(outcome: Option<Outcome>) -> AsIcon {
    match outcome {
        None => icons::TEST_NOT_RUN,
        Some(Outcome::Passed) => icons::TEST_PASSED,
        Some(Outcome::Failed) => icons::TEST_FAILED,
        Some(Outcome::Error) => icons::TEST_ERROR,
        Some(Outcome::Ignored) => icons::TEST_IGNORED,
        Some(Outcome::Running) => icons::TEST_PAUSED,
        Some(Outcome::Terminated) => icons::TEST_TERMINATED,
    }
}

impl TestView {
    pub fn new(service: Entity<TestService>, cx: &mut Context<Self>) -> Self {
        let _observe = cx.observe(&service, |_, _, cx| cx.notify());
        Self {
            service,
            selected: Node::Root,
            collapsed: HashSet::new(),
            show_passed: true,
            show_ignored: true,
            sort_alphabetically: false,
            sort_by_duration: false,
            tree_scroll: UniformListScrollHandle::new(),
            console_scroll: UniformListScrollHandle::new(),
            console: Vec::new(),
            shown_lines: 0,
            rows: Vec::new(),
            _observe,
        }
    }

    /// A new run starts at the root again.
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.selected = Node::Root;
        self.collapsed.clear();
        self.shown_lines = 0;
        cx.notify();
    }

    fn visible(&self, test: &TestCase) -> bool {
        match test.outcome {
            Outcome::Passed => self.show_passed,
            Outcome::Ignored => self.show_ignored,
            _ => true,
        }
    }

    /// The tree's rows: the root, then each class and its tests, filtered
    /// and sorted as the toolbar says.
    fn build_rows(&self, run: &TestRun) -> Vec<Row> {
        let results = &run.results;
        let root_outcome = if run.build_failed {
            Some(Outcome::Error)
        } else if run.running {
            Some(Outcome::Running)
        } else {
            results.suites.iter().map(TestSuite::outcome).max().or(Some(Outcome::Passed)).filter(|_| !results.suites.is_empty())
        };
        let mut rows = vec![Row { node: Node::Root, depth: 0, label: "Test Results".into(), outcome: root_outcome, duration_ms: results.duration_ms(), collapsed: None }];
        let mut suites: Vec<&TestSuite> = results.suites.iter().filter(|s| s.tests.iter().any(|t| self.visible(t)) || s.tests.is_empty()).collect();
        if self.sort_alphabetically {
            suites.sort_by_key(|s| s.simple_name().to_lowercase());
        }
        if self.sort_by_duration {
            suites.sort_by_key(|s| std::cmp::Reverse(s.duration_ms().unwrap_or(0)));
        }
        for suite in suites {
            let collapsed = self.collapsed.contains(&suite.class);
            rows.push(Row {
                node: Node::Suite(suite.class.clone()),
                depth: 1,
                label: suite.simple_name().replace('$', ".").into(),
                outcome: Some(suite.outcome()),
                duration_ms: suite.duration_ms(),
                collapsed: Some(collapsed),
            });
            if collapsed {
                continue;
            }
            let mut tests: Vec<(usize, &TestCase)> = suite.tests.iter().enumerate().filter(|(_, t)| self.visible(t)).collect();
            if self.sort_alphabetically {
                tests.sort_by_key(|(_, t)| t.name.to_lowercase());
            }
            if self.sort_by_duration {
                tests.sort_by_key(|(_, t)| std::cmp::Reverse(t.duration_ms.unwrap_or(0)));
            }
            for (ix, test) in tests {
                rows.push(Row { node: Node::Test(suite.class.clone(), ix), depth: 2, label: test.name.clone().into(), outcome: Some(test.outcome), duration_ms: test.duration_ms, collapsed: None });
            }
        }
        rows
    }

    /// The selected node's console.
    fn build_console(&self, run: &TestRun, cx: &gpui_kit::App) -> Vec<ConsoleLine> {
        let palette = cx.palette();
        let (normal, error, secondary) = (palette.text, palette.status_conflict, palette.text_secondary);
        let mut out: Vec<ConsoleLine> = Vec::new();
        let mut push = |text: &str, color: Hsla| out.push(ConsoleLine { text: text.to_owned().into(), color });
        let test_lines = |test: &TestCase, push: &mut dyn FnMut(&str, Hsla)| {
            for line in &test.output {
                push(&line.text, if line.stderr { error } else { normal });
            }
            if let Some(failure) = &test.failure {
                if let Some(c) = &test.comparison {
                    // IntelliJ prints the two sides under the message.
                    push("", normal);
                    push(&format!("Expected :{}", c.expected), normal);
                    push(&format!("Actual   :{}", c.actual), normal);
                    push("", normal);
                }
                for line in failure.lines() {
                    push(line, error);
                }
            }
        };
        match &self.selected {
            Node::Root => {
                for line in &run.lines {
                    let failed = line.starts_with("e: ") || line.starts_with("FAILURE") || line.contains("FAILED");
                    push(line, if failed { error } else { normal });
                }
            }
            Node::Suite(class) => {
                if let Some(suite) = run.results.suites.iter().find(|s| s.class == *class) {
                    for line in &suite.output {
                        push(&line.text, if line.stderr { error } else { normal });
                    }
                    for test in suite.tests.iter().filter(|t| self.visible(t)) {
                        test_lines(test, &mut push);
                    }
                    if suite.tests.iter().all(|t| t.output.is_empty() && t.failure.is_none()) && suite.output.is_empty() {
                        push(&summary_of(suite), secondary);
                    }
                }
            }
            Node::Test(class, ix) => {
                if let Some(test) = run.results.suites.iter().find(|s| s.class == *class).and_then(|s| s.tests.get(*ix)) {
                    test_lines(test, &mut push);
                    if test.output.is_empty() && test.failure.is_none() {
                        let text = match test.outcome {
                            Outcome::Ignored => "Test ignored.",
                            Outcome::Running => "Running…",
                            Outcome::Terminated => "Test terminated.",
                            _ => "Test passed.",
                        };
                        push(text, secondary);
                    }
                }
            }
        }
        out
    }

    fn select(&mut self, node: Node, click: &ClickEvent, cx: &mut Context<Self>) {
        if click.click_count() >= 2 {
            self.open_source(&node, cx);
        }
        if self.selected != node {
            self.selected = node;
            self.shown_lines = 0;
        }
        cx.notify();
    }

    /// Double-click: the test's (or class's) source, where the run came
    /// from; else the first frame of its failure in a project file.
    fn open_source(&mut self, node: &Node, cx: &mut Context<Self>) {
        let service = self.service.read(cx);
        let Some(run) = service.run.as_ref() else { return };
        let (class, method) = match node {
            Node::Root => return,
            Node::Suite(class) => (class.clone(), None),
            Node::Test(class, ix) => (class.clone(), run.results.suites.iter().find(|s| s.class == *class).and_then(|s| s.tests.get(*ix)).map(|t| t.method().to_owned())),
        };
        if let Some((path, line)) = service.source_of(&class, method.as_deref()) {
            cx.emit(RunViewEvent::OpenFile { path, line, col: 0 });
        }
    }

    fn render_tree(&mut self, range: Range<usize>, _: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let palette = cx.palette().clone();
        range
            .filter_map(|ix| self.rows.get(ix).map(|row| (ix, row)))
            .map(|(ix, row)| {
                let selected = row.node == self.selected;
                let node = row.node.clone();
                let toggle = match &row.node {
                    Node::Suite(class) => Some(class.clone()),
                    _ => None,
                };
                let chevron = row.collapsed.map(|collapsed| {
                    let class = toggle.clone().unwrap_or_default();
                    div()
                        .id(("test-toggle", ix))
                        .child(Icon::new(if collapsed { icons::CHEVRON_RIGHT } else { icons::CHEVRON_DOWN }).xsmall())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.collapsed.remove(&class) {
                                this.collapsed.insert(class.clone());
                            }
                            cx.stop_propagation();
                            cx.notify();
                        }))
                });
                h_flex()
                    .id(("test-row", ix))
                    .h(px(ROW))
                    .pl(px(6. + 16. * row.depth as f32 + if row.collapsed.is_none() && row.depth > 0 { 16. } else { 0. }))
                    .pr_2()
                    .gap_1()
                    .text_sm()
                    .when(selected, |el| el.bg(palette.selection))
                    .when(!selected, |el| el.hover(|s| s.bg(palette.hover)))
                    .children(chevron)
                    .child(Icon::new(outcome_icon(row.outcome)).xsmall())
                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(row.label.clone()))
                    .children(row.duration_ms.map(|ms| div().flex_none().text_xs().text_color(palette.text_secondary).child(testing::duration_narrow(ms))))
                    .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| this.select(node.clone(), event, cx)))
                    .into_any_element()
            })
            .collect()
    }

    fn render_console(&mut self, range: Range<usize>, _: &mut Window, _: &mut Context<Self>) -> Vec<AnyElement> {
        range
            .filter_map(|ix| self.console.get(ix).map(|l| (ix, l.clone())))
            .map(|(ix, line)| div().id(ix).h(px(LINE)).px_2().whitespace_nowrap().text_color(line.color).child(line.text).into_any_element())
            .collect()
    }

    fn render_toolbar(&self, run: Option<&TestRun>, cx: &mut Context<Self>) -> AnyElement {
        let service = self.service.read(cx);
        let running = service.is_running();
        let has_failed = service.has_failed();
        let can_rerun = run.is_some() && !running;
        let (rerun, rerun_failed, stop) = (self.service.clone(), self.service.clone(), self.service.clone());
        h_flex()
            .h(px(28.))
            .px_1()
            .gap_0p5()
            .flex_none()
            .child(tool_button("tests-rerun", icons::RERUN, "Rerun").disabled(!can_rerun).on_click(move |_, _, cx| rerun.update(cx, |s, cx| s.rerun(cx))))
            .child(tool_button("tests-rerun-failed", icons::RERUN_FAILED_TESTS, "Rerun Failed Tests").disabled(!has_failed).on_click(move |_, _, cx| rerun_failed.update(cx, |s, cx| s.rerun_failed(cx))))
            .child(tool_button("tests-stop", icons::STOP, stop_label()).disabled(!running).on_click(move |_, _, cx| stop.update(cx, |s, cx| s.stop(cx))))
            .child(div().w(px(1.)).h(px(16.)).mx_1().bg(cx.palette().border))
            .child(tool_button("tests-show-passed", icons::SHOW_PASSED, "Show Passed").selected(self.show_passed).on_click(cx.listener(|this, _, _, cx| {
                this.show_passed = !this.show_passed;
                cx.notify();
            })))
            .child(tool_button("tests-show-ignored", icons::SHOW_IGNORED, "Show Ignored").selected(self.show_ignored).on_click(cx.listener(|this, _, _, cx| {
                this.show_ignored = !this.show_ignored;
                cx.notify();
            })))
            .child(tool_button("tests-sort-alpha", icons::SORT_ALPHABETICALLY, "Sort Alphabetically").selected(self.sort_alphabetically).on_click(cx.listener(|this, _, _, cx| {
                this.sort_alphabetically = !this.sort_alphabetically;
                cx.notify();
            })))
            .child(tool_button("tests-sort-duration", icons::SORT_BY_DURATION, "Sort by Duration").selected(self.sort_by_duration).on_click(cx.listener(|this, _, _, cx| {
                this.sort_by_duration = !this.sort_by_duration;
                cx.notify();
            })))
            .child(tool_button("tests-expand", icons::EXPAND_ALL, "Expand All").on_click(cx.listener(|this, _, _, cx| {
                this.collapsed.clear();
                cx.notify();
            })))
            .child(tool_button("tests-collapse", icons::COLLAPSE_ALL, "Collapse All").on_click(cx.listener(|this, _, _, cx| {
                let classes: Vec<String> = this.service.read(cx).run.as_ref().map(|r| r.results.suites.iter().map(|s| s.class.clone()).collect()).unwrap_or_default();
                this.collapsed.extend(classes);
                cx.notify();
            })))
            .child(div().flex_1())
            .children(run.map(|run| self.render_status(run, cx)))
            .into_any_element()
    }

    /// IntelliJ's status line, at the right of the toolbar.
    fn render_status(&self, run: &TestRun, cx: &Context<Self>) -> AnyElement {
        let palette = cx.palette().clone();
        let counts = run.results.counts();
        let duration = run.results.duration_ms().or(run.took.map(|t| t.as_millis() as u64).filter(|_| !run.running));
        let (parts, description) = testing::status_line(&counts, !run.running, duration);
        let mut line = h_flex().gap_0().text_sm().whitespace_nowrap().pr_2();
        if run.build_failed {
            return line.child(div().text_color(palette.status_conflict).child("Test events were not received")).into_any_element();
        }
        if parts.is_empty() {
            let text = if run.running { "Running tests…" } else if run.cancelled { "Stopped." } else { "No tests were found" };
            return line.child(div().text_color(palette.text_secondary).child(text)).into_any_element();
        }
        for (text, tone) in parts {
            let color = match tone {
                Tone::Normal => palette.text,
                Tone::Failed => palette.status_conflict,
                Tone::Ignored => palette.text_secondary,
            };
            line = line.child(div().text_color(color).child(text));
        }
        if run.running {
            description_or_progress(&mut line, &format!("  {} / {}", counts.finished(), counts.finished() + counts.running), palette.text_secondary);
        } else if !description.is_empty() {
            description_or_progress(&mut line, &format!("  {description}"), palette.text_secondary);
        }
        line.into_any_element()
    }
}

fn description_or_progress(line: &mut gpui_kit::Div, text: &str, color: Hsla) {
    let taken = std::mem::replace(line, h_flex());
    *line = taken.child(div().text_color(color).child(text.to_owned()));
}

/// A class without output: how its tests went.
fn summary_of(suite: &TestSuite) -> String {
    let results = TestResults { suites: vec![suite.clone()] };
    let (parts, _) = testing::status_line(&results.counts(), true, None);
    parts.into_iter().map(|(t, _)| t).collect()
}

impl Render for TestView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mono = {
            use gpui_kit::component::ActiveTheme as _;
            cx.theme().mono_font_family.clone()
        };
        let service = self.service.clone();
        let service = service.read(cx);
        let Some(run) = service.run.as_ref() else {
            return v_flex()
                .size_full()
                .child(div().p_4().text_sm().text_color(palette.text_secondary).child("No tests have run yet. Click ▶ in the gutter of a test, or press Ctrl+Shift+F10 in it."))
                .into_any_element();
        };
        self.rows = self.build_rows(run);
        if !self.rows.iter().any(|r| r.node == self.selected) {
            self.selected = Node::Root;
        }
        self.console = self.build_console(run, cx);
        // The root's console follows Gradle while it runs.
        let count = self.console.len();
        if self.selected == Node::Root && count != self.shown_lines {
            self.shown_lines = count;
            if count > 0 {
                self.console_scroll.scroll_to_item(count - 1, gpui_kit::ScrollStrategy::Bottom);
            }
        }
        let toolbar = self.render_toolbar(Some(run), cx);
        let rows = self.rows.len();
        v_flex()
            .size_full()
            .child(toolbar)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .border_t_1()
                    .border_color(palette.border)
                    .child(
                        div()
                            .w(px(320.))
                            .flex_none()
                            .h_full()
                            .border_r_1()
                            .border_color(palette.border)
                            .child(uniform_list("test-tree", rows, cx.processor(Self::render_tree)).track_scroll(&self.tree_scroll).size_full()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .font_family(mono)
                            .text_xs()
                            .child(uniform_list("test-console", count, cx.processor(Self::render_console)).track_scroll(&self.console_scroll).size_full()),
                    ),
            )
            .into_any_element()
    }
}

/// The tab's title in the Run window.
pub fn tab_title(cx: &gpui_kit::App) -> Option<(String, bool)> {
    let service = TestService::try_global(cx)?;
    let run = service.read(cx).run.as_ref()?;
    Some((run.name.clone(), run.running))
}

#[allow(unused)]
fn heading(text: &str) -> gpui_kit::Div {
    div().text_sm().font_weight(FontWeight::SEMIBOLD).child(text.to_owned())
}
