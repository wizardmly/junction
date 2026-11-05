//! IntelliJ's Add Selection for Next Occurrence (Alt+J), Unselect
//! Occurrence (Shift+Alt+J) and Select All Occurrences
//! (Ctrl+Alt+Shift+J): which text the next caret selects. The first press
//! on a bare caret selects the word there. A selection that is a whole
//! word then matches whole words only (IntelliJ does so after selecting
//! the word from a caret), any other selection matches anywhere. Matching
//! is case-sensitive.

use std::ops::Range;

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// The word at (or just before) `offset`.
pub fn word_at(text: &str, offset: usize) -> Option<Range<usize>> {
    let offset = offset.min(text.len());
    if !text.is_char_boundary(offset) {
        return None;
    }
    let before = text[..offset].char_indices().rev().take_while(|&(_, c)| is_word(c)).last().map_or(offset, |(i, _)| i);
    let after = text[offset..].char_indices().find(|&(_, c)| !is_word(c)).map_or(text.len(), |(i, _)| offset + i);
    (before < after).then_some(before..after)
}

/// Whether `range` holds one whole word (later presses match whole words).
pub fn is_word_selection(text: &str, range: &Range<usize>) -> bool {
    !range.is_empty() && text[range.clone()].chars().all(is_word) && whole_word(text, range)
}

fn whole_word(text: &str, range: &Range<usize>) -> bool {
    !text[..range.start].chars().next_back().is_some_and(is_word) && !text[range.end..].chars().next().is_some_and(is_word)
}

/// Every match of `needle`, left to right, without overlaps.
pub fn all(text: &str, needle: &str, whole: bool) -> Vec<Range<usize>> {
    if needle.is_empty() {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(i) = text[from..].find(needle) {
        let range = from + i..from + i + needle.len();
        from = range.end;
        if !whole || whole_word(text, &range) {
            found.push(range);
        }
    }
    found
}

/// The next match after `after`, wrapping round to the top, that isn't one
/// of `taken`.
pub fn next(text: &str, needle: &str, whole: bool, after: usize, taken: &[Range<usize>]) -> Option<Range<usize>> {
    let matches = all(text, needle, whole);
    let free = |r: &&Range<usize>| !taken.iter().any(|t| t.start < r.end && r.start < t.end);
    matches.iter().filter(|r| r.start >= after).find(free).or_else(|| matches.iter().find(free)).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_at_caret() {
        let text = "int sum = a + sum2;";
        assert_eq!(word_at(text, 5), Some(4..7));
        assert_eq!(word_at(text, 7), Some(4..7));
        assert_eq!(word_at(text, 4), Some(4..7));
        assert_eq!(word_at(text, 8), None);
    }

    #[test]
    fn whole_words_only_after_a_caret() {
        let text = "sum + sum2 + asum + sum";
        assert_eq!(all(text, "sum", true), vec![0..3, 20..23]);
        assert_eq!(all(text, "sum", false).len(), 4);
    }

    #[test]
    fn word_selections() {
        let text = "a sum2 b";
        assert!(is_word_selection(text, &(2..6)));
        assert!(!is_word_selection(text, &(2..5)));
        assert!(!is_word_selection(text, &(1..6)));
    }

    #[test]
    fn next_skips_taken_and_wraps() {
        let text = "a b a c a";
        assert_eq!(next(text, "a", true, 1, &[0..1]), Some(4..5));
        assert_eq!(next(text, "a", true, 9, &[4..5, 8..9]), Some(0..1));
        assert_eq!(next(text, "a", true, 9, &[0..1, 4..5, 8..9]), None);
    }
}
