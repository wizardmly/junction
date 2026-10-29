//! Extend Selection (Ctrl+W), as IntelliJ's: the word at the caret, then
//! each enclosing syntax node, a block's contents before the block itself;
//! Shrink Selection goes back through the same steps.

use std::ops::Range;

use super::lang::Lang;
use super::symbols;
use crate::domain::line_edit::{line_end, line_start};

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

/// Move Statement Up / Down (Ctrl+Shift+↑/↓), as IntelliJ's: the statement
/// or declaration starting on the caret's line (all of its lines) swaps
/// with the one before or after it in the same block, the lines between
/// them staying; a statement in a body moves into or out of the blocks
/// around it (`into_or_out_of_block`), elsewhere nothing moves at the
/// block's edge. Without syntax, the lines move.
pub fn move_statement(lang: Option<Lang>, text: &str, sel: Range<usize>, up: bool) -> Option<crate::domain::line_edit::Edited> {
    let Some(tree) = lang.and_then(|lang| symbols::parse(lang, text)) else { return crate::domain::line_edit::move_lines(text, sel, up) };
    let first = line_start(text, sel.start);
    let code = first + text[first..].len() - text[first..].trim_start_matches([' ', '\t']).len();
    if code >= text.len() || text[code..].starts_with('\n') {
        return crate::domain::line_edit::move_lines(text, sel, up);
    }
    // The outermost node starting on this line, below the file itself.
    let mut node = tree.root_node().descendant_for_byte_range(code, code)?;
    while let Some(parent) = node.parent() {
        if parent.parent().is_none() || parent.start_byte() < first || is_statement_list(parent) {
            break;
        }
        node = parent;
    }
    if node.start_byte() < first || node.parent().is_none() {
        return None;
    }
    let lines_of = |n: tree_sitter::Node| line_start(text, n.start_byte())..line_end(text, n.end_byte().max(n.start_byte() + 1) - 1);
    let mine = lines_of(node);
    let mine = mine.start..mine.end.max(line_end(text, sel.end.max(sel.start)));
    let mut other = if up { node.prev_named_sibling() } else { node.next_named_sibling() };
    // A sibling on the same line (`a; b`) isn't a line of its own.
    while let Some(o) = other.filter(|o| if up { o.end_byte() > mine.start } else { o.start_byte() < mine.end }) {
        other = if up { o.prev_named_sibling() } else { o.next_named_sibling() };
    }
    // A statement goes into the block next to it, or out of its own block.
    if node.parent().is_some_and(is_statement_list) {
        if let Some(moved) = into_or_out_of_block(text, node, other, mine.clone(), sel.clone(), up) {
            return Some(moved);
        }
    }
    let other = other?;
    let theirs = lines_of(other);
    let (a, b) = if up { (theirs.clone(), mine.clone()) } else { (mine.clone(), theirs.clone()) };
    let gap = &text[a.end..b.start];
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..a.start]);
    out.push_str(&text[b.clone()]);
    out.push_str(gap);
    out.push_str(&text[a.clone()]);
    out.push_str(&text[b.end..]);
    // The selection moves with its statement.
    let shift = |at: usize| if up { at - mine.start + a.start } else { at - mine.start + a.start + b.len() + gap.len() };
    Some(crate::domain::line_edit::Edited { text: out, selection: shift(sel.start)..shift(sel.end) })
}

/// A list of statements: a Kotlin body's `statements`, a Java `{ … }`.
fn is_statement_list(n: tree_sitter::Node) -> bool {
    matches!(n.kind(), "statements" | "block")
}

/// A statement with blocks a statement can move into and out of.
fn is_compound(n: tree_sitter::Node) -> bool {
    matches!(
        n.kind(),
        "if_expression" | "for_statement" | "while_statement" | "do_while_statement" | "try_expression" | "call_expression"
            | "if_statement" | "enhanced_for_statement" | "do_statement" | "try_statement" | "try_with_resources_statement"
            | "synchronized_statement" | "block"
    )
}

/// IntelliJ's Move Statement across a block's edge: up into the block that
/// ends just above (before its `}`), down into the one that starts just
/// below (after its `{`); at its own block's edge, out of it, before or
/// after the statement the block belongs to, or into the neighbouring
/// `} else {` / `} catch (…) {` block. The lines are reindented.
fn into_or_out_of_block(text: &str, node: tree_sitter::Node, other: Option<tree_sitter::Node>, mine: Range<usize>, sel: Range<usize>, up: bool) -> Option<crate::domain::line_edit::Edited> {
    let line = |at: usize| line_start(text, at)..line_end(text, at);
    let indent = |at: usize| {
        let l = &text[line(at)];
        &l[..l.len() - l.trim_start_matches([' ', '\t']).len()]
    };
    let unit = if indent(mine.start).starts_with('\t') { "\t" } else { "    " };
    let blank = |r: &Range<usize>| text[r.clone()].trim().is_empty();
    // The statement that a statement list belongs to.
    let outer = || {
        let mut n = node.parent()?;
        while !n.parent().is_some_and(is_statement_list) {
            n = n.parent()?;
        }
        Some(n).filter(|n| is_compound(*n))
    };
    let (at, new_indent) = match other {
        Some(s) if up => {
            let last = line(s.end_byte() - 1);
            if !is_compound(s) || !text[last.clone()].trim_start().starts_with('}') || last.start <= s.start_byte() {
                return None;
            }
            (last.start, format!("{}{unit}", indent(last.start)))
        }
        Some(s) => {
            if !is_compound(s) {
                return None;
            }
            let mut l = line(s.start_byte());
            while !text[l.clone()].trim_end().ends_with('{') {
                if l.end >= s.end_byte() {
                    return None;
                }
                l = line(l.end + 1);
            }
            (l.end + 1, format!("{}{unit}", indent(s.start_byte())))
        }
        None if up => {
            let mut l = line(mine.start);
            loop {
                l = line(l.start.checked_sub(1)?);
                if !blank(&l) {
                    break;
                }
            }
            let brace = text[l.clone()].trim();
            if !brace.ends_with('{') {
                return None;
            }
            if brace.starts_with('}') {
                (l.start, format!("{}{unit}", indent(l.start)))
            } else {
                let o = outer()?;
                (line_start(text, o.start_byte()), indent(o.start_byte()).to_owned())
            }
        }
        None => {
            let mut l = line(mine.end);
            loop {
                if l.end >= text.len() {
                    return None;
                }
                l = line(l.end + 1);
                if !blank(&l) {
                    break;
                }
            }
            let brace = text[l.clone()].trim();
            if !brace.starts_with('}') {
                return None;
            }
            if brace.ends_with('{') {
                (l.end + 1, format!("{}{unit}", indent(l.start)))
            } else {
                let o = outer()?;
                let end = line_end(text, o.end_byte() - 1);
                if end >= text.len() {
                    return None;
                }
                (end + 1, indent(o.start_byte()).to_owned())
            }
        }
    };
    if mine.end >= text.len() || (mine.start..=mine.end).contains(&at) && at != mine.start {
        return None;
    }
    // The lines, reindented, and where the selection lands in them.
    let old_indent = indent(mine.start);
    let mut moved = String::new();
    let mut starts = Vec::new();
    for l in text[mine.clone()].split('\n') {
        starts.push(moved.len());
        if let Some(rest) = l.strip_prefix(old_indent).filter(|r| !r.trim().is_empty()) {
            moved.push_str(&new_indent);
            moved.push_str(rest);
        } else if !l.trim().is_empty() {
            moved.push_str(l);
        }
        moved.push('\n');
    }
    let shift = |o: usize| {
        let rel = o.clamp(mine.start, mine.end) - mine.start;
        let (i, line_rel) = text[mine.start..mine.start + rel].split('\n').enumerate().last().map(|(i, l)| (i, l.len())).unwrap_or((0, 0));
        let col = if line_rel >= old_indent.len() { line_rel - old_indent.len() + new_indent.len() } else { line_rel.min(new_indent.len()) };
        starts[i] + col
    };
    let removed = mine.start..mine.end + 1;
    let mut out = String::with_capacity(text.len() + moved.len());
    let base = if at <= mine.start {
        out.push_str(&text[..at]);
        let base = out.len();
        out.push_str(&moved);
        out.push_str(&text[at..removed.start]);
        out.push_str(&text[removed.end..]);
        base
    } else {
        out.push_str(&text[..removed.start]);
        out.push_str(&text[removed.end..at]);
        let base = out.len();
        out.push_str(&moved);
        out.push_str(&text[at..]);
        base
    };
    Some(crate::domain::line_edit::Edited { text: out, selection: base + shift(sel.start)..base + shift(sel.end.max(sel.start)) })
}

/// Move Element Left / Right (Ctrl+Alt+Shift+←/→), as IntelliJ's: the
/// argument, parameter or list item at the caret swaps with the one before
/// or after it, in the nearest comma-separated list around the caret.
pub fn move_element(lang: Option<Lang>, text: &str, sel: Range<usize>, left: bool) -> Option<crate::domain::line_edit::Edited> {
    let tree = symbols::parse(lang?, text)?;
    let mut node = tree.root_node().descendant_for_byte_range(sel.start, sel.end);
    while let Some(list) = node {
        node = list.parent();
        let mut cursor = list.walk();
        let children: Vec<tree_sitter::Node> = list.children(&mut cursor).collect();
        if !children.iter().any(|c| c.kind() == ",") {
            continue;
        }
        // The elements: the runs between commas, without the brackets.
        let mut elements: Vec<Range<usize>> = Vec::new();
        let mut run: Option<Range<usize>> = None;
        for (i, c) in children.iter().enumerate() {
            let bracket = !c.is_named() && (i == 0 || i == children.len() - 1) && matches!(c.kind(), "(" | ")" | "[" | "]" | "{" | "}" | "<" | ">");
            if c.kind() == "," || bracket {
                elements.extend(run.take());
                continue;
            }
            run = Some(match run {
                Some(r) => r.start..c.end_byte(),
                None => c.start_byte()..c.end_byte(),
            });
        }
        elements.extend(run);
        let Some(at) = elements.iter().position(|r| r.start <= sel.start && sel.end <= r.end) else { continue };
        let other = if left { at.checked_sub(1)? } else { at + 1 };
        let (mine, theirs) = (elements[at].clone(), elements.get(other)?.clone());
        let (a, b) = if left { (theirs, mine.clone()) } else { (mine.clone(), theirs) };
        let mut out = String::with_capacity(text.len());
        out.push_str(&text[..a.start]);
        out.push_str(&text[b.clone()]);
        out.push_str(&text[a.end..b.start]);
        out.push_str(&text[a.clone()]);
        out.push_str(&text[b.end..]);
        let new_start = if left { a.start } else { b.end - mine.len() };
        let shift = |at: usize| at - mine.start + new_start;
        return Some(crate::domain::line_edit::Edited { text: out, selection: shift(sel.start)..shift(sel.end) });
    }
    None
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

    #[test]
    fn moves_whole_statements() {
        let text = "class A {\n    fun f() {\n        a()\n    }\n\n    fun g() = 1\n}\n";
        let at = text.find("fun g").unwrap();
        let up = move_statement(Some(Lang::Kotlin), text, at..at, true).unwrap();
        assert_eq!(up.text, "class A {\n    fun g() = 1\n\n    fun f() {\n        a()\n    }\n}\n");
        assert_eq!(&up.text[up.selection.start..up.selection.start + 5], "fun g");
        // And back down past the whole function.
        let down = move_statement(Some(Lang::Kotlin), &up.text, up.selection.clone(), false).unwrap();
        assert_eq!(down.text, text);
        assert_eq!(down.selection, at..at);
        // At the block's edge: nothing.
        let f = text.find("fun f").unwrap();
        assert!(move_statement(Some(Lang::Kotlin), text, f..f, true).is_none());
        // Statements in a body.
        let body = "void m() {\n    int x = 1;\n    foo(x,\n        2);\n    bar();\n}\n";
        let at = body.find("int x").unwrap();
        let down = move_statement(Some(Lang::Java), body, at..at, false).unwrap();
        assert_eq!(down.text, "void m() {\n    foo(x,\n        2);\n    int x = 1;\n    bar();\n}\n");
    }

    #[test]
    fn moves_statements_into_and_out_of_blocks() {
        let mv = |lang, text: &str, at: &str, up| {
            let at = text.find(at).unwrap();
            let moved = move_statement(Some(lang), text, at..at, up).unwrap();
            (moved.text.clone(), moved.text[moved.selection.start..].lines().next().unwrap().to_owned())
        };
        let kt = "fun f() {\n    a()\n    if (x) {\n        b()\n    } else {\n        c()\n    }\n    d()\n}\n";
        // Down into the `if`, after its `{`.
        let (down, caret) = mv(Lang::Kotlin, kt, "a()", false);
        assert_eq!(down, "fun f() {\n    if (x) {\n        a()\n        b()\n    } else {\n        c()\n    }\n    d()\n}\n");
        assert_eq!(caret, "a()");
        // Up out of it again.
        assert_eq!(mv(Lang::Kotlin, &down, "a()", true).0, kt);
        // Up into the `else` block, before its `}`.
        assert_eq!(mv(Lang::Kotlin, kt, "d()", true).0, "fun f() {\n    a()\n    if (x) {\n        b()\n    } else {\n        c()\n        d()\n    }\n}\n");
        // From the `if` block down into the `else` block, and back up.
        let (else_first, _) = mv(Lang::Kotlin, kt, "b()", false);
        assert_eq!(else_first, "fun f() {\n    a()\n    if (x) {\n    } else {\n        b()\n        c()\n    }\n    d()\n}\n");
        assert_eq!(mv(Lang::Kotlin, &else_first, "b()", true).0, kt);
        // Down out of the `else` block.
        assert_eq!(mv(Lang::Kotlin, kt, "c()", false).0, "fun f() {\n    a()\n    if (x) {\n        b()\n    } else {\n    }\n    c()\n    d()\n}\n");
        // A function body's edge: nothing.
        let first = kt.find("a()").unwrap();
        assert!(move_statement(Some(Lang::Kotlin), kt, first..first, true).is_none());
        // Java: a multi-line statement into a loop, reindented.
        let java = "class A {\n    void m() {\n        foo(1,\n            2);\n        for (int i : xs) {\n            e();\n        }\n    }\n}\n";
        let (down, _) = mv(Lang::Java, java, "foo", false);
        assert_eq!(down, "class A {\n    void m() {\n        for (int i : xs) {\n            foo(1,\n                2);\n            e();\n        }\n    }\n}\n");
        assert_eq!(mv(Lang::Java, &down, "foo", true).0, java);
        // A plain statement still swaps.
        let plain = "fun f() {\n    a()\n    b()\n}\n";
        assert_eq!(mv(Lang::Kotlin, plain, "a()", false).0, "fun f() {\n    b()\n    a()\n}\n");
    }

    #[test]
    fn moves_list_elements() {
        let text = "fun f(a: Int, b: String = \"x\") = g(one, h(2, 3), last)\n";
        let at = text.find("h(").unwrap();
        let right = move_element(Some(Lang::Kotlin), text, at..at, false).unwrap();
        assert_eq!(right.text, "fun f(a: Int, b: String = \"x\") = g(one, last, h(2, 3))\n");
        assert_eq!(&right.text[right.selection.start..right.selection.start + 2], "h(");
        let inner = text.find("3)").unwrap();
        assert_eq!(move_element(Some(Lang::Kotlin), text, inner..inner, true).unwrap().text, "fun f(a: Int, b: String = \"x\") = g(one, h(3, 2), last)\n");
        let param = text.find("b: S").unwrap();
        assert_eq!(move_element(Some(Lang::Kotlin), text, param..param, true).unwrap().text, "fun f(b: String = \"x\", a: Int) = g(one, h(2, 3), last)\n");
        // At the end: nothing.
        let last = text.find("last").unwrap();
        assert!(move_element(Some(Lang::Kotlin), text, last..last, false).is_none());
        let java = "class A { void m(int x, long y) { m(1, 2L); } }";
        let at = java.find("int x").unwrap();
        assert_eq!(move_element(Some(Lang::Java), java, at..at, false).unwrap().text, "class A { void m(long y, int x) { m(1, 2L); } }");
    }
}
