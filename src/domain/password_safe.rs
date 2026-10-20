//! Passwords kept by the system, IntelliJ's `PasswordSafe`: the Windows
//! user's data protection, the macOS Keychain, the Linux Secret Service.

/// Keeps a password under a key.
pub trait PasswordSafe: Send + Sync {
    fn get(&self, key: &str) -> Option<String>;
    /// `None` forgets the key's password.
    fn set(&self, key: &str, password: Option<&str>);
}
