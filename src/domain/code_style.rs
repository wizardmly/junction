//! Settings › Editor › Code Style: each language's indent (tab or
//! spaces, its size) and continuation indent, and General's "Detect and
//! use existing file indents", as IntelliJ keeps them; what Reformat Code
//! indents with, under `.editorconfig`.

use crate::domain::imports::{ImportStyle, StarImports};
use crate::domain::line_edit::LineComments;
use crate::domain::reformat::{AnnotationWrap, Braces, Force, IndentStyle, KeepBlankLines, KeepOneLine, MinBlankLines, Operations, Spaces, Wrap, Wrapping};

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
    /// Code Generation › Comment Code (not XML's).
    comments: [LineComments; 4],
    /// Wrapping and Braces (Java's and Kotlin's).
    wrapping: [Wrapping; 4],
    /// Imports › when Optimize Imports uses `*` (Java's, Kotlin's).
    stars: [StarImports; 2],
}

impl Default for CodeStyle {
    /// IntelliJ's defaults: Java and Groovy continue by 8, Kotlin's
    /// official style by 4, Android's XML style by 4.
    fn default() -> Self {
        let style = |continuation| LangStyle { use_tab: false, indent: 4, continuation };
        Self { detect_indents: true, hard_wrap: crate::domain::reformat::RIGHT_MARGIN, langs: [style(8), style(4), style(8), style(4)], blank: Default::default(), min_blank: Default::default(), spaces: Default::default(), comments: Default::default(), wrapping: [Wrapping::JAVA, Wrapping::KOTLIN, Wrapping::JAVA, Wrapping::JAVA], stars: [StarImports::default_for(ImportStyle::Java), StarImports::default_for(ImportStyle::Kotlin)] }
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

    pub fn comments(&self, lang: StyleLang) -> LineComments {
        self.comments[lang.index()]
    }

    pub fn set_comments(&mut self, lang: StyleLang, comments: LineComments) {
        self.comments[lang.index()] = comments;
    }

    /// Wrapping and Braces: how long lists, chains and assignments wrap.
    pub fn wrapping(&self, lang: StyleLang) -> Wrapping {
        self.wrapping[lang.index()]
    }

    pub fn set_wrapping(&mut self, lang: StyleLang, wrapping: Wrapping) {
        self.wrapping[lang.index()] = wrapping;
    }

    fn default_wrapping(lang: StyleLang) -> Wrapping {
        if lang == StyleLang::Kotlin { Wrapping::KOTLIN } else { Wrapping::JAVA }
    }

    /// Imports › "Class count to use import with '*'" and the rest, for
    /// Optimize Imports in a file of `style`.
    pub fn star_imports(&self, style: ImportStyle) -> StarImports {
        self.stars[style as usize].clone()
    }

    pub fn set_star_imports(&mut self, style: ImportStyle, stars: StarImports) {
        let packages = stars.packages.into_iter().map(|p| p.trim().to_owned()).filter(|p| !p.is_empty()).collect();
        self.stars[style as usize] = StarImports { names: stars.names.clamp(1, 999), members: stars.members.clamp(1, 999), packages };
    }

    /// What a file of `lang` is indented with: `.editorconfig`'s unit
    /// first, then the file's own when detecting (XML never does), else
    /// the language's; the continuation in proportion to the indent. Long
    /// lines wrap at `.editorconfig`'s `max_line_length`, else "Hard wrap at".
    pub fn indents(&self, lang: StyleLang, editorconfig: Option<String>, max_line: Option<usize>) -> IndentStyle {
        let style = self.get(lang);
        let own = if style.use_tab { "\t".to_owned() } else { " ".repeat(style.indent) };
        let detect = self.detect_indents && lang != StyleLang::Xml;
        IndentStyle { unit: editorconfig.or_else(|| (!detect).then(|| own.clone())), fallback: Some(own), continuation: Some((style.continuation, style.indent)), right_margin: Some(max_line.unwrap_or(self.hard_wrap)), keep_blank: self.keep_blank(lang), min_blank: self.min_blank(lang), spaces: self.spaces(lang), wrapping: matches!(lang, StyleLang::Java | StyleLang::Kotlin).then(|| self.wrapping(lang)) }
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
            let c = self.comments(lang);
            if c != LineComments::default() {
                out.push_str(&format!("code_style_{}_comments={},{}\n", lang.key(), c.first_column, c.add_space));
            }
            let spaces = self.spaces(lang);
            if spaces != Spaces::default() {
                let values: String = spaces.values().iter().map(|v| if *v { '1' } else { '0' }).collect();
                out.push_str(&format!("code_style_{}_spaces={values}\n", lang.key()));
            }
        }
        for lang in [StyleLang::Java, StyleLang::Kotlin] {
            let w = self.wrapping(lang);
            if w != Self::default_wrapping(lang) {
                let mut values: Vec<String> = w.values().iter().map(|v| v.index().to_string()).collect();
                values.push(u8::from(w.keep_line_breaks).to_string());
                values.extend(w.braces.values().iter().map(|b| u8::from(*b).to_string()));
                values.extend(w.braces.force.iter().map(|f| f.index().to_string()));
                let o = w.ops;
                values.extend([o.binary.index().to_string(), u8::from(o.binary_sign_next).to_string(), o.ternary.index().to_string(), u8::from(o.ternary_signs_next).to_string()]);
                values.extend(w.annotations.iter().map(|a| a.index().to_string()));
                values.push(w.enum_constants.index().to_string());
                values.push(w.elvis.index().to_string());
                values.push(w.local_annotations.index().to_string());
                values.push(w.array_init.index().to_string());
                values.push(w.throws_list.index().to_string());
                values.push(w.throws_keyword.index().to_string());
                values.push(w.annotation_params.index().to_string());
                values.push(w.param_annotations.index().to_string());
                values.extend(w.keep_one_line.values().iter().map(|k| u8::from(*k).to_string()));
                out.push_str(&format!("code_style_{}_wrapping={}\n", lang.key(), values.join(",")));
            }
        }
        for (style, key) in [(ImportStyle::Java, "java"), (ImportStyle::Kotlin, "kotlin")] {
            let stars = self.star_imports(style);
            if stars != StarImports::default_for(style) {
                out.push_str(&format!("code_style_{key}_imports={},{},{}\n", stars.names, stars.members, stars.packages.join(";")));
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
        if let Some(lang) = key.strip_prefix("code_style_").and_then(|k| k.strip_suffix("_wrapping")).and_then(|k| StyleLang::ALL.into_iter().find(|l| l.key() == k)) {
            // Five wraps, Keep line breaks, the seven `Braces`, Force braces,
            // binary and ternary wrap and sign, then class / method / field
            // annotations, enum constants, Elvis, local variable annotations,
            // array initializer, throws list, throws keyword, annotation
            // parameters (older files stop sooner).
            let parts: Vec<&str> = value.split(',').map(str::trim).collect();
            let v: Option<Vec<Wrap>> = parts.iter().take(5).map(|n| n.parse().ok().and_then(Wrap::from_index)).collect();
            if let Some(v) = v.and_then(|v| <[Wrap; 5]>::try_from(v).ok()) {
                let mut braces = [false; 7];
                for (i, b) in braces.iter_mut().enumerate() {
                    *b = parts.get(6 + i).is_some_and(|v| *v == "1");
                }
                let mut force = [Force::DoNotForce; 3];
                for (i, f) in force.iter_mut().enumerate() {
                    *f = parts.get(13 + i).and_then(|v| v.parse().ok()).and_then(Force::from_index).unwrap_or_default();
                }
                let wrap_at = |i: usize| parts.get(i).and_then(|v| v.parse().ok()).and_then(Wrap::from_index).unwrap_or(Wrap::DoNotWrap);
                let ops = Operations { binary: wrap_at(16), binary_sign_next: parts.get(17).is_some_and(|v| *v == "1"), ternary: wrap_at(18), ternary_signs_next: parts.get(19).is_some_and(|v| *v == "1") };
                let mut annotations = [AnnotationWrap::DoNotWrap; 3];
                for (i, a) in annotations.iter_mut().enumerate() {
                    *a = parts.get(20 + i).and_then(|v| v.parse().ok()).and_then(AnnotationWrap::from_index).unwrap_or_default();
                }
                self.set_wrapping(lang, Wrapping { keep_line_breaks: parts.get(5).is_none_or(|k| *k != "0"), braces: Braces { force, ..Braces::from_values(braces) }, ops, annotations, enum_constants: parts.get(23).and_then(|v| v.parse().ok()).and_then(AnnotationWrap::from_index).unwrap_or_default(), elvis: parts.get(24).and_then(|v| v.parse().ok()).and_then(Wrap::from_index).unwrap_or(Self::default_wrapping(lang).elvis), local_annotations: parts.get(25).and_then(|v| v.parse().ok()).and_then(AnnotationWrap::from_index).unwrap_or_default(), array_init: wrap_at(26), throws_list: wrap_at(27), throws_keyword: wrap_at(28), annotation_params: wrap_at(29), param_annotations: parts.get(30).and_then(|v| v.parse().ok()).and_then(AnnotationWrap::from_index).unwrap_or_default(), keep_one_line: KeepOneLine::from_values(std::array::from_fn(|i| parts.get(31 + i).is_none_or(|k| *k != "0"))), ..Wrapping::from_values(v) });
            }
            return true;
        }
        if let Some(style) = match key {
            "code_style_java_imports" => Some(ImportStyle::Java),
            "code_style_kotlin_imports" => Some(ImportStyle::Kotlin),
            _ => None,
        } {
            let mut parts = value.trim().splitn(3, ',');
            if let (Some(Ok(names)), Some(Ok(members))) = (parts.next().map(|n| n.trim().parse()), parts.next().map(|n| n.trim().parse())) {
                let packages = parts.next().unwrap_or("").split(';').map(str::to_owned).collect();
                self.set_star_imports(style, StarImports { names, members, packages });
            }
            return true;
        }
        if let Some(lang) = key.strip_prefix("code_style_").and_then(|k| k.strip_suffix("_comments")).and_then(|k| StyleLang::ALL.into_iter().find(|l| l.key() == k)) {
            if let Some((first, space)) = value.trim().split_once(',') {
                self.set_comments(lang, LineComments { first_column: first == "true", add_space: space == "true" });
            }
            return true;
        }
        if let Some(lang) = key.strip_prefix("code_style_").and_then(|k| k.strip_suffix("_spaces")).and_then(|k| StyleLang::ALL.into_iter().find(|l| l.key() == k)) {
            let values: Vec<bool> = value.trim().chars().map(|c| c == '1').collect();
            if let Some(spaces) = Spaces::from_prefix(&values) {
                self.set_spaces(lang, spaces);
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
        style.set_comments(StyleLang::Java, LineComments { first_column: false, add_space: true });
        assert!(style.lines().contains("code_style_java_comments=false,true\n") && !style.lines().contains("kotlin_comments"));
        style.set_spaces(StyleLang::Kotlin, Spaces { after_comma: false, ..Default::default() });
        assert!(style.lines().contains("code_style_kotlin_spaces=111111111111001111011111111111111111111111110010000000000000000000000\n"));
        assert!(!style.indents(StyleLang::Kotlin, None, None).spaces.after_comma);
        style.set_wrapping(StyleLang::Java, Wrapping { call_args: Wrap::ChopIfLong, braces: Braces { else_new_line: true, force: [Force::Always, Force::DoNotForce, Force::WhenMultiline], ..Default::default() }, ops: Operations { binary: Wrap::WrapIfLong, binary_sign_next: true, ternary: Wrap::ChopIfLong, ternary_signs_next: false }, annotations: [AnnotationWrap::WrapAlways, AnnotationWrap::DoNotWrap, AnnotationWrap::WrapIfLong], enum_constants: AnnotationWrap::ChopIfLong, local_annotations: AnnotationWrap::WrapAlways, array_init: Wrap::ChopIfLong, throws_list: Wrap::WrapIfLong, throws_keyword: Wrap::WrapIfLong, annotation_params: Wrap::ChopIfLong, param_annotations: AnnotationWrap::WrapIfLong, keep_one_line: KeepOneLine { methods: false, lambdas: false, ..KeepOneLine::ALL }, ..Wrapping::JAVA });
        style.set_wrapping(StyleLang::Kotlin, Wrapping { keep_line_breaks: false, annotations: [AnnotationWrap::WrapAlways, AnnotationWrap::DoNotWrap, AnnotationWrap::ChopIfLong], local_annotations: AnnotationWrap::WrapIfLong, ..Wrapping::KOTLIN });
        assert!(style.lines().contains("code_style_kotlin_wrapping=2,2,1,1,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,3,0,2,0,1,1,0,0,0,0,0,1,1,1,1,1\n"));
        assert!(style.lines().contains("code_style_java_wrapping=2,0,0,0,0,1,1,0,0,0,0,0,0,2,0,1,1,1,2,0,3,0,1,2,0,3,2,1,1,2,1,1,0,0,1,1\n"));
        style.set_star_imports(ImportStyle::Java, StarImports { names: 99, members: 0, packages: vec!["a.b.*".into(), " ".into(), "c.**".into()] });
        assert!(style.lines().contains("code_style_java_imports=99,1,a.b.*;c.**\n") && !style.lines().contains("kotlin_imports"));
        let mut read = CodeStyle::default();
        for line in style.lines().lines() {
            let (k, v) = line.split_once('=').unwrap();
            assert!(read.read(k, v));
        }
        assert_eq!(read, style);
        assert!(read.read("code_style_kotlin_wrapping", "0,0,0,0,0"));
        assert_eq!(read.wrapping(StyleLang::Kotlin).elvis, Wrap::WrapIfLong);
        assert!(!read.read("code_subject", "1"));
        // An older file's twelve values: the rest the defaults.
        assert!(read.read("code_style_kotlin_min_blank", "1,1,1,1,2,0,0,1,1,1,0,1"));
        assert!(read.read("code_style_kotlin_wrapping", "2,2,1,1,1"));
        assert_eq!(read.wrapping(StyleLang::Kotlin), Wrapping::KOTLIN);
        assert_eq!(read.min_blank(StyleLang::Kotlin), MinBlankLines { after_class_header: 2, ..Default::default() });
        assert_eq!(StyleLang::of_path("app/build.gradle.kts"), Some(StyleLang::Kotlin));
        assert_eq!(StyleLang::of_path("build.gradle"), Some(StyleLang::Groovy));
    }
}
