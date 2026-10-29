//! Libraries without sources, as IntelliJ shows them ("Decompiled .class
//! file"): a class file read for its name, supertypes and public members,
//! and written back as a stub source whose bodies are
//! `{ /* compiled code */ }`, so they are indexed and navigated like any
//! source. A class compiled from Kotlin (it carries `kotlin.Metadata`)
//! becomes Kotlin: a file facade (`BoxKt`) its top-level functions, `getX()`
//! a property, an `INSTANCE` an object, `$Companion` a companion object;
//! any other Java. Parameter names aren't kept in class files: `p0`, `p1`…,
//! except in Kotlin's metadata, which writes the declarations as declared
//! when it can be read (`kotlin_metadata`).

use std::collections::BTreeMap;

use super::kotlin_metadata::{self, Metadata};

const ACC_PUBLIC: u16 = 0x0001;
const ACC_PRIVATE: u16 = 0x0002;
const ACC_PROTECTED: u16 = 0x0004;
const ACC_STATIC: u16 = 0x0008;
const ACC_FINAL: u16 = 0x0010;
const ACC_BRIDGE: u16 = 0x0040;
const ACC_VARARGS: u16 = 0x0080;
const ACC_INTERFACE: u16 = 0x0200;
const ACC_ABSTRACT: u16 = 0x0400;
const ACC_SYNTHETIC: u16 = 0x1000;
const ACC_ANNOTATION: u16 = 0x2000;
const ACC_ENUM: u16 = 0x4000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    pub descriptor: String,
    pub access: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassInfo {
    /// `kotlin/text/Regex$Companion`.
    pub name: String,
    pub access: u16,
    pub super_name: Option<String>,
    pub interfaces: Vec<String>,
    pub fields: Vec<Member>,
    pub methods: Vec<Member>,
    /// Compiled from Kotlin.
    pub kotlin: bool,
    /// Its `kotlin.Metadata`.
    pub metadata: Option<Metadata>,
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let out = self.data.get(self.at..self.at + n)?;
        self.at += n;
        Some(out)
    }
    fn u8(&mut self) -> Option<u8> {
        self.bytes(1).map(|b| b[0])
    }
    fn u16(&mut self) -> Option<u16> {
        self.bytes(2).map(|b| u16::from_be_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Option<u32> {
        self.bytes(4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
}

/// Reads a class file; none when it isn't one.
pub fn parse(data: &[u8]) -> Option<ClassInfo> {
    let mut r = Reader { data, at: 0 };
    if r.u32()? != 0xCAFE_BABE {
        return None;
    }
    r.bytes(4)?;
    // The constant pool: its UTF-8 entries and class references.
    let count = r.u16()? as usize;
    let mut utf8: Vec<Option<String>> = vec![None; count];
    let mut ints: Vec<Option<i32>> = vec![None; count];
    let mut class_name: Vec<Option<u16>> = vec![None; count];
    let mut i = 1;
    while i < count {
        match r.u8()? {
            1 => {
                let len = r.u16()? as usize;
                utf8[i] = Some(modified_utf8(r.bytes(len)?));
            }
            7 => class_name[i] = Some(r.u16()?),
            8 | 16 | 19 | 20 => {
                r.bytes(2)?;
            }
            15 => {
                r.bytes(3)?;
            }
            3 => ints[i] = Some(r.u32()? as i32),
            4 | 9 | 10 | 11 | 12 | 17 | 18 => {
                r.bytes(4)?;
            }
            5 | 6 => {
                r.bytes(8)?;
                i += 1;
            }
            _ => return None,
        }
        i += 1;
    }
    let text = |ix: u16| utf8.get(ix as usize).cloned().flatten();
    let class = |ix: u16| class_name.get(ix as usize).copied().flatten().and_then(text);
    let access = r.u16()?;
    let name = class(r.u16()?)?;
    let super_name = class(r.u16()?);
    let interfaces = (0..r.u16()?).filter_map(|_| r.u16().and_then(class)).collect();
    let members = |r: &mut Reader| -> Option<Vec<Member>> {
        let n = r.u16()?;
        let mut out = Vec::new();
        for _ in 0..n {
            let access = r.u16()?;
            let name = text(r.u16()?)?;
            let descriptor = text(r.u16()?)?;
            for _ in 0..r.u16()? {
                r.u16()?;
                let len = r.u32()? as usize;
                r.bytes(len)?;
            }
            out.push(Member { name, descriptor, access });
        }
        Some(out)
    };
    let fields = members(&mut r)?;
    let methods = members(&mut r)?;
    let kotlin = utf8.iter().flatten().any(|s| s == "Lkotlin/Metadata;");
    // The class's annotations: `kotlin.Metadata`.
    let mut metadata = None;
    for _ in 0..r.u16().unwrap_or(0) {
        let (Some(attr), Some(len)) = (r.u16(), r.u32()) else { break };
        let Some(body) = r.bytes(len as usize) else { break };
        if kotlin && text(attr).as_deref() == Some("RuntimeVisibleAnnotations") {
            metadata = kotlin_annotation(&mut Reader { data: body, at: 0 }, &text, &|ix| ints.get(ix as usize).copied().flatten());
        }
    }
    Some(ClassInfo { name, access, super_name, interfaces, fields, methods, kotlin, metadata })
}

/// The class file's "modified UTF-8": `\0` as two bytes, supplementary
/// characters as surrogate pairs.
fn modified_utf8(bytes: &[u8]) -> String {
    let mut units: Vec<u16> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        let (unit, n) = if b & 0x80 == 0 {
            (u16::from(b), 1)
        } else if b & 0xE0 == 0xC0 && i + 1 < bytes.len() {
            ((u16::from(b & 0x1F) << 6) | u16::from(bytes[i + 1] & 0x3F), 2)
        } else if b & 0xF0 == 0xE0 && i + 2 < bytes.len() {
            ((u16::from(b & 0x0F) << 12) | (u16::from(bytes[i + 1] & 0x3F) << 6) | u16::from(bytes[i + 2] & 0x3F), 3)
        } else {
            (0xFFFD, 1)
        };
        units.push(unit);
        i += n;
    }
    String::from_utf16_lossy(&units)
}

/// `@kotlin.Metadata(k = …, d1 = […], d2 = […])` among the annotations.
fn kotlin_annotation(r: &mut Reader, text: &dyn Fn(u16) -> Option<String>, int: &dyn Fn(u16) -> Option<i32>) -> Option<Metadata> {
    // Skips an element value, or reads the strings / int it holds.
    fn value(r: &mut Reader, text: &dyn Fn(u16) -> Option<String>, int: &dyn Fn(u16) -> Option<i32>, strings: &mut Vec<String>, number: &mut Option<i32>) -> Option<()> {
        match r.u8()? {
            b's' => strings.extend(text(r.u16()?)),
            b'I' => *number = int(r.u16()?),
            b'B' | b'C' | b'D' | b'F' | b'J' | b'S' | b'Z' | b'c' => {
                r.u16()?;
            }
            b'e' => {
                r.bytes(4)?;
            }
            b'@' => {
                r.u16()?;
                for _ in 0..r.u16()? {
                    r.u16()?;
                    value(r, text, int, &mut Vec::new(), &mut None)?;
                }
            }
            b'[' => {
                for _ in 0..r.u16()? {
                    value(r, text, int, strings, number)?;
                }
            }
            _ => return None,
        }
        Some(())
    }
    for _ in 0..r.u16()? {
        let ty = text(r.u16()?)?;
        let mut meta = Metadata::default();
        for _ in 0..r.u16()? {
            let name = text(r.u16()?)?;
            let (mut strings, mut number) = (Vec::new(), None);
            value(r, text, int, &mut strings, &mut number)?;
            match name.as_str() {
                "k" => meta.kind = number.unwrap_or(0),
                "d1" => meta.d1 = strings,
                "d2" => meta.d2 = strings,
                _ => {}
            }
        }
        if ty == "Lkotlin/Metadata;" {
            return Some(meta);
        }
    }
    None
}

// --- Types -----------------------------------------------------------------

/// `Ljava/util/List;` → its simple name; `$` nesting → `.`.
fn simple(internal: &str) -> String {
    internal.rsplit('/').next().unwrap_or(internal).replace('$', ".")
}

fn kotlin_object(internal: &str) -> String {
    match internal {
        "java/lang/Object" => "Any".into(),
        "java/lang/String" => "String".into(),
        "java/lang/CharSequence" => "CharSequence".into(),
        "java/lang/Throwable" => "Throwable".into(),
        "java/lang/Integer" => "Int".into(),
        "java/lang/Boolean" => "Boolean".into(),
        "java/lang/Long" => "Long".into(),
        "java/lang/Double" => "Double".into(),
        "java/lang/Float" => "Float".into(),
        "java/lang/Character" => "Char".into(),
        "java/util/List" => "List<*>".into(),
        "java/util/Map" => "Map<*, *>".into(),
        "java/util/Set" => "Set<*>".into(),
        "java/util/Collection" => "Collection<*>".into(),
        "java/lang/Iterable" => "Iterable<*>".into(),
        "java/lang/Comparable" => "Comparable<*>".into(),
        s if s.starts_with("kotlin/jvm/functions/Function") => format!("{}<*>", simple(s)),
        s => simple(s),
    }
}

/// One type of a descriptor, and the rest.
fn split_type(d: &str) -> Option<(&str, &str)> {
    let end = match d.as_bytes().first()? {
        b'L' => d.find(';')? + 1,
        b'[' => d.len() - split_type(&d[1..])?.1.len(),
        _ => 1,
    };
    Some((&d[..end], &d[end..]))
}

fn type_name(d: &str, kotlin: bool) -> String {
    let prim = |c: u8| -> Option<(&str, &str)> {
        Some(match c {
            b'B' => ("Byte", "byte"),
            b'C' => ("Char", "char"),
            b'D' => ("Double", "double"),
            b'F' => ("Float", "float"),
            b'I' => ("Int", "int"),
            b'J' => ("Long", "long"),
            b'S' => ("Short", "short"),
            b'Z' => ("Boolean", "boolean"),
            b'V' => ("Unit", "void"),
            _ => return None,
        })
    };
    match d.as_bytes().first() {
        Some(b'L') => {
            let internal = &d[1..d.len() - 1];
            if kotlin { kotlin_object(internal) } else { simple(internal) }
        }
        Some(b'[') => {
            let inner = &d[1..];
            match (kotlin, prim(inner.as_bytes()[0]).filter(|_| inner.len() == 1)) {
                (true, Some((k, _))) => format!("{k}Array"),
                (true, None) => format!("Array<{}>", type_name(inner, true)),
                (false, _) => format!("{}[]", type_name(inner, false)),
            }
        }
        Some(&c) => prim(c).map(|(k, j)| if kotlin { k } else { j }.to_owned()).unwrap_or_default(),
        None => String::new(),
    }
}

/// A method descriptor's parameter types and return type.
fn signature(descriptor: &str) -> Option<(Vec<&str>, &str)> {
    let inner = descriptor.strip_prefix('(')?;
    let close = inner.find(')')?;
    let (mut params, ret) = (&inner[..close], &inner[close + 1..]);
    let mut out = Vec::new();
    while !params.is_empty() {
        let (t, rest) = split_type(params)?;
        out.push(t);
        params = rest;
    }
    Some((out, ret))
}

// --- Stubs -----------------------------------------------------------------

fn visible(access: u16) -> bool {
    access & (ACC_PUBLIC | ACC_PROTECTED) != 0 && access & (ACC_SYNTHETIC | ACC_BRIDGE) == 0
}

/// Kotlin's compiler-made members: `$default` bridges, `access$…`,
/// `component1()`, `copy$default`.
fn generated(name: &str) -> bool {
    name.contains('$') || name == "<clinit>"
}

fn modifier(access: u16, kotlin: bool) -> &'static str {
    match (access & ACC_PROTECTED != 0, kotlin) {
        (true, _) => "protected ",
        (false, true) => "",
        (false, false) => "public ",
    }
}

const BODY: &str = "{ /* compiled code */ }";

fn params(types: &[&str], kotlin: bool, varargs: bool) -> String {
    types
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let last = varargs && i + 1 == types.len() && t.starts_with('[');
            match (kotlin, last) {
                (true, true) => format!("vararg p{i}: {}", type_name(&t[1..], true)),
                (true, false) => format!("p{i}: {}", type_name(t, true)),
                (false, true) => format!("{}... p{i}", type_name(&t[1..], false)),
                (false, false) => format!("{} p{i}", type_name(t, false)),
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// `getColorScheme` → `colorScheme`, `isEmpty` → `isEmpty`.
fn property_of(method: &str) -> Option<String> {
    if method.starts_with("is") && method.len() > 2 && method.as_bytes()[2].is_ascii_uppercase() {
        return Some(method.to_owned());
    }
    let rest = method.strip_prefix("get").filter(|r| r.as_bytes().first().is_some_and(u8::is_ascii_uppercase))?;
    let mut chars = rest.chars();
    Some(chars.next().map(|c| c.to_lowercase().chain(chars).collect()).unwrap_or_default())
}

fn render_kotlin(class: &ClassInfo, all: &BTreeMap<String, ClassInfo>, indent: &str, out: &mut String) {
    let simple_name = class.name.rsplit(['/', '$']).next().unwrap_or(&class.name).to_owned();
    let own = format!("L{};", class.name);
    let facade = simple_name.ends_with("Kt") && class.access & ACC_INTERFACE == 0 && class.methods.iter().filter(|m| m.name != "<init>").all(|m| m.access & ACC_STATIC != 0);
    let is_object = class.fields.iter().any(|f| f.name == "INSTANCE" && f.descriptor == own && f.access & ACC_STATIC != 0);
    let inner = format!("{indent}    ");
    let (body_indent, statics_only) = if facade { (indent.to_owned(), true) } else { (inner.clone(), false) };
    if !facade {
        let keyword = if class.access & ACC_ANNOTATION != 0 {
            "annotation class"
        } else if class.access & ACC_INTERFACE != 0 {
            "interface"
        } else if class.access & ACC_ENUM != 0 {
            "enum class"
        } else if class.name.ends_with("$Companion") {
            "companion object"
        } else if is_object {
            "object"
        } else if class.access & ACC_ABSTRACT != 0 {
            "abstract class"
        } else if class.access & ACC_FINAL == 0 {
            "open class"
        } else {
            "class"
        };
        let mut supers: Vec<String> = Vec::new();
        if let Some(s) = class.super_name.as_deref().filter(|s| !matches!(*s, "java/lang/Object" | "java/lang/Enum" | "java/lang/Record")) {
            supers.push(format!("{}()", kotlin_object(s)));
        }
        supers.extend(class.interfaces.iter().filter(|i| *i != "java/lang/annotation/Annotation").map(|i| kotlin_object(i)));
        let name = if keyword == "companion object" { String::new() } else { format!(" {simple_name}") };
        let colon = if supers.is_empty() { String::new() } else { format!(" : {}", supers.join(", ")) };
        out.push_str(&format!("{indent}{keyword}{name}{colon} {{\n"));
    }
    // Enum constants first.
    if class.access & ACC_ENUM != 0 {
        let constants: Vec<&str> = class.fields.iter().filter(|f| f.access & ACC_ENUM != 0).map(|f| f.name.as_str()).collect();
        if !constants.is_empty() {
            out.push_str(&format!("{body_indent}{};\n", constants.join(", ")));
        }
    }
    // Properties: fields and getters.
    let mut properties: Vec<String> = Vec::new();
    let setters: Vec<&str> = class.methods.iter().filter(|m| m.name.starts_with("set")).map(|m| m.name.as_str()).collect();
    for m in class.methods.iter().filter(|m| visible(m.access) && !generated(&m.name) && (!statics_only || m.access & ACC_STATIC != 0)) {
        let Some((ps, ret)) = signature(&m.descriptor) else { continue };
        if m.name == "<init>" {
            if !facade && !is_object && class.access & (ACC_INTERFACE | ACC_ENUM) == 0 {
                out.push_str(&format!("{body_indent}{}constructor({}) {BODY}\n", modifier(m.access, true), params(&ps, true, m.access & ACC_VARARGS != 0)));
            }
            continue;
        }
        if let Some(prop) = property_of(&m.name).filter(|_| ps.is_empty() && ret != "V") {
            let mutable = setters.iter().any(|s| s[3..] == m.name[if m.name.starts_with("is") { 2 } else { 3 }..]);
            properties.push(prop.clone());
            out.push_str(&format!("{body_indent}{}{} {prop}: {}\n", modifier(m.access, true), if mutable { "var" } else { "val" }, type_name(ret, true)));
            continue;
        }
        if m.name.starts_with("set") && ps.len() == 1 && property_of(&format!("get{}", &m.name[3..])).is_some_and(|p| properties.contains(&p)) {
            continue;
        }
        let abstract_ = if m.access & ACC_ABSTRACT != 0 && class.access & ACC_INTERFACE == 0 { "abstract " } else { "" };
        let body = if m.access & ACC_ABSTRACT != 0 { String::new() } else { format!(" {BODY}") };
        let returns = if ret == "V" { String::new() } else { format!(": {}", type_name(ret, true)) };
        out.push_str(&format!("{body_indent}{}{abstract_}fun {}({}){returns}{body}\n", modifier(m.access, true), m.name, params(&ps, true, m.access & ACC_VARARGS != 0)));
    }
    for f in class.fields.iter().filter(|f| visible(f.access) && f.access & ACC_ENUM == 0 && f.name != "INSTANCE" && f.name != "Companion" && !generated(&f.name)) {
        if properties.contains(&f.name) {
            continue;
        }
        let kind = if f.access & ACC_FINAL != 0 { if f.access & ACC_STATIC != 0 { "const val" } else { "val" } } else { "var" };
        out.push_str(&format!("{body_indent}{}{kind} {}: {}\n", modifier(f.access, true), f.name, type_name(&f.descriptor, true)));
    }
    for nested in nested_of(class, all) {
        if !render_metadata(nested, all, &body_indent, out) {
            render_kotlin(nested, all, &body_indent, out);
        }
    }
    if !facade {
        out.push_str(&format!("{indent}}}\n"));
    }
}

fn render_java(class: &ClassInfo, all: &BTreeMap<String, ClassInfo>, indent: &str, out: &mut String) {
    let simple_name = class.name.rsplit(['/', '$']).next().unwrap_or(&class.name).to_owned();
    let keyword = if class.access & ACC_ANNOTATION != 0 {
        "@interface"
    } else if class.access & ACC_INTERFACE != 0 {
        "interface"
    } else if class.access & ACC_ENUM != 0 {
        "enum"
    } else {
        "class"
    };
    let mods = format!(
        "public {}{}",
        if class.access & ACC_ABSTRACT != 0 && class.access & ACC_INTERFACE == 0 { "abstract " } else { "" },
        if class.access & ACC_FINAL != 0 && class.access & ACC_ENUM == 0 { "final " } else { "" }
    );
    let extends = class.super_name.as_deref().filter(|s| !matches!(*s, "java/lang/Object" | "java/lang/Enum" | "java/lang/Record")).map(|s| format!(" extends {}", simple(s))).unwrap_or_default();
    let ifaces: Vec<String> = class.interfaces.iter().filter(|i| *i != "java/lang/annotation/Annotation").map(|i| simple(i)).collect();
    let implements = if ifaces.is_empty() { String::new() } else { format!(" {} {}", if class.access & ACC_INTERFACE != 0 { "extends" } else { "implements" }, ifaces.join(", ")) };
    out.push_str(&format!("{indent}{mods}{keyword} {simple_name}{extends}{implements} {{\n"));
    let inner = format!("{indent}    ");
    if class.access & ACC_ENUM != 0 {
        let constants: Vec<&str> = class.fields.iter().filter(|f| f.access & ACC_ENUM != 0).map(|f| f.name.as_str()).collect();
        out.push_str(&format!("{inner}{};\n", constants.join(", ")));
    }
    for f in class.fields.iter().filter(|f| visible(f.access) && f.access & ACC_ENUM == 0) {
        let stat = if f.access & ACC_STATIC != 0 { "static " } else { "" };
        let fin = if f.access & ACC_FINAL != 0 { "final " } else { "" };
        out.push_str(&format!("{inner}{}{stat}{fin}{} {};\n", modifier(f.access, false), type_name(&f.descriptor, false), f.name));
    }
    for m in class.methods.iter().filter(|m| visible(m.access) && m.name != "<clinit>" && !m.name.contains("lambda$")) {
        let Some((ps, ret)) = signature(&m.descriptor) else { continue };
        let ps = params(&ps, false, m.access & ACC_VARARGS != 0);
        if m.name == "<init>" {
            out.push_str(&format!("{inner}{}{simple_name}({ps}) {BODY}\n", modifier(m.access, false)));
            continue;
        }
        let stat = if m.access & ACC_STATIC != 0 { "static " } else { "" };
        let body = if m.access & ACC_ABSTRACT != 0 { ";".to_owned() } else { format!(" {BODY}") };
        out.push_str(&format!("{inner}{}{stat}{} {}({ps}){body}\n", modifier(m.access, false), type_name(ret, false), m.name));
    }
    for nested in nested_of(class, all) {
        render_java(nested, all, &inner, out);
    }
    out.push_str(&format!("{indent}}}\n"));
}

/// A Kotlin class or file from its `kotlin.Metadata`, as declared: real
/// parameter names, nullable and generic types, `@InlineOnly` functions.
/// False (nothing written) when it has none or it can't be read.
fn render_metadata(class: &ClassInfo, all: &BTreeMap<String, ClassInfo>, indent: &str, out: &mut String) -> bool {
    let Some(meta) = &class.metadata else { return false };
    let package = class.name.rsplit_once('/').map_or("", |(p, _)| p);
    let lines = |meta: &Metadata, indent: &str, out: &mut String| -> bool {
        let bytes = kotlin_metadata::bytes(meta);
        let Some(decoded) = kotlin_metadata::decode(meta, &bytes) else { return false };
        for line in decoded.members() {
            out.push_str(&format!("{indent}{line}\n"));
        }
        true
    };
    match meta.kind {
        // A multi-file facade (`CollectionsKt`): its parts' declarations.
        4 => {
            let mut any = false;
            for part in kotlin_metadata::parts(meta, package) {
                if let Some(meta) = all.get(&part).and_then(|c| c.metadata.as_ref()) {
                    any |= lines(meta, indent, out);
                }
            }
            any
        }
        2 | 5 => lines(meta, indent, out),
        1 => {
            let bytes = kotlin_metadata::bytes(meta);
            let Some(decoded) = kotlin_metadata::decode(meta, &bytes) else { return false };
            let simple_name = class.name.rsplit(['/', '$']).next().unwrap_or(&class.name);
            out.push_str(&format!("{indent}{} {{\n", decoded.header(simple_name, class.super_name.as_deref())));
            let inner = format!("{indent}    ");
            for line in decoded.members() {
                out.push_str(&format!("{inner}{line}\n"));
            }
            // Compiler-made nested classes (`DefaultImpls`, `WhenMappings`) aren't shown.
            for nested in nested_of(class, all).into_iter().filter(|n| n.metadata.as_ref().is_none_or(|m| m.kind == 1)) {
                if !render_metadata(nested, all, &inner, out) {
                    render_kotlin(nested, all, &inner, out);
                }
            }
            out.push_str(&format!("{indent}}}\n"));
            true
        }
        _ => false,
    }
}

/// A class's named nested classes (`Outer$Inner`, not `Outer$1`).
fn nested_of<'a>(class: &ClassInfo, all: &'a BTreeMap<String, ClassInfo>) -> Vec<&'a ClassInfo> {
    let prefix = format!("{}$", class.name);
    all.range(prefix.clone()..)
        .take_while(|(k, _)| k.starts_with(&prefix))
        .filter(|(k, c)| {
            let rest = &k[prefix.len()..];
            !rest.contains('$') && !rest.starts_with(|ch: char| ch.is_ascii_digit()) && visible(c.access | if c.access & ACC_PRIVATE == 0 { ACC_PUBLIC } else { 0 })
        })
        .map(|(_, c)| c)
        .collect()
}

/// A top-level class's stub: its path in the library (`kotlin/io/ConsoleKt.kt`)
/// and text; `all` holds its nested classes. None for a nested, anonymous
/// or non-public class.
pub fn stub(class: &ClassInfo, all: &BTreeMap<String, ClassInfo>) -> Option<(String, String)> {
    // A multi-file class's part is shown in its facade.
    if class.metadata.as_ref().is_some_and(|m| m.kind == 5) {
        return None;
    }
    if class.name.contains('$') || class.access & ACC_PUBLIC == 0 || class.access & ACC_SYNTHETIC != 0 || class.name.ends_with("package-info") || class.name == "module-info" {
        return None;
    }
    let package = class.name.rsplit_once('/').map(|(p, _)| p.replace('/', ".")).unwrap_or_default();
    let mut text = String::from("// Decompiled .class file: no sources were found for this library.\n");
    if !package.is_empty() {
        text.push_str(&format!("package {package}{}\n\n", if class.kotlin { "" } else { ";" }));
    }
    let ext = if class.kotlin {
        if !render_metadata(class, all, "", &mut text) {
            render_kotlin(class, all, "", &mut text);
        }
        "kt"
    } else {
        render_java(class, all, "", &mut text);
        "java"
    };
    Some((format!("{}.{ext}", class.name), text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(name: &str) -> ClassInfo {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/domain/testdata/class_file").join(name);
        parse(&std::fs::read(path).unwrap()).unwrap()
    }

    fn one(class: ClassInfo, nested: Vec<ClassInfo>) -> (String, String) {
        let mut all: BTreeMap<String, ClassInfo> = nested.into_iter().map(|c| (c.name.clone(), c)).collect();
        all.insert(class.name.clone(), class.clone());
        stub(&class, &all).unwrap()
    }

    #[test]
    fn kotlin_facades_become_top_level_functions() {
        let (path, text) = one(load("ConsoleKt.class"), vec![]);
        assert_eq!(path, "kotlin/io/ConsoleKt.kt");
        assert!(text.contains("package kotlin.io\n"), "{text}");
        // From the metadata: `println` (`@InlineOnly`, private in the class file), names, nullability.
        assert!(text.contains("\ninline fun println(message: Any?) { /* compiled code */ }"), "{text}");
        assert!(text.contains("\nfun readln(): String { /* compiled code */ }"), "{text}");
        assert!(text.contains("fun readLine(): String? { /* compiled code */ }"), "{text}");
        assert!(!text.contains("class ConsoleKt"), "{text}");
    }

    #[test]
    fn kotlin_classes_objects_and_properties() {
        let (_, pair) = one(load("Pair.class"), vec![]);
        assert!(pair.contains("data class Pair<out A, out B> : Serializable {"), "{pair}");
        assert!(pair.contains("    val first: A\n"), "{pair}");
        assert!(pair.contains("    constructor(first: A, second: B) { /* compiled code */ }"), "{pair}");
        assert!(pair.contains("    fun copy(first: A = COMPILED_CODE, second: B = COMPILED_CODE): Pair<A, B> { /* compiled code */ }"), "{pair}");
        let (_, unit) = one(load("Unit.class"), vec![]);
        assert!(unit.contains("object Unit {"), "{unit}");
        let regex = ClassInfo { name: "kotlin/text/Regex".into(), access: ACC_PUBLIC | ACC_FINAL, super_name: Some("java/lang/Object".into()), interfaces: vec![], fields: vec![], methods: vec![], kotlin: true, metadata: None };
        let (_, text) = one(regex, vec![load("Regex$Companion.class")]);
        assert!(text.contains("class Regex {\n    companion object {\n") && text.contains("        fun escape(literal: String): String { /* compiled code */ }"), "{text}");
    }

    #[test]
    fn kotlin_multi_file_facades_and_function_types() {
        let parts = vec![load("BuildersKt__BuildersKt.class"), load("BuildersKt__Builders_commonKt.class")];
        assert!(parts.iter().all(|p| stub(p, &BTreeMap::new()).is_none()));
        let (path, text) = one(load("BuildersKt.class"), parts);
        assert_eq!(path, "kotlinx/coroutines/BuildersKt.kt");
        assert!(text.contains("\nfun CoroutineScope.launch(context: CoroutineContext = COMPILED_CODE, start: CoroutineStart = COMPILED_CODE, block: suspend CoroutineScope.() -> Unit): Job { /* compiled code */ }"), "{text}");
        assert!(text.contains("\nsuspend fun <T> withContext(context: CoroutineContext, block: suspend CoroutineScope.() -> T): T { /* compiled code */ }"), "{text}");
        assert!(text.contains("fun <T> runBlocking("), "{text}");
    }

    #[test]
    fn java_classes() {
        let (path, text) = one(load("Calc.class"), vec![]);
        assert_eq!(path, "com/example/calc/Calc.java");
        assert!(text.contains("package com.example.calc;"), "{text}");
        assert!(text.contains("public class Calc {"), "{text}");
        assert!(text.contains("    public static int add(int p0, int p1) { /* compiled code */ }"), "{text}");
    }

    #[test]
    fn descriptors() {
        assert_eq!(signature("(I[Ljava/lang/String;[[J)V"), Some((vec!["I", "[Ljava/lang/String;", "[[J"], "V")));
        assert_eq!(type_name("[I", true), "IntArray");
        assert_eq!(type_name("[[J", false), "long[][]");
        assert_eq!(type_name("Lkotlin/jvm/functions/Function1;", true), "Function1<*>");
    }
}

