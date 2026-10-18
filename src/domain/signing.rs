//! Generate Signed Bundle / APK (Build menu), as Android Studio does it:
//! the release build runs with the keystore and key given to Gradle as the
//! `android.injected.signing.*` properties, which take the place of the
//! module's own signing config (a `keystore.jks` the project doesn't ship
//! included). The keystore's path and the key's alias are remembered per
//! module; the passwords aren't.

use std::path::{Path, PathBuf};

use super::run::{ModuleInfo, capitalize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PackageFormat {
    /// Android App Bundle (`.aab`), Android Studio's default.
    #[default]
    Bundle,
    Apk,
}

impl PackageFormat {
    pub fn label(self) -> &'static str {
        match self {
            PackageFormat::Bundle => "Android App Bundle",
            PackageFormat::Apk => "APK",
        }
    }

    /// "Generate Signed App Bundle", "Generate Signed APK": the build's
    /// title and the notification's.
    pub fn title(self) -> &'static str {
        match self {
            PackageFormat::Bundle => "Generate Signed App Bundle",
            PackageFormat::Apk => "Generate Signed APK",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SigningRequest {
    pub format: PackageFormat,
    pub store_file: PathBuf,
    pub store_password: String,
    pub key_alias: String,
    pub key_password: String,
}

/// What the dialog remembers for a module.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RememberedKey {
    pub store_file: PathBuf,
    pub key_alias: String,
}

/// Where the keystore paths and key aliases are kept, by module folder.
pub trait SigningStore {
    fn load(&self, module_dir: &Path) -> Option<RememberedKey>;
    fn save(&self, module_dir: &Path, key: &RememberedKey);
}

/// Android Studio's messages for a field left empty; `exists` says whether
/// the keystore file is there.
pub fn check(request: &SigningRequest, exists: bool) -> Result<(), &'static str> {
    if request.store_file.as_os_str().is_empty() {
        return Err("Key store path is not specified.");
    }
    if !exists {
        return Err("Key store file not found.");
    }
    if request.store_password.is_empty() {
        return Err("Key store password is not specified.");
    }
    if request.key_alias.trim().is_empty() {
        return Err("Key alias is not specified.");
    }
    if request.key_password.is_empty() {
        return Err("Key password is not specified.");
    }
    Ok(())
}

/// The task: `:app:bundleRelease`, `:app:assembleRelease`.
pub fn task(module: &ModuleInfo, variant: &str, format: PackageFormat) -> String {
    match format {
        PackageFormat::Apk => module.assemble_task(variant),
        PackageFormat::Bundle => {
            let prefix = if module.path == ":" { String::new() } else { module.path.clone() };
            format!("{prefix}:bundle{}", capitalize(variant))
        }
    }
}

/// The Gradle properties signing the build.
pub fn properties(request: &SigningRequest) -> Vec<(String, String)> {
    vec![
        ("android.injected.signing.store.file".into(), request.store_file.to_string_lossy().into_owned()),
        ("android.injected.signing.store.password".into(), request.store_password.clone()),
        ("android.injected.signing.key.alias".into(), request.key_alias.trim().to_owned()),
        ("android.injected.signing.key.password".into(), request.key_password.clone()),
    ]
}

/// The variant the dialog starts on: the module's first release one.
pub fn default_variant(module: &ModuleInfo) -> String {
    module
        .variants
        .iter()
        .find(|v| *v == "release")
        .or_else(|| module.variants.iter().find(|v| v.ends_with("Release")))
        .or_else(|| module.variants.first())
        .cloned()
        .unwrap_or_else(|| "release".into())
}

/// Where the build leaves the package: `app/build/outputs/bundle/release`,
/// `app/build/outputs/apk/release` (a flavored variant's APKs one level
/// down, `apk/demo/release`).
pub fn output_dir(module: &ModuleInfo, variant: &str, format: PackageFormat) -> PathBuf {
    let outputs = module.dir.join("build").join("outputs");
    match format {
        PackageFormat::Bundle => outputs.join("bundle").join(variant),
        PackageFormat::Apk => {
            let build_type = variant.rfind(|c: char| c.is_uppercase()).map(|i| &variant[i..]).unwrap_or(variant);
            if build_type.len() == variant.len() {
                outputs.join("apk").join(variant)
            } else {
                let mut flavor = variant[..variant.len() - build_type.len()].to_owned();
                flavor.get_mut(..1).map(|f| f.make_ascii_lowercase());
                outputs.join("apk").join(flavor).join(build_type.to_lowercase())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::run::ModuleType;

    fn app() -> ModuleInfo {
        ModuleInfo { path: ":app".into(), dir: PathBuf::from("/p/app"), kind: ModuleType::Application, variants: vec!["debug".into(), "release".into()] }
    }

    #[test]
    fn signs_a_release_build() {
        let m = app();
        assert_eq!(default_variant(&m), "release");
        assert_eq!(task(&m, "release", PackageFormat::Bundle), ":app:bundleRelease");
        assert_eq!(task(&m, "release", PackageFormat::Apk), ":app:assembleRelease");
        let request = SigningRequest { format: PackageFormat::Apk, store_file: "/k/my.jks".into(), store_password: "s".into(), key_alias: " key0 ".into(), key_password: "k".into() };
        assert_eq!(check(&request, true), Ok(()));
        assert_eq!(check(&request, false), Err("Key store file not found."));
        assert_eq!(check(&SigningRequest { key_alias: "".into(), ..request.clone() }, true), Err("Key alias is not specified."));
        let props = properties(&request);
        assert_eq!(props[0], ("android.injected.signing.store.file".to_owned(), "/k/my.jks".to_owned()));
        assert_eq!(props[2].1, "key0");
    }

    #[test]
    fn finds_the_output_folder() {
        let m = app();
        assert_eq!(output_dir(&m, "release", PackageFormat::Bundle), PathBuf::from("/p/app/build/outputs/bundle/release"));
        assert_eq!(output_dir(&m, "release", PackageFormat::Apk), PathBuf::from("/p/app/build/outputs/apk/release"));
        assert_eq!(output_dir(&m, "demoRelease", PackageFormat::Apk), PathBuf::from("/p/app/build/outputs/apk/demo/release"));
        let flavored = ModuleInfo { variants: vec!["demoDebug".into(), "demoRelease".into()], ..m };
        assert_eq!(default_variant(&flavored), "demoRelease");
    }
}
