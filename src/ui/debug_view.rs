//! The Debug tool window, as Android Studio's (new UI): the session's tab,
//! the toolbar (Resume, Pause, Stop, Disconnect; Step Over, Step Into, Step Out, Run
//! to Cursor), and the "Threads & Variables" tab: the thread list and the
//! paused thread's frames on the left, the selected frame's variables as a
//! tree on the right; the "Console" tab has what the session printed.

use std::ops::Range;

use gpui_kit::component::{
    Disableable as _, Icon, Sizable as _, h_flex,
    input::{Input, InputEvent, InputState},
    button::{Button, ButtonVariants as _},
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Div, ElementId, Stateful, Entity, EventEmitter, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, UniformListScrollHandle, Window, div, prelude::FluentBuilder as _, px, uniform_list,
};

use crate::domain::debugger::{self, StepDepth, Value};
use crate::theme::ActivePalette as _;
use crate::ui::as_icons::{self as icons, AsIcon};
use crate::ui::common::tool_button;
use crate::ui::debug_service::{DebugRun, DebugService, DebugStatus, NodeKind, VarNode};
use crate::ui::run_views::{RunViewEvent, stop_label};

const ROW: f32 = 22.;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Variables,
    Console,
}

/// A visible row of the Variables tree.
pub(crate) struct VarRow {
    pub(crate) path: Vec<usize>,
    depth: usize,
    pub(crate) kind: NodeKind,
    name: SharedString,
    value: SharedString,
    /// The object's `toString()`, a collection's size.
    detail: Option<SharedString>,
    /// A watch or evaluation that failed.
    error: Option<SharedString>,
    icon: AsIcon,
    pub(crate) expandable: bool,
    expanded: bool,
    loading: bool,
}

pub struct DebugView {
    service: Entity<DebugService>,
    tab: Tab,
    rows: Vec<VarRow>,
    frames_scroll: UniformListScrollHandle,
    vars_scroll: UniformListScrollHandle,
    /// The evaluate field over the Variables view.
    expression: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<RunViewEvent> for DebugView {}

/// IntelliJ's icon for a value: an object, an array, a primitive.
fn value_icon(value: &Value) -> AsIcon {
    match value {
        Value::Array { .. } => icons::DB_ARRAY,
        Value::Object { .. } | Value::String { .. } => icons::DB_OBJECT,
        Value::Null => icons::DB_OBJECT,
        _ => icons::DB_PRIMITIVE,
    }
}

pub(crate) fn flatten(nodes: &[VarNode], depth: usize, prefix: &mut Vec<usize>, out: &mut Vec<VarRow>) {
    for (ix, node) in nodes.iter().enumerate() {
        prefix.push(ix);
        let expandable = node.error.is_none() && node.var.value.object_id().is_some();
        let icon = match (depth, node.kind) {
            (0, NodeKind::Watch(_)) => icons::WATCH,
            (0, NodeKind::Evaluation) => icons::EVALUATION_RESULT,
            // The thrown exception at an exception breakpoint.
            (0, NodeKind::Variable) if node.var.name == "Exception" => icons::BREAKPOINT_EXCEPTION,
            _ => value_icon(&node.var.value),
        };
        out.push(VarRow {
            path: prefix.clone(),
            depth,
            kind: if depth == 0 { node.kind } else { NodeKind::Variable },
            name: node.var.name.clone().into(),
            value: debugger::value_text(&node.var.value).into(),
            detail: node.detail.clone().map(Into::into),
            error: node.error.clone().map(Into::into),
            icon,
            expandable,
            expanded: node.expanded,
            loading: node.expanded && node.children.is_none(),
        });
        if node.expanded {
            if let Some(children) = &node.children {
                flatten(children, depth + 1, prefix, out);
            }
        }
        prefix.pop();
    }
}

/// A Variables tree row as IntelliJ draws it: the chevron, the value's
/// icon, `name = value` and its `toString()` greyed (or the error in red).
pub(crate) fn var_row(id: impl Into<ElementId>, row: &VarRow, palette: &crate::theme::Palette) -> Stateful<Div> {
    let chevron = row.expandable.then(|| Icon::new(if row.expanded { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT }).xsmall());
    h_flex()
        .id(id)
        .h(px(ROW))
        .pl(px(6. + 16. * row.depth as f32))
        .pr_2()
        .gap_1()
        .text_sm()
        .whitespace_nowrap()
        .hover(|s| s.bg(palette.hover))
        .child(div().w(px(14.)).flex_none().children(chevron))
        .child(Icon::new(row.icon).xsmall())
        .child(div().child(row.name.clone()))
        .child(div().text_color(palette.text_secondary).child("="))
        .map(|el| match &row.error {
            Some(error) => el.child(div().overflow_hidden().text_ellipsis().text_color(palette.status_conflict).child(error.clone())),
            None => el
                .child(div().flex_none().child(row.value.clone()))
                .children(row.detail.clone().map(|d| div().overflow_hidden().text_ellipsis().text_color(palette.text_secondary).child(d))),
        })
        .when(row.loading, |el| el.child(div().text_color(palette.text_secondary).child("Collecting data…")))
        .child(div().flex_1())
}

/// F9, F8, F7, Shift+F8, Alt+F9 (⌥⌘R, F8, F7, ⇧F8, ⌥F9 on macOS).
fn shortcut(action: &str) -> &'static str {
    let mac = cfg!(target_os = "macos");
    match action {
        "resume" => if mac { "⌥⌘R" } else { "F9" },
        "over" => "F8",
        "into" => "F7",
        "out" => "Shift+F8",
        "cursor" => if mac { "⌥F9" } else { "Alt+F9" },
        _ => "",
    }
}

impl DebugView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let service = DebugService::global(cx);
        let placeholder = if cfg!(target_os = "macos") { "Evaluate expression (↩) or add a watch (⇧⌘↩)" } else { "Evaluate expression (Enter) or add a watch (Ctrl+Shift+Enter)" };
        let expression = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let _subscriptions = vec![
            cx.observe(&service, |_, _, cx| cx.notify()),
            // Enter evaluates; Ctrl+Shift+Enter adds a watch, as IntelliJ's field.
            cx.subscribe_in(&expression, window, |this, input, event: &InputEvent, window, cx| {
                let InputEvent::PressEnter { secondary, shift } = event else { return };
                let text = input.read(cx).value().to_string();
                if text.trim().is_empty() {
                    return;
                }
                if *secondary && *shift {
                    this.service.update(cx, |s, cx| s.add_watch(text, cx));
                    input.update(cx, |s, cx| s.set_value("", window, cx));
                } else {
                    this.service.update(cx, |s, cx| s.evaluate(text, cx));
                }
            }),
        ];
        Self { service, tab: Tab::Variables, rows: Vec::new(), frames_scroll: UniformListScrollHandle::new(), vars_scroll: UniformListScrollHandle::new(), expression, _subscriptions }
    }

    fn render_toolbar(&self, run: &DebugRun, cx: &Context<Self>) -> AnyElement {
        let paused = run.status == DebugStatus::Paused;
        let running = run.status == DebugStatus::Running;
        let active = run.status != DebugStatus::Ended;
        let service = self.service.clone();
        let button = |id: &'static str, icon: AsIcon, tip: String, enabled: bool, f: fn(&mut DebugService, &mut Context<DebugService>)| {
            let service = service.clone();
            div().when(!enabled, |el| el.opacity(0.4)).child(tool_button(id, icon, tip).disabled(!enabled).on_click(move |_, _, cx| service.update(cx, |s, cx| f(s, cx))))
        };
        h_flex()
            .h(px(30.))
            .px_1()
            .gap_0p5()
            .flex_none()
            .border_b_1()
            .border_color(cx.palette().border)
            .child(button("debug-resume", icons::RESUME, format!("Resume Program  {}", shortcut("resume")), paused, |s, cx| s.resume(cx)))
            .child(button("debug-pause", icons::PAUSE, "Pause Program".into(), running, |s, cx| s.pause(cx)))
            .child(button("debug-stop", icons::STOP, stop_label().into(), active, |s, cx| s.stop(cx)))
            .child(button("debug-disconnect", icons::EXIT, "Disconnect: the program runs on without the debugger".into(), paused || running, |s, cx| s.detach(cx)))
            .child(div().w(px(1.)).h(px(16.)).mx_1().bg(cx.palette().border))
            .child(button("debug-step-over", icons::STEP_OVER, format!("Step Over  {}", shortcut("over")), paused, |s, cx| s.step(StepDepth::Over, cx)))
            .child(button("debug-step-into", icons::STEP_INTO, format!("Step Into  {}", shortcut("into")), paused, |s, cx| s.step(StepDepth::Into, cx)))
            .child(button("debug-step-out", icons::STEP_OUT, format!("Step Out  {}", shortcut("out")), paused, |s, cx| s.step(StepDepth::Out, cx)))
            .child(
                div().when(!paused, |el| el.opacity(0.4)).child(
                    tool_button("debug-run-to-cursor", icons::RUN_TO_CURSOR, format!("Run to Cursor  {}", shortcut("cursor")))
                        .disabled(!paused)
                        .on_click(cx.listener(|_, _, window, cx| window.dispatch_action(Box::new(crate::ui::file_editor::RunToCursor), cx))),
                ),
            )
            .child(div().w(px(1.)).h(px(16.)).mx_1().bg(cx.palette().border))
            .child(
                div().when(!paused, |el| el.opacity(0.4)).child(
                    tool_button("debug-evaluate", icons::EVALUATE_EXPRESSION, format!("Evaluate Expression  {}", if cfg!(target_os = "macos") { "⌥F8" } else { "Alt+F8" }))
                        .disabled(!paused)
                        .on_click(cx.listener(|this, _, window, cx| {
                            let text = this.expression.read(cx).value().to_string();
                            crate::ui::evaluate_dialog::open(text, window, cx);
                        })),
                ),
            )
            .child(div().flex_1())
            .child(self.render_tabs(cx))
            .into_any_element()
    }

    fn render_tabs(&self, cx: &Context<Self>) -> AnyElement {
        let palette = cx.palette().clone();
        let tab = |id: &'static str, label: &'static str, which: Tab, active: bool| {
            h_flex()
                .id(id)
                .h_full()
                .px_2()
                .text_sm()
                .cursor_pointer()
                .border_b_2()
                .border_color(if active { palette.accent } else { gpui_kit::transparent_black() })
                .when(!active, |el| el.hover(|s| s.bg(palette.hover)))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.tab = which;
                    cx.notify();
                }))
        };
        h_flex()
            .h_full()
            .gap_0p5()
            .child(tab("debug-tab-vars", "Threads & Variables", Tab::Variables, self.tab == Tab::Variables))
            .child(tab("debug-tab-console", "Console", Tab::Console, self.tab == Tab::Console))
            .child(tool_button("debug-hide", icons::HIDE, "Hide").on_click(cx.listener(|_, _, _, cx| cx.emit(RunViewEvent::Hide))))
            .into_any_element()
    }

    /// The thread list: the selected thread, and a dropdown of the others.
    fn render_thread_picker(&self, run: &DebugRun, cx: &Context<Self>) -> AnyElement {
        let palette = cx.palette().clone();
        let current = run.thread.and_then(|id| run.threads.iter().find(|t| t.id == id));
        let label = current.map(debugger::thread_text).unwrap_or_default();
        let threads = run.threads.clone();
        let selected = run.thread;
        let service = self.service.clone();
        h_flex()
            .h(px(28.))
            .px_1()
            .flex_none()
            .border_b_1()
            .border_color(palette.border)
            .child(
                Button::new("debug-threads")
                    .ghost()
                    .small()
                    .icon(Icon::new(icons::THREAD_AT_BREAKPOINT))
                    .label(label)
                    .child(Icon::new(icons::CHEVRON_DOWN).xsmall())
                    .dropdown_menu(move |mut menu, _, _| {
                        for thread in &threads {
                            let (id, service) = (thread.id, service.clone());
                            menu = menu.item(PopupMenuItem::new(debugger::thread_text(thread)).checked(Some(id) == selected).on_click(move |_, _, cx| {
                                service.update(cx, |s, cx| s.select_thread(id, cx));
                            }));
                        }
                        menu
                    }),
            )
            .into_any_element()
    }

    fn render_frames(&mut self, range: Range<usize>, _: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let palette = cx.palette().clone();
        let service = self.service.read(cx);
        let Some(run) = service.run.as_ref() else { return Vec::new() };
        range
            .filter_map(|ix| run.frames.get(ix).map(|f| (ix, f)))
            .map(|(ix, frame)| {
                let selected = ix == run.frame;
                // Frames outside the project's sources are greyed, as IntelliJ's library frames.
                let library = frame.class.starts_with("java.") || frame.class.starts_with("jdk.") || frame.class.starts_with("sun.") || frame.class.starts_with("org.junit.") || frame.class.starts_with("org.gradle.");
                h_flex()
                    .id(("debug-frame", ix))
                    .h(px(ROW))
                    .px_2()
                    .gap_1()
                    .text_sm()
                    .whitespace_nowrap()
                    .when(selected, |el| el.bg(palette.selection))
                    .when(!selected, |el| el.hover(|s| s.bg(palette.hover)))
                    .when(library, |el| el.text_color(palette.text_secondary))
                    .child(Icon::new(icons::FRAME).xsmall())
                    .child(debugger::frame_text(frame))
                    .on_click(cx.listener(move |this, _, _, cx| this.service.update(cx, |s, cx| s.select_frame(ix, cx))))
                    .into_any_element()
            })
            .collect()
    }

    fn render_variables(&mut self, range: Range<usize>, _: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let palette = cx.palette().clone();
        range
            .filter_map(|ix| self.rows.get(ix).map(|r| (ix, r)))
            .map(|(ix, row)| {
                let path = row.path.clone();
                var_row(("debug-var", ix), row, &palette)
                    .when_some(match row.kind {
                        NodeKind::Watch(ix) => Some(ix),
                        _ => None,
                    }, |el, ix| {
                        el.child(tool_button(("debug-remove-watch", ix), icons::CLOSE_SMALL, "Remove Watch").on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.service.update(cx, |s, cx| s.remove_watch(ix, cx));
                        })))
                    })
                    .when(row.kind == NodeKind::Evaluation, |el| {
                        el.child(tool_button("debug-clear-evaluation", icons::CLOSE_SMALL, "Clear").on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.service.update(cx, |s, cx| s.clear_evaluation(cx));
                        })))
                    })
                    .when(row.expandable, |el| el.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| this.service.update(cx, |s, cx| s.toggle_variable(&path, cx)))))
                    .into_any_element()
            })
            .collect()
    }

    fn render_threads_and_variables(&mut self, run: &DebugRun, cx: &mut Context<Self>) -> AnyElement {
        let palette = cx.palette().clone();
        let paused = run.status == DebugStatus::Paused;
        let empty = |text: &'static str| div().p_4().text_sm().text_color(palette.text_secondary).child(text);
        let frames: AnyElement = if paused && !run.frames.is_empty() {
            uniform_list("debug-frames", run.frames.len(), cx.processor(Self::render_frames)).track_scroll(&self.frames_scroll).size_full().into_any_element()
        } else {
            empty(match run.status {
                DebugStatus::Connecting => "Waiting for the test JVM…",
                DebugStatus::Running => "The application is running",
                _ => "Frames are not available",
            })
            .into_any_element()
        };
        let mut rows = Vec::new();
        flatten(&run.variables, 0, &mut Vec::new(), &mut rows);
        self.rows = rows;
        let variables: AnyElement = if !self.rows.is_empty() {
            uniform_list("debug-vars", self.rows.len(), cx.processor(Self::render_variables)).track_scroll(&self.vars_scroll).size_full().into_any_element()
        } else if run.variables_loading {
            empty("Collecting data…").into_any_element()
        } else {
            empty("Variables are not available").into_any_element()
        };
        h_flex()
            .flex_1()
            .min_h_0()
            .child(
                v_flex()
                    .w(px(360.))
                    .flex_none()
                    .h_full()
                    .border_r_1()
                    .border_color(palette.border)
                    .when(paused, |el| el.child(self.render_thread_picker(run, cx)))
                    .child(div().flex_1().min_h_0().child(frames)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    // The evaluate field, as the new UI puts it over the variables.
                    .when(paused, |el| {
                        el.child(
                            h_flex()
                                .h(px(30.))
                                .px_1()
                                .flex_none()
                                .border_b_1()
                                .border_color(palette.border)
                                .child(Input::new(&self.expression).small().appearance(false).prefix(Icon::new(icons::EVALUATION_RESULT).xsmall()))
                                .child(tool_button("debug-add-watch", icons::ADD, "New Watch").on_click(cx.listener(|this, _, window, cx| {
                                    let text = this.expression.read(cx).value().to_string();
                                    if text.trim().is_empty() {
                                        this.expression.update(cx, |s, cx| s.focus(window, cx));
                                        return;
                                    }
                                    this.service.update(cx, |s, cx| s.add_watch(text, cx));
                                    this.expression.update(cx, |s, cx| s.set_value("", window, cx));
                                }))),
                        )
                    })
                    .child(div().flex_1().min_h_0().child(variables)),
            )
            .into_any_element()
    }

    fn render_console(&self, run: &DebugRun, cx: &Context<Self>) -> AnyElement {
        let mono = {
            use gpui_kit::component::ActiveTheme as _;
            cx.theme().mono_font_family.clone()
        };
        v_flex()
            .id("debug-console")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_2()
            .font_family(mono)
            .text_xs()
            .children(run.console.iter().map(|l| div().child(l.clone())))
            .into_any_element()
    }
}

impl Render for DebugView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        // A copy: the panes borrow the view mutably.
        let Some(run) = self.service.read(cx).run.clone() else {
            return v_flex()
                .size_full()
                .bg(palette.panel)
                .child(div().p_4().text_sm().text_color(palette.text_secondary).child("Nothing is being debugged. Click the run icon in the gutter of a test and choose Debug, or press Ctrl+Shift+F9 in it."))
                .into_any_element();
        };
        let (dot, status) = match run.status {
            DebugStatus::Connecting => (palette.text_secondary, "connecting"),
            DebugStatus::Running => (palette.status_added, "running"),
            DebugStatus::Paused => (palette.ref_head, "paused"),
            DebugStatus::Ended => (palette.text_secondary, "ended"),
        };
        let header = h_flex()
            .h(px(30.))
            .px_2()
            .gap_1()
            .flex_none()
            .border_b_1()
            .border_color(palette.border)
            .text_sm()
            .child(Icon::new(icons::DEBUG).xsmall())
            .child(div().font_weight(FontWeight::SEMIBOLD).child(run.name.clone()))
            .child(div().size(px(6.)).rounded_full().bg(dot))
            .child(div().text_xs().text_color(palette.text_secondary).child(status))
            .when_some(run.exception.clone().filter(|_| run.status == DebugStatus::Paused), |el, text| {
                el.child(Icon::new(icons::BREAKPOINT_EXCEPTION).xsmall()).child(div().text_xs().text_color(palette.status_conflict).child(text))
            });
        let toolbar = self.render_toolbar(&run, cx);
        let body = match self.tab {
            Tab::Variables => self.render_threads_and_variables(&run, cx),
            Tab::Console => self.render_console(&run, cx),
        };
        v_flex().size_full().bg(palette.panel).child(header).child(toolbar).child(body).into_any_element()
    }
}
