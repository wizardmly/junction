//! GitLab REST API v4 (gitlab.com and self-managed): the calls behind the
//! Merge Requests tool window, the Clone dialog's repository list, Share
//! Project and Create Snippet. Answers are mapped onto the GitHub types so
//! the tool windows show both hosts the same way.

use serde::Deserialize;
use serde_json::json;

use super::github::{BranchRef, Client, Comment, Label, MergeMethod, PrFile, PullRequest, Repo, ReviewEvent, User};
use super::{ApiResult, agent_for, api_error};

/// A project path as the API's `:id` (`group/sub/project` URL-encoded).
pub fn project_id(path: &str) -> String {
    let mut out = String::new();
    for b in path.trim_matches('/').bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// `group/sub/project` from a web URL like `https://gitlab.com/group/sub/project`
/// (GitLab nests groups, so every segment counts).
pub fn repo_path(base: &str) -> Option<String> {
    let rest = base.split_once("://").map_or(base, |(_, r)| r);
    let (_, path) = rest.split_once('/')?;
    let path = path.trim_matches('/').trim_end_matches(".git");
    (path.contains('/') && !path.split('/').any(str::is_empty)).then(|| path.to_owned())
}

#[derive(Deserialize)]
struct GlUser {
    username: String,
    #[serde(default)]
    name: Option<String>,
}

impl From<GlUser> for User {
    fn from(u: GlUser) -> Self {
        User { login: u.username, name: u.name }
    }
}

#[derive(Deserialize)]
struct GlProject {
    path_with_namespace: String,
    name: String,
    #[serde(default)]
    visibility: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    http_url_to_repo: String,
    #[serde(default)]
    ssh_url_to_repo: String,
    #[serde(default)]
    web_url: String,
}

impl From<GlProject> for Repo {
    fn from(p: GlProject) -> Self {
        Repo {
            full_name: p.path_with_namespace,
            name: p.name,
            private: p.visibility.as_deref() != Some("public"),
            description: p.description.filter(|d| !d.is_empty()),
            clone_url: p.http_url_to_repo,
            ssh_url: p.ssh_url_to_repo,
            html_url: p.web_url,
        }
    }
}

#[derive(Deserialize, Default)]
struct DiffRefs {
    #[serde(default)]
    base_sha: Option<String>,
    #[serde(default)]
    start_sha: Option<String>,
    #[serde(default)]
    head_sha: Option<String>,
}

#[derive(Deserialize)]
struct GlMergeRequest {
    iid: u64,
    title: String,
    #[serde(default)]
    description: Option<String>,
    state: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    work_in_progress: bool,
    #[serde(default)]
    merged_at: Option<String>,
    author: GlUser,
    source_branch: String,
    target_branch: String,
    #[serde(default)]
    sha: Option<String>,
    #[serde(default)]
    diff_refs: Option<DiffRefs>,
    created_at: String,
    updated_at: String,
    #[serde(default)]
    web_url: String,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    merge_status: Option<String>,
    #[serde(default)]
    source_project_id: Option<u64>,
    #[serde(default)]
    target_project_id: Option<u64>,
    #[serde(default)]
    assignees: Vec<GlUser>,
    #[serde(default)]
    reviewers: Vec<GlUser>,
}

impl From<GlMergeRequest> for PullRequest {
    fn from(mr: GlMergeRequest) -> Self {
        let refs = mr.diff_refs.unwrap_or_default();
        let merged = mr.state == "merged";
        let fork = mr.source_project_id.is_some() && mr.source_project_id != mr.target_project_id;
        PullRequest {
            number: mr.iid,
            title: mr.title,
            body: mr.description.filter(|d| !d.is_empty()),
            state: if mr.state == "opened" { "open".into() } else { "closed".into() },
            draft: mr.draft || mr.work_in_progress,
            merged_at: if merged { mr.merged_at.or_else(|| Some(mr.updated_at.clone())) } else { None },
            user: mr.author.into(),
            head: BranchRef {
                label: if fork { format!("fork:{}", mr.source_branch) } else { mr.source_branch.clone() },
                name: mr.source_branch,
                sha: mr.sha.or(refs.head_sha).unwrap_or_default(),
            },
            base: BranchRef { label: mr.target_branch.clone(), name: mr.target_branch, sha: refs.base_sha.unwrap_or_default() },
            created_at: mr.created_at,
            updated_at: mr.updated_at,
            html_url: mr.web_url,
            labels: mr.labels.into_iter().map(|name| Label { name, color: String::new() }).collect(),
            mergeable: mr.merge_status.map(|s| s == "can_be_merged"),
            assignees: mr.assignees.into_iter().map(Into::into).collect(),
            requested_reviewers: mr.reviewers.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Deserialize)]
struct GlDiff {
    old_path: String,
    new_path: String,
    #[serde(default)]
    new_file: bool,
    #[serde(default)]
    renamed_file: bool,
    #[serde(default)]
    deleted_file: bool,
    #[serde(default)]
    diff: String,
}

impl From<GlDiff> for PrFile {
    fn from(d: GlDiff) -> Self {
        let count = |sign: char| d.diff.lines().filter(|l| l.starts_with(sign) && !l.starts_with("+++") && !l.starts_with("---")).count() as u64;
        let status = if d.new_file {
            "added"
        } else if d.deleted_file {
            "removed"
        } else if d.renamed_file {
            "renamed"
        } else {
            "modified"
        };
        PrFile {
            additions: count('+'),
            deletions: count('-'),
            status: status.into(),
            previous_filename: d.renamed_file.then(|| d.old_path.clone()),
            filename: if d.deleted_file { d.old_path } else { d.new_path },
        }
    }
}

#[derive(Deserialize, Default)]
struct Position {
    #[serde(default)]
    new_path: Option<String>,
    #[serde(default)]
    new_line: Option<u64>,
}

#[derive(Deserialize)]
struct GlNote {
    id: u64,
    author: GlUser,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    system: bool,
    #[serde(default)]
    position: Option<Position>,
}

impl From<GlNote> for Comment {
    fn from(n: GlNote) -> Self {
        let position = n.position.unwrap_or_default();
        Comment {
            id: n.id,
            user: n.author.into(),
            // IntelliJ lists GitLab's system notes ("added 2 commits") in the timeline too.
            body: n.body.map(|b| if n.system { format!("_{b}_") } else { b }),
            created_at: n.created_at,
            state: None,
            submitted_at: None,
            path: position.new_path,
            line: position.new_line,
        }
    }
}

impl Client {
    fn gl_get<T: for<'de> Deserialize<'de>>(&self, path: &str) -> ApiResult<T> {
        let url = format!("{}{path}", self.base);
        let response = agent_for(&url).get(&url).set("PRIVATE-TOKEN", &self.token).call().map_err(api_error)?;
        Ok(response.into_json()?)
    }

    fn gl_send<T: for<'de> Deserialize<'de>>(&self, method: &str, path: &str, body: serde_json::Value) -> ApiResult<T> {
        let url = format!("{}{path}", self.base);
        let response = agent_for(&url).request(method, &url).set("PRIVATE-TOKEN", &self.token).send_json(body).map_err(api_error)?;
        Ok(response.into_json()?)
    }

    fn mr_path(repo: &str, number: u64) -> String {
        format!("/projects/{}/merge_requests/{number}", project_id(repo))
    }

    pub(super) fn gl_user(&self) -> ApiResult<User> {
        self.gl_get::<GlUser>("/user").map(Into::into)
    }

    pub(super) fn gl_repos(&self) -> ApiResult<Vec<Repo>> {
        let projects: Vec<GlProject> = self.gl_get("/projects?membership=true&order_by=last_activity_at&per_page=100")?;
        Ok(projects.into_iter().map(Into::into).collect())
    }

    /// `state` as GitHub's: open, closed (merged ones included) or all.
    pub(super) fn gl_pulls(&self, repo: &str, state: &str) -> ApiResult<Vec<PullRequest>> {
        let gl_state = if state == "open" { "opened" } else { "all" };
        let mrs: Vec<GlMergeRequest> = self.gl_get(&format!(
            "/projects/{}/merge_requests?state={gl_state}&order_by=updated_at&sort=desc&per_page=100",
            project_id(repo)
        ))?;
        Ok(mrs.into_iter().map(PullRequest::from).filter(|pr| state != "closed" || pr.state == "closed").collect())
    }

    /// The Review filter's GitHub qualifiers as merge request list
    /// parameters: approvals stand for reviews (GitLab has no review states
    /// to search; "changes requested" has no equivalent).
    pub(super) fn gl_search_pulls(&self, repo: &str, qualifier: &str) -> ApiResult<Vec<u64>> {
        let filter = match qualifier {
            "review:none" => "approved_by_ids=None".to_owned(),
            "review:approved" => "approved_by_ids=Any".to_owned(),
            "reviewed-by:@me" => format!("approved_by_usernames[]={}", project_id(&self.gl_user()?.login)),
            _ => return Ok(Vec::new()),
        };
        let mrs: Vec<GlMergeRequest> =
            self.gl_get(&format!("/projects/{}/merge_requests?state=all&{filter}&per_page=100", project_id(repo)))?;
        Ok(mrs.into_iter().map(|mr| mr.iid).collect())
    }

    pub(super) fn gl_pull(&self, repo: &str, number: u64) -> ApiResult<PullRequest> {
        self.gl_get::<GlMergeRequest>(&Self::mr_path(repo, number)).map(Into::into)
    }

    pub(super) fn gl_pull_files(&self, repo: &str, number: u64) -> ApiResult<Vec<PrFile>> {
        let diffs: Vec<GlDiff> = self.gl_get(&format!("{}/diffs?per_page=100", Self::mr_path(repo, number)))?;
        Ok(diffs.into_iter().map(Into::into).collect())
    }

    pub(super) fn gl_timeline(&self, repo: &str, number: u64) -> ApiResult<Vec<Comment>> {
        let notes: Vec<GlNote> = self.gl_get(&format!("{}/notes?sort=asc&order_by=created_at&per_page=100", Self::mr_path(repo, number)))?;
        Ok(notes.into_iter().map(Into::into).collect())
    }

    pub(super) fn gl_add_comment(&self, repo: &str, number: u64, body: &str) -> ApiResult<Comment> {
        self.gl_send::<GlNote>("POST", &format!("{}/notes", Self::mr_path(repo, number)), json!({ "body": body })).map(Into::into)
    }

    /// A diff note on a new-side line: a discussion positioned by the MR's diff refs.
    pub(super) fn gl_add_line_comment(&self, repo: &str, number: u64, commit: &str, path: &str, line: u64, body: &str) -> ApiResult<Comment> {
        let mr: GlMergeRequest = self.gl_get(&Self::mr_path(repo, number))?;
        let refs = mr.diff_refs.unwrap_or_default();
        let head = refs.head_sha.unwrap_or_else(|| commit.to_owned());
        let position = json!({
            "position_type": "text",
            "base_sha": refs.base_sha.clone().unwrap_or_default(),
            "start_sha": refs.start_sha.or(refs.base_sha).unwrap_or_default(),
            "head_sha": head,
            "new_path": path,
            "old_path": path,
            "new_line": line,
        });
        let discussion: serde_json::Value =
            self.gl_send("POST", &format!("{}/discussions", Self::mr_path(repo, number)), json!({ "body": body, "position": position }))?;
        let note = discussion.get("notes").and_then(|n| n.get(0)).cloned().unwrap_or_default();
        Ok(serde_json::from_value::<GlNote>(note).map(Into::into)?)
    }

    /// Approve approves; GitLab has no "request changes" review, so that
    /// (and a plain review comment) becomes a note.
    pub(super) fn gl_submit_review(&self, repo: &str, number: u64, event: ReviewEvent, body: &str) -> ApiResult<Comment> {
        let note = |text: String| self.gl_add_comment(repo, number, &text);
        match event {
            ReviewEvent::Approve => {
                let _: serde_json::Value = self.gl_send("POST", &format!("{}/approve", Self::mr_path(repo, number)), json!({}))?;
                if body.trim().is_empty() {
                    Ok(Comment { id: 0, user: self.gl_user()?, body: Some("approved".into()), created_at: None, state: Some("APPROVED".into()), submitted_at: None, path: None, line: None })
                } else {
                    note(body.to_owned())
                }
            }
            ReviewEvent::RequestChanges => note(format!("**Changes requested**\n\n{body}")),
            ReviewEvent::Comment => note(body.to_owned()),
        }
    }

    /// Merge, Squash and Merge, or Rebase (GitLab rebases the source branch;
    /// the merge then fast-forwards where the project allows it).
    pub(super) fn gl_merge(&self, repo: &str, number: u64, method: MergeMethod, message: Option<&str>) -> ApiResult<serde_json::Value> {
        let path = Self::mr_path(repo, number);
        match method {
            MergeMethod::Merge => {
                let body = message.map_or(json!({}), |m| json!({ "merge_commit_message": m }));
                self.gl_send("PUT", &format!("{path}/merge"), body)
            }
            MergeMethod::Squash => {
                let mut body = json!({ "squash": true });
                if let Some(message) = message {
                    body["squash_commit_message"] = json!(message);
                }
                self.gl_send("PUT", &format!("{path}/merge"), body)
            }
            MergeMethod::Rebase => {
                let _: serde_json::Value = self.gl_send("PUT", &format!("{path}/rebase"), json!({}))?;
                self.gl_send("PUT", &format!("{path}/merge"), json!({}))
            }
        }
    }

    /// `head` may be GitHub's `owner:branch`; GitLab takes the branch.
    pub(super) fn gl_create_pull(&self, repo: &str, title: &str, body: &str, head: &str, base: &str, draft: bool) -> ApiResult<PullRequest> {
        let source = head.rsplit_once(':').map_or(head, |(_, b)| b);
        let title = if draft && !title.starts_with("Draft:") { format!("Draft: {title}") } else { title.to_owned() };
        self.gl_send::<GlMergeRequest>(
            "POST",
            &format!("/projects/{}/merge_requests", project_id(repo)),
            json!({ "source_branch": source, "target_branch": base, "title": title, "description": body }),
        )
        .map(Into::into)
    }

    pub(super) fn gl_create_repo(&self, name: &str, private: bool, description: &str) -> ApiResult<Repo> {
        let visibility = if private { "private" } else { "public" };
        self.gl_send::<GlProject>("POST", "/projects", json!({ "name": name, "visibility": visibility, "description": description })).map(Into::into)
    }

    /// Create Snippet (GitLab's gists): returns its web URL.
    pub(super) fn gl_create_gist(&self, description: &str, public: bool, files: &[(String, String)]) -> ApiResult<String> {
        let title = if description.is_empty() { files.first().map(|f| f.0.clone()).unwrap_or_default() } else { description.to_owned() };
        let files: Vec<serde_json::Value> = files.iter().map(|(name, content)| json!({ "file_path": name, "content": content })).collect();
        let visibility = if public { "public" } else { "private" };
        let snippet: serde_json::Value =
            self.gl_send("POST", "/snippets", json!({ "title": title, "description": description, "visibility": visibility, "files": files }))?;
        Ok(snippet.get("web_url").and_then(|u| u.as_str()).unwrap_or_default().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hosting::account::{Account, Service};
    use crate::hosting::github::tests::mock;

    const MR: &str = r#"{"iid":5,"title":"Draft: Add login","description":"Closes #3","state":"opened","draft":true,
        "author":{"username":"ana","name":"Ana"},"source_branch":"feature/login","target_branch":"main","sha":"abc",
        "diff_refs":{"base_sha":"def","start_sha":"def","head_sha":"abc"},"created_at":"2026-10-01T10:00:00Z",
        "updated_at":"2026-10-02T10:00:00Z","web_url":"https://gitlab.com/g/sub/p/-/merge_requests/5","labels":["ui"],
        "merge_status":"can_be_merged","source_project_id":1,"target_project_id":1}"#;

    #[test]
    fn maps_merge_requests_onto_pull_requests() {
        let mrs: &'static str = Box::leak(format!("[{MR}]").into_boxed_str());
        let (base, seen) = mock(vec![
            ("GET /api/v4/user", r#"{"username":"me","name":"Me"}"#),
            ("GET /api/v4/projects/g%2Fsub%2Fp/merge_requests", mrs),
            ("GET /api/v4/projects/g%2Fsub%2Fp/merge_requests/5", MR),
            ("GET /api/v4/projects/g%2Fsub%2Fp/merge_requests/5/diffs", r#"[{"old_path":"a.rs","new_path":"b.rs","renamed_file":true,"diff":"@@ -1 +1,2 @@\n-x\n+y\n+z\n"}]"#),
            ("GET /api/v4/projects/g%2Fsub%2Fp/merge_requests/5/notes", r#"[{"id":1,"author":{"username":"bo"},"body":"Nice","created_at":"2026-10-02T09:00:00Z"},
                {"id":2,"author":{"username":"bo"},"body":"typo","created_at":"2026-10-02T09:30:00Z","position":{"new_path":"b.rs","new_line":2}}]"#),
            ("POST /api/v4/projects/g%2Fsub%2Fp/merge_requests/5/approve", r#"{"id":5}"#),
            ("PUT /api/v4/projects/g%2Fsub%2Fp/merge_requests/5/merge", r#"{"iid":5}"#),
            ("POST /api/v4/projects/g%2Fsub%2Fp/merge_requests/5/discussions", r#"{"id":"d","notes":[{"id":9,"author":{"username":"me"},"body":"hm","position":{"new_path":"b.rs","new_line":1}}]}"#),
            ("POST /api/v4/projects/g%2Fsub%2Fp/merge_requests", MR),
        ]);
        let account = Account { service: Service::GitLab, server: base.clone(), login: "me".into(), token: "tok".into() };
        let client = Client::new(&account);
        assert_eq!(client.user().unwrap().login, "me");
        let prs = client.pulls("g/sub/p", "open").unwrap();
        assert_eq!((prs[0].number, prs[0].status(), prs[0].head.name.as_str(), prs[0].base.sha.as_str()), (5, "Draft", "feature/login", "def"));
        let files = client.pull_files("g/sub/p", 5).unwrap();
        assert_eq!((files[0].status.as_str(), files[0].additions, files[0].deletions), ("renamed", 2, 1));
        assert_eq!(files[0].previous_filename.as_deref(), Some("a.rs"));
        let timeline = client.timeline("g/sub/p", 5).unwrap();
        assert_eq!(timeline[1].line, Some(2));
        client.submit_review("g/sub/p", 5, ReviewEvent::Approve, "").unwrap();
        client.merge("g/sub/p", 5, MergeMethod::Squash, None).unwrap();
        assert_eq!(client.add_line_comment("g/sub/p", 5, "abc", "b.rs", 1, "hm").unwrap().id, 9);
        client.create_pull("g/sub/p", "T", "", "me:topic", "main", true).unwrap();
        assert_eq!(client.search_pulls("g/sub/p", "review:approved").unwrap(), vec![5]);
        assert_eq!(client.search_pulls("g/sub/p", "reviewed-by:@me").unwrap(), vec![5]);
        assert!(client.search_pulls("g/sub/p", "review:changes_requested").unwrap().is_empty());
        let log = seen.lock().unwrap();
        assert!(log.iter().any(|l| l.contains("merge_requests?state=all&approved_by_ids=Any&")), "{log:?}");
        assert!(log.iter().any(|l| l.contains("&approved_by_usernames[]=me&")), "{log:?}");
        assert!(log.iter().any(|l| l.starts_with("PUT /api/v4/projects/g%2Fsub%2Fp/merge_requests/5/merge") && l.contains("squash")));
        assert!(log.iter().any(|l| l.contains("/discussions") && l.contains("\"head_sha\":\"abc\"") && l.contains("\"new_line\":1")));
        assert!(log.iter().any(|l| l.starts_with("POST /api/v4/projects/g%2Fsub%2Fp/merge_requests ") && l.contains("Draft: T") && l.contains("\"source_branch\":\"topic\"")));
    }

    #[test]
    fn project_paths() {
        assert_eq!(project_id("g/sub/p"), "g%2Fsub%2Fp");
        assert_eq!(repo_path("https://gitlab.com/g/sub/p"), Some("g/sub/p".into()));
        assert_eq!(repo_path("https://gitlab.com/p"), None);
    }
}
