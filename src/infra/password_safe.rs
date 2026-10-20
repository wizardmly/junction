//! The system's password safe, as IntelliJ's `PasswordSafe` uses it: the
//! macOS Keychain; on Linux the Secret Service through `secret-tool`
//! (libsecret); on Windows, and on Linux without a Secret Service, the
//! `passwords.json` file in the settings folder, sealed on Windows with the
//! user's data protection (DPAPI) and readable only by the user elsewhere
//! (IntelliJ's own fallback is a KeePass file next to its master key).

use crate::domain::password_safe::PasswordSafe;

/// The service the passwords are filed under.
const SERVICE: &str = "Junction Studio";

pub struct SystemPasswordSafe;

impl PasswordSafe for SystemPasswordSafe {
    fn get(&self, key: &str) -> Option<String> {
        #[cfg(target_os = "macos")]
        return keychain::get(key);
        #[cfg(not(target_os = "macos"))]
        {
            #[cfg(target_os = "linux")]
            if let Some(found) = secret_tool::get(key) {
                return found;
            }
            file::get(key)
        }
    }

    fn set(&self, key: &str, password: Option<&str>) {
        #[cfg(target_os = "macos")]
        keychain::set(key, password);
        #[cfg(not(target_os = "macos"))]
        {
            #[cfg(target_os = "linux")]
            if secret_tool::set(key, password) {
                // Nothing of the key stays in the file.
                if file::get(key).is_some() {
                    file::set(key, None);
                }
                return;
            }
            file::set(key, password);
        }
    }
}

#[cfg(target_os = "macos")]
mod keychain {
    use security_framework::passwords::{delete_generic_password, get_generic_password, set_generic_password};

    pub fn get(key: &str) -> Option<String> {
        get_generic_password(super::SERVICE, key).ok().and_then(|bytes| String::from_utf8(bytes).ok())
    }

    pub fn set(key: &str, password: Option<&str>) {
        match password {
            Some(password) => drop(set_generic_password(super::SERVICE, key, password.as_bytes())),
            None => drop(delete_generic_password(super::SERVICE, key)),
        }
    }
}

/// `secret-tool` of libsecret-tools; `None` / false when it isn't there or
/// no Secret Service answers, so the file takes over.
#[cfg(target_os = "linux")]
mod secret_tool {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    const ATTRIBUTE: &str = "junction-key";

    /// `Some(None)`: the safe works and has no such password.
    pub fn get(key: &str) -> Option<Option<String>> {
        let out = Command::new("secret-tool").args(["lookup", "service", super::SERVICE, ATTRIBUTE, key]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
        if out.status.success() {
            return Some(String::from_utf8(out.stdout).ok());
        }
        // Exit 1 with nothing said is "not found"; anything else (no D-Bus,
        // no keyring) leaves it to the file.
        (out.status.code() == Some(1) && out.stdout.is_empty() && available()).then_some(None)
    }

    pub fn set(key: &str, password: Option<&str>) -> bool {
        if !available() {
            return false;
        }
        let Some(password) = password else {
            return Command::new("secret-tool").args(["clear", "service", super::SERVICE, ATTRIBUTE, key]).stdin(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
        };
        // The password goes in on stdin, never on the command line.
        let label = format!("{} {key}", super::SERVICE);
        let Ok(mut child) = Command::new("secret-tool").args(["store", "--label", &label, "service", super::SERVICE, ATTRIBUTE, key]).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else {
            return false;
        };
        let written = child.stdin.take().is_some_and(|mut stdin| stdin.write_all(password.as_bytes()).is_ok());
        child.wait().is_ok_and(|s| s.success()) && written
    }

    /// Whether a Secret Service is there to talk to: a lookup of a key no
    /// one stores succeeds or reports not found quietly.
    fn available() -> bool {
        use std::sync::OnceLock;
        static AVAILABLE: OnceLock<bool> = OnceLock::new();
        *AVAILABLE.get_or_init(|| {
            Command::new("secret-tool")
                .args(["search", "service", super::SERVICE, ATTRIBUTE, "-"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .output()
                .is_ok_and(|out| out.status.success() || (out.status.code() == Some(1) && out.stderr.is_empty()))
        })
    }
}

#[cfg(not(target_os = "macos"))]
mod file {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn path() -> Option<PathBuf> {
        crate::settings::config_dir().map(|d| d.join("passwords.json"))
    }

    fn read() -> BTreeMap<String, String> {
        path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn get(key: &str) -> Option<String> {
        read().get(key).and_then(|sealed| super::seal::open(sealed))
    }

    pub fn set(key: &str, password: Option<&str>) {
        let Some(path) = path() else { return };
        let mut all = read();
        match password.and_then(super::seal::seal) {
            Some(sealed) => drop(all.insert(key.to_owned(), sealed)),
            None => drop(all.remove(key)),
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let Ok(json) = serde_json::to_vec_pretty(&all) else { return };
        write_private(&path, &json);
    }

    /// Written so only the user can read it.
    fn write_private(path: &std::path::Path, bytes: &[u8]) {
        #[cfg(unix)]
        {
            use std::io::Write as _;
            use std::os::unix::fs::OpenOptionsExt as _;
            let file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path);
            if let Ok(mut file) = file {
                use std::os::unix::fs::PermissionsExt as _;
                let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
                let _ = file.write_all(bytes);
            }
        }
        #[cfg(not(unix))]
        let _ = std::fs::write(path, bytes);
    }
}

/// How a password is kept in the file: sealed with DPAPI on Windows, as
/// is elsewhere; hex either way.
#[cfg(not(target_os = "macos"))]
mod seal {
    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn unhex(text: &str) -> Option<Vec<u8>> {
        (0..text.len()).step_by(2).map(|i| text.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok())).collect()
    }

    #[cfg(not(windows))]
    pub fn seal(password: &str) -> Option<String> {
        Some(hex(password.as_bytes()))
    }

    #[cfg(not(windows))]
    pub fn open(sealed: &str) -> Option<String> {
        String::from_utf8(unhex(sealed)?).ok()
    }

    #[cfg(windows)]
    pub fn seal(password: &str) -> Option<String> {
        dpapi::protect(password.as_bytes()).map(|b| hex(&b))
    }

    #[cfg(windows)]
    pub fn open(sealed: &str) -> Option<String> {
        String::from_utf8(dpapi::unprotect(&unhex(sealed)?)?).ok()
    }

    #[cfg(windows)]
    mod dpapi {
        use windows_sys::Win32::Foundation::{HLOCAL, LocalFree};
        use windows_sys::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData};

        fn blob(bytes: &[u8]) -> CRYPT_INTEGER_BLOB {
            CRYPT_INTEGER_BLOB { cbData: bytes.len() as u32, pbData: bytes.as_ptr() as *mut u8 }
        }

        /// Takes the bytes out of a blob Windows allocated, and frees it.
        unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
            let bytes = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
            unsafe { LocalFree(out.pbData as HLOCAL) };
            bytes
        }

        pub fn protect(data: &[u8]) -> Option<Vec<u8>> {
            let input = blob(data);
            let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
            let ok = unsafe { CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out) };
            (ok != 0).then(|| unsafe { take(out) })
        }

        pub fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
            let input = blob(data);
            let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
            let ok = unsafe { CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out) };
            (ok != 0).then(|| unsafe { take(out) })
        }
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn seals_and_opens() {
            let sealed = super::seal("p&a^s% w\"d 密码").unwrap();
            assert!(!sealed.contains("p&a"));
            assert_eq!(super::open(&sealed).as_deref(), Some("p&a^s% w\"d 密码"));
            assert_eq!(super::open("zz"), None);
        }
    }
}
