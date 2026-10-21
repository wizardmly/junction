//! The Android SDK folder and Google's SDK repository, as Android Studio's
//! SDK Manager uses them: the installed packages (each folder's
//! `package.xml`), the repository's list (`repository2-3.xml`, kept for
//! when it can't be reached), and each package's archive downloaded, its
//! SHA-1 checked and unpacked into its folder without its root folder.
//! Accepted licenses are the hashes under `licenses/`, which Gradle reads too.
//!
//! `JUNCTION_SDK_REPO` names another repository folder (a mirror).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::domain::sdk_packages::{self, Package, Repository, SdkInstaller};

const DEFAULT_REPO: &str = "https://dl.google.com/android/repository/";
const LIST: &str = "repository2-3.xml";

/// The repository's folder URL, ending in `/`.
pub fn repository_url() -> String {
    let base = std::env::var("JUNCTION_SDK_REPO").ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| DEFAULT_REPO.to_owned());
    if base.ends_with('/') { base } else { format!("{base}/") }
}

fn cached_list() -> Option<PathBuf> {
    crate::index::libraries::cache_dir().map(|d| d.join("sdk").join(LIST))
}

/// The repository's packages; the copy kept from last time when it can't
/// be downloaded (and whether that happened).
pub fn fetch_repository() -> Result<(Repository, bool), String> {
    let url = format!("{}{LIST}", repository_url());
    let downloaded = crate::infra::jdk_download::agent(&url).get(&url).call().map_err(|e| e.to_string()).and_then(|r| {
        let mut text = String::new();
        r.into_reader().take(64 << 20).read_to_string(&mut text).map_err(|e| e.to_string())?;
        Ok(text)
    });
    match downloaded {
        Ok(text) => {
            if let Some(file) = cached_list() {
                let _ = file.parent().map(std::fs::create_dir_all);
                let _ = std::fs::write(file, &text);
            }
            Ok((sdk_packages::parse_repository(&text), false))
        }
        Err(e) => match cached_list().and_then(|f| std::fs::read_to_string(f).ok()) {
            Some(text) => Ok((sdk_packages::parse_repository(&text), true)),
            None => Err(format!("Failed to download the list of SDK packages from {url}: {e}")),
        },
    }
}

/// The packages installed in the SDK folder.
pub fn installed(sdk: &Path) -> Vec<Package> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<Package>) {
        if let Some(package) = std::fs::read_to_string(dir.join("package.xml")).ok().and_then(|x| sdk_packages::parse_local(&x)) {
            out.push(package);
            return;
        }
        if depth == 0 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') || name == "licenses" {
                continue;
            }
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                walk(&entry.path(), depth - 1, out);
            }
        }
    }
    let mut out = Vec::new();
    // `system-images;android-34;google_apis;x86_64` is four folders deep.
    walk(sdk, 4, &mut out);
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn sha1_hex(bytes: &[u8]) -> String {
    sha1_smol::Sha1::from(bytes).digest().to_string()
}

/// Whether the license text was accepted: its hash is in `licenses/<id>`.
pub fn license_accepted(sdk: &Path, id: &str, text: &str) -> bool {
    let hash = sha1_hex(text.trim().as_bytes());
    std::fs::read_to_string(sdk.join("licenses").join(id)).is_ok_and(|t| t.lines().any(|l| l.trim() == hash))
}

pub fn accept_license(sdk: &Path, id: &str, text: &str) -> Result<(), String> {
    if license_accepted(sdk, id, text) {
        return Ok(());
    }
    let dir = sdk.join("licenses");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(dir.join(id)).map_err(|e| e.to_string())?;
    write!(file, "\n{}", sha1_hex(text.trim().as_bytes())).map_err(|e| e.to_string())
}

/// The SDK folder packages are installed into.
pub struct SdkFolder {
    pub root: PathBuf,
    pub repository: Arc<Repository>,
}

impl SdkFolder {
    fn temp(&self) -> PathBuf {
        self.root.join(".temp")
    }

    fn download(&self, url: &str, sha1: &str, to: &Path, progress: &mut dyn FnMut(u64, u64), cancelled: &dyn Fn() -> bool) -> Result<(), String> {
        let response = crate::infra::jdk_download::agent(url).get(url).call().map_err(|e| format!("Cannot download {url}: {e}"))?;
        let total = response.header("Content-Length").and_then(|l| l.parse().ok()).unwrap_or(0);
        let mut reader = response.into_reader();
        let mut file = std::fs::File::create(to).map_err(|e| e.to_string())?;
        let mut hasher = sha1_smol::Sha1::new();
        let mut buf = vec![0u8; 256 * 1024];
        let mut done = 0u64;
        loop {
            if cancelled() {
                return Err("Cancelled".into());
            }
            let n = reader.read(&mut buf).map_err(|e| format!("Download failed: {e}"))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            done += n as u64;
            progress(done, total.max(done));
        }
        let sum = hasher.digest().to_string();
        if !sha1.is_empty() && !sum.eq_ignore_ascii_case(sha1) {
            return Err(format!("The checksum of {url} doesn't match: expected {sha1}, got {sum}"));
        }
        Ok(())
    }
}

/// Unpacks the zip into `target`, without the root folder all its entries share.
pub fn unzip(archive: &Path, target: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let names: Vec<String> = zip.file_names().map(str::to_owned).collect();
    let first = |n: &str| n.split('/').next().unwrap_or_default().to_owned();
    let root = names.first().map(|n| first(n)).filter(|r| names.iter().all(|n| first(n) == *r && (n.len() > r.len())));
    std::fs::create_dir_all(target).map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().to_owned();
        let rel = match &root {
            Some(r) => name[r.len()..].trim_start_matches('/'),
            None => name.as_str(),
        };
        let rel = rel.trim_end_matches('/');
        if rel.is_empty() {
            continue;
        }
        let path = Path::new(rel);
        if !path.components().all(|c| matches!(c, std::path::Component::Normal(_))) {
            continue;
        }
        let out = target.join(path);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let mode = entry.unix_mode();
        #[cfg(unix)]
        if mode.is_some_and(|m| m & 0o170000 == 0o120000) {
            let mut link = String::new();
            entry.read_to_string(&mut link).map_err(|e| e.to_string())?;
            let _ = std::fs::remove_file(&out);
            std::os::unix::fs::symlink(link, &out).map_err(|e| format!("{name}: {e}"))?;
            continue;
        }
        let mut f = std::fs::File::create(&out).map_err(|e| format!("{name}: {e}"))?;
        std::io::copy(&mut entry, &mut f).map_err(|e| format!("{name}: {e}"))?;
        #[cfg(unix)]
        if let Some(mode) = mode {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode & 0o7777));
        }
        #[cfg(not(unix))]
        let _ = mode;
    }
    Ok(())
}

impl SdkInstaller for SdkFolder {
    fn install(&self, package: &Package, progress: &mut dyn FnMut(u64, u64), cancelled: &dyn Fn() -> bool) -> Result<(), String> {
        let archive = sdk_packages::archive_for(package, std::env::consts::OS, std::env::consts::ARCH).ok_or_else(|| format!("{} has no download for this machine", package.display_name))?;
        let url = if archive.url.contains("://") { archive.url.clone() } else { format!("{}{}", repository_url(), archive.url) };
        let temp = self.temp();
        std::fs::create_dir_all(&temp).map_err(|e| format!("Cannot write to {}: {e}", self.root.display()))?;
        let stem = sdk_packages::relative_dir(&package.path).replace('/', "-");
        let zip = temp.join(format!("{stem}.zip"));
        let unpacked = temp.join(format!("{stem}.unzip"));
        let _ = std::fs::remove_dir_all(&unpacked);
        let result = (|| {
            self.download(&url, &archive.sha1, &zip, progress, cancelled)?;
            unzip(&zip, &unpacked)?;
            std::fs::write(unpacked.join("package.xml"), sdk_packages::package_xml(package, &self.repository)).map_err(|e| e.to_string())?;
            let target = self.root.join(sdk_packages::relative_dir(&package.path));
            // An update replaces the older revision.
            if target.exists() {
                std::fs::remove_dir_all(&target).map_err(|e| format!("Cannot remove {}: {e}", target.display()))?;
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::rename(&unpacked, &target).map_err(|e| format!("Cannot move the package to {}: {e}", target.display()))
        })();
        let _ = std::fs::remove_file(&zip);
        let _ = std::fs::remove_dir_all(&unpacked);
        let _ = std::fs::remove_dir(&temp);
        result
    }

    fn uninstall(&self, package: &Package) -> Result<(), String> {
        let target = self.root.join(sdk_packages::relative_dir(&package.path));
        std::fs::remove_dir_all(&target).map_err(|e| format!("Cannot remove {}: {e}", target.display()))?;
        // Empty folders above it go too (`system-images/android-34/google_apis`).
        let mut dir = target.parent();
        while let Some(d) = dir.filter(|d| *d != self.root && d.starts_with(&self.root)) {
            if std::fs::remove_dir(d).is_err() {
                break;
            }
            dir = d.parent();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn licenses_hash_trimmed_text() {
        let dir = std::env::temp_dir().join(format!("junction-sdk-lic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!license_accepted(&dir, "android-sdk-license", "Terms\n"));
        accept_license(&dir, "android-sdk-license", "Terms\n").unwrap();
        assert!(license_accepted(&dir, "android-sdk-license", " Terms"));
        // The hash Gradle and sdkmanager write for the same text.
        assert_eq!(std::fs::read_to_string(dir.join("licenses/android-sdk-license")).unwrap(), format!("\n{}", sha1_hex(b"Terms")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unzip_strips_root_and_scan_finds_package() {
        let dir = std::env::temp_dir().join(format!("junction-sdk-zip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("p.zip");
        {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
            let options = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
            zip.add_directory("android-14/", options).unwrap();
            zip.start_file("android-14/android.jar", options).unwrap();
            zip.write_all(b"jar").unwrap();
            zip.start_file("android-14/data/a.txt", options).unwrap();
            zip.write_all(b"a").unwrap();
            zip.finish().unwrap();
        }
        let sdk = dir.join("sdk");
        let target = sdk.join("platforms/android-34");
        unzip(&zip_path, &target).unwrap();
        assert_eq!(std::fs::read_to_string(target.join("android.jar")).unwrap(), "jar");
        assert!(target.join("data/a.txt").is_file());
        std::fs::write(target.join("package.xml"), r#"<repository><localPackage path="platforms;android-34"><revision><major>3</major></revision><display-name>Android SDK Platform 34</display-name></localPackage></repository>"#).unwrap();
        let found = installed(&sdk);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, "platforms;android-34");
        let folder = SdkFolder { root: sdk.clone(), repository: Arc::default() };
        folder.uninstall(&found[0]).unwrap();
        assert!(!sdk.join("platforms").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
