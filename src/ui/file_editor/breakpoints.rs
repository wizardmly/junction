//! Line breakpoints and the execution point in the editor, as IntelliJ's
//! new UI shows them: a click on a line number (or Ctrl+F8) toggles a
//! breakpoint, drawn over the line number, its line tinted red; while
//! paused, the frame's line is tinted blue. Shift+click adds a logging
//! breakpoint, drawn yellow: it prints its hits and doesn't suspend. Run to Cursor (Alt+F9) and
//! Debug the test at the caret (Ctrl+Shift+F9) start here too.

use gpui_kit::component::input::RopeExt as _;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Sizable as _, button::{Button, ButtonVariants as _}, h_flex, v_flex};
use gpui_kit::{
    AppContext as _, Bounds, Context, DispatchPhase, Entity, InteractiveElement as _, IntoElement, KeyBinding, MouseButton, MouseDownEvent, ParentElement as _, Pixels, Point, Styled as _,
    Subscription, Window, actions, anchored, canvas, deferred, div, point, px, size,
};

use super::FileEditor;
use crate::domain::testing;
use crate::infra::breakpoint_store::SavedBreakpoint;
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::debug_service::DebugService;

actions!(breakpoints, [ToggleLineBreakpoint, RunToCursor, DebugTestAtCaret]);

pub fn init(cx: &mut gpui_kit::App) {
    let mac = cfg!(target_os = "macos");
    cx.bind_keys([
        KeyBinding::new(if mac { "cmd-f8" } else { "ctrl-f8" }, ToggleLineBreakpoint, Some(super::CONTEXT)),
        KeyBinding::new("alt-f9", RunToCursor, Some(super::CONTEXT)),
        KeyBinding::new(if mac { "ctrl-shift-d" } else { "ctrl-shift-f9" }, DebugTestAtCaret, Some(super::CONTEXT)),
    ]);
}

pub(super) fn debug_shortcut() -> &'static str {
    if cfg!(target_os = "macos") { "⌃⇧D" } else { "Ctrl+Shift+F9" }
}

impl FileEditor {
    /// Ctrl+F8: the caret's line.
    pub(super) fn toggle_line_breakpoint(&mut self, _: &ToggleLineBreakpoint, _: &mut Window, cx: &mut Context<Self>) {
        let Some(file) = self.breakpoint_file() else { return };
        let line = self.state.read(cx).cursor_position().line;
        DebugService::global(cx).update(cx, |s, cx| s.toggle_breakpoint(&file, line, cx));
    }

    /// Alt+F9: the paused program runs to the caret's line.
    pub(super) fn run_to_cursor(&mut self, _: &RunToCursor, _: &mut Window, cx: &mut Context<Self>) {
        let Some(file) = self.breakpoint_file() else { return };
        let line = self.state.read(cx).cursor_position().line;
        DebugService::global(cx).update(cx, |s, cx| s.run_to_line(&file, line, cx));
    }

    /// Ctrl+Shift+F9: debugs the test method at the caret, else its class.
    pub(super) fn debug_test_at_caret(&mut self, _: &DebugTestAtCaret, _: &mut Window, cx: &mut Context<Self>) {
        let line = self.state.read(cx).cursor_position().line;
        if let Some(mark) = testing::test_at(&self.test_marks, line).cloned() {
            self.debug_test(mark, cx);
        }
    }

    /// Moves the breakpoints below an edit by the lines it added or removed.
    pub(super) fn follow_breakpoints(&mut self, cx: &mut Context<Self>) {
        let (lines, caret) = {
            let state = self.state.read(cx);
            (state.text().lines_len(), state.cursor_position().line)
        };
        let delta = lines as i64 - self.line_count as i64;
        self.line_count = lines;
        let Some(file) = self.breakpoint_file().filter(|_| delta != 0) else { return };
        // Added lines end at the caret; removed ones were below it.
        let at = if delta > 0 { caret.saturating_sub(delta as u32) } else { caret };
        DebugService::global(cx).update(cx, |s, cx| s.shift_breakpoints(&file, at, delta, cx));
    }

    /// The file breakpoints are kept under: Kotlin and Java sources only.
    pub(super) fn breakpoint_file(&self) -> Option<String> {
        self.bookmark_file().filter(|f| f.ends_with(".kt") || f.ends_with(".java"))
    }

    /// The breakpoints over the line numbers (a click there toggles one),
    /// and the breakpoint and execution point lines' tints.
    pub(super) fn breakpoint_strip(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let file = self.breakpoint_file()?;
        let service = DebugService::global(cx);
        let (marks, execution) = {
            let s = service.read(cx);
            let point = s.execution_point(cx).filter(|(p, _)| p.to_string_lossy() == file).map(|(_, line)| line);
            (s.breakpoints_of(&file), point)
        };
        let state = self.state.clone();
        let editor = cx.entity().downgrade();
        let palette = cx.palette().clone();
        let dark = palette.dark;
        let background: gpui_kit::Hsla = gpui_kit::rgb(if dark { 0x1e1f22 } else { 0xffffff }).into();
        let breakpoint_tint: gpui_kit::Hsla = gpui_kit::rgba(if dark { 0xdb5c5c26 } else { 0xe557651f }).into();
        let point_tint: gpui_kit::Hsla = gpui_kit::rgba(if dark { 0x548af740 } else { 0x2154d92e }).into();
        Some(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    // Each visible line's top, and the line-number column.
                    let (rows, column, line_height, text_right) = {
                        let state = state.read(cx);
                        let (Some(line_height), Some(visible), Some(extra_left)) = (state.line_height(), state.visible_row_range(), state.gutter_extra_left()) else { return };
                        let input = state.input_bounds();
                        let rope = state.text();
                        let lines = rope.lines_len().max(1);
                        let rows: Vec<(u32, Pixels)> = (visible.start..(visible.end + 1).min(lines))
                            .filter_map(|l| {
                                let at = rope.line_start_offset(l);
                                state.range_to_bounds(&(at..at)).map(|b| (l as u32, b.top()))
                            })
                            .collect();
                        (rows, (input.left(), extra_left), line_height, bounds.right())
                    };
                    let tint = |line: u32, color: gpui_kit::Hsla, window: &mut Window| {
                        if let Some((_, top)) = rows.iter().find(|(l, _)| *l == line) {
                            let rect = Bounds::new(point(column.1, *top), size(text_right - column.1, line_height));
                            window.paint_quad(gpui_kit::fill(rect, color));
                        }
                    };
                    for mark in marks.iter().filter(|m| m.enabled) {
                        tint(mark.line, breakpoint_tint, window);
                    }
                    if let Some(line) = execution {
                        tint(line, point_tint, window);
                    }
                    for mark in &marks {
                        let Some((_, top)) = rows.iter().find(|(l, _)| *l == mark.line) else { continue };
                        // Over the line number, as the new UI draws it.
                        let cell = Bounds::new(point(column.0, *top), size(column.1 - column.0, line_height));
                        if !bounds.contains(&cell.center()) {
                            continue;
                        }
                        window.paint_quad(gpui_kit::fill(cell, background));
                        let icon_size = px(14.);
                        let rect = Bounds::new(point(column.1 - icon_size - px(4.), *top + (line_height - icon_size) / 2.), size(icon_size, icon_size));
                        let icon = match (mark.enabled, mark.verified, mark.suspend) {
                            (false, _, true) => icons::BREAKPOINT_DISABLED,
                            (true, true, true) => icons::BREAKPOINT_VALID,
                            (true, false, true) => icons::BREAKPOINT,
                            (false, _, false) => icons::BREAKPOINT_UNSUSPENDENT_DISABLED,
                            (true, true, false) => icons::BREAKPOINT_UNSUSPENDENT_VALID,
                            (true, false, false) => icons::BREAKPOINT_UNSUSPENDENT,
                        };
                        for (path, rgba) in if dark { icon.dark } else { icon.light } {
                            let color: gpui_kit::Hsla = gpui_kit::rgba(*rgba).into();
                            window.paint_svg(rect, (*path).into(), None, gpui_kit::TransformationMatrix::unit(), color, cx).ok();
                        }
                        // A condition: a question mark on the dot.
                        if mark.conditional && mark.enabled && !mark.verified {
                            let label: gpui_kit::SharedString = "?".into();
                            let run = gpui_kit::TextRun { len: 1, font: gpui_kit::font(".SystemUIFont"), color: gpui_kit::white(), background_color: None, underline: None, strikethrough: None };
                            let shaped = window.text_system().shape_line(label, px(10.), &[run], None);
                            let x = rect.left() + (rect.size.width - shaped.width) / 2.;
                            let _ = shaped.paint(point(x, rect.top()), rect.size.height, gpui_kit::TextAlign::Left, None, window, cx);
                        }
                    }
                    // A click on a line number toggles its breakpoint (with
                    // Shift, a logging one); a right-click on one opens its
                    // properties.
                    let (file, service, editor) = (file.clone(), service.clone(), editor.clone());
                    let lines: Vec<u32> = marks.iter().map(|m| m.line).collect();
                    window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                        if phase != DispatchPhase::Capture || e.click_count != 1 || !matches!(e.button, MouseButton::Left | MouseButton::Right) {
                            return;
                        }
                        if e.position.x < column.0 || e.position.x >= column.1 || !bounds.contains(&e.position) {
                            return;
                        }
                        let Some((line, _)) = rows.iter().take_while(|(_, top)| *top <= e.position.y).last() else { return };
                        if e.position.y > rows.last().map_or(e.position.y, |(_, t)| *t + line_height) {
                            return;
                        }
                        let line = *line;
                        if e.button == MouseButton::Right {
                            if !lines.contains(&line) {
                                return;
                            }
                            cx.stop_propagation();
                            let at = e.position;
                            editor.update(cx, |this, cx| this.open_breakpoint_popup(line, at, window, cx)).ok();
                            return;
                        }
                        cx.stop_propagation();
                        if e.modifiers.shift {
                            service.update(cx, |s, cx| s.toggle_logging_breakpoint(&file, line, cx));
                        } else {
                            service.update(cx, |s, cx| s.toggle_breakpoint(&file, line, cx));
                        }
                    });
                },
            )
            .absolute()
            .inset_0(),
        )
    }
}

/// The breakpoint popup, as IntelliJ's: Enabled, Suspend execution,
/// Condition, what a hit logs, Remove, Done.
pub(super) struct BreakpointPopup {
    line: u32,
    at: Point<Pixels>,
    enabled: bool,
    suspend: bool,
    log_message: bool,
    log_stack: bool,
    log_expression_on: bool,
    condition: Entity<InputState>,
    log_expression: Entity<InputState>,
    _enter: [Subscription; 2],
}

impl FileEditor {
    fn open_breakpoint_popup(&mut self, line: u32, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(file) = self.breakpoint_file() else { return };
        let Some(saved) = DebugService::global(cx).read(cx).breakpoint(&file, line) else { return };
        let condition = cx.new(|cx| InputState::new(window, cx).default_value(saved.condition.clone().unwrap_or_default()).placeholder("Condition, e.g. i > 10"));
        let log_expression = cx.new(|cx| InputState::new(window, cx).default_value(saved.log_expression.clone().unwrap_or_default()).placeholder("Expression, e.g. \"x = \" + x"));
        let enter = |this: &mut Self, _: &Entity<InputState>, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.close_breakpoint_popup(true, window, cx);
            }
        };
        let _enter = [cx.subscribe_in(&condition, window, enter), cx.subscribe_in(&log_expression, window, enter)];
        condition.update(cx, |s, cx| s.focus(window, cx));
        self.breakpoint_popup = Some(BreakpointPopup {
            line,
            at,
            enabled: saved.enabled,
            suspend: saved.suspend,
            log_message: saved.log_message,
            log_stack: saved.log_stack,
            log_expression_on: saved.log_expression.is_some(),
            condition,
            log_expression,
            _enter,
        });
        cx.notify();
    }

    /// Done applies the popup; a click outside does too, as IntelliJ's.
    fn close_breakpoint_popup(&mut self, apply: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(popup) = self.breakpoint_popup.take() else { return };
        if apply {
            if let Some(file) = self.breakpoint_file() {
                let expression = popup.log_expression.read(cx).value().to_string();
                let updated = SavedBreakpoint {
                    enabled: popup.enabled,
                    condition: Some(popup.condition.read(cx).value().to_string()),
                    suspend: popup.suspend,
                    log_message: popup.log_message,
                    log_stack: popup.log_stack,
                    log_expression: popup.log_expression_on.then_some(expression),
                    ..SavedBreakpoint::new(&file, popup.line)
                };
                DebugService::global(cx).update(cx, |s, cx| s.set_breakpoint_properties(updated, cx));
            }
        }
        self.focus(window, cx);
        cx.notify();
    }

    pub(super) fn render_breakpoint_popup(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let popup = self.breakpoint_popup.as_ref()?;
        let palette = cx.palette().clone();
        let name = std::path::Path::new(&self.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let line = popup.line;
        Some(
            deferred(
                anchored().position(popup.at).snap_to_window_with_margin(px(8.)).child(
                    v_flex()
                        .id("breakpoint-popup")
                        .occlude()
                        .on_mouse_down_out(cx.listener(|this, _, window, cx| this.close_breakpoint_popup(true, window, cx)))
                        .w(px(320.))
                        .p_2()
                        .gap_2()
                        .text_sm()
                        .bg(gpui_kit::component::ActiveTheme::theme(&**cx).popover)
                        .border_1()
                        .border_color(palette.border)
                        .rounded_md()
                        .shadow_lg()
                        .child(div().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(format!("{name}:{}", line + 1)))
                        .child(Checkbox::new("breakpoint-enabled").label("Enabled").checked(popup.enabled).on_click(cx.listener(|this, checked: &bool, _, cx| {
                            if let Some(popup) = this.breakpoint_popup.as_mut() {
                                popup.enabled = *checked;
                            }
                            cx.notify();
                        })))
                        .child(Checkbox::new("breakpoint-suspend").label("Suspend execution").checked(popup.suspend).on_click(cx.listener(|this, checked: &bool, _, cx| {
                            if let Some(popup) = this.breakpoint_popup.as_mut() {
                                popup.suspend = *checked;
                            }
                            cx.notify();
                        })))
                        .child(v_flex().gap_1().child(div().text_color(palette.text_secondary).child("Condition:")).child(Input::new(&popup.condition).small()))
                        .child(
                            v_flex()
                                .gap_1()
                                .child(div().text_color(palette.text_secondary).child("Log:"))
                                .child(Checkbox::new("breakpoint-log-message").label("\"Breakpoint hit\" message").checked(popup.log_message).on_click(cx.listener(|this, checked: &bool, _, cx| {
                                    if let Some(popup) = this.breakpoint_popup.as_mut() {
                                        popup.log_message = *checked;
                                    }
                                    cx.notify();
                                })))
                                .child(Checkbox::new("breakpoint-log-stack").label("Stack trace").checked(popup.log_stack).on_click(cx.listener(|this, checked: &bool, _, cx| {
                                    if let Some(popup) = this.breakpoint_popup.as_mut() {
                                        popup.log_stack = *checked;
                                    }
                                    cx.notify();
                                })))
                                .child(Checkbox::new("breakpoint-log-expression").label("Evaluate and log:").checked(popup.log_expression_on).on_click(cx.listener(|this, checked: &bool, window, cx| {
                                    if let Some(popup) = this.breakpoint_popup.as_mut() {
                                        popup.log_expression_on = *checked;
                                        if *checked {
                                            popup.log_expression.update(cx, |s, cx| s.focus(window, cx));
                                        }
                                    }
                                    cx.notify();
                                })))
                                .child(Input::new(&popup.log_expression).small().disabled(!popup.log_expression_on)),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(div().flex_1())
                                .child(Button::new("breakpoint-remove").ghost().small().label("Remove").on_click(cx.listener(move |this, _, window, cx| {
                                    this.breakpoint_popup = None;
                                    if let Some(file) = this.breakpoint_file() {
                                        DebugService::global(cx).update(cx, |s, cx| s.toggle_breakpoint(&file, line, cx));
                                    }
                                    this.focus(window, cx);
                                    cx.notify();
                                })))
                                .child(Button::new("breakpoint-done").primary().small().label("Done").on_click(cx.listener(|this, _, window, cx| this.close_breakpoint_popup(true, window, cx)))),
                        ),
                ),
            )
            .with_priority(2),
        )
    }
}
