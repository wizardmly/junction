//! Extend Selection (Ctrl+W), as IntelliJ's: the word at the caret, then
//! each enclosing syntax node, a block's contents before the block itself;
//! Shrink Selection goes back through the same steps.

use std::ops::Range;

use super::lang::Lang;
use super::symbols;

/// The next larger selection around `sel`; none at the whole text.
pub fn extend(lang: Option<Lang>, text: &str, sel: Range<usize>) -> Option<Range<usize>> {
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    if sel.is_empty() {
        let start = text[..sel.start].char_indices().rev().take_while(|(_, c)| is_word(*c)).last().map_or(sel.start, |(i, _)| i);
        let end = text[sel.start..].char_indices().find(|(_, c)| !is_word(*c)).map_or(text.len(), |(i, _)| sel.start + i);
        if start < end {
            return Some(start..end);
        }
    }
    let grows = |r: &Range<usize>| r.start <= sel.start && sel.end <= r.end && r.len() > sel.len();
    if let Some(tree) = lang.and_then(|lang| symbols::parse(lang, text)) {
        let mut node = tree.root_node().descendant_for_byte_range(sel.start, sel.end);
        while let Some(n) = node {
            let r = n.byte_range();
            // A block's contents (inside the braces, trimmed) come first.
            let body = &text[r.clone()];
            if r.len() >= 2 && matches!((body.as_bytes()[0], body.as_bytes()[body.len() - 1]), (b'{', b'}') | (b'(', b')') | (b'[', b']')) {
                let inner = &body[1..body.len() - 1];
                let lead = inner.len() - inner.trim_start().len();
                let trimmed = r.start + 1 + lead..r.start + 1 + inner.trim_end().len();
                if trimmed.start < trimmed.end && grows(&trimmed) {
                    return Some(trimmed);
                }
            }
            if grows(&r) {
                return Some(r);
            }
            node = n.parent();
        }
    }
    // No syntax: the line, then everything.
    let line = text[..sel.start].rfind('\n').map_or(0, |i| i + 1)..text[sel.end..].find('\n').map_or(text.len(), |i| sel.end + i);
    [line, 0..text.len()].into_iter().find(grows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grows_through_the_syntax() {
        let text = "fun f() {\n    val x = foo(a + b)\n}\n";
        let at = text.find("a +").unwrap();
        let mut sel = at..at;
        let mut steps = Vec::new();
        while let Some(next) = extend(Some(Lang::Kotlin), text, sel.clone()) {
            steps.push(text[next.clone()].to_owned());
            sel = next;
            if steps.len() > 12 {
                break;
            }
        }
        assert_eq!(&steps[..3], ["a", "a + b", "(a + b)"]);
        assert!(steps.contains(&"foo(a + b)".to_owned()), "{steps:?}");
        assert!(steps.contains(&"val x = foo(a + b)".to_owned()), "{steps:?}");
        assert_eq!(steps.last().unwrap(), text);
        assert_eq!(extend(None, "one\ntwo", 5..6), Some(4..7));
    }
}
