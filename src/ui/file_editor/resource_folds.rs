//! Android Studio's "Android String References" folding: app resource
//! references shown as their values (`getString(R.string.hello)` →
//! `"Hello"`), expanded on the caret's line like IntelliJ expands a fold
//! the caret enters.

use std::ops::Range;

use gpui_kit::component::input::RopeExt as _;
use gpui_kit::{Context, SharedString};

use super::FileEditor;

/// The folds of the text at one edit count, each with its line.
#[derive(Default)]
pub(super) struct ResourceFolds {
    folds: Vec<(usize, Range<usize>, SharedString)>,
    revision: u64,
    /// The caret lines the shown folds leave out, once applied.
    applied: Option<Vec<usize>>,
}

impl FileEditor {
    /// The folds computed for `text` (as it was at edit `revision`).
    pub(super) fn set_resource_folds(&mut self, folds: Vec<(Range<usize>, String)>, text: &str, revision: u64, cx: &mut Context<Self>) {
        let mut line = 0;
        let mut at = 0;
        let folds = folds
            .into_iter()
            .map(|(range, label)| {
                line += text[at..range.start].matches('\n').count();
                at = range.start;
                (line, range, SharedString::from(label))
            })
            .collect();
        self.resource_folds = ResourceFolds { folds, revision, applied: None };
        self.follow_resource_folds(cx);
    }

    /// Shows the folds except on the lines that hold a caret or selection.
    /// Folds of an older text wait for the next computation; the editor
    /// moves the shown ones with each edit meanwhile.
    pub(super) fn follow_resource_folds(&mut self, cx: &mut Context<Self>) {
        let folds = &self.resource_folds;
        if folds.revision != self.server_edits || (folds.folds.is_empty() && folds.applied.is_some()) {
            return;
        }
        let state = self.state.read(cx);
        let text = state.text();
        let mut lines: Vec<usize> = state.selection_ranges().into_iter().flat_map(|r| text.offset_to_point(r.start).row..=text.offset_to_point(r.end).row).collect();
        lines.sort_unstable();
        lines.dedup();
        if folds.applied.as_ref() == Some(&lines) {
            return;
        }
        let shown: Vec<(Range<usize>, SharedString)> = folds.folds.iter().filter(|(line, _, _)| lines.binary_search(line).is_err()).map(|(_, range, label)| (range.clone(), label.clone())).collect();
        self.resource_folds.applied = Some(lines);
        self.state.update(cx, |state, cx| state.set_display_folds(shown, cx));
    }
}
