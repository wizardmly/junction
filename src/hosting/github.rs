//! GitHub REST API v3 (github.com and Enterprise): the calls behind the
//! Pull Requests tool window, Share Project on GitHub and Create Gist.

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::account::Account;
use super::{ApiResult, agent_for, api_error};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct User {
    pub login: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Repo {
    pub full_name: String,
    pub name: String,
    #[serde(default)]
    pub private: bool,
    #[serde(default)]
    pub description: Option<String>,
    pub clone_url: String,
    #[serde(default)]
    pub ssh_url: String,
    #[serde(default)]
    pub html_url: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Label {
    pub name: String,
    #[serde(default)]
    pub color: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct BranchRef {
    #[serde(rename = "ref")]
    pub name: String,
    pub sha: String,
    #[serde(default)]
    pub label: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    pub state: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub merged_at: Option<String>,
    pub user: User,
    pub head: BranchRef,
    pub base: BranchRef,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub mergeable: Option<bool>,
    #[serde(default)]
    pub assignees: Vec<User>,
    #[serde(default)]
    pub requested_reviewers: Vec<User>,
}

impl PullRequest {
    /// "Open", "Draft", "Merged" or "Closed", as the list shows it.
    pub fn status(&self) -> &'static str {
        match (self.state.as_str(), self.draft, self.merged_at.is_some()) {
            (_, _, true) => "Merged",
            ("open", true, _) => "Draft",
            ("open", false, _) => "Open",
            _ => "Closed",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct PrFile {
    pub filename: String,
    pub status: String,
    #[serde(default)]
    pub additions: u64,
    #[serde(default)]
    pub deletions: u64,
    #[serde(default)]
    pub previous_filename: Option<String>,
}

/// A conversation comment or a review (with its state) on the timeline.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Comment {
    pub id: u64,
    pub user: User,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    /// Reviews: APPROVED, CHANGES_REQUESTED, COMMENTED.
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub submitted_at: Option<String>,
    /// Review comments: the file and line they're on.
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub line: Option<u64>,
}

impl Comment {
    pub fn time(&self) -> &str {
        self.created_at.as_deref().or(self.submitted_at.as_deref()).unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewEvent {
    Approve,
    RequestChanges,
    Comment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

/// A hosting account's API: GitHub's, or GitLab's mapped onto the same
/// types (see `gitlab.rs`).
pub struct Client {
    pub(super) base: String,
    pub(super) token: String,
    gitlab: bool,
}

impl Client {
    pub fn new(account: &Account) -> Self {
        Self { base: account.api_base(), token: account.token.clone(), gitlab: account.service == super::account::Service::GitLab }
    }

    fn get<T: for<'de> Deserialize<'de>>(&self, path: &str) -> ApiResult<T> {
        let url = format!("{}{path}", self.base);
        let response = agent_for(&url)
            .get(&url)
            .set("Authorization", &format!("Bearer {}", self.token))
            .set("Accept", "application/vnd.github+json")
            .call()
            .map_err(api_error)?;
        Ok(response.into_json()?)
    }

    fn send<T: for<'de> Deserialize<'de>>(&self, method: &str, path: &str, body: serde_json::Value) -> ApiResult<T> {
        let url = format!("{}{path}", self.base);
        let response = agent_for(&url)
            .request(method, &url)
            .set("Authorization", &format!("Bearer {}", self.token))
            .set("Accept", "application/vnd.github+json")
            .send_json(body)
            .map_err(api_error)?;
        Ok(response.into_json()?)
    }

    pub fn user(&self) -> ApiResult<User> {
        if self.gitlab {
            return self.gl_user();
        }
        self.get("/user")
    }

    /// The account's repositories, most recently updated first (Clone dialog).
    pub fn repos(&self) -> ApiResult<Vec<Repo>> {
        if self.gitlab {
            return self.gl_repos();
        }
        self.get("/user/repos?per_page=100&sort=updated")
    }

    /// `state`: open, closed or all.
    pub fn pulls(&self, repo: &str, state: &str) -> ApiResult<Vec<PullRequest>> {
        if self.gitlab {
            return self.gl_pulls(repo, state);
        }
        self.get(&format!("/repos/{repo}/pulls?state={state}&per_page=100&sort=updated&direction=desc"))
    }

    pub fn pull(&self, repo: &str, number: u64) -> ApiResult<PullRequest> {
        if self.gitlab {
            return self.gl_pull(repo, number);
        }
        self.get(&format!("/repos/{repo}/pulls/{number}"))
    }

    pub fn pull_files(&self, repo: &str, number: u64) -> ApiResult<Vec<PrFile>> {
        if self.gitlab {
            return self.gl_pull_files(repo, number);
        }
        self.get(&format!("/repos/{repo}/pulls/{number}/files?per_page=100"))
    }

    /// Conversation comments, reviews and review comments, oldest first.
    pub fn timeline(&self, repo: &str, number: u64) -> ApiResult<Vec<Comment>> {
        if self.gitlab {
            return self.gl_timeline(repo, number);
        }
        let mut all: Vec<Comment> = self.get(&format!("/repos/{repo}/issues/{number}/comments?per_page=100"))?;
        let reviews: Vec<Comment> = self.get(&format!("/repos/{repo}/pulls/{number}/reviews?per_page=100"))?;
        all.extend(reviews.into_iter().filter(|r| r.state.as_deref() != Some("PENDING")));
        let review_comments: Vec<Comment> = self.get(&format!("/repos/{repo}/pulls/{number}/comments?per_page=100"))?;
        all.extend(review_comments);
        all.sort_by(|a, b| a.time().cmp(b.time()));
        Ok(all)
    }

    pub fn add_comment(&self, repo: &str, number: u64, body: &str) -> ApiResult<Comment> {
        if self.gitlab {
            return self.gl_add_comment(repo, number, body);
        }
        self.send("POST", &format!("/repos/{repo}/issues/{number}/comments"), json!({ "body": body }))
    }

    /// A comment on a line of the PR's diff (right side = the new version).
    pub fn add_line_comment(&self, repo: &str, number: u64, commit: &str, path: &str, line: u64, body: &str) -> ApiResult<Comment> {
        if self.gitlab {
            return self.gl_add_line_comment(repo, number, commit, path, line, body);
        }
        self.send(
            "POST",
            &format!("/repos/{repo}/pulls/{number}/comments"),
            json!({ "body": body, "commit_id": commit, "path": path, "line": line, "side": "RIGHT" }),
        )
    }

    pub fn submit_review(&self, repo: &str, number: u64, event: ReviewEvent, body: &str) -> ApiResult<Comment> {
        if self.gitlab {
            return self.gl_submit_review(repo, number, event, body);
        }
        self.send("POST", &format!("/repos/{repo}/pulls/{number}/reviews"), json!({ "event": event, "body": body }))
    }

    /// Merges with the commit message the Merge dialog edited (its first
    /// line is the title), or the server's default when `None`.
    pub fn merge(&self, repo: &str, number: u64, method: MergeMethod, message: Option<&str>) -> ApiResult<serde_json::Value> {
        if self.gitlab {
            return self.gl_merge(repo, number, method, message);
        }
        let method = match method {
            MergeMethod::Merge => "merge",
            MergeMethod::Squash => "squash",
            MergeMethod::Rebase => "rebase",
        };
        let mut body = json!({ "merge_method": method });
        if let Some(message) = message {
            let (title, rest) = message.split_once('\n').unwrap_or((message, ""));
            body["commit_title"] = json!(title.trim());
            body["commit_message"] = json!(rest.trim());
        }
        self.send("PUT", &format!("/repos/{repo}/pulls/{number}/merge"), body)
    }

    /// The numbers of the repository's PRs matching a search qualifier
    /// (`review:approved`, `reviewed-by:@me`), for the list's Review filter.
    pub fn search_pulls(&self, repo: &str, qualifier: &str) -> ApiResult<Vec<u64>> {
        #[derive(Deserialize)]
        struct Item {
            number: u64,
        }
        #[derive(Deserialize)]
        struct Found {
            items: Vec<Item>,
        }
        let query = format!("repo:{repo} is:pr {qualifier}").replace(' ', "+").replace('@', "%40").replace(':', "%3A");
        let found: Found = self.get(&format!("/search/issues?q={query}&per_page=100"))?;
        Ok(found.items.into_iter().map(|i| i.number).collect())
    }

    pub fn create_pull(&self, repo: &str, title: &str, body: &str, head: &str, base: &str, draft: bool) -> ApiResult<PullRequest> {
        if self.gitlab {
            return self.gl_create_pull(repo, title, body, head, base, draft);
        }
        self.send("POST", &format!("/repos/{repo}/pulls"), json!({ "title": title, "body": body, "head": head, "base": base, "draft": draft }))
    }

    /// Share Project on GitHub: a new repository for the account.
    pub fn create_repo(&self, name: &str, private: bool, description: &str) -> ApiResult<Repo> {
        if self.gitlab {
            return self.gl_create_repo(name, private, description);
        }
        self.send("POST", "/user/repos", json!({ "name": name, "private": private, "description": description }))
    }

    /// Create Gist: returns its web URL.
    pub fn create_gist(&self, description: &str, public: bool, files: &[(String, String)]) -> ApiResult<String> {
        if self.gitlab {
            return self.gl_create_gist(description, public, files);
        }
        let files: serde_json::Map<String, serde_json::Value> =
            files.iter().map(|(name, content)| (name.clone(), json!({ "content": content }))).collect();
        let gist: serde_json::Value = self.send("POST", "/gists", json!({ "description": description, "public": public, "files": files }))?;
        Ok(gist.get("html_url").and_then(|u| u.as_str()).unwrap_or_default().to_owned())
    }
}

/// `owner/repo` from a web URL like `https://github.com/owner/repo`.
pub fn repo_path(base: &str) -> Option<String> {
    let rest = base.split_once("://").map_or(base, |(_, r)| r);
    let (_, path) = rest.split_once('/')?;
    let mut parts = path.trim_matches('/').splitn(3, '/');
    let (owner, repo) = (parts.next()?, parts.next()?);
    (!owner.is_empty() && !repo.is_empty()).then(|| format!("{owner}/{repo}"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::hosting::account::Service;
    use std::io::{BufRead as _, BufReader, Read as _, Write as _};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// A tiny HTTP server answering `METHOD path` with canned JSON and
    /// recording request bodies.
    pub(crate) fn mock(routes: Vec<(&'static str, &'static str)>) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line.trim().is_empty() {
                        break;
                    }
                    if let Some(v) = line.to_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let mut parts = request.split_whitespace();
                let key = format!("{} {}", parts.next().unwrap_or(""), parts.next().unwrap_or("").split('?').next().unwrap_or(""));
                log.lock().unwrap().push(format!("{key} {}", String::from_utf8_lossy(&body)));
                let (status, reply) = routes.iter().find(|(k, _)| *k == key).map_or(("404 Not Found", r#"{"message":"Not Found"}"#), |(_, v)| ("200 OK", *v));
                let mut stream = stream;
                write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).unwrap();
            }
        });
        (format!("http://{addr}"), seen)
    }

    pub(crate) fn account(base: &str) -> Account {
        // An Enterprise-style server URL maps to `<server>/api/v3`.
        Account { service: Service::GitHub, server: base.to_owned(), login: "me".into(), token: "tok".into() }
    }

    const PR: &str = r#"{"number":7,"title":"Add login","body":"Fixes #3","state":"open","draft":false,"merged_at":null,
        "user":{"login":"ana"},"head":{"ref":"feature/login","sha":"abc","label":"ana:feature/login"},
        "base":{"ref":"main","sha":"def","label":"o:main"},"created_at":"2026-10-01T10:00:00Z","updated_at":"2026-10-02T10:00:00Z",
        "html_url":"https://github.com/o/r/pull/7","labels":[{"name":"ui","color":"00ff00"}]}"#;

    #[test]
    fn lists_pull_requests_and_reviews() {
        let pulls: &'static str = Box::leak(format!("[{PR}]").into_boxed_str());
        let (base, seen) = mock(vec![
            ("GET /api/v3/user", r#"{"login":"me","name":"Me"}"#),
            ("GET /api/v3/repos/o/r/pulls", pulls),
            ("GET /api/v3/repos/o/r/pulls/7/files", r#"[{"filename":"src/a.rs","status":"modified","additions":3,"deletions":1}]"#),
            ("GET /api/v3/repos/o/r/issues/7/comments", r#"[{"id":1,"user":{"login":"bo"},"body":"Nice","created_at":"2026-10-02T09:00:00Z"}]"#),
            ("GET /api/v3/repos/o/r/pulls/7/reviews", r#"[{"id":2,"user":{"login":"cy"},"body":"","state":"APPROVED","submitted_at":"2026-10-02T11:00:00Z"}]"#),
            ("GET /api/v3/repos/o/r/pulls/7/comments", r#"[{"id":3,"user":{"login":"bo"},"body":"typo","path":"src/a.rs","line":4,"created_at":"2026-10-02T08:00:00Z"}]"#),
            ("POST /api/v3/repos/o/r/pulls/7/reviews", r#"{"id":9,"user":{"login":"me"},"state":"APPROVED"}"#),
            ("PUT /api/v3/repos/o/r/pulls/7/merge", r#"{"merged":true}"#),
            ("GET /api/v3/search/issues", r#"{"total_count":1,"items":[{"number":7}]}"#),
        ]);
        let client = Client::new(&account(&base));
        assert_eq!(client.user().unwrap().login, "me");
        let pulls = client.pulls("o/r", "open").unwrap();
        assert_eq!(pulls[0].number, 7);
        assert_eq!(pulls[0].status(), "Open");
        assert_eq!(pulls[0].head.name, "feature/login");
        assert_eq!(client.pull_files("o/r", 7).unwrap()[0].additions, 3);
        let timeline = client.timeline("o/r", 7).unwrap();
        assert_eq!(timeline.iter().map(|c| c.id).collect::<Vec<_>>(), vec![3, 1, 2]);
        client.submit_review("o/r", 7, ReviewEvent::Approve, "LGTM").unwrap();
        client.merge("o/r", 7, MergeMethod::Squash, Some("Add login (#7)\n\n* first")).unwrap();
        assert_eq!(client.search_pulls("o/r", "review:approved").unwrap(), vec![7]);
        let log = seen.lock().unwrap();
        assert!(log.iter().any(|l| l.contains("\"commit_title\":\"Add login (#7)\"")), "{log:?}");
        assert!(log.iter().any(|l| l.starts_with("GET /api/v3/search/issues")), "{log:?}");
        assert!(log.iter().any(|l| l.starts_with("POST /api/v3/repos/o/r/pulls/7/reviews") && l.contains("\"APPROVE\"")));
        assert!(log.iter().any(|l| l.starts_with("PUT /api/v3/repos/o/r/pulls/7/merge") && l.contains("squash")));
    }

    #[test]
    fn reports_api_errors() {
        let (base, _) = mock(vec![]);
        let error = Client::new(&account(&base)).pull("o/r", 1).unwrap_err().to_string();
        assert!(error.contains("404"), "{error}");
        assert_eq!(repo_path("https://github.com/o/r"), Some("o/r".into()));
        assert_eq!(repo_path("https://gitlab.com/g/sub/p"), Some("g/sub".into()));
    }
}
