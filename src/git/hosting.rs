//! "Open on GitHub / GitLab / Bitbucket": web URLs for commits and files
//! from a remote's URL.

use super::Repository;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    GitHub,
    GitLab,
    Bitbucket,
    /// Gitea, Codeberg and other GitHub-style hosts.
    Other,
}

impl Host {
    pub fn name(self) -> &'static str {
        match self {
            Host::GitHub => "GitHub",
            Host::GitLab => "GitLab",
            Host::Bitbucket => "Bitbucket",
            Host::Other => "Web",
        }
    }
}

/// A remote as a browsable repository: `https://host/owner/repo`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebRepo {
    pub base: String,
    pub host: Host,
}

/// Turns `git@github.com:o/r.git`, `ssh://git@host:22/o/r`, `https://u@host/o/r.git`
/// into `https://host/o/r`.
pub fn parse_remote(url: &str) -> Option<WebRepo> {
    let url = url.trim();
    let (host, path) = if let Some(rest) = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")) {
        let rest = rest.rsplit_once('@').map_or(rest, |(_, r)| r);
        rest.split_once('/')?
    } else if let Some(rest) = url.strip_prefix("ssh://") {
        let rest = rest.rsplit_once('@').map_or(rest, |(_, r)| r);
        let (host, path) = rest.split_once('/')?;
        (host.split(':').next()?, path)
    } else if let Some((user_host, path)) = url.split_once(':') {
        // scp-like: git@host:owner/repo.git
        if user_host.contains('/') {
            return None;
        }
        (user_host.rsplit_once('@').map_or(user_host, |(_, h)| h), path)
    } else {
        return None;
    };
    let host = host.split(':').next()?;
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    if host.is_empty() || path.is_empty() {
        return None;
    }
    let kind = if host.contains("github") {
        Host::GitHub
    } else if host.contains("gitlab") {
        Host::GitLab
    } else if host.contains("bitbucket") {
        Host::Bitbucket
    } else {
        Host::Other
    };
    Some(WebRepo { base: format!("https://{host}/{path}"), host: kind })
}

impl WebRepo {
    pub fn commit_url(&self, hash: &str) -> String {
        match self.host {
            Host::GitLab => format!("{}/-/commit/{hash}", self.base),
            Host::Bitbucket => format!("{}/commits/{hash}", self.base),
            _ => format!("{}/commit/{hash}", self.base),
        }
    }

    /// A file at `revision`, optionally at a line range (1-based).
    pub fn file_url(&self, revision: &str, path: &str, lines: Option<(usize, usize)>) -> String {
        let anchor = match (self.host, lines) {
            (_, None) => String::new(),
            (Host::Bitbucket, Some((a, b))) => format!("#lines-{a}:{b}"),
            (Host::GitLab, Some((a, b))) if a != b => format!("#L{a}-{b}"),
            (_, Some((a, b))) if a != b => format!("#L{a}-L{b}"),
            (_, Some((a, _))) => format!("#L{a}"),
        };
        match self.host {
            Host::GitLab => format!("{}/-/blob/{revision}/{path}{anchor}", self.base),
            Host::Bitbucket => format!("{}/src/{revision}/{path}{anchor}", self.base),
            _ => format!("{}/blob/{revision}/{path}{anchor}", self.base),
        }
    }
}

/// The web repository of the current branch's remote, else `origin`, else
/// the first remote.
pub fn web_repo(repository: &Repository) -> Option<WebRepo> {
    let remote = repository
        .run(["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"])
        .ok()
        .and_then(|u| u.trim().split_once('/').map(|(r, _)| r.to_owned()))
        .or_else(|| {
            let remotes = repository.run(["remote"]).ok()?;
            let list: Vec<&str> = remotes.lines().collect();
            list.iter().find(|r| **r == "origin").or(list.first()).map(|r| r.to_string())
        })?;
    let url = repository.run(["remote", "get-url", &remote]).ok()?;
    parse_remote(&url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_remote_urls() {
        let gh = parse_remote("git@github.com:owner/repo.git").unwrap();
        assert_eq!(gh, WebRepo { base: "https://github.com/owner/repo".into(), host: Host::GitHub });
        assert_eq!(parse_remote("https://user@gitlab.com/group/sub/proj.git").unwrap().base, "https://gitlab.com/group/sub/proj");
        assert_eq!(parse_remote("ssh://git@bitbucket.org:22/team/repo").unwrap().host, Host::Bitbucket);
        assert_eq!(parse_remote("https://codeberg.org/a/b/").unwrap().base, "https://codeberg.org/a/b");
        assert_eq!(parse_remote("/local/path/repo.git"), None);
    }

    #[test]
    fn builds_urls() {
        let gh = parse_remote("https://github.com/o/r").unwrap();
        assert_eq!(gh.commit_url("abc"), "https://github.com/o/r/commit/abc");
        assert_eq!(gh.file_url("main", "src/a.rs", Some((3, 5))), "https://github.com/o/r/blob/main/src/a.rs#L3-L5");
        let gl = parse_remote("git@gitlab.com:g/p.git").unwrap();
        assert_eq!(gl.commit_url("abc"), "https://gitlab.com/g/p/-/commit/abc");
        assert_eq!(gl.file_url("v1", "x", Some((2, 2))), "https://gitlab.com/g/p/-/blob/v1/x#L2");
    }
}
