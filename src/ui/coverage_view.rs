//! IntelliJ's Coverage tool window after Run with Coverage: the run's
//! packages and their classes with the share of classes, methods, lines and
//! branches the tests ran ("Class, %", "Method, %", "Line, %", "Branch, %",
//! as "50% (1/2)"). A double click opens a class's source; Hide Coverage
//! drops the report and the editors' stripes.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    ClickEvent, Context, Entity, EventEmitter, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div, prelude::FluentBuilder as _, px,
};

use crate::domain::coverage::{ClassCoverage, Counter, CoverageReport};
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::common::tool_button;
use crate::ui::test_service::{TestService, TestServiceEvent};

pub enum CoverageEvent {
    /// Open a class's source file.
    Open(PathBuf),
    Hide,
}

/// The percentage columns' width.
const COLUMN: f32 = 104.;

#[derive(Clone, PartialEq)]
enum Row {
    Package(String),
    Class(String),
}

pub struct CoverageView {
    tests: Entity<TestService>,
    report: Option<Arc<CoverageReport>>,
    expanded: HashSet<String>,
    selected: Option<Row>,
    _subscription: gpui_kit::Subscription,
}

impl EventEmitter<CoverageEvent> for CoverageView {}

impl CoverageView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let tests = TestService::global(cx);
        let subscription = cx.subscribe(&tests, |this, _, event: &TestServiceEvent, cx| {
            if matches!(event, TestServiceEvent::Coverage) {
                this.reload(cx);
            }
        });
        let mut this = Self { tests, report: None, expanded: HashSet::new(), selected: None, _subscription: subscription };
        this.reload(cx);
        this
    }

    pub fn is_empty(&self) -> bool {
        self.report.is_none()
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.report = self.tests.read(cx).coverage.clone();
        self.selected = None;
        self.expanded.clear();
        // A single package opens, as IntelliJ shows a small run's classes.
        if let Some(report) = &self.report {
            let packages = report.packages();
            if packages.len() == 1 {
                self.expanded.insert(packages[0].name.clone());
            }
        }
        cx.notify();
    }

    fn open(&mut self, class: &ClassCoverage, cx: &mut Context<Self>) {
        let root = self.tests.read(cx).coverage_root.clone();
        if let Some(path) = root.and_then(|root| source_of(&root, class)) {
            cx.emit(CoverageEvent::Open(path));
        }
    }

    fn row(&self, id: SharedString, depth: usize, selected: bool, cx: &mut Context<Self>) -> gpui_kit::Stateful<gpui_kit::Div> {
        let palette = cx.palette().clone();
        h_flex()
            .id(id)
            .h(px(22.))
            .pl(px(8. + depth as f32 * 16.))
            .pr_2()
            .whitespace_nowrap()
            .when(selected, |el| el.bg(palette.selection))
            .when(!selected, |el| el.hover(|s| s.bg(palette.hover)))
    }
}

/// A class's source: `src/<set>/java|kotlin/<package>/<file>` of the module,
/// main sources first.
fn source_of(root: &Path, class: &ClassCoverage) -> Option<PathBuf> {
    let file = class.source.as_deref()?;
    let rel: PathBuf = class.package().split('.').filter(|p| !p.is_empty()).chain(std::iter::once(file)).collect();
    let mut sets: Vec<String> = std::fs::read_dir(root.join("src")).ok()?.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|s| crate::domain::testing::test_kind(&format!("src/{s}/")).is_none()).collect();
    sets.sort_by_key(|s| (s != "main", s.clone()));
    sets.iter().flat_map(|set| ["java", "kotlin"].map(|lang| root.join("src").join(set).join(lang).join(&rel))).find(|p| p.is_file())
}

fn cells(classes: Option<Counter>, methods: Counter, lines: Counter, branches: Counter) -> impl Iterator<Item = gpui_kit::Div> {
    [classes.map(|c| c.label()).unwrap_or_default(), methods.label(), lines.label(), branches.label()].into_iter().map(|text| div().w(px(COLUMN)).flex_none().child(text))
}

impl Render for CoverageView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mut list = v_flex().id("coverage-rows").flex_1().min_h_0().overflow_y_scroll().py_1().text_sm();
        let title = self.report.as_ref().map(|r| r.name.clone()).unwrap_or_default();
        match self.report.clone() {
            None => list = list.child(div().pt_8().w_full().text_center().text_color(palette.text_secondary).child("No coverage. Run tests with Coverage from the gutter or the run widget.")),
            Some(report) => {
                for (ix, package) in report.packages().into_iter().enumerate() {
                    let expanded = self.expanded.contains(&package.name);
                    let row = Row::Package(package.name.clone());
                    let name = package.name.clone();
                    let (toggle, click) = (name.clone(), row.clone());
                    list = list.child(
                        self.row(format!("coverage-package-{ix}").into(), 0, self.selected.as_ref() == Some(&row), cx)
                            .child(
                                div()
                                    .id(("coverage-toggle", ix))
                                    .w(px(16.))
                                    .child(Icon::new(if expanded { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT }).xsmall())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if !this.expanded.remove(&toggle) {
                                            this.expanded.insert(toggle.clone());
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(h_flex().flex_1().min_w(px(120.)).overflow_hidden().gap_1().child(Icon::new(icons::PACKAGE).xsmall()).child(if name.is_empty() { "<default>".to_owned() } else { name.clone() }))
                            .children(cells(Some(package.classes), package.methods, package.lines, package.branches))
                            .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                                this.selected = Some(click.clone());
                                if e.click_count() == 2 && !this.expanded.remove(&name) {
                                    this.expanded.insert(name.clone());
                                }
                                cx.notify();
                            })),
                    );
                    if !expanded {
                        continue;
                    }
                    for (cix, class) in report.classes_in(&package.name).enumerate() {
                        let row = Row::Class(class.name.clone());
                        let (click, owned) = (row.clone(), class.clone());
                        let one = Counter::new(!class.covered() as u32, class.covered() as u32);
                        list = list.child(
                            self.row(format!("coverage-class-{ix}-{cix}").into(), 1, self.selected.as_ref() == Some(&row), cx)
                                .child(div().w(px(16.)))
                                .child(h_flex().flex_1().min_w(px(104.)).overflow_hidden().gap_1().child(Icon::new(icons::CLASS).xsmall()).child(class.simple_name().to_owned()))
                                .children(cells(Some(one), class.methods, class.lines, class.branches))
                                .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                                    this.selected = Some(click.clone());
                                    if e.click_count() == 2 {
                                        this.open(&owned, cx);
                                    }
                                    cx.notify();
                                })),
                        );
                    }
                }
            }
        }
        let has_report = self.report.is_some();
        v_flex()
            .size_full()
            .bg(palette.panel)
            .child(
                h_flex()
                    .h(px(30.))
                    .px_2()
                    .gap_1()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Coverage"))
                    .child(div().text_sm().text_color(palette.text_secondary).child(title))
                    .child(div().flex_1())
                    .when(has_report, |el| {
                        el.child(tool_button("coverage-close", icons::CLOSE, "Hide Coverage").on_click(cx.listener(|this, _, _, cx| this.tests.update(cx, |t, cx| t.hide_coverage(cx)))))
                    })
                    .child(tool_button("coverage-hide", icons::HIDE, "Hide").on_click(cx.listener(|_, _, _, cx| cx.emit(CoverageEvent::Hide)))),
            )
            .child(
                h_flex()
                    .h(px(24.))
                    .pl(px(24.))
                    .pr_2()
                    .text_xs()
                    .text_color(palette.text_secondary)
                    .border_b_1()
                    .border_color(palette.border)
                    .child(div().flex_1().min_w(px(120.)).child("Element"))
                    .children(["Class, %", "Method, %", "Line, %", "Branch, %"].map(|h| div().w(px(COLUMN)).flex_none().child(h))),
            )
            .child(list)
    }
}
