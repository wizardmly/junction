//! The completion use case's code model over the project's tree-sitter
//! index: the edited file's syntax tree for locals and its own
//! declarations, the index for the project's and libraries' declarations.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::domain::completion::{self, Candidate, CandidateKind, CodeContext, CodeModel, Scope};
use crate::domain::imports::ImportStyle;
use crate::index::lang::Lang;
use crate::index::nav;
use crate::index::store::ProjectIndex;
use crate::index::symbols::{self, SymbolKind};

pub struct IndexCodeModel<'a> {
    index: &'a ProjectIndex,
    lang: Lang,
    /// The file being edited, relative to the project (absolute for a
    /// library file); its indexed symbols are left to the live text.
    path: &'a str,
    /// Packages of declaring files, read once per request.
    packages: RefCell<HashMap<String, Option<String>>>,
}

impl<'a> IndexCodeModel<'a> {
    pub fn new(index: &'a ProjectIndex, lang: Lang, path: &'a str) -> Self {
        Self { index, lang, path, packages: RefCell::default() }
    }

    pub fn import_style(lang: Lang) -> Option<ImportStyle> {
        match lang {
            Lang::Java => Some(ImportStyle::Java),
            Lang::Kotlin => Some(ImportStyle::Kotlin),
            _ => None,
        }
    }

    fn jvm(&self) -> bool {
        matches!(self.lang, Lang::Java | Lang::Kotlin)
    }

    /// The package line of a declaring file (JVM only).
    fn package_of(&self, path: &str) -> Option<String> {
        if let Some(known) = self.packages.borrow().get(path) {
            return known.clone();
        }
        let full = if ProjectIndex::is_external(path) { PathBuf::from(path) } else { self.index.root.join(path) };
        let head = read_head(&full);
        let package = package_line(&head);
        self.packages.borrow_mut().insert(path.to_owned(), package.clone());
        package
    }

    fn candidate(&self, path: &str, lang: Lang, s: &symbols::Symbol, scope: Scope) -> Candidate {
        let mut c = Candidate::new(s.name.clone(), kind_of(s.kind), scope);
        c.owner = s.container.as_deref().map(|o| o.rsplit(['.', ':']).next().unwrap_or(o).to_owned()).filter(|o| !o.is_empty());
        // Top-level types and Kotlin top-level functions are imported.
        if self.jvm() && matches!(lang, Lang::Java | Lang::Kotlin) && c.owner.is_none() {
            if let Some(package) = self.package_of(path) {
                c.import = Some(format!("{package}.{}", s.name));
                c.location = Some(package);
            }
        } else if c.owner.is_none() {
            c.location = Some(short_location(path));
        }
        c
    }
}

impl CodeModel for IndexCodeModel<'_> {
    fn context(&self, text: &str, offset: usize) -> CodeContext {
        let start = text[..offset].char_indices().rev().take_while(|(_, c)| nav::is_word_char(*c)).last().map_or(offset, |(i, _)| i);
        let in_comment_or_string = nav::comments_and_strings(text, self.lang).iter().any(|r| r.start < offset && offset <= r.end);
        let receivers = nav::member_receivers(self.index, self.lang, text, start);
        let (package, imports) = if self.jvm() { (package_line(text), import_lines(text)) } else { (None, Vec::new()) };
        let implicit_packages = Self::import_style(self.lang).map(crate::domain::imports::implicit_packages).unwrap_or_default();
        CodeContext { prefix: start..offset, member_access: receivers.is_some(), receivers: receivers.unwrap_or_default(), in_comment_or_string, package, imports, implicit_packages }
    }

    fn locals(&self, text: &str, offset: usize) -> Vec<Candidate> {
        crate::index::locals::visible(self.lang, text, offset)
            .into_iter()
            .map(|(name, param)| Candidate::new(name, if param { CandidateKind::Parameter } else { CandidateKind::Local }, Scope::Local))
            .collect()
    }

    fn file_symbols(&self, text: &str, offset: usize) -> Vec<Candidate> {
        let line = text[..offset].matches('\n').count() as u32;
        let symbols = symbols::extract(self.lang, text);
        // The types around the caret: their members are `this`'s.
        let enclosing: Vec<&str> = symbols.iter().filter(|s| s.kind.is_type() && s.line <= line && line <= s.end_line).map(|s| s.name.as_str()).collect();
        symbols
            .iter()
            // Not the declaration being typed.
            .filter(|s| !(s.line == line && text[..offset].ends_with(&s.name)))
            .map(|s| {
                let own = s.container.as_deref().and_then(|c| c.rsplit(['.', ':']).next()).is_some_and(|c| enclosing.contains(&c));
                let mut c = Candidate::new(s.name.clone(), kind_of(s.kind), if own { Scope::Member } else { Scope::File });
                c.owner = s.container.clone();
                c
            })
            .collect()
    }

    fn members(&self, types: &[String]) -> Vec<Candidate> {
        let family = self.lang.family();
        let mut out = Vec::new();
        let mut seen_files = std::collections::HashSet::new();
        for ty in types {
            for (path, entry, s) in self.index.symbols_named(ty) {
                if !s.kind.is_type() || !family.contains(&entry.lang) || !seen_files.insert((path.to_owned(), ty.clone())) {
                    continue;
                }
                let scope = if ProjectIndex::is_external(path) { Scope::Library } else { Scope::Project };
                for m in &entry.symbols {
                    let owner = m.container.as_deref().and_then(|c| c.rsplit(['.', ':']).next());
                    if owner == Some(ty.as_str()) && !matches!(m.kind, SymbolKind::Constructor) {
                        let mut c = Candidate::new(m.name.clone(), kind_of(m.kind), scope);
                        c.owner = Some(ty.clone());
                        out.push(c);
                    }
                }
            }
        }
        out
    }

    fn declarations(&self, prefix: &str, limit: usize) -> Vec<Candidate> {
        let family = self.lang.family();
        let first = prefix.chars().next();
        // Best-matching names first, then their declarations.
        let mut names: Vec<(i32, &String)> = self
            .index
            .names()
            .chain(self.index.external_names())
            .filter(|n| n.chars().next().zip(first).is_some_and(|(a, b)| a.eq_ignore_ascii_case(&b)))
            .filter_map(|n| completion::match_score(prefix, n).map(|s| (s, n)))
            .collect();
        names.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.len().cmp(&b.1.len())));
        let mut out = Vec::new();
        for (_, name) in names {
            for (path, entry, s) in self.index.symbols_named(name) {
                // What the file's module and source set can't see isn't offered.
                if path == self.path || !family.contains(&entry.lang) || matches!(s.kind, SymbolKind::Variable | SymbolKind::Constructor | SymbolKind::Module) || !self.index.modules.visible(self.path, path) {
                    continue;
                }
                let scope = if ProjectIndex::is_external(path) { Scope::Library } else { Scope::Project };
                out.push(self.candidate(path, entry.lang, s, scope));
            }
            if out.len() >= limit {
                break;
            }
        }
        out
    }

    fn keywords(&self) -> &'static [&'static str] {
        keywords(self.lang)
    }
}

fn kind_of(kind: SymbolKind) -> CandidateKind {
    match kind {
        SymbolKind::Class | SymbolKind::Struct => CandidateKind::Class,
        SymbolKind::Interface | SymbolKind::Protocol | SymbolKind::Trait => CandidateKind::Interface,
        SymbolKind::Enum => CandidateKind::Enum,
        SymbolKind::EnumMember => CandidateKind::EnumMember,
        SymbolKind::TypeAlias => CandidateKind::TypeAlias,
        SymbolKind::Function | SymbolKind::Extension => CandidateKind::Function,
        SymbolKind::Method => CandidateKind::Method,
        SymbolKind::Constructor => CandidateKind::Constructor,
        SymbolKind::Field | SymbolKind::Variable => CandidateKind::Field,
        SymbolKind::Property => CandidateKind::Property,
        SymbolKind::Constant => CandidateKind::Constant,
        SymbolKind::Module | SymbolKind::Namespace => CandidateKind::Module,
        SymbolKind::Macro => CandidateKind::Macro,
    }
}

/// The start of a file: enough for its package line.
fn read_head(path: &std::path::Path) -> String {
    use std::io::Read as _;
    let mut buf = vec![0u8; 4096];
    let n = std::fs::File::open(path).and_then(|mut f| f.read(&mut buf)).unwrap_or(0);
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

fn package_line(text: &str) -> Option<String> {
    text.lines().map(str::trim).find_map(|l| l.strip_prefix("package ")).map(|p| p.trim_end_matches(';').trim().trim_matches('`').to_owned()).filter(|p| !p.is_empty())
}

fn import_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter_map(|l| l.strip_prefix("import "))
        .map(|i| i.trim_start_matches("static ").trim_end_matches(';').split(" as ").next().unwrap_or("").trim().to_owned())
        .collect()
}

/// A declaring file's name for the popup's right column, outside the JVM.
fn short_location(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_owned()
}

fn keywords(lang: Lang) -> &'static [&'static str] {
    match lang {
        Lang::Kotlin => &[
            "abstract", "annotation", "as", "break", "by", "catch", "class", "companion", "const", "constructor", "continue", "crossinline", "data",
            "do", "else", "enum", "external", "false", "final", "finally", "for", "fun", "get", "if", "import", "in", "infix", "init", "inline",
            "inner", "interface", "internal", "is", "lateinit", "noinline", "null", "object", "open", "operator", "out", "override", "package",
            "private", "protected", "public", "reified", "return", "sealed", "set", "super", "suspend", "tailrec", "this", "throw", "true", "try",
            "typealias", "val", "value", "var", "vararg", "when", "where", "while",
        ],
        Lang::Java => &[
            "abstract", "assert", "boolean", "break", "byte", "case", "catch", "char", "class", "continue", "default", "do", "double", "else",
            "enum", "extends", "false", "final", "finally", "float", "for", "if", "implements", "import", "instanceof", "int", "interface",
            "long", "native", "new", "null", "package", "private", "protected", "public", "record", "return", "short", "static", "super",
            "switch", "synchronized", "this", "throw", "throws", "transient", "true", "try", "var", "void", "volatile", "while", "yield",
        ],
        Lang::Rust => &[
            "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn", "for", "if", "impl",
            "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait",
            "true", "type", "unsafe", "use", "where", "while",
        ],
        Lang::C | Lang::Cpp | Lang::ObjC => &[
            "auto", "bool", "break", "case", "char", "class", "const", "constexpr", "continue", "default", "delete", "do", "double", "else",
            "enum", "explicit", "extern", "false", "float", "for", "if", "inline", "int", "long", "namespace", "new", "nullptr", "private",
            "protected", "public", "return", "short", "signed", "sizeof", "static", "struct", "switch", "template", "this", "true", "typedef",
            "typename", "union", "unsigned", "using", "virtual", "void", "volatile", "while",
        ],
        Lang::Go => &[
            "break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough", "for", "func", "go", "goto", "if",
            "import", "interface", "map", "package", "range", "return", "select", "struct", "switch", "type", "var", "nil", "true", "false",
        ],
        Lang::Swift => &[
            "as", "break", "case", "catch", "class", "continue", "default", "defer", "do", "else", "enum", "extension", "false", "for",
            "func", "guard", "if", "import", "in", "init", "inout", "internal", "is", "let", "nil", "private", "protocol", "public", "return",
            "self", "Self", "static", "struct", "super", "switch", "throw", "throws", "true", "try", "var", "where", "while",
        ],
        Lang::Dart => &[
            "abstract", "as", "async", "await", "break", "case", "catch", "class", "const", "continue", "default", "do", "else", "enum",
            "extends", "extension", "false", "final", "finally", "for", "if", "implements", "import", "in", "is", "late", "mixin", "new",
            "null", "required", "return", "super", "switch", "this", "throw", "true", "try", "var", "void", "while", "with",
        ],
        Lang::Python => &[
            "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except", "False", "finally",
            "for", "from", "global", "if", "import", "in", "is", "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "True",
            "try", "while", "with", "yield",
        ],
        Lang::JavaScript | Lang::TypeScript | Lang::Tsx => &[
            "async", "await", "break", "case", "catch", "class", "const", "continue", "default", "delete", "do", "else", "enum", "export",
            "extends", "false", "finally", "for", "function", "if", "import", "in", "instanceof", "interface", "let", "new", "null", "return",
            "super", "switch", "this", "throw", "true", "try", "type", "typeof", "undefined", "var", "void", "while", "yield",
        ],
        Lang::V => &[
            "as", "break", "const", "continue", "defer", "else", "enum", "false", "fn", "for", "go", "if", "import", "in", "interface", "is",
            "match", "module", "mut", "none", "or", "pub", "return", "struct", "true", "type", "unsafe",
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::store::FileEntry;
    use crate::usecase::complete_code;

    fn project(files: &[(&str, &str)]) -> (tempfile_dir::Dir, ProjectIndex) {
        let dir = tempfile_dir::Dir::new();
        let mut index = ProjectIndex::default();
        index.root = dir.path.clone();
        for (path, text) in files {
            let full = dir.path.join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(&full, text).unwrap();
            let lang = Lang::from_path(path).unwrap();
            index.insert_for_test(path, FileEntry { lang, mtime: 0, size: 0, symbols: symbols::extract(lang, text), bridges: Vec::new() });
        }
        (dir, index)
    }

    mod tempfile_dir {
        pub struct Dir {
            pub path: std::path::PathBuf,
        }
        impl Dir {
            pub fn new() -> Self {
                static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
                let path = std::env::temp_dir().join(format!("junction-complete-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)));
                std::fs::create_dir_all(&path).unwrap();
                Self { path }
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.path);
            }
        }
    }

    fn names(out: &complete_code::Completions) -> Vec<&str> {
        out.items.iter().map(|c| c.candidate.name.as_str()).collect()
    }

    #[test]
    fn kotlin_class_with_auto_import() {
        let (_dir, index) = project(&[("ui/src/main/java/com/app/ui/Badge.kt", "package com.app.ui\n\nclass BadgeView {\n    fun show() {}\n    val count = 0\n}\n")]);
        let text = "package com.app\n\nfun main() {\n    val total = 1\n    Bad\n}\n";
        let at = text.find("Bad\n").unwrap() + 3;
        let model = IndexCodeModel::new(&index, Lang::Kotlin, "app/Main.kt");
        let out = complete_code::complete(&model, text, at, false, Some(ImportStyle::Kotlin));
        assert_eq!(names(&out), ["BadgeView"]);
        assert_eq!(out.items[0].candidate.location.as_deref(), Some("com.app.ui"));
        assert_eq!(out.items[0].import.as_ref().unwrap().1, "\nimport com.app.ui.BadgeView\n");
        // Locals and keywords for a short prefix.
        let text2 = "fun main() {\n    val total = 1\n    t\n}\n";
        let at2 = text2.find("t\n}").unwrap() + 1;
        let out2 = complete_code::complete(&model, text2, at2, false, Some(ImportStyle::Kotlin));
        let n = names(&out2);
        assert_eq!(n[0], "total", "{n:?}");
        assert!(n.contains(&"true") && n.contains(&"try"), "{n:?}");
    }

    #[test]
    fn members_of_a_declared_receiver() {
        let (_dir, index) = project(&[("Badge.kt", "package com.app\n\nclass BadgeView {\n    fun show() {}\n    val count = 0\n}\n")]);
        let text = "fun main() {\n    val badge: BadgeView = make()\n    badge.\n}\n";
        let at = text.find("badge.\n").unwrap() + 6;
        let model = IndexCodeModel::new(&index, Lang::Kotlin, "Main.kt");
        let out = complete_code::complete(&model, text, at, false, Some(ImportStyle::Kotlin));
        assert_eq!(names(&out), ["count", "show"]);
        assert!(out.items.iter().all(|c| c.import.is_none()));
    }

    #[test]
    fn a_fragment_completes_at_the_paused_line() {
        // Evaluate Expression's field: the fragment as if typed at the start of the paused line.
        let (_dir, index) = project(&[("Badge.kt", "package com.app\n\nclass BadgeView {\n    fun show() {}\n    val count = 0\n}\n")]);
        let context = "fun main() {\n    val badge: BadgeView = make()\n    val total = 1\n    println(total)\n    val later = 2\n}\n";
        let at = crate::domain::expression::line_start(context, 3);
        let model = IndexCodeModel::new(&index, Lang::Kotlin, "Main.kt");
        let complete = |fragment: &str| {
            let text = format!("{}{fragment}\n{}", &context[..at], &context[at..]);
            names(&complete_code::complete(&model, &text, at + fragment.len(), true, Some(ImportStyle::Kotlin))).into_iter().map(str::to_owned).collect::<Vec<_>>()
        };
        assert_eq!(complete("badge."), ["count", "show"]);
        let locals = complete("t");
        assert_eq!(locals[0], "total", "{locals:?}");
        assert!(!complete("lat").contains(&"later".to_owned()));
    }

    #[test]
    fn nothing_in_a_comment() {
        let (_dir, index) = project(&[]);
        let text = "// Bad";
        let model = IndexCodeModel::new(&index, Lang::Kotlin, "Main.kt");
        assert!(complete_code::complete(&model, text, text.len(), true, None).items.is_empty());
    }
}
