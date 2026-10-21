//! Type hierarchy over the built-in index, for the editor gutter's
//! implementation markers as IntelliJ draws them: "Is implemented in" /
//! "Is overridden in" next to an interface, a class with subclasses or a
//! method with overrides, and "Implements" / "Overrides" next to a method
//! that implements or overrides one in a supertype.
//!
//! The index keeps supertypes by simple name; where a Java or Kotlin name
//! is ambiguous, the file's package and imports pick the class it means,
//! as the compiler does. Elsewhere names match by simple name.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::domain::imports::{self, Header};

use super::nav::Target;
use super::store::{FileEntry, ProjectIndex};
use super::symbols::{Symbol, SymbolKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MarkKind {
    /// An interface (or abstract member) with implementations: green I↓.
    Implemented,
    /// A class with subclasses, a method with overrides: blue O↓.
    Overridden,
    /// Implements an interface member: green I↑.
    Implementing,
    /// Overrides a superclass member: blue O↑.
    Overriding,
}

impl MarkKind {
    pub fn goes_down(self) -> bool {
        matches!(self, MarkKind::Implemented | MarkKind::Overridden)
    }
}

#[derive(Clone, Debug)]
pub struct GutterMark {
    /// 0-based line of the declaration's name.
    pub line: u32,
    pub kind: MarkKind,
    /// The declaration's name, for the popup title.
    pub name: String,
    /// Its parameter types, "(Bundle?)", for a function; filled from the
    /// file's text.
    pub signature: String,
    /// Where the marker leads: implementations, or the super declaration.
    pub targets: Vec<Target>,
}

impl GutterMark {
    /// IntelliJ's popup title, Kotlin's wording in a Kotlin file:
    /// "Choose Implementation of onCreate(Bundle?) (6 found)".
    pub fn title(&self, kotlin: bool) -> String {
        let types = self.targets.first().is_some_and(|t| t.label.split(' ').next().is_some_and(|k| k != "method" && k != "function" && k != "property" && k != "field"));
        let name = format!("{}{}", self.name, self.signature);
        let n = self.targets.len();
        let what = match self.kind {
            MarkKind::Implemented if types => "Choose Implementation of",
            MarkKind::Overridden if types => "Choose Subclass of",
            MarkKind::Implemented => "Choose Implementation of",
            MarkKind::Overridden if kotlin => "Choose Implementation of",
            MarkKind::Overridden => "Choose Overriding Method of",
            MarkKind::Implementing | MarkKind::Overriding => "Choose Super Method of",
        };
        format!("{what} {name} ({n} found)")
    }
}

/// "(Bundle?, Int)": the parameter types of the function named `name` whose
/// declaration starts on `line`.
pub fn signature(text: &str, line: u32, name: &str) -> String {
    let Some(start) = (if line == 0 { Some(0) } else { text.match_indices('\n').nth(line as usize - 1).map(|(i, _)| i + 1) }) else { return String::new() };
    let rest = &text[start..];
    let Some(at) = rest.find(&format!("{name}(")).or_else(|| rest.find(&format!("{name} ("))).filter(|i| *i < rest.find('\n').unwrap_or(rest.len())) else { return String::new() };
    let open = start + at + rest[at..].find('(').unwrap_or(0);
    let bytes = text.as_bytes();
    let (mut depth, mut end) = (0i32, open);
    for (i, b) in bytes.iter().enumerate().skip(open) {
        match b {
            b'(' | b'<' | b'[' | b'{' => depth += 1,
            b')' | b'>' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    end = i;
                    break;
                }
            }
            _ => {}
        }
        if i - open > 2000 {
            return String::new();
        }
    }
    if end <= open {
        return String::new();
    }
    // Split at top-level commas.
    let inner = &text[open + 1..end];
    let mut params = Vec::new();
    let (mut depth, mut from) = (0i32, 0usize);
    for (i, c) in inner.char_indices() {
        match c {
            '(' | '<' | '[' | '{' => depth += 1,
            ')' | '>' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                params.push(&inner[from..i]);
                from = i + 1;
            }
            _ => {}
        }
    }
    params.push(&inner[from..]);
    let types: Vec<String> = params
        .iter()
        .map(|p| p.split('=').next().unwrap_or(p).trim())
        .filter(|p| !p.is_empty())
        .map(|p| {
            // Drop annotations.
            let p = p.split_whitespace().filter(|w| !w.starts_with('@')).collect::<Vec<_>>().join(" ");
            match p.split_once(':') {
                // Kotlin / Swift / TS: `name: Type`.
                Some((_, ty)) => ty.trim().to_owned(),
                // Java / C: `final Type name`.
                None => {
                    let words: Vec<&str> = p.split_whitespace().filter(|w| !matches!(*w, "final" | "const" | "vararg")).collect();
                    match words.len() {
                        0 => String::new(),
                        1 => words[0].to_owned(),
                        n => words[..n - 1].join(" "),
                    }
                }
            }
        })
        .collect();
    format!("({})", types.join(", "))
}

const LIMIT: usize = 500;

fn is_abstract(kind: SymbolKind) -> bool {
    matches!(kind, SymbolKind::Interface | SymbolKind::Protocol | SymbolKind::Trait)
}

/// Types that can have subtypes.
fn is_class_like(kind: SymbolKind) -> bool {
    kind.is_type() && kind != SymbolKind::TypeAlias
}

fn is_member(kind: SymbolKind) -> bool {
    matches!(kind, SymbolKind::Method | SymbolKind::Function | SymbolKind::Property | SymbolKind::Field)
}

fn last_segment(container: &str) -> &str {
    container.rsplit('.').next().unwrap_or(container)
}

fn owner_of(s: &Symbol) -> Option<&str> {
    s.container.as_deref().map(last_segment)
}

fn target(path: &str, entry: &FileEntry, s: &Symbol) -> Target {
    Target {
        path: path.to_owned(),
        line: s.line,
        col: s.col,
        name: s.name.clone(),
        label: format!("{} · {}", s.kind.label(), entry.lang.name()),
        container: s.container.clone(),
    }
}

type Decl<'a> = (&'a str, &'a FileEntry, &'a Symbol);

fn is_jvm(path: &str) -> bool {
    path.ends_with(".java") || path.ends_with(".kt") || path.ends_with(".kts")
}

/// A file's text, for its package, imports and qualified supertypes; the
/// first 512 KB.
fn read_text(index: &ProjectIndex, path: &str) -> Option<String> {
    use std::io::Read;
    let full = if ProjectIndex::is_external(path) { std::path::PathBuf::from(path) } else { index.root.join(path) };
    let mut buf = Vec::new();
    std::fs::File::open(full).ok()?.take(512 * 1024).read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Which declaration a type name in a file means. Headers are read only
/// for names more than one class carries, and once per file.
struct Scopes<'a> {
    index: &'a ProjectIndex,
    read: &'a dyn Fn(&str) -> Option<String>,
    /// Each file's header, and its text for qualifiers.
    headers: HashMap<String, Option<(Header, String)>>,
    /// Class-like declarations per name, with their qualified names; none
    /// when the name is unique.
    decls: HashMap<String, (Vec<Decl<'a>>, Vec<String>)>,
    /// What a name means where it is used: (file, line of the naming type,
    /// name).
    meant: HashMap<(String, Option<u32>, String), Option<Vec<Decl<'a>>>>,
}

impl<'a> Scopes<'a> {
    fn new(index: &'a ProjectIndex, read: &'a dyn Fn(&str) -> Option<String>) -> Self {
        Scopes { index, read, headers: HashMap::new(), decls: HashMap::new(), meant: HashMap::new() }
    }

    fn header(&mut self, path: &str) -> Option<&Header> {
        if !self.headers.contains_key(path) {
            let header = (self.read)(path).map(|text| (imports::header(&text), text));
            self.headers.insert(path.to_owned(), header);
        }
        self.headers.get(path)?.as_ref().map(|(h, _)| h)
    }

    /// The declarations `name` means in the file at `from`, named by the
    /// type declared on line `by` there; `None` when it can't tell, or
    /// there is only one.
    fn meant(&mut self, from: &str, by: Option<u32>, name: &str) -> Option<Vec<Decl<'a>>> {
        if !is_jvm(from) {
            return None;
        }
        let key = (from.to_owned(), by, name.to_owned());
        if let Some(meant) = self.meant.get(&key) {
            return meant.clone();
        }
        let meant = self.resolve(from, by, name);
        self.meant.insert(key, meant.clone());
        meant
    }

    fn resolve(&mut self, from: &str, by: Option<u32>, name: &str) -> Option<Vec<Decl<'a>>> {
        if !self.decls.contains_key(name) {
            let found: Vec<Decl<'a>> = self.index.symbols_named(name).filter(|(_, _, s)| is_class_like(s.kind)).collect();
            let (mut decls, mut qualified) = (Vec::new(), Vec::new());
            if found.len() > 1 {
                for d in found {
                    let package = if is_jvm(d.0) { self.header(d.0).map(|h| h.package.clone()) } else { None };
                    qualified.push(match package {
                        None => String::new(),
                        Some(p) => [p.as_str(), d.2.container.as_deref().unwrap_or(""), name].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join("."),
                    });
                    decls.push(d);
                }
            }
            self.decls.insert(name.to_owned(), (decls, qualified));
        }
        if self.decls[name].0.len() < 2 {
            return None;
        }
        // The file's own class first; not the naming type itself, as in
        // `class Factory : Job.Factory`.
        let here: Vec<Decl<'a>> = self.decls[name].0.iter().filter(|d| d.0 == from && Some(d.2.line) != by).copied().collect();
        if !here.is_empty() {
            return Some(here);
        }
        self.header(from)?;
        let (decls, qualified) = &self.decls[name];
        let (header, text) = self.headers[from].as_ref()?;
        // `Job.Factory`: the index keeps only the last name.
        let qualifiers = by.map(|line| imports::qualifiers(text, line, name)).unwrap_or_default();
        let picked: Vec<usize> = if qualifiers.is_empty() {
            // One the header doesn't point at is a library's or the JDK's,
            // not any of these.
            imports::resolve(header, name, qualified).unwrap_or_default()
        } else {
            let mut picked: Vec<usize> = qualifiers.iter().flat_map(|q| imports::resolve_qualified(header, q, name, qualified)).collect();
            picked.dedup();
            picked
        };
        Some(picked.into_iter().map(|i| decls[i]).collect())
    }

    /// Whether `name`, a supertype of the type on line `by` in the file at
    /// `from`, can mean the declaration at `path`:`line`.
    fn refers_to(&mut self, from: &str, by: u32, name: &str, path: &str, line: u32) -> bool {
        self.meant(from, Some(by), name).is_none_or(|decls| decls.iter().any(|(p, _, s)| *p == path && s.line == line))
    }
}

/// Every project type extending `name` (declared at `decl`, when known),
/// directly or further down.
fn subtypes<'a>(scopes: &mut Scopes<'a>, name: &str, decl: Option<(&str, u32)>) -> Vec<Decl<'a>> {
    let index = scopes.index;
    let mut out = Vec::new();
    let mut seen: HashSet<(&str, u32)> = HashSet::new();
    let mut queue = VecDeque::from([(name.to_owned(), decl.map(|(p, l)| (p.to_owned(), l)))]);
    while let Some((parent, decl)) = queue.pop_front() {
        for (path, entry, s) in index.subtypes_of(&parent) {
            if !is_class_like(s.kind) || seen.contains(&(path, s.line)) {
                continue;
            }
            if decl.as_ref().is_some_and(|(p, l)| !scopes.refers_to(path, s.line, &parent, p, *l)) {
                continue;
            }
            seen.insert((path, s.line));
            out.push((path, entry, s));
            queue.push_back((s.name.clone(), Some((path.to_owned(), s.line))));
            if out.len() >= LIMIT {
                return out;
            }
        }
    }
    out
}

/// As IntelliJ lists them: the project's before libraries', by class
/// name, then location.
fn sort(targets: &mut [Target]) {
    targets.sort_by(|a, b| {
        // The project's first, then libraries'.
        let key = |t: &Target| (ProjectIndex::is_external(&t.path), t.container.clone().unwrap_or_default(), t.name.clone(), t.path.clone(), t.line);
        key(a).cmp(&key(b))
    });
}

/// Members of the types below one type, by member name, built once per
/// type and shared by all of its members.
type MembersBelow<'a> = HashMap<&'a str, Vec<(&'a str, &'a FileEntry, &'a Symbol)>>;

/// The members named `name` with each owner, built once per member name:
/// walking up the supertypes then looks each one up instead of scanning
/// every namesake again.
type ByOwner<'a> = HashMap<&'a str, Vec<(&'a str, &'a FileEntry, &'a Symbol)>>;

/// Per supertype: whether it is abstract, and its own supertypes with the
/// file and line naming them.
struct Parent {
    is_abstract: bool,
    supers: Vec<(String, String, Option<u32>)>,
}

/// The gutter markers of a file, from its current text and symbols.
pub fn gutter_marks(index: &ProjectIndex, path: &str, text: &str, symbols: &[Symbol]) -> Vec<GutterMark> {
    let read = |p: &str| if p == path { Some(text.to_owned()) } else { read_text(index, p) };
    marks_with(index, path, symbols, &read)
}

fn marks_with(index: &ProjectIndex, path: &str, symbols: &[Symbol], read: &dyn Fn(&str) -> Option<String>) -> Vec<GutterMark> {
    let mut out = Vec::new();
    let mut scopes = Scopes::new(index, read);
    // Subtypes per type name, shared by the type and its members.
    let mut below: HashMap<String, Vec<Decl>> = HashMap::new();
    let mut members_below: HashMap<String, MembersBelow> = HashMap::new();
    let mut by_owner: HashMap<&str, ByOwner> = HashMap::new();
    let mut parents: HashMap<(String, Vec<String>), Parent> = HashMap::new();
    let types_here: HashMap<&str, &Symbol> = symbols.iter().filter(|s| is_class_like(s.kind)).map(|s| (s.name.as_str(), s)).collect();

    for s in symbols {
        if is_class_like(s.kind) {
            let subs = below.entry(s.name.clone()).or_insert_with(|| subtypes(&mut scopes, &s.name, Some((path, s.line))));
            let mut targets: Vec<Target> = subs.iter().map(|(p, e, sub)| target(p, e, sub)).collect();
            // A Rust trait is implemented by `impl Trait for T` blocks, known
            // by their methods: one entry per implementing type.
            if s.kind == SymbolKind::Trait {
                let mut types = HashSet::new();
                for (p, e, m) in index.subtypes_of(&s.name) {
                    if is_member(m.kind) && types.insert((p.to_owned(), m.container.clone())) {
                        let mut t = target(p, e, m);
                        t.name = owner_of(m).unwrap_or(&m.name).to_owned();
                        t.container = None;
                        targets.push(t);
                    }
                }
            }
            if !targets.is_empty() {
                sort(&mut targets);
                let kind = if is_abstract(s.kind) { MarkKind::Implemented } else { MarkKind::Overridden };
                out.push(GutterMark { line: s.line, kind, name: s.name.clone(), signature: String::new(), targets });
            }
            continue;
        }
        if !is_member(s.kind) || s.kind == SymbolKind::Field && s.supers.is_empty() {
            continue;
        }
        let Some(owner) = owner_of(s) else { continue };
        let owner_type = types_here.get(owner).copied();
        let abstract_owner = owner_type.is_some_and(|t| is_abstract(t.kind));

        // Down: the same member in subtypes (Rust: in impls of the trait).
        let members = members_below.entry(owner.to_owned()).or_insert_with(|| {
            let subs = below.entry(owner.to_owned()).or_insert_with(|| subtypes(&mut scopes, owner, owner_type.map(|t| (path, t.line))));
            let mut members: MembersBelow = HashMap::new();
            for (p, _, sub) in subs.iter() {
                let Some(entry) = index.file(p) else { continue };
                for m in entry.symbols.iter().filter(|m| is_member(m.kind) && owner_of(m) == Some(sub.name.as_str())) {
                    members.entry(m.name.as_str()).or_default().push((p, entry, m));
                }
            }
            if owner_type.is_some_and(|t| t.kind == SymbolKind::Trait) {
                for (p, e, m) in index.subtypes_of(owner) {
                    if is_member(m.kind) {
                        members.entry(m.name.as_str()).or_default().push((p, e, m));
                    }
                }
            }
            members
        });
        let mut targets: Vec<Target> = members.get(s.name.as_str()).into_iter().flatten().map(|(p, e, m)| target(p, e, m)).collect();
        if !targets.is_empty() {
            sort(&mut targets);
            let kind = if s.decl || abstract_owner { MarkKind::Implemented } else { MarkKind::Overridden };
            out.push(GutterMark { line: s.line, kind, name: s.name.clone(), signature: String::new(), targets });
        }

        // Up: the nearest supertype declaring the same member.
        let mut queue: VecDeque<(String, String, Option<u32>)> = owner_type
            .into_iter()
            .flat_map(|t| t.supers.iter().map(|n| (n.clone(), path.to_owned(), Some(t.line))))
            .chain(s.supers.iter().map(|n| (n.clone(), path.to_owned(), None)))
            .collect();
        if queue.is_empty() {
            continue;
        }
        let namesakes = by_owner.entry(s.name.as_str()).or_insert_with(|| {
            let mut map: ByOwner = HashMap::new();
            for (p, e, m) in index.symbols_named(&s.name) {
                if is_member(m.kind) {
                    if let Some(owner) = owner_of(m) {
                        map.entry(owner).or_default().push((p, e, m));
                    }
                }
            }
            map
        });
        let mut seen: HashSet<String> = HashSet::from([owner.to_owned()]);
        let mut found: Vec<(Target, bool)> = Vec::new();
        while let Some((parent, from, by)) = queue.pop_front() {
            if !seen.insert(parent.clone()) || seen.len() > 64 {
                continue;
            }
            let meant = scopes.meant(&from, by, &parent);
            let key = (parent.clone(), meant.as_ref().map(|d| d.iter().map(|(p, _, s)| format!("{p}:{}", s.line)).collect()).unwrap_or_default());
            let info = parents.entry(key).or_insert_with(|| {
                let mut info = Parent { is_abstract: false, supers: Vec::new() };
                let decls: Vec<Decl> = match &meant {
                    Some(decls) => decls.clone(),
                    None => index.symbols_named(&parent).filter(|(_, _, t)| is_class_like(t.kind)).collect(),
                };
                for (p, _, t) in decls {
                    info.is_abstract |= is_abstract(t.kind);
                    info.supers.extend(t.supers.iter().map(|n| (n.clone(), p.to_owned(), Some(t.line))));
                }
                info
            });
            let inside = |p: &str, m: &Symbol| meant.as_ref().is_none_or(|decls| decls.iter().any(|(dp, _, d)| *dp == p && d.line <= m.line && m.line <= d.end_line));
            for (p, e, m) in namesakes.get(parent.as_str()).into_iter().flatten() {
                if !(*p == path && m.line == s.line) && inside(p, m) {
                    found.push((target(p, e, m), m.decl || info.is_abstract));
                }
            }
            if !found.is_empty() {
                break;
            }
            queue.extend(info.supers.iter().cloned());
        }
        if !found.is_empty() {
            let kind = if found.iter().any(|(_, implements)| *implements) { MarkKind::Implementing } else { MarkKind::Overriding };
            out.push(GutterMark { line: s.line, kind, name: s.name.clone(), signature: String::new(), targets: found.into_iter().map(|(t, _)| t).collect() });
        }
    }
    out
}

/// Type Hierarchy's (Ctrl+H) three views, as IntelliJ's toolbar switches
/// them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeView {
    /// The superclass chain down to the type, then its subtypes.
    Type,
    /// The type, and everything it extends or implements.
    Supertypes,
    /// The type, and everything extending it.
    Subtypes,
}

/// A type in the hierarchy: its declaration (none for a type outside the
/// project and its libraries) and the types under it.
#[derive(Clone, Debug)]
pub struct TypeNode {
    pub name: String,
    pub target: Option<Target>,
    pub symbol: Option<Symbol>,
    pub kotlin: bool,
    pub children: Vec<TypeNode>,
    /// In a Method Hierarchy: whether the type defines the method, and
    /// where.
    pub method: Option<(MethodMark, Option<Target>)>,
}

/// Method Hierarchy's (Ctrl+Shift+H) marks, IntelliJ's legend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MethodMark {
    /// The type defines the method: +.
    Defined,
    /// It doesn't, and inherits it: −.
    Inherited,
    /// It doesn't, but should: a concrete class under an abstract one: !.
    ShouldDefine,
}

/// A type's declaration: the project's first, then the libraries'.
fn declaration<'a>(index: &'a ProjectIndex, name: &str) -> Option<(&'a str, &'a FileEntry, &'a Symbol)> {
    index.symbols_named(name).find(|(_, _, s)| is_class_like(s.kind))
}

fn type_node(index: &ProjectIndex, name: &str) -> TypeNode {
    let decl = declaration(index, name);
    TypeNode {
        name: name.to_owned(),
        target: decl.map(|(p, e, s)| target(p, e, s)),
        symbol: decl.map(|(_, _, s)| s.clone()),
        kotlin: decl.is_some_and(|(_, e, _)| e.lang == super::lang::Lang::Kotlin),
        children: Vec::new(),
        method: None,
    }
}

fn subtype_tree(index: &ProjectIndex, name: &str, seen: &mut HashSet<String>, budget: &mut usize) -> Vec<TypeNode> {
    let mut found: Vec<(&str, &FileEntry, &Symbol)> = Vec::new();
    for (p, e, s) in index.subtypes_of(name) {
        if is_class_like(s.kind) && !found.iter().any(|(fp, _, fs)| *fp == p && fs.line == s.line) {
            found.push((p, e, s));
        }
    }
    found.sort_by(|a, b| (ProjectIndex::is_external(a.0), &a.2.name).cmp(&(ProjectIndex::is_external(b.0), &b.2.name)));
    let mut out = Vec::new();
    for (p, e, s) in found {
        if *budget == 0 {
            break;
        }
        *budget -= 1;
        let mut node = TypeNode { name: s.name.clone(), target: Some(target(p, e, s)), symbol: Some(s.clone()), kotlin: e.lang == super::lang::Lang::Kotlin, children: Vec::new(), method: None };
        if seen.insert(s.name.clone()) {
            node.children = subtype_tree(index, &s.name, seen, budget);
        }
        out.push(node);
    }
    out
}

fn supertype_tree(index: &ProjectIndex, name: &str, seen: &mut HashSet<String>, budget: &mut usize) -> Vec<TypeNode> {
    let supers = declaration(index, name).map(|(_, _, s)| s.supers.clone()).unwrap_or_default();
    let mut out = Vec::new();
    for parent in supers {
        if *budget == 0 {
            break;
        }
        *budget -= 1;
        let mut node = type_node(index, &parent);
        if seen.insert(parent.clone()) {
            node.children = supertype_tree(index, &parent, seen, budget);
        }
        out.push(node);
    }
    out
}

/// A type's superclass: the first supertype that isn't an interface (one
/// not found counts, as Kotlin lists the class first).
fn superclass(index: &ProjectIndex, name: &str) -> Option<String> {
    let (_, _, s) = declaration(index, name)?;
    if is_abstract(s.kind) {
        return None;
    }
    s.supers.iter().find(|p| declaration(index, p).is_none_or(|(_, _, d)| !is_abstract(d.kind))).cloned()
}

/// The hierarchy of the type named `name`, in one of the three views.
pub fn type_hierarchy(index: &ProjectIndex, name: &str, view: TypeView) -> TypeNode {
    let mut seen = HashSet::from([name.to_owned()]);
    let mut budget = LIMIT;
    let mut node = type_node(index, name);
    match view {
        TypeView::Supertypes => node.children = supertype_tree(index, name, &mut seen, &mut budget),
        TypeView::Subtypes => node.children = subtype_tree(index, name, &mut seen, &mut budget),
        TypeView::Type => {
            node.children = subtype_tree(index, name, &mut seen, &mut budget);
            let mut at = name.to_owned();
            while let Some(parent) = superclass(index, &at) {
                if !seen.insert(parent.clone()) {
                    break;
                }
                let mut up = type_node(index, &parent);
                up.children = vec![node];
                node = up;
                at = parent;
            }
            // A class's chain starts at Any (Kotlin) / Object (Java), as
            // IntelliJ shows it; an interface's doesn't.
            let class = node.symbol.as_ref().is_some_and(|s| !is_abstract(s.kind) && s.kind != SymbolKind::Enum);
            let root = if node.kotlin { "Any" } else { "Object" };
            if class && node.name != root && !seen.contains(root) {
                let mut top = type_node(index, root);
                top.children = vec![node];
                node = top;
            }
        }
    }
    node
}

/// The member `method` of the type named `owner`.
fn member_of<'a>(index: &'a ProjectIndex, owner: &str, method: &str) -> Option<(&'a str, &'a FileEntry, &'a Symbol)> {
    index.symbols_named(method).find(|(_, _, m)| is_member(m.kind) && owner_of(m) == Some(owner))
}

fn mark_method(index: &ProjectIndex, node: &mut TypeNode, method: &str, must_define: bool) {
    let concrete = node.symbol.as_ref().is_some_and(|s| !is_abstract(s.kind) && s.detail.as_deref() != Some("abstract"));
    let (mark, must_define) = match member_of(index, &node.name, method) {
        Some((p, e, m)) => {
            node.method = Some((MethodMark::Defined, Some(target(p, e, m))));
            // An interface's or abstract member: subclasses must define it.
            let abstract_member = m.decl && !concrete;
            return node.children.iter_mut().for_each(|c| mark_method(index, c, method, abstract_member));
        }
        None if must_define && concrete => (MethodMark::ShouldDefine, false),
        None => (MethodMark::Inherited, must_define),
    };
    node.method = Some((mark, None));
    for c in &mut node.children {
        mark_method(index, c, method, must_define);
    }
}

/// The Method Hierarchy of `method` in the type named `owner`: from the
/// topmost supertype declaring it, every subtype, each marked.
pub fn method_hierarchy(index: &ProjectIndex, owner: &str, method: &str) -> TypeNode {
    let mut top = owner.to_owned();
    let mut seen = HashSet::from([owner.to_owned()]);
    'up: loop {
        let mut queue: VecDeque<String> = declaration(index, &top).map(|(_, _, s)| s.supers.clone()).unwrap_or_default().into();
        while let Some(parent) = queue.pop_front() {
            if !seen.insert(parent.clone()) || seen.len() > 64 {
                continue;
            }
            if member_of(index, &parent, method).is_some() {
                top = parent;
                continue 'up;
            }
            queue.extend(declaration(index, &parent).map(|(_, _, s)| s.supers.clone()).unwrap_or_default());
        }
        break;
    }
    let mut node = type_node(index, &top);
    let mut seen = HashSet::from([top.clone()]);
    let mut budget = LIMIT;
    node.children = subtype_tree(index, &top, &mut seen, &mut budget);
    mark_method(index, &mut node, method, false);
    node
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::lang::Lang;
    use crate::index::symbols;

    fn index_of(files: &[(&str, Lang, &str)]) -> ProjectIndex {
        let mut index = ProjectIndex::default();
        for (path, lang, text) in files {
            let entry = FileEntry { lang: *lang, mtime: 0, size: 0, symbols: symbols::extract(*lang, text), bridges: Vec::new() };
            index.insert_for_test(path, entry);
        }
        index
    }

    fn marks(index: &ProjectIndex, path: &str) -> Vec<(u32, MarkKind, Vec<String>)> {
        let symbols = index.files[path].symbols.clone();
        marks_with(index, path, &symbols, &|_| None)
            .into_iter()
            .map(|m| (m.line, m.kind, m.targets.iter().map(|t| format!("{}:{}", t.path, t.line)).collect()))
            .collect()
    }

    #[test]
    fn kotlin_interface_and_overrides() {
        let index = index_of(&[
            ("Shape.kt", Lang::Kotlin, "interface Shape {\n    fun area(): Double\n}\nopen class Base : Shape {\n    override fun area() = 1.0\n}\n"),
            ("Square.kt", Lang::Kotlin, "class Square : Base() {\n    override fun area() = 2.0\n}\n"),
        ]);
        assert_eq!(
            marks(&index, "Shape.kt"),
            [
                (0, MarkKind::Implemented, vec!["Shape.kt:3".to_owned(), "Square.kt:0".to_owned()]),
                (1, MarkKind::Implemented, vec!["Shape.kt:4".to_owned(), "Square.kt:1".to_owned()]),
                (3, MarkKind::Overridden, vec!["Square.kt:0".to_owned()]),
                (4, MarkKind::Overridden, vec!["Square.kt:1".to_owned()]),
                (4, MarkKind::Implementing, vec!["Shape.kt:1".to_owned()]),
            ]
        );
        assert_eq!(marks(&index, "Square.kt"), [(1, MarkKind::Overriding, vec!["Shape.kt:4".to_owned()])]);
    }

    #[test]
    fn imports_pick_the_supertype() {
        // Two classes called Base; each subclass imports one of them.
        let files = [
            ("a/Base.kt", Lang::Kotlin, "package a\n\nopen class Base {\n    open fun run() {}\n}\n"),
            ("b/Base.java", Lang::Java, "package b;\n\npublic class Base {\n    public void run() {}\n}\n"),
            ("c/One.kt", Lang::Kotlin, "package c\n\nimport a.Base\n\nclass One : Base() {\n    override fun run() {}\n}\n"),
            ("c/Two.java", Lang::Java, "package c;\n\nimport b.*;\n\nclass Two extends Base {\n    public void run() {}\n}\n"),
            ("b/Three.java", Lang::Java, "package b;\n\nclass Three extends Base {}\n"),
        ];
        let index = index_of(&files);
        let texts: HashMap<&str, &str> = files.iter().map(|(p, _, t)| (*p, *t)).collect();
        let marks = |path: &str| {
            let symbols = index.files[path].symbols.clone();
            marks_with(&index, path, &symbols, &|p| texts.get(p).map(|t| t.to_string()))
                .into_iter()
                .map(|m| (m.line, m.targets.iter().map(|t| t.path.clone()).collect::<Vec<_>>()))
                .collect::<Vec<_>>()
        };
        assert_eq!(marks("a/Base.kt"), [(2, vec!["c/One.kt".to_owned()]), (3, vec!["c/One.kt".to_owned()])]);
        assert_eq!(marks("b/Base.java"), [(2, vec!["b/Three.java".to_owned(), "c/Two.java".to_owned()]), (3, vec!["c/Two.java".to_owned()])]);
        assert_eq!(marks("c/One.kt"), [(5, vec!["a/Base.kt".to_owned()])]);
        assert_eq!(marks("c/Two.java"), [(5, vec!["b/Base.java".to_owned()])]);

        // `class Factory : Job.Factory`: nested classes of one name.
        let files = [
            ("j/Job.kt", Lang::Kotlin, "package j\n\nabstract class Job {\n    interface Factory\n}\n"),
            ("k/Other.kt", Lang::Kotlin, "package k\n\nclass Other {\n    interface Factory\n}\n"),
            ("w/Work.kt", Lang::Kotlin, "package w\n\nimport j.Job\n\nclass Work : Job() {\n    class Factory : Job.Factory\n}\n"),
        ];
        let index = index_of(&files);
        let texts: HashMap<&str, &str> = files.iter().map(|(p, _, t)| (*p, *t)).collect();
        let marks = |path: &str| {
            let symbols = index.files[path].symbols.clone();
            marks_with(&index, path, &symbols, &|p| texts.get(p).map(|t| t.to_string())).into_iter().map(|m| (m.line, m.targets.len())).collect::<Vec<_>>()
        };
        assert_eq!(marks("j/Job.kt"), [(2, 1), (3, 1)]);
        assert_eq!(marks("k/Other.kt"), []);
    }

    #[test]
    fn java_swift_cpp_rust() {
        let java = index_of(&[("A.java", Lang::Java, "interface I { void run(); }\nclass A extends java.util.Base<String> implements I, J<X> { public void run() {} }\n")]);
        assert_eq!(java.files["A.java"].symbols.iter().find(|s| s.name == "A").unwrap().supers, ["Base", "I", "J"]);
        assert_eq!(marks(&java, "A.java").len(), 3);

        let swift = index_of(&[("a.swift", Lang::Swift, "protocol P { func go() }\nclass A: NSObject, P {\n    func go() {}\n}\n")]);
        assert_eq!(swift.files["a.swift"].symbols.iter().find(|s| s.name == "A").unwrap().supers, ["NSObject", "P"]);
        assert_eq!(marks(&swift, "a.swift").iter().map(|m| m.1).collect::<Vec<_>>(), [MarkKind::Implemented, MarkKind::Implemented, MarkKind::Implementing]);

        let cpp = index_of(&[("a.h", Lang::Cpp, "class B { public: virtual void f() = 0; };\nclass A : public B, private ns::C<int> { public: void f() override; };\n")]);
        assert_eq!(cpp.files["a.h"].symbols.iter().find(|s| s.name == "A").unwrap().supers, ["B", "C"]);

        let rust = index_of(&[("a.rs", Lang::Rust, "trait Shape { fn area(&self) -> f64; }\nstruct Sq;\nimpl Shape for Sq {\n    fn area(&self) -> f64 { 1.0 }\n}\n")]);
        assert_eq!(
            marks(&rust, "a.rs"),
            [
                (0, MarkKind::Implemented, vec!["a.rs:3".to_owned()]),
                (0, MarkKind::Implemented, vec!["a.rs:3".to_owned()]),
                (3, MarkKind::Implementing, vec!["a.rs:0".to_owned()]),
            ]
        );
    }

    #[test]
    fn dart_ts_python_objc() {
        let dart = index_of(&[("a.dart", Lang::Dart, "abstract class I { void m(); }\nclass A extends B with M implements I, J<T> { void m() {} }\n")]);
        assert_eq!(dart.files["a.dart"].symbols.iter().find(|s| s.name == "A").unwrap().supers, ["B", "M", "I", "J"]);
        let ts = index_of(&[("a.ts", Lang::TypeScript, "class A extends B<T> implements I { m() {} }\ninterface I extends K, L { m(): void }\n")]);
        assert_eq!(ts.files["a.ts"].symbols.iter().find(|s| s.name == "A").unwrap().supers, ["B", "I"]);
        assert_eq!(ts.files["a.ts"].symbols.iter().find(|s| s.name == "I").unwrap().supers, ["K", "L"]);
        let py = index_of(&[("a.py", Lang::Python, "class A(B, mod.C):\n    def m(self): pass\n")]);
        assert_eq!(py.files["a.py"].symbols.iter().find(|s| s.name == "A").unwrap().supers, ["B", "C"]);
        let objc = index_of(&[("a.m", Lang::ObjC, "@interface A : NSObject <P, Q>\n@end\n@protocol P <R>\n@end\n")]);
        assert_eq!(objc.files["a.m"].symbols.iter().find(|s| s.name == "A").unwrap().supers, ["NSObject", "P", "Q"]);
        assert_eq!(objc.files["a.m"].symbols.iter().find(|s| s.name == "P").unwrap().supers, ["R"]);
    }

    #[test]
    fn type_hierarchy_views() {
        let index = index_of(&[
            ("Shape.kt", Lang::Kotlin, "interface Shape\nopen class Base : Shape\nopen class Square : Base(), Comparable<Square>\nclass Tile : Square()\nclass Disc : Base()\n"),
        ]);
        fn show(n: &TypeNode, depth: usize, out: &mut Vec<String>) {
            out.push(format!("{}{}{}", "  ".repeat(depth), n.name, if n.target.is_some() { "" } else { "?" }));
            for c in &n.children {
                show(c, depth + 1, out);
            }
        }
        let lines = |view| {
            let mut out = Vec::new();
            show(&type_hierarchy(&index, "Square", view), 0, &mut out);
            out
        };
        assert_eq!(lines(TypeView::Type), ["Any?", "  Base", "    Square", "      Tile"]);
        assert_eq!(lines(TypeView::Supertypes), ["Square", "  Base", "    Shape", "  Comparable?"]);
        assert_eq!(lines(TypeView::Subtypes), ["Square", "  Tile"]);
        let mut out = Vec::new();
        show(&type_hierarchy(&index, "Shape", TypeView::Type), 0, &mut out);
        assert_eq!(out, ["Shape", "  Base", "    Disc", "    Square", "      Tile"]);
    }

    #[test]
    fn method_hierarchy_marks() {
        let index = index_of(&[(
            "Shape.kt",
            Lang::Kotlin,
            "interface Shape {\n    fun area(): Double\n}\nabstract class Base : Shape\nopen class Square : Base() {\n    override fun area() = 1.0\n}\nclass Tile : Square()\nclass Disc : Base()\n",
        )]);
        fn show(n: &TypeNode, out: &mut Vec<String>) {
            let mark = match n.method {
                Some((MethodMark::Defined, _)) => "+",
                Some((MethodMark::Inherited, _)) => "-",
                Some((MethodMark::ShouldDefine, _)) => "!",
                None => "?",
            };
            out.push(format!("{mark}{}", n.name));
            n.children.iter().for_each(|c| show(c, out));
        }
        let mut out = Vec::new();
        show(&method_hierarchy(&index, "Square", "area"), &mut out);
        assert_eq!(out, ["+Shape", "-Base", "!Disc", "+Square", "-Tile"]);
    }
}

