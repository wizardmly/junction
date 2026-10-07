//! The languages GitGlass indexes: file detection, tree-sitter grammars and
//! the language servers tried for each.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub enum Lang {
    C,
    Cpp,
    ObjC,
    Rust,
    Go,
    Java,
    Kotlin,
    Swift,
    Dart,
    V,
    JavaScript,
    TypeScript,
    Tsx,
    Python,
}

impl Lang {
    pub const ALL: [Lang; 14] = [
        Lang::C,
        Lang::Cpp,
        Lang::ObjC,
        Lang::Rust,
        Lang::Go,
        Lang::Java,
        Lang::Kotlin,
        Lang::Swift,
        Lang::Dart,
        Lang::V,
        Lang::JavaScript,
        Lang::TypeScript,
        Lang::Tsx,
        Lang::Python,
    ];

    /// The language of a file, by extension. `.h` is C unless the
    /// repository says otherwise (see [`Lang::for_header`]).
    pub fn from_path(path: &str) -> Option<Lang> {
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())?;
        Some(match ext.as_str() {
            "c" | "h" => Lang::C,
            "cc" | "cpp" | "cxx" | "c++" | "hpp" | "hh" | "hxx" | "h++" | "ipp" | "inl" | "tpp" => Lang::Cpp,
            "m" | "mm" => Lang::ObjC,
            "rs" => Lang::Rust,
            "go" => Lang::Go,
            "java" => Lang::Java,
            "kt" | "kts" => Lang::Kotlin,
            "swift" => Lang::Swift,
            "dart" => Lang::Dart,
            "v" | "vsh" => Lang::V,
            "js" | "mjs" | "cjs" | "jsx" => Lang::JavaScript,
            "ts" | "mts" | "cts" => Lang::TypeScript,
            "tsx" => Lang::Tsx,
            "py" | "pyi" => Lang::Python,
            _ => return None,
        })
    }

    /// A header's language from its text: Objective-C and C++ headers share
    /// `.h` with C.
    pub fn for_header(text: &str) -> Lang {
        let objc = ["@interface", "@protocol", "@class", "#import", "NS_ASSUME_NONNULL"];
        if objc.iter().any(|m| text.contains(m)) {
            return Lang::ObjC;
        }
        let cpp = ["namespace ", "class ", "template<", "template <", "std::", "public:", "private:"];
        if cpp.iter().any(|m| text.contains(m)) {
            return Lang::Cpp;
        }
        Lang::C
    }

    pub fn name(self) -> &'static str {
        match self {
            Lang::C => "C",
            Lang::Cpp => "C++",
            Lang::ObjC => "Objective-C",
            Lang::Rust => "Rust",
            Lang::Go => "Go",
            Lang::Java => "Java",
            Lang::Kotlin => "Kotlin",
            Lang::Swift => "Swift",
            Lang::Dart => "Dart",
            Lang::V => "V",
            Lang::JavaScript => "JavaScript",
            Lang::TypeScript => "TypeScript",
            Lang::Tsx => "TSX",
            Lang::Python => "Python",
        }
    }

    /// The settings key for this language's server command.
    pub fn key(self) -> &'static str {
        match self {
            Lang::C => "c",
            Lang::Cpp => "cpp",
            Lang::ObjC => "objc",
            Lang::Rust => "rust",
            Lang::Go => "go",
            Lang::Java => "java",
            Lang::Kotlin => "kotlin",
            Lang::Swift => "swift",
            Lang::Dart => "dart",
            Lang::V => "v",
            Lang::JavaScript => "javascript",
            Lang::TypeScript => "typescript",
            Lang::Tsx => "tsx",
            Lang::Python => "python",
        }
    }

    /// The LSP `languageId`.
    pub fn language_id(self) -> &'static str {
        match self {
            Lang::C => "c",
            Lang::Cpp => "cpp",
            Lang::ObjC => "objective-c",
            Lang::Rust => "rust",
            Lang::Go => "go",
            Lang::Java => "java",
            Lang::Kotlin => "kotlin",
            Lang::Swift => "swift",
            Lang::Dart => "dart",
            Lang::V => "v",
            Lang::JavaScript => "javascript",
            Lang::TypeScript => "typescript",
            Lang::Tsx => "typescriptreact",
            Lang::Python => "python",
        }
    }

    pub fn grammar(self) -> tree_sitter::Language {
        match self {
            Lang::C => tree_sitter_c::LANGUAGE.into(),
            Lang::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            Lang::ObjC => tree_sitter_objc::LANGUAGE.into(),
            Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
            Lang::Go => tree_sitter_go::LANGUAGE.into(),
            Lang::Java => tree_sitter_java::LANGUAGE.into(),
            Lang::Kotlin => tree_sitter_kotlin_sg::LANGUAGE.into(),
            Lang::Swift => tree_sitter_swift::LANGUAGE.into(),
            Lang::Dart => tree_sitter_dart::LANGUAGE.into(),
            Lang::V => tree_sitter_vlang::LANGUAGE.into(),
            Lang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Lang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Lang::Python => tree_sitter_python::LANGUAGE.into(),
        }
    }

    /// Default language server commands, tried in order until one is on PATH.
    pub fn default_servers(self) -> &'static [&'static str] {
        match self {
            Lang::C | Lang::Cpp => &["clangd"],
            Lang::ObjC => &["clangd", "sourcekit-lsp"],
            Lang::Rust => &["rust-analyzer"],
            Lang::Go => &["gopls"],
            Lang::Java => &["jdtls"],
            Lang::Kotlin => &["kotlin-lsp --stdio", "kotlin-language-server"],
            Lang::Swift => &["sourcekit-lsp"],
            Lang::Dart => &["dart language-server --protocol=lsp"],
            Lang::V => &["v-analyzer"],
            Lang::JavaScript | Lang::TypeScript | Lang::Tsx => &["typescript-language-server --stdio"],
            Lang::Python => &["pyright-langserver --stdio", "pylsp"],
        }
    }

    /// Languages whose symbols this one can reach directly, beyond the
    /// bridges: shared headers (C family), JVM interop, Swift's imported
    /// Objective-C and C, TypeScript's JavaScript.
    pub fn family(self) -> &'static [Lang] {
        match self {
            Lang::C | Lang::Cpp | Lang::ObjC => &[Lang::C, Lang::Cpp, Lang::ObjC],
            Lang::Java | Lang::Kotlin => &[Lang::Java, Lang::Kotlin],
            Lang::Swift => &[Lang::Swift, Lang::ObjC, Lang::C],
            Lang::JavaScript | Lang::TypeScript | Lang::Tsx => &[Lang::JavaScript, Lang::TypeScript, Lang::Tsx],
            Lang::Rust => &[Lang::Rust],
            Lang::Go => &[Lang::Go],
            Lang::Dart => &[Lang::Dart],
            Lang::V => &[Lang::V],
            Lang::Python => &[Lang::Python],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Lang;

    #[test]
    fn detects_languages() {
        assert_eq!(Lang::from_path("src/main.rs"), Some(Lang::Rust));
        assert_eq!(Lang::from_path("app/src/main/java/a/B.java"), Some(Lang::Java));
        assert_eq!(Lang::from_path("build.gradle.kts"), Some(Lang::Kotlin));
        assert_eq!(Lang::from_path("Foo.mm"), Some(Lang::ObjC));
        assert_eq!(Lang::from_path("x.vsh"), Some(Lang::V));
        assert_eq!(Lang::from_path("README.md"), None);
        assert_eq!(Lang::for_header("@interface Foo : NSObject"), Lang::ObjC);
        assert_eq!(Lang::for_header("namespace a { class B; }"), Lang::Cpp);
        assert_eq!(Lang::for_header("int add(int a, int b);"), Lang::C);
    }
}
