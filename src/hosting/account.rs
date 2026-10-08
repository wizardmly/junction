//! Hosting accounts: server, login and access token. Kept in the config
//! directory in a file only the user can read.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Service {
    GitHub,
    GitLab,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub service: Service,
    /// `github.com`, `gitlab.com` or an Enterprise / self-managed host.
    pub server: String,
    pub login: String,
    pub token: String,
}

impl Account {
    /// The REST API root for this account's server. `JUNCTION_GITHUB_API`
    /// overrides github.com's (used by tests against a local server).
    pub fn api_base(&self) -> String {
        match self.service {
            Service::GitHub if self.server == "github.com" => {
                std::env::var("JUNCTION_GITHUB_API").unwrap_or_else(|_| "https://api.github.com".into())
            }
            Service::GitHub if self.server.starts_with("http") => format!("{}/api/v3", self.server.trim_end_matches('/')),
            Service::GitHub => format!("https://{}/api/v3", self.server),
            Service::GitLab if self.server.starts_with("http") => format!("{}/api/v4", self.server.trim_end_matches('/')),
            Service::GitLab => format!("https://{}/api/v4", self.server),
        }
    }

    /// Does this account serve a remote on `host`?
    pub fn matches_host(&self, host: &str) -> bool {
        let server = self.server.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/');
        server == host
    }
}

fn path() -> Option<PathBuf> {
    Some(crate::settings::config_dir()?.join("accounts.json"))
}

pub fn load() -> Vec<Account> {
    path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(accounts: &[Account]) -> anyhow::Result<()> {
    let Some(path) = path() else { anyhow::bail!("no config directory") };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(accounts)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// The account for a remote's host, GitHub first.
pub fn for_host(host: &str) -> Option<Account> {
    load().into_iter().find(|a| a.matches_host(host))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_roots() {
        let mut a = Account { service: Service::GitHub, server: "github.com".into(), login: "me".into(), token: "t".into() };
        if std::env::var("JUNCTION_GITHUB_API").is_err() {
            assert_eq!(a.api_base(), "https://api.github.com");
        }
        a.server = "ghe.corp.com".into();
        assert_eq!(a.api_base(), "https://ghe.corp.com/api/v3");
        a.service = Service::GitLab;
        a.server = "gitlab.com".into();
        assert_eq!(a.api_base(), "https://gitlab.com/api/v4");
        assert!(a.matches_host("gitlab.com"));
    }
}
