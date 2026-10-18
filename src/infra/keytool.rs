//! New Key Store over the JDK's `keytool -genkeypair`: an RSA 2048 key in
//! a keystore of the extension's type (`.jks` JKS, else PKCS12). The
//! passwords go through the environment (`-storepass:env`), not the command
//! line.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::domain::signing::{KeyTool, NewKeystore, distinguished_name};

/// The keytool of the JDK a build's Gradle runs on, else the one on the
/// PATH.
pub struct JdkKeyTool {
    pub build_root: PathBuf,
}

fn keytool(build_root: &Path) -> PathBuf {
    let exe = if cfg!(windows) { "keytool.exe" } else { "keytool" };
    super::gradle::jdk_for(build_root).map(|jdk| jdk.home.join("bin").join(exe)).filter(|p| p.is_file()).unwrap_or_else(|| PathBuf::from(exe))
}

impl KeyTool for JdkKeyTool {
    fn create(&self, new: &NewKeystore, validity_days: u32) -> Result<(), String> {
        if let Some(dir) = new.store_file.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
        }
        let store_type = if new.store_file.extension().is_some_and(|e| e.eq_ignore_ascii_case("jks")) { "JKS" } else { "PKCS12" };
        let mut command = Command::new(keytool(&self.build_root));
        command
            .args(["-genkeypair", "-noprompt", "-keyalg", "RSA", "-keysize", "2048", "-storetype", store_type])
            .arg("-keystore")
            .arg(&new.store_file)
            .args(["-storepass:env", "JUNCTION_STORE_PASSWORD", "-keypass:env", "JUNCTION_KEY_PASSWORD", "-alias", new.key_alias.trim()])
            .args(["-validity", &validity_days.to_string(), "-dname", &distinguished_name(&new.certificate)])
            .env("JUNCTION_STORE_PASSWORD", &new.store_password)
            .env("JUNCTION_KEY_PASSWORD", &new.key_password);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let output = command.output().map_err(|e| format!("Could not run keytool: {e}"))?;
        if output.status.success() {
            return Ok(());
        }
        let text = String::from_utf8_lossy(&output.stdout).into_owned() + &String::from_utf8_lossy(&output.stderr);
        let message = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("Picked up")).map(|l| l.trim_start_matches("keytool error: ")).last().unwrap_or("keytool failed").to_owned();
        Err(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "runs the JDK's keytool"]
    fn creates_a_keystore() {
        let dir = std::env::temp_dir().join(format!("junction-keytool-{}", std::process::id()));
        let new = NewKeystore {
            store_file: dir.join("upload.jks"),
            store_password: "p&a^s% w\"d".into(),
            store_confirm: String::new(),
            key_password: "p&a^s% w\"d".into(),
            certificate: ["Ann Lee".into(), String::new(), "Acme, Inc.".into(), String::new(), String::new(), "CN".into()],
            ..Default::default()
        };
        JdkKeyTool { build_root: dir.clone() }.create(&new, 365).unwrap();
        assert!(new.store_file.is_file());
        // A second time: the alias exists.
        assert!(JdkKeyTool { build_root: dir.clone() }.create(&new, 365).unwrap_err().contains("key0"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
