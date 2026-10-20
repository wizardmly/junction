//! HotSwap, as IntelliJ's debugger does it (`HotSwapManager`,
//! `ReloadClassesWorker`): the class files compiled since the session
//! started (or since the last reload) replace the loaded classes of the
//! same name through JDWP `RedefineClasses`; classes the VM hasn't loaded
//! are skipped. On Android the VM takes dex, so each class file is dexed
//! first. The messages are IntelliJ's (JavaDebuggerBundle).

use std::path::{Path, PathBuf};

use super::build::{Module, ModuleKind};

/// What a reload did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HotSwapOutcome {
    /// None of the changed classes is loaded (or nothing changed).
    UpToDate,
    Reloaded(usize),
    /// The compile or the VM refused; IntelliJ's message.
    Failed(String),
}

impl HotSwapOutcome {
    /// The status IntelliJ shows in its HotSwap notification.
    pub fn message(&self) -> String {
        match self {
            Self::UpToDate => "Loaded classes are up to date. Nothing to reload.".into(),
            Self::Reloaded(n) => format!("{n} {} reloaded", if *n == 1 { "class" } else { "classes" }),
            Self::Failed(reason) => reason.clone(),
        }
    }
}

/// A class file's class: `com/example/Calc$Inner.class` under its output
/// root is `com.example.Calc$Inner`.
pub fn class_name(relative: &Path) -> Option<String> {
    let text = relative.to_str()?.replace('\\', "/");
    let name = text.strip_suffix(".class")?;
    // `module-info` and `package-info` aren't classes to redefine.
    if name.is_empty() || name.ends_with("-info") {
        return None;
    }
    Some(name.replace('/', "."))
}

/// Whether a source file is a test source (`src/test/…`, `src/androidTest/…`
/// or a Kotlin Multiplatform `src/jvmTest/…`).
pub fn is_test_source(file: &Path) -> bool {
    let text = file.to_string_lossy().replace('\\', "/");
    text.split("/src/").nth(1).and_then(|rest| rest.split('/').next()).is_some_and(|set| set.ends_with("Test") || set == "test")
}

/// The task compiling a changed source before its classes are reloaded:
/// the module's main classes, or its unit test classes for a test source.
pub fn compile_task(module: &Module, variant: &str, test: bool) -> String {
    if !test {
        return module.compile_task(variant);
    }
    let prefix = if module.path == ":" { String::new() } else { module.path.clone() };
    match module.kind {
        ModuleKind::Android => format!("{prefix}:compile{}UnitTestJavaWithJavac", capitalize(variant)),
        ModuleKind::Jvm => format!("{prefix}:testClasses"),
    }
}

/// Where a module's compiled classes are, as Gradle and AGP lay them out:
/// Java and Kotlin, main and unit test.
pub fn output_roots(module: &Module, variant: &str) -> Vec<PathBuf> {
    let build = module.dir.join("build");
    match module.kind {
        ModuleKind::Jvm => ["java/main", "kotlin/main", "java/test", "kotlin/test"].iter().map(|d| build.join("classes").join(d)).collect(),
        ModuleKind::Android => {
            let test = format!("{variant}UnitTest");
            let mut roots = Vec::new();
            for v in [variant, test.as_str()] {
                let task = format!("compile{}JavaWithJavac", capitalize(v));
                roots.push(build.join("tmp/kotlin-classes").join(v));
                roots.push(build.join("intermediates/javac").join(v).join(task).join("classes"));
                roots.push(build.join("intermediates/javac").join(v).join("classes"));
                roots.push(build.join("intermediates/built_in_kotlinc").join(v).join(format!("compile{}Kotlin", capitalize(v))).join("classes"));
            }
            roots
        }
    }
}

/// JDI's wording of a `RedefineClasses` error, under IntelliJ's heading.
pub fn redefine_error(code: u16) -> String {
    match code {
        60 => "Class format error : class format error".into(),
        61 => "Class circularity error : circular class definition".into(),
        62 => "Verification error : failed verification".into(),
        63 => "Operation not supported by VM: add method not implemented".into(),
        64 => "Operation not supported by VM: schema change not implemented".into(),
        66 => "Operation not supported by VM: hierarchy change not implemented".into(),
        67 => "Operation not supported by VM: delete method not implemented".into(),
        68 => "Unsupported class version : unsupported version".into(),
        69 => "Class definition not found : names don't match".into(),
        70 => "Operation not supported by VM: changes to class modifiers not implemented".into(),
        71 => "Operation not supported by VM: changes to method modifiers not implemented".into(),
        72 => "Operation not supported by VM: changes to class attribute not implemented".into(),
        99 => "Operation not supported by VM: redefineClasses not supported".into(),
        _ => format!("Exception while reloading classes : JDWP error {code}"),
    }
}

/// The compiled classes on disk.
pub trait ClassFiles: Send + Sync {
    /// The class files under the roots written after `since`: (class, file).
    fn changed_since(&self, roots: &[PathBuf], since: std::time::SystemTime) -> Vec<(String, PathBuf)>;
    fn read(&self, file: &Path) -> Result<Vec<u8>, String>;
}

/// Turns class files into what an Android VM redefines: one dex for each,
/// in order.
pub trait Dexer: Send + Sync {
    fn dex(&self, class_files: &[PathBuf]) -> Result<Vec<Vec<u8>>, String>;
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(kind: ModuleKind, path: &str) -> Module {
        Module { build_root: "/p".into(), dir: "/p/app".into(), path: path.into(), kind }
    }

    #[test]
    fn class_names_of_class_files() {
        assert_eq!(class_name(Path::new("com/example/Calc$Inner.class")).as_deref(), Some("com.example.Calc$Inner"));
        assert_eq!(class_name(Path::new("Main.class")).as_deref(), Some("Main"));
        assert_eq!(class_name(Path::new("com/example/package-info.class")), None);
        assert_eq!(class_name(Path::new("META-INF/main.kotlin_module")), None);
    }

    #[test]
    fn test_sources_compile_their_test_classes() {
        assert!(is_test_source(Path::new("/p/app/src/test/java/a/ATest.kt")));
        assert!(is_test_source(Path::new("/p/app/src/androidTest/java/a/ATest.kt")));
        assert!(!is_test_source(Path::new("/p/app/src/main/java/test/A.kt")));
        assert_eq!(compile_task(&module(ModuleKind::Jvm, ":core"), "debug", true), ":core:testClasses");
        assert_eq!(compile_task(&module(ModuleKind::Jvm, ":"), "debug", false), ":classes");
        assert_eq!(compile_task(&module(ModuleKind::Android, ":app"), "debug", true), ":app:compileDebugUnitTestJavaWithJavac");
        assert_eq!(compile_task(&module(ModuleKind::Android, ":app"), "debug", false), ":app:compileDebugJavaWithJavac");
    }

    #[test]
    fn output_roots_follow_gradle_and_agp() {
        let jvm = output_roots(&module(ModuleKind::Jvm, ":"), "debug");
        assert!(jvm.contains(&PathBuf::from("/p/app/build/classes/kotlin/main")));
        let android = output_roots(&module(ModuleKind::Android, ":app"), "debug");
        assert!(android.contains(&PathBuf::from("/p/app/build/tmp/kotlin-classes/debug")));
        assert!(android.contains(&PathBuf::from("/p/app/build/intermediates/javac/debug/compileDebugJavaWithJavac/classes")));
    }

    #[test]
    fn messages_are_intellijs() {
        assert_eq!(HotSwapOutcome::Reloaded(1).message(), "1 class reloaded");
        assert_eq!(HotSwapOutcome::Reloaded(3).message(), "3 classes reloaded");
        assert_eq!(HotSwapOutcome::UpToDate.message(), "Loaded classes are up to date. Nothing to reload.");
        assert_eq!(redefine_error(63), "Operation not supported by VM: add method not implemented");
    }
}
