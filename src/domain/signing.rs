//! Generate Signed Bundle / APK (Build menu), as Android Studio does it:
//! the release build runs with the keystore and key given to Gradle as the
//! `android.injected.signing.*` properties, which take the place of the
//! module's own signing config (a `keystore.jks` the project doesn't ship
//! included). The keystore's path, the key's alias and the destination
//! folder are remembered per module; the passwords only with Remember
//! passwords, in the system's password safe.

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
    /// Destination Folder: the package lands in its variant's folder there
    /// (`app/release/app-release.apk`), as Android Studio has it.
    pub destination: PathBuf,
}

/// What the dialog remembers for a module.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RememberedKey {
    pub store_file: PathBuf,
    pub key_alias: String,
    #[serde(default)]
    pub remember_passwords: bool,
    #[serde(default)]
    pub apk: bool,
    #[serde(default)]
    pub apk_destination: Option<PathBuf>,
    #[serde(default)]
    pub bundle_destination: Option<PathBuf>,
}

impl RememberedKey {
    /// The format's destination: the last one chosen, else the module's
    /// folder, Android Studio's `GradleSignStep.getInitialPath`.
    pub fn destination(&self, format: PackageFormat, module_dir: &Path) -> PathBuf {
        let last = match format {
            PackageFormat::Apk => &self.apk_destination,
            PackageFormat::Bundle => &self.bundle_destination,
        };
        last.clone().filter(|d| !d.as_os_str().is_empty()).unwrap_or_else(|| module_dir.to_owned())
    }

    pub fn format(&self) -> PackageFormat {
        if self.apk { PackageFormat::Apk } else { PackageFormat::Bundle }
    }

    /// Remembers what a request used.
    pub fn record(&mut self, request: &SigningRequest, remember_passwords: bool) {
        self.store_file = request.store_file.clone();
        self.key_alias = request.key_alias.clone();
        self.remember_passwords = remember_passwords;
        self.apk = request.format == PackageFormat::Apk;
        match request.format {
            PackageFormat::Apk => self.apk_destination = Some(request.destination.clone()),
            PackageFormat::Bundle => self.bundle_destination = Some(request.destination.clone()),
        }
    }
}

/// The password safe's keys of the keystore's password and the key's,
/// Android Studio's `KeystoreStep.makePasswordKey`.
pub fn password_keys(store_file: &Path, key_alias: &str) -> (String, String) {
    let path = store_file.to_string_lossy();
    (format!("KEY_STORE_PASSWORD__{path}"), format!("KEY_PASSWORD__{path}__{}", key_alias.trim()))
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

/// The destination folder's check, with Android Studio's messages;
/// `writable` says whether it is a folder the build can write to.
pub fn check_destination(request: &SigningRequest, writable: bool) -> Result<(), &'static str> {
    if request.destination.as_os_str().is_empty() {
        return Err(match request.format {
            PackageFormat::Apk => "Please specify destination folder for the signed apk",
            PackageFormat::Bundle => "Please specify destination folder for the signed bundle",
        });
    }
    if !writable {
        return Err("The destination folder does not exist or is not writeable");
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
        ("android.injected.apk.location".into(), request.destination.to_string_lossy().into_owned()),
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

/// Where the build leaves the package: the variant's folder in the
/// destination, `app/release`, a flavored variant's one level down,
/// `app/demo/release` (Android Gradle plugin's `dirName` under
/// `android.injected.apk.location`).
pub fn output_dir(variant: &str, destination: &Path) -> PathBuf {
    let build_type = variant.rfind(|c: char| c.is_uppercase()).map(|i| &variant[i..]).unwrap_or(variant);
    if build_type.len() == variant.len() {
        destination.join(variant)
    } else {
        let mut flavor = variant[..variant.len() - build_type.len()].to_owned();
        flavor.get_mut(..1).map(|f| f.make_ascii_lowercase());
        destination.join(flavor).join(build_type.to_lowercase())
    }
}

/// Android Studio's New Key Store dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewKeystore {
    pub store_file: PathBuf,
    pub store_password: String,
    pub store_confirm: String,
    pub key_alias: String,
    pub key_password: String,
    pub key_confirm: String,
    pub validity_years: String,
    /// First and Last Name, Organizational Unit, Organization, City or
    /// Locality, State or Province, Country Code.
    pub certificate: [String; 6],
}

impl Default for NewKeystore {
    fn default() -> Self {
        Self {
            store_file: PathBuf::new(),
            store_password: String::new(),
            store_confirm: String::new(),
            key_alias: "key0".into(),
            key_password: String::new(),
            key_confirm: String::new(),
            validity_years: "25".into(),
            certificate: Default::default(),
        }
    }
}

/// The certificate fields' labels, in `NewKeystore::certificate`'s order.
pub const CERTIFICATE_FIELDS: [&str; 6] = ["First and Last Name:", "Organizational Unit:", "Organization:", "City or Locality:", "State or Province:", "Country Code (XX):"];
const DN_KEYS: [&str; 6] = ["CN", "OU", "O", "L", "ST", "C"];

/// The New Key Store dialog's checks, with Android Studio's messages.
pub fn check_new(new: &NewKeystore) -> Result<u32, &'static str> {
    if new.store_file.as_os_str().is_empty() {
        return Err("Key store path is not specified.");
    }
    if new.store_password.chars().count() < 6 {
        return Err("Key store password must be at least 6 characters.");
    }
    if new.store_password != new.store_confirm {
        return Err("Key store passwords do not match.");
    }
    if new.key_alias.trim().is_empty() {
        return Err("Key alias is not specified.");
    }
    if new.key_password.chars().count() < 6 {
        return Err("Key password must be at least 6 characters.");
    }
    if new.key_password != new.key_confirm {
        return Err("Key passwords do not match.");
    }
    let years = new.validity_years.trim().parse::<u32>().ok().filter(|y| (1..=1000).contains(y)).ok_or("Validity must be a number of years from 1 to 1000.")?;
    if new.certificate.iter().all(|f| f.trim().is_empty()) {
        return Err("At least one Certificate issuer field is required to be non-empty.");
    }
    Ok(years)
}

/// The certificate's distinguished name, `CN=Ann Lee, O=Example`: the
/// fields given, escaped as RFC 2253 asks.
pub fn distinguished_name(certificate: &[String; 6]) -> String {
    let escape = |value: &str| {
        let mut out = String::new();
        for (i, c) in value.chars().enumerate() {
            if matches!(c, ',' | '+' | '"' | '\\' | '<' | '>' | ';' | '=') || (i == 0 && matches!(c, '#' | ' ')) {
                out.push('\\');
            }
            out.push(c);
        }
        out
    };
    certificate.iter().zip(DN_KEYS).filter(|(v, _)| !v.trim().is_empty()).map(|(v, k)| format!("{k}={}", escape(v.trim()))).collect::<Vec<_>>().join(", ")
}

/// Creates a keystore with one key. Implemented over the JDK's keytool.
pub trait KeyTool {
    fn create(&self, new: &NewKeystore, validity_days: u32) -> Result<(), String>;
}

#[cfg(test)]
mod tests {
    #[test]
    fn checks_a_new_keystore() {
        let mut new = NewKeystore { store_file: "/k/new.jks".into(), store_password: "secret1".into(), store_confirm: "secret1".into(), key_password: "secret2".into(), key_confirm: "secret2".into(), ..Default::default() };
        assert_eq!(check_new(&new), Err("At least one Certificate issuer field is required to be non-empty."));
        new.certificate[0] = "Ann Lee".into();
        assert_eq!(check_new(&new), Ok(25));
        assert_eq!(check_new(&NewKeystore { store_confirm: "x".into(), ..new.clone() }), Err("Key store passwords do not match."));
        assert_eq!(check_new(&NewKeystore { key_password: "12345".into(), ..new.clone() }), Err("Key password must be at least 6 characters."));
        assert_eq!(check_new(&NewKeystore { validity_years: "0".into(), ..new }), Err("Validity must be a number of years from 1 to 1000."));
    }

    #[test]
    fn writes_the_distinguished_name() {
        let cert = ["Ann Lee".to_owned(), String::new(), "Acme, Inc.".into(), String::new(), String::new(), "CN".into()];
        assert_eq!(distinguished_name(&cert), "CN=Ann Lee, O=Acme\\, Inc., C=CN");
    }

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
        let request = SigningRequest { format: PackageFormat::Apk, store_file: "/k/my.jks".into(), store_password: "s".into(), key_alias: " key0 ".into(), key_password: "k".into(), destination: "/p/app".into() };
        assert_eq!(check(&request, true), Ok(()));
        assert_eq!(check(&request, false), Err("Key store file not found."));
        assert_eq!(check(&SigningRequest { key_alias: "".into(), ..request.clone() }, true), Err("Key alias is not specified."));
        let props = properties(&request);
        assert_eq!(props[0], ("android.injected.signing.store.file".to_owned(), "/k/my.jks".to_owned()));
        assert_eq!(props[2].1, "key0");
        assert_eq!(props[4], ("android.injected.apk.location".to_owned(), "/p/app".to_owned()));
        assert_eq!(check_destination(&request, true), Ok(()));
        assert_eq!(check_destination(&request, false), Err("The destination folder does not exist or is not writeable"));
        assert_eq!(check_destination(&SigningRequest { destination: "".into(), ..request }, true), Err("Please specify destination folder for the signed apk"));
    }

    #[test]
    fn remembers_destinations_and_passwords_choice() {
        let dir = Path::new("/p/app");
        let mut key = RememberedKey::default();
        assert_eq!(key.destination(PackageFormat::Apk, dir), dir);
        assert_eq!(key.format(), PackageFormat::Bundle);
        let request = SigningRequest { format: PackageFormat::Apk, store_file: "/k/my.jks".into(), key_alias: "key0".into(), destination: "/out".into(), ..Default::default() };
        key.record(&request, true);
        assert!(key.remember_passwords);
        assert_eq!(key.format(), PackageFormat::Apk);
        assert_eq!(key.destination(PackageFormat::Apk, dir), PathBuf::from("/out"));
        assert_eq!(key.destination(PackageFormat::Bundle, dir), dir);
        // Files written before these fields were added still read.
        let old: RememberedKey = serde_json::from_str(r#"{"store_file":"/k/my.jks","key_alias":"key0"}"#).unwrap();
        assert!(!old.remember_passwords);
        assert_eq!(password_keys(Path::new("/k/my.jks"), " key0"), ("KEY_STORE_PASSWORD__/k/my.jks".to_owned(), "KEY_PASSWORD__/k/my.jks__key0".to_owned()));
    }

    #[test]
    fn finds_the_output_folder() {
        let m = app();
        assert_eq!(output_dir("release", &m.dir), PathBuf::from("/p/app/release"));
        assert_eq!(output_dir("demoRelease", Path::new("/out")), PathBuf::from("/out/demo/release"));
        let flavored = ModuleInfo { variants: vec!["demoDebug".into(), "demoRelease".into()], ..m };
        assert_eq!(default_variant(&flavored), "demoRelease");
    }
}
