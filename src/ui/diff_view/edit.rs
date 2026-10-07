//! Carets, selection and editing in the side-by-side panes: both panes
//! select and copy, the right one edits when it is the working-tree file
//! (as in IntelliJ), and every edit re-diffs and saves.

use std::ops::Range;
use std::time::Duration;

use gpui_kit::{
    AppContext as _, App, Bounds, ClipboardItem, Context, EntityInputHandler, Font, KeyDownEvent, MouseDownEvent, MouseMoveEvent, Pixels,
    SharedString, TextRun, UTF16Selection, Window, font, point, px, size,
};

use super::{DiffSource, DiffView, LINE_HEIGHT, expand_tabs};
use crate::git::diff;
use crate::ui::diff_panes::PaneRow;
use crate::ui::text_buffer::{Buffer, EditKind, Selection};

pub(super) const FONT_SIZE: f32 = 12.5;
/// Text starts this far right of its cell's left edge.
pub(super) const TEXT_PADDING: f32 = 8.;

/// Where a buffer line is shown in a pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LineRow {
    Row(usize),
    /// Inside a collapsed fragment: (fold id, its row).
    Fold(usize, usize),
}

/// Maps each buffer line of a pane to its row.
pub(super) fn line_rows(rows: &[PaneRow]) -> Vec<LineRow> {
    let mut out = Vec::new();
    for (ix, row) in rows.iter().enumerate() {
        match row {
            PaneRow::Line { side, .. } => {
                let line = side.line - 1;
                // Lines skipped by nothing should not happen; pad defensively.
                while out.len() < line {
                    out.push(LineRow::Row(ix));
                }
                out.truncate(line);
                out.push(LineRow::Row(ix));
            }
            PaneRow::Fold { id, count } => out.extend(std::iter::repeat_n(LineRow::Fold(*id, ix), *count)),
        }
    }
    out
}

/// Byte offset in a line to its offset in the tab-expanded display text.
pub(super) fn display_offset(text: &str, byte: usize) -> usize {
    expand_tabs(text, &[0..byte]).1[0].end
}

/// The reverse: a display offset (from hit testing) back to a byte offset.
fn byte_offset(text: &str, display: usize) -> usize {
    let mut best = 0;
    for (i, _) in text.char_indices().chain(std::iter::once((text.len(), ' '))) {
        if display_offset(text, i) <= display {
            best = i;
        } else {
            break;
        }
    }
    best
}

pub(super) fn mono_font(cx: &App) -> Font {
    use gpui_kit::component::ActiveTheme as _;
    font(cx.theme().mono_font_family.clone())
}

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

    fn buffer(&self, pane: usize) -> &Buffer {
        &self.buffers[pane]
    }

    /// The x of a byte offset within its line's text, from shaping.
    pub(super) fn x_for(&self, pane: usize, offset: usize, window: &Window, cx: &App) -> f32 {
        let buffer = self.buffer(pane);
        let line = buffer.line_of(offset);
        let range = buffer.line_range(line);
        let text = buffer.line(line);
        let (display, _) = expand_tabs(text, &[]);
        let at = display_offset(text, offset - range.start);
        let shaped = shape(&display, window, cx);
        f32::from(shaped.x_for_index(at))
    }

    /// The pane-local left of the text, before horizontal scrolling.
    pub(super) fn text_left(&self, pane: usize, cx: &App) -> f32 {
        let gutter = if pane == 0 { 0. } else { 3. + super::GUTTER_WIDTH + if self.partial_path(cx).is_some() { super::BUTTON_WIDTH + 2. } else { 0. } };
        gutter + TEXT_PADDING
    }

    /// Where a click lands: the pane, and a buffer offset (or a fold to open).
    fn hit(&self, position: gpui_kit::Point<Pixels>, window: &Window, cx: &App) -> Option<(usize, Result<usize, usize>)> {
        let bounds = self.pane_bounds.get();
        let pane = (0..2).find(|p| bounds[*p].contains(&position)).or_else(|| self.caret.map(|c| c.0))?;
        let b = bounds[pane];
        let (scroll_x, scroll_y) = self.pane_scroll[pane];
        let rows = self.pane_rows(pane);
        let y = f32::from(position.y - b.origin.y) + scroll_y;
        let row = ((y / LINE_HEIGHT).floor().max(0.) as usize).min(rows.len().saturating_sub(1));
        let line = match rows.get(row)? {
            PaneRow::Fold { id, .. } => return Some((pane, Err(*id))),
            PaneRow::Line { side, .. } => side.line - 1,
        };
        let buffer = self.buffer(pane);
        let text = buffer.line(line);
        let (display, _) = expand_tabs(text, &[]);
        let x = f32::from(position.x - b.origin.x) - self.text_left(pane, cx) + scroll_x;
        let shaped = shape(&display, window, cx);
        let index = shaped.closest_index_for_x(px(x.max(0.)));
        Some((pane, Ok(buffer.line_range(line).start + byte_offset(text, index))))
    }

    pub(super) fn on_pane_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let Some((pane, target)) = self.hit(event.position, window, cx) else { return };
        let offset = match target {
            Err(fold) => {
                self.expand_fold(fold, cx);
                return;
            }
            Ok(offset) => offset,
        };
        let buffer = self.buffer(pane);
        let selection = match event.click_count {
            2 => {
                let word = buffer.word_at(offset);
                Selection { anchor: word.start, head: word.end }
            }
            n if n >= 3 => {
                let line = buffer.line_of(offset);
                let end = if line + 1 < buffer.line_count() { buffer.line_range(line + 1).start } else { buffer.line_range(line).end };
                Selection { anchor: buffer.line_range(line).start, head: end }
            }
            _ => match self.caret {
                Some((p, sel)) if p == pane && event.modifiers.shift => Selection { anchor: sel.anchor, head: offset },
                _ => Selection::caret(offset),
            },
        };
        self.history.break_run();
        self.caret = Some((pane, selection));
        self.goal = None;
        self.selecting = true;
        cx.notify();
    }

    pub(super) fn on_pane_mouse_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selecting || event.pressed_button != Some(gpui_kit::MouseButton::Left) {
            self.selecting = false;
            return;
        }
        let Some((pane, sel)) = self.caret else { return };
        // Dragging past an edge scrolls.
        let b = self.pane_bounds.get()[pane];
        let (x, y) = self.pane_scroll[pane];
        if event.position.y < b.origin.y {
            self.scroll_pane_to(pane, x, y - LINE_HEIGHT);
        } else if event.position.y > b.origin.y + b.size.height {
            self.scroll_pane_to(pane, x, y + LINE_HEIGHT);
        }
        let position = point(event.position.x.clamp(b.origin.x, b.origin.x + b.size.width), event.position.y.clamp(b.origin.y, b.origin.y + b.size.height - px(1.)));
        if let Some((p, Ok(offset))) = self.hit(position, window, cx) {
            if p == pane && offset != sel.head {
                self.caret = Some((pane, Selection { anchor: sel.anchor, head: offset }));
                cx.notify();
            }
        }
    }

    /// Moves the caret (extending the selection with Shift) and keeps it in view.
    fn move_caret(&mut self, select: bool, keep_goal: bool, window: &mut Window, cx: &mut Context<Self>, to: impl FnOnce(&Self, usize, Selection) -> usize) {
        let Some((pane, sel)) = self.caret else { return };
        let head = to(self, pane, sel);
        let sel = if select { Selection { anchor: sel.anchor, head } } else { Selection::caret(head) };
        if !keep_goal {
            self.goal = None;
        }
        self.history.break_run();
        self.caret = Some((pane, sel));
        self.reveal_caret(window, cx);
        cx.notify();
    }

    /// Up / Down / Page keys: whole lines, keeping the column it started from.
    fn vertical(&mut self, lines: isize, select: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((pane, sel)) = self.caret else { return };
        let buffer = self.buffer(pane);
        let line = buffer.line_of(sel.head);
        let goal = self.goal.unwrap_or_else(|| {
            let range = buffer.line_range(line);
            display_offset(buffer.line(line), sel.head - range.start)
        });
        let last = self.visible_line_count(pane).saturating_sub(1) as isize;
        let target = (line as isize + lines).clamp(0, last) as usize;
        let text = buffer.line(target);
        let offset = buffer.line_range(target).start + byte_offset(text, goal);
        self.goal = Some(goal);
        self.move_caret(select, true, window, cx, |_, _, _| offset);
    }

    /// Lines the pane shows: the empty line after a final line break counts.
    pub(super) fn visible_line_count(&self, pane: usize) -> usize {
        self.buffer(pane).line_count()
    }

    pub(super) fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.caret.is_none() {
            return;
        }
        let keystroke = &event.keystroke;
        let m = keystroke.modifiers;
        let (ctrl, shift) = (m.secondary(), m.shift);
        if m.alt {
            return;
        }
        let page = ((self.view_height.get() / LINE_HEIGHT) as isize - 1).max(1);
        match (keystroke.key.as_str(), ctrl) {
            ("left", false) => self.move_caret(shift, false, window, cx, |this, pane, sel| {
                if !shift && !sel.is_empty() { sel.range().start } else { this.buffer(pane).prev_char(sel.head) }
            }),
            ("right", false) => self.move_caret(shift, false, window, cx, |this, pane, sel| {
                if !shift && !sel.is_empty() { sel.range().end } else { this.buffer(pane).next_char(sel.head) }
            }),
            ("left", true) => self.move_caret(shift, false, window, cx, |this, pane, sel| this.buffer(pane).prev_word(sel.head)),
            ("right", true) => self.move_caret(shift, false, window, cx, |this, pane, sel| this.buffer(pane).next_word(sel.head)),
            ("up", false) => self.vertical(-1, shift, window, cx),
            ("down", false) => self.vertical(1, shift, window, cx),
            ("pageup", _) => self.vertical(-page, shift, window, cx),
            ("pagedown", _) => self.vertical(page, shift, window, cx),
            // Smart Home: the first non-blank, then the line start.
            ("home", false) => self.move_caret(shift, false, window, cx, |this, pane, sel| {
                let b = this.buffer(pane);
                let line = b.line_of(sel.head);
                let start = b.line_range(line).start;
                let text_start = start + b.indent(line).len();
                if sel.head == text_start { start } else { text_start }
            }),
            ("end", false) => self.move_caret(shift, false, window, cx, |this, pane, sel| {
                let b = this.buffer(pane);
                b.line_range(b.line_of(sel.head)).end
            }),
            ("home", true) => self.move_caret(shift, false, window, cx, |_, _, _| 0),
            ("end", true) => self.move_caret(shift, false, window, cx, |this, pane, _| this.buffer(pane).text().len()),
            ("a", true) => {
                if let Some((pane, _)) = self.caret {
                    let end = self.buffer(pane).text().len();
                    self.caret = Some((pane, Selection { anchor: 0, head: end }));
                    cx.notify();
                }
            }
            ("c", true) | ("insert", true) => self.copy(false, cx),
            ("x", true) => self.copy(true, cx),
            ("z", true) if shift => self.redo(window, cx),
            ("z", true) => self.undo(window, cx),
            ("escape", false) => {
                if let Some((pane, sel)) = self.caret.filter(|c| !c.1.is_empty()) {
                    self.caret = Some((pane, Selection::caret(sel.head)));
                    cx.notify();
                } else {
                    return;
                }
            }
            _ if !self.caret_editable() => return,
            ("backspace", _) => self.delete(false, ctrl, window, cx),
            ("delete", _) => self.delete(true, ctrl, window, cx),
            ("enter", false) => self.newline(window, cx),
            ("tab", false) if shift => self.unindent(window, cx),
            ("tab", false) => {
                let indent = self.indent_unit().to_owned();
                self.insert(&indent, EditKind::Typing, window, cx);
            }
            ("d", true) => self.duplicate_line(window, cx),
            ("y", true) => self.delete_line(window, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn caret_editable(&self) -> bool {
        self.caret.is_some_and(|c| c.0 == 1) && self.editable()
    }

    fn indent_unit(&self) -> &'static str {
        let b = self.buffer(1);
        if (0..b.line_count()).any(|l| b.indent(l).starts_with('\t')) { "\t" } else { "    " }
    }

    /// Copies the selection, or the whole line when nothing is selected
    /// (IntelliJ); with `cut`, removes it too.
    fn copy(&mut self, cut: bool, cx: &mut Context<Self>) {
        let Some((pane, sel)) = self.caret else { return };
        let b = self.buffer(pane);
        let range = if sel.is_empty() {
            let line = b.line_of(sel.head);
            let start = b.line_range(line).start;
            let end = if line + 1 < b.line_count() { b.line_range(line + 1).start } else { b.text().len() };
            start..end
        } else {
            sel.range()
        };
        let mut text = b.text()[range.clone()].to_owned();
        if sel.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        if cut && self.caret_editable() {
            self.edit(range, "", EditKind::Other, None, cx);
            self.after_edit(cx);
        }
    }

    pub(super) fn paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.caret_editable() {
            return;
        }
        let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else { return };
        let text = text.replace("\r\n", "\n").replace('\n', self.buffer(1).newline());
        self.insert(&text, EditKind::Other, window, cx);
    }

    fn undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.caret_editable() {
            return;
        }
        let mut sel = self.caret.map(|c| c.1).unwrap_or_default();
        if self.history.undo(&mut self.buffers[1], &mut sel) {
            self.caret = Some((1, sel));
            self.after_edit(cx);
            self.reveal_caret(window, cx);
        }
    }

    fn redo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.caret_editable() {
            return;
        }
        let mut sel = self.caret.map(|c| c.1).unwrap_or_default();
        if self.history.redo(&mut self.buffers[1], &mut sel) {
            self.caret = Some((1, sel));
            self.after_edit(cx);
            self.reveal_caret(window, cx);
        }
    }

    /// Replaces a range of the editable buffer, recording undo; the caret
    /// lands after the new text unless `caret` says otherwise.
    fn edit(&mut self, range: Range<usize>, text: &str, kind: EditKind, caret: Option<usize>, _cx: &mut Context<Self>) {
        let sel = self.caret.map(|c| c.1).unwrap_or_default();
        self.history.record(&self.buffers[1], sel, kind);
        self.buffers[1].replace(range.clone(), text);
        let head = caret.unwrap_or(range.start + text.len());
        self.caret = Some((1, Selection::caret(head)));
        self.goal = None;
    }

    fn insert(&mut self, text: &str, kind: EditKind, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, sel)) = self.caret else { return };
        let kind = if sel.is_empty() { kind } else { EditKind::Other };
        self.edit(sel.range(), text, kind, None, cx);
        self.after_edit(cx);
        self.reveal_caret(window, cx);
    }

    fn delete(&mut self, forward: bool, word: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, sel)) = self.caret else { return };
        let b = self.buffer(1);
        let range = if !sel.is_empty() {
            sel.range()
        } else if forward {
            sel.head..if word { b.next_word(sel.head) } else { b.next_char(sel.head) }
        } else {
            (if word { b.prev_word(sel.head) } else { b.prev_char(sel.head) })..sel.head
        };
        if range.is_empty() {
            return;
        }
        // A line break deleted at a line end takes its "\r" too.
        let range = if forward && b.text()[range.clone()].starts_with('\r') { range.start..b.next_char(range.start) } else { range };
        // Backspace over "\r\n" removes both.
        let range = if !forward && b.text()[range.clone()] == *"\n" && range.start > 0 && b.text().as_bytes()[range.start - 1] == b'\r' {
            range.start - 1..range.end
        } else {
            range
        };
        self.edit(range, "", EditKind::Deleting, None, cx);
        self.after_edit(cx);
        self.reveal_caret(window, cx);
    }

    /// Enter: a line break keeping the current line's indentation.
    fn newline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, sel)) = self.caret else { return };
        let b = self.buffer(1);
        let line = b.line_of(sel.range().start);
        let indent = b.indent(line);
        let start = b.line_range(line).start;
        let indent = &indent[..indent.len().min(sel.range().start - start)];
        let text = format!("{}{}", b.newline(), indent);
        self.insert(&text, EditKind::Other, window, cx);
    }

    /// Shift+Tab: removes one indent level from the caret's lines.
    fn unindent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, sel)) = self.caret else { return };
        let b = self.buffer(1);
        let (first, last) = (b.line_of(sel.range().start), b.line_of(sel.range().end));
        let unit = self.indent_unit();
        let mut text = String::new();
        let span = b.line_range(first).start..b.line_range(last).end;
        for line in first..=last {
            let l = b.line(line);
            let removed = if l.starts_with(unit) {
                unit.len()
            } else if l.starts_with('\t') {
                1
            } else {
                (l.len() - l.trim_start_matches(' ').len()).min(unit.len())
            };
            text.push_str(&l[removed..]);
            if line < last {
                let r = b.line_range(line);
                text.push_str(&b.text()[r.end..b.line_range(line + 1).start]);
            }
        }
        let caret = span.start + text.len();
        self.edit(span, &text, EditKind::Other, Some(caret), cx);
        self.after_edit(cx);
        self.reveal_caret(window, cx);
    }

    /// Ctrl+D: duplicates the caret's line below it.
    fn duplicate_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, sel)) = self.caret else { return };
        let b = self.buffer(1);
        let line = b.line_of(sel.head);
        let range = b.line_range(line);
        let column = sel.head - range.start;
        let text = format!("{}{}", b.newline(), b.line(line));
        let caret = range.end + b.newline().len() + column;
        self.edit(range.end..range.end, &text, EditKind::Other, Some(caret), cx);
        self.after_edit(cx);
        self.reveal_caret(window, cx);
    }

    /// Ctrl+Y: deletes the caret's line.
    fn delete_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, sel)) = self.caret else { return };
        let b = self.buffer(1);
        let line = b.line_of(sel.head);
        let start = b.line_range(line).start;
        let range = if line + 1 < b.line_count() {
            start..b.line_range(line + 1).start
        } else if line > 0 {
            b.line_range(line - 1).end..b.text().len()
        } else {
            0..b.text().len()
        };
        let caret = range.start.min(b.text().len() - range.len());
        self.edit(range, "", EditKind::Other, Some(caret), cx);
        self.after_edit(cx);
        self.reveal_caret(window, cx);
    }

    /// `>>` on an editable right pane: puts the left lines in place of the
    /// right ones (or after them, with Ctrl: Append), undoably.
    pub(super) fn revert_change(&mut self, change: usize, append: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hunk) = self.diff.hunks.get(change).cloned() else { return };
        let old = &self.buffers[0];
        let new = &self.buffers[1];
        let lines_text = |b: &Buffer, lines: Range<usize>| -> String {
            if lines.is_empty() {
                return String::new();
            }
            let start = b.line_range(lines.start).start;
            let end = if lines.end < b.line_count() { b.line_range(lines.end).start } else { b.text().len() };
            b.text()[start..end].to_owned()
        };
        let span = |b: &Buffer, lines: Range<usize>| -> Range<usize> {
            let at = |line: usize| if line < b.line_count() { b.line_range(line).start } else { b.text().len() };
            at(lines.start)..at(lines.end)
        };
        let mut text = lines_text(old, hunk.old.clone());
        let mut range = span(new, hunk.new.clone());
        if append {
            range = range.end..range.end;
        }
        // Keep the right file's own ending at its end.
        if range.end == new.text().len() {
            let had_break = if range.is_empty() { new.text().is_empty() || new.text().ends_with('\n') } else { new.text()[range.clone()].ends_with('\n') };
            if had_break && !text.is_empty() && !text.ends_with('\n') {
                text.push_str(new.newline());
            } else if !had_break && text.ends_with('\n') {
                text.truncate(text.trim_end_matches(['\r', '\n']).len());
                if range.is_empty() && !new.text().is_empty() {
                    text.insert_str(0, new.newline());
                }
            }
        }
        let caret = range.start;
        let saved = self.caret;
        self.edit(range, &text, EditKind::Other, Some(caret), cx);
        if saved.is_none_or(|c| c.0 != 1) {
            self.caret = saved;
        }
        self.after_edit(cx);
        let _ = window;
    }

    /// After any edit: re-diff, keep the caret's fragment open, save soon.
    pub(super) fn after_edit(&mut self, cx: &mut Context<Self>) {
        let text = self.buffers[1].text().to_owned();
        if let Some(loaded) = self.loaded.as_mut() {
            loaded.new = text.clone();
        }
        let current = self.current;
        self.diff = diff::compute(&self.buffers[0].text().to_owned(), &text, self.options);
        self.expanded.clear();
        self.current = current.filter(|c| *c < self.diff.changes);
        self.highlight(1, cx);
        self.rebuild_rows();
        self.open_caret_fold();
        self.schedule_save(cx);
        cx.notify();
    }

    /// Expands the collapsed fragment the caret is in.
    fn open_caret_fold(&mut self) {
        let Some((pane, sel)) = self.caret else { return };
        let line = self.buffer(pane).line_of(sel.head);
        if let Some(LineRow::Fold(id, _)) = self.line_rows[pane].get(line) {
            self.expanded.insert(*id);
            self.rebuild_rows();
        }
    }

    /// Writes a pending save now (the view is about to show another file).
    pub(super) fn flush_save(&mut self) {
        if self.save_task.take().is_none() {
            return;
        }
        if let (Some(repository), Some(source)) = (&self.repository, &self.source) {
            if let Err(error) = std::fs::write(repository.root().join(source.path()), self.buffers[1].text()) {
                eprintln!("cannot save {}: {error}", source.path());
            }
        }
    }

    /// Saves the right pane to disk shortly after the last edit.
    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        let (Some(repository), Some(source)) = (self.repository.clone(), self.source.clone()) else { return };
        let path = repository.root().join(source.path());
        let text = self.buffers[1].text().to_owned();
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

    /// Syntax highlighting for one pane's whole text.
    pub(super) fn highlight(&mut self, pane: usize, _cx: &mut Context<Self>) {
        let language = crate::ui::file_editor::language_for(self.source.as_ref().map(|s| s.path()).unwrap_or_default());
        let mut highlighter = gpui_kit::component::highlighter::SyntaxHighlighter::new(language);
        highlighter.update(None, &gpui_kit::component::Rope::from_str(self.buffers[pane].text()), Some(Duration::from_millis(200)));
        self.highlighters[pane] = Some(highlighter);
    }

    /// Scrolls the caret's pane so the caret is in view.
    pub(super) fn reveal_caret(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((pane, sel)) = self.caret else { return };
        self.open_caret_fold();
        let line = self.buffer(pane).line_of(sel.head);
        let Some(LineRow::Row(row)) = self.line_rows[pane].get(line).copied() else { return };
        let (mut x, mut y) = self.pane_scroll[pane];
        let height = self.view_height.get();
        let top = row as f32 * LINE_HEIGHT;
        if top < y {
            y = top;
        } else if height > 0. && top + LINE_HEIGHT > y + height {
            y = top + LINE_HEIGHT - height;
        }
        let width = f32::from(self.pane_bounds.get()[pane].size.width) - self.text_left(pane, cx);
        let caret_x = self.x_for(pane, sel.head, window, cx);
        if caret_x < x {
            x = (caret_x - 40.).max(0.);
        } else if width > 0. && caret_x > x + width - 20. {
            x = caret_x - width + 60.;
        }
        let max_y = (self.pane_rows(pane).len() as f32 * LINE_HEIGHT - height / 2.).max(y);
        self.pane_scroll[pane] = (x, y.min(max_y));
        if self.sync_scroll {
            let (x0, y0) = self.pane_scroll[pane];
            // Re-run the sync from this pane's position.
            self.scroll_pane_to(pane, x0, y0);
            self.pane_scroll[pane] = (x0, y0);
        }
    }
}

pub(super) fn shape(text: &str, window: &Window, cx: &App) -> gpui_kit::ShapedLine {
    let run = TextRun { len: text.len(), font: mono_font(cx), color: gpui_kit::black(), background_color: None, underline: None, strikethrough: None };
    window.text_system().shape_line(SharedString::from(text.to_owned()), px(FONT_SIZE), &[run], None)
}

impl EntityInputHandler for DiffView {
    fn text_for_range(&mut self, range: Range<usize>, adjusted: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        let b = &self.buffers[1];
        let (start, end) = (b.utf16_to_offset(range.start), b.utf16_to_offset(range.end));
        *adjusted = Some(b.offset_to_utf16(start)..b.offset_to_utf16(end));
        Some(b.text()[start..end].to_owned())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        let (pane, sel) = self.caret?;
        let b = &self.buffers[pane];
        let range = sel.range();
        Some(UTF16Selection { range: b.offset_to_utf16(range.start)..b.offset_to_utf16(range.end), reversed: sel.head < sel.anchor })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        let b = &self.buffers[1];
        self.marked.as_ref().map(|r| b.offset_to_utf16(r.start)..b.offset_to_utf16(r.end))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.caret_editable() {
            return;
        }
        let b = &self.buffers[1];
        let range = range
            .map(|r| b.utf16_to_offset(r.start)..b.utf16_to_offset(r.end))
            .or(self.marked.clone())
            .unwrap_or_else(|| self.caret.map(|c| c.1.range()).unwrap_or(0..0));
        self.marked = None;
        let kind = if text.chars().all(|c| !c.is_whitespace()) && range.is_empty() { EditKind::Typing } else { EditKind::Other };
        self.edit(range, text, kind, None, cx);
        self.after_edit(cx);
        self.reveal_caret(window, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.caret_editable() {
            return;
        }
        let b = &self.buffers[1];
        let range = range
            .map(|r| b.utf16_to_offset(r.start)..b.utf16_to_offset(r.end))
            .or(self.marked.clone())
            .unwrap_or_else(|| self.caret.map(|c| c.1.range()).unwrap_or(0..0));
        self.edit(range.clone(), text, EditKind::Typing, None, cx);
        self.marked = (!text.is_empty()).then(|| range.start..range.start + text.len());
        if let Some(selected) = selected {
            // `selected` is in UTF-16 within the new text.
            let to_byte = |u: usize| text.char_indices().scan(0, |n, (i, c)| { let at = *n; *n += c.len_utf16(); Some((at, i)) }).find(|(at, _)| *at >= u).map_or(text.len(), |(_, i)| i);
            let (s, e) = (range.start + to_byte(selected.start), range.start + to_byte(selected.end));
            self.caret = Some((1, Selection { anchor: s, head: e }));
        }
        self.after_edit(cx);
        self.reveal_caret(window, cx);
    }

    fn bounds_for_range(&mut self, range: Range<usize>, _: Bounds<Pixels>, window: &mut Window, cx: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let offset = self.buffers[1].utf16_to_offset(range.start);
        let line = self.buffers[1].line_of(offset);
        let Some(LineRow::Row(row)) = self.line_rows[1].get(line).copied() else { return None };
        let b = self.pane_bounds.get()[1];
        let (sx, sy) = self.pane_scroll[1];
        let x = b.origin.x + px(self.text_left(1, cx) + self.x_for(1, offset, window, cx) - sx);
        let y = b.origin.y + px(row as f32 * LINE_HEIGHT - sy);
        Some(Bounds::new(point(x, y), size(px(2.), px(LINE_HEIGHT))))
    }

    fn character_index_for_point(&mut self, position: gpui_kit::Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) -> Option<usize> {
        match self.hit(position, window, cx)? {
            (1, Ok(offset)) => Some(self.buffers[1].offset_to_utf16(offset)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_and_byte_offsets_round_trip() {
        let text = "\tab\tc";
        assert_eq!(display_offset(text, 1), 4);
        assert_eq!(byte_offset(text, 4), 1);
        assert_eq!(byte_offset(text, 5), 2);
        // Inside a tab's spaces: snaps to before the tab.
        assert_eq!(byte_offset(text, 2), 0);
    }
}
