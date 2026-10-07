//! The editable right pane: IntelliJ edits the working-tree side of a diff
//! in place, re-diffs on every change and saves the file.

use std::ops::Range;
use std::time::Duration;

use gpui_kit::{AppContext as _, Context, Window};

use super::{DiffSource, DiffView};
use crate::git::diff;
use crate::ui::text_buffer::{Buffer, EditKind};
use crate::ui::text_panes::{PaneHost, TextPanes};

impl DiffView {
    /// The right pane edits when it is the working-tree file and text.
    pub(super) fn editable(&self) -> bool {
        let working_tree = matches!(
            self.source,
            Some(DiffSource::WorkingTree { .. } | DiffSource::Unstaged { .. } | DiffSource::Between { new: None, .. })
        );
        working_tree
            && !self.diff.binary
            && self.loaded.as_ref().is_some_and(|l| !l.new.contains('\u{FFFD}') && !l.new.starts_with("Subproject commit "))
    }

    /// `>>` on an editable right pane: puts the left lines in place of the
    /// right ones (or after them, with Ctrl: Append), undoably.
    pub(super) fn revert_change(&mut self, change: usize, append: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hunk) = self.diff.hunks.get(change).cloned() else { return };
        let (text, range) = replacement(&self.panes.buffers[0], hunk.old, &self.panes.buffers[1], hunk.new, append);
        let caret = range.start;
        self.panes.edit(range, &text, EditKind::Other, Some(caret));
        self.edited(window, cx);
        cx.notify();
    }

    /// Writes a pending save now (the view is about to show another file).
    pub(super) fn flush_save(&mut self) {
        if self.save_task.take().is_none() {
            return;
        }
        if let (Some(repository), Some(source)) = (&self.repository, &self.source) {
            if let Err(error) = std::fs::write(repository.root().join(source.path()), self.panes.buffers[1].text()) {
                eprintln!("cannot save {}: {error}", source.path());
            }
        }
    }

    /// Saves the right pane to disk shortly after the last edit.
    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        let (Some(repository), Some(source)) = (self.repository.clone(), self.source.clone()) else { return };
        let path = repository.root().join(source.path());
        let text = self.panes.buffers[1].text().to_owned();
        self.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(400)).await;
            let result = cx.background_spawn(async move { std::fs::write(&path, text) }).await;
            this.update(cx, |this, cx| match result {
                Ok(()) => {
                    this.save_task = None;
                    cx.emit(super::FilesChanged);
                }
                Err(error) => {
                    this.error = Some(format!("Cannot save: {error}"));
                    cx.notify();
                }
            })
            .ok();
        }));
    }
}

/// The text that puts `from_lines` of one buffer in place of (or, with
/// `append`, after) `to_lines` of another, and the byte range it replaces.
/// The target keeps its own ending at the end of the file.
pub(crate) fn replacement(from: &Buffer, from_lines: Range<usize>, to: &Buffer, to_lines: Range<usize>, append: bool) -> (String, Range<usize>) {
    let mut text = from.lines_text(from_lines).to_owned();
    let mut range = to.lines_span(to_lines);
    if append {
        range = range.end..range.end;
    }
    if range.end == to.text().len() {
        let had_break = if range.is_empty() { to.text().is_empty() || to.text().ends_with('\n') } else { to.text()[range.clone()].ends_with('\n') };
        if had_break && !text.is_empty() && !text.ends_with('\n') {
            text.push_str(to.newline());
        } else if !had_break && text.ends_with('\n') {
            text.truncate(text.trim_end_matches(['\r', '\n']).len());
            if range.is_empty() && !to.text().is_empty() {
                text.insert_str(0, to.newline());
            }
        }
    }
    (text, range)
}

impl PaneHost for DiffView {
    type Extra = ();

    fn panes(&mut self) -> &mut TextPanes {
        &mut self.panes
    }

    fn text_panes(&self) -> &TextPanes {
        &self.panes
    }

    /// Re-diffs, re-highlights and saves soon.
    fn edited(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.panes.line_edits.clear();
        let text = self.panes.buffers[1].text().to_owned();
        if let Some(loaded) = self.loaded.as_mut() {
            loaded.new = text.clone();
        }
        let current = self.current;
        self.diff = diff::compute(self.panes.buffers[0].text(), &text, self.options);
        self.expanded.clear();
        self.current = current.filter(|c| *c < self.diff.changes);
        let language = crate::ui::file_editor::language_for(self.source.as_ref().map(|s| s.path()).unwrap_or_default());
        self.panes.highlight(1, language);
        self.rebuild_rows();
        self.schedule_save(cx);
    }

    fn open_fold(&mut self, id: usize, cx: &mut Context<Self>) {
        self.expanded.insert(id);
        self.rebuild_rows();
        cx.notify();
    }
}

crate::impl_pane_input!(DiffView);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_keeps_the_file_ending() {
        let old = Buffer::new("a\nb\n".into());
        let new = Buffer::new("a\nB".into());
        // Line 1 ("b\n") replaces "B", which had no line break.
        let (text, range) = replacement(&old, 1..2, &new, 1..2, false);
        assert_eq!((text.as_str(), range), ("b", 2..3));
        // Appending after the last line adds a break before it.
        let (text, range) = replacement(&old, 1..2, &new, 1..2, true);
        assert_eq!((text.as_str(), range), ("\nb", 3..3));
    }
}
