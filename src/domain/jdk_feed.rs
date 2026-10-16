//! IntelliJ's Download JDK list: JetBrains' feed
//! (`download.jetbrains.com/jdk/feed/v1/jdks.json.xz`, `JdkList.kt`), each
//! product (vendor, version) with a package per OS and architecture: its
//! archive, checksum and the folders inside it.

use serde_json::Value;

pub const FEED_URL: &str = "https://download.jetbrains.com/jdk/feed/v1/jdks.json.xz";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Archive {
    TarGz,
    Zip,
}

/// A JDK the feed offers for this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JdkItem {
    /// "Oracle", "Amazon", "JetBrains".
    pub vendor: String,
    /// "OpenJDK", "Corretto", "Runtime".
    pub product: Option<String>,
    pub major: u32,
    /// "21.0.5".
    pub version: String,
    /// IntelliJ shows it first (Oracle OpenJDK).
    pub default: bool,
    pub url: String,
    pub sha256: String,
    pub archive: Archive,
    pub archive_name: String,
    /// The folder the archive's files are in (`jdk-21.0.5`), stripped.
    pub root_prefix: String,
    /// From the unpacked folder to the JDK home (`Contents/Home` on macOS).
    pub java_home_prefix: String,
    /// `~/.jdks/<this>`.
    pub install_folder: String,
    pub size: u64,
}

impl JdkItem {
    /// "Amazon Corretto 21.0.5", as the dialog's vendor list names it.
    pub fn label(&self) -> String {
        match &self.product {
            Some(p) if !self.vendor.contains(p.as_str()) => format!("{} {} {}", self.vendor, p, self.version),
            _ => format!("{} {}", self.vendor, self.version),
        }
    }
}

/// The feed's names for this machine: `linux` / `macOS` / `windows`,
/// `x86_64` / `aarch64`.
pub fn platform(os: &str, arch: &str) -> (&'static str, &'static str) {
    let os = match os {
        "macos" => "macOS",
        "windows" => "windows",
        _ => "linux",
    };
    let arch = if arch == "aarch64" { "aarch64" } else { "x86_64" };
    (os, arch)
}

/// The products with a package for the platform, newest versions first,
/// the default (Oracle OpenJDK) before the others of a version.
pub fn parse(json: &str, os: &str, arch: &str) -> Result<Vec<JdkItem>, String> {
    let root: Value = serde_json::from_str(json).map_err(|e| format!("Bad JDK list: {e}"))?;
    let jdks = root.get("jdks").and_then(Value::as_array).ok_or("Bad JDK list: no `jdks`")?;
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    let mut out = Vec::new();
    for item in jdks {
        let (Some(vendor), Some(major)) = (text(item, "vendor"), item.get("jdk_version_major").and_then(Value::as_u64)) else { continue };
        // `"default": true`, or a predicate: a predicate is for other setups.
        let default = item.get("default").and_then(Value::as_bool).unwrap_or(false);
        let Some(packages) = item.get("packages").and_then(Value::as_array) else { continue };
        for p in packages.iter().filter(|p| text(p, "os").as_deref() == Some(os) && text(p, "arch").as_deref() == Some(arch)) {
            let archive = match text(p, "package_type").as_deref() {
                Some("targz") => Archive::TarGz,
                Some("zip") => Archive::Zip,
                _ => continue,
            };
            let (Some(url), Some(sha256), Some(archive_name), Some(install_folder)) = (text(p, "url"), text(p, "sha256"), text(p, "archive_file_name"), text(p, "install_folder_name")) else { continue };
            out.push(JdkItem {
                vendor: vendor.clone(),
                product: text(item, "product"),
                major: major as u32,
                version: text(p, "version").or_else(|| text(item, "jdk_version")).unwrap_or_else(|| major.to_string()),
                default,
                url,
                sha256,
                archive,
                archive_name,
                root_prefix: text(p, "package_root_prefix").unwrap_or_default(),
                java_home_prefix: text(p, "package_to_java_home_prefix").unwrap_or_default(),
                install_folder,
                size: p.get("archive_size").and_then(Value::as_u64).unwrap_or(0),
            });
        }
    }
    out.sort_by(|a, b| b.major.cmp(&a.major).then(b.default.cmp(&a.default)).then(a.vendor.cmp(&b.vendor)));
    Ok(out)
}

/// The versions the dialog offers, newest first.
pub fn majors(items: &[JdkItem]) -> Vec<u32> {
    let mut m: Vec<u32> = items.iter().map(|i| i.major).collect();
    m.dedup();
    m
}

/// A path inside the archive, with the root folder taken off; none for
/// the root itself and for what's outside it.
pub fn strip_root<'a>(path: &'a str, root: &str) -> Option<&'a str> {
    let path = path.trim_start_matches("./");
    let rest = if root.is_empty() { path } else { path.strip_prefix(root)?.strip_prefix('/').unwrap_or("") };
    (!rest.is_empty()).then_some(rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &str = r#"{"jdks":[
      {"vendor":"Amazon","product":"Corretto","jdk_version_major":21,"jdk_version":"21.0.5","packages":[
        {"os":"linux","arch":"x86_64","version":"21.0.5","url":"https://c/21.tar.gz","package_type":"targz","package_root_prefix":"amazon-corretto-21.0.5","package_to_java_home_prefix":"","archive_file_name":"21.tar.gz","install_folder_name":"corretto-21.0.5","archive_size":200,"sha256":"aa"},
        {"os":"windows","arch":"x86_64","url":"https://c/21.zip","package_type":"zip","package_root_prefix":"jdk21","package_to_java_home_prefix":"","archive_file_name":"21.zip","install_folder_name":"corretto-21.0.5","sha256":"bb"}]},
      {"vendor":"Oracle","product":"OpenJDK","default":true,"jdk_version_major":21,"jdk_version":"21","packages":[
        {"os":"linux","arch":"x86_64","url":"https://o/21.tar.gz","package_type":"targz","package_root_prefix":"jdk-21","package_to_java_home_prefix":"","archive_file_name":"o.tar.gz","install_folder_name":"openjdk-21","sha256":"cc"}]},
      {"vendor":"JetBrains","product":"Runtime","jdk_version_major":17,"packages":[
        {"os":"macOS","arch":"aarch64","url":"https://j/17.tar.gz","package_type":"targz","package_root_prefix":"jbr","package_to_java_home_prefix":"Contents/Home","archive_file_name":"j.tar.gz","install_folder_name":"jbr-17","sha256":"dd"}]}
    ]}"#;

    #[test]
    fn reads_the_feed() {
        let linux = parse(FEED, "linux", "x86_64").unwrap();
        assert_eq!(linux.iter().map(|i| i.label()).collect::<Vec<_>>(), ["Oracle OpenJDK 21", "Amazon Corretto 21.0.5"]);
        assert_eq!((linux[1].archive, linux[1].install_folder.as_str(), linux[1].size), (Archive::TarGz, "corretto-21.0.5", 200));
        let mac = parse(FEED, "macOS", "aarch64").unwrap();
        assert_eq!((mac[0].label(), mac[0].java_home_prefix.as_str()), ("JetBrains Runtime 17".to_owned(), "Contents/Home"));
        assert_eq!(parse(FEED, "windows", "x86_64").unwrap()[0].archive, Archive::Zip);
        assert_eq!(majors(&linux), [21]);
        assert_eq!(platform("macos", "aarch64"), ("macOS", "aarch64"));
        assert!(parse("{}", "linux", "x86_64").is_err());
    }

    #[test]
    fn strips_the_root_folder() {
        assert_eq!(strip_root("jdk-21/bin/java", "jdk-21"), Some("bin/java"));
        assert_eq!(strip_root("./jdk-21/lib/", "jdk-21"), Some("lib/"));
        assert_eq!(strip_root("jdk-21/", "jdk-21"), None);
        assert_eq!(strip_root("other/x", "jdk-21"), None);
        assert_eq!(strip_root("bin/java", ""), Some("bin/java"));
    }
}
