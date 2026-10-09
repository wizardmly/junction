//! Remotes, for the Pull and Manage Remotes dialogs.

use anyhow::Result;

use super::Repository;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    pub fetch_url: String,
    /// Shown only when it differs from the fetch URL.
    pub push_url: Option<String>,
}

pub fn list(repository: &Repository) -> Result<Vec<Remote>> {
    Ok(parse(&repository.run(["remote", "-v"])?))
}

fn parse(output: &str) -> Vec<Remote> {
    let mut remotes: Vec<Remote> = Vec::new();
    for line in output.lines() {
        let mut parts = line.split_whitespace();
        let (Some(name), Some(url), Some(kind)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let index = match remotes.iter().position(|r| r.name == name) {
            Some(ix) => ix,
            None => {
                remotes.push(Remote { name: name.to_owned(), fetch_url: String::new(), push_url: None });
                remotes.len() - 1
            }
        };
        let remote = &mut remotes[index];
        if kind == "(fetch)" {
            remote.fetch_url = url.to_owned();
        } else {
            remote.push_url = Some(url.to_owned());
        }
    }
    for remote in &mut remotes {
        if remote.push_url.as_deref() == Some(remote.fetch_url.as_str()) {
            remote.push_url = None;
        }
    }
    remotes
}

pub fn add(repository: &Repository, name: &str, url: &str) -> Result<()> {
    repository.run(["remote", "add", name, url])?;
    Ok(())
}

/// Edit Remote: renames and/or changes the URL.
pub fn edit(repository: &Repository, old_name: &str, name: &str, url: &str) -> Result<()> {
    if old_name != name {
        repository.run(["remote", "rename", old_name, name])?;
    }
    repository.run(["remote", "set-url", name, url])?;
    Ok(())
}

/// IntelliJ's check when defining a remote: `git ls-remote` must reach it.
pub fn check_url(repository: &Repository, url: &str) -> Result<()> {
    repository.run(["ls-remote", "--heads", url]).map_err(|error| {
        let text = error.to_string();
        let reason = text.lines().filter(|l| !l.trim().is_empty()).find(|l| l.contains("fatal:")).or(text.lines().last()).unwrap_or_default();
        anyhow::anyhow!("Remote URL test failed: {}", reason.split_once("fatal: ").map_or(reason, |(_, r)| r).trim())
    })?;
    Ok(())
}

pub fn remove(repository: &Repository, name: &str) -> Result<()> {
    repository.run(["remote", "remove", name])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_remote_urls() {
        let dir = std::env::temp_dir().join(format!("junction-test-remote-url-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(std::process::Command::new("git").arg("-C").arg(&dir).args(["init", "-q"]).status().unwrap().success());
        let repo = Repository::discover(&dir, crate::git::GitConsole::default()).unwrap();
        assert!(check_url(&repo, dir.to_str().unwrap()).is_ok());
        let error = check_url(&repo, "/nonexistent/repo").unwrap_err().to_string();
        assert!(error.starts_with("Remote URL test failed: ") && error.contains("/nonexistent/repo"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_remote_v() {
        let remotes = parse(
            "origin\thttps://a/x.git (fetch)\norigin\thttps://a/x.git (push)\n\
             up\tgit@b:y.git (fetch)\nup\tgit@c:y.git (push)\n",
        );
        assert_eq!(remotes.len(), 2);
        assert_eq!(remotes[0].push_url, None);
        assert_eq!(remotes[1].push_url.as_deref(), Some("git@c:y.git"));
    }
}
