//! Code hosting (Settings › Version Control › GitHub / GitLab): accounts,
//! pull / merge requests, sharing projects and gists, over the REST APIs.

pub mod account;
pub mod github;
pub mod gitlab;

use anyhow::{Result, anyhow};

/// An HTTP agent for `url`: uses HTTPS_PROXY / HTTP_PROXY unless the host
/// is listed in NO_PROXY (ureq's own env handling ignores NO_PROXY).
pub(crate) fn agent_for(url: &str) -> ureq::Agent {
    let mut builder = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(30)).user_agent("Junction");
    let host = url.split_once("://").map_or(url, |(_, r)| r).split(['/', ':']).next().unwrap_or_default();
    let env = |k: &str| std::env::var(k).or_else(|_| std::env::var(k.to_lowercase())).ok().filter(|v| !v.is_empty());
    let bypass = env("NO_PROXY").is_some_and(|list| no_proxy_matches(&list, host));
    let proxy = if url.starts_with("https") { env("HTTPS_PROXY") } else { env("HTTP_PROXY") };
    if let (false, Some(proxy)) = (bypass, proxy) {
        if let Ok(proxy) = ureq::Proxy::new(proxy) {
            builder = builder.proxy(proxy);
        }
    }
    builder.build()
}

fn no_proxy_matches(list: &str, host: &str) -> bool {
    list.split(',').map(str::trim).filter(|e| !e.is_empty()).any(|entry| {
        let entry = entry.trim_start_matches("*.");
        entry == "*" || host == entry || host.ends_with(&format!(".{}", entry.trim_start_matches('.')))
            || (entry.contains('/') && host.starts_with("127.") && entry.starts_with("127."))
    })
}

/// Turns a ureq error into the API's own message when it sent one.
pub(crate) fn api_error(error: ureq::Error) -> anyhow::Error {
    match error {
        ureq::Error::Status(code, response) => {
            let body = response.into_string().unwrap_or_default();
            let message = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(str::to_owned))
                .unwrap_or(body);
            match code {
                401 => anyhow!("Authentication failed: the token is invalid or expired"),
                403 if message.contains("rate limit") => anyhow!("API rate limit exceeded; log in to raise it"),
                _ => anyhow!("{code}: {message}"),
            }
        }
        ureq::Error::Transport(t) => anyhow!("Cannot reach the server: {t}"),
    }
}

pub(crate) type ApiResult<T> = Result<T>;

#[cfg(test)]
mod tests {
    #[test]
    fn no_proxy_list() {
        let list = "localhost,127.0.0.1,.svc.cluster.local,*.corp.com,10.0.0.0/8";
        assert!(super::no_proxy_matches(list, "127.0.0.1"));
        assert!(super::no_proxy_matches(list, "git.corp.com"));
        assert!(super::no_proxy_matches(list, "a.svc.cluster.local"));
        assert!(!super::no_proxy_matches(list, "api.github.com"));
    }
}
