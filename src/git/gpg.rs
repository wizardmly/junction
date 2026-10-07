//! GPG keys for commit signing (Settings › Git › Configure GPG Key).

use super::Repository;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretKey {
    pub id: String,
    pub user: String,
}

/// Secret keys from `gpg --list-secret-keys`; empty when gpg isn't installed.
pub fn secret_keys(repository: &Repository) -> Vec<SecretKey> {
    let program = repository.run(["config", "--get", "gpg.program"]).map(|p| p.trim().to_owned()).unwrap_or_default();
    let program = if program.is_empty() { "gpg".to_owned() } else { program };
    let mut command = std::process::Command::new(program);
    command.args(["--list-secret-keys", "--with-colons", "--keyid-format=long"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(0x0800_0000);
    }
    let Ok(output) = command.output() else { return Vec::new() };
    parse_keys(&String::from_utf8_lossy(&output.stdout))
}

fn parse_keys(output: &str) -> Vec<SecretKey> {
    let mut keys: Vec<SecretKey> = Vec::new();
    for line in output.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        match fields.first() {
            Some(&"sec") => keys.push(SecretKey { id: fields.get(4).unwrap_or(&"").to_string(), user: String::new() }),
            Some(&"uid") => {
                if let Some(key) = keys.last_mut().filter(|k| k.user.is_empty()) {
                    key.user = fields.get(9).unwrap_or(&"").replace("\\x3a", ":");
                }
            }
            _ => {}
        }
    }
    keys
}

/// The repository's signing setup: `commit.gpgSign` and `user.signingKey`.
pub fn signing_config(repository: &Repository) -> (bool, String) {
    let get = |key: &str| repository.run(["config", "--get", key]).map(|v| v.trim().to_owned()).unwrap_or_default();
    (get("commit.gpgsign") == "true", get("user.signingkey"))
}

pub fn set_signing_config(repository: &Repository, sign: bool, key: &str) -> anyhow::Result<()> {
    repository.run(["config", "commit.gpgsign", if sign { "true" } else { "false" }])?;
    if key.is_empty() {
        repository.run(["config", "--unset", "user.signingkey"]).ok();
    } else {
        repository.run(["config", "user.signingkey", key])?;
        // A GPG key id needs the OpenPGP format, whatever the global default (e.g. SSH signing).
        repository.run(["config", "gpg.format", "openpgp"])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_colon_listing() {
        let output = "sec:u:4096:1:ABCDEF0123456789:1700000000:::u:::scESC:::+:::23::0:\nfpr:::::::::0123:\nuid:u::::1700000000::HASH::Jake <jake@example.com>::::::::::0:\nssb:u:4096:1:1111:1700000000::::::e:::+:::23:\n";
        assert_eq!(parse_keys(output), [SecretKey { id: "ABCDEF0123456789".into(), user: "Jake <jake@example.com>".into() }]);
    }
}
