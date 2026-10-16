//! Download Sources, as IntelliJ's for a library shown decompiled: the
//! artifact's `-sources.jar` from Google's Maven repository or Maven
//! Central, saved beside the artifact in Gradle's cache, where the library
//! scan picks it up.

use std::io::Read as _;
use std::path::PathBuf;

const REPOSITORIES: &[&str] = &["https://dl.google.com/dl/android/maven2", "https://repo.maven.apache.org/maven2", "https://maven-central.storage-download.googleapis.com/maven2"];

fn agent(url: &str) -> ureq::Agent {
    let mut builder = ureq::AgentBuilder::new().timeout_connect(std::time::Duration::from_secs(20)).timeout_read(std::time::Duration::from_secs(60)).user_agent("Junction");
    let env = |k: &str| std::env::var(k).or_else(|_| std::env::var(k.to_lowercase())).ok().filter(|v| !v.is_empty());
    if let Some(proxy) = if url.starts_with("https") { env("HTTPS_PROXY") } else { env("HTTP_PROXY") }.and_then(|p| ureq::Proxy::new(p).ok()) {
        builder = builder.proxy(proxy);
    }
    builder.build()
}

/// `group:artifact:version`'s sources jar URL in a repository.
pub fn sources_url(repository: &str, group: &str, artifact: &str, version: &str) -> String {
    format!("{repository}/{}/{artifact}/{version}/{artifact}-{version}-sources.jar", group.replace('.', "/"))
}

/// Downloads the sources jar into Gradle's cache; where it went.
pub fn download(group: &str, artifact: &str, version: &str) -> Result<PathBuf, String> {
    let mut last = String::from("no repository answered");
    for repo in REPOSITORIES {
        let url = sources_url(repo, group, artifact, version);
        match agent(&url).get(&url).call() {
            Ok(response) => {
                let mut bytes = Vec::new();
                response.into_reader().take(512 << 20).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
                let home = std::env::var_os("GRADLE_USER_HOME").map(PathBuf::from).or_else(|| dirs_home().map(|h| h.join(".gradle"))).ok_or("No Gradle home")?;
                // Gradle names the folder by the file's SHA-1; any name is read.
                use sha2::Digest as _;
                let hash: String = sha2::Sha256::digest(&bytes).iter().take(20).map(|b| format!("{b:02x}")).collect();
                let dir = home.join("caches/modules-2/files-2.1").join(group).join(artifact).join(version).join(hash);
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let path = dir.join(format!("{artifact}-{version}-sources.jar"));
                std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
                return Ok(path);
            }
            Err(ureq::Error::Status(code, _)) => last = format!("{url}: HTTP {code}"),
            Err(e) => last = format!("{url}: {e}"),
        }
    }
    Err(format!("Sources not found for {group}:{artifact}:{version} ({last})"))
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    #[test]
    fn urls() {
        assert_eq!(super::sources_url("https://r", "androidx.compose.foundation", "foundation-layout-android", "1.7.0"), "https://r/androidx/compose/foundation/foundation-layout-android/1.7.0/foundation-layout-android-1.7.0-sources.jar");
    }
}
