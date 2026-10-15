//! Line breakpoints and the execution point in the editor, as IntelliJ's
//! new UI shows them: a click on a line number (or Ctrl+F8) toggles a
//! breakpoint, drawn over the line number, its line tinted red; while
//! paused, the frame's line is tinted blue. Run to Cursor (Alt+F9) and
//! Debug the test at the caret (Ctrl+Shift+F9) start here too.

use gpui_kit::component::input::RopeExt as _;
use gpui_kit::{Bounds, Context, DispatchPhase, IntoElement, KeyBinding, MouseButton, MouseDownEvent, Pixels, Styled as _, Window, actions, canvas, point, px, size};

use super::FileEditor;
use crate::domain::testing;
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
                    for (line, _) in &marks {
                        tint(*line, breakpoint_tint, window);
                    }
                    if let Some(line) = execution {
                        tint(line, point_tint, window);
                    }
                    for (line, verified) in &marks {
                        let Some((_, top)) = rows.iter().find(|(l, _)| l == line) else { continue };
                        // Over the line number, as the new UI draws it.
                        let cell = Bounds::new(point(column.0, *top), size(column.1 - column.0, line_height));
                        if !bounds.contains(&cell.center()) {
                            continue;
                        }
                        window.paint_quad(gpui_kit::fill(cell, background));
                        let icon_size = px(14.);
                        let rect = Bounds::new(point(column.1 - icon_size - px(4.), *top + (line_height - icon_size) / 2.), size(icon_size, icon_size));
                        let icon = if *verified { icons::BREAKPOINT_VALID } else { icons::BREAKPOINT };
                        for (path, rgba) in if dark { icon.dark } else { icon.light } {
                            let color: gpui_kit::Hsla = gpui_kit::rgba(*rgba).into();
                            window.paint_svg(rect, (*path).into(), None, gpui_kit::TransformationMatrix::unit(), color, cx).ok();
                        }
                    }
                    // A click on a line number toggles its breakpoint.
                    let (file, service) = (file.clone(), service.clone());
                    window.on_mouse_event(move |e: &MouseDownEvent, phase, _, cx| {
                        if phase != DispatchPhase::Capture || e.button != MouseButton::Left || e.click_count != 1 {
                            return;
                        }
                        if e.position.x < column.0 || e.position.x >= column.1 || !bounds.contains(&e.position) {
                            return;
                        }
                        let Some((line, _)) = rows.iter().take_while(|(_, top)| *top <= e.position.y).last() else { return };
                        if e.position.y > rows.last().map_or(e.position.y, |(_, t)| *t + line_height) {
                            return;
                        }
                        cx.stop_propagation();
                        let line = *line;
                        service.update(cx, |s, cx| s.toggle_breakpoint(&file, line, cx));
                    });
                },
            )
            .absolute()
            .inset_0(),
        )
    }
}
