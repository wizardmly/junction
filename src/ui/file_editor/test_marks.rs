//! Android Studio's run icons in the editor gutter of a test file: ▶▶ on a
//! test class, ▶ on a test method, showing the last run's outcome (green
//! check, yellow or red cross). A click opens the "Run 'ExampleUnitTest'"
//! popup; Ctrl+Shift+F10 runs the test at the caret.

use gpui_kit::component::input::RopeExt as _;
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AppContext as _, Bounds, Context, DispatchPhase, InteractiveElement as _, IntoElement, KeyBinding, MouseButton, MouseDownEvent, ParentElement as _, Pixels,
    StatefulInteractiveElement as _, Styled as _, Window, actions, anchored, canvas, deferred, div, point, px, size,
};

use super::{FileEditor, GUTTER_ICON, GUTTER_ICON_GAP, GUTTER_ICON_SLOT};
use crate::domain::testing::{self, Outcome, TestMark};
use crate::theme::ActivePalette as _;
use crate::ui::as_icons::{self as icons, AsIcon};
use crate::ui::test_service::TestService;

actions!(test_marks, [RunTestAtCaret]);

pub fn init(cx: &mut gpui_kit::App) {
    // Run 'ExampleUnitTest': Ctrl+Shift+F10 (⌃⇧R on macOS).
    cx.bind_keys([KeyBinding::new(if cfg!(target_os = "macos") { "ctrl-shift-r" } else { "ctrl-shift-f10" }, RunTestAtCaret, Some(super::CONTEXT))]);
}

pub(super) fn run_shortcut() -> &'static str {
    if cfg!(target_os = "macos") { "⌃⇧R" } else { "Ctrl+Shift+F10" }
}

/// The gutter icon of a test, by its last outcome.
fn gutter_icon(mark: &TestMark, outcome: Option<Outcome>) -> AsIcon {
    match outcome {
        Some(Outcome::Passed) => icons::GUTTER_RUN_SUCCESS,
        Some(Outcome::Failed) => icons::GUTTER_RUN_FAILED,
        Some(Outcome::Error) => icons::GUTTER_RUN_ERROR,
        _ if mark.method.is_none() => icons::GUTTER_RUN_CLASS,
        _ => icons::GUTTER_RUN,
    }
}

impl FileEditor {
    /// The file's test classes and methods, recomputed from the text (a
    /// moment after typing). Only files in test source sets have them.
    pub(super) fn update_test_marks(&mut self, debounce: bool, cx: &mut Context<Self>) {
        let Some(file) = self.bookmark_file() else { return };
        if !(file.ends_with(".kt") || file.ends_with(".java")) || testing::test_kind(&file).is_none() {
            return;
        }
        let text = self.text(cx);
        self.test_marks_task = Some(cx.spawn(async move |this, cx| {
            if debounce {
                cx.background_executor().timer(std::time::Duration::from_millis(400)).await;
            }
            let marks = cx.background_spawn(async move { testing::find_tests(&text) }).await;
            this.update(cx, |this, cx| {
                if this.test_marks != marks {
                    this.test_marks = marks;
                    this.apply_gutter_extra(cx);
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// Ctrl+Shift+F10: the test method at the caret, else its class.
    pub(super) fn run_test_at_caret(&mut self, _: &RunTestAtCaret, _: &mut Window, cx: &mut Context<Self>) {
        let line = self.state.read(cx).cursor_position().line;
        let Some(mark) = testing::test_at(&self.test_marks, line).cloned() else { return };
        self.run_test(mark, cx);
    }

    /// Debug: the test with its JVM waiting for the debugger.
    pub(super) fn debug_test(&mut self, mark: TestMark, cx: &mut Context<Self>) {
        let Some(file) = self.bookmark_file() else { return };
        let marks = self.test_marks.clone();
        self.test_menu = None;
        crate::ui::debug_service::DebugService::global(cx).update(cx, |s, cx| s.debug_test(file.into(), marks, &mark, cx));
    }

    fn run_test(&mut self, mark: TestMark, cx: &mut Context<Self>) {
        let Some(file) = self.bookmark_file() else { return };
        let marks = self.test_marks.clone();
        self.test_menu = None;
        TestService::global(cx).update(cx, |s, cx| s.run_mark(file.into(), marks, &mark, None, cx));
    }

    /// The run icons' column, the gutter's last.
    pub(super) fn test_strip(&self, slot: usize, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        if self.test_marks.is_empty() {
            return None;
        }
        let service = TestService::try_global(cx);
        let marks: Vec<(TestMark, AsIcon)> = self
            .test_marks
            .iter()
            .map(|m| {
                let outcome = service.as_ref().and_then(|s| s.read(cx).outcome_of(&m.class, m.method.as_deref()));
                (m.clone(), gutter_icon(m, outcome))
            })
            .collect();
        let state = self.state.clone();
        let this = cx.entity().downgrade();
        let dark = cx.palette().dark;
        Some(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    let placed: Vec<(Bounds<Pixels>, TestMark, AsIcon)> = {
                        let state = state.read(cx);
                        let (Some(line_height), Some(visible), Some(left)) = (state.line_height(), state.visible_row_range(), state.gutter_extra_left()) else { return };
                        let rope = state.text();
                        let lines = rope.lines_len().max(1);
                        marks
                            .iter()
                            .filter(|(m, _)| (m.line as usize) >= visible.start && (m.line as usize) <= visible.end && (m.line as usize) < lines)
                            .filter_map(|(mark, icon)| {
                                let at = rope.line_start_offset(mark.line as usize);
                                let top = state.range_to_bounds(&(at..at))?.top();
                                let size_px = px(GUTTER_ICON);
                                let rect = Bounds::new(point(left + px(GUTTER_ICON_GAP + 1. + GUTTER_ICON_SLOT * slot as f32), top + (line_height - size_px) / 2.), size(size_px, size_px));
                                bounds.contains(&rect.center()).then(|| (rect, mark.clone(), *icon))
                            })
                            .collect()
                    };
                    let mut hits: Vec<(Bounds<Pixels>, TestMark)> = Vec::new();
                    for (rect, mark, icon) in placed {
                        for (path, rgba) in if dark { icon.dark } else { icon.light } {
                            let color: gpui_kit::Hsla = gpui_kit::rgba(*rgba).into();
                            window.paint_svg(rect, (*path).into(), None, gpui_kit::TransformationMatrix::unit(), color, cx).ok();
                        }
                        hits.push((rect, mark));
                    }
                    let this = this.clone();
                    window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                        if phase != DispatchPhase::Bubble || e.button != MouseButton::Left {
                            return;
                        }
                        let Some((rect, mark)) = hits.iter().find(|(r, _)| r.dilate(px(2.)).contains(&e.position)) else { return };
                        cx.stop_propagation();
                        // The popup under the icon, as IntelliJ's.
                        let (mark, at) = (mark.clone(), rect.bottom_left() + point(px(0.), px(2.)));
                        this.update(cx, |this, cx| {
                            this.test_menu = Some((mark, at));
                            cx.notify();
                        })
                        .ok();
                        window.refresh();
                    });
                },
            )
            .absolute()
            .inset_0(),
        )
    }

    /// The run icon's popup: "Run 'ExampleUnitTest.addition_isCorrect'".
    pub(super) fn render_test_menu(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let (mark, at) = self.test_menu.clone()?;
        let palette = cx.palette().clone();
        let (run, debug) = (mark.clone(), mark.clone());
        Some(
            deferred(
                anchored().position(at).snap_to_window_with_margin(px(8.)).child(
                    v_flex()
                        .id("test-run-menu")
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.test_menu = None;
                            cx.notify();
                        }))
                        .min_w(px(220.))
                        .p_1()
                        .text_sm()
                        .bg(gpui_kit::component::ActiveTheme::theme(&**cx).popover)
                        .border_1()
                        .border_color(palette.border)
                        .rounded_md()
                        .shadow_lg()
                        .child(
                            h_flex()
                                .id("test-run-item")
                                .gap_1p5()
                                .px_2()
                                .py_0p5()
                                .rounded_sm()
                                .cursor_pointer()
                                .hover(|el| el.bg(palette.hover))
                                .child(Icon::new(icons::RUN).xsmall())
                                .child(div().flex_1().child(format!("Run '{}'", mark.name())))
                                .child(div().pl_4().text_xs().text_color(palette.text_secondary).child(run_shortcut()))
                                .on_click(cx.listener(move |this, _, _, cx| this.run_test(run.clone(), cx))),
                        )
                        .child(
                            h_flex()
                                .id("test-debug-item")
                                .gap_1p5()
                                .px_2()
                                .py_0p5()
                                .rounded_sm()
                                .cursor_pointer()
                                .hover(|el| el.bg(palette.hover))
                                .child(Icon::new(icons::DEBUG).xsmall())
                                .child(div().flex_1().child(format!("Debug '{}'", mark.name())))
                                .child(div().pl_4().text_xs().text_color(palette.text_secondary).child(super::breakpoints::debug_shortcut()))
                                .on_click(cx.listener(move |this, _, _, cx| this.debug_test(debug.clone(), cx))),
                        ),
                ),
            )
            .with_priority(2),
        )
    }
}
