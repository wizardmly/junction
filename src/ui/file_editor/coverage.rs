//! The last Run with Coverage's lines in the editor's gutter, as IntelliJ
//! marks them: a green bar on lines the tests ran, yellow on lines they ran
//! partly (a branch missed), red on lines they never reached.

use gpui_kit::component::input::RopeExt as _;
use gpui_kit::{Bounds, Context, IntoElement, Pixels, Styled as _, canvas, fill, point, px, rgb, size};

use super::{FileEditor, MARKER_GAP, MARKER_WIDTH};
use crate::domain::coverage::LineCoverage;
use crate::theme::ActivePalette as _;
use crate::ui::test_service::{TestService, TestServiceEvent};

impl FileEditor {
    /// Follows the Coverage window: a new report or Hide Coverage.
    pub(super) fn watch_coverage(&mut self, cx: &mut Context<Self>) {
        let Some(file) = self.bookmark_file() else { return };
        if !(file.ends_with(".java") || file.ends_with(".kt")) {
            return;
        }
        let tests = TestService::global(cx);
        self._subscriptions.push(cx.subscribe(&tests, |this, _, event: &TestServiceEvent, cx| {
            if matches!(event, TestServiceEvent::Coverage) {
                this.load_coverage(cx);
            }
        }));
        self.load_coverage(cx);
    }

    fn load_coverage(&mut self, cx: &mut Context<Self>) {
        let report = TestService::try_global(cx).and_then(|s| s.read(cx).coverage.clone());
        let lines = match (report, self.bookmark_file()) {
            (Some(report), Some(file)) => report.file(&file).map(|f| f.lines.iter().filter(|(n, _)| **n > 0).map(|(n, c)| (n - 1, *c)).collect()).unwrap_or_default(),
            _ => Vec::new(),
        };
        if lines != self.coverage {
            self.coverage = lines;
            cx.notify();
        }
    }

    /// The bars, in the change markers' column (those paint over them).
    pub(super) fn coverage_strip(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let state = self.state.clone();
        let lines = self.coverage.clone();
        let dark = cx.palette().dark;
        let color = move |c: LineCoverage| match (c, dark) {
            (LineCoverage::Full, false) => rgb(0x5fb865),
            (LineCoverage::Full, true) => rgb(0x4e8c4a),
            (LineCoverage::Partial, false) => rgb(0xe2b33d),
            (LineCoverage::Partial, true) => rgb(0xa58a33),
            (LineCoverage::None, false) => rgb(0xf08080),
            (LineCoverage::None, true) => rgb(0xa54646),
        };
        canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let state = state.read(cx);
                let (Some(line_height), Some(visible)) = (state.line_height(), state.visible_row_range()) else { return };
                let rope = state.text();
                let count = rope.lines_len().max(1);
                for (line, coverage) in &lines {
                    let line = *line as usize;
                    if line < visible.start || line > visible.end || line >= count {
                        continue;
                    }
                    let at = rope.line_start_offset(line);
                    let Some(text) = state.range_to_bounds(&(at..at)) else { continue };
                    let x = text.left() - px(MARKER_GAP + MARKER_WIDTH + 1.);
                    let rect: Bounds<Pixels> = Bounds::new(point(x, text.top()), size(px(MARKER_WIDTH + 2.), line_height)).intersect(&bounds);
                    if rect.size.height > px(0.) {
                        window.paint_quad(fill(rect, color(*coverage)));
                    }
                }
            },
        )
        .absolute()
        .inset_0()
    }
}
