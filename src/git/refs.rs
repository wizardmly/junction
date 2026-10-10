use std::collections::HashMap;

use anyhow::Result;

use super::Repository;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RefKind {
    LocalBranch,
    RemoteBranch,
    Tag,
}

/// A branch or tag pointing at a commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefName {
    pub kind: RefKind,
    /// Short name, e.g. `main`, `origin/main`, `v1.0`.
    pub name: String,
    /// Full name, e.g. `refs/heads/main`.
    pub full_name: String,
    pub target: String,
    /// Upstream short name for local branches, e.g. `origin/main`.
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
}

impl RefName {
    /// For remote branches, the remote part (`origin` of `origin/main`).
    pub fn remote(&self) -> Option<&str> {
        (self.kind == RefKind::RemoteBranch)
            .then(|| self.full_name.strip_prefix("refs/remotes/")?.split('/').next())
            .flatten()
    }

    /// The name without the remote prefix (`main` of `origin/main`).
    pub fn branch_without_remote(&self) -> &str {
        match self.remote() {
            Some(remote) => self.name.strip_prefix(remote).and_then(|n| n.strip_prefix('/')).unwrap_or(&self.name),
            None => &self.name,
        }
    }
}

/// Every branch and tag, plus where HEAD points.
#[derive(Clone, Debug, Default)]
pub struct RepositoryRefs {
    pub head_commit: Option<String>,
    /// `None` when HEAD is detached.
    pub current_branch: Option<String>,
    pub refs: Vec<RefName>,
    /// Recently checked-out local branches, newest first (the popup's Recent group).
    pub recent: Vec<String>,
    /// Starred refs (full names), listed first in each group.
    pub favorites: std::collections::HashSet<String>,
    /// The configured remotes (`git remote`), origin first.
    pub remotes: Vec<String>,
    by_commit: HashMap<String, Vec<usize>>,
}

impl RepositoryRefs {
    pub fn load(repository: &Repository) -> Result<Self> {
        let head_commit = repository
            .run(["rev-parse", "--verify", "-q", "HEAD"])
            .ok()
            .map(|hash| hash.trim().to_owned())
            .filter(|hash| !hash.is_empty());
        let current_branch = repository
            .run(["symbolic-ref", "-q", "--short", "HEAD"])
            .ok()
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty());

        let output = repository.run([
            "for-each-ref",
            "--format=%(objectname)%00%(*objectname)%00%(refname)%00%(refname:short)%00%(upstream:short)%00%(upstream:track,nobracket)",
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ])?;
        let mut refs = parse_for_each_ref(&output);
        refs.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.name.cmp(&b.name)));
        let reflog = repository.run(["reflog", "-n", "300", "--format=%gs", "HEAD"]).unwrap_or_default();
        let mut this = Self::new(head_commit, current_branch, refs);
        this.recent = recent_branches(&reflog, |name| this.find(&format!("refs/heads/{name}")).is_some());
        this.favorites = load_favorites(repository);
        let mut remotes: Vec<String> = repository.run(["remote"]).unwrap_or_default().lines().map(str::trim).filter(|r| !r.is_empty()).map(str::to_owned).collect();
        remotes.sort_by_key(|r| r != "origin");
        this.remotes = remotes;
        Ok(this)
    }

    pub fn new(head_commit: Option<String>, current_branch: Option<String>, refs: Vec<RefName>) -> Self {
        let mut by_commit: HashMap<String, Vec<usize>> = HashMap::new();
        for (ix, reference) in refs.iter().enumerate() {
            by_commit.entry(reference.target.clone()).or_default().push(ix);
        }
        Self { head_commit, current_branch, refs, recent: Vec::new(), favorites: Default::default(), remotes: Vec::new(), by_commit }
    }

    /// Refs pointing at `hash`, ordered the way IntelliJ labels a row:
    /// HEAD, the current branch, other local branches, remote branches, tags.
    pub fn for_commit(&self, hash: &str) -> Vec<&RefName> {
        let mut refs: Vec<&RefName> = self
            .by_commit
            .get(hash)
            .into_iter()
            .flatten()
            .map(|&ix| &self.refs[ix])
            .collect();
        refs.sort_by_key(|r| (Some(&r.name) != self.current_branch.as_ref() || r.kind != RefKind::LocalBranch, r.kind));
        refs
    }

    pub fn local_branches(&self) -> impl Iterator<Item = &RefName> {
        self.refs.iter().filter(|r| r.kind == RefKind::LocalBranch)
    }

    pub fn remote_branches(&self) -> impl Iterator<Item = &RefName> {
        // `origin/HEAD` is a symbolic pointer, not a branch the user works with.
        self.refs.iter().filter(|r| r.kind == RefKind::RemoteBranch && !r.name.ends_with("/HEAD"))
    }

    pub fn tags(&self) -> impl Iterator<Item = &RefName> {
        self.refs.iter().filter(|r| r.kind == RefKind::Tag)
    }

    pub fn find(&self, full_name: &str) -> Option<&RefName> {
        self.refs.iter().find(|r| r.full_name == full_name)
    }
}

fn parse_for_each_ref(output: &str) -> Vec<RefName> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\0');
            let object = fields.next()?;
            let peeled = fields.next()?;
            let full_name = fields.next()?.to_owned();
            // `origin/HEAD` is a pointer to the default branch, not a branch (IntelliJ hides it).
            if full_name.starts_with("refs/remotes/") && full_name.ends_with("/HEAD") {
                return None;
            }
            let name = fields.next()?.to_owned();
            let upstream = fields.next().filter(|u| !u.is_empty()).map(str::to_owned);
            let track = fields.next().unwrap_or_default();
            let kind = if full_name.starts_with("refs/heads/") {
                RefKind::LocalBranch
            } else if full_name.starts_with("refs/remotes/") {
                RefKind::RemoteBranch
            } else {
                RefKind::Tag
            };
            // Annotated tags point at a tag object; label the commit it peels to.
            let target = if peeled.is_empty() { object } else { peeled }.to_owned();
            let (ahead, behind) = parse_track(track);
            Some(RefName { kind, name, full_name, target, upstream, ahead, behind })
        })
        .collect()
}

fn favorites_path(repository: &Repository) -> std::path::PathBuf {
    repository.git_dir().join("junction").join("favorites")
}

/// Starred branches and tags. Until the user stars anything, `main` /
/// `master` and their remote counterparts are favorites, as in IntelliJ.
pub fn load_favorites(repository: &Repository) -> std::collections::HashSet<String> {
    match std::fs::read_to_string(favorites_path(repository)) {
        Ok(text) => text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect(),
        Err(_) => ["refs/heads/main", "refs/heads/master", "refs/remotes/origin/main", "refs/remotes/origin/master"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
    }
}

pub fn set_favorite(repository: &Repository, full_name: &str, favorite: bool) -> Result<()> {
    let mut favorites = load_favorites(repository);
    if favorite {
        favorites.insert(full_name.to_owned());
    } else {
        favorites.remove(full_name);
    }
    let mut lines: Vec<&String> = favorites.iter().collect();
    lines.sort();
    let path = favorites_path(repository);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, lines.iter().map(|l| format!("{l}\n")).collect::<String>())?;
    Ok(())
}

/// Up to five existing branches from "checkout: moving from A to B" reflog
/// entries, newest first, as IntelliJ's Recent group lists them.
fn recent_branches(reflog: &str, exists: impl Fn(&str) -> bool) -> Vec<String> {
    let mut recent: Vec<String> = Vec::new();
    for line in reflog.lines() {
        let Some(rest) = line.strip_prefix("checkout: moving from ") else { continue };
        let Some((from, to)) = rest.split_once(" to ") else { continue };
        for name in [to, from] {
            if recent.len() < 5 && exists(name) && !recent.iter().any(|r| r == name) {
                recent.push(name.to_owned());
            }
        }
    }
    recent
}

/// Parses `ahead 2, behind 1` from `%(upstream:track,nobracket)`.
fn parse_track(track: &str) -> (u32, u32) {
    let mut ahead = 0;
    let mut behind = 0;
    for part in track.split(", ") {
        if let Some(n) = part.strip_prefix("ahead ") {
            ahead = n.parse().unwrap_or(0);
        } else if let Some(n) = part.strip_prefix("behind ") {
            behind = n.parse().unwrap_or(0);
        }
    }
    (ahead, behind)
}

/// Whether `reference` (a full ref name) is merged into `target`.
pub fn is_merged(repository: &Repository, reference: &str, target: &str) -> bool {
    repository.run(["merge-base", "--is-ancestor", reference, target]).is_ok()
}

/// Up to `limit` commits on `reference` that aren't in HEAD, as "hash subject".
pub fn unmerged_commits(repository: &Repository, reference: &str, limit: usize) -> Vec<String> {
    repository
        .run(["log", "--format=%h %s", "-n", &limit.to_string(), &format!("HEAD..{reference}")])
        .map(|out| out.lines().map(str::to_owned).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_branches_from_reflog() {
        let reflog = "commit: x\ncheckout: moving from feature to main\ncheckout: moving from main to feature\ncheckout: moving from gone to main\n";
        assert_eq!(recent_branches(reflog, |n| n != "gone"), ["main", "feature"]);
    }

    #[test]
    fn parses_refs_and_tracking() {
        let output = "aaa\0\0refs/heads/main\0main\0origin/main\0ahead 2, behind 1\n\
                      bbb\0ccc\0refs/tags/v1\0v1\0\0\n\
                      ddd\0\0refs/remotes/origin/feature/x\0origin/feature/x\0\0\n";
        let refs = parse_for_each_ref(output);
        assert_eq!(refs[0].kind, RefKind::LocalBranch);
        assert_eq!((refs[0].ahead, refs[0].behind), (2, 1));
        assert_eq!(refs[0].upstream.as_deref(), Some("origin/main"));
        assert_eq!(refs[1].target, "ccc");
        assert_eq!(refs[2].remote(), Some("origin"));
        assert_eq!(refs[2].branch_without_remote(), "feature/x");
    }

    #[test]
    fn merged_and_unmerged_commits() {
        let t = crate::git::test_support::TestRepo::new("refs-merged");
        t.git(&["branch", "old"]);
        t.git(&["checkout", "-q", "-b", "topic"]);
        t.commit("t.txt", "t", "topic work");
        t.git(&["checkout", "-q", "main"]);
        assert!(is_merged(&t.repo, "refs/heads/old", "HEAD"));
        assert!(!is_merged(&t.repo, "refs/heads/topic", "HEAD"));
        let commits = unmerged_commits(&t.repo, "refs/heads/topic", 20);
        assert_eq!(commits.len(), 1);
        assert!(commits[0].ends_with(" topic work"));
        assert!(unmerged_commits(&t.repo, "refs/heads/old", 20).is_empty());
    }
}
