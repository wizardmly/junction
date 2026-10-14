//! Compile on save: after a Kotlin or Java file is saved, its module is
//! compiled in the background and the compiler's problems replace that
//! module's previous ones (the "B" of Junction's lightweight analysis:
//! exact errors from the real compiler, a few seconds after saving).

use std::path::Path;

use crate::domain::build::{BuildSystem, CompileReport, Module};

/// Whether saving this file should compile.
pub fn compiles(file: &Path) -> bool {
    matches!(file.extension().and_then(|e| e.to_str()), Some("kt" | "java"))
}

/// The module of a saved file, when it has one and is a source file.
pub fn module_to_compile(build: &dyn BuildSystem, file: &Path) -> Option<Module> {
    compiles(file).then(|| build.module_of(file)).flatten()
}

pub fn compile(build: &dyn BuildSystem, module: &Module, variant: &str) -> CompileReport {
    build.compile(module, variant)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::build::ModuleKind;

    struct Fake;
    impl BuildSystem for Fake {
        fn module_of(&self, _: &Path) -> Option<Module> {
            Some(Module { build_root: "/p".into(), dir: "/p/app".into(), path: ":app".into(), kind: ModuleKind::Android })
        }
        fn compile(&self, module: &Module, variant: &str) -> CompileReport {
            CompileReport { succeeded: true, problems: Vec::new(), output: module.compile_task(variant) }
        }
    }

    #[test]
    fn only_sources_compile() {
        assert!(module_to_compile(&Fake, Path::new("/p/app/A.kt")).is_some());
        assert!(module_to_compile(&Fake, Path::new("/p/app/build.gradle.kts")).is_none());
        assert!(module_to_compile(&Fake, Path::new("/p/app/res/values/strings.xml")).is_none());
        let m = module_to_compile(&Fake, Path::new("/p/app/B.java")).unwrap();
        assert_eq!(compile(&Fake, &m, "debug").output, ":app:compileDebugJavaWithJavac");
    }
}
