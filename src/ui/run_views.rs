//! The Build, Run and Build Variants tool windows, over [`RunService`].
//! Build: the last build's status, its problems (click opens the source)
//! and Gradle's output. Run: the launch's console with Rerun and Stop.
//! Build Variants: each Android module's active variant.


use gpui_kit::component::{
    Icon, Sizable as _, h_flex,
    button::{Button, ButtonVariants as _},
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, EventEmitter, FontWeight, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, div, prelude::FluentBuilder as _, px,
};

use crate::domain::diagnostics::Severity;
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::common::tool_button;
use crate::ui::run_service::{BuildStatus, RunEvent, RunService};
use crate::ui::test_service::{TestService, TestServiceEvent};
use crate::ui::test_view::TestView;
use crate::ui::sync_service::{self, SyncService};
use crate::usecase::run_app::ConsoleLine;
use crate::ui::console_view::{ConsoleEvent, ConsoleView};

#[derive(Clone)]
pub enum RunViewEvent {
    /// Open a file at a 0-based line and column.
    OpenFile { path: std::path::PathBuf, line: u32, col: u32 },
    Hide,
}

fn header(title: impl IntoElement, cx: &gpui_kit::App) -> gpui_kit::Div {
    let palette = cx.palette().clone();
    h_flex().h(px(30.)).px_2().gap_1().border_b_1().border_color(palette.border).child(title)
}

/// A console's file link opens the file.
fn forward_open<T: EventEmitter<RunViewEvent>>(_: &mut T, _: Entity<ConsoleView>, event: &ConsoleEvent, cx: &mut Context<T>) {
    if let ConsoleEvent::OpenFile { path, line, col } = event {
        cx.emit(RunViewEvent::OpenFile { path: path.clone(), line: *line, col: *col });
    }
}

// --- Build -------------------------------------------------------------------

pub struct BuildView {
    service: Entity<RunService>,
    console: Entity<ConsoleView>,
    _subscriptions: Vec<gpui_kit::Subscription>,
}

impl EventEmitter<RunViewEvent> for BuildView {}

impl BuildView {
    pub fn new(service: Entity<RunService>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let console = cx.new(|cx| ConsoleView::new(window, cx).with_toolbar(true));
        let _subscriptions = vec![
            cx.observe_in(&service, window, |this, _, window, cx| {
                this.sync(window, cx);
                cx.notify();
            }),
            cx.subscribe(&console, forward_open),
            cx.subscribe(&console, |this: &mut Self, _, event: &ConsoleEvent, cx| {
                if let ConsoleEvent::ClearAll = event {
                    this.service.update(cx, |s, cx| s.clear_build_output(cx));
                }
            }),
        ];
        let mut this = Self { service, console, _subscriptions };
        this.sync(window, cx);
        this
    }

    /// Gradle's output into the console, failures red.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let red = cx.palette().status_conflict;
        let lines: Vec<(SharedString, Option<Hsla>)> = self.service.read(cx).build.as_ref().map_or_else(Vec::new, |build| {
            build
                .lines
                .iter()
                .map(|line| {
                    let failed = line.starts_with("e: ") || line.contains(": error:") || line.starts_with("FAILURE") || line.contains("FAILED");
                    (SharedString::from(line.to_string()), failed.then_some(red))
                })
                .collect()
        });
        self.console.update(cx, |c, cx| c.set_lines(&lines, window, cx));
    }

}

impl Render for BuildView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let service = self.service.read(cx);
        let Some(build) = service.build.as_ref() else {
            return v_flex()
                .size_full()
                .bg(palette.panel)
                .child(header(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Build Output"), cx))
                .child(div().p_4().text_sm().text_color(palette.text_secondary).child("No builds yet. Make Project (Ctrl+F9) or Run (Shift+F10)."))
                .into_any_element();
        };
        let (icon, color) = match build.status {
            BuildStatus::Running => (icons::RERUN, palette.text_secondary),
            BuildStatus::Succeeded => (icons::STATUS_SUCCESS, palette.status_added),
            BuildStatus::Failed => (icons::STATUS_ERROR, palette.status_conflict),
            BuildStatus::Cancelled => (icons::STATUS_WARNING, palette.ref_head),
        };
        let when = build.started.format("%H:%M").to_string();
        let summary = match (build.status, build.took) {
            (BuildStatus::Running, _) => format!("{}: running…", build.title),
            (BuildStatus::Succeeded, Some(t)) => format!("{}: finished at {when}  {}", build.title, seconds(t)),
            (BuildStatus::Failed, Some(t)) => format!("{}: failed at {when}  {}", build.title, seconds(t)),
            (BuildStatus::Cancelled, _) => format!("{}: cancelled at {when}", build.title),
            _ => build.title.clone(),
        };
        let errors = build.problems.iter().filter(|p| p.problem.severity == Severity::Error).count();
        let warnings = build.problems.len() - errors;
        let mut problems = v_flex().id("build-problems").w(px(380.)).flex_none().h_full().overflow_y_scroll().border_r_1().border_color(palette.border).text_sm();
        problems = problems.child(
            h_flex()
                .h(px(24.))
                .px_2()
                .gap_1()
                .child(Icon::new(icon).xsmall().text_color(color))
                .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().child(summary))
                .when(errors > 0 || warnings > 0, |el| {
                    el.child(div().text_xs().text_color(palette.text_secondary).child(format!("{}, {}", count(errors, "error"), count(warnings, "warning"))))
                }),
        );
        for (ix, p) in build.problems.iter().enumerate() {
            let name = p.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let (path, line, col) = (p.file.clone(), p.problem.start.line, p.problem.start.col);
            let error = p.problem.severity == Severity::Error;
            problems = problems.child(
                v_flex()
                    .id(("build-problem", ix))
                    .px_2()
                    .py_0p5()
                    .pl(px(24.))
                    .cursor_pointer()
                    .hover(|s| s.bg(palette.hover))
                    .child(
                        h_flex()
                            .gap_1()
                            .child(Icon::new(if error { icons::STATUS_ERROR } else { icons::STATUS_WARNING }).xsmall().text_color(if error { palette.status_conflict } else { palette.ref_head }))
                            .child(div().overflow_hidden().whitespace_nowrap().text_ellipsis().child(format!("{name}:{}", line + 1))),
                    )
                    .child(div().pl(px(20.)).text_xs().text_color(palette.text_secondary).overflow_hidden().whitespace_nowrap().text_ellipsis().child(p.problem.message.lines().next().unwrap_or_default().to_owned()))
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(RunViewEvent::OpenFile { path: path.clone(), line, col }))),
            );
        }
        let running = build.status == BuildStatus::Running;
        let service = self.service.clone();
        v_flex()
            .size_full()
            .bg(palette.panel)
            .child(
                header(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Build Output"), cx)
                    .child(div().flex_1())
                    .when(running, |el| el.child(tool_button("build-stop", icons::STOP, "Stop").on_click(move |_, _, cx| service.update(cx, |s, cx| s.stop(cx)))))
                    .child(tool_button("build-hide", icons::HIDE, "Hide").on_click(cx.listener(|_, _, _, cx| cx.emit(RunViewEvent::Hide)))),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .child(problems)
                    .child(div().flex_1().min_w_0().h_full().child(self.console.clone())),
            )
            .into_any_element()
    }
}

/// "12 s", "1 m 5 s"; under a second "640 ms".
fn seconds(d: std::time::Duration) -> String {
    match d.as_secs() {
        0 => format!("{} ms", d.subsec_millis()),
        s if s >= 60 => format!("{} m {} s", s / 60, s % 60),
        s => format!("{s} s"),
    }
}

fn count(n: usize, what: &str) -> String {
    format!("{n} {what}{}", if n == 1 { "" } else { "s" })
}

// --- Run ---------------------------------------------------------------------

/// The Run window's tabs, as Android Studio's: the app's launch, and the
/// last test run.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RunTab {
    App,
    Tests,
}

pub struct RunView {
    service: Entity<RunService>,
    tests: Entity<TestView>,
    tab: RunTab,
    console: Entity<ConsoleView>,
    _subscriptions: Vec<gpui_kit::Subscription>,
}

impl EventEmitter<RunViewEvent> for RunView {}

impl RunView {
    pub fn new(service: Entity<RunService>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let test_service = TestService::global(cx);
        let tests = cx.new(|cx| TestView::new(test_service.clone(), cx));
        let console = cx.new(|cx| ConsoleView::new(window, cx).with_toolbar(true));
        let _subscriptions = vec![
            cx.observe_in(&service, window, |this: &mut Self, _, window, cx| {
                this.sync(window, cx);
                cx.notify();
            }),
            cx.subscribe(&console, forward_open),
            cx.subscribe(&console, |this: &mut Self, _, event: &ConsoleEvent, cx| {
                if let ConsoleEvent::ClearAll = event {
                    this.service.update(cx, |s, cx| s.clear_console(cx));
                }
            }),
            cx.observe(&test_service, |_, _, cx| cx.notify()),
            // A launch shows its tab, a test run its own.
            cx.subscribe(&service, |this, _, event: &RunEvent, cx| {
                if matches!(event, RunEvent::ShowRun) {
                    this.tab = RunTab::App;
                    cx.notify();
                }
            }),
            cx.subscribe(&test_service, |this, _, event: &TestServiceEvent, cx| {
                if matches!(event, TestServiceEvent::Show) {
                    this.tab = RunTab::Tests;
                    this.tests.update(cx, |view, cx| view.reset(cx));
                    cx.notify();
                }
            }),
            cx.subscribe(&tests, |_, _, event: &RunViewEvent, cx| cx.emit(event.clone())),
        ];
        let mut this = Self { service, tests, tab: RunTab::App, console, _subscriptions };
        this.sync(window, cx);
        this
    }

    /// The launch's output into the console: commands gray, errors red.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let palette = cx.palette().clone();
        let lines: Vec<(SharedString, Option<Hsla>)> = self.service.read(cx).console.as_ref().map_or_else(Vec::new, |console| {
            console
                .lines
                .iter()
                .map(|line| match line {
                    ConsoleLine::Info(t) => (SharedString::from(t.to_string()), None),
                    ConsoleLine::Command(t) => (SharedString::from(t.to_string()), Some(palette.text_secondary)),
                    ConsoleLine::Error(t) => (SharedString::from(t.to_string()), Some(palette.status_conflict)),
                })
                .collect()
        });
        self.console.update(cx, |c, cx| c.set_lines(&lines, window, cx));
    }

    /// The tab strip, once a test has run: "app" and "ExampleUnitTest".
    fn render_tabs(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (test_name, testing) = crate::ui::test_view::tab_title(cx)?;
        let palette = cx.palette().clone();
        let app_name = self.service.read(cx).console.as_ref().map(|c| c.name.clone());
        let tab = |id: &'static str, icon: icons::AsIcon, label: String, active: bool| {
            h_flex()
                .id(id)
                .h_full()
                .px_2()
                .gap_1()
                .text_sm()
                .cursor_pointer()
                .border_b_2()
                .border_color(if active { palette.accent } else { gpui_kit::transparent_black() })
                .when(!active, |el| el.hover(|s| s.bg(palette.hover)))
                .child(Icon::new(icon).xsmall())
                .child(label)
        };
        let mut strip = h_flex().h(px(30.)).px_1().gap_0p5().border_b_1().border_color(palette.border).flex_none();
        if let Some(name) = app_name {
            strip = strip.child(tab("run-tab-app", icons::MODULE_ANDROID, name, self.tab == RunTab::App).on_click(cx.listener(|this, _, _, cx| {
                this.tab = RunTab::App;
                cx.notify();
            })));
        }
        let test_icon = if testing { icons::RERUN } else { icons::JUNIT };
        strip = strip.child(tab("run-tab-tests", test_icon, test_name, self.tab == RunTab::Tests).on_click(cx.listener(|this, _, _, cx| {
            this.tab = RunTab::Tests;
            cx.notify();
        })));
        Some(strip.child(div().flex_1()).child(tool_button("run-hide", icons::HIDE, "Hide").on_click(cx.listener(|_, _, _, cx| cx.emit(RunViewEvent::Hide)))).into_any_element())
    }

}

impl Render for RunView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let tabs = self.render_tabs(cx);
        let has_app = self.service.read(cx).console.is_some();
        if tabs.is_some() && (self.tab == RunTab::Tests || !has_app) {
            return v_flex().size_full().bg(palette.panel).children(tabs).child(div().flex_1().min_h_0().child(self.tests.clone())).into_any_element();
        }
        let mono = { use gpui_kit::component::ActiveTheme as _; cx.theme().mono_font_family.clone() };
        let service = self.service.read(cx);
        let title = service.console.as_ref().map(|c| c.name.clone()).unwrap_or_else(|| "Run".into());
        let can_stop = service.can_stop();
        let (rerun, stop) = (self.service.clone(), self.service.clone());
        v_flex()
            .size_full()
            .bg(palette.panel)
            .children(tabs)
            .child(
                header(
                    h_flex().gap_1().text_sm().font_weight(FontWeight::SEMIBOLD).child(Icon::new(icons::MODULE_ANDROID).xsmall()).child(title),
                    cx,
                )
                .child(div().flex_1())
                .child(tool_button("run-rerun", icons::RERUN, rerun_label()).on_click(move |_, _, cx| rerun.update(cx, |s, cx| s.run(cx))))
                .when(can_stop, |el| el.child(tool_button("run-stop", icons::STOP, stop_label()).on_click(move |_, _, cx| stop.update(cx, |s, cx| s.stop(cx)))))
                .child(tool_button("run-hide", icons::HIDE, "Hide").on_click(cx.listener(|_, _, _, cx| cx.emit(RunViewEvent::Hide)))),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .font_family(mono)
                    .text_xs()
                    .when(!has_app, |el| el.child(div().p_4().text_color(palette.text_secondary).child("Nothing has run yet. Run 'app' with Shift+F10.")))
                    .when(has_app, |el| el.child(self.console.clone())),
            )
            .into_any_element()
    }
}

pub fn run_label(name: &str) -> String {
    format!("Run '{name}'  {}", if cfg!(target_os = "macos") { "⌃R" } else { "Shift+F10" })
}

pub fn debug_label(name: &str) -> String {
    format!("Debug '{name}'  {}", if cfg!(target_os = "macos") { "⌃D" } else { "Shift+F9" })
}

fn rerun_label() -> &'static str {
    if cfg!(target_os = "macos") { "Rerun  ⌃R" } else { "Rerun  Shift+F10" }
}

pub fn stop_label() -> &'static str {
    if cfg!(target_os = "macos") { "Stop  ⌘F2" } else { "Stop  Ctrl+F2" }
}

// --- Build Variants ----------------------------------------------------------

pub struct VariantsView {
    _observe: gpui_kit::Subscription,
}

impl EventEmitter<RunViewEvent> for VariantsView {}

impl VariantsView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let sync = SyncService::global(cx);
        Self { _observe: cx.observe(&sync, |_, _, cx| cx.notify()) }
    }
}

impl Render for VariantsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let builds = SyncService::global(cx).read(cx).builds().to_vec();
        let mut table = v_flex().id("variants").flex_1().min_h_0().overflow_y_scroll().text_sm();
        table = table.child(
            h_flex()
                .h(px(24.))
                .px_2()
                .border_b_1()
                .border_color(palette.border)
                .text_color(palette.text_secondary)
                .child(div().w(px(200.)).child("Module"))
                .child(div().flex_1().child("Active Build Variant")),
        );
        let mut any = false;
        for (build, modules) in &builds {
            for (ix, module) in modules.iter().filter(|m| m.kind.is_android()).enumerate() {
                any = true;
                let active = sync_service::variant_of(build, module, cx);
                let name = module.name();
                let (build, module) = (build.clone(), module.clone());
                let key = sync_service::variant_key(&build, &module.path);
                let picker = Button::new(gpui_kit::SharedString::from(format!("variant-{key}")))
                    .ghost()
                    .small()
                    .label(active.clone())
                    .child(Icon::new(icons::CHEVRON_DOWN).xsmall())
                    .dropdown_menu(move |mut menu, _, _| {
                        for variant in &module.variants {
                            let (build, key, variant) = (build.clone(), key.clone(), variant.clone());
                            menu = menu.item(PopupMenuItem::new(variant.clone()).checked(variant == active).on_click(move |_, _, cx| {
                                let (key, variant) = (key.clone(), variant.clone());
                                crate::settings::Settings::update(cx, |s| {
                                    s.build_variants.insert(key, variant);
                                });
                                // As Android Studio: the libraries follow the variant.
                                let build = build.clone();
                                SyncService::global(cx).update(cx, |s, cx| s.sync(build, cx));
                            }));
                        }
                        menu
                    });
                table = table.child(
                    h_flex()
                        .id(("variant-row", ix))
                        .h(px(26.))
                        .px_2()
                        .child(h_flex().w(px(200.)).gap_1().child(Icon::new(icons::MODULE_ANDROID).xsmall()).child(name))
                        .child(h_flex().flex_1().child(picker)),
                );
            }
        }
        if !any {
            table = table.child(div().p_4().text_color(palette.text_secondary).child("No Android modules. Sync the project with Gradle files first."));
        }
        v_flex()
            .size_full()
            .bg(palette.panel)
            .child(
                header(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Build Variants"), cx)
                    .child(div().flex_1())
                    .child(tool_button("variants-hide", icons::HIDE, "Hide").on_click(cx.listener(|_, _, _, cx| cx.emit(RunViewEvent::Hide)))),
            )
            .child(table)
    }
}
