//! The text behind a diff or merge pane: lines, carets, edits and undo.
//! Offsets are bytes into the whole text; lines exclude their line break.

use std::ops::Range;

#[derive(Clone, Debug, Default)]
pub struct Buffer {
    text: String,
    /// Byte offset where each line starts.
    starts: Vec<usize>,
}

impl Buffer {
    pub fn new(text: String) -> Self {
        let mut buffer = Buffer { text, starts: Vec::new() };
        buffer.reindex();
        buffer
    }

    fn reindex(&mut self) {
        self.starts.clear();
        self.starts.push(0);
        self.starts.extend(self.text.match_indices('\n').map(|(i, _)| i + 1));
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Lines including the empty one after a final line break.
    pub fn line_count(&self) -> usize {
        self.starts.len()
    }

    /// The line's text range, without its "\n" or "\r\n".
    pub fn line_range(&self, line: usize) -> Range<usize> {
        let line = line.min(self.starts.len() - 1);
        let start = self.starts[line];
        let mut end = self.starts.get(line + 1).map_or(self.text.len(), |next| next - 1);
        if end > start && self.text.as_bytes()[end - 1] == b'\r' && end < self.text.len() {
            end -= 1;
        }
        start..end
    }

    pub fn line(&self, line: usize) -> &str {
        &self.text[self.line_range(line)]
    }

    pub fn line_of(&self, offset: usize) -> usize {
        self.starts.partition_point(|s| *s <= offset).saturating_sub(1)
    }

    /// The bytes of whole lines, line breaks included (the last line may
    /// have none).
    pub fn lines_span(&self, lines: Range<usize>) -> Range<usize> {
        let at = |line: usize| if line < self.line_count() { self.starts[line] } else { self.text.len() };
        at(lines.start)..at(lines.end)
    }

    pub fn lines_text(&self, lines: Range<usize>) -> &str {
        &self.text[self.lines_span(lines)]
    }

    /// The line break the file mostly uses.
    pub fn newline(&self) -> &'static str {
        if self.text.contains("\r\n") { "\r\n" } else { "\n" }
    }

    pub fn replace(&mut self, range: Range<usize>, with: &str) {
        self.text.replace_range(range, with);
        self.reindex();
    }

    pub fn set_text(&mut self, text: String) {
        self.text = text;
        self.reindex();
    }

    pub fn prev_char(&self, offset: usize) -> usize {
        let range = self.line_range(self.line_of(offset));
        if offset <= range.start {
            // To the end of the previous line.
            return match self.line_of(offset) {
                0 => 0,
                line => self.line_range(line - 1).end,
            };
        }
        self.text[..offset].char_indices().next_back().map_or(0, |(i, _)| i)
    }

    pub fn next_char(&self, offset: usize) -> usize {
        let line = self.line_of(offset);
        let range = self.line_range(line);
        if offset >= range.end {
            return if line + 1 < self.line_count() { self.starts[line + 1] } else { offset };
        }
        offset + self.text[offset..].chars().next().map_or(0, char::len_utf8)
    }

    /// Ctrl+Left: to the start of the word before, IntelliJ-style (a line
    /// start is a stop of its own).
    pub fn prev_word(&self, offset: usize) -> usize {
        let range = self.line_range(self.line_of(offset));
        if offset <= range.start {
            return self.prev_char(offset);
        }
        let before: Vec<(usize, char)> = self.text[range.start..offset].char_indices().collect();
        let mut i = before.len();
        while i > 0 && before[i - 1].1.is_whitespace() {
            i -= 1;
        }
        if i > 0 {
            let class = char_class(before[i - 1].1);
            while i > 0 && char_class(before[i - 1].1) == class && !before[i - 1].1.is_whitespace() {
                i -= 1;
            }
        }
        range.start + before.get(i).map_or(offset - range.start, |(b, _)| *b)
    }

    /// Ctrl+Right: to the end of the next word.
    pub fn next_word(&self, offset: usize) -> usize {
        let range = self.line_range(self.line_of(offset));
        if offset >= range.end {
            return self.next_char(offset);
        }
        let after: Vec<(usize, char)> = self.text[offset..range.end].char_indices().collect();
        let mut i = 0;
        while i < after.len() && after[i].1.is_whitespace() {
            i += 1;
        }
        if i < after.len() {
            let class = char_class(after[i].1);
            while i < after.len() && char_class(after[i].1) == class && !after[i].1.is_whitespace() {
                i += 1;
            }
        }
        offset + after.get(i).map_or(range.end - offset, |(b, _)| *b)
    }

    /// The word around an offset, for double-click.
    pub fn word_at(&self, offset: usize) -> Range<usize> {
        let range = self.line_range(self.line_of(offset));
        let line = &self.text[range.clone()];
        let at = offset - range.start;
        let Some(ch) = line[at..].chars().next().or_else(|| line[..at].chars().next_back()) else {
            return offset..offset;
        };
        let class = char_class(ch);
        let start = line[..at].char_indices().rev().take_while(|(_, c)| char_class(*c) == class).last().map_or(at, |(i, _)| i);
        let end = line[at..].char_indices().find(|(_, c)| char_class(*c) != class).map_or(line.len(), |(i, _)| at + i);
        range.start + start..range.start + end
    }

    /// The leading whitespace of a line, for auto-indent.
    pub fn indent(&self, line: usize) -> &str {
        let text = self.line(line);
        &text[..text.len() - text.trim_start_matches([' ', '\t']).len()]
    }

    pub fn utf16_to_offset(&self, utf16: usize) -> usize {
        let mut count = 0;
        for (i, ch) in self.text.char_indices() {
            if count >= utf16 {
                return i;
            }
            count += ch.len_utf16();
        }
        self.text.len()
    }

    pub fn offset_to_utf16(&self, offset: usize) -> usize {
        self.text[..offset.min(self.text.len())].chars().map(char::len_utf16).sum()
    }
}

fn char_class(ch: char) -> u8 {
    if ch.is_whitespace() {
        0
    } else if ch.is_alphanumeric() || ch == '_' {
        1
    } else {
        2
    }
}

/// A caret with its selection: `anchor` stays put while `head` moves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub fn caret(offset: usize) -> Self {
        Selection { anchor: offset, head: offset }
    }

    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

/// What an edit was, so that runs of typing undo together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    Typing,
    Deleting,
    Other,
}

#[derive(Clone, Debug)]
struct Snapshot<T> {
    text: String,
    selection: Selection,
    extra: T,
}

/// Undo and redo as whole-text snapshots: a pane holds one file, and a
/// snapshot keeps every edit (typing, revert, paste) on one simple path.
/// `T` is the owner's own state that must undo along with the text (the
/// merge tool's change states).
pub struct History<T = ()> {
    undo: Vec<Snapshot<T>>,
    redo: Vec<Snapshot<T>>,
    last: Option<EditKind>,
}

impl<T> Default for History<T> {
    fn default() -> Self {
        History { undo: Vec::new(), redo: Vec::new(), last: None }
    }
}

impl<T: Clone> History<T> {
    /// Records the state before an edit; typing runs coalesce.
    pub fn record(&mut self, buffer: &Buffer, selection: Selection, extra: &T, kind: EditKind) {
        if kind != EditKind::Other && self.last == Some(kind) {
            return;
        }
        self.undo.push(Snapshot { text: buffer.text().to_owned(), selection, extra: extra.clone() });
        if self.undo.len() > 500 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.last = Some(kind);
    }

    /// Ends a typing run (the caret moved, focus left).
    pub fn break_run(&mut self) {
        self.last = None;
    }

    pub fn undo(&mut self, buffer: &mut Buffer, selection: &mut Selection, extra: &mut T) -> bool {
        let Some(snapshot) = self.undo.pop() else { return false };
        self.redo.push(Snapshot { text: buffer.text().to_owned(), selection: *selection, extra: extra.clone() });
        buffer.set_text(snapshot.text);
        *selection = snapshot.selection;
        *extra = snapshot.extra;
        self.last = None;
        true
    }

    pub fn redo(&mut self, buffer: &mut Buffer, selection: &mut Selection, extra: &mut T) -> bool {
        let Some(snapshot) = self.redo.pop() else { return false };
        self.undo.push(Snapshot { text: buffer.text().to_owned(), selection: *selection, extra: extra.clone() });
        buffer.set_text(snapshot.text);
        *selection = snapshot.selection;
        *extra = snapshot.extra;
        self.last = None;
        true
    }

    pub fn clear(&mut self) {
        *self = History::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_and_breaks() {
        let b = Buffer::new("ab\r\ncd\n".into());
        assert_eq!(b.line_count(), 3);
        assert_eq!(b.line(0), "ab");
        assert_eq!(b.line(1), "cd");
        assert_eq!(b.line(2), "");
        assert_eq!(b.newline(), "\r\n");
        assert_eq!(b.line_of(4), 1);
        assert_eq!(b.next_char(2), 4);
        assert_eq!(b.prev_char(4), 2);
    }

    #[test]
    fn words() {
        let b = Buffer::new("  fun step(x: Int)".into());
        assert_eq!(b.next_word(0), 5);
        assert_eq!(b.next_word(5), 10);
        assert_eq!(b.prev_word(10), 6);
        assert_eq!(b.prev_word(6), 2);
        assert_eq!(b.word_at(7), 6..10);
    }

    #[test]
    fn undo_coalesces_typing() {
        let mut b = Buffer::new("a".into());
        let mut sel = Selection::caret(1);
        let mut h = History::default();
        for ch in ["b", "c"] {
            h.record(&b, sel, &(), EditKind::Typing);
            b.replace(sel.head..sel.head, ch);
            sel = Selection::caret(sel.head + 1);
        }
        assert_eq!(b.text(), "abc");
        assert!(h.undo(&mut b, &mut sel, &mut ()));
        assert_eq!((b.text(), sel.head), ("a", 1));
        assert!(h.redo(&mut b, &mut sel, &mut ()));
        assert_eq!(b.text(), "abc");
    }

    #[test]
    fn utf16_offsets() {
        let b = Buffer::new("a中😀b".into());
        assert_eq!(b.offset_to_utf16(4), 2);
        assert_eq!(b.utf16_to_offset(4), 8);
    }
}
