//! Generate (Alt+Insert), as IntelliJ's: `toString()`, `equals()` and
//! `hashCode()` for Kotlin and Java (calling `super`'s when a superclass
//! overrides them), Kotlin's secondary constructor, and Java's
//! constructor, getters and setters, written as IntelliJ's default templates write them (Kotlin's
//! singleExpr.vm / defaultEquals.vm / defaultHashcode.vm, Java's
//! DefaultConcatMember.vm / objectsEquals.vm / objectsHashCode.vm /
//! defaultGetter.vm / defaultSetter.vm), then indented for the class.

use super::imports::ImportStyle;

/// A property (Kotlin) or field (Java) the members use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    /// As written: `String?`, `IntArray`, `int[]`; empty when not written.
    pub ty: String,
    pub is_static: bool,
    pub is_final: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Member {
    Constructor,
    /// Kotlin's, for a class without a primary constructor.
    SecondaryConstructor,
    Getter,
    Setter,
    GetterAndSetter,
    EqualsHashCode,
    ToString,
}

impl Member {
    pub fn label(self) -> &'static str {
        match self {
            Member::Constructor => "Constructor",
            Member::SecondaryConstructor => "Secondary Constructor",
            Member::Getter => "Getter",
            Member::Setter => "Setter",
            Member::GetterAndSetter => "Getter and Setter",
            Member::EqualsHashCode => "equals() and hashCode()",
            Member::ToString => "toString()",
        }
    }

    /// The member chooser's title, IntelliJ's.
    pub fn chooser_title(self, style: ImportStyle) -> &'static str {
        match (self, style) {
            (Member::Constructor, _) => "Choose Fields to Initialize by Constructor",
            (Member::SecondaryConstructor, _) => "Choose Properties to Initialize by Constructor",
            (Member::Getter, _) => "Select Fields to Generate Getters",
            (Member::Setter, _) => "Select Fields to Generate Setters",
            (Member::GetterAndSetter, _) => "Select Fields to Generate Getters and Setters",
            (Member::EqualsHashCode, ImportStyle::Kotlin) => "Choose Properties To Be Included In 'equals()' and 'hashCode()'",
            (Member::EqualsHashCode, ImportStyle::Java) => "Choose Fields to Be Included in equals() and hashCode()",
            (Member::ToString, ImportStyle::Kotlin) => "Generate 'toString()'",
            (Member::ToString, ImportStyle::Java) => "Generate toString()",
        }
    }
}

/// The Generate popup's items for a class, in IntelliJ's order.
pub fn members(style: ImportStyle) -> &'static [Member] {
    match style {
        ImportStyle::Kotlin => &[Member::SecondaryConstructor, Member::EqualsHashCode, Member::ToString],
        ImportStyle::Java => &[Member::Constructor, Member::Getter, Member::Setter, Member::GetterAndSetter, Member::EqualsHashCode, Member::ToString],
    }
}

const KOTLIN_PRIMITIVES: &[&str] = &["Int", "Long", "Short", "Byte", "Char", "Boolean", "Float", "Double"];
const JAVA_PRIMITIVES: &[&str] = &["int", "long", "short", "byte", "char", "boolean", "float", "double"];

impl Field {
    fn base(&self) -> &str {
        self.ty.trim_end_matches('?').trim()
    }
    fn nullable(&self) -> bool {
        self.ty.ends_with('?')
    }
    fn kotlin_array(&self) -> bool {
        let b = self.base();
        b.starts_with("Array<") || (b.ends_with("Array") && KOTLIN_PRIMITIVES.iter().any(|p| b == format!("{p}Array")))
    }
    fn kotlin_nested_array(&self) -> bool {
        self.base().strip_prefix("Array<").is_some_and(|inner| inner.starts_with("Array<") || KOTLIN_PRIMITIVES.iter().any(|p| inner.starts_with(&format!("{p}Array"))))
    }
    fn java_array(&self) -> bool {
        self.ty.ends_with("[]")
    }
    fn java_primitive(&self) -> bool {
        JAVA_PRIMITIVES.contains(&self.ty.as_str())
    }
}

/// The generated members, one per string, each without indentation in
/// its first line and `unit` per level inside; plus the imports they need.
pub struct Generated {
    pub members: Vec<String>,
    pub imports: Vec<&'static str>,
}

/// What the superclass (not `Any` / `Object`) already overrides: the
/// generated `equals()` / `hashCode()` call it first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Supers {
    pub equals: bool,
    pub hash_code: bool,
    pub to_string: bool,
}

/// Generates `member` for class `class` (with `type_params` type
/// parameters) from `fields` (already chosen).
pub fn generate(style: ImportStyle, member: Member, class: &str, type_params: usize, fields: &[Field], unit: &str) -> Generated {
    generate_with(style, member, class, type_params, fields, Supers::default(), unit)
}

/// [`generate`], `super`'s `equals()` / `hashCode()` called as `supers` says.
pub fn generate_with(style: ImportStyle, member: Member, class: &str, type_params: usize, fields: &[Field], supers: Supers, unit: &str) -> Generated {
    let u = unit;
    let mut imports = Vec::new();
    let members = match (style, member) {
        (ImportStyle::Kotlin, Member::ToString) => vec![kotlin_to_string(class, fields, u)],
        (ImportStyle::Kotlin, Member::EqualsHashCode) => {
            // Properties of primitive types first, as IntelliJ sorts them.
            let mut sorted: Vec<&Field> = fields.iter().collect();
            sorted.sort_by_key(|f| !KOTLIN_PRIMITIVES.contains(&f.base()));
            vec![kotlin_equals(class, type_params, &sorted, supers.equals, u), kotlin_hash_code(&sorted, supers.hash_code, u)]
        }
        (ImportStyle::Java, Member::ToString) => {
            if fields.iter().any(|f| f.java_array()) {
                imports.push("java.util.Arrays");
            }
            vec![java_to_string(class, fields, u)]
        }
        (ImportStyle::Java, Member::EqualsHashCode) => {
            imports.push("java.util.Objects");
            if fields.iter().any(|f| f.java_array()) {
                imports.push("java.util.Arrays");
            }
            vec![java_equals(class, type_params, fields, supers.equals, u), java_hash_code(fields, supers.hash_code, u)]
        }
        (ImportStyle::Java, Member::Constructor) => vec![java_constructor(class, fields, u)],
        (ImportStyle::Kotlin, Member::SecondaryConstructor) => vec![kotlin_secondary_constructor(fields, u)],
        (ImportStyle::Java, Member::Getter) => fields.iter().map(|f| java_getter(f, u)).collect(),
        (ImportStyle::Java, Member::Setter) => fields.iter().map(|f| java_setter(class, f, u)).collect(),
        (ImportStyle::Java, Member::GetterAndSetter) => fields.iter().flat_map(|f| [java_getter(f, u), java_setter(class, f, u)]).collect(),
        (ImportStyle::Kotlin, _) | (ImportStyle::Java, Member::SecondaryConstructor) => Vec::new(),
    };
    Generated { members, imports }
}

fn kotlin_to_string(class: &str, fields: &[Field], u: &str) -> String {
    let parts: Vec<String> = fields
        .iter()
        .map(|f| {
            let quote = if f.base() == "String" && !f.nullable() { "'" } else { "" };
            let value = if f.kotlin_array() { format!("${{{}.contentToString()}}", f.name) } else { format!("${}", f.name) };
            format!("{}={quote}{value}{quote}", f.name)
        })
        .collect();
    format!("override fun toString(): String {{\n{u}return \"{class}({})\"\n}}", parts.join(", "))
}

fn wildcards(n: usize, wildcard: &str) -> String {
    if n == 0 { String::new() } else { format!("<{}>", vec![wildcard; n].join(", ")) }
}

fn kotlin_equals(class: &str, type_params: usize, fields: &[&Field], call_super: bool, u: &str) -> String {
    let mut out = format!("override fun equals(other: Any?): Boolean {{\n{u}if (this === other) return true\n{u}if (javaClass != other?.javaClass) return false\n");
    if call_super {
        out.push_str(&format!("{u}if (!super.equals(other)) return false\n"));
    }
    if !fields.is_empty() {
        out.push_str(&format!("\n{u}other as {class}{}\n\n", wildcards(type_params, "*")));
        for f in fields {
            let condition = if f.kotlin_nested_array() {
                format!("!{0}.contentDeepEquals(other.{0})", f.name)
            } else if f.kotlin_array() {
                format!("!{0}.contentEquals(other.{0})", f.name)
            } else {
                format!("{0} != other.{0}", f.name)
            };
            out.push_str(&format!("{u}if ({condition}) return false\n"));
        }
    }
    out.push_str(&format!("\n{u}return true\n}}"));
    out
}

fn kotlin_field_hash(f: &Field) -> String {
    let n = &f.name;
    if f.kotlin_array() {
        let call = if f.kotlin_nested_array() { "contentDeepHashCode()" } else { "contentHashCode()" };
        return if f.nullable() { format!("({n}?.{call} ?: 0)") } else { format!("{n}.{call}") };
    }
    if !f.nullable() && matches!(f.base(), "Byte" | "Short" | "Int") {
        return n.clone();
    }
    format!("{n}.hashCode()")
}

fn kotlin_hash_code(fields: &[&Field], call_super: bool, u: &str) -> String {
    // Alone, `(x?.contentHashCode() ?: 0)` needs no parentheses.
    let alone = |f: &Field| {
        let h = kotlin_field_hash(f);
        match h.strip_prefix('(').and_then(|h| h.strip_suffix(')')) {
            Some(inner) => inner.to_owned(),
            None => h,
        }
    };
    let body = match fields {
        [] if call_super => format!("{u}return super.hashCode()"),
        _ if call_super => {
            let mut lines = vec![format!("{u}var result = super.hashCode()")];
            lines.extend(fields.iter().map(|f| format!("{u}result = 31 * result + {}", kotlin_field_hash(f))));
            lines.push(format!("{u}return result"));
            lines.join("\n")
        }
        [] => format!("{u}return javaClass.hashCode()"),
        [f] => format!("{u}return {}", alone(f)),
        [first, rest @ ..] => {
            let mut lines = vec![format!("{u}var result = {}", alone(first))];
            lines.extend(rest.iter().map(|f| format!("{u}result = 31 * result + {}", kotlin_field_hash(f))));
            lines.push(format!("{u}return result"));
            lines.join("\n")
        }
    };
    format!("override fun hashCode(): Int {{\n{body}\n}}")
}

fn java_to_string(class: &str, fields: &[Field], u: &str) -> String {
    let c = format!("{u}{u}{u}");
    if fields.is_empty() {
        return format!("@Override\npublic String toString() {{\n{u}return \"{class}{{}}\";\n}}");
    }
    let mut lines = vec![format!("{u}return \"{class}{{\" +")];
    for (i, f) in fields.iter().enumerate() {
        let sep = if i == 0 { "\"" } else { "\", " };
        let line = if f.java_array() {
            format!("{c}{sep}{}=\" + Arrays.toString({}) +", f.name, f.name)
        } else if f.ty == "String" || f.ty == "java.lang.String" {
            format!("{c}{sep}{}='\" + {} + '\\'' +", f.name, f.name)
        } else {
            format!("{c}{sep}{}=\" + {} +", f.name, f.name)
        };
        lines.push(line);
    }
    lines.push(format!("{c}'}}';"));
    format!("@Override\npublic String toString() {{\n{}\n}}", lines.join("\n"))
}

/// Generate toString()'s templates, as IntelliJ's chooser names them
/// (the first is the default).
pub fn to_string_templates(style: ImportStyle) -> &'static [&'static str] {
    match style {
        ImportStyle::Kotlin => &[
            "Single template",
            "Single template with super call",
            "Multiple templates with concatenation",
            "Multiple templates with concatenation and super call",
        ],
        ImportStyle::Java => &[
            "String concat (+)",
            "String concat (+) and super.toString()",
            "StringBuffer",
            "StringBuilder (JDK 1.5)",
            "ToStringBuilder (Apache commons-lang)",
            "ToStringBuilder (Apache commons-lang 3)",
            "Objects.toStringHelper (Guava)",
            "MoreObjects.toStringHelper (Guava 18+)",
            "StringJoiner (JDK 1.8)",
        ],
    }
}

/// The class a library template needs (only offered when it's there).
pub fn to_string_template_class(template: &str) -> Option<&'static str> {
    match template {
        "ToStringBuilder (Apache commons-lang)" => Some("org.apache.commons.lang.builder.ToStringBuilder"),
        "ToStringBuilder (Apache commons-lang 3)" => Some("org.apache.commons.lang3.builder.ToStringBuilder"),
        "Objects.toStringHelper (Guava)" => Some("com.google.common.base.Objects"),
        "MoreObjects.toStringHelper (Guava 18+)" => Some("com.google.common.base.MoreObjects"),
        _ => None,
    }
}

/// toString() from a template of [`to_string_templates`], as IntelliJ's
/// .vm writes it once reformatted; `call_super`: the class has a
/// superclass whose toString() the super-call templates add (Kotlin's
/// only when it overrides Any's).
pub fn to_string(style: ImportStyle, template: &str, class: &str, fields: &[Field], call_super: bool, u: &str) -> Generated {
    let c = format!("{u}{u}{u}");
    let mut imports: Vec<&'static str> = Vec::new();
    let member = match (style, template) {
        (ImportStyle::Kotlin, _) => {
            let multiple = template.starts_with("Multiple");
            let with_super = template.ends_with("super call") && call_super;
            let parts: Vec<String> = fields
                .iter()
                .map(|f| {
                    let quote = if f.base() == "String" && !f.nullable() { "'" } else { "" };
                    let value = if f.kotlin_array() { format!("${{{}.contentToString()}}", f.name) } else { format!("${}", f.name) };
                    format!("{}={quote}{value}{quote}", f.name)
                })
                .collect();
            let sup = if with_super { " ${super.toString()}" } else { "" };
            if !multiple || parts.is_empty() {
                let body = if multiple && with_super { format!("\"{class}()\" +\n{u}{u}\"{sup}\"") } else { format!("\"{class}({}){sup}\"", parts.join(", ")) };
                format!("override fun toString(): String {{\n{u}return {body}\n}}")
            } else {
                let k = format!("{u}{u}");
                let mut lines = vec![format!("{u}return \"{class}(\" +")];
                for (i, part) in parts.iter().enumerate() {
                    let sep = if i + 1 < parts.len() { ", " } else { "" };
                    lines.push(format!("{k}\"{part}{sep}\" +"));
                }
                if with_super {
                    lines.push(format!("{k}\")\" +"));
                    lines.push(format!("{k}\"{sup}\""));
                } else {
                    lines.push(format!("{k}\")\""));
                }
                format!("override fun toString(): String {{\n{}\n}}", lines.join("\n"))
            }
        }
        (ImportStyle::Java, "String concat (+)") => {
            if fields.iter().any(|f| f.java_array()) {
                imports.push("java.util.Arrays");
            }
            java_to_string(class, fields, u)
        }
        (ImportStyle::Java, "String concat (+) and super.toString()") => {
            if fields.iter().any(|f| f.java_array()) {
                imports.push("java.util.Arrays");
            }
            let plain = java_to_string(class, fields, u);
            if !call_super {
                plain
            } else if fields.is_empty() {
                plain.replace(&format!("\"{class}{{}}\""), &format!("\"{class}{{}} \" + super.toString()"))
            } else {
                plain.replace(&format!("{c}'}}';"), &format!("{c}\"}} \" + super.toString();"))
            }
        }
        (ImportStyle::Java, "StringBuffer" | "StringBuilder (JDK 1.5)") => {
            let ty = if template == "StringBuffer" { "StringBuffer" } else { "StringBuilder" };
            let mut lines = vec![format!("{u}final {ty} sb = new {ty}(\"{class}{{\");")];
            for (i, f) in fields.iter().enumerate() {
                let sep = if i == 0 { "" } else { ", " };
                let string = f.ty == "String" || f.ty == "java.lang.String";
                let label = if string { format!("{sep}{}='", f.name) } else { format!("{sep}{}=", f.name) };
                let primitive_array = f.java_array() && JAVA_PRIMITIVES.contains(&f.ty.trim_end_matches("[]"));
                let n = &f.name;
                if ty == "StringBuffer" && primitive_array {
                    lines.push(format!("{u}sb.append(\"{label}\");"));
                    lines.push(format!("{u}if ({n} == null) sb.append(\"null\");"));
                    lines.push(format!("{u}else {{"));
                    lines.push(format!("{u}{u}sb.append('[');"));
                    lines.push(format!("{u}{u}for (int i = 0; i < {n}.length; ++i)"));
                    lines.push(format!("{u}{u}{u}sb.append(i == 0 ? \"\" : \", \").append({n}[i]);"));
                    lines.push(format!("{u}{u}sb.append(']');"));
                    lines.push(format!("{u}}}"));
                } else if ty == "StringBuffer" && f.java_array() {
                    imports.push("java.util.Arrays");
                    lines.push(format!("{u}sb.append(\"{label}\").append({n} == null ? \"null\" : Arrays.asList({n}).toString());"));
                } else if f.java_array() {
                    imports.push("java.util.Arrays");
                    lines.push(format!("{u}sb.append(\"{label}\").append(Arrays.toString({n}));"));
                } else if string {
                    lines.push(format!("{u}sb.append(\"{label}\").append({n}).append('\\'');"));
                } else {
                    lines.push(format!("{u}sb.append(\"{label}\").append({n});"));
                }
            }
            lines.push(format!("{u}sb.append('}}');"));
            lines.push(format!("{u}return sb.toString();"));
            format!("@Override\npublic String toString() {{\n{}\n}}", lines.join("\n"))
        }
        (ImportStyle::Java, "StringJoiner (JDK 1.8)") => {
            imports.push("java.util.StringJoiner");
            let mut lines = vec![format!("{u}return new StringJoiner(\", \", {class}.class.getSimpleName() + \"[\", \"]\")")];
            for f in fields.iter().filter(|f| !f.is_static) {
                let n = &f.name;
                let add = if f.java_array() {
                    imports.push("java.util.Arrays");
                    format!("\"{n}=\" + Arrays.toString({n})")
                } else if f.ty == "String" || f.ty == "java.lang.String" {
                    format!("\"{n}='\" + {n} + \"'\"")
                } else {
                    format!("\"{n}=\" + {n}")
                };
                lines.push(format!("{c}.add({add})"));
            }
            lines.push(format!("{c}.toString();"));
            format!("@Override\npublic String toString() {{\n{}\n}}", lines.join("\n"))
        }
        (ImportStyle::Java, _) => {
            // The libraries' builders: `.add` / `.append` of each name and value.
            let (start, add, import) = match template {
                "ToStringBuilder (Apache commons-lang)" => ("new ToStringBuilder(this)", "append", "org.apache.commons.lang.builder.ToStringBuilder"),
                "ToStringBuilder (Apache commons-lang 3)" => ("new ToStringBuilder(this)", "append", "org.apache.commons.lang3.builder.ToStringBuilder"),
                "Objects.toStringHelper (Guava)" => ("Objects.toStringHelper(this)", "add", "com.google.common.base.Objects"),
                _ => ("MoreObjects.toStringHelper(this)", "add", "com.google.common.base.MoreObjects"),
            };
            imports.push(import);
            let mut lines = vec![format!("{u}return {start}")];
            lines.extend(fields.iter().map(|f| format!("{c}.{add}(\"{0}\", {0})", f.name)));
            lines.push(format!("{c}.toString();"));
            format!("@Override\npublic String toString() {{\n{}\n}}", lines.join("\n"))
        }
    };
    imports.dedup();
    Generated { members: vec![member], imports }
}

/// IntelliJ's unique local name: `base`, else `base1`, `base2`… not a
/// field's name.
fn unique(base: &str, fields: &[Field]) -> String {
    if !fields.iter().any(|f| f.name == base) {
        return base.to_owned();
    }
    (1..).map(|i| format!("{base}{i}")).find(|n| !fields.iter().any(|f| &f.name == n)).unwrap_or_default()
}

fn decapitalize(name: &str) -> String {
    let mut chars = name.chars();
    chars.next().map(|c| c.to_lowercase().chain(chars).collect()).unwrap_or_default()
}

fn java_equals(class: &str, type_params: usize, fields: &[Field], call_super: bool, u: &str) -> String {
    let param = unique("o", fields);
    let instance = unique(&decapitalize(class), fields);
    let ty = format!("{class}{}", wildcards(type_params, "?"));
    let conditions: Vec<String> = fields
        .iter()
        .map(|f| {
            let (a, b) = (&f.name, format!("{instance}.{}", f.name));
            match f.ty.as_str() {
                "double" => format!("Double.compare({a}, {b}) == 0"),
                "float" => format!("Float.compare({a}, {b}) == 0"),
                _ if f.java_primitive() => format!("{a} == {b}"),
                _ if f.java_array() => format!("Objects.deepEquals({a}, {b})"),
                _ => format!("Objects.equals({a}, {b})"),
            }
        })
        .collect();
    let result = if conditions.is_empty() { "true".to_owned() } else { conditions.join(" && ") };
    let sup = if call_super { format!("{u}if (!super.equals({param})) return false;\n") } else { String::new() };
    format!(
        "@Override\npublic boolean equals(Object {param}) {{\n{u}if ({param} == null || getClass() != {param}.getClass()) return false;\n{sup}{u}{ty} {instance} = ({ty}) {param};\n{u}return {result};\n}}"
    )
}

fn java_hash_code(fields: &[Field], call_super: bool, u: &str) -> String {
    let array_hash = |f: &Field| if f.ty.ends_with("[][]") { format!("Arrays.deepHashCode({})", f.name) } else { format!("Arrays.hashCode({})", f.name) };
    let body = match fields {
        [] if call_super => "super.hashCode()".to_owned(),
        _ if call_super => format!("Objects.hash(super.hashCode(), {})", fields.iter().map(|f| if f.java_array() { array_hash(f) } else { f.name.clone() }).collect::<Vec<_>>().join(", ")),
        [f] if !f.java_array() => format!("Objects.hashCode({})", f.name),
        [f] => array_hash(f),
        _ => format!("Objects.hash({})", fields.iter().map(|f| if f.java_array() { array_hash(f) } else { f.name.clone() }).collect::<Vec<_>>().join(", ")),
    };
    format!("@Override\npublic int hashCode() {{\n{u}return {body};\n}}")
}

fn java_constructor(class: &str, fields: &[Field], u: &str) -> String {
    constructor_calling(ImportStyle::Java, class, fields, &[], u)
}

fn kotlin_secondary_constructor(fields: &[Field], u: &str) -> String {
    constructor_calling(ImportStyle::Kotlin, "", fields, &[], u)
}

/// A constructor or Kotlin secondary constructor for each chosen
/// superclass constructor (one without when none), as IntelliJ writes
/// them: the super constructor's parameters first, then one per field
/// (renamed `name1` when the super constructor took the name); Java's
/// `super(…);` before the assignments, Kotlin's `: super(…)` after the
/// parameters and no body without fields.
pub fn constructors(style: ImportStyle, class: &str, fields: &[Field], supers: &[Vec<String>], u: &str) -> Vec<String> {
    if supers.is_empty() {
        return vec![constructor_calling(style, class, fields, &[], u)];
    }
    supers.iter().map(|params| constructor_calling(style, class, fields, params, u)).collect()
}

fn constructor_calling(style: ImportStyle, class: &str, fields: &[Field], super_params: &[String], u: &str) -> String {
    let kotlin = style == ImportStyle::Kotlin;
    // (declaration, name, vararg) of each super parameter.
    let supers: Vec<(String, String, bool)> = super_params
        .iter()
        .map(|p| {
            let p = p.trim();
            if kotlin {
                let vararg = p.starts_with("vararg ");
                let name = p.trim_start_matches("vararg ").split(':').next().unwrap_or("").trim().to_owned();
                (p.to_owned(), name, vararg)
            } else {
                let name = p.rsplit(' ').next().unwrap_or("").to_owned();
                (p.trim_start_matches("final ").to_owned(), name, p.contains("..."))
            }
        })
        .collect();
    let mut taken: Vec<String> = supers.iter().map(|(_, n, _)| n.clone()).collect();
    let fields: Vec<(&Field, String)> = fields
        .iter()
        .map(|f| {
            let mut name = f.name.clone();
            let mut n = 1;
            while taken.contains(&name) {
                name = format!("{}{n}", f.name);
                n += 1;
            }
            taken.push(name.clone());
            (f, name)
        })
        .collect();
    let field_params: Vec<String> = fields.iter().map(|(f, n)| if kotlin { format!("{n}: {}", f.ty) } else { format!("{} {n}", f.ty) }).collect();
    let mut params: Vec<String> = supers.iter().map(|(d, _, _)| d.clone()).collect();
    // Java's fields go before a trailing vararg.
    let vararg_last = !kotlin && supers.last().is_some_and(|(_, _, v)| *v);
    let at = if vararg_last { params.len() - 1 } else { params.len() };
    params.splice(at..at, field_params);
    let arguments: Vec<String> = supers.iter().map(|(_, n, v)| if kotlin && *v { format!("*{n}") } else { n.clone() }).collect();
    let assignments: Vec<String> = fields.iter().map(|(f, n)| if kotlin { format!("{u}this.{} = {n}", f.name) } else { format!("{u}this.{} = {n};", f.name) }).collect();
    if kotlin {
        let call = if supers.is_empty() { String::new() } else { format!(" : super({})", arguments.join(", ")) };
        let body = if assignments.is_empty() { String::new() } else { format!(" {{\n{}\n}}", assignments.join("\n")) };
        return format!("constructor({}){call}{body}", params.join(", "));
    }
    let mut body: Vec<String> = Vec::new();
    if !supers.is_empty() {
        body.push(format!("{u}super({});", arguments.join(", ")));
    }
    body.extend(assignments);
    if body.is_empty() {
        return format!("public {class}() {{\n}}");
    }
    format!("public {class}({}) {{\n{}\n}}", params.join(", "), body.join("\n"))
}

/// `name` → `Name`, as JavaBeans capitalize (`xValue` stays `xValue`).
fn bean_capitalize(name: &str) -> String {
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(a), Some(b)) if a.is_lowercase() && b.is_uppercase() => name.to_owned(),
        (Some(a), _) => a.to_uppercase().chain(name.chars().skip(1)).collect(),
        _ => String::new(),
    }
}

fn java_getter(f: &Field, u: &str) -> String {
    let prefix = if f.ty == "boolean" { "is" } else { "get" };
    let statik = if f.is_static { "static " } else { "" };
    format!("public {statik}{} {prefix}{}() {{\n{u}return {};\n}}", f.ty, bean_capitalize(&f.name), f.name)
}

fn java_setter(class: &str, f: &Field, u: &str) -> String {
    let statik = if f.is_static { "static " } else { "" };
    let target = if f.is_static { format!("{class}.{}", f.name) } else { format!("this.{}", f.name) };
    format!("public {statik}void set{}({} {}) {{\n{u}{target} = {};\n}}", bean_capitalize(&f.name), f.ty, f.name, f.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(name: &str, ty: &str) -> Field {
        Field { name: name.into(), ty: ty.into(), is_static: false, is_final: false }
    }

    #[test]
    fn kotlin_members() {
        let fields = [f("name", "String"), f("age", "Int"), f("tags", "IntArray"), f("nick", "String?")];
        let g = generate(ImportStyle::Kotlin, Member::ToString, "Person", 0, &fields, "    ");
        assert_eq!(g.members[0], "override fun toString(): String {\n    return \"Person(name='$name', age=$age, tags=${tags.contentToString()}, nick=$nick)\"\n}");
        let g = generate(ImportStyle::Kotlin, Member::EqualsHashCode, "Person", 0, &fields, "    ");
        assert_eq!(
            g.members[0],
            "override fun equals(other: Any?): Boolean {\n    if (this === other) return true\n    if (javaClass != other?.javaClass) return false\n\n    other as Person\n\n    if (age != other.age) return false\n    if (name != other.name) return false\n    if (!tags.contentEquals(other.tags)) return false\n    if (nick != other.nick) return false\n\n    return true\n}"
        );
        assert_eq!(
            g.members[1],
            "override fun hashCode(): Int {\n    var result = age\n    result = 31 * result + name.hashCode()\n    result = 31 * result + tags.contentHashCode()\n    result = 31 * result + nick.hashCode()\n    return result\n}"
        );
        let g = generate(ImportStyle::Kotlin, Member::EqualsHashCode, "Box", 1, &[f("value", "String")], "    ");
        assert!(g.members[0].contains("other as Box<*>"));
        assert_eq!(g.members[1], "override fun hashCode(): Int {\n    return value.hashCode()\n}");
    }

    #[test]
    fn java_members() {
        let fields = [f("name", "String"), f("age", "int"), f("score", "double"), f("tags", "int[]")];
        let g = generate(ImportStyle::Java, Member::ToString, "Person", 0, &fields, "    ");
        assert_eq!(
            g.members[0],
            "@Override\npublic String toString() {\n    return \"Person{\" +\n            \"name='\" + name + '\\'' +\n            \", age=\" + age +\n            \", score=\" + score +\n            \", tags=\" + Arrays.toString(tags) +\n            '}';\n}"
        );
        assert_eq!(g.imports, ["java.util.Arrays"]);
        let g = generate(ImportStyle::Java, Member::EqualsHashCode, "Person", 0, &fields, "    ");
        assert_eq!(
            g.members[0],
            "@Override\npublic boolean equals(Object o) {\n    if (o == null || getClass() != o.getClass()) return false;\n    Person person = (Person) o;\n    return Objects.equals(name, person.name) && age == person.age && Double.compare(score, person.score) == 0 && Objects.deepEquals(tags, person.tags);\n}"
        );
        assert_eq!(g.members[1], "@Override\npublic int hashCode() {\n    return Objects.hash(name, age, score, Arrays.hashCode(tags));\n}");
        let g = generate(ImportStyle::Java, Member::GetterAndSetter, "Person", 0, &[f("active", "boolean")], "    ");
        assert_eq!(g.members, ["public boolean isActive() {\n    return active;\n}", "public void setActive(boolean active) {\n    this.active = active;\n}"]);
        let g = generate(ImportStyle::Java, Member::Constructor, "Person", 0, &fields[..2], "    ");
        assert_eq!(g.members[0], "public Person(String name, int age) {\n    this.name = name;\n    this.age = age;\n}");
    }

    #[test]
    fn calls_super_and_secondary_constructors() {
        let supers = Supers { equals: true, hash_code: true, to_string: false };
        let g = generate_with(ImportStyle::Kotlin, Member::EqualsHashCode, "Dog", 0, &[f("name", "String")], supers, "    ");
        assert!(g.members[0].contains("if (javaClass != other?.javaClass) return false\n    if (!super.equals(other)) return false\n\n    other as Dog"));
        assert_eq!(g.members[1], "override fun hashCode(): Int {\n    var result = super.hashCode()\n    result = 31 * result + name.hashCode()\n    return result\n}");
        let g = generate_with(ImportStyle::Java, Member::EqualsHashCode, "Dog", 0, &[f("name", "String"), f("age", "int")], supers, "    ");
        assert!(g.members[0].contains("getClass() != o.getClass()) return false;\n    if (!super.equals(o)) return false;\n    Dog dog = (Dog) o;"));
        assert_eq!(g.members[1], "@Override\npublic int hashCode() {\n    return Objects.hash(super.hashCode(), name, age);\n}");
        let g = generate(ImportStyle::Kotlin, Member::SecondaryConstructor, "Dog", 0, &[f("name", "String"), f("age", "Int")], "    ");
        assert_eq!(g.members[0], "constructor(name: String, age: Int) {\n    this.name = name\n    this.age = age\n}");
        let g = generate(ImportStyle::Kotlin, Member::SecondaryConstructor, "Dog", 0, &[], "    ");
        assert_eq!(g.members[0], "constructor()");
    }

    #[test]
    fn to_string_templates() {
        let fields = [f("name", "String"), f("age", "Int")];
        let kt = |t: &str, sup: bool| to_string(ImportStyle::Kotlin, t, "Dog", &fields, sup, "    ").members.remove(0);
        assert_eq!(kt("Single template", true), "override fun toString(): String {\n    return \"Dog(name='$name', age=$age)\"\n}");
        assert_eq!(kt("Single template with super call", true), "override fun toString(): String {\n    return \"Dog(name='$name', age=$age) ${super.toString()}\"\n}");
        assert_eq!(kt("Single template with super call", false), kt("Single template", true));
        assert_eq!(kt("Multiple templates with concatenation", false), "override fun toString(): String {\n    return \"Dog(\" +\n        \"name='$name', \" +\n        \"age=$age\" +\n        \")\"\n}");
        assert_eq!(kt("Multiple templates with concatenation and super call", true), "override fun toString(): String {\n    return \"Dog(\" +\n        \"name='$name', \" +\n        \"age=$age\" +\n        \")\" +\n        \" ${super.toString()}\"\n}");
        let fields = [f("name", "String"), f("ids", "int[]"), f("tags", "String[]")];
        let java = |t: &str| to_string(ImportStyle::Java, t, "Cat", &fields, true, "    ");
        let g = java("String concat (+) and super.toString()");
        assert!(g.members[0].ends_with("            \", tags=\" + Arrays.toString(tags) +\n            \"} \" + super.toString();\n}"), "{}", g.members[0]);
        let g = java("StringBuffer");
        assert_eq!(g.members[0], "@Override\npublic String toString() {\n    final StringBuffer sb = new StringBuffer(\"Cat{\");\n    sb.append(\"name='\").append(name).append('\\'');\n    sb.append(\", ids=\");\n    if (ids == null) sb.append(\"null\");\n    else {\n        sb.append('[');\n        for (int i = 0; i < ids.length; ++i)\n            sb.append(i == 0 ? \"\" : \", \").append(ids[i]);\n        sb.append(']');\n    }\n    sb.append(\", tags=\").append(tags == null ? \"null\" : Arrays.asList(tags).toString());\n    sb.append('}');\n    return sb.toString();\n}");
        assert_eq!(g.imports, ["java.util.Arrays"]);
        let g = java("StringJoiner (JDK 1.8)");
        assert_eq!(g.members[0], "@Override\npublic String toString() {\n    return new StringJoiner(\", \", Cat.class.getSimpleName() + \"[\", \"]\")\n            .add(\"name='\" + name + \"'\")\n            .add(\"ids=\" + Arrays.toString(ids))\n            .add(\"tags=\" + Arrays.toString(tags))\n            .toString();\n}");
        assert_eq!(g.imports, ["java.util.StringJoiner", "java.util.Arrays"]);
        let g = java("MoreObjects.toStringHelper (Guava 18+)");
        assert!(g.members[0].contains("    return MoreObjects.toStringHelper(this)\n            .add(\"name\", name)\n") && g.imports == ["com.google.common.base.MoreObjects"]);
    }

    #[test]
    fn constructors_call_the_super_constructor() {
        let fields = [f("name", "String"), f("age", "Int")];
        let supers = vec![vec!["name: String".to_owned(), "vararg tags: String".into()], vec![]];
        let got = constructors(ImportStyle::Kotlin, "Dog", &fields, &supers, "    ");
        assert_eq!(got[0], "constructor(name: String, vararg tags: String, name1: String, age: Int) : super(name, *tags) {\n    this.name = name1\n    this.age = age\n}");
        assert_eq!(got[1], "constructor(name: String, age: Int) {\n    this.name = name\n    this.age = age\n}");
        assert_eq!(constructors(ImportStyle::Kotlin, "Dog", &[], &supers[..1], "    "), ["constructor(name: String, vararg tags: String) : super(name, *tags)"]);
        let java = [f("lives", "int")];
        let got = constructors(ImportStyle::Java, "Cat", &java, &[vec!["String name".to_owned(), "int... ids".into()]], "    ");
        assert_eq!(got, ["public Cat(String name, int lives, int... ids) {\n    super(name, ids);\n    this.lives = lives;\n}"]);
        assert_eq!(constructors(ImportStyle::Java, "Cat", &java, &[], "    "), ["public Cat(int lives) {\n    this.lives = lives;\n}"]);
    }
}