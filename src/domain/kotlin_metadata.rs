//! `kotlin.Metadata` of a class compiled from Kotlin, as IntelliJ reads it
//! for a decompiled stub: the declarations as written, with their parameter
//! names, nullable and generic types, default values, `suspend` / `inline` /
//! `operator`, extension receivers, and the `@InlineOnly` functions the class
//! file keeps private. `d1` is a protobuf message (the compiler's
//! `metadata.proto`) after a string table (`JvmProtoBuf.StringTableTypes`),
//! its strings in `d2`.

/// The annotation's values: `k` (1 class, 2 file facade, 4 multi-file
/// facade, 5 multi-file part), `d1`, `d2`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Metadata {
    pub kind: i32,
    pub d1: Vec<String>,
    pub d2: Vec<String>,
}

// --- Protobuf --------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum Val<'a> {
    Int(u64),
    Bytes(&'a [u8]),
}

/// A message's fields in order.
#[derive(Clone, Debug, Default)]
struct Msg<'a>(Vec<(u32, Val<'a>)>);

fn varint(data: &[u8], at: &mut usize) -> Option<u64> {
    let mut out = 0u64;
    for shift in (0..64).step_by(7) {
        let b = *data.get(*at)?;
        *at += 1;
        out |= u64::from(b & 0x7F) << shift;
        if b & 0x80 == 0 {
            return Some(out);
        }
    }
    None
}

impl<'a> Msg<'a> {
    fn parse(data: &'a [u8]) -> Option<Msg<'a>> {
        let mut at = 0;
        let mut out = Vec::new();
        while at < data.len() {
            let key = varint(data, &mut at)?;
            let (field, wire) = ((key >> 3) as u32, key & 7);
            let val = match wire {
                0 => Val::Int(varint(data, &mut at)?),
                1 => {
                    at += 8;
                    Val::Int(0)
                }
                2 => {
                    let len = varint(data, &mut at)? as usize;
                    let bytes = data.get(at..at.checked_add(len)?)?;
                    at += len;
                    Val::Bytes(bytes)
                }
                5 => {
                    at += 4;
                    Val::Int(0)
                }
                _ => return None,
            };
            out.push((field, val));
        }
        Some(Msg(out))
    }

    fn int(&self, field: u32) -> Option<u64> {
        self.0.iter().rev().find_map(|(f, v)| match v {
            Val::Int(i) if *f == field => Some(*i),
            _ => None,
        })
    }

    /// A repeated int, packed or not.
    fn ints(&self, field: u32) -> Vec<u64> {
        let mut out = Vec::new();
        for (f, v) in &self.0 {
            if *f != field {
                continue;
            }
            match v {
                Val::Int(i) => out.push(*i),
                Val::Bytes(b) => {
                    let mut at = 0;
                    while at < b.len() {
                        match varint(b, &mut at) {
                            Some(i) => out.push(i),
                            None => break,
                        }
                    }
                }
            }
        }
        out
    }

    fn msgs(&self, field: u32) -> Vec<Msg<'a>> {
        self.0
            .iter()
            .filter_map(|(f, v)| match v {
                Val::Bytes(b) if *f == field => Msg::parse(b),
                _ => None,
            })
            .collect()
    }

    fn msg(&self, field: u32) -> Option<Msg<'a>> {
        self.msgs(field).pop()
    }

    fn bytes(&self, field: u32) -> Option<&'a [u8]> {
        self.0.iter().rev().find_map(|(f, v)| match v {
            Val::Bytes(b) if *f == field => Some(*b),
            _ => None,
        })
    }
}

// --- Strings ---------------------------------------------------------------

/// `d1` as bytes: each char a byte after a `\0` marker (Kotlin 1.3+), else
/// the old 7-bit encoding.
fn decode_bytes(d1: &[String]) -> Vec<u8> {
    let mut strings: Vec<&str> = d1.iter().map(String::as_str).collect();
    if let Some(first) = strings.first().copied() {
        if let Some(rest) = first.strip_prefix('\0') {
            strings[0] = rest;
            return strings.iter().flat_map(|s| s.chars().map(|c| c as u32 as u8)).collect();
        }
        if let Some(rest) = first.strip_prefix('\u{FFFF}') {
            strings[0] = rest;
        }
    }
    let mut data: Vec<u8> = strings.iter().flat_map(|s| s.chars().map(|c| (c as u32 as u8).wrapping_add(0x7F) & 0x7F)).collect();
    data.push(0);
    let len = 7 * (data.len() - 1) / 8;
    let mut out = Vec::with_capacity(len);
    let (mut at, mut bit) = (0usize, 0u32);
    for _ in 0..len {
        let first = u32::from(data[at]) >> bit;
        at += 1;
        let second = (u32::from(data[at]) & ((1 << (bit + 1)) - 1)) << (7 - bit);
        out.push((first + second) as u8);
        if bit == 6 {
            at += 1;
            bit = 0;
        } else {
            bit += 1;
        }
    }
    out
}

const PREDEFINED: &[&str] = &[
    "kotlin/Any", "kotlin/Nothing", "kotlin/Unit", "kotlin/Throwable", "kotlin/Number", "kotlin/Byte", "kotlin/Double", "kotlin/Float", "kotlin/Int", "kotlin/Long",
    "kotlin/Short", "kotlin/Boolean", "kotlin/Char", "kotlin/CharSequence", "kotlin/String", "kotlin/Comparable", "kotlin/Enum", "kotlin/Array", "kotlin/ByteArray",
    "kotlin/DoubleArray", "kotlin/FloatArray", "kotlin/IntArray", "kotlin/LongArray", "kotlin/ShortArray", "kotlin/BooleanArray", "kotlin/CharArray", "kotlin/Cloneable",
    "kotlin/Annotation", "kotlin/collections/Iterable", "kotlin/collections/MutableIterable", "kotlin/collections/Collection", "kotlin/collections/MutableCollection",
    "kotlin/collections/List", "kotlin/collections/MutableList", "kotlin/collections/Set", "kotlin/collections/MutableSet", "kotlin/collections/Map",
    "kotlin/collections/MutableMap", "kotlin/collections/Map.Entry", "kotlin/collections/MutableMap.MutableEntry", "kotlin/collections/Iterator",
    "kotlin/collections/MutableIterator", "kotlin/collections/ListIterator", "kotlin/collections/MutableListIterator",
];

/// The string table (`JvmNameResolver`): `d2` with the records' predefined
/// strings, substrings, char replacements and class-id conversions.
fn string_table(types: &Msg, d2: &[String]) -> Vec<String> {
    let mut records: Vec<Msg> = Vec::new();
    for r in types.msgs(1) {
        let range = r.int(1).unwrap_or(1).min(1 << 16) as usize;
        records.extend(std::iter::repeat_n(r, range));
    }
    (0..records.len().max(d2.len()))
        .map(|i| {
            let plain = d2.get(i).cloned().unwrap_or_default();
            let Some(r) = records.get(i) else { return plain };
            let mut s = if let Some(b) = r.bytes(6) {
                String::from_utf8_lossy(b).into_owned()
            } else if let Some(p) = r.int(2) {
                PREDEFINED.get(p as usize).map_or(plain, |s| (*s).to_owned())
            } else {
                plain
            };
            let sub = r.ints(4);
            if sub.len() >= 2 {
                let chars: Vec<char> = s.chars().collect();
                let (b, e) = (sub[0] as usize, sub[1] as usize);
                if b <= e && e <= chars.len() {
                    s = chars[b..e].iter().collect();
                }
            }
            let rep = r.ints(5);
            if rep.len() >= 2 {
                if let (Some(from), Some(to)) = (char::from_u32(rep[0] as u32), char::from_u32(rep[1] as u32)) {
                    s = s.replace(from, &to.to_string());
                }
            }
            match r.int(3).unwrap_or(0) {
                1 => s = s.replace('$', "."),
                2 => {
                    if s.len() >= 2 {
                        s = s[1..s.len() - 1].to_owned();
                    }
                    s = s.replace('$', ".");
                }
                _ => {}
            }
            s
        })
        .collect()
}

// --- Declarations ----------------------------------------------------------

/// What a stub needs of a declaration list: functions, properties,
/// constructors, enum entries, as Kotlin source lines.
pub struct Decoded<'a> {
    names: Vec<String>,
    kind: i32,
    root: Msg<'a>,
    table: Vec<Msg<'a>>,
    first_nullable: Option<usize>,
}

/// The kinds of class (`Flags.CLASS_KIND`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClassKind {
    Class,
    Interface,
    EnumClass,
    AnnotationClass,
    Object,
    CompanionObject,
}

const PUBLIC: u64 = 3;
const PROTECTED: u64 = 2;

fn visibility(flags: u64) -> u64 {
    (flags >> 1) & 7
}

fn shown(flags: u64) -> bool {
    matches!(visibility(flags), PUBLIC | PROTECTED)
}

fn bit(flags: u64, n: u32) -> bool {
    flags >> n & 1 == 1
}

struct Scope<'s> {
    names: &'s [String],
    params: Vec<(u64, String)>,
    table: &'s [Msg<'s>],
    first_nullable: Option<usize>,
}

impl<'s> Scope<'s> {
    fn name(&self, ix: u64) -> String {
        self.names.get(ix as usize).cloned().unwrap_or_default()
    }

    /// A class name as written: `kotlin/collections/Map.Entry` → `Map.Entry`.
    fn class(&self, ix: u64) -> String {
        let full = self.name(ix);
        full.rsplit('/').next().unwrap_or(&full).trim_start_matches('.').to_owned()
    }

    fn with_params(&self, params: &[Msg<'s>]) -> Scope<'s> {
        let mut out = Scope { names: self.names, params: self.params.clone(), table: self.table, first_nullable: self.first_nullable };
        out.params.extend(params.iter().map(|p| (p.int(1).unwrap_or(0), self.name(p.int(2).unwrap_or(0)))));
        out
    }

    /// A type given inline (`field`) or as a type table index (`id_field`).
    fn ty_of(&self, m: &Msg<'s>, field: u32, id_field: u32) -> Option<String> {
        if let Some(t) = m.msg(field) {
            return Some(self.ty(&t, false));
        }
        let id = m.int(id_field)? as usize;
        let t = self.table.get(id)?;
        Some(self.ty(t, self.first_nullable.is_some_and(|f| id >= f)))
    }

    fn ty(&self, t: &Msg<'s>, table_nullable: bool) -> String {
        if let Some(abbreviated) = t.msg(13) {
            let mut text = self.ty(&abbreviated, false);
            if (t.int(3) == Some(1) || table_nullable) && !text.ends_with('?') {
                text.push('?');
            }
            return text;
        }
        let nullable = t.int(3) == Some(1) || table_nullable;
        let args: Vec<String> = t
            .msgs(2)
            .iter()
            .map(|a| match a.int(1).unwrap_or(2) {
                3 => "*".to_owned(),
                p => {
                    let inner = self.ty_of(a, 2, 3).unwrap_or_else(|| "*".into());
                    match p {
                        0 => format!("in {inner}"),
                        1 => format!("out {inner}"),
                        _ => inner,
                    }
                }
            })
            .collect();
        let text = if let Some(ix) = t.int(6) {
            let full = self.name(ix);
            let function = full.strip_prefix("kotlin/Function").or_else(|| full.strip_prefix("kotlin/jvm/functions/Function")).is_some_and(|n| n.parse::<u32>().is_ok());
            if function && !args.is_empty() {
                // `(A) -> R`, `T.() -> Unit`, `suspend () -> R`.
                let suspend = t.int(1).is_some_and(|f| f & 1 == 1);
                let (mut params, mut ret) = (&args[..args.len() - 1], args[args.len() - 1].clone());
                // A suspend function type's last parameter is its
                // continuation, `Continuation<R>`, and it returns `Any?`.
                if let Some(r) = params.last().and_then(|c| c.strip_prefix("Continuation<")).and_then(|c| c.strip_suffix('>')).filter(|_| suspend) {
                    ret = r.to_owned();
                    params = &params[..params.len() - 1];
                }
                let extension = t.msgs(100).iter().any(|a| a.int(1).is_some_and(|id| self.name(id) == "kotlin/ExtensionFunctionType"));
                let (receiver, params) = match (extension, params.split_first()) {
                    (true, Some((r, rest))) => (format!("{}.", if r.contains("->") { format!("({r})") } else { r.clone() }), rest),
                    _ => (String::new(), params),
                };
                let suspend = if suspend { "suspend " } else { "" };
                let text = format!("{suspend}{receiver}({}) -> {ret}", params.join(", "));
                return if nullable { format!("({text})?") } else { text };
            }
            let name = self.class(ix);
            if args.is_empty() { name } else { format!("{name}<{}>", args.join(", ")) }
        } else if let Some(id) = t.int(7) {
            self.params.iter().rev().find(|(i, _)| *i == id).map(|(_, n)| n.clone()).unwrap_or_else(|| "Any".into())
        } else if let Some(ix) = t.int(9) {
            self.name(ix)
        } else if let Some(ix) = t.int(12) {
            let name = self.class(ix);
            if args.is_empty() { name } else { format!("{name}<{}>", args.join(", ")) }
        } else {
            "Any".to_owned()
        };
        if nullable { format!("{text}?") } else { text }
    }

    /// `<reified T : Comparable<T>, out R>`.
    fn type_params(&self, params: &[Msg<'s>]) -> String {
        if params.is_empty() {
            return String::new();
        }
        let items: Vec<String> = params
            .iter()
            .map(|p| {
                let mut s = String::new();
                if p.int(3) == Some(1) {
                    s.push_str("reified ");
                }
                match p.int(4) {
                    Some(0) => s.push_str("in "),
                    Some(1) => s.push_str("out "),
                    _ => {}
                }
                s.push_str(&self.name(p.int(2).unwrap_or(0)));
                if let Some(bound) = self.ty_of(p, 5, 6).filter(|b| b != "Any?") {
                    s.push_str(&format!(" : {bound}"));
                }
                s
            })
            .collect();
        format!("<{}> ", items.join(", "))
    }

    fn value_params(&self, params: &[Msg<'s>]) -> String {
        params
            .iter()
            .map(|p| {
                let flags = p.int(1).unwrap_or(0);
                let name = self.name(p.int(2).unwrap_or(0));
                let mut s = String::new();
                if bit(flags, 2) {
                    s.push_str("crossinline ");
                }
                if bit(flags, 3) {
                    s.push_str("noinline ");
                }
                match self.ty_of(p, 4, 6) {
                    Some(element) => s.push_str(&format!("vararg {name}: {element}")),
                    None => s.push_str(&format!("{name}: {}", self.ty_of(p, 3, 5).unwrap_or_else(|| "Any".into()))),
                }
                if bit(flags, 1) {
                    s.push_str(" = COMPILED_CODE");
                }
                s
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

const BODY: &str = "{ /* compiled code */ }";

fn modality(flags: u64, interface: bool) -> &'static str {
    match ((flags >> 4) & 3, interface) {
        (_, true) => "",
        (1, _) => "open ",
        (2, _) => "abstract ",
        (3, _) => "sealed ",
        _ => "",
    }
}

fn vis(flags: u64) -> &'static str {
    if visibility(flags) == PROTECTED { "protected " } else { "" }
}

impl<'a> Decoded<'a> {
    fn scope(&self) -> Scope<'_> {
        Scope { names: &self.names, params: Vec::new(), table: &self.table, first_nullable: self.first_nullable }
    }

    /// The class's kind (a class's metadata only).
    pub fn class_kind(&self) -> ClassKind {
        match (self.root.int(1).unwrap_or(6) >> 6) & 7 {
            1 => ClassKind::Interface,
            2 => ClassKind::EnumClass,
            4 => ClassKind::AnnotationClass,
            5 => ClassKind::Object,
            6 => ClassKind::CompanionObject,
            _ => ClassKind::Class,
        }
    }

    /// The class header: `data class Pair<out A, out B> : Serializable`
    /// (the JVM superclass, `super_class`, gets `()`).
    pub fn header(&self, simple_name: &str, super_class: Option<&str>) -> String {
        let flags = self.root.int(1).unwrap_or(6);
        let scope = self.scope();
        let params = self.root.msgs(5);
        let scope = scope.with_params(&params);
        let kind = self.class_kind();
        let mut s = String::from(vis(flags));
        if kind == ClassKind::Class {
            s.push_str(modality(flags, false));
        }
        if kind == ClassKind::Interface && bit(flags, 14) {
            s.push_str("fun ");
        }
        if bit(flags, 9) {
            s.push_str("inner ");
        }
        if bit(flags, 10) {
            s.push_str("data ");
        }
        if bit(flags, 13) {
            s.push_str("value ");
        }
        s.push_str(match kind {
            ClassKind::Class => "class",
            ClassKind::Interface => "interface",
            ClassKind::EnumClass => "enum class",
            ClassKind::AnnotationClass => "annotation class",
            ClassKind::Object => "object",
            ClassKind::CompanionObject => "companion object",
        });
        if kind != ClassKind::CompanionObject || simple_name != "Companion" {
            s.push(' ');
            s.push_str(simple_name);
        }
        let generics = scope.type_params(&params);
        s.push_str(generics.trim_end());
        let mut supers = Vec::new();
        for t in self.root.msgs(6).iter().map(|t| (t, false)).chain(self.root.ints(2).iter().filter_map(|id| scope.table.get(*id as usize).map(|t| (t, true)))) {
            let text = scope.ty(t.0, false);
            let full = t.0.int(6).map(|ix| scope.name(ix)).unwrap_or_default();
            if matches!(full.as_str(), "kotlin/Any" | "kotlin/Enum" | "kotlin/Annotation") {
                continue;
            }
            let is_super = super_class.is_some_and(|s| s == full.replace('.', "$"));
            supers.push(if is_super { format!("{text}()") } else { text });
        }
        if !supers.is_empty() {
            s.push_str(&format!(" : {}", supers.join(", ")));
        }
        s
    }

    /// The members as Kotlin lines (no indent): enum entries, constructors,
    /// properties, functions.
    pub fn members(&self) -> Vec<String> {
        let mut out = Vec::new();
        let class = self.kind == 1;
        let kind = if class { self.class_kind() } else { ClassKind::Class };
        let interface = kind == ClassKind::Interface;
        let mut scope = self.scope();
        if class {
            scope = scope.with_params(&self.root.msgs(5));
            let entries: Vec<String> = self.root.msgs(13).iter().filter_map(|e| e.int(1)).map(|ix| scope.name(ix)).collect();
            if !entries.is_empty() {
                out.push(format!("{};", entries.join(", ")));
            }
            if !matches!(kind, ClassKind::Object | ClassKind::CompanionObject | ClassKind::Interface) {
                for c in self.root.msgs(8) {
                    let flags = c.int(1).unwrap_or(6);
                    if shown(flags) && kind != ClassKind::EnumClass {
                        out.push(format!("{}constructor({}) {BODY}", vis(flags), scope.value_params(&c.msgs(2))));
                    }
                }
            }
        }
        let (functions, properties) = if class { (9, 10) } else { (3, 4) };
        for p in self.root.msgs(properties) {
            let flags = p.int(11).unwrap_or(518);
            // Fake overrides are inherited, not declared here.
            if !shown(flags) || (flags >> 6) & 3 == 1 {
                continue;
            }
            let params = p.msgs(4);
            let scope = scope.with_params(&params);
            let keyword = if bit(flags, 11) {
                "const val"
            } else if bit(flags, 12) {
                "lateinit var"
            } else if bit(flags, 8) {
                "var"
            } else {
                "val"
            };
            let receiver = scope.ty_of(&p, 5, 10).map(|r| format!("{r}.")).unwrap_or_default();
            let ty = scope.ty_of(&p, 3, 9).unwrap_or_else(|| "Any".into());
            out.push(format!("{}{}{keyword} {}{receiver}{}: {ty}", vis(flags), modality(flags, interface || !class), scope.type_params(&params), scope.name(p.int(2).unwrap_or(0))));
        }
        for f in self.root.msgs(functions) {
            let flags = f.int(9).unwrap_or(6);
            if !shown(flags) || (flags >> 6) & 3 == 1 {
                continue;
            }
            let params = f.msgs(4);
            let scope = scope.with_params(&params);
            let mut s = format!("{}{}", vis(flags), modality(flags, interface || !class));
            for (n, word) in [(10, "inline "), (9, "infix "), (8, "operator "), (13, "suspend "), (11, "tailrec "), (12, "external ")] {
                if bit(flags, n) {
                    s.push_str(word);
                }
            }
            let receiver = scope.ty_of(&f, 5, 8).map(|r| format!("{}.", if r.contains("->") { format!("({r})") } else { r })).unwrap_or_default();
            let ret = scope.ty_of(&f, 3, 7).unwrap_or_else(|| "Unit".into());
            let returns = if ret == "Unit" { String::new() } else { format!(": {ret}") };
            let abstract_ = (flags >> 4) & 3 == 2;
            let body = if abstract_ { String::new() } else { format!(" {BODY}") };
            s.push_str(&format!("fun {}{receiver}{}({}){returns}{body}", scope.type_params(&params), scope.name(f.int(2).unwrap_or(0)), scope.value_params(&f.msgs(6))));
            out.push(s);
        }
        out
    }
}

/// `d1`'s bytes, for [`decode`].
pub fn bytes(meta: &Metadata) -> Vec<u8> {
    decode_bytes(&meta.d1)
}

/// Decodes a class's or file's metadata from its [`bytes`]; none for a
/// multi-file facade (its `d1` names the parts) or what can't be read.
pub fn decode<'a>(meta: &Metadata, bytes: &'a [u8]) -> Option<Decoded<'a>> {
    if !matches!(meta.kind, 1 | 2 | 5) {
        return None;
    }
    let mut at = 0;
    let len = varint(bytes, &mut at)? as usize;
    let types = Msg::parse(bytes.get(at..at.checked_add(len)?)?)?;
    let root = Msg::parse(&bytes[at + len..])?;
    let names = string_table(&types, &meta.d2);
    let table = root.msg(30);
    let first_nullable = table.as_ref().and_then(|t| t.int(2)).map(|n| n as usize);
    let table = table.map(|t| t.msgs(1)).unwrap_or_default();
    Some(Decoded { names, kind: meta.kind, root, table, first_nullable })
}

/// A multi-file facade's parts (`kotlin/collections/CollectionsKt___CollectionsKt`).
pub fn parts(meta: &Metadata, package: &str) -> Vec<String> {
    if meta.kind != 4 {
        return Vec::new();
    }
    meta.d1.iter().map(|p| if p.contains('/') || package.is_empty() { p.clone() } else { format!("{package}/{p}") }).collect()
}
