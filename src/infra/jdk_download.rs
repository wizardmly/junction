//! Download JDK, as IntelliJ's `JdkInstaller`: the feed's list for this
//! machine, then the archive into `~/.jdks`, its SHA-256 checked, unpacked
//! without its root folder (a tar.gz keeps its file modes; a zip's are
//! set from its entries on Unix).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::domain::jdk_feed::{self, Archive, JdkItem};

fn agent(url: &str) -> ureq::Agent {
    // Long downloads: no overall timeout, only for connecting and reading.
    let mut builder = ureq::AgentBuilder::new().timeout_connect(std::time::Duration::from_secs(30)).timeout_read(std::time::Duration::from_secs(60)).user_agent("Junction");
    let env = |k: &str| std::env::var(k).or_else(|_| std::env::var(k.to_lowercase())).ok().filter(|v| !v.is_empty());
    if let Some(proxy) = if url.starts_with("https") { env("HTTPS_PROXY") } else { env("HTTP_PROXY") }.and_then(|p| ureq::Proxy::new(p).ok()) {
        builder = builder.proxy(proxy);
    }
    builder.build()
}

/// The JDKs the feed offers for this machine.
pub fn fetch_list() -> Result<Vec<JdkItem>, String> {
    let response = agent(jdk_feed::FEED_URL).get(jdk_feed::FEED_URL).call().map_err(|e| format!("Cannot download the list of JDKs: {e}"))?;
    let mut compressed = Vec::new();
    response.into_reader().take(64 << 20).read_to_end(&mut compressed).map_err(|e| e.to_string())?;
    let mut json = Vec::new();
    lzma_rs::xz_decompress(&mut compressed.as_slice(), &mut json).map_err(|e| format!("Cannot unpack the list of JDKs: {e:?}"))?;
    let (os, arch) = jdk_feed::platform(std::env::consts::OS, std::env::consts::ARCH);
    jdk_feed::parse(&String::from_utf8_lossy(&json), os, arch)
}

/// IntelliJ's place for downloaded JDKs: `~/.jdks/<install folder>`.
pub fn default_location(item: &JdkItem) -> PathBuf {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_default();
    home.join(".jdks").join(&item.install_folder)
}

/// Downloads and unpacks the JDK into `target` (which must not hold
/// anything yet); its home. `progress` gets the bytes so far and the total.
pub fn install(item: &JdkItem, target: &Path, progress: &mut dyn FnMut(u64, u64), cancelled: &dyn Fn() -> bool) -> Result<PathBuf, String> {
    if target.read_dir().is_ok_and(|mut d| d.next().is_some()) {
        return Err(format!("{} is not empty", target.display()));
    }
    let parent = target.parent().ok_or("Bad location")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let archive = parent.join(format!("{}.download", item.archive_name));
    let result = download(item, &archive, progress, cancelled).and_then(|_| unpack(item, &archive, target));
    let _ = std::fs::remove_file(&archive);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(target);
    }
    result
}

fn download(item: &JdkItem, to: &Path, progress: &mut dyn FnMut(u64, u64), cancelled: &dyn Fn() -> bool) -> Result<(), String> {
    let response = agent(&item.url).get(&item.url).call().map_err(|e| format!("Cannot download {}: {e}", item.archive_name))?;
    let total = response.header("Content-Length").and_then(|l| l.parse().ok()).unwrap_or(item.size);
    let mut reader = response.into_reader();
    let mut file = std::fs::File::create(to).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
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
        progress(done, total);
    }
    let sum: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if !sum.eq_ignore_ascii_case(&item.sha256) {
        return Err(format!("The checksum of {} doesn't match: expected {}, got {sum}", item.archive_name, item.sha256));
    }
    Ok(())
}

/// Unpacks an archive into `target` without its root folder; the JDK home.
pub fn unpack(item: &JdkItem, archive: &Path, target: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(target).map_err(|e| e.to_string())?;
    let safe = |rel: &str| -> Option<PathBuf> {
        let p = Path::new(rel);
        p.components().all(|c| matches!(c, std::path::Component::Normal(_))).then(|| target.join(p))
    };
    match item.archive {
        Archive::TarGz => {
            let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
            let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
            tar.set_preserve_permissions(true);
            for entry in tar.entries().map_err(|e| e.to_string())? {
                let mut entry = entry.map_err(|e| e.to_string())?;
                let name = entry.path().map_err(|e| e.to_string())?.to_string_lossy().into_owned();
                let Some(rel) = jdk_feed::strip_root(&name, &item.root_prefix) else { continue };
                let Some(out) = safe(rel.trim_end_matches('/')) else { continue };
                if let Some(dir) = out.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                }
                entry.unpack(&out).map_err(|e| format!("{name}: {e}"))?;
            }
        }
        Archive::Zip => {
            let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
            let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
            for i in 0..zip.len() {
                let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
                let name = entry.name().to_owned();
                let Some(rel) = jdk_feed::strip_root(&name, &item.root_prefix) else { continue };
                let Some(out) = safe(rel.trim_end_matches('/')) else { continue };
                if entry.is_dir() {
                    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
                    continue;
                }
                if let Some(dir) = out.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                }
                let mut f = std::fs::File::create(&out).map_err(|e| e.to_string())?;
                std::io::copy(&mut entry, &mut f).map_err(|e| e.to_string())?;
                #[cfg(unix)]
                if let Some(mode) = entry.unix_mode() {
                    use std::os::unix::fs::PermissionsExt as _;
                    let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode));
                }
            }
        }
    }
    let home = if item.java_home_prefix.is_empty() { target.to_path_buf() } else { target.join(&item.java_home_prefix) };
    let java = home.join("bin").join(if cfg!(windows) { "java.exe" } else { "java" });
    if !java.is_file() {
        return Err(format!("The JDK has no {}", java.display()));
    }
    Ok(home)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(archive: Archive, name: &str) -> JdkItem {
        JdkItem {
            vendor: "Test".into(),
            product: None,
            major: 21,
            version: "21".into(),
            default: false,
            url: String::new(),
            sha256: String::new(),
            archive,
            archive_name: name.into(),
            root_prefix: "jdk-21".into(),
            java_home_prefix: String::new(),
            install_folder: "test-21".into(),
            size: 0,
        }
    }

    #[test]
    fn unpacks_without_the_root_folder() {
        let dir = std::env::temp_dir().join(format!("junction-jdk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A tar.gz with an executable java and a stray file outside the root.
        let tgz = dir.join("a.tar.gz");
        {
            let gz = flate2::write::GzEncoder::new(std::fs::File::create(&tgz).unwrap(), flate2::Compression::fast());
            let mut tar = tar::Builder::new(gz);
            let mut add = |path: &str, mode: u32, body: &[u8]| {
                let mut h = tar::Header::new_gnu();
                h.set_size(body.len() as u64);
                h.set_mode(mode);
                h.set_cksum();
                tar.append_data(&mut h, path, body).unwrap();
            };
            add("jdk-21/bin/java", 0o755, b"#!/bin/sh\n");
            add("jdk-21/release", 0o644, b"JAVA_VERSION=\"21\"\n");
            add("README", 0o644, b"x");
            tar.into_inner().unwrap().finish().unwrap();
        }
        let home = unpack(&item(Archive::TarGz, "a.tar.gz"), &tgz, &dir.join("t")).unwrap();
        assert_eq!(home, dir.join("t"));
        assert!(home.join("release").is_file() && !home.join("README").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(std::fs::metadata(home.join("bin/java")).unwrap().permissions().mode() & 0o111, 0o111);
        }
        // A zip.
        let zip_path = dir.join("a.zip");
        {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
            let options = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
            zip.start_file(if cfg!(windows) { "jdk-21/bin/java.exe" } else { "jdk-21/bin/java" }, options).unwrap();
            zip.write_all(b"x").unwrap();
            zip.finish().unwrap();
        }
        assert!(unpack(&item(Archive::Zip, "a.zip"), &zip_path, &dir.join("z")).is_ok());
        // No java: an error.
        let mut wrong = item(Archive::TarGz, "a.tar.gz");
        wrong.root_prefix = "other".into();
        assert!(unpack(&wrong, &tgz, &dir.join("w")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The real feed (network): `cargo test fetches_the_feed -- --ignored`.
    #[test]
    #[ignore]
    fn fetches_the_feed() {
        let list = fetch_list().unwrap();
        assert!(list.iter().any(|i| i.major >= 21), "{}", list.len());
        println!("{} JDKs, newest {}", list.len(), list[0].label());
    }
}
