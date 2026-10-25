//! Code completion: what a candidate is, how a typed prefix matches it and
//! how candidates rank, after IntelliJ's `CamelHumpMatcher` and its
//! completion sorting (prefix matches first, then nearer scopes).

use std::ops::Range;

/// What a candidate names; the popup shows its icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CandidateKind {
    Keyword,
    Local,
    Parameter,
    Class,
    Interface,
    Enum,
    EnumMember,
    TypeAlias,
    Function,
    Method,
    Constructor,
    Field,
    Property,
    Constant,
    Module,
    Macro,
}

impl CandidateKind {
    pub fn is_type(self) -> bool {
        matches!(self, CandidateKind::Class | CandidateKind::Interface | CandidateKind::Enum | CandidateKind::TypeAlias)
    }

    pub fn is_callable(self) -> bool {
        matches!(self, CandidateKind::Function | CandidateKind::Method | CandidateKind::Constructor | CandidateKind::Macro)
    }
}

/// Where a candidate comes from, nearest first: IntelliJ lists locals,
/// then members of the class being edited, then the rest of the project,
/// then libraries; keywords sort among them by match only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    Local,
    Member,
    File,
    Project,
    Library,
    Keyword,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub name: String,
    pub kind: CandidateKind,
    pub scope: Scope,
    /// The declaring type or namespace, shown greyed after the name.
    pub owner: Option<String>,
    /// The package or module, shown at the right (IntelliJ's "type text").
    pub location: Option<String>,
    /// The qualified name to import when the candidate is picked, when the
    /// file does not see it yet.
    pub import: Option<String>,
}

impl Candidate {
    pub fn new(name: impl Into<String>, kind: CandidateKind, scope: Scope) -> Self {
        Self { name: name.into(), kind, scope, owner: None, location: None, import: None }
    }
}

/// What the text around the caret asks for. Built by the code model.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodeContext {
    /// The identifier part already typed before the caret, as a range of
    /// the text; the picked candidate replaces it.
    pub prefix: Range<usize>,
    /// `x.|`: the candidates are members. `receivers` are the receiver's
    /// type and its supertypes, nearest first, when known.
    pub member_access: bool,
    pub receivers: Vec<String>,
    /// In a comment or a string: no popup.
    pub in_comment_or_string: bool,
    /// The file's package (JVM) or module, for auto-import.
    pub package: Option<String>,
    /// Qualified names the file imports (`a.b.C`, or `a.b.*`).
    pub imports: Vec<String>,
    /// Packages every file sees without an import (`java.lang`, `kotlin`…).
    pub implicit_packages: Vec<String>,
}

/// The code model the completion use case reads: the editor's file, the
/// project and its libraries. Implemented by the index adapter.
pub trait CodeModel {
    /// The context at `offset` of the file being edited.
    fn context(&self, text: &str, offset: usize) -> CodeContext;
    /// Locals and parameters visible at `offset`.
    fn locals(&self, text: &str, offset: usize) -> Vec<Candidate>;
    /// Declarations of the file being edited (its own members first).
    fn file_symbols(&self, text: &str, offset: usize) -> Vec<Candidate>;
    /// The members of these types, nearest type first.
    fn members(&self, types: &[String]) -> Vec<Candidate>;
    /// Project and library declarations whose names match `prefix`, at
    /// most `limit`; types, top-level functions and constants.
    fn declarations(&self, prefix: &str, limit: usize) -> Vec<Candidate>;
    /// The language's keywords.
    fn keywords(&self) -> &'static [&'static str];
}

/// How well `name` matches the typed `prefix`, IntelliJ-style, or `None`.
/// The first letter must match; every next typed letter continues the
/// current word or starts a later one ("gTV" → "getTextView"). Exact case
/// and plain prefixes score higher.
pub fn match_score(prefix: &str, name: &str) -> Option<i32> {
    if prefix.is_empty() {
        return Some(0);
    }
    let p: Vec<char> = prefix.chars().collect();
    let n: Vec<char> = name.chars().collect();
    if !n.first().is_some_and(|c| c.eq_ignore_ascii_case(&p[0])) {
        return None;
    }
    if name.starts_with(prefix) {
        return Some(1000 - (n.len() as i32 - p.len() as i32).min(100) + if name.len() == prefix.len() { 200 } else { 0 });
    }
    let lower = name.to_lowercase();
    if lower.starts_with(&prefix.to_lowercase()) {
        return Some(800 - (n.len() as i32 - p.len() as i32).min(100));
    }
    // Camel humps: each typed letter continues the word or starts a new one.
    let starts: Vec<bool> = (0..n.len())
        .map(|i| i == 0 || n[i].is_uppercase() && !n[i - 1].is_uppercase() || n[i - 1] == '_' || n[i].is_ascii_digit() && !n[i - 1].is_ascii_digit())
        .collect();
    let (mut ni, mut humps) = (0usize, 0);
    for (pi, c) in p.iter().enumerate() {
        let continues = ni < n.len() && n[ni].eq_ignore_ascii_case(c) && (pi == 0 || !c.is_uppercase() || starts[ni]);
        if continues {
            ni += 1;
            continue;
        }
        let next = (ni..n.len()).find(|&i| starts[i] && n[i].eq_ignore_ascii_case(c))?;
        humps += 1;
        ni = next + 1;
    }
    Some(500 - humps * 10 - (n.len() as i32 - p.len() as i32).min(100))
}

/// The candidates matching `prefix`, best first, without duplicates (the
/// same name and kind from a farther scope).
pub fn rank(prefix: &str, candidates: Vec<Candidate>, limit: usize) -> Vec<Candidate> {
    let mut scored: Vec<(i32, Candidate)> = candidates
        .into_iter()
        .filter_map(|c| {
            let score = match_score(prefix, &c.name)?;
            // A keyword shows only once typed out far enough to mean it.
            (c.scope != Scope::Keyword || prefix.len() >= 1).then_some((score, c))
        })
        .collect();
    // Within the same match quality, the nearer scope wins (IntelliJ's
    // "proximity"); then shorter names (while typing), then alphabetical.
    let band = |s: i32| s / 100;
    scored.sort_by(|a, b| {
        band(b.0)
            .cmp(&band(a.0))
            .then(a.1.scope.cmp(&b.1.scope))
            .then(b.0.cmp(&a.0))
            .then(if prefix.is_empty() { std::cmp::Ordering::Equal } else { a.1.name.len().cmp(&b.1.name.len()) })
            .then(a.1.name.to_lowercase().cmp(&b.1.name.to_lowercase()))
    });
    let mut seen = std::collections::HashSet::new();
    scored
        .into_iter()
        .map(|(_, c)| c)
        .filter(|c| seen.insert((c.name.clone(), c.kind.is_callable(), c.kind.is_type(), c.owner.clone(), c.location.clone())))
        .take(limit)
        .collect()
}

/// A language server's completions in the popup's order, as indices into
/// `items` (label, filter text, sort text): those matching the typed
/// `prefix`, better matches first and, within the same match quality, in
/// the server's own order (it knows which are locals and which need an
/// import); items whose label isn't what they filter by (rust-analyzer's
/// `iter().le` for `le`) after the rest. At most `limit`.
pub fn order_server(prefix: &str, items: &[(&str, &str, &str)], limit: usize) -> Vec<usize> {
    let mut scored: Vec<(bool, i32, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, (label, filter, _))| Some((!label.starts_with(filter), match_score(prefix, filter)? / 100, i)))
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(items[a.2].2.cmp(items[b.2].2)).then(a.2.cmp(&b.2)));
    scored.into_iter().map(|(_, _, i)| i).take(limit).collect()
}

/// A snippet's text without its tab stops (`${1:x}` → `x`, `${2|a,b|}` →
/// `a`, `$1` and `$0` dropped, `\$` → `$`), and where the first tab stop
/// is (else `$0`), for the caret.
pub fn plain_snippet(snippet: &str) -> (String, Option<usize>) {
    let chars: Vec<char> = snippet.chars().collect();
    let mut out = String::new();
    // (tab stop number, where it is in `out`)
    let mut stops: Vec<(u32, usize)> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            out.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if c != '$' {
            out.push(c);
            i += 1;
            continue;
        }
        let digits = |from: usize| chars[from..].iter().take_while(|c| c.is_ascii_digit()).count();
        let n = digits(i + 1);
        if n > 0 {
            stops.push((chars[i + 1..i + 1 + n].iter().collect::<String>().parse().unwrap_or(0), out.len()));
            i += 1 + n;
            continue;
        }
        let m = if chars.get(i + 1) == Some(&'{') { digits(i + 2) } else { 0 };
        if m == 0 {
            out.push(c);
            i += 1;
            continue;
        }
        let number = chars[i + 2..i + 2 + m].iter().collect::<String>().parse().unwrap_or(0);
        // The placeholder's text, through its closing brace.
        let mut j = i + 2 + m;
        let mut depth = 1;
        let mut inner = String::new();
        while j < chars.len() {
            match chars[j] {
                '\\' if j + 1 < chars.len() => {
                    inner.push('\\');
                    inner.push(chars[j + 1]);
                    j += 1;
                }
                '{' => {
                    depth += 1;
                    inner.push('{');
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    inner.push('}');
                }
                c => inner.push(c),
            }
            j += 1;
        }
        stops.push((number, out.len()));
        let text = match (inner.strip_prefix(':'), inner.strip_prefix('|')) {
            (Some(text), _) => plain_snippet(text).0,
            (_, Some(choices)) => choices.split([',', '|']).next().unwrap_or("").to_owned(),
            _ => String::new(),
        };
        out.push_str(&text);
        i = j + 1;
    }
    let caret = stops.iter().filter(|(n, _)| *n > 0).min_by_key(|(n, _)| *n).or_else(|| stops.iter().find(|(n, _)| *n == 0)).map(|(_, at)| *at);
    (out, caret)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_server_items() {
        let items = [("print", "print", "09.9999.print"), ("price", "price", "09.0001.price"), ("Path", "Path", "12.0001.Path"), ("xprice", "xprice", "09.0000"), ("pr", "pr", "10")];
        // The exact match, then the server's order among prefix matches.
        assert_eq!(order_server("pr", &items, 10), [4, 1, 0]);
        assert_eq!(order_server("", &items, 2), [3, 1]);
        assert_eq!(order_server("P", &items, 10), [2, 1, 0, 4]);
        // A derived item goes after the plain ones.
        let items = [("iter().le", "le", "0"), ("len", "len", "1"), ("le", "le", "2")];
        assert_eq!(order_server("le", &items, 10), [2, 1, 0]);
    }

    #[test]
    fn reads_snippets() {
        assert_eq!(plain_snippet("greet(${1:name})$0"), ("greet(name)".to_owned(), Some(6)));
        assert_eq!(plain_snippet("foo($0)"), ("foo()".to_owned(), Some(4)));
        assert_eq!(plain_snippet("for ${1:x} in ${2|a,b|} {\n\t$0\n}"), ("for x in a {\n\t\n}".to_owned(), Some(4)));
        assert_eq!(plain_snippet("cost \\$5 ${1:a${2:b}}"), ("cost $5 ab".to_owned(), Some(8)));
        assert_eq!(plain_snippet("plain"), ("plain".to_owned(), None));
    }

    #[test]
    fn camel_humps_like_intellij() {
        assert!(match_score("gTV", "getTextView").is_some());
        assert!(match_score("getTV", "getTextView").is_some());
        assert!(match_score("tv", "TextView").is_some());
        assert!(match_score("TV", "TextView").is_some());
        assert!(match_score("xy", "TextView").is_none());
        // The first letter must match.
        assert!(match_score("View", "TextView").is_none());
        // Exact case beats any case, which beats humps.
        assert!(match_score("Str", "String") > match_score("str", "String"));
        assert!(match_score("str", "String") > match_score("SB", "StringBuilder"));
    }

    #[test]
    fn nearer_scopes_first() {
        let c = |name: &str, scope| Candidate::new(name, CandidateKind::Field, scope);
        let ranked = rank("co", vec![c("count", Scope::Library), c("context", Scope::Project), c("cost", Scope::Local), c("xx", Scope::Local)], 10);
        let names: Vec<&str> = ranked.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["cost", "context", "count"]);
    }
}
