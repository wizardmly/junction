//! IntelliJ's Breakpoints dialog (Ctrl+Shift+F8, Run › View Breakpoints…):
//! every line breakpoint, grouped by language, each with its checkbox;
//! the selected one's properties (Enabled, Suspend execution, Condition,
//! what a hit logs) on the right; Remove, and View Source (or a double
//! click) to open its line.

use std::collections::HashMap;

use gpui_kit::component::{
    Disableable as _, Icon, Sizable as _, WindowExt as _, h_flex,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    dialog::{DialogAction, DialogFooter},
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px,
};

use crate::infra::breakpoint_store::{SavedBreakpoint, SavedExceptionBreakpoint};
use crate::theme::ActivePalette as _;
use crate::ui::as_icons::{self as icons, AsIcon};
use crate::ui::common::tool_button;
use crate::ui::debug_service::DebugService;

/// IntelliJ's group for a file's breakpoints.
fn group(file: &str) -> &'static str {
    match file.rsplit('.').next() {
        Some("java") => "Java Line Breakpoints",
        Some("kt" | "kts") => "Kotlin Line Breakpoints",
        _ => "Line Breakpoints",
    }
}

/// "CalcTest.java:34", the line 1-based.
pub fn title(b: &SavedBreakpoint) -> String {
    format!("{}:{}", b.file.rsplit(['/', '\\']).next().unwrap_or(&b.file), b.line + 1)
}

/// The gutter's icon for a breakpoint: red or, for a logging one, yellow;
/// hollow when disabled, checked once it stops somewhere.
fn icon(b: &SavedBreakpoint, verified: bool) -> AsIcon {
    match (b.suspend, b.enabled, verified) {
        (true, false, _) => icons::BREAKPOINT_DISABLED,
        (true, true, true) => icons::BREAKPOINT_VALID,
        (true, true, false) => icons::BREAKPOINT,
        (false, false, _) => icons::BREAKPOINT_UNSUSPENDENT_DISABLED,
        (false, true, true) => icons::BREAKPOINT_UNSUSPENDENT_VALID,
        (false, true, false) => icons::BREAKPOINT_UNSUSPENDENT,
    }
}

pub struct BreakpointsView {
    service: Entity<DebugService>,
    /// The breakpoints, grouped as the list shows them.
    entries: Vec<(SavedBreakpoint, bool)>,
    /// The selected breakpoint's file and line.
    selected: Option<(String, u32)>,
    condition: Entity<InputState>,
    log_expression: Entity<InputState>,
    /// "Evaluate and log" is on (its expression may still be empty).
    log_on: bool,
    /// The source lines, by file, for the list's second line.
    sources: HashMap<String, Vec<String>>,
    /// Java Exception Breakpoints, "Any exception" first.
    exceptions: Vec<SavedExceptionBreakpoint>,
    /// The selected exception breakpoint's class (instead of a line one).
    exception: Option<String>,
    /// "+": the new exception breakpoint's class field.
    adding: Option<Entity<InputState>>,
    /// Enter in the class field also reaches the dialog: it stays open.
    just_added: bool,
    _subscriptions: Vec<Subscription>,
}

/// "Any exception", or the class.
fn exception_title(e: &SavedExceptionBreakpoint) -> String {
    if e.class.is_empty() { "Any exception".into() } else { e.class.clone() }
}

impl BreakpointsView {
    fn refresh(&mut self, cx: &App) {
        let mut entries = self.service.read(cx).all_breakpoints();
        // Groups in IntelliJ's order, then by file and line.
        entries.sort_by(|(a, _), (b, _)| (group(&a.file), title(a), a.line).cmp(&(group(&b.file), title(b), b.line)));
        for (b, _) in &entries {
            self.sources.entry(b.file.clone()).or_insert_with(|| std::fs::read_to_string(&b.file).map(|t| t.lines().map(|l| l.trim().to_owned()).collect()).unwrap_or_default());
        }
        self.exceptions = self.service.read(cx).exception_breakpoints();
        if self.exception.as_ref().is_some_and(|c| !self.exceptions.iter().any(|e| &e.class == c)) {
            self.exception = None;
        }
        if self.exception.is_none() && self.selected.as_ref().is_none_or(|(f, l)| !entries.iter().any(|(b, _)| &b.file == f && b.line == *l)) {
            self.selected = entries.first().map(|(b, _)| (b.file.clone(), b.line));
            if self.selected.is_none() {
                self.exception = Some(String::new());
            }
        }
        self.entries = entries;
    }

    fn current_exception(&self) -> Option<&SavedExceptionBreakpoint> {
        let class = self.exception.as_ref()?;
        self.exceptions.iter().find(|e| &e.class == class)
    }

    fn select_exception(&mut self, class: String, cx: &mut Context<Self>) {
        self.apply(|_| {}, cx);
        self.selected = None;
        self.exception = Some(class);
        cx.notify();
    }

    fn apply_exception(&mut self, change: impl FnOnce(&mut SavedExceptionBreakpoint), cx: &mut Context<Self>) {
        let Some(mut e) = self.current_exception().cloned() else { return };
        change(&mut e);
        self.service.update(cx, |s, cx| s.set_exception_breakpoint(e, cx));
        self.refresh(cx);
        cx.notify();
    }

    /// "+": a field for the new exception breakpoint's class.
    fn start_adding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Exception class, e.g. java.lang.IllegalStateException"));
        self._subscriptions.push(cx.subscribe(&input, |this, input, event: &InputEvent, cx| {
            if !matches!(event, InputEvent::PressEnter { .. }) {
                return;
            }
            let class = input.read(cx).value().trim().to_owned();
            this.adding = None;
            this.just_added = true;
            // Only this Enter: cleared a moment later.
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(std::time::Duration::from_millis(200)).await;
                this.update(cx, |this, _| this.just_added = false).ok();
            })
            .detach();
            if !class.is_empty() && class.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '_' || c == '$') {
                this.service.update(cx, |s, cx| s.set_exception_breakpoint(SavedExceptionBreakpoint::new(&class), cx));
                this.refresh(cx);
                this.select_exception(class, cx);
            }
            cx.notify();
        }));
        input.update(cx, |s, cx| s.focus(window, cx));
        self.adding = Some(input);
        cx.notify();
    }

    fn current(&self) -> Option<&SavedBreakpoint> {
        let (file, line) = self.selected.as_ref()?;
        self.entries.iter().map(|(b, _)| b).find(|b| &b.file == file && b.line == *line)
    }

    /// The fields into the selected breakpoint's text properties.
    fn load_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (condition, log) = self.current().map(|b| (b.condition.clone().unwrap_or_default(), b.log_expression.clone().unwrap_or_default())).unwrap_or_default();
        self.log_on = self.current().is_some_and(|b| b.log_expression.is_some());
        self.condition.update(cx, |s, cx| s.set_value(condition, window, cx));
        self.log_expression.update(cx, |s, cx| s.set_value(log, window, cx));
    }

    /// Applies a change to the selected breakpoint, with the fields' text.
    fn apply(&mut self, change: impl FnOnce(&mut SavedBreakpoint), cx: &mut Context<Self>) {
        let Some(mut b) = self.current().cloned() else { return };
        let text = |input: &Entity<InputState>| Some(input.read(cx).value().to_string());
        b.condition = text(&self.condition);
        b.log_expression = if self.log_on { text(&self.log_expression) } else { None };
        change(&mut b);
        self.service.update(cx, |s, cx| s.set_breakpoint_properties(b, cx));
        self.refresh(cx);
        cx.notify();
    }

    fn select(&mut self, file: String, line: u32, window: &mut Window, cx: &mut Context<Self>) {
        self.apply(|_| {}, cx);
        self.exception = None;
        self.selected = Some((file, line));
        self.load_fields(window, cx);
        cx.notify();
    }

    fn remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(class) = self.exception.clone() {
            self.service.update(cx, |s, cx| s.remove_exception_breakpoint(&class, cx));
            self.exception = Some(String::new());
            self.refresh(cx);
            cx.notify();
            return;
        }
        let Some((file, line)) = self.selected.clone() else { return };
        // The next one down is selected, as IntelliJ's.
        let ix = self.entries.iter().position(|(b, _)| b.file == file && b.line == line).unwrap_or(0);
        self.service.update(cx, |s, cx| s.remove_breakpoint(&file, line, cx));
        self.refresh(cx);
        let next = self.entries.get(ix).or(self.entries.last()).map(|(b, _)| (b.file.clone(), b.line));
        self.selected = next;
        self.load_fields(window, cx);
        cx.notify();
    }

    fn show_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apply(|_| {}, cx);
        let Some((file, line)) = self.selected.clone() else { return };
        window.close_dialog(cx);
        self.service.update(cx, |s, cx| s.show_source(&file, line, cx));
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mut list = v_flex().id("breakpoints-list").flex_1().min_h_0().overflow_y_scroll().py_1();
        let mut last_group = "";
        for (ix, (b, verified)) in self.entries.iter().enumerate() {
            let g = group(&b.file);
            if g != last_group {
                last_group = g;
                list = list.child(
                    h_flex()
                        .h(px(24.))
                        .px_2()
                        .gap_1()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(Icon::new(icons::CHEVRON_DOWN).xsmall())
                        .child(Icon::new(icons::BREAKPOINT).xsmall())
                        .child(g),
                );
            }
            let selected = self.selected.as_ref().is_some_and(|(f, l)| f == &b.file && *l == b.line);
            let source = self.sources.get(&b.file).and_then(|lines| lines.get(b.line as usize)).cloned().unwrap_or_default();
            let (file, line, enabled) = (b.file.clone(), b.line, b.enabled);
            let (file2, line2) = (file.clone(), line);
            list = list.child(
                h_flex()
                    .id(("breakpoint-row", ix))
                    .h(px(24.))
                    .pl(px(26.))
                    .pr_2()
                    .gap_1()
                    .text_sm()
                    .whitespace_nowrap()
                    .when(selected, |el| el.bg(palette.selection))
                    .when(!selected, |el| el.hover(|s| s.bg(palette.hover)))
                    .child(Checkbox::new(("breakpoint-enabled", ix)).checked(enabled).on_click(cx.listener(move |this, checked: &bool, window, cx| {
                        let checked = *checked;
                        this.select(file2.clone(), line2, window, cx);
                        this.apply(|b| b.enabled = checked, cx);
                    })))
                    .child(Icon::new(icon(b, *verified)).xsmall())
                    .child(div().flex_none().child(title(b)))
                    .child(div().overflow_hidden().text_ellipsis().text_color(palette.text_secondary).child(source))
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        this.select(file.clone(), line, window, cx);
                        if event.click_count() == 2 {
                            this.show_source(window, cx);
                        }
                    })),
            );
        }
        // Java Exception Breakpoints, "Any exception" first.
        list = list.child(
            h_flex()
                .h(px(24.))
                .px_2()
                .gap_1()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(Icon::new(icons::CHEVRON_DOWN).xsmall())
                .child(Icon::new(icons::BREAKPOINT_EXCEPTION).xsmall())
                .child("Java Exception Breakpoints"),
        );
        if let Some(input) = &self.adding {
            list = list.child(div().pl(px(26.)).pr_2().py_0p5().child(Input::new(input).small()));
        }
        for (ix, e) in self.exceptions.iter().enumerate() {
            let selected = self.exception.as_ref() == Some(&e.class);
            let (class, class2, enabled) = (e.class.clone(), e.class.clone(), e.enabled);
            list = list.child(
                h_flex()
                    .id(("exception-row", ix))
                    .h(px(24.))
                    .pl(px(26.))
                    .pr_2()
                    .gap_1()
                    .text_sm()
                    .whitespace_nowrap()
                    .when(selected, |el| el.bg(palette.selection))
                    .when(!selected, |el| el.hover(|s| s.bg(palette.hover)))
                    .child(Checkbox::new(("exception-enabled", ix)).checked(enabled).on_click(cx.listener(move |this, checked: &bool, _, cx| {
                        let checked = *checked;
                        this.select_exception(class2.clone(), cx);
                        this.apply_exception(|e| e.enabled = checked, cx);
                    })))
                    .child(Icon::new(if enabled { icons::BREAKPOINT_EXCEPTION } else { icons::BREAKPOINT_EXCEPTION_DISABLED }).xsmall())
                    .child(div().overflow_hidden().text_ellipsis().child(exception_title(e)))
                    .on_click(cx.listener(move |this, _, _, cx| this.select_exception(class.clone(), cx))),
            );
        }
        list
    }

    fn render_exception_properties(&self, e: SavedExceptionBreakpoint, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let palette = cx.palette().clone();
        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .p_3()
            .gap_2()
            .text_sm()
            .child(div().font_weight(FontWeight::SEMIBOLD).child(SharedString::from(exception_title(&e))))
            .child(Checkbox::new("exception-enabled-prop").label("Enabled").checked(e.enabled).on_click(cx.listener(|this, checked: &bool, _, cx| {
                let checked = *checked;
                this.apply_exception(|e| e.enabled = checked, cx);
            })))
            .child(div().text_color(palette.text_secondary).child("Notifications:"))
            .child(Checkbox::new("exception-caught").label("Caught exception").checked(e.caught).on_click(cx.listener(|this, checked: &bool, _, cx| {
                let checked = *checked;
                this.apply_exception(|e| e.caught = checked, cx);
            })))
            .child(Checkbox::new("exception-uncaught").label("Uncaught exception").checked(e.uncaught).on_click(cx.listener(|this, checked: &bool, _, cx| {
                let checked = *checked;
                this.apply_exception(|e| e.uncaught = checked, cx);
            })))
            .into_any_element()
    }

    fn render_properties(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        if let Some(e) = self.current_exception().cloned() {
            return self.render_exception_properties(e, cx);
        }
        let Some(b) = self.current().cloned() else {
            return v_flex().flex_1().into_any_element();
        };
        let logging = self.log_on;
        let label = |text: &'static str| div().text_sm().text_color(palette.text_secondary).child(text);
        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .p_3()
            .gap_2()
            .text_sm()
            .child(div().font_weight(FontWeight::SEMIBOLD).child(SharedString::from(title(&b))))
            .child(Checkbox::new("breakpoints-enabled").label("Enabled").checked(b.enabled).on_click(cx.listener(|this, checked: &bool, _, cx| {
                let checked = *checked;
                this.apply(|b| b.enabled = checked, cx);
            })))
            .child(Checkbox::new("breakpoints-suspend").label("Suspend execution").checked(b.suspend).on_click(cx.listener(|this, checked: &bool, _, cx| {
                let checked = *checked;
                this.apply(|b| b.suspend = checked, cx);
            })))
            .child(v_flex().gap_1().child(label("Condition:")).child(Input::new(&self.condition).small()))
            .child(
                v_flex()
                    .gap_1()
                    .child(label("Log:"))
                    .child(Checkbox::new("breakpoints-log-message").label("\"Breakpoint hit\" message").checked(b.log_message).on_click(cx.listener(|this, checked: &bool, _, cx| {
                        let checked = *checked;
                        this.apply(|b| b.log_message = checked, cx);
                    })))
                    .child(Checkbox::new("breakpoints-log-stack").label("Stack trace").checked(b.log_stack).on_click(cx.listener(|this, checked: &bool, _, cx| {
                        let checked = *checked;
                        this.apply(|b| b.log_stack = checked, cx);
                    })))
                    .child(Checkbox::new("breakpoints-log-expression").label("Evaluate and log:").checked(logging).on_click(cx.listener(|this, checked: &bool, window, cx| {
                        let checked = *checked;
                        this.log_on = checked;
                        this.apply(|_| {}, cx);
                        if checked {
                            this.log_expression.update(cx, |s, cx| s.focus(window, cx));
                        }
                    })))
                    .child(Input::new(&self.log_expression).small().disabled(!logging)),
            )
            .into_any_element()
    }
}

impl Render for BreakpointsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let has_selection = self.current().is_some() || self.current_exception().is_some_and(|e| !e.class.is_empty());
        h_flex()
            .items_start()
            .h(px(420.))
            .border_1()
            .border_color(palette.border)
            .rounded_md()
            .child(
                v_flex()
                    .w(px(340.))
                    .flex_none()
                    .h_full()
                    .border_r_1()
                    .border_color(palette.border)
                    .child(
                        h_flex()
                            .h(px(28.))
                            .px_1()
                            .flex_none()
                            .border_b_1()
                            .border_color(palette.border)
                            .child(tool_button("breakpoints-add", icons::ADD, "Add Java Exception Breakpoint").on_click(cx.listener(|this, _, window, cx| this.start_adding(window, cx))))
                            .child(
                                tool_button("breakpoints-remove", icons::REMOVE, "Remove")
                                    .disabled(!has_selection)
                                    .on_click(cx.listener(|this, _, window, cx| this.remove(window, cx))),
                            ),
                    )
                    .child(self.render_list(cx)),
            )
            .child(self.render_properties(cx))
    }
}

/// Opens the Breakpoints dialog, the breakpoint on `at` (file, 0-based
/// line) selected when given.
pub fn open(at: Option<(String, u32)>, window: &mut Window, cx: &mut App) {
    let service = DebugService::global(cx);
    let view = cx.new(|cx| {
        let condition = cx.new(|cx| InputState::new(window, cx).placeholder("Condition, e.g. i > 10"));
        let log_expression = cx.new(|cx| InputState::new(window, cx).placeholder("Expression, e.g. \"x = \" + x"));
        // Text applies when the field is left or Enter is pressed.
        fn commit(this: &mut BreakpointsView, _: Entity<InputState>, event: &InputEvent, cx: &mut Context<BreakpointsView>) {
            if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                this.apply(|_| {}, cx);
            }
        }
        let _subscriptions = vec![cx.subscribe(&condition, commit), cx.subscribe(&log_expression, commit)];
        let mut view = BreakpointsView { service, entries: Vec::new(), selected: at, condition, log_expression, log_on: false, sources: HashMap::new(), exceptions: Vec::new(), exception: None, adding: None, just_added: false, _subscriptions };
        view.refresh(cx);
        view.load_fields(window, cx);
        view
    });
    window.open_dialog(cx, move |dialog, _, _| {
        let (view_ok, view_source) = (view.clone(), view.clone());
        let footer = DialogFooter::new()
            .gap_2()
            .child(div().size_full().child(Button::new("breakpoints-view-source").label("View Source").outline().w_full().on_click(move |_, window, cx| view_source.update(cx, |v, cx| v.show_source(window, cx)))))
            .child(DialogAction::new().child(Button::new("breakpoints-done").label("Done").primary()));
        dialog.title("Breakpoints").w(px(860.)).child(view.clone()).footer(footer).on_ok(move |_, _, cx| {
            view_ok.update(cx, |v, cx| {
                if v.adding.is_some() || std::mem::take(&mut v.just_added) {
                    return false;
                }
                v.apply(|_| {}, cx);
                true
            })
        })
    });
}
