//! Live Templates, as IntelliJ's: type an abbreviation and press Tab. The
//! built-in Kotlin and Java templates (from IntelliJ's Kotlin.xml and
//! Java.xml: names, texts, variables, contexts, descriptions), their
//! expansion re-indented as IntelliJ's reformat does, and the template
//! session: Tab / Enter from one variable to the next, a variable's copies
//! following as it is typed, Escape or the last Tab ending at `$END$`.
//! Variables IntelliJ computes from the code (`suggestIndexName()`,
//! `iterableComponentType(…)`) start from their usual values.

use std::borrow::Cow;
use std::ops::Range;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

type Text = Cow<'static, str>;

const fn s(text: &'static str) -> Text {
    Cow::Borrowed(text)
}

/// Where a template applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Context {
    /// A statement in a function body.
    Statement,
    /// An expression (in a function body).
    Expression,
    /// A class body: a member declaration.
    Class,
    /// Kotlin: the file's top level.
    TopLevel,
    /// Kotlin: an `object` body.
    Object,
    /// Java: after an `if`'s block (else-if).
    Else,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Var {
    pub name: Text,
    pub default: Text,
    /// Tab stops at it.
    pub stop: bool,
    /// Follows another variable (`escapeString(EXPR)`, `PARAM`).
    pub copy_of: Option<Text>,
    /// Worked out from the code where the template expands (IntelliJ's
    /// `className()`, `suggestIndexName()`, `variableOfType(…)`…); the
    /// default when nothing fits.
    pub computed: Option<Computed>,
}

/// What a variable's value is worked out from, as IntelliJ's template
/// expressions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Computed {
    /// `className()` / `kotlinClassName()`.
    ClassName,
    /// `methodName()` / `kotlinFunctionName()`.
    MethodName,
    /// Java soutp's `parameterInfo()`: `"a = " + a + ", b = " + b`.
    ParameterInfo,
    /// Kotlin soutp's `functionParameters()` script: `a = [${a}], b = [${b}]`.
    KotlinParameters,
    /// `suggestIndexName()`: `i`, `j`, `k`… not yet used here.
    IndexName,
    /// `variableOfType(…)` / `arrayVariable()` / `kotlinVariable()`: the
    /// nearest variable here of that kind.
    VariableOf(Kind),
    /// `componentTypeOf(V)` / `iterableComponentType(V)` /
    /// `guessElementType(V)`: the element type of variable V's value.
    ElementOf(&'static str),
    /// `typeParameterOf(V, "n")`: the nth type argument of variable V's type.
    TypeArgument(&'static str, usize),
}

/// The kinds of variable `variableOfType` looks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Any,
    Array,
    /// Arrays and collections.
    Iterable,
    List,
    Map,
    /// `double` and the other numbers.
    Number,
}

impl Kind {
    /// Whether a variable of `ty` (as written: `int[]`, `List<String>`,
    /// `IntArray`) is of this kind.
    pub fn matches(self, ty: &str) -> bool {
        let base = ty.split('<').next().unwrap_or(ty).trim().trim_end_matches('?');
        let base = base.rsplit('.').next().unwrap_or(base);
        let array = ty.trim_end().ends_with("[]") || base == "Array" || (base.ends_with("Array") && base.len() > 5 && base != "JSONArray");
        let collection = ["List", "Set", "Collection", "Iterable", "Queue", "Deque", "Sequence"].iter().any(|c| base.ends_with(c)) || base.ends_with("Vector");
        match self {
            Kind::Any => true,
            Kind::Array => array,
            Kind::Iterable => array || collection,
            Kind::List => base.ends_with("List"),
            Kind::Map => base.ends_with("Map"),
            Kind::Number => matches!(base, "int" | "long" | "double" | "float" | "short" | "byte" | "Int" | "Long" | "Double" | "Float" | "Short" | "Byte" | "Integer"),
        }
    }
}

/// What the code around a template's place declares: the enclosing class
/// and function, the function's parameters and the locals before the
/// place, nearest last, with their types as written (empty if unknown).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scope {
    pub class_name: String,
    pub method_name: String,
    pub parameters: Vec<(String, String)>,
    pub variables: Vec<(String, String)>,
    /// The class's fields (Kotlin properties), farthest of all.
    pub fields: Vec<(String, String)>,
}

/// The element type of a type as written: `int[]` → `int`,
/// `List<String>` → `String`, `IntArray` → `Int`.
pub fn element_type(ty: &str) -> Option<String> {
    let ty = ty.trim().trim_end_matches('?');
    if let Some(inner) = ty.strip_suffix("[]") {
        return Some(inner.trim().to_owned());
    }
    if let Some(args) = type_arguments(ty) {
        let last = if ty.split('<').next().is_some_and(|b| b.trim().ends_with("Map")) { args.get(1) } else { args.last() };
        return last.map(|a| a.trim_start_matches("out ").trim_start_matches("? extends ").trim().to_owned());
    }
    let base = ty.rsplit('.').next().unwrap_or(ty);
    match base {
        "IntArray" | "LongArray" | "ShortArray" | "ByteArray" | "CharArray" | "FloatArray" | "DoubleArray" | "BooleanArray" => Some(base.trim_end_matches("Array").to_owned()),
        _ => None,
    }
}

/// The type arguments of `A<B, C<D>>`: [B, C<D>].
fn type_arguments(ty: &str) -> Option<Vec<String>> {
    let open = ty.find('<')?;
    let inner = ty[open + 1..].strip_suffix('>')?;
    let mut out = Vec::new();
    let (mut depth, mut start) = (0, 0);
    for (i, c) in inner.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => {
                out.push(inner[start..i].trim().to_owned());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(inner[start..].trim().to_owned());
    Some(out)
}

impl Scope {
    /// Every variable here, nearest first.
    fn nearest(&self) -> impl Iterator<Item = &(String, String)> {
        self.variables.iter().rev().chain(self.parameters.iter().rev()).chain(self.fields.iter())
    }

    fn type_of(&self, name: &str) -> Option<&str> {
        self.nearest().find(|(n, _)| n == name).map(|(_, t)| t.as_str())
    }
}

/// The template with its computed variables' defaults worked out for
/// `scope` (those that find nothing keep theirs).
pub fn resolved(template: &Template, scope: &Scope) -> Template {
    let mut values: Vec<(String, String)> = Vec::new();
    let mut vars: Vec<Var> = Vec::new();
    for v in template.vars.iter() {
        let value = v.computed.and_then(|c| compute(c, scope, &values));
        let default = value.map(Cow::Owned).unwrap_or_else(|| v.default.clone());
        values.push((v.name.to_string(), default.to_string()));
        vars.push(Var { default, ..v.clone() });
    }
    Template { vars: Cow::Owned(vars), ..template.clone() }
}

fn compute(c: Computed, scope: &Scope, values: &[(String, String)]) -> Option<String> {
    let value_of = |name: &str| values.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str());
    let non_empty = |s: &str| (!s.is_empty()).then(|| s.to_owned());
    match c {
        Computed::ClassName => non_empty(&scope.class_name),
        Computed::MethodName => non_empty(&scope.method_name),
        Computed::ParameterInfo => {
            let parts: Vec<String> = scope.parameters.iter().enumerate().map(|(i, (n, _))| format!("\"{}{n} = \" + {n}", if i == 0 { "" } else { ", " })).collect();
            Some(parts.join(" + "))
        }
        Computed::KotlinParameters => Some(scope.parameters.iter().map(|(n, _)| format!("{n} = [${{{n}}}]")).collect::<Vec<_>>().join(", ")),
        Computed::IndexName => {
            let used = |n: &str| scope.nearest().any(|(v, _)| v == n);
            ["i", "j", "k", "l", "m", "n"].into_iter().find(|n| !used(n)).map(str::to_owned)
        }
        Computed::VariableOf(kind) => scope.nearest().find(|(_, t)| kind == Kind::Any || kind.matches(t)).map(|(n, _)| n.clone()),
        Computed::ElementOf(var) => {
            let ty = scope.type_of(value_of(var)?)?;
            element_type(ty)
        }
        Computed::TypeArgument(var, n) => type_arguments(scope.type_of(value_of(var)?)?)?.get(n).cloned(),
    }
}


#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Template {
    pub name: Text,
    pub description: Text,
    pub value: Text,
    pub vars: Cow<'static, [Var]>,
    pub contexts: Cow<'static, [Context]>,
}

const KOTLIN: &[Template] = &[
    Template { name: s("main"), description: s("main() function"), value: s("fun main($ARGS$) {\n  $END$\n}"), vars: Cow::Borrowed(&[Var { name: s("ARGS"), default: s(""), stop: false, copy_of: None, computed: None }]), contexts: Cow::Borrowed(&[Context::TopLevel]) },
    Template { name: s("maina"), description: s("main(args) function"), value: s("fun main(args: Array<String>) {\n  $END$\n}"), vars: Cow::Borrowed(&[]), contexts: Cow::Borrowed(&[Context::TopLevel]) },
    Template { name: s("maino"), description: s("main(args) function"), value: s("@JvmStatic\nfun main(args: Array<String>) {\n  $END$\n}"), vars: Cow::Borrowed(&[]), contexts: Cow::Borrowed(&[Context::Object]) },
    Template { name: s("sout"), description: s("Prints a string to System.out"), value: s("println($END$)"), vars: Cow::Borrowed(&[]), contexts: Cow::Borrowed(&[Context::Statement]) },
    Template { name: s("serr"), description: s("Prints a string to System.err"), value: s("System.err.println($END$)"), vars: Cow::Borrowed(&[]), contexts: Cow::Borrowed(&[Context::Statement]) },
    Template { name: s("soutv"), description: s("Prints a value to System.out"), value: s("println(\"$EXPR_COPY$ = $DOLLAR${$EXPR$}\")"), vars: Cow::Borrowed(&[Var { name: s("DOLLAR"), default: s("$"), stop: false, copy_of: None, computed: None }, Var { name: s("EXPR"), default: s("expr"), stop: true, copy_of: None, computed: None }, Var { name: s("EXPR_COPY"), default: s(""), stop: false, copy_of: Some(s("EXPR")), computed: None }]), contexts: Cow::Borrowed(&[Context::Statement]) },
    Template { name: s("iter"), description: s("Iterate over elements of iterable (for-in loop)"), value: s("for ($VAR$ in $ITERABLE$) {\n$END$\n}"), vars: Cow::Borrowed(&[Var { name: s("ITERABLE"), default: s("iterable"), stop: true, copy_of: None, computed: Some(Computed::VariableOf(Kind::Iterable)) }, Var { name: s("VAR"), default: s("i"), stop: true, copy_of: None, computed: None }]), contexts: Cow::Borrowed(&[Context::Statement]) },
    Template { name: s("ifn"), description: s("Inserts 'if null' expression"), value: s("if ($VAR$ == null) {\n$END$\n}"), vars: Cow::Borrowed(&[Var { name: s("VAR"), default: s("i"), stop: true, copy_of: None, computed: Some(Computed::VariableOf(Kind::Any)) }]), contexts: Cow::Borrowed(&[Context::Expression]) },
    Template { name: s("inn"), description: s("Inserts 'if not null' expression"), value: s("if ($VAR$ != null) {\n$END$\n}"), vars: Cow::Borrowed(&[Var { name: s("VAR"), default: s("i"), stop: true, copy_of: None, computed: Some(Computed::VariableOf(Kind::Any)) }]), contexts: Cow::Borrowed(&[Context::Expression]) },
    Template { name: s("void"), description: s("Function returning nothing"), value: s("fun $NAME$($PARAMS$) {\n$END$\n}"), vars: Cow::Borrowed(&[Var { name: s("NAME"), default: s(""), stop: true, copy_of: None, computed: None }, Var { name: s("PARAMS"), default: s(""), stop: true, copy_of: None, computed: None }]), contexts: Cow::Borrowed(&[Context::Class, Context::Statement, Context::TopLevel]) },
    Template { name: s("fun0"), description: s("Function with no parameters"), value: s("fun $NAME$() : $RETURN$ {\n$END$\n}"), vars: Cow::Borrowed(&[Var { name: s("NAME"), default: s(""), stop: true, copy_of: None, computed: None }, Var { name: s("RETURN"), default: s("Unit"), stop: true, copy_of: None, computed: None }]), contexts: Cow::Borrowed(&[Context::Class, Context::Statement, Context::TopLevel]) },
    Template { name: s("fun1"), description: s("Function with one parameter"), value: s("fun $NAME$($PARAM1$ : $PARAM1TYPE$) : $RETURN$ {\n$END$\n}"), vars: Cow::Borrowed(&[Var { name: s("NAME"), default: s(""), stop: true, copy_of: None, computed: None }, Var { name: s("PARAM1"), default: s("x"), stop: true, copy_of: None, computed: None }, Var { name: s("PARAM1TYPE"), default: s("Any"), stop: true, copy_of: None, computed: None }, Var { name: s("RETURN"), default: s("Unit"), stop: true, copy_of: None, computed: None }]), contexts: Cow::Borrowed(&[Context::Class, Context::Statement, Context::TopLevel]) },
    Template { name: s("fun2"), description: s("Function with two parameters"), value: s("fun $NAME$($PARAM1$ : $PARAM1TYPE$, $PARAM2$ : $PARAM2TYPE$) : $RETURN$ {\n$END$\n}"), vars: Cow::Borrowed(&[Var { name: s("NAME"), default: s(""), stop: true, copy_of: None, computed: None }, Var { name: s("PARAM1"), default: s("x"), stop: true, copy_of: None, computed: None }, Var { name: s("PARAM1TYPE"), default: s("Any"), stop: true, copy_of: None, computed: None }, Var { name: s("PARAM2"), default: s("y"), stop: true, copy_of: None, computed: None }, Var { name: s("PARAM2TYPE"), default: s("Any"), stop: true, copy_of: None, computed: None }, Var { name: s("RETURN"), default: s("Unit"), stop: true, copy_of: None, computed: None }]), contexts: Cow::Borrowed(&[Context::Class, Context::Statement, Context::TopLevel]) },
    Template { name: s("interface"), description: s("Interface"), value: s("interface $NAME$ {\n$END$\n}"), vars: Cow::Borrowed(&[Var { name: s("NAME"), default: s(""), stop: true, copy_of: None, computed: None }]), contexts: Cow::Borrowed(&[Context::Class, Context::Statement, Context::TopLevel]) },
    Template { name: s("singleton"), description: s("Singleton"), value: s("object $NAME$ {\n$END$\n}"), vars: Cow::Borrowed(&[Var { name: s("NAME"), default: s(""), stop: true, copy_of: None, computed: None }]), contexts: Cow::Borrowed(&[Context::Class, Context::Statement, Context::TopLevel]) },
    Template { name: s("closure"), description: s("Closure (function without name)"), value: s("{$PARAM$ -> $PARAM_COPY$}"), vars: Cow::Borrowed(&[Var { name: s("PARAM"), default: s("x"), stop: true, copy_of: None, computed: None }, Var { name: s("PARAM_COPY"), default: s(""), stop: false, copy_of: Some(s("PARAM")), computed: None }]), contexts: Cow::Borrowed(&[Context::Expression, Context::Statement]) },
    Template { name: s("anonymous"), description: s("Anonymous class"), value: s("object : $SUPERTYPE$ {\n$END$\n}"), vars: Cow::Borrowed(&[Var { name: s("SUPERTYPE"), default: s("Object"), stop: true, copy_of: None, computed: None }]), contexts: Cow::Borrowed(&[Context::Expression, Context::Statement]) },
    Template { name: s("exfun"), description: s("Extension function"), value: s("fun $RECEIVER$.$NAME$($PARAMS$) : $RETURN$ {\n$END$\n}"), vars: Cow::Borrowed(&[Var { name: s("RECEIVER"), default: s("Any"), stop: true, copy_of: None, computed: None }, Var { name: s("NAME"), default: s("f"), stop: true, copy_of: None, computed: None }, Var { name: s("PARAMS"), default: s(""), stop: true, copy_of: None, computed: None }, Var { name: s("RETURN"), default: s("Unit"), stop: true, copy_of: None, computed: None }]), contexts: Cow::Borrowed(&[Context::Class, Context::TopLevel]) },
    Template { name: s("exval"), description: s("Extension read-only property"), value: s("val $RECEIVER$.$NAME$ : $TYPE$\nget() {\n$END$\n}"), vars: Cow::Borrowed(&[Var { name: s("RECEIVER"), default: s("Any"), stop: true, copy_of: None, computed: None }, Var { name: s("NAME"), default: s("v"), stop: true, copy_of: None, computed: None }, Var { name: s("TYPE"), default: s("Any"), stop: true, copy_of: None, computed: None }]), contexts: Cow::Borrowed(&[Context::Class, Context::TopLevel]) },
    Template { name: s("exvar"), description: s("Extension read-write property"), value: s("var $RECEIVER$.$NAME$ : $TYPE$\nget() {\n$END$\n}\nset(value) {\n \n}"), vars: Cow::Borrowed(&[Var { name: s("RECEIVER"), default: s("Any"), stop: true, copy_of: None, computed: None }, Var { name: s("NAME"), default: s("v"), stop: true, copy_of: None, computed: None }, Var { name: s("TYPE"), default: s("Any"), stop: true, copy_of: None, computed: None }]), contexts: Cow::Borrowed(&[Context::Class, Context::TopLevel]) },
    Template { name: s("psvm"), description: s("main() function"), value: s("fun main($ARGS$) {\n  $END$\n}"), vars: Cow::Borrowed(&[Var { name: s("ARGS"), default: s(""), stop: false, copy_of: None, computed: None }]), contexts: Cow::Borrowed(&[Context::TopLevel]) },
    Template { name: s("psvma"), description: s("main(args) function"), value: s("fun main(args: Array<String>) {\n  $END$\n}"), vars: Cow::Borrowed(&[]), contexts: Cow::Borrowed(&[Context::TopLevel]) },
    Template { name: s("psvmo"), description: s("main(args) function"), value: s("@JvmStatic\nfun main(args: Array<String>) {\n  $END$\n}"), vars: Cow::Borrowed(&[]), contexts: Cow::Borrowed(&[Context::Object]) },
    Template { name: s("object"), description: s("Anonymous class"), value: s("object : $SUPERTYPE$ {\n$END$\n}"), vars: Cow::Borrowed(&[Var { name: s("SUPERTYPE"), default: s("Object"), stop: true, copy_of: None, computed: None }]), contexts: Cow::Borrowed(&[Context::Expression, Context::Statement]) },
    Template { name: s("soutf"), description: s("Prints current class and function name to System.out"), value: s("println(\"$CLASS_NAME$.$FUNCTION_NAME$\")"), vars: Cow::Borrowed(&[Var { name: s("CLASS_NAME"), default: s(""), stop: false, copy_of: None, computed: Some(Computed::ClassName) }, Var { name: s("FUNCTION_NAME"), default: s(""), stop: false, copy_of: None, computed: Some(Computed::MethodName) }]), contexts: Cow::Borrowed(&[Context::Statement]) },
    Template { name: s("soutp"), description: s("Prints function parameter names and values to System.out"), value: s("println(\"$FORMAT$\")"), vars: Cow::Borrowed(&[Var { name: s("FORMAT"), default: s(""), stop: false, copy_of: None, computed: Some(Computed::KotlinParameters) }]), contexts: Cow::Borrowed(&[Context::Statement]) },
];
const JAVA: &[Template] = &[
    Template { name: s("sout"), description: s("Prints a string to System.out"), value: s("System.out.println($END$);"), vars: Cow::Borrowed(&[]), contexts: Cow::Borrowed(&[Context::Statement]) },
    Template { name: s("serr"), description: s("Prints a string to System.err"), value: s("System.err.println($END$);"), vars: Cow::Borrowed(&[]), contexts: Cow::Borrowed(&[Context::Statement]) },
    Template { name: s("souf"), description: s("Prints a formatted string to System.out"), value: s("System.out.printf(\"$END$\");"), vars: Cow::Borrowed(&[]), contexts: Cow::Borrowed(&[Context::Statement]) },
    Template { name: s("soutv"), description: s("Prints a value to System.out"), value: s("System.out.println(\"$EXPR_COPY$ = \" + $EXPR$);"), vars: Cow::Borrowed(&[Var { name: s("EXPR"), default: s("expr"), stop: true, copy_of: None, computed: None }, Var { name: s("EXPR_COPY"), default: s(""), stop: false, copy_of: Some(s("EXPR")), computed: None }]), contexts: Cow::Borrowed(&[Context::Statement]) },
    Template { name: s("psvm"), description: s("main() method declaration"), value: s("public static void main(String[] args){\n  $END$\n}"), vars: Cow::Borrowed(&[]), contexts: Cow::Borrowed(&[Context::Class]) },
    Template { name: s("fori"), description: s("Create iteration loop"), value: s("for(int $INDEX$ = 0; $INDEX$ < $LIMIT$; $INDEX$++) {\n  $END$\n}"), vars: Cow::Borrowed(&[Var { name: s("INDEX"), default: s("i"), stop: true, copy_of: None, computed: Some(Computed::IndexName) }, Var { name: s("LIMIT"), default: s(""), stop: true, copy_of: None, computed: None }]), contexts: Cow::Borrowed(&[Context::Statement]) },
    Template { name: s("itar"), description: s("Iterate elements of array"), value: s("for(int $INDEX$ = 0; $INDEX$ < $ARRAY$.length; $INDEX$++) {\n  $ELEMENT_TYPE$ $VAR$ = $ARRAY$[$INDEX$];\n  $END$\n}"), vars: Cow::Borrowed(&[Var { name: s("INDEX"), default: s("i"), stop: true, copy_of: None, computed: Some(Computed::IndexName) }, Var { name: s("ARRAY"), default: s("array"), stop: true, copy_of: None, computed: Some(Computed::VariableOf(Kind::Array)) }, Var { name: s("ELEMENT_TYPE"), default: s("Object"), stop: true, copy_of: None, computed: Some(Computed::ElementOf("ARRAY")) }, Var { name: s("VAR"), default: s("o"), stop: true, copy_of: None, computed: None }]), contexts: Cow::Borrowed(&[Context::Statement]) },
    Template { name: s("iter"), description: s("Iterate Iterable or array"), value: s("for ($ELEMENT_TYPE$ $VAR$ : $ITERABLE_TYPE$) {\n  $END$\n}"), vars: Cow::Borrowed(&[Var { name: s("ITERABLE_TYPE"), default: s("iterable"), stop: true, copy_of: None, computed: Some(Computed::VariableOf(Kind::Iterable)) }, Var { name: s("ELEMENT_TYPE"), default: s("Object"), stop: true, copy_of: None, computed: Some(Computed::ElementOf("ITERABLE_TYPE")) }, Var { name: s("VAR"), default: s("o"), stop: true, copy_of: None, computed: None }]), contexts: Cow::Borrowed(&[Context::Statement]) },
    Template { name: s("ifn"), description: s("Inserts 'if null' statement"), value: s("if ($VAR$ == null) {\n$END$\n}"), vars: Cow::Borrowed(&[Var { name: s("VAR"), default: s("var"), stop: true, copy_of: None, computed: Some(Computed::VariableOf(Kind::Any)) }]), contexts: Cow::Borrowed(&[Context::Statement]) },
    Template { name: s("inn"), description: s("Inserts 'if not null' statement"), value: s("if ($VAR$ != null) {\n$END$\n}"), vars: Cow::Borrowed(&[Var { name: s("VAR"), default: s("var"), stop: true, copy_of: None, computed: Some(Computed::VariableOf(Kind::Any)) }]), contexts: Cow::Borrowed(&[Context::Statement]) },
    Template { name: s("St"), description: s("String"), value: s("String "), vars: Cow::Borrowed(&[]), contexts: Cow::Borrowed(&[Context::Class, Context::Statement]) },
    Template { name: s("psf"), description: s("public static final"), value: s("public static final "), vars: Cow::Borrowed(&[]), contexts: Cow::Borrowed(&[Context::Class]) },
    Template { name: s("prsf"), description: s("private static final"), value: s("private static final "), vars: Cow::Borrowed(&[]), contexts: Cow::Borrowed(&[Context::Class]) },
    Template { name: s("psfi"), description: s("public static final int"), value: s("public static final int "), vars: Cow::Borrowed(&[]), contexts: Cow::Borrowed(&[Context::Class]) },
    Template { name: s("psfs"), description: s("public static final String"), value: s("public static final String "), vars: Cow::Borrowed(&[]), contexts: Cow::Borrowed(&[Context::Class]) },
    Template { name: s("thr"), description: s("throw new"), value: s("throw new "), vars: Cow::Borrowed(&[]), contexts: Cow::Borrowed(&[Context::Statement]) },
    Template { name: s("else-if"), description: s("Add else-if branch"), value: s("else if($CONDITION$){\n$END$\n} "), vars: Cow::Borrowed(&[Var { name: s("CONDITION"), default: s(""), stop: true, copy_of: None, computed: None }]), contexts: Cow::Borrowed(&[Context::Else]) },
    Template { name: s("C"), description: s("Surround with Callable"), value: s("java.util.concurrent.Callable<$RET$> callable = new java.util.concurrent.Callable<$RET$>() {\n  public $RET$ call() throws Exception {\n    $SELECTION$\n    $END$ \n  }\n};"), vars: Cow::Borrowed(&[Var { name: s("RET"), default: s("java.lang.Object"), stop: true, copy_of: None, computed: None }]), contexts: Cow::Borrowed(&[Context::Statement]) },
    Template { name: s("RL"), description: s("Surround with ReadWriteLock.readLock"), value: s("$LOCK$.readLock().lock();\ntry { \n  $SELECTION$\n} finally {\n  $LOCK$.readLock().unlock();\n}\n"), vars: Cow::Borrowed(&[Var { name: s("LOCK"), default: s(""), stop: true, copy_of: None, computed: None }]), contexts: Cow::Borrowed(&[Context::Statement]) },
    Template { name: s("WL"), description: s("Surround with ReadWriteLock.writeLock"), value: s("$LOCK$.writeLock().lock();\ntry { \n  $SELECTION$\n} finally {\n  $LOCK$.writeLock().unlock();\n}\n"), vars: Cow::Borrowed(&[Var { name: s("LOCK"), default: s(""), stop: true, copy_of: None, computed: None }]), contexts: Cow::Borrowed(&[Context::Statement]) },
    Template { name: s("I"), description: s("Iterate Iterable or array"), value: s("for ($ELEMENT_TYPE$ $VAR$ : $SELECTION$) {\n  $END$\n}\n"), vars: Cow::Borrowed(&[Var { name: s("ELEMENT_TYPE"), default: s("java.lang.Object"), stop: false, copy_of: None, computed: None }, Var { name: s("VAR"), default: s(""), stop: true, copy_of: None, computed: None }]), contexts: Cow::Borrowed(&[Context::Statement]) },
    Template { name: s("soutm"), description: s("Prints current class and method names to System.out"), value: s("System.out.println(\"$CLASS_NAME$.$METHOD_NAME$\");"), vars: Cow::Borrowed(&[Var { name: s("CLASS_NAME"), default: s(""), stop: false, copy_of: None, computed: Some(Computed::ClassName) }, Var { name: s("METHOD_NAME"), default: s(""), stop: false, copy_of: None, computed: Some(Computed::MethodName) }]), contexts: Cow::Borrowed(&[Context::Statement]) },
    Template { name: s("soutp"), description: s("Prints method parameter names and values to System.out"), value: s("System.out.println($FORMAT$);"), vars: Cow::Borrowed(&[Var { name: s("FORMAT"), default: s(""), stop: false, copy_of: None, computed: Some(Computed::ParameterInfo) }]), contexts: Cow::Borrowed(&[Context::Statement]) },
    Template { name: s("ritar"), description: s("Iterate elements of array in reverse order"), value: s("for(int $INDEX$ = $ARRAY$.length - 1; $INDEX$ >= 0; $INDEX$--) {\n  $ELEMENT_TYPE$ $VAR$ = $ARRAY$[$INDEX$];\n  $END$\n}"), vars: Cow::Borrowed(&[Var { name: s("INDEX"), default: s("i"), stop: true, copy_of: None, computed: Some(Computed::IndexName) }, Var { name: s("ARRAY"), default: s("array"), stop: true, copy_of: None, computed: Some(Computed::VariableOf(Kind::Array)) }, Var { name: s("ELEMENT_TYPE"), default: s("Object"), stop: false, copy_of: None, computed: Some(Computed::ElementOf("ARRAY")) }, Var { name: s("VAR"), default: s("o"), stop: true, copy_of: None, computed: None }]), contexts: Cow::Borrowed(&[Context::Statement]) },
    Template { name: s("itli"), description: s("Iterate elements of java.util.List"), value: s("for (int $INDEX$ = 0; $INDEX$ < $LIST$.size(); $INDEX$++) {\n  $ELEMENT_TYPE$ $VAR$ = $LIST$.get($INDEX$);\n  $END$\n}"), vars: Cow::Borrowed(&[Var { name: s("INDEX"), default: s("i"), stop: true, copy_of: None, computed: Some(Computed::IndexName) }, Var { name: s("LIST"), default: s("list"), stop: true, copy_of: None, computed: Some(Computed::VariableOf(Kind::List)) }, Var { name: s("ELEMENT_TYPE"), default: s("Object"), stop: true, copy_of: None, computed: Some(Computed::ElementOf("LIST")) }, Var { name: s("VAR"), default: s("o"), stop: true, copy_of: None, computed: None }]), contexts: Cow::Borrowed(&[Context::Statement]) },
    Template { name: s("itm"), description: s("Iterate entries of a map"), value: s("for (java.util.Map.Entry<$KEY_TYPE$, $VALUE_TYPE$> $ENTRY$: $MAP$.entrySet()) {\n  $KEY_TYPE$ $KEY$ = $ENTRY$.getKey();\n  $VALUE_TYPE$ $VALUE$ = $ENTRY$.getValue();\n  $END$\n}"), vars: Cow::Borrowed(&[Var { name: s("MAP"), default: s("map"), stop: true, copy_of: None, computed: Some(Computed::VariableOf(Kind::Map)) }, Var { name: s("KEY_TYPE"), default: s("Object"), stop: false, copy_of: None, computed: Some(Computed::TypeArgument("MAP", 0)) }, Var { name: s("VALUE_TYPE"), default: s("Object"), stop: false, copy_of: None, computed: Some(Computed::TypeArgument("MAP", 1)) }, Var { name: s("ENTRY"), default: s("entry"), stop: true, copy_of: None, computed: None }, Var { name: s("KEY"), default: s("key"), stop: true, copy_of: None, computed: None }, Var { name: s("VALUE"), default: s("value"), stop: true, copy_of: None, computed: None }]), contexts: Cow::Borrowed(&[Context::Statement]) },
    Template { name: s("lst"), description: s("Get the last element of an array"), value: s("$ARRAY$[$ARRAY$.length - 1]"), vars: Cow::Borrowed(&[Var { name: s("ARRAY"), default: s("array"), stop: true, copy_of: None, computed: Some(Computed::VariableOf(Kind::Array)) }]), contexts: Cow::Borrowed(&[Context::Expression]) },
    Template { name: s("mn"), description: s("Set lesser value to a variable"), value: s("$VAR$ = Math.min($VAR$, $END$);"), vars: Cow::Borrowed(&[Var { name: s("VAR"), default: s(""), stop: true, copy_of: None, computed: Some(Computed::VariableOf(Kind::Number)) }]), contexts: Cow::Borrowed(&[Context::Statement]) },
    Template { name: s("mx"), description: s("Set greater value to a variable"), value: s("$VAR$ = Math.max($VAR$, $END$);"), vars: Cow::Borrowed(&[Var { name: s("VAR"), default: s(""), stop: true, copy_of: None, computed: Some(Computed::VariableOf(Kind::Number)) }]), contexts: Cow::Borrowed(&[Context::Statement]) },
    Template { name: s("geti"), description: s("Inserts singleton method getInstance"), value: s("public static $CLASS_NAME$ getInstance() {\n  return $VALUE$;\n}"), vars: Cow::Borrowed(&[Var { name: s("CLASS_NAME"), default: s(""), stop: false, copy_of: None, computed: Some(Computed::ClassName) }, Var { name: s("VALUE"), default: s("null"), stop: true, copy_of: None, computed: None }]), contexts: Cow::Borrowed(&[Context::Class]) },
];

/// The built-in templates of a language: `kotlin` or Java.
pub fn builtin(kotlin: bool) -> &'static [Template] {
    if kotlin { KOTLIN } else { JAVA }
}

/// A template the person made (Settings › Editor › Live Templates, the
/// "user" group, as IntelliJ's): its text's `$NAME$` variables are tab
/// stops in order, a repeated one following the first; it applies in
/// the Kotlin and Java contexts ticked.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct UserTemplate {
    pub abbreviation: String,
    #[serde(default)]
    pub description: String,
    pub text: String,
    /// Default values by variable name.
    #[serde(default)]
    pub defaults: Vec<(String, String)>,
    #[serde(default)]
    pub kotlin: Vec<Context>,
    #[serde(default)]
    pub java: Vec<Context>,
    #[serde(default = "enabled")]
    pub enabled: bool,
}

fn enabled() -> bool {
    true
}

/// The `$NAME$` variables of a template's text in order, once each
/// (`$END$` and `$SELECTION$` aside).
pub fn variables(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find('$') {
        let after = &rest[at + 1..];
        let Some(close) = after.find('$') else { break };
        let name = &after[..close];
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            rest = after;
            continue;
        }
        if name != "END" && name != "SELECTION" && !out.iter().any(|n| n == name) {
            out.push(name.to_owned());
        }
        rest = &after[close + 1..];
    }
    out
}

impl UserTemplate {
    /// As a template of a language, when it applies in any of its contexts.
    pub fn template(&self, kotlin: bool) -> Option<Template> {
        let contexts = if kotlin { &self.kotlin } else { &self.java };
        if !self.enabled || contexts.is_empty() || self.abbreviation.is_empty() {
            return None;
        }
        let vars = variables(&self.text)
            .into_iter()
            .map(|name| {
                let default = self.defaults.iter().find(|(n, _)| *n == name).map(|(_, d)| d.clone()).unwrap_or_default();
                Var { name: Cow::Owned(name), default: Cow::Owned(default), stop: true, copy_of: None, computed: None }
            })
            .collect::<Vec<_>>();
        Some(Template {
            name: Cow::Owned(self.abbreviation.clone()),
            description: Cow::Owned(self.description.clone()),
            value: Cow::Owned(self.text.clone()),
            vars: Cow::Owned(vars),
            contexts: Cow::Owned(contexts.clone()),
        })
    }
}

/// The person's templates and the built-in ones turned off
/// (`Kotlin/sout`, `Java/fori`).
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CustomTemplates {
    #[serde(default)]
    pub user: Vec<UserTemplate>,
    #[serde(default)]
    pub disabled: Vec<String>,
}

impl CustomTemplates {
    /// A built-in template's key in `disabled`.
    pub fn key(kotlin: bool, name: &str) -> String {
        format!("{}/{name}", if kotlin { "Kotlin" } else { "Java" })
    }

    pub fn is_enabled(&self, kotlin: bool, name: &str) -> bool {
        !self.disabled.contains(&Self::key(kotlin, name))
    }
}

static CUSTOM: RwLock<CustomTemplates> = RwLock::new(CustomTemplates { user: Vec::new(), disabled: Vec::new() });

/// The templates settings, as saved: what the editor expands from now on.
pub fn set_custom(custom: CustomTemplates) {
    *CUSTOM.write().unwrap() = custom;
}

pub fn custom() -> CustomTemplates {
    CUSTOM.read().unwrap().clone()
}

/// The templates of a language that are on: the person's first (one
/// can take a built-in's abbreviation), then the built-in ones.
pub fn templates(kotlin: bool) -> Vec<Template> {
    templates_in(&CUSTOM.read().unwrap(), kotlin)
}

fn templates_in(custom: &CustomTemplates, kotlin: bool) -> Vec<Template> {
    let mut out: Vec<Template> = custom.user.iter().filter_map(|u| u.template(kotlin)).collect();
    out.extend(builtin(kotlin).iter().filter(|t| custom.is_enabled(kotlin, &t.name)).cloned());
    out
}

impl Template {
    /// A surround template (`$SELECTION$` in it): it wraps the selection
    /// (Code › Surround With… / Surround with Live Template) and doesn't
    /// expand from its abbreviation.
    pub fn surrounds(&self) -> bool {
        self.value.contains("$SELECTION$")
    }
}

/// The template an abbreviation expands to here.
pub fn find(kotlin: bool, abbreviation: &str, contexts: &[Context]) -> Option<Template> {
    templates(kotlin).into_iter().find(|t| t.name == abbreviation && !t.surrounds() && t.contexts.iter().any(|c| contexts.contains(c)))
}

/// The surround templates of a language that are on, once per
/// abbreviation (the person's first).
pub fn surrounding(kotlin: bool) -> Vec<Template> {
    let mut out: Vec<Template> = Vec::new();
    for t in templates(kotlin) {
        if t.surrounds() && !out.iter().any(|o| o.name == t.name) {
            out.push(t);
        }
    }
    out
}

/// IntelliJ's toShortenFQNames: each qualified class name in the
/// template's text and defaults (`java.util.concurrent.Callable`) becomes
/// its simple name; the names to import (none for `java.lang` and
/// `kotlin`).
pub fn shortened(template: &Template) -> (Template, Vec<String>) {
    static FQN: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\b(?:java|javax|kotlin|android|androidx)(?:\.[a-z_][a-z0-9_]*)*\.([A-Z][A-Za-z0-9_]*)\b").unwrap());
    let mut imports: Vec<String> = Vec::new();
    let mut short = |text: &str| -> String {
        FQN.replace_all(text, |c: &regex::Captures| {
            let qualified = c[0].to_owned();
            let package = &qualified[..qualified.len() - c[1].len() - 1];
            if package != "java.lang" && package != "kotlin" && !imports.contains(&qualified) {
                imports.push(qualified.clone());
            }
            c[1].to_owned()
        })
        .into_owned()
    };
    let value = short(&template.value);
    let vars: Vec<Var> = template.vars.iter().map(|v| Var { default: Cow::Owned(short(&v.default)), ..v.clone() }).collect();
    (Template { value: Cow::Owned(value), vars: Cow::Owned(vars), ..template.clone() }, imports)
}

/// The templates code completion offers for the word typed before the
/// caret, as IntelliJ's `LiveTemplateCompletionContributor`: those that
/// apply here whose abbreviation starts with it (`so` → `sout`, `soutv`).
/// Nothing for an empty word.
pub fn completing(kotlin: bool, typed: &str, contexts: &[Context]) -> Vec<Template> {
    if typed.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<Template> = Vec::new();
    for t in templates(kotlin) {
        if t.name.starts_with(typed) && !t.surrounds() && t.contexts.iter().any(|c| contexts.contains(c)) && !out.iter().any(|o| o.name == t.name) {
            out.push(t);
        }
    }
    out
}

/// Whether any template's abbreviation starts with the word: when none
/// does, completion needn't work out where the caret is.
pub fn any_starts_with(kotlin: bool, typed: &str) -> bool {
    !typed.is_empty() && templates(kotlin).iter().any(|t| t.name.starts_with(typed) && !t.surrounds())
}

/// The completion command that expands the template just completed.
pub const EXPAND_COMMAND: &str = "junction.expandLiveTemplate";

/// A template written out: its text, each tab stop's ranges (a variable
/// with its copies), and where `$END$` is.
#[derive(Debug, PartialEq, Eq)]
pub struct Expansion {
    pub text: String,
    pub stops: Vec<Vec<Range<usize>>>,
    pub end: usize,
}

/// Writes a template out at a line indented `indent`: each following line
/// indented by its brace depth in steps of `unit`.
pub fn expand(template: &Template, indent: &str, unit: &str) -> Expansion {
    expand_around(template, "", indent, unit)
}

/// `expand` with `selection` at `$SELECTION$`: its lines keep their
/// indentation relative to each other, at the template's level there.
pub fn expand_around(template: &Template, selection: &str, indent: &str, unit: &str) -> Expansion {
    let selection_lines: Vec<&str> = selection.trim_end_matches('\n').split('\n').collect();
    let base = selection_lines.iter().filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    let value_of = |name: &str| -> &str {
        let var = template.vars.iter().find(|v| v.name == name);
        let source = var.and_then(|v| v.copy_of.as_deref()).and_then(|c| template.vars.iter().find(|v| v.name == c));
        source.or(var).map_or("", |v| &v.default)
    };
    let mut out = String::new();
    let mut ranges: Vec<(&str, Range<usize>)> = Vec::new();
    let mut end = None;
    let mut depth: usize = 0;
    for (n, line) in template.value.split('\n').enumerate() {
        let line = if n == 0 { line } else { line.trim_start() };
        let closing = line.starts_with('}');
        let level = if closing { depth.saturating_sub(1) } else { depth };
        if n > 0 {
            out.push('\n');
            if !line.trim().is_empty() || line.contains("$END$") {
                out.push_str(indent);
                out.push_str(&unit.repeat(level));
            }
        }
        let mut rest = line;
        while let Some(at) = rest.find('$') {
            let after = &rest[at + 1..];
            let Some(close) = after.find('$') else { break };
            let name = &after[..close];
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                out.push_str(&rest[..at + 1]);
                rest = after;
                continue;
            }
            out.push_str(&rest[..at]);
            match name {
                "END" => end = Some(out.len()),
                "SELECTION" => {
                    for (i, l) in selection_lines.iter().enumerate() {
                        let l = l.get(base..).unwrap_or(l.trim_start());
                        if i > 0 {
                            out.push('\n');
                            if !l.trim().is_empty() {
                                out.push_str(indent);
                                out.push_str(&unit.repeat(if n == 0 { 0 } else { level }));
                            }
                        }
                        out.push_str(if i == 0 { l.trim_start() } else { l });
                    }
                }
                _ => {
                    let start = out.len();
                    out.push_str(value_of(name));
                    ranges.push((name, start..out.len()));
                }
            }
            rest = &after[close + 1..];
        }
        out.push_str(rest);
        depth = (depth + line.matches('{').count()).saturating_sub(line.matches('}').count());
    }
    let stops = template
        .vars
        .iter()
        .filter(|v| v.stop)
        .map(|v| {
            let copies: Vec<&str> = template.vars.iter().filter(|c| c.copy_of.as_deref() == Some(&*v.name)).map(|c| &*c.name).collect();
            ranges.iter().filter(|(n, _)| *n == v.name || copies.contains(n)).map(|(_, r)| r.clone()).collect()
        })
        .filter(|r: &Vec<Range<usize>>| !r.is_empty())
        .collect();
    let end = end.unwrap_or(out.len());
    Expansion { text: out, stops, end }
}

/// A template being filled in, in document offsets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub stops: Vec<Vec<Range<usize>>>,
    pub current: usize,
    pub end: usize,
}

/// The copies of the current variable to update after it was typed in:
/// the range of the text to replace, its new text, and where the caret
/// goes after.
#[derive(Debug, PartialEq, Eq)]
pub struct Mirror {
    pub range: Range<usize>,
    pub text: String,
    pub caret: usize,
}

fn shift(at: usize, from: usize, delta: isize) -> usize {
    if at >= from { (at as isize + delta).max(0) as usize } else { at }
}

impl Session {
    /// The session of an expansion inserted at `at`.
    pub fn new(expansion: &Expansion, at: usize) -> Self {
        let stops = expansion.stops.iter().map(|g| g.iter().map(|r| r.start + at..r.end + at).collect()).collect();
        Session { stops, current: 0, end: expansion.end + at }
    }

    /// The current variable's first range (to select), if any.
    pub fn selection(&self) -> Option<Range<usize>> {
        self.stops.get(self.current).and_then(|g| g.first().cloned())
    }

    /// To the next variable; `None` past the last (the session ends at
    /// `end`).
    pub fn next(&mut self) -> Option<Range<usize>> {
        self.current += 1;
        self.selection()
    }

    pub fn previous(&mut self) -> Option<Range<usize>> {
        self.current = self.current.saturating_sub(1);
        self.selection()
    }

    /// The text changed from `old` to `new` (the caret now at `caret`).
    /// `None`: the edit was outside the current variable (the session
    /// ends). Otherwise the copies to update, if they differ now.
    pub fn edited(&mut self, old: &str, new: &str, caret: usize) -> Option<Option<Mirror>> {
        if old == new {
            return Some(None);
        }
        let caret = caret.min(new.len());
        let mut prefix = old.bytes().zip(new.bytes()).take_while(|(a, b)| a == b).count().min(caret);
        while !old.is_char_boundary(prefix) || !new.is_char_boundary(prefix) {
            prefix -= 1;
        }
        let max_suffix = (old.len() - prefix).min(new.len() - prefix).min(new.len() - caret);
        let mut suffix = old.bytes().rev().zip(new.bytes().rev()).take_while(|(a, b)| a == b).count().min(max_suffix);
        while !old.is_char_boundary(old.len() - suffix) || !new.is_char_boundary(new.len() - suffix) {
            suffix -= 1;
        }
        let edit = prefix..old.len() - suffix;
        let delta = new.len() as isize - old.len() as isize;
        let group = self.stops.get(self.current)?;
        let primary = group.iter().position(|r| r.start <= edit.start && edit.end <= r.end)?;
        // Everything after the edit moves; the variable edited grows.
        for (g, ranges) in self.stops.iter_mut().enumerate() {
            for (i, r) in ranges.iter_mut().enumerate() {
                if g == self.current && i == primary {
                    r.end = (r.end as isize + delta) as usize;
                } else {
                    *r = shift(r.start, edit.end, delta)..shift(r.end, edit.end, delta);
                }
            }
        }
        self.end = shift(self.end, edit.end, delta);
        let value = new[self.stops[self.current][primary].clone()].to_owned();
        let mirrors: Vec<usize> = (0..self.stops[self.current].len()).filter(|&i| i != primary && new[self.stops[self.current][i].clone()] != value).collect();
        if mirrors.is_empty() {
            return Some(None);
        }
        // One replacement over the copies (and whatever lies between); a
        // position then moves by the growth of every copy ending before it.
        let current = self.current;
        let mut copies: Vec<(usize, Range<usize>)> = mirrors.iter().map(|&i| (i, self.stops[current][i].clone())).collect();
        copies.sort_by_key(|(_, r)| r.start);
        let growth = |r: &Range<usize>| value.len() as isize - r.len() as isize;
        let moved = |pos: usize| -> isize { copies.iter().filter(|(_, r)| r.end <= pos).map(|(_, r)| growth(r)).sum() };
        let span_start = copies.first().map_or(0, |(_, r)| r.start);
        let span_end = copies.iter().map(|(_, r)| r.end).max().unwrap_or(span_start);
        let mut text = String::new();
        let mut last = span_start;
        for (_, r) in &copies {
            text.push_str(&new[last..r.start]);
            text.push_str(&value);
            last = r.end;
        }
        text.push_str(&new[last..span_end]);
        let at = |pos: usize| (pos as isize + moved(pos)) as usize;
        for (g, ranges) in self.stops.iter_mut().enumerate() {
            for (i, x) in ranges.iter_mut().enumerate() {
                if g == current && mirrors.contains(&i) {
                    let start = (x.start as isize + moved(x.start)) as usize;
                    *x = start..start + value.len();
                } else {
                    *x = at(x.start)..at(x.end);
                }
            }
        }
        self.end = at(self.end);
        let caret_after = at(caret) as isize;
        Some(Some(Mirror { range: span_start..span_end, text, caret: caret_after.max(0) as usize }))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn computes_variables_from_the_scope() {
        use super::*;
        let scope = Scope {
            class_name: "Shop".into(),
            method_name: "sell".into(),
            parameters: vec![("item".into(), "String".into()), ("i".into(), "int".into())],
            variables: vec![("prices".into(), "double[]".into()), ("stock".into(), "Map<String, List<Integer>>".into()), ("names".into(), "List<String>".into())],
            fields: vec![],
        };
        let java = |name: &str| {
            let t = resolved(builtin(false).iter().find(|t| t.name == name).unwrap(), &scope);
            let (t, _) = shortened(&t);
            expand(&t, "", "  ").text
        };
        assert_eq!(java("soutp"), "System.out.println(\"item = \" + item + \", i = \" + i);");
        assert_eq!(java("soutm"), "System.out.println(\"Shop.sell\");");
        assert_eq!(java("fori"), "for(int j = 0; j < ; j++) {\n  \n}");
        assert_eq!(java("itar"), "for(int j = 0; j < prices.length; j++) {\n  double o = prices[j];\n  \n}");
        assert_eq!(java("iter"), "for (String o : names) {\n  \n}");
        assert_eq!(java("itm"), "for (Map.Entry<String, List<Integer>> entry: stock.entrySet()) {\n  String key = entry.getKey();\n  List<Integer> value = entry.getValue();\n  \n}");
        assert_eq!(java("mx"), "i = Math.max(i, );");
        assert_eq!(java("ifn"), "if (names == null) {\n  \n}");
        let kotlin = Scope { parameters: vec![("tax".into(), "Double".into()), ("items".into(), "List<Item>".into())], ..scope.clone() };
        let kt = |name: &str| expand(&resolved(builtin(true).iter().find(|t| t.name == name).unwrap(), &kotlin), "", "    ").text;
        assert_eq!(kt("soutp"), "println(\"tax = [${tax}], items = [${items}]\")");
        assert_eq!(kt("soutf"), "println(\"Shop.sell\")");
        assert_eq!(element_type("IntArray").as_deref(), Some("Int"));
        assert!(Kind::Iterable.matches("Array<String>") && !Kind::Array.matches("List<Int>"));
    }

    #[test]
    fn offers_templates_starting_with_the_word() {
        let names = |typed, contexts: &[Context]| super::completing(true, typed, contexts).into_iter().map(|t| t.name.into_owned()).collect::<Vec<_>>();
        assert_eq!(names("so", &[Context::Statement]), ["sout", "soutv", "soutf", "soutp"]);
        assert!(names("sout", &[Context::Class]).is_empty());
        assert!(names("", &[Context::Statement]).is_empty());
        assert!(super::any_starts_with(false, "fo"));
        assert!(!super::any_starts_with(false, "xyz"));
    }

    use super::*;

    fn template(kotlin: bool, name: &str) -> &'static Template {
        builtin(kotlin).iter().find(|t| t.name == name).unwrap()
    }

    #[test]
    fn surrounds_with_templates() {
        let wl = builtin(false).iter().find(|t| t.name == "WL").unwrap();
        assert!(wl.surrounds() && find(false, "WL", &[Context::Statement]).is_none());
        let e = expand_around(wl, "    a();\n    if (b) {\n        c();\n    }\n", "    ", "    ");
        assert_eq!(e.text, "$LOCK$.writeLock().lock();\n    try { \n        a();\n        if (b) {\n            c();\n        }\n    } finally {\n        $LOCK$.writeLock().unlock();\n    }\n".replace("$LOCK$", ""));
        assert_eq!(e.stops, vec![vec![0..0, e.text.find(".writeLock().unlock").unwrap()..e.text.find(".writeLock().unlock").unwrap()]]);
        let (c, imports) = shortened(builtin(false).iter().find(|t| t.name == "C").unwrap());
        assert_eq!(imports, ["java.util.concurrent.Callable"]);
        let e = expand_around(&c, "run();", "  ", "  ");
        assert_eq!(e.text, "Callable<Object> callable = new Callable<Object>() {\n    public Object call() throws Exception {\n      run();\n       \n    }\n  };");
        assert_eq!(e.stops.len(), 1);
        let i = builtin(false).iter().find(|t| t.name == "I").unwrap();
        let (i, imports) = shortened(i);
        assert!(imports.is_empty());
        assert_eq!(expand_around(&i, "names", "", "  ").text, "for (Object  : names) {\n  \n}\n");
    }

    #[test]
    fn expands_and_indents() {
        let e = expand(template(true, "iter"), "    ", "    ");
        assert_eq!(e.text, "for (i in iterable) {\n        \n    }");
        // ITERABLE first (the variables' order), then VAR.
        assert_eq!(&e.text[e.stops[0][0].clone()], "iterable");
        assert_eq!(&e.text[e.stops[1][0].clone()], "i");
        assert_eq!(e.end, "for (i in iterable) {\n        ".len());
        let e = expand(template(false, "fori"), "  ", "  ");
        assert_eq!(e.text, "for(int i = 0; i < ; i++) {\n    \n  }");
        assert_eq!(e.stops[0].len(), 3);
        let e = expand(template(true, "soutv"), "", "    ");
        assert_eq!(e.text, "println(\"expr = ${expr}\")");
        // EXPR with its escaped copy.
        assert_eq!(e.stops.len(), 1);
        assert_eq!(e.stops[0].len(), 2);
        assert_eq!(find(true, "sout", &[Context::Statement, Context::Expression]).map(|t| t.value), Some(s("println($END$)")));
        assert!(find(true, "sout", &[Context::TopLevel]).is_none());
    }

    #[test]
    fn copies_follow_the_variable() {
        let line = "    ";
        let e = expand(template(false, "fori"), line, "    ");
        let at = line.len();
        let mut text = format!("{line}{}", e.text);
        let mut session = Session::new(&e, at);
        let first = session.selection().unwrap();
        assert_eq!(&text[first.clone()], "i");
        // Type `j` over the selected `i`.
        let old = text.clone();
        text.replace_range(first.clone(), "j");
        let mirror = session.edited(&old, &text, first.start + 1).unwrap().unwrap();
        text.replace_range(mirror.range.clone(), &mirror.text);
        assert_eq!(text, "    for(int j = 0; j < ; j++) {\n        \n    }");
        assert_eq!(mirror.caret, first.start + 1);
        // On to LIMIT, then typing `10` there.
        let limit = session.next().unwrap();
        assert_eq!(limit, text.find("< ;").unwrap() + 2..text.find("< ;").unwrap() + 2);
        let old = text.clone();
        text.insert_str(limit.start, "10");
        assert_eq!(session.edited(&old, &text, limit.start + 2), Some(None));
        assert_eq!(session.next(), None);
        assert_eq!(&text[session.end - 8..session.end], "        ");
        // An edit outside the variable ends the session.
        let mut session = Session::new(&e, at);
        let old = format!("{line}{}", e.text);
        let mut moved = old.clone();
        moved.insert(0, 'x');
        assert_eq!(session.edited(&old, &moved, 1), None);
    }

    #[test]
    fn the_persons_templates_come_first() {
        let custom = CustomTemplates {
            user: vec![UserTemplate {
                abbreviation: "logd".into(),
                description: "Log.d".into(),
                text: "Log.d($TAG$, \"$MSG$: ${$MSG$}\")$END$".into(),
                defaults: vec![("TAG".into(), "TAG".into())],
                kotlin: vec![Context::Statement],
                java: vec![],
                enabled: true,
            }],
            disabled: vec![CustomTemplates::key(true, "sout")],
        };
        let kotlin = templates_in(&custom, true);
        assert_eq!(kotlin[0].name, "logd");
        assert!(!kotlin.iter().any(|t| t.name == "sout"));
        assert!(templates_in(&custom, false).iter().any(|t| t.name == "sout"));
        assert!(!templates_in(&custom, false).iter().any(|t| t.name == "logd"));
        assert_eq!(variables(&custom.user[0].text), ["TAG", "MSG"]);
        let e = expand(&kotlin[0], "", "    ");
        assert_eq!(e.text, "Log.d(TAG, \": ${}\")");
        // TAG, then MSG with its repeat.
        assert_eq!(e.stops.len(), 2);
        assert_eq!(e.stops[1].len(), 2);
        assert_eq!(e.end, e.text.len());
        let off = CustomTemplates { user: vec![UserTemplate { enabled: false, ..custom.user[0].clone() }], disabled: Vec::new() };
        assert!(templates_in(&off, true).iter().all(|t| t.name != "logd"));
    }

    #[test]
    fn longer_copies_move_what_follows() {
        let e = expand(template(true, "soutv"), "", "    ");
        let mut text = e.text.clone();
        let mut session = Session::new(&e, 0);
        let r = session.selection().unwrap();
        let old = text.clone();
        text.replace_range(r.clone(), "count");
        let m = session.edited(&old, &text, r.start + 5).unwrap().unwrap();
        text.replace_range(m.range.clone(), &m.text);
        assert_eq!(text, "println(\"count = ${count}\")");
        assert_eq!(&text[m.caret - 5..m.caret], "count");
        assert_eq!(session.end, text.len());
    }
}
