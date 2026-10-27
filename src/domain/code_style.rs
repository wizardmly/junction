//! Settings › Editor › Code Style: each language's indent (tab or
//! spaces, its size) and continuation indent, and General's "Detect and
//! use existing file indents", as IntelliJ keeps them; what Reformat Code
//! indents with, under `.editorconfig`.

use crate::domain::reformat::IndentStyle;

/// The languages with a code style page.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StyleLang {
    Java,
    Kotlin,
    Groovy,
    Xml,
}

impl StyleLang {
    pub const ALL: [StyleLang; 4] = [StyleLang::Java, StyleLang::Kotlin, StyleLang::Groovy, StyleLang::Xml];

    pub fn name(self) -> &'static str {
        match self {
            StyleLang::Java => "Java",
            StyleLang::Kotlin => "Kotlin",
            StyleLang::Groovy => "Groovy",
            StyleLang::Xml => "XML",
        }
    }

    fn key(self) -> &'static str {
        match self {
            StyleLang::Java => "java",
            StyleLang::Kotlin => "kotlin",
            StyleLang::Groovy => "groovy",
            StyleLang::Xml => "xml",
        }
    }

    /// The language Reformat Code formats a file as.
    pub fn of_path(path: &str) -> Option<Self> {
        let ext = path.rsplit_once('.').map(|(_, e)| e)?;
        Some(match ext {
            "java" => StyleLang::Java,
            "kt" | "kts" => StyleLang::Kotlin,
            "gradle" => StyleLang::Groovy,
            "xml" => StyleLang::Xml,
            _ => return None,
        })
    }

    fn index(self) -> usize {
        StyleLang::ALL.iter().position(|l| *l == self).unwrap_or(0)
    }
}

/// One language's Tabs and Indents.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LangStyle {
    pub use_tab: bool,
    pub indent: usize,
    pub continuation: usize,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CodeStyle {
    /// General › "Detect and use existing file indents for editing".
    pub detect_indents: bool,
    langs: [LangStyle; 4],
}

impl Default for CodeStyle {
    /// IntelliJ's defaults: Java and Groovy continue by 8, Kotlin's
    /// official style by 4, Android's XML style by 4.
    fn default() -> Self {
        let style = |continuation| LangStyle { use_tab: false, indent: 4, continuation };
        Self { detect_indents: true, langs: [style(8), style(4), style(8), style(4)] }
    }
}

impl CodeStyle {
    pub fn get(&self, lang: StyleLang) -> LangStyle {
        self.langs[lang.index()]
    }

    pub fn set(&mut self, lang: StyleLang, style: LangStyle) {
        self.langs[lang.index()] = LangStyle { indent: style.indent.clamp(1, 16), continuation: style.continuation.clamp(0, 32), ..style };
    }

    /// What a file of `lang` is indented with: `.editorconfig`'s unit
    /// first, then the file's own when detecting (XML never does), else
    /// the language's; the continuation in proportion to the indent.
    pub fn indents(&self, lang: StyleLang, editorconfig: Option<String>) -> IndentStyle {
        let style = self.get(lang);
        let own = if style.use_tab { "\t".to_owned() } else { " ".repeat(style.indent) };
        let detect = self.detect_indents && lang != StyleLang::Xml;
        IndentStyle { unit: editorconfig.or_else(|| (!detect).then(|| own.clone())), fallback: Some(own), continuation: Some((style.continuation, style.indent)) }
    }

    /// The settings file's lines: `code_style_java=false,4,8` (tab,
    /// indent, continuation).
    pub fn lines(&self) -> String {
        let mut out = format!("code_style_detect_indents={}\n", self.detect_indents);
        for lang in StyleLang::ALL {
            let s = self.get(lang);
            out.push_str(&format!("code_style_{}={},{},{}\n", lang.key(), s.use_tab, s.indent, s.continuation));
        }
        out
    }

    /// Takes one settings line's key and value; false when it isn't one.
    pub fn read(&mut self, key: &str, value: &str) -> bool {
        if key == "code_style_detect_indents" {
            self.detect_indents = value == "true";
            return true;
        }
        let Some(lang) = key.strip_prefix("code_style_").and_then(|k| StyleLang::ALL.into_iter().find(|l| l.key() == k)) else { return false };
        let parts: Vec<&str> = value.split(',').map(str::trim).collect();
        if let [tab, indent, continuation] = parts.as_slice() {
            if let (Ok(indent), Ok(continuation)) = (indent.parse(), continuation.parse()) {
                self.set(lang, LangStyle { use_tab: *tab == "true", indent, continuation });
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indents_by_editorconfig_file_then_settings() {
        let mut style = CodeStyle::default();
        // Detecting: the file's unit, 4 spaces when it has none.
        let java = style.indents(StyleLang::Java, None);
        assert_eq!((java.unit, java.fallback.as_deref(), java.continuation), (None, Some("    "), Some((8, 4))));
        // `.editorconfig` wins.
        assert_eq!(style.indents(StyleLang::Kotlin, Some("  ".into())).unit.as_deref(), Some("  "));
        // XML never detects.
        assert_eq!(style.indents(StyleLang::Xml, None).unit.as_deref(), Some("    "));
        style.detect_indents = false;
        style.set(StyleLang::Kotlin, LangStyle { use_tab: true, indent: 4, continuation: 4 });
        assert_eq!(style.indents(StyleLang::Kotlin, None).unit.as_deref(), Some("\t"));
    }

    #[test]
    fn round_trips_settings_lines() {
        let mut style = CodeStyle::default();
        style.detect_indents = false;
        style.set(StyleLang::Groovy, LangStyle { use_tab: false, indent: 2, continuation: 4 });
        let mut read = CodeStyle::default();
        for line in style.lines().lines() {
            let (k, v) = line.split_once('=').unwrap();
            assert!(read.read(k, v));
        }
        assert_eq!(read, style);
        assert!(!read.read("code_subject", "1"));
        assert_eq!(StyleLang::of_path("app/build.gradle.kts"), Some(StyleLang::Kotlin));
        assert_eq!(StyleLang::of_path("build.gradle"), Some(StyleLang::Groovy));
    }
}
