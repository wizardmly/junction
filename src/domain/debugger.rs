//! The debugger, as Android Studio's (IntelliJ's Java debugger over JDWP):
//! what a debugged VM is made of (classes, methods and their line tables,
//! threads, frames, values), the port a debug session drives it through,
//! and the rules between them: which code a line breakpoint stops at,
//! which classes a source file holds, and how the Variables view writes a
//! value ("{CalcTest@1234}", "\"hello\"", "{int[3]@56}").

use std::time::Duration;

// --- The VM ------------------------------------------------------------------

/// A loaded class (or interface, array type): its JDWP id and Java name
/// (`com.example.Calc$Inner`, `int[]`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ClassRef {
    pub id: u64,
    pub name: String,
}

/// A method and its line table: `(code index, line)` pairs, 1-based lines,
/// as `Method.LineTable` gives them. Native and abstract methods have none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MethodInfo {
    pub id: u64,
    pub name: String,
    pub signature: String,
    pub is_static: bool,
    pub lines: Vec<(u64, u32)>,
}

/// A place in the code: a method of a class, and a bytecode index in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Location {
    pub class: u64,
    pub method: u64,
    pub index: u64,
}

/// A thread's state, as the Frames view's thread list shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadStatus {
    Running,
    Sleeping,
    Waiting,
    Monitor,
    Zombie,
    NotStarted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadInfo {
    pub id: u64,
    pub name: String,
    pub status: ThreadStatus,
    pub suspended: bool,
}

/// A frame of a suspended thread, resolved for the Frames view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub id: u64,
    pub location: Location,
    /// `com.example.CalcTest`.
    pub class: String,
    pub method: String,
    /// The 1-based line, when the class has a line table.
    pub line: Option<u32>,
    /// `CalcTest.java`, when the class was compiled with it.
    pub source: Option<String>,
}

/// A value, as JDWP tags it.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Void,
    Null,
    Boolean(bool),
    Byte(i8),
    Char(u16),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    /// A string, with its text (fetched with the value).
    String { id: u64, text: String },
    /// An array: its type (`int[]`) and length.
    Array { id: u64, class: String, length: usize },
    /// Any other object, with its class's name.
    Object { id: u64, class: String },
}

impl Value {
    /// The object behind a value, when it has fields or elements to expand.
    pub fn object_id(&self) -> Option<u64> {
        match self {
            Value::Array { id, length, .. } if *length > 0 => Some(*id),
            Value::Object { id, .. } => Some(*id),
            _ => None,
        }
    }
}

/// A local variable, field or array element.
#[derive(Clone, Debug, PartialEq)]
pub struct Variable {
    /// `x`, `this`, `[0]`.
    pub name: String,
    pub value: Value,
}

/// What a step does, as F8 / F7 / Shift+F8.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepDepth {
    /// Step Into.
    Into,
    /// Step Over.
    Over,
    /// Step Out.
    Out,
}

/// What the VM reports.
#[derive(Clone, Debug, PartialEq)]
pub enum DebugEvent {
    /// The VM started (suspended, when launched to wait for a debugger).
    VmStart { thread: u64 },
    /// The VM is gone: the program ended, or was stopped.
    VmDeath,
    /// A class was loaded: breakpoints in it can be set now.
    ClassPrepare { thread: u64, class: ClassRef, request: u32 },
    /// A breakpoint was hit.
    Breakpoint { thread: u64, location: Location, request: u32 },
    /// A step ended.
    Step { thread: u64, location: Location, request: u32 },
    /// An exception was thrown: the exception object, and whether
    /// something catches it.
    Exception { thread: u64, location: Location, exception: u64, caught: bool, request: u32 },
    ThreadStart { thread: u64 },
    ThreadDeath { thread: u64 },
}

/// A set of events, and whether they suspended the VM (all threads) or
/// the event's thread.
#[derive(Clone, Debug, PartialEq)]
pub struct EventSet {
    pub suspended: Suspend,
    pub events: Vec<DebugEvent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Suspend {
    None,
    Thread,
    All,
}

/// A debugged VM, over JDWP. Calls block until the VM replies; they may be
/// made from any thread while another waits for events.
pub trait DebugVm: Send + Sync {
    /// `VirtualMachine.AllClasses`.
    fn all_classes(&self) -> Result<Vec<ClassRef>, String>;
    /// `ReferenceType.SourceFile`: `CalcTest.java`, or none when compiled
    /// without it.
    fn source_file(&self, class: u64) -> Result<Option<String>, String>;
    /// The class's own methods with their line tables.
    fn methods(&self, class: u64) -> Result<Vec<MethodInfo>, String>;
    /// Tells when a class matching a pattern (`com.example.*`, `*`) loads,
    /// suspending all threads until the session resumes them.
    fn watch_class_prepare(&self, pattern: &str) -> Result<u32, String>;
    /// A breakpoint suspending all threads, as IntelliJ's default.
    fn set_breakpoint(&self, location: Location) -> Result<u32, String>;
    fn clear_breakpoint(&self, request: u32) -> Result<(), String>;
    /// An exception breakpoint suspending all threads: exceptions of the
    /// class (and its subclasses), or any (`None`); caught, uncaught or both.
    fn set_exception_breakpoint(&self, class: Option<u64>, caught: bool, uncaught: bool) -> Result<u32, String>;
    fn clear_exception_breakpoint(&self, request: u32) -> Result<(), String>;
    /// A line step on a suspended thread; the next event of it ends it.
    fn step(&self, thread: u64, depth: StepDepth) -> Result<u32, String>;
    fn clear_step(&self, request: u32) -> Result<(), String>;
    fn suspend(&self) -> Result<(), String>;
    fn resume(&self) -> Result<(), String>;
    fn threads(&self) -> Result<Vec<ThreadInfo>, String>;
    fn frames(&self, thread: u64) -> Result<Vec<Frame>, String>;
    /// `this` (unless static), then the visible locals in declaration order.
    fn locals(&self, thread: u64, frame: &Frame) -> Result<Vec<Variable>, String>;
    /// An object's fields (all of its class and superclasses'), or an
    /// array's elements (the first `limit`).
    fn children(&self, object: u64, limit: usize) -> Result<Vec<Variable>, String>;
    /// `VirtualMachine.ClassesBySignature`: a loaded class by Java name.
    fn find_class(&self, name: &str) -> Result<Option<ClassRef>, String>;
    /// An object's class.
    fn class_of(&self, object: u64) -> Result<ClassRef, String>;
    /// A class's Java name.
    fn class_name(&self, class: u64) -> Result<String, String>;
    /// A class's superclass; 0 for `Object`, interfaces and array types.
    fn superclass(&self, class: u64) -> Result<u64, String>;
    /// A field by name, up the class chain: an object's (`object`), else
    /// a static one. `None` when there's no such field.
    fn field(&self, class: u64, object: Option<u64>, name: &str) -> Result<Option<Value>, String>;
    /// Calls a method on a thread suspended by an event, the other threads
    /// staying suspended: a static one (`object` none) or an instance one.
    /// A thrown exception is the error, as IntelliJ writes it.
    fn invoke(&self, thread: u64, class: u64, object: Option<u64>, method: u64, args: &[Value]) -> Result<Value, String>;
    /// A string in the VM, to pass to a method.
    fn create_string(&self, text: &str) -> Result<Value, String>;
    fn array_element(&self, array: u64, index: usize) -> Result<Value, String>;
    /// Waits for the VM's next events; `None` on timeout.
    fn next_events(&self, timeout: Duration) -> Result<Option<EventSet>, String>;
    /// Stop: `VirtualMachine.Exit`.
    fn exit(&self, code: i32);
    /// Detach: `VirtualMachine.Dispose`; the VM drops the debugger's
    /// requests and resumes its threads.
    fn dispose(&self);
}

// --- Rules --------------------------------------------------------------------

/// The code a line breakpoint stops at in a class: in each method with
/// code on the line, its first index there (a line can span several
/// methods: a lambda, a field initializer in each constructor).
pub fn line_locations(class: u64, methods: &[MethodInfo], line: u32) -> Vec<Location> {
    methods
        .iter()
        .filter_map(|m| m.lines.iter().filter(|(_, l)| *l == line).map(|(ix, _)| *ix).min().map(|index| Location { class, method: m.id, index }))
        .collect()
}

/// The line of a location, from its method's line table: the entry with
/// the greatest index not after it.
pub fn line_of(method: &MethodInfo, index: u64) -> Option<u32> {
    method.lines.iter().filter(|(ix, _)| *ix <= index).max_by_key(|(ix, _)| *ix).map(|(_, line)| *line)
}

/// Whether a class may come from a source file of a package: its name is
/// in the package (`com.example.Calc`, `com.example.Calc$1`,
/// `com.example.CalcKt`), not a subpackage. The class's `SourceFile`
/// decides.
pub fn in_package(class: &str, package: &str) -> bool {
    match class.rsplit_once('.') {
        Some((p, _)) => p == package,
        None => package.is_empty(),
    }
}

/// The Java name of a JNI signature: `Lcom/example/Calc;` →
/// `com.example.Calc`, `[I` → `int[]`.
pub fn java_name(signature: &str) -> String {
    let dims = signature.chars().take_while(|c| *c == '[').count();
    let base = &signature[dims..];
    let name = match base {
        "Z" => "boolean".to_owned(),
        "B" => "byte".to_owned(),
        "C" => "char".to_owned(),
        "S" => "short".to_owned(),
        "I" => "int".to_owned(),
        "J" => "long".to_owned(),
        "F" => "float".to_owned(),
        "D" => "double".to_owned(),
        "V" => "void".to_owned(),
        _ => base.strip_prefix('L').and_then(|s| s.strip_suffix(';')).unwrap_or(base).replace('/', "."),
    };
    format!("{name}{}", "[]".repeat(dims))
}

/// The package a Kotlin or Java file declares (`package com.example`),
/// empty for the default package.
pub fn package_of(text: &str) -> String {
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("package ") {
            return rest.trim_end_matches(';').trim().to_owned();
        }
        // Code before any package line: the default package.
        let code = !(line.is_empty() || line.starts_with("//") || line.starts_with("/*") || line.starts_with('*') || line.starts_with('@'));
        if code && !line.starts_with("import ") {
            break;
        }
    }
    String::new()
}

/// Where a frame's source is, relative to a source root:
/// `com/example/calc/CalcTest.java` for `com.example.calc.CalcTest$1`
/// compiled from `CalcTest.java`.
pub fn source_path(class: &str, source: &str) -> String {
    match class.rsplit_once('.') {
        Some((package, _)) => format!("{}/{source}", package.replace('.', "/")),
        None => source.to_owned(),
    }
}

/// The ClassMatch pattern for the classes of a package.
pub fn package_pattern(package: &str) -> String {
    if package.is_empty() { "*".into() } else { format!("{package}.*") }
}

/// The JNI signature of a Java class name: `com.example.Calc` →
/// `Lcom/example/Calc;`.
pub fn signature_of(class: &str) -> String {
    format!("L{};", class.replace('.', "/"))
}

// --- Presentation -------------------------------------------------------------

/// A class's name as IntelliJ writes it in a value: without its package,
/// nested classes with `$` (`Calc$Inner`).
fn short_class(class: &str) -> &str {
    class.rsplit('.').next().unwrap_or(class)
}

/// A value as the Variables view writes it after `name = `: `4`, `'a' 97`,
/// `"hello"`, `null`, `{CalcTest@1234}`, `{int[3]@56}`.
pub fn value_text(value: &Value) -> String {
    match value {
        Value::Void => "void".into(),
        Value::Null => "null".into(),
        Value::Boolean(b) => b.to_string(),
        Value::Byte(n) => n.to_string(),
        Value::Short(n) => n.to_string(),
        Value::Int(n) => n.to_string(),
        Value::Long(n) => n.to_string(),
        Value::Float(f) => float_text(*f as f64, true),
        Value::Double(f) => float_text(*f, false),
        Value::Char(c) => {
            let ch = char::from_u32(*c as u32).map(|ch| escape_char(ch, '\'')).unwrap_or_default();
            format!("'{ch}' {c}")
        }
        Value::String { text, .. } => format!("\"{}\"", text.chars().map(|c| escape_char(c, '"')).collect::<String>()),
        Value::Array { id, class, length } => {
            // `int[]` with 3 elements is `int[3]`.
            let sized = match class.find("[]") {
                Some(at) => format!("{}[{length}]{}", &class[..at], &class[at + 2..]),
                None => class.clone(),
            };
            format!("{{{}@{id}}}", short_class(&sized))
        }
        Value::Object { id, class } => format!("{{{}@{id}}}", short_class(class)),
    }
}

/// Java's way: `1.0`, `0.5`, `1.0E10`; `NaN`, `Infinity`.
fn float_text(f: f64, single: bool) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0. { "Infinity".into() } else { "-Infinity".into() };
    }
    let text = if single { (f as f32).to_string() } else { f.to_string() };
    if text.contains('.') || text.contains('e') || text.contains("inf") { text } else { format!("{text}.0") }
}

fn escape_char(c: char, quote: char) -> String {
    match c {
        '\n' => "\\n".into(),
        '\t' => "\\t".into(),
        '\r' => "\\r".into(),
        '\\' => "\\\\".into(),
        c if c == quote => format!("\\{c}"),
        c => c.to_string(),
    }
}

/// A frame as the Frames view lists it: `addsWrong:17, CalcTest
/// (com.example.calc)`.
pub fn frame_text(frame: &Frame) -> String {
    let (package, class) = match frame.class.rsplit_once('.') {
        Some((p, c)) => (Some(p), c),
        None => (None, frame.class.as_str()),
    };
    let mut text = match frame.line {
        Some(line) => format!("{}:{line}, {class}", frame.method),
        None => format!("{}, {class}", frame.method),
    };
    if let Some(package) = package {
        text.push_str(&format!(" ({package})"));
    }
    text
}

/// A thread as the Frames view's list writes it: `"main"@1 in group
/// "main": RUNNING`; IntelliJ's statuses.
pub fn thread_text(thread: &ThreadInfo) -> String {
    let status = match thread.status {
        _ if thread.suspended => "SUSPENDED",
        ThreadStatus::Running => "RUNNING",
        ThreadStatus::Sleeping => "SLEEPING",
        ThreadStatus::Waiting => "WAIT",
        ThreadStatus::Monitor => "MONITOR",
        ThreadStatus::Zombie => "ZOMBIE",
        ThreadStatus::NotStarted => "NOT_STARTED",
    };
    format!("\"{}\"@{}: {status}", thread.name, thread.id)
}

/// Where breakpoints on `lines` (0-based) of `old` are in `new`, as
/// IntelliJ's range markers follow an edit: unchanged lines keep their
/// line, a changed or deleted line's breakpoint goes to where the change
/// starts (the matching line of a replacement).
pub fn follow_lines(old: &str, new: &str, lines: &[u32]) -> Vec<u32> {
    use similar::{DiffOp, TextDiff};
    let diff = TextDiff::from_lines(old, new);
    let ops = diff.ops();
    let (old_lines, new_lines): (Vec<&str>, Vec<&str>) = (old.lines().collect(), new.lines().collect());
    // New lines that aren't old ones kept: where a moved line went.
    let mut added: Vec<usize> = Vec::new();
    for op in ops {
        match *op {
            DiffOp::Insert { new_index, new_len, .. } | DiffOp::Replace { new_index, new_len, .. } => added.extend(new_index..new_index + new_len),
            _ => {}
        }
    }
    let mut claimed = vec![false; new_lines.len()];
    lines
        .iter()
        .map(|&line| {
            let line = line as usize;
            // A deleted or replaced line that's identically elsewhere moved there.
            let text = old_lines.get(line).map(|l| l.trim()).unwrap_or_default();
            let kept = ops.iter().any(|op| matches!(*op, DiffOp::Equal { old_index, len, .. } if (old_index..old_index + len).contains(&line)));
            if !kept && !text.is_empty() {
                if let Some(&to) = added.iter().find(|&&n| !claimed[n] && new_lines.get(n).map(|l| l.trim()) == Some(text)) {
                    claimed[to] = true;
                    return to as u32;
                }
            }
            for op in ops {
                let (old_range, new_start, new_len) = match *op {
                    DiffOp::Equal { old_index, new_index, len } => (old_index..old_index + len, new_index, len),
                    DiffOp::Delete { old_index, old_len, new_index } => (old_index..old_index + old_len, new_index, 0),
                    DiffOp::Replace { old_index, old_len, new_index, new_len } => (old_index..old_index + old_len, new_index, new_len),
                    DiffOp::Insert { .. } => continue,
                };
                if old_range.contains(&line) {
                    let offset = (line - old_range.start).min(new_len.saturating_sub(1));
                    return (new_start + offset) as u32;
                }
            }
            line as u32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn breakpoints_follow_edits() {
        let old = "a\nb\nc\nd\ne\n";
        // Two lines inserted above c: c and below move down.
        assert_eq!(super::follow_lines(old, "a\nb\nx\ny\nc\nd\ne\n", &[0, 2, 4]), [0, 4, 6]);
        // b..d replaced by one line (Extract Method's call), e below it.
        // Its lines follow into the new method; e, re-inserted, stays e.
        assert_eq!(super::follow_lines(old, "a\ncall()\ne\nfun x() {\nb\nc\nd\n}\n", &[2, 4]), [5, 2]);
        // Deleted: to where the deletion was.
        assert_eq!(super::follow_lines(old, "a\ne\n", &[2, 4]), [1, 1]);
    }

    use super::*;

    fn method(id: u64, lines: &[(u64, u32)]) -> MethodInfo {
        MethodInfo { id, name: format!("m{id}"), signature: "()V".into(), is_static: false, lines: lines.to_vec() }
    }

    #[test]
    fn line_breakpoints_stop_at_each_methods_first_code() {
        // A constructor and a lambda both have code on line 5.
        let methods = [method(1, &[(0, 4), (4, 5), (9, 5), (12, 6)]), method(2, &[(0, 10)]), method(3, &[(3, 5), (0, 7)])];
        assert_eq!(line_locations(7, &methods, 5), vec![Location { class: 7, method: 1, index: 4 }, Location { class: 7, method: 3, index: 3 }]);
        assert!(line_locations(7, &methods, 8).is_empty());
        assert_eq!(line_of(&methods[0], 10), Some(5));
        assert_eq!(line_of(&methods[0], 12), Some(6));
        assert_eq!(line_of(&method(4, &[]), 3), None);
    }

    #[test]
    fn names_and_packages() {
        assert_eq!(java_name("Lcom/example/Calc$Inner;"), "com.example.Calc$Inner");
        assert_eq!(java_name("[I"), "int[]");
        assert_eq!(java_name("[[Ljava/lang/String;"), "java.lang.String[][]");
        assert!(in_package("com.example.Calc$1", "com.example"));
        assert!(!in_package("com.example.sub.Calc", "com.example"));
        assert!(in_package("Main", ""));
    }

    #[test]
    fn packages_and_source_paths() {
        assert_eq!(package_of("// header\n/* x */\npackage com.example.calc;\n\nimport a.B;"), "com.example.calc");
        assert_eq!(package_of("@file:JvmName(\"X\")\npackage a.b\n"), "a.b");
        assert_eq!(package_of("import a.B\nclass C"), "");
        assert_eq!(package_of("class C\npackage x"), "");
        assert_eq!(source_path("com.example.calc.CalcTest$1", "CalcTest.java"), "com/example/calc/CalcTest.java");
        assert_eq!(source_path("Main", "Main.kt"), "Main.kt");
        assert_eq!((package_pattern("a.b").as_str(), package_pattern("").as_str()), ("a.b.*", "*"));
    }

    #[test]
    fn writes_values_as_intellij() {
        assert_eq!(value_text(&Value::Int(4)), "4");
        assert_eq!(value_text(&Value::Boolean(true)), "true");
        assert_eq!(value_text(&Value::Char(97)), "'a' 97");
        assert_eq!(value_text(&Value::Double(1.0)), "1.0");
        assert_eq!(value_text(&Value::Float(0.5)), "0.5");
        assert_eq!(value_text(&Value::Double(f64::NAN)), "NaN");
        assert_eq!(value_text(&Value::String { id: 1, text: "a \"b\"\n".into() }), "\"a \\\"b\\\"\\n\"");
        assert_eq!(value_text(&Value::Null), "null");
        assert_eq!(value_text(&Value::Object { id: 1234, class: "com.example.CalcTest".into() }), "{CalcTest@1234}");
        assert_eq!(value_text(&Value::Array { id: 56, class: "int[]".into(), length: 3 }), "{int[3]@56}");
        assert_eq!(value_text(&Value::Array { id: 57, class: "java.lang.String[][]".into(), length: 2 }), "{String[2][]@57}");
        assert_eq!(Value::Array { id: 1, class: "int[]".into(), length: 0 }.object_id(), None);
    }

    #[test]
    fn writes_frames_and_threads() {
        let frame = Frame { id: 1, location: Location { class: 1, method: 1, index: 0 }, class: "com.example.calc.CalcTest".into(), method: "addsWrong".into(), line: Some(17), source: None };
        assert_eq!(frame_text(&frame), "addsWrong:17, CalcTest (com.example.calc)");
        let thread = ThreadInfo { id: 1, name: "main".into(), status: ThreadStatus::Running, suspended: true };
        assert_eq!(thread_text(&thread), "\"main\"@1: SUSPENDED");
    }
}
