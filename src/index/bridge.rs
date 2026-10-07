//! The bridge index: places where code crosses a language boundary, keyed so
//! both sides meet.
//!
//! - `jni:Java_pkg_Class_method`: Java `native` / Kotlin `external` methods
//!   and the C / C++ functions implementing them.
//! - `jnidyn:method`: methods registered through `RegisterNatives` tables.
//! - `c:symbol`: the C ABI. Exports are C / C++ / Objective-C definitions,
//!   Rust `#[no_mangle]` / `#[export_name]`, Swift `@_cdecl`, Go `//export`,
//!   V `@[export]`; imports are C prototypes, Rust `extern "C"` blocks,
//!   Dart FFI lookups and `@Native`, Go `C.name`, V `C.name`.
//! - `objc:name`: Swift declarations exposed with `@objc`, and Objective-C
//!   selectors under the name Swift imports them as.

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use super::lang::Lang;
use super::symbols::{Symbol, SymbolKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    /// The implementation other languages reach.
    Export,
    /// A use or declaration of something implemented elsewhere.
    Import,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeItem {
    pub key: String,
    pub role: Role,
    /// The identifier at the site.
    pub name: String,
    pub line: u32,
    pub col: u32,
    /// A symbol in the same file the site stands for (a `RegisterNatives`
    /// entry names its C function).
    pub via: Option<String>,
}

impl BridgeItem {
    /// What kind of bridge, for display.
    pub fn label(&self) -> &'static str {
        match self.key.split_once(':').map(|(k, _)| k) {
            Some("jni") | Some("jnidyn") => "JNI",
            Some("objc") => "Swift / Objective-C",
            _ => "C ABI",
        }
    }
}

/// Byte offsets to (line, UTF-16 column).
struct Lines<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    fn new(text: &'a str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        Self { text, starts }
    }

    fn position(&self, offset: usize) -> (u32, u32) {
        let line = self.starts.partition_point(|&s| s <= offset) - 1;
        let col = self.text[self.starts[line]..offset].encode_utf16().count();
        (line as u32, col as u32)
    }

    fn line_text(&self, line: u32) -> &'a str {
        let start = self.starts.get(line as usize).copied().unwrap_or(self.text.len());
        let end = self.starts.get(line as usize + 1).map_or(self.text.len(), |e| e - 1);
        &self.text[start..end.max(start)]
    }
}

/// JNI's name mangling for one class or method name component.
pub fn jni_mangle(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' => out.push(ch),
            '.' | '/' => out.push('_'),
            '_' => out.push_str("_1"),
            ';' => out.push_str("_2"),
            '[' => out.push_str("_3"),
            other => {
                let mut buf = [0u16; 2];
                for unit in other.encode_utf16(&mut buf) {
                    out.push_str(&format!("_0{:04x}", unit));
                }
            }
        }
    }
    out
}

/// `Java_pkg_Class_method` for a method of `pkg.Outer.Inner`.
pub fn jni_symbol(package: &str, class_path: &str, method: &str) -> String {
    let class = class_path.replace('.', "$");
    let qualified = if package.is_empty() { class } else { format!("{}/{class}", package.replace('.', "/")) };
    format!("Java_{}_{}", jni_mangle(&qualified), jni_mangle(method))
}

static PACKAGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\s*package\s+([\w.]+)").unwrap());
static JVM_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"@file\s*:\s*JvmName\s*\(\s*"(\w+)"\s*\)"#).unwrap());
static NATIVE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bnative\b").unwrap());
static EXTERNAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bexternal\b").unwrap());
static REGISTER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\{\s*"(\w+)"\s*,\s*"[^"]*"\s*,\s*(?:\(\s*void\s*\*\s*\)\s*)?&?\s*(\w+)\s*\}"#).unwrap());
static NO_MANGLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"#\[\s*(?:unsafe\s*\(\s*)?(no_mangle|export_name\s*=\s*"(\w+)")\s*\)?\s*\]"#).unwrap());
static FN_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bfn\s+(\w+)").unwrap());
static CDECL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"@_cdecl\s*\(\s*"(\w+)"\s*\)"#).unwrap());
static FUNC_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bfunc\s+(\w+)").unwrap());
static GO_EXPORT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^//export\s+(\w+)").unwrap());
static C_CALL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bC\.(\w+)").unwrap());
static V_EXPORT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"@\[\s*export\s*:\s*'(\w+)'\s*\]").unwrap());
static DART_LOOKUP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\.(?:lookupFunction|lookup)\s*<[^()]*>\s*\(\s*['"](\w+)['"]"#).unwrap());
static DART_NATIVE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"@(?:Native|FfiNative)\s*<[^@]*?>\s*\(\s*(?:symbol\s*:\s*)?(?:['"](\w+)['"])?"#).unwrap());
static DART_EXTERNAL_FN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bexternal\b[^;(]*?\b(\w+)\s*\(").unwrap());
static OBJC_ATTR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"@objc(?:\s*\(\s*([\w:]+)\s*\))?").unwrap());
static SWIFT_DECL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:func|class|var|let|protocol|enum|init)\b\s*(\w*)").unwrap());
static SWIFT_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"NS_SWIFT_NAME\s*\(\s*(\w+)").unwrap());

/// The bridge sites in one file.
pub fn extract(lang: Lang, path: &str, text: &str, symbols: &[Symbol]) -> Vec<BridgeItem> {
    let lines = Lines::new(text);
    let mut out = Vec::new();
    let mut push = |key: String, role: Role, name: &str, offset: usize, via: Option<String>| {
        let (line, col) = lines.position(offset);
        out.push(BridgeItem { key, role, name: name.to_owned(), line, col, via });
    };
    let at = |s: &Symbol| -> usize {
        // The byte offset of a symbol's name, from its line / UTF-16 column.
        let line_start = lines.starts.get(s.line as usize).copied().unwrap_or(0);
        let line = lines.line_text(s.line);
        let mut units = 0;
        for (i, ch) in line.char_indices() {
            if units >= s.col as usize {
                return line_start + i;
            }
            units += ch.len_utf16();
        }
        line_start + line.len()
    };
    match lang {
        Lang::Java | Lang::Kotlin => {
            let package = PACKAGE.captures(text).map(|c| c[1].to_owned()).unwrap_or_default();
            let marker = if lang == Lang::Java { &*NATIVE } else { &*EXTERNAL };
            let file_class = || {
                if let Some(c) = JVM_NAME.captures(text) {
                    return c[1].to_owned();
                }
                let stem = path.rsplit('/').next().unwrap_or(path).trim_end_matches(".kts").trim_end_matches(".kt");
                let mut chars = stem.chars();
                chars.next().map(|f| format!("{}{}Kt", f.to_uppercase(), chars.as_str())).unwrap_or_default()
            };
            for s in symbols.iter().filter(|s| matches!(s.kind, SymbolKind::Method | SymbolKind::Function)) {
                let head = &lines.line_text(s.line)[..];
                let before = head.encode_utf16().take(s.col as usize).collect::<Vec<u16>>();
                if !marker.is_match(&String::from_utf16_lossy(&before)) {
                    continue;
                }
                let class = s.container.clone().unwrap_or_else(file_class);
                push(format!("jni:{}", jni_symbol(&package, &class, &s.name)), Role::Import, &s.name, at(s), None);
                push(format!("jnidyn:{}", s.name), Role::Import, &s.name, at(s), None);
            }
        }
        _ => {}
    }
    if matches!(lang, Lang::C | Lang::Cpp | Lang::ObjC) {
        for s in symbols.iter().filter(|s| s.kind == SymbolKind::Function && s.container.is_none()) {
            if let Some(rest) = s.name.strip_prefix("Java_") {
                let base = rest.split("__").next().unwrap_or(rest);
                push(format!("jni:Java_{base}"), Role::Export, &s.name, at(s), None);
                continue;
            }
            let line = lines.line_text(s.line);
            if line.trim_start().starts_with("static ") || line.contains(" static ") {
                continue;
            }
            let role = if s.decl { Role::Import } else { Role::Export };
            push(format!("c:{}", s.name), role, &s.name, at(s), None);
        }
        for c in REGISTER.captures_iter(text) {
            let (java, func) = (c.get(1).unwrap(), c.get(2).unwrap());
            push(format!("jnidyn:{}", java.as_str()), Role::Export, java.as_str(), java.start(), Some(func.as_str().to_owned()));
        }
        if lang == Lang::ObjC {
            for s in symbols.iter().filter(|s| s.kind == SymbolKind::Method) {
                let line = lines.line_text(s.line);
                let swift = match SWIFT_NAME.captures(line) {
                    Some(c) => c[1].to_owned(),
                    None => swift_method_name(&s.name),
                };
                if swift != s.name {
                    push(format!("objc:{swift}"), Role::Export, &s.name, at(s), None);
                }
            }
            for s in symbols.iter().filter(|s| s.kind.is_type()) {
                if let Some(c) = SWIFT_NAME.captures(lines.line_text(s.line)) {
                    push(format!("objc:{}", &c[1]), Role::Export, &s.name, at(s), None);
                }
            }
        }
    }
    match lang {
        Lang::Rust => {
            for c in NO_MANGLE.captures_iter(text) {
                let after = c.get(0).unwrap().end();
                if let Some(f) = FN_NAME.captures(&text[after..]).and_then(|f| f.get(1)).filter(|f| f.start() < 400) {
                    let exported = c.get(2).map_or(f.as_str(), |n| n.as_str());
                    push(format!("c:{exported}"), Role::Export, f.as_str(), after + f.start(), None);
                }
            }
            // Functions declared in `extern "C"` blocks.
            for s in symbols.iter().filter(|s| s.kind == SymbolKind::Function && s.decl && s.container.is_none()) {
                push(format!("c:{}", s.name), Role::Import, &s.name, at(s), None);
            }
        }
        Lang::Swift => {
            for c in CDECL.captures_iter(text) {
                let after = c.get(0).unwrap().end();
                if let Some(f) = FUNC_NAME.captures(&text[after..]).and_then(|f| f.get(1)).filter(|f| f.start() < 400) {
                    push(format!("c:{}", &c[1]), Role::Export, f.as_str(), after + f.start(), None);
                }
            }
            for c in OBJC_ATTR.captures_iter(text) {
                let after = c.get(0).unwrap().end();
                let Some(decl) = SWIFT_DECL.captures(&text[after..]).filter(|d| d.get(0).unwrap().start() < 200) else { continue };
                let name = decl.get(1).unwrap();
                let (site, offset) = if name.as_str().is_empty() {
                    ("init", after + decl.get(0).unwrap().start())
                } else {
                    (name.as_str(), after + name.start())
                };
                let exposed = c.get(1).map(|n| n.as_str().split(':').next().unwrap_or_default().to_owned()).unwrap_or_else(|| site.to_owned());
                push(format!("objc:{exposed}"), Role::Export, site, offset, None);
            }
        }
        Lang::Go => {
            for c in GO_EXPORT.captures_iter(text) {
                let after = c.get(0).unwrap().end();
                if let Some(f) = FUNC_NAME.captures(&text[after..]).and_then(|f| f.get(1)).filter(|f| f.start() < 200) {
                    push(format!("c:{}", &c[1]), Role::Export, f.as_str(), after + f.start(), None);
                }
            }
            if text.contains("import \"C\"") {
                for c in C_CALL.captures_iter(text) {
                    let m = c.get(1).unwrap();
                    push(format!("c:{}", m.as_str()), Role::Import, m.as_str(), m.start(), None);
                }
            }
        }
        Lang::V => {
            for c in V_EXPORT.captures_iter(text) {
                let after = c.get(0).unwrap().end();
                if let Some(f) = FN_NAME.captures(&text[after..]).and_then(|f| f.get(1)).filter(|f| f.start() < 200) {
                    push(format!("c:{}", &c[1]), Role::Export, f.as_str(), after + f.start(), None);
                }
            }
            for c in C_CALL.captures_iter(text) {
                let m = c.get(1).unwrap();
                push(format!("c:{}", m.as_str()), Role::Import, m.as_str(), m.start(), None);
            }
        }
        Lang::Dart => {
            for c in DART_LOOKUP.captures_iter(text) {
                let m = c.get(1).unwrap();
                push(format!("c:{}", m.as_str()), Role::Import, m.as_str(), m.start(), None);
            }
            for c in DART_NATIVE.captures_iter(text) {
                let after = c.get(0).unwrap().end();
                let function = DART_EXTERNAL_FN.captures(&text[after..]).and_then(|f| f.get(1)).filter(|f| f.start() < 300);
                match (c.get(1), function) {
                    (Some(symbol), Some(f)) => {
                        push(format!("c:{}", symbol.as_str()), Role::Import, symbol.as_str(), symbol.start(), None);
                        push(format!("c:{}", symbol.as_str()), Role::Import, f.as_str(), after + f.start(), None);
                    }
                    (Some(symbol), None) => push(format!("c:{}", symbol.as_str()), Role::Import, symbol.as_str(), symbol.start(), None),
                    (None, Some(f)) => push(format!("c:{}", f.as_str()), Role::Import, f.as_str(), after + f.start(), None),
                    (None, None) => {}
                }
            }
        }
        _ => {}
    }
    out.sort_by_key(|b| (b.line, b.col));
    out.dedup();
    out
}

/// The name Swift imports an Objective-C method's first selector piece
/// as: `initWithName` → `init`, `fetchDataWithURL` → `fetchData`.
pub fn swift_method_name(first_piece: &str) -> String {
    for marker in ["With", "For", "From", "Using"] {
        if let Some(i) = first_piece.find(marker) {
            let after = &first_piece[i + marker.len()..];
            if i > 0 && after.chars().next().is_some_and(|c| c.is_uppercase()) {
                return first_piece[..i].to_owned();
            }
        }
    }
    first_piece.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::symbols;

    fn items(lang: Lang, path: &str, src: &str) -> Vec<(String, Role, String, u32)> {
        let syms = symbols::extract(lang, src);
        extract(lang, path, src, &syms).into_iter().map(|b| (b.key, b.role, b.name, b.line)).collect()
    }

    #[test]
    fn mangles_like_jni() {
        assert_eq!(jni_symbol("com.example.app", "Native", "add"), "Java_com_example_app_Native_add");
        assert_eq!(jni_symbol("a.b_c", "Outer.Inner", "do_it"), "Java_a_b_1c_Outer_00024Inner_do_1it");
    }

    #[test]
    fn jni_both_sides() {
        let java = items(Lang::Java, "src/com/ex/Native.java", "package com.ex;\npublic class Native {\n  public static native int add(int a, int b);\n  void plain() {}\n}\n");
        assert!(java.contains(&("jni:Java_com_ex_Native_add".into(), Role::Import, "add".into(), 2)));
        let kotlin = items(Lang::Kotlin, "app/src/main/kotlin/com/ex/Bridge.kt", "package com.ex\nclass Lib {\n  external fun hash(s: String): Long\n}\nexternal fun topLevel(): Int\n");
        assert!(kotlin.iter().any(|i| i.0 == "jni:Java_com_ex_Lib_hash"));
        assert!(kotlin.iter().any(|i| i.0 == "jni:Java_com_ex_BridgeKt_topLevel"));
        let c = items(Lang::C, "jni/native.c", "#include <jni.h>\nJNIEXPORT jint JNICALL Java_com_ex_Native_add(JNIEnv *env, jclass c, jint a, jint b) { return a + b; }\nstatic jlong hash(JNIEnv* e, jobject o, jstring s) { return 0; }\nstatic JNINativeMethod methods[] = {\n  {\"hash\", \"(Ljava/lang/String;)J\", (void *) hash},\n};\n");
        assert!(c.contains(&("jni:Java_com_ex_Native_add".into(), Role::Export, "Java_com_ex_Native_add".into(), 1)));
        assert!(c.contains(&("jnidyn:hash".into(), Role::Export, "hash".into(), 4)));
    }

    #[test]
    fn c_abi_sides() {
        let rust = items(Lang::Rust, "src/lib.rs", "#[no_mangle]\npub extern \"C\" fn rust_add(a: i32, b: i32) -> i32 { a + b }\n#[unsafe(export_name = \"renamed\")]\npub extern \"C\" fn inner() {}\nextern \"C\" {\n    fn c_helper(x: i32) -> i32;\n}\n");
        assert_eq!(
            rust,
            [("c:rust_add".into(), Role::Export, "rust_add".into(), 1), ("c:renamed".into(), Role::Export, "inner".into(), 3), ("c:c_helper".into(), Role::Import, "c_helper".into(), 5)]
        );
        let c = items(Lang::C, "include/api.h", "int rust_add(int a, int b);\nint c_helper(int x) { return x; }\nstatic int hidden(void) { return 0; }\n");
        assert_eq!(c, [("c:rust_add".into(), Role::Import, "rust_add".into(), 0), ("c:c_helper".into(), Role::Export, "c_helper".into(), 1)]);
        let dart = items(Lang::Dart, "lib/ffi.dart", "final add = lib.lookupFunction<NativeAdd, DartAdd>('rust_add');\n@Native<Int32 Function(Int32)>(symbol: 'c_helper')\nexternal int helper(int x);\n@Native<Void Function()>()\nexternal void c_init();\n");
        assert_eq!(
            dart,
            [
                ("c:rust_add".into(), Role::Import, "rust_add".into(), 0),
                ("c:c_helper".into(), Role::Import, "c_helper".into(), 1),
                ("c:c_helper".into(), Role::Import, "helper".into(), 2),
                ("c:c_init".into(), Role::Import, "c_init".into(), 4)
            ]
        );
        let go = items(Lang::Go, "main.go", "package main\n// #include \"api.h\"\nimport \"C\"\n//export GoCallback\nfunc GoCallback() {}\nfunc main() { C.c_helper(1) }\n");
        assert_eq!(go, [("c:GoCallback".into(), Role::Export, "GoCallback".into(), 4), ("c:c_helper".into(), Role::Import, "c_helper".into(), 5)]);
        let swift = items(Lang::Swift, "Sources/Api.swift", "@_cdecl(\"swift_sum\")\npublic func sum(_ a: Int32) -> Int32 { a }\n@objc(GGFoo) class Foo: NSObject {\n  @objc func refresh() {}\n}\n");
        assert_eq!(
            swift,
            [("c:swift_sum".into(), Role::Export, "sum".into(), 1), ("objc:GGFoo".into(), Role::Export, "Foo".into(), 2), ("objc:refresh".into(), Role::Export, "refresh".into(), 3)]
        );
        let v = items(Lang::V, "main.v", "fn C.puts(s &char) int\n@[export: 'v_add']\npub fn add(a int, b int) int { return C.abs(a) }\n");
        assert_eq!(
            v,
            [("c:puts".into(), Role::Import, "puts".into(), 0), ("c:v_add".into(), Role::Export, "add".into(), 2), ("c:abs".into(), Role::Import, "abs".into(), 2)]
        );
    }

    #[test]
    fn objc_swift_names() {
        assert_eq!(swift_method_name("initWithName"), "init");
        assert_eq!(swift_method_name("fetchDataWithURL"), "fetchData");
        assert_eq!(swift_method_name("withdraw"), "withdraw");
        let objc = items(Lang::ObjC, "Foo.h", "@interface Foo : NSObject\n- (instancetype)initWithName:(NSString *)name;\n- (void)loadDataFromURL:(NSURL *)url NS_SWIFT_NAME(load(from:));\n- (void)run;\n@end\n");
        assert_eq!(objc, [("objc:init".into(), Role::Export, "initWithName".into(), 1), ("objc:load".into(), Role::Export, "loadDataFromURL".into(), 2)]);
    }
}
