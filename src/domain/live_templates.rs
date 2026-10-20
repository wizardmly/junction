//! Live Templates, as IntelliJ's: type an abbreviation and press Tab. The
//! built-in Kotlin and Java templates (from IntelliJ's Kotlin.xml and
//! Java.xml: names, texts, variables, contexts, descriptions), their
//! expansion re-indented as IntelliJ's reformat does, and the template
//! session: Tab / Enter from one variable to the next, a variable's copies
//! following as it is typed, Escape or the last Tab ending at `$END$`.
//! Variables IntelliJ computes from the code (`suggestIndexName()`,
//! `iterableComponentType(…)`) start from their usual values.

use std::ops::Range;

/// Where a template applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    /// A statement in a function body.
    Statement,
    /// An expression (in a function body).
    Expression,
    /// A class body: a member declaration.
    Class,
    /// Kotlin: the file's top level.
    TopLevel,
    /// Kotlin: an `object` body.
    Object,
    /// Java: after an `if`'s block (else-if).
    Else,
}

#[derive(Debug)]
pub struct Var {
    pub name: &'static str,
    pub default: &'static str,
    /// Tab stops at it.
    pub stop: bool,
    /// Follows another variable (`escapeString(EXPR)`, `PARAM`).
    pub copy_of: Option<&'static str>,
}

#[derive(Debug)]
pub struct Template {
    pub name: &'static str,
    pub description: &'static str,
    pub value: &'static str,
    pub vars: &'static [Var],
    pub contexts: &'static [Context],
}

const KOTLIN: &[Template] = &[
    Template { name: "main", description: "main() function", value: "fun main($ARGS$) {\n  $END$\n}", vars: &[Var { name: "ARGS", default: "", stop: false, copy_of: None }], contexts: &[Context::TopLevel] },
    Template { name: "maina", description: "main(args) function", value: "fun main(args: Array<String>) {\n  $END$\n}", vars: &[], contexts: &[Context::TopLevel] },
    Template { name: "maino", description: "main(args) function", value: "@JvmStatic\nfun main(args: Array<String>) {\n  $END$\n}", vars: &[], contexts: &[Context::Object] },
    Template { name: "sout", description: "Prints a string to System.out", value: "println($END$)", vars: &[], contexts: &[Context::Statement] },
    Template { name: "serr", description: "Prints a string to System.err", value: "System.err.println($END$)", vars: &[], contexts: &[Context::Statement] },
    Template { name: "soutv", description: "Prints a value to System.out", value: "println(\"$EXPR_COPY$ = $DOLLAR${$EXPR$}\")", vars: &[Var { name: "DOLLAR", default: "$", stop: false, copy_of: None }, Var { name: "EXPR", default: "expr", stop: true, copy_of: None }, Var { name: "EXPR_COPY", default: "", stop: false, copy_of: Some("EXPR") }], contexts: &[Context::Statement] },
    Template { name: "iter", description: "Iterate over elements of iterable (for-in loop)", value: "for ($VAR$ in $ITERABLE$) {\n$END$\n}", vars: &[Var { name: "ITERABLE", default: "iterable", stop: true, copy_of: None }, Var { name: "VAR", default: "i", stop: true, copy_of: None }], contexts: &[Context::Statement] },
    Template { name: "ifn", description: "Inserts 'if null' expression", value: "if ($VAR$ == null) {\n$END$\n}", vars: &[Var { name: "VAR", default: "i", stop: true, copy_of: None }], contexts: &[Context::Expression] },
    Template { name: "inn", description: "Inserts 'if not null' expression", value: "if ($VAR$ != null) {\n$END$\n}", vars: &[Var { name: "VAR", default: "i", stop: true, copy_of: None }], contexts: &[Context::Expression] },
    Template { name: "void", description: "Function returning nothing", value: "fun $NAME$($PARAMS$) {\n$END$\n}", vars: &[Var { name: "NAME", default: "", stop: true, copy_of: None }, Var { name: "PARAMS", default: "", stop: true, copy_of: None }], contexts: &[Context::Class, Context::Statement, Context::TopLevel] },
    Template { name: "fun0", description: "Function with no parameters", value: "fun $NAME$() : $RETURN$ {\n$END$\n}", vars: &[Var { name: "NAME", default: "", stop: true, copy_of: None }, Var { name: "RETURN", default: "Unit", stop: true, copy_of: None }], contexts: &[Context::Class, Context::Statement, Context::TopLevel] },
    Template { name: "fun1", description: "Function with one parameter", value: "fun $NAME$($PARAM1$ : $PARAM1TYPE$) : $RETURN$ {\n$END$\n}", vars: &[Var { name: "NAME", default: "", stop: true, copy_of: None }, Var { name: "PARAM1", default: "x", stop: true, copy_of: None }, Var { name: "PARAM1TYPE", default: "Any", stop: true, copy_of: None }, Var { name: "RETURN", default: "Unit", stop: true, copy_of: None }], contexts: &[Context::Class, Context::Statement, Context::TopLevel] },
    Template { name: "fun2", description: "Function with two parameters", value: "fun $NAME$($PARAM1$ : $PARAM1TYPE$, $PARAM2$ : $PARAM2TYPE$) : $RETURN$ {\n$END$\n}", vars: &[Var { name: "NAME", default: "", stop: true, copy_of: None }, Var { name: "PARAM1", default: "x", stop: true, copy_of: None }, Var { name: "PARAM1TYPE", default: "Any", stop: true, copy_of: None }, Var { name: "PARAM2", default: "y", stop: true, copy_of: None }, Var { name: "PARAM2TYPE", default: "Any", stop: true, copy_of: None }, Var { name: "RETURN", default: "Unit", stop: true, copy_of: None }], contexts: &[Context::Class, Context::Statement, Context::TopLevel] },
    Template { name: "interface", description: "Interface", value: "interface $NAME$ {\n$END$\n}", vars: &[Var { name: "NAME", default: "", stop: true, copy_of: None }], contexts: &[Context::Class, Context::Statement, Context::TopLevel] },
    Template { name: "singleton", description: "Singleton", value: "object $NAME$ {\n$END$\n}", vars: &[Var { name: "NAME", default: "", stop: true, copy_of: None }], contexts: &[Context::Class, Context::Statement, Context::TopLevel] },
    Template { name: "closure", description: "Closure (function without name)", value: "{$PARAM$ -> $PARAM_COPY$}", vars: &[Var { name: "PARAM", default: "x", stop: true, copy_of: None }, Var { name: "PARAM_COPY", default: "", stop: false, copy_of: Some("PARAM") }], contexts: &[Context::Expression, Context::Statement] },
    Template { name: "anonymous", description: "Anonymous class", value: "object : $SUPERTYPE$ {\n$END$\n}", vars: &[Var { name: "SUPERTYPE", default: "Object", stop: true, copy_of: None }], contexts: &[Context::Expression, Context::Statement] },
    Template { name: "exfun", description: "Extension function", value: "fun $RECEIVER$.$NAME$($PARAMS$) : $RETURN$ {\n$END$\n}", vars: &[Var { name: "RECEIVER", default: "Any", stop: true, copy_of: None }, Var { name: "NAME", default: "f", stop: true, copy_of: None }, Var { name: "PARAMS", default: "", stop: true, copy_of: None }, Var { name: "RETURN", default: "Unit", stop: true, copy_of: None }], contexts: &[Context::Class, Context::TopLevel] },
    Template { name: "exval", description: "Extension read-only property", value: "val $RECEIVER$.$NAME$ : $TYPE$\nget() {\n$END$\n}", vars: &[Var { name: "RECEIVER", default: "Any", stop: true, copy_of: None }, Var { name: "NAME", default: "v", stop: true, copy_of: None }, Var { name: "TYPE", default: "Any", stop: true, copy_of: None }], contexts: &[Context::Class, Context::TopLevel] },
    Template { name: "exvar", description: "Extension read-write property", value: "var $RECEIVER$.$NAME$ : $TYPE$\nget() {\n$END$\n}\nset(value) {\n \n}", vars: &[Var { name: "RECEIVER", default: "Any", stop: true, copy_of: None }, Var { name: "NAME", default: "v", stop: true, copy_of: None }, Var { name: "TYPE", default: "Any", stop: true, copy_of: None }], contexts: &[Context::Class, Context::TopLevel] },
    Template { name: "psvm", description: "main() function", value: "fun main($ARGS$) {\n  $END$\n}", vars: &[Var { name: "ARGS", default: "", stop: false, copy_of: None }], contexts: &[Context::TopLevel] },
    Template { name: "psvma", description: "main(args) function", value: "fun main(args: Array<String>) {\n  $END$\n}", vars: &[], contexts: &[Context::TopLevel] },
    Template { name: "psvmo", description: "main(args) function", value: "@JvmStatic\nfun main(args: Array<String>) {\n  $END$\n}", vars: &[], contexts: &[Context::Object] },
    Template { name: "object", description: "Anonymous class", value: "object : $SUPERTYPE$ {\n$END$\n}", vars: &[Var { name: "SUPERTYPE", default: "Object", stop: true, copy_of: None }], contexts: &[Context::Expression, Context::Statement] },
];
const JAVA: &[Template] = &[
    Template { name: "sout", description: "Prints a string to System.out", value: "System.out.println($END$);", vars: &[], contexts: &[Context::Statement] },
    Template { name: "serr", description: "Prints a string to System.err", value: "System.err.println($END$);", vars: &[], contexts: &[Context::Statement] },
    Template { name: "souf", description: "Prints a formatted string to System.out", value: "System.out.printf(\"$END$\");", vars: &[], contexts: &[Context::Statement] },
    Template { name: "soutv", description: "Prints a value to System.out", value: "System.out.println(\"$EXPR_COPY$ = \" + $EXPR$);", vars: &[Var { name: "EXPR", default: "expr", stop: true, copy_of: None }, Var { name: "EXPR_COPY", default: "", stop: false, copy_of: Some("EXPR") }], contexts: &[Context::Statement] },
    Template { name: "psvm", description: "main() method declaration", value: "public static void main(String[] args){\n  $END$\n}", vars: &[], contexts: &[Context::Class] },
    Template { name: "fori", description: "Create iteration loop", value: "for(int $INDEX$ = 0; $INDEX$ < $LIMIT$; $INDEX$++) {\n  $END$\n}", vars: &[Var { name: "INDEX", default: "i", stop: true, copy_of: None }, Var { name: "LIMIT", default: "", stop: true, copy_of: None }], contexts: &[Context::Statement] },
    Template { name: "itar", description: "Iterate elements of array", value: "for(int $INDEX$ = 0; $INDEX$ < $ARRAY$.length; $INDEX$++) {\n  $ELEMENT_TYPE$ $VAR$ = $ARRAY$[$INDEX$];\n  $END$\n}", vars: &[Var { name: "INDEX", default: "i", stop: true, copy_of: None }, Var { name: "ARRAY", default: "array", stop: true, copy_of: None }, Var { name: "ELEMENT_TYPE", default: "Object", stop: true, copy_of: None }, Var { name: "VAR", default: "o", stop: true, copy_of: None }], contexts: &[Context::Statement] },
    Template { name: "iter", description: "Iterate Iterable or array", value: "for ($ELEMENT_TYPE$ $VAR$ : $ITERABLE_TYPE$) {\n  $END$\n}", vars: &[Var { name: "ITERABLE_TYPE", default: "iterable", stop: true, copy_of: None }, Var { name: "ELEMENT_TYPE", default: "Object", stop: true, copy_of: None }, Var { name: "VAR", default: "o", stop: true, copy_of: None }], contexts: &[Context::Statement] },
    Template { name: "ifn", description: "Inserts 'if null' statement", value: "if ($VAR$ == null) {\n$END$\n}", vars: &[Var { name: "VAR", default: "var", stop: true, copy_of: None }], contexts: &[Context::Statement] },
    Template { name: "inn", description: "Inserts 'if not null' statement", value: "if ($VAR$ != null) {\n$END$\n}", vars: &[Var { name: "VAR", default: "var", stop: true, copy_of: None }], contexts: &[Context::Statement] },
    Template { name: "St", description: "String", value: "String ", vars: &[], contexts: &[Context::Class, Context::Statement] },
    Template { name: "psf", description: "public static final", value: "public static final ", vars: &[], contexts: &[Context::Class] },
    Template { name: "prsf", description: "private static final", value: "private static final ", vars: &[], contexts: &[Context::Class] },
    Template { name: "psfi", description: "public static final int", value: "public static final int ", vars: &[], contexts: &[Context::Class] },
    Template { name: "psfs", description: "public static final String", value: "public static final String ", vars: &[], contexts: &[Context::Class] },
    Template { name: "thr", description: "throw new", value: "throw new ", vars: &[], contexts: &[Context::Statement] },
    Template { name: "else-if", description: "Add else-if branch", value: "else if($CONDITION$){\n$END$\n} ", vars: &[Var { name: "CONDITION", default: "", stop: true, copy_of: None }], contexts: &[Context::Else] },
];

/// The templates of a language: `kotlin` or Java.
pub fn templates(kotlin: bool) -> &'static [Template] {
    if kotlin { KOTLIN } else { JAVA }
}

/// The template an abbreviation expands to here.
pub fn find(kotlin: bool, abbreviation: &str, contexts: &[Context]) -> Option<&'static Template> {
    templates(kotlin).iter().find(|t| t.name == abbreviation && t.contexts.iter().any(|c| contexts.contains(c)))
}

/// The templates code completion offers for the word typed before the
/// caret, as IntelliJ's `LiveTemplateCompletionContributor`: those that
/// apply here whose abbreviation starts with it (`so` → `sout`, `soutv`).
/// Nothing for an empty word.
pub fn completing(kotlin: bool, typed: &str, contexts: &[Context]) -> Vec<&'static Template> {
    if typed.is_empty() {
        return Vec::new();
    }
    templates(kotlin).iter().filter(|t| t.name.starts_with(typed) && t.contexts.iter().any(|c| contexts.contains(c))).collect()
}

/// Whether any template's abbreviation starts with the word: when none
/// does, completion needn't work out where the caret is.
pub fn any_starts_with(kotlin: bool, typed: &str) -> bool {
    !typed.is_empty() && templates(kotlin).iter().any(|t| t.name.starts_with(typed))
}

/// The completion command that expands the template just completed.
pub const EXPAND_COMMAND: &str = "junction.expandLiveTemplate";

/// A template written out: its text, each tab stop's ranges (a variable
/// with its copies), and where `$END$` is.
#[derive(Debug, PartialEq, Eq)]
pub struct Expansion {
    pub text: String,
    pub stops: Vec<Vec<Range<usize>>>,
    pub end: usize,
}

/// Writes a template out at a line indented `indent`: each following line
/// indented by its brace depth in steps of `unit`.
pub fn expand(template: &Template, indent: &str, unit: &str) -> Expansion {
    let value_of = |name: &str| -> &str {
        let var = template.vars.iter().find(|v| v.name == name);
        let source = var.and_then(|v| v.copy_of).and_then(|c| template.vars.iter().find(|v| v.name == c));
        source.or(var).map_or("", |v| v.default)
    };
    let mut out = String::new();
    let mut ranges: Vec<(&str, Range<usize>)> = Vec::new();
    let mut end = None;
    let mut depth: usize = 0;
    for (n, line) in template.value.split('\n').enumerate() {
        let line = if n == 0 { line } else { line.trim_start() };
        if n > 0 {
            out.push('\n');
            let closing = line.starts_with('}');
            let level = if closing { depth.saturating_sub(1) } else { depth };
            if !line.trim().is_empty() || line.contains("$END$") {
                out.push_str(indent);
                out.push_str(&unit.repeat(level));
            }
        }
        let mut rest = line;
        while let Some(at) = rest.find('$') {
            let after = &rest[at + 1..];
            let Some(close) = after.find('$') else { break };
            let name = &after[..close];
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
                out.push_str(&rest[..at + 1]);
                rest = after;
                continue;
            }
            out.push_str(&rest[..at]);
            match name {
                "END" => end = Some(out.len()),
                "SELECTION" => {}
                _ => {
                    let start = out.len();
                    out.push_str(value_of(name));
                    ranges.push((name, start..out.len()));
                }
            }
            rest = &after[close + 1..];
        }
        out.push_str(rest);
        depth = (depth + line.matches('{').count()).saturating_sub(line.matches('}').count());
    }
    let stops = template
        .vars
        .iter()
        .filter(|v| v.stop)
        .map(|v| {
            let copies: Vec<&str> = template.vars.iter().filter(|c| c.copy_of == Some(v.name)).map(|c| c.name).collect();
            ranges.iter().filter(|(n, _)| *n == v.name || copies.contains(n)).map(|(_, r)| r.clone()).collect()
        })
        .filter(|r: &Vec<Range<usize>>| !r.is_empty())
        .collect();
    let end = end.unwrap_or(out.len());
    Expansion { text: out, stops, end }
}

/// A template being filled in, in document offsets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub stops: Vec<Vec<Range<usize>>>,
    pub current: usize,
    pub end: usize,
}

/// The copies of the current variable to update after it was typed in:
/// the range of the text to replace, its new text, and where the caret
/// goes after.
#[derive(Debug, PartialEq, Eq)]
pub struct Mirror {
    pub range: Range<usize>,
    pub text: String,
    pub caret: usize,
}

fn shift(at: usize, from: usize, delta: isize) -> usize {
    if at >= from { (at as isize + delta).max(0) as usize } else { at }
}

impl Session {
    /// The session of an expansion inserted at `at`.
    pub fn new(expansion: &Expansion, at: usize) -> Self {
        let stops = expansion.stops.iter().map(|g| g.iter().map(|r| r.start + at..r.end + at).collect()).collect();
        Session { stops, current: 0, end: expansion.end + at }
    }

    /// The current variable's first range (to select), if any.
    pub fn selection(&self) -> Option<Range<usize>> {
        self.stops.get(self.current).and_then(|g| g.first().cloned())
    }

    /// To the next variable; `None` past the last (the session ends at
    /// `end`).
    pub fn next(&mut self) -> Option<Range<usize>> {
        self.current += 1;
        self.selection()
    }

    pub fn previous(&mut self) -> Option<Range<usize>> {
        self.current = self.current.saturating_sub(1);
        self.selection()
    }

    /// The text changed from `old` to `new` (the caret now at `caret`).
    /// `None`: the edit was outside the current variable (the session
    /// ends). Otherwise the copies to update, if they differ now.
    pub fn edited(&mut self, old: &str, new: &str, caret: usize) -> Option<Option<Mirror>> {
        if old == new {
            return Some(None);
        }
        let caret = caret.min(new.len());
        let mut prefix = old.bytes().zip(new.bytes()).take_while(|(a, b)| a == b).count().min(caret);
        while !old.is_char_boundary(prefix) || !new.is_char_boundary(prefix) {
            prefix -= 1;
        }
        let max_suffix = (old.len() - prefix).min(new.len() - prefix).min(new.len() - caret);
        let mut suffix = old.bytes().rev().zip(new.bytes().rev()).take_while(|(a, b)| a == b).count().min(max_suffix);
        while !old.is_char_boundary(old.len() - suffix) || !new.is_char_boundary(new.len() - suffix) {
            suffix -= 1;
        }
        let edit = prefix..old.len() - suffix;
        let delta = new.len() as isize - old.len() as isize;
        let group = self.stops.get(self.current)?;
        let primary = group.iter().position(|r| r.start <= edit.start && edit.end <= r.end)?;
        // Everything after the edit moves; the variable edited grows.
        for (g, ranges) in self.stops.iter_mut().enumerate() {
            for (i, r) in ranges.iter_mut().enumerate() {
                if g == self.current && i == primary {
                    r.end = (r.end as isize + delta) as usize;
                } else {
                    *r = shift(r.start, edit.end, delta)..shift(r.end, edit.end, delta);
                }
            }
        }
        self.end = shift(self.end, edit.end, delta);
        let value = new[self.stops[self.current][primary].clone()].to_owned();
        let mirrors: Vec<usize> = (0..self.stops[self.current].len()).filter(|&i| i != primary && new[self.stops[self.current][i].clone()] != value).collect();
        if mirrors.is_empty() {
            return Some(None);
        }
        // One replacement over the copies (and whatever lies between); a
        // position then moves by the growth of every copy ending before it.
        let current = self.current;
        let mut copies: Vec<(usize, Range<usize>)> = mirrors.iter().map(|&i| (i, self.stops[current][i].clone())).collect();
        copies.sort_by_key(|(_, r)| r.start);
        let growth = |r: &Range<usize>| value.len() as isize - r.len() as isize;
        let moved = |pos: usize| -> isize { copies.iter().filter(|(_, r)| r.end <= pos).map(|(_, r)| growth(r)).sum() };
        let span_start = copies.first().map_or(0, |(_, r)| r.start);
        let span_end = copies.iter().map(|(_, r)| r.end).max().unwrap_or(span_start);
        let mut text = String::new();
        let mut last = span_start;
        for (_, r) in &copies {
            text.push_str(&new[last..r.start]);
            text.push_str(&value);
            last = r.end;
        }
        text.push_str(&new[last..span_end]);
        let at = |pos: usize| (pos as isize + moved(pos)) as usize;
        for (g, ranges) in self.stops.iter_mut().enumerate() {
            for (i, x) in ranges.iter_mut().enumerate() {
                if g == current && mirrors.contains(&i) {
                    let start = (x.start as isize + moved(x.start)) as usize;
                    *x = start..start + value.len();
                } else {
                    *x = at(x.start)..at(x.end);
                }
            }
        }
        self.end = at(self.end);
        let caret_after = at(caret) as isize;
        Some(Some(Mirror { range: span_start..span_end, text, caret: caret_after.max(0) as usize }))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn offers_templates_starting_with_the_word() {
        let names = |typed, contexts: &[Context]| super::completing(true, typed, contexts).iter().map(|t| t.name).collect::<Vec<_>>();
        assert_eq!(names("so", &[Context::Statement]), ["sout", "soutv"]);
        assert_eq!(names("sout", &[Context::Class]), Vec::<&str>::new());
        assert_eq!(names("", &[Context::Statement]), Vec::<&str>::new());
        assert!(super::any_starts_with(false, "fo"));
        assert!(!super::any_starts_with(false, "xyz"));
    }

    use super::*;

    fn template(kotlin: bool, name: &str) -> &'static Template {
        templates(kotlin).iter().find(|t| t.name == name).unwrap()
    }

    #[test]
    fn expands_and_indents() {
        let e = expand(template(true, "iter"), "    ", "    ");
        assert_eq!(e.text, "for (i in iterable) {\n        \n    }");
        // ITERABLE first (the variables' order), then VAR.
        assert_eq!(&e.text[e.stops[0][0].clone()], "iterable");
        assert_eq!(&e.text[e.stops[1][0].clone()], "i");
        assert_eq!(e.end, "for (i in iterable) {\n        ".len());
        let e = expand(template(false, "fori"), "  ", "  ");
        assert_eq!(e.text, "for(int i = 0; i < ; i++) {\n    \n  }");
        assert_eq!(e.stops[0].len(), 3);
        let e = expand(template(true, "soutv"), "", "    ");
        assert_eq!(e.text, "println(\"expr = ${expr}\")");
        // EXPR with its escaped copy.
        assert_eq!(e.stops.len(), 1);
        assert_eq!(e.stops[0].len(), 2);
        assert_eq!(find(true, "sout", &[Context::Statement, Context::Expression]).map(|t| t.value), Some("println($END$)"));
        assert!(find(true, "sout", &[Context::TopLevel]).is_none());
    }

    #[test]
    fn copies_follow_the_variable() {
        let line = "    ";
        let e = expand(template(false, "fori"), line, "    ");
        let at = line.len();
        let mut text = format!("{line}{}", e.text);
        let mut session = Session::new(&e, at);
        let first = session.selection().unwrap();
        assert_eq!(&text[first.clone()], "i");
        // Type `j` over the selected `i`.
        let old = text.clone();
        text.replace_range(first.clone(), "j");
        let mirror = session.edited(&old, &text, first.start + 1).unwrap().unwrap();
        text.replace_range(mirror.range.clone(), &mirror.text);
        assert_eq!(text, "    for(int j = 0; j < ; j++) {\n        \n    }");
        assert_eq!(mirror.caret, first.start + 1);
        // On to LIMIT, then typing `10` there.
        let limit = session.next().unwrap();
        assert_eq!(limit, text.find("< ;").unwrap() + 2..text.find("< ;").unwrap() + 2);
        let old = text.clone();
        text.insert_str(limit.start, "10");
        assert_eq!(session.edited(&old, &text, limit.start + 2), Some(None));
        assert_eq!(session.next(), None);
        assert_eq!(&text[session.end - 8..session.end], "        ");
        // An edit outside the variable ends the session.
        let mut session = Session::new(&e, at);
        let old = format!("{line}{}", e.text);
        let mut moved = old.clone();
        moved.insert(0, 'x');
        assert_eq!(session.edited(&old, &moved, 1), None);
    }

    #[test]
    fn longer_copies_move_what_follows() {
        let e = expand(template(true, "soutv"), "", "    ");
        let mut text = e.text.clone();
        let mut session = Session::new(&e, 0);
        let r = session.selection().unwrap();
        let old = text.clone();
        text.replace_range(r.clone(), "count");
        let m = session.edited(&old, &text, r.start + 5).unwrap().unwrap();
        text.replace_range(m.range.clone(), &m.text);
        assert_eq!(text, "println(\"count = ${count}\")");
        assert_eq!(&text[m.caret - 5..m.caret], "count");
        assert_eq!(session.end, text.len());
    }
}
