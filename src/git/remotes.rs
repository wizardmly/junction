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

pub fn remove(repository: &Repository, name: &str) -> Result<()> {
    repository.run(["remote", "remove", name])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
