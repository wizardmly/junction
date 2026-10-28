//! Settings › Editor › Code Style: each language's indent (tab or
//! spaces, its size) and continuation indent, and General's "Detect and
//! use existing file indents", as IntelliJ keeps them; what Reformat Code
//! indents with, under `.editorconfig`.

use crate::domain::reformat::{IndentStyle, KeepBlankLines, MinBlankLines, Spaces};

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
    /// General › "Hard wrap at": where Reformat Code wraps long lines.
    pub hard_wrap: usize,
    langs: [LangStyle; 4],
    /// Blank Lines › "Keep maximum blank lines" (XML's are its own).
    blank: [KeepBlankLines; 4],
    /// Blank Lines › "Minimum blank lines" (Java's and Kotlin's).
    min_blank: [MinBlankLines; 4],
    /// Spaces (Java's and Kotlin's).
    spaces: [Spaces; 4],
}

impl Default for CodeStyle {
    /// IntelliJ's defaults: Java and Groovy continue by 8, Kotlin's
    /// official style by 4, Android's XML style by 4.
    fn default() -> Self {
        let style = |continuation| LangStyle { use_tab: false, indent: 4, continuation };
        Self { detect_indents: true, hard_wrap: crate::domain::reformat::RIGHT_MARGIN, langs: [style(8), style(4), style(8), style(4)], blank: Default::default(), min_blank: Default::default(), spaces: Default::default() }
    }
}

impl CodeStyle {
    pub fn get(&self, lang: StyleLang) -> LangStyle {
        self.langs[lang.index()]
    }

    pub fn set(&mut self, lang: StyleLang, style: LangStyle) {
        self.langs[lang.index()] = LangStyle { indent: style.indent.clamp(1, 16), continuation: style.continuation.clamp(0, 32), ..style };
    }

    pub fn keep_blank(&self, lang: StyleLang) -> KeepBlankLines {
        self.blank[lang.index()]
    }

    pub fn set_keep_blank(&mut self, lang: StyleLang, keep: KeepBlankLines) {
        let clamp = |n: usize| n.min(99);
        self.blank[lang.index()] = KeepBlankLines { declarations: clamp(keep.declarations), code: clamp(keep.code), before_rbrace: clamp(keep.before_rbrace) };
    }

    pub fn min_blank(&self, lang: StyleLang) -> MinBlankLines {
        self.min_blank[lang.index()]
    }

    pub fn set_min_blank(&mut self, lang: StyleLang, min: MinBlankLines) {
        self.min_blank[lang.index()] = MinBlankLines::from_values(min.values().map(|n| n.min(99)));
    }

    pub fn spaces(&self, lang: StyleLang) -> Spaces {
        self.spaces[lang.index()]
    }

    pub fn set_spaces(&mut self, lang: StyleLang, spaces: Spaces) {
        self.spaces[lang.index()] = spaces;
    }

    /// What a file of `lang` is indented with: `.editorconfig`'s unit
    /// first, then the file's own when detecting (XML never does), else
    /// the language's; the continuation in proportion to the indent. Long
    /// lines wrap at `.editorconfig`'s `max_line_length`, else "Hard wrap at".
    pub fn indents(&self, lang: StyleLang, editorconfig: Option<String>, max_line: Option<usize>) -> IndentStyle {
        let style = self.get(lang);
        let own = if style.use_tab { "\t".to_owned() } else { " ".repeat(style.indent) };
        let detect = self.detect_indents && lang != StyleLang::Xml;
        IndentStyle { unit: editorconfig.or_else(|| (!detect).then(|| own.clone())), fallback: Some(own), continuation: Some((style.continuation, style.indent)), right_margin: Some(max_line.unwrap_or(self.hard_wrap)), keep_blank: self.keep_blank(lang), min_blank: self.min_blank(lang), spaces: self.spaces(lang) }
    }

    /// The settings file's lines: `code_style_java=false,4,8` (tab,
    /// indent, continuation), `code_style_java_blank=2,2,2` (blank lines
    /// kept in declarations, in code, before `}`) and
    /// `code_style_java_min_blank=1,1,…` (`MinBlankLines::values`) and
    /// `code_style_java_spaces=1…10` (`Spaces::values`) when not
    /// the defaults.
    pub fn lines(&self) -> String {
        let mut out = format!("code_style_detect_indents={}\ncode_style_hard_wrap={}\n", self.detect_indents, self.hard_wrap);
        for lang in StyleLang::ALL {
            let s = self.get(lang);
            out.push_str(&format!("code_style_{}={},{},{}\n", lang.key(), s.use_tab, s.indent, s.continuation));
            let k = self.keep_blank(lang);
            if k != KeepBlankLines::default() {
                out.push_str(&format!("code_style_{}_blank={},{},{}\n", lang.key(), k.declarations, k.code, k.before_rbrace));
            }
            let m = self.min_blank(lang);
            if m != MinBlankLines::default() {
                let values: Vec<String> = m.values().iter().map(usize::to_string).collect();
                out.push_str(&format!("code_style_{}_min_blank={}\n", lang.key(), values.join(",")));
            }
            let spaces = self.spaces(lang);
            if spaces != Spaces::default() {
                let values: String = spaces.values().iter().map(|v| if *v { '1' } else { '0' }).collect();
                out.push_str(&format!("code_style_{}_spaces={values}\n", lang.key()));
            }
        }
        out
    }

    /// Takes one settings line's key and value; false when it isn't one.
    pub fn read(&mut self, key: &str, value: &str) -> bool {
        if key == "code_style_detect_indents" {
            self.detect_indents = value == "true";
            return true;
        }
        if key == "code_style_hard_wrap" {
            if let Ok(n) = value.trim().parse::<usize>() {
                self.hard_wrap = n.clamp(10, 1000);
            }
            return true;
        }
        if let Some(lang) = key.strip_prefix("code_style_").and_then(|k| k.strip_suffix("_spaces")).and_then(|k| StyleLang::ALL.into_iter().find(|l| l.key() == k)) {
            let values: Vec<bool> = value.trim().chars().map(|c| c == '1').collect();
            if let Ok(values) = <[bool; 14]>::try_from(values) {
                self.set_spaces(lang, Spaces::from_values(values));
            }
            return true;
        }
        if let Some(lang) = key.strip_prefix("code_style_").and_then(|k| k.strip_suffix("_min_blank")).and_then(|k| StyleLang::ALL.into_iter().find(|l| l.key() == k)) {
            let n: Option<Vec<usize>> = value.split(',').map(|n| n.trim().parse().ok()).collect();
            if let Some(min) = n.and_then(|n| MinBlankLines::from_prefix(&n)) {
                self.set_min_blank(lang, min);
            }
            return true;
        }
        if let Some(lang) = key.strip_prefix("code_style_").and_then(|k| k.strip_suffix("_blank")).and_then(|k| StyleLang::ALL.into_iter().find(|l| l.key() == k)) {
            let n: Vec<Option<usize>> = value.split(',').map(|n| n.trim().parse().ok()).collect();
            if let [Some(declarations), Some(code), Some(before_rbrace)] = n.as_slice() {
                self.set_keep_blank(lang, KeepBlankLines { declarations: *declarations, code: *code, before_rbrace: *before_rbrace });
            }
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
        let java = style.indents(StyleLang::Java, None, None);
        assert_eq!((java.unit, java.fallback.as_deref(), java.continuation), (None, Some("    "), Some((8, 4))));
        // `.editorconfig` wins.
        assert_eq!(style.indents(StyleLang::Kotlin, Some("  ".into()), None).unit.as_deref(), Some("  "));
        // XML never detects.
        assert_eq!(style.indents(StyleLang::Xml, None, None).unit.as_deref(), Some("    "));
        style.detect_indents = false;
        style.set(StyleLang::Kotlin, LangStyle { use_tab: true, indent: 4, continuation: 4 });
        assert_eq!(style.indents(StyleLang::Kotlin, None, None).unit.as_deref(), Some("\t"));
        // `max_line_length` wins over "Hard wrap at".
        style.hard_wrap = 100;
        assert_eq!(style.indents(StyleLang::Java, None, None).right_margin, Some(100));
        assert_eq!(style.indents(StyleLang::Java, None, Some(80)).right_margin, Some(80));
    }

    #[test]
    fn round_trips_settings_lines() {
        let mut style = CodeStyle::default();
        style.detect_indents = false;
        style.hard_wrap = 80;
        style.set(StyleLang::Groovy, LangStyle { use_tab: false, indent: 2, continuation: 4 });
        style.set_keep_blank(StyleLang::Kotlin, KeepBlankLines { declarations: 1, code: 0, before_rbrace: 0 });
        assert!(style.lines().contains("code_style_kotlin_blank=1,0,0\n") && !style.lines().contains("java_blank"));
        assert_eq!(style.indents(StyleLang::Kotlin, None, None).keep_blank.code, 0);
        style.set_min_blank(StyleLang::Java, MinBlankLines { around_field: 1, ..Default::default() });
        assert!(style.lines().contains("code_style_java_min_blank=1,1,1,1,0,0,1,1,1,1,0,1,0,0,0,0\n") && !style.lines().contains("kotlin_min_blank"));
        assert_eq!(style.indents(StyleLang::Java, None, None).min_blank.around_field, 1);
        style.set_spaces(StyleLang::Kotlin, Spaces { after_comma: false, ..Default::default() });
        assert!(style.lines().contains("code_style_kotlin_spaces=11111111111100\n"));
        assert!(!style.indents(StyleLang::Kotlin, None, None).spaces.after_comma);
        let mut read = CodeStyle::default();
        for line in style.lines().lines() {
            let (k, v) = line.split_once('=').unwrap();
            assert!(read.read(k, v));
        }
        assert_eq!(read, style);
        assert!(!read.read("code_subject", "1"));
        // An older file's twelve values: the rest the defaults.
        assert!(read.read("code_style_kotlin_min_blank", "1,1,1,1,2,0,0,1,1,1,0,1"));
        assert_eq!(read.min_blank(StyleLang::Kotlin), MinBlankLines { after_class_header: 2, ..Default::default() });
        assert_eq!(StyleLang::of_path("app/build.gradle.kts"), Some(StyleLang::Kotlin));
        assert_eq!(StyleLang::of_path("build.gradle"), Some(StyleLang::Groovy));
    }
}
