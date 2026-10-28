//! Generate › Test… (IntelliJ's Create Test): the test class written for
//! a Kotlin or Java class, from IntelliJ's JUnit 4 / JUnit 5 file
//! templates ("JUnit4 Test Class", "JUnit5 SetUp Method", …) with the
//! annotations imported; where it goes (the module's `src/test` root,
//! by the class's package); which of the class's methods can get a test.

use crate::domain::imports::ImportStyle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestLibrary {
    JUnit4,
    JUnit5,
}

impl TestLibrary {
    pub const ALL: [TestLibrary; 2] = [TestLibrary::JUnit4, TestLibrary::JUnit5];

    pub fn label(self) -> &'static str {
        match self {
            TestLibrary::JUnit4 => "JUnit4",
            TestLibrary::JUnit5 => "JUnit5",
        }
    }

    /// The package of its annotations, and its assertions class.
    fn package(self) -> &'static str {
        match self {
            TestLibrary::JUnit4 => "org.junit",
            TestLibrary::JUnit5 => "org.junit.jupiter.api",
        }
    }

    fn assertions(self) -> &'static str {
        match self {
            TestLibrary::JUnit4 => "Assert",
            TestLibrary::JUnit5 => "Assertions",
        }
    }

    fn before(self) -> &'static str {
        match self {
            TestLibrary::JUnit4 => "Before",
            TestLibrary::JUnit5 => "BeforeEach",
        }
    }

    fn after(self) -> &'static str {
        match self {
            TestLibrary::JUnit4 => "After",
            TestLibrary::JUnit5 => "AfterEach",
        }
    }
}

/// What the Create Test dialog asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestRequest {
    pub library: TestLibrary,
    pub class_name: String,
    /// Empty for none; qualified or not.
    pub superclass: String,
    pub package: String,
    pub set_up: bool,
    pub tear_down: bool,
    /// The methods to write a test for, by name.
    pub methods: Vec<String>,
}

/// The libraries a module's build files (and the version catalog) declare.
pub fn libraries_in(build: &str) -> Vec<TestLibrary> {
    let mut found = Vec::new();
    let junit4 = build.contains("junit:junit") || build.contains("group = \"junit\"") || build.contains("module = \"junit:junit\"");
    if junit4 {
        found.push(TestLibrary::JUnit4);
    }
    if build.contains("junit-jupiter") || build.contains("org.junit.jupiter") {
        found.push(TestLibrary::JUnit5);
    }
    found
}

/// The test source root for a source file (relative paths): the module's
/// `src/test/kotlin` for Kotlin when it exists, else `src/test/java`
/// when it exists, else the one named like the file's own root. None
/// outside a `src/<set>/` folder.
pub fn test_root(source: &str, kotlin: bool, exists: impl Fn(&str) -> bool) -> Option<String> {
    let (module, rest) = match source.split_once("src/") {
        Some((module, rest)) if module.is_empty() || module.ends_with('/') => (module, rest),
        _ => return None,
    };
    let own = rest.split('/').nth(1).filter(|d| matches!(*d, "java" | "kotlin"))?;
    let root = |dir: &str| format!("{module}src/test/{dir}");
    if kotlin && exists(&root("kotlin")) {
        return Some(root("kotlin"));
    }
    if exists(&root("java")) {
        return Some(root("java"));
    }
    Some(root(own))
}

/// A method the dialog lists: its name and how it shows (`add(a: Int):
/// Int`, `add(int, int): int`), from its declaration; none for a private
/// one.
pub fn test_candidate(style: ImportStyle, decl: &str, name: &str) -> Option<(String, String)> {
    let at = decl.find(&format!("{name}("))?;
    let modifiers = &decl[..at];
    if modifiers.split_whitespace().any(|w| w == "private") {
        return None;
    }
    let open = at + name.len();
    let mut depth = 0;
    let close = decl[open..].char_indices().find_map(|(i, c)| {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i);
                }
            }
            _ => {}
        }
        None
    })?;
    let params = &decl[open + 1..close];
    let params: Vec<String> = split_top(params).into_iter().map(|p| p.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|p| !p.is_empty()).collect();
    let shown = match style {
        ImportStyle::Kotlin => {
            let after = decl[close + 1..].trim_start();
            let ret = after.strip_prefix(':').map(|r| r.split(['{', '=']).next().unwrap_or("").trim()).filter(|r| !r.is_empty());
            let params: Vec<String> = params.iter().map(|p| p.split_whitespace().filter(|w| !matches!(*w, "vararg" | "crossinline" | "noinline")).collect::<Vec<_>>().join(" ")).collect();
            format!("{name}({}){}", params.join(", "), ret.map(|r| format!(": {r}")).unwrap_or_default())
        }
        ImportStyle::Java => {
            // The parameters' types; the return type is the word before the name.
            let types: Vec<String> = params
                .iter()
                .map(|p| {
                    let words: Vec<&str> = p.split_whitespace().filter(|w| !w.starts_with('@') && *w != "final").collect();
                    words[..words.len().saturating_sub(1)].join(" ")
                })
                .collect();
            let ret = modifiers.split_whitespace().last().unwrap_or("void");
            format!("{name}({}): {ret}", types.join(", "))
        }
    };
    Some((name.to_owned(), shown))
}

/// Splits on the commas outside `<…>`, `(…)` and `[…]`.
fn split_top(text: &str) -> Vec<&str> {
    let (mut parts, mut depth, mut start) = (Vec::new(), 0i32, 0);
    for (i, c) in text.char_indices() {
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

/// The test class's file, IntelliJ's templates with the annotations
/// imported: `unit` indents the members.
pub fn test_class(style: ImportStyle, request: &TestRequest, unit: &str) -> String {
    let lib = request.library;
    let kotlin = style == ImportStyle::Kotlin;
    let end = if kotlin { "" } else { ";" };
    let mut imports: Vec<String> = Vec::new();
    let (superclass, super_import) = match request.superclass.trim() {
        "" => (None, None),
        s => match s.rsplit_once('.') {
            Some((package, short)) if package != request.package && package.chars().next().is_some_and(char::is_lowercase) => (Some(short.to_owned()), Some(s.to_owned())),
            _ => (Some(s.rsplit('.').next().unwrap_or(s).to_owned()), None),
        },
    };
    let mut members: Vec<String> = Vec::new();
    let mut annotated = |annotation: &str, name: &str, throws: bool| {
        let import = format!("{}.{annotation}", lib.package());
        if !imports.contains(&import) {
            imports.push(import);
        }
        let declaration = match (kotlin, lib) {
            (true, _) => format!("fun {name}() {{"),
            (false, TestLibrary::JUnit4) => format!("public void {name}(){} {{", if throws { " throws Exception" } else { "" }),
            (false, TestLibrary::JUnit5) => format!("void {name}() {{"),
        };
        members.push(format!("{unit}@{annotation}\n{unit}{declaration}\n{unit}}}"));
    };
    if request.set_up {
        annotated(lib.before(), "setUp", true);
    }
    if request.tear_down {
        annotated(lib.after(), "tearDown", true);
    }
    let mut seen: Vec<&str> = Vec::new();
    for method in &request.methods {
        if seen.contains(&method.as_str()) {
            continue;
        }
        seen.push(method);
        annotated("Test", method, false);
    }
    imports.extend(super_import);
    imports.sort();
    let mut out = String::new();
    if !request.package.is_empty() {
        out.push_str(&format!("package {}{end}\n\n", request.package));
    }
    for import in &imports {
        out.push_str(&format!("import {import}{end}\n"));
    }
    if !imports.is_empty() {
        out.push('\n');
    }
    let assertions = format!("{}.{}.*", lib.package(), lib.assertions());
    if kotlin {
        out.push_str(&format!("import {assertions}\n\n"));
    } else {
        out.push_str(&format!("import static {assertions};\n\n"));
    }
    let visibility = if !kotlin && lib == TestLibrary::JUnit4 { "public " } else { "" };
    let extends = match (&superclass, kotlin) {
        (None, _) => String::new(),
        (Some(s), true) => format!(" : {s}()"),
        (Some(s), false) => format!(" extends {s}"),
    };
    out.push_str(&format!("{visibility}class {}{extends} {{\n", request.class_name));
    if !members.is_empty() {
        out.push('\n');
        out.push_str(&members.join("\n\n"));
        out.push('\n');
    }
    out.push_str("}\n");
    out
}

/// A package name as typed: empty, or dot-separated identifiers.
pub fn check_package(extension: &str, package: &str) -> Result<(), String> {
    if package.is_empty() {
        return Ok(());
    }
    package.split('.').try_for_each(|part| crate::domain::rename::check_name(extension, part)).map_err(|_| format!("'{package}' is not a valid package name"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(library: TestLibrary) -> TestRequest {
        TestRequest { library, class_name: "CalcTest".into(), superclass: String::new(), package: "com.example".into(), set_up: true, tear_down: true, methods: vec!["add".into(), "add".into(), "div".into()] }
    }

    #[test]
    fn writes_test_classes() {
        assert_eq!(
            test_class(ImportStyle::Java, &request(TestLibrary::JUnit5), "    "),
            "package com.example;\n\nimport org.junit.jupiter.api.AfterEach;\nimport org.junit.jupiter.api.BeforeEach;\nimport org.junit.jupiter.api.Test;\n\nimport static org.junit.jupiter.api.Assertions.*;\n\nclass CalcTest {\n\n    @BeforeEach\n    void setUp() {\n    }\n\n    @AfterEach\n    void tearDown() {\n    }\n\n    @Test\n    void add() {\n    }\n\n    @Test\n    void div() {\n    }\n}\n"
        );
        let mut r = request(TestLibrary::JUnit4);
        r.tear_down = false;
        r.methods.truncate(1);
        r.superclass = "com.base.BaseTest".into();
        assert_eq!(
            test_class(ImportStyle::Java, &r, "    "),
            "package com.example;\n\nimport com.base.BaseTest;\nimport org.junit.Before;\nimport org.junit.Test;\n\nimport static org.junit.Assert.*;\n\npublic class CalcTest extends BaseTest {\n\n    @Before\n    public void setUp() throws Exception {\n    }\n\n    @Test\n    public void add() {\n    }\n}\n"
        );
        r.superclass = "Base".into();
        r.set_up = false;
        r.package.clear();
        assert_eq!(
            test_class(ImportStyle::Kotlin, &r, "    "),
            "import org.junit.Test\n\nimport org.junit.Assert.*\n\nclass CalcTest : Base() {\n\n    @Test\n    fun add() {\n    }\n}\n"
        );
        r.methods.clear();
        assert_eq!(test_class(ImportStyle::Kotlin, &r, "  "), "import org.junit.Assert.*\n\nclass CalcTest : Base() {\n}\n");
    }

    #[test]
    fn finds_roots_and_candidates() {
        let none = |_: &str| false;
        assert_eq!(test_root("app/src/main/java/a/Calc.kt", true, none).as_deref(), Some("app/src/test/java"));
        assert_eq!(test_root("app/src/main/kotlin/a/Calc.kt", true, none).as_deref(), Some("app/src/test/kotlin"));
        assert_eq!(test_root("app/src/main/kotlin/a/Calc.kt", true, |d: &str| d == "app/src/test/java").as_deref(), Some("app/src/test/java"));
        assert_eq!(test_root("src/main/java/a/Calc.java", false, |d: &str| d == "src/test/kotlin").as_deref(), Some("src/test/java"));
        assert_eq!(test_root("lib/Calc.kt", true, none), None);
        assert_eq!(test_root("mysrc/main/java/Calc.kt", true, none), None);

        let kt = |decl: &str, name: &str| test_candidate(ImportStyle::Kotlin, decl, name).map(|c| c.1);
        assert_eq!(kt("fun add(a: Int, b: Int): Int = a + b", "add").as_deref(), Some("add(a: Int, b: Int): Int"));
        assert_eq!(kt("override fun sum(vararg xs: Map<String, Int>) {\n}", "sum").as_deref(), Some("sum(xs: Map<String, Int>)"));
        assert_eq!(kt("private fun hidden() {}", "hidden"), None);
        let java = |decl: &str, name: &str| test_candidate(ImportStyle::Java, decl, name).map(|c| c.1);
        assert_eq!(java("@Override\n    public static int add(final int a, @NonNull List<String> b) {", "add").as_deref(), Some("add(int, List<String>): int"));
        assert_eq!(java("private void hidden() {}", "hidden"), None);

        assert_eq!(libraries_in("testImplementation(\"junit:junit:4.13.2\")"), [TestLibrary::JUnit4]);
        assert_eq!(libraries_in("junit = { group = \"junit\", name = \"junit\" }\njunit-jupiter = \"5.10.0\""), [TestLibrary::JUnit4, TestLibrary::JUnit5]);
        assert!(check_package("java", "com.example").is_ok() && check_package("java", "com..x").is_err() && check_package("kt", "").is_ok());
    }
}
