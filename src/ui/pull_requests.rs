//! The Pull Requests tool window (GitHub; Merge Requests for GitLab): the repository's PRs with state
//! and search filters; a PR's details with its files, Checkout, review and
//! merge actions; the conversation timeline opens in the editor area.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Icon, Selectable as _, Sizable as _, WindowExt as _, h_flex,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    menu::{DropdownMenu as _, PopupMenuItem},
    scroll::ScrollableElement as _,
    text::TextView,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div, prelude::FluentBuilder as _,
    px,
};

use crate::hosting::account::{self, Account};
use crate::hosting::github::{self, Client, Comment, MergeMethod, PrFile, PullRequest, ReviewEvent};
use crate::model::{RepoEvent, RepoModel};
use crate::theme::ActivePalette as _;
use crate::ui::common::{row_height, tool_button};
use crate::ui::diff_view::DiffSource;

/// Where the PRs come from: an account and `owner/repo` on its server.
#[derive(Clone)]
pub struct PrTarget {
    pub account: Account,
    pub repo: String,
    pub remote: String,
}

impl PrTarget {
    pub fn gitlab(&self) -> bool {
        self.account.service == account::Service::GitLab
    }

    /// "Pull Request" on GitHub, "Merge Request" on GitLab.
    pub fn noun(&self) -> &'static str {
        if self.gitlab() { "Merge Request" } else { "Pull Request" }
    }

    /// `#7` on GitHub, `!7` on GitLab.
    pub fn number(&self, n: u64) -> String {
        if self.gitlab() { format!("!{n}") } else { format!("#{n}") }
    }

    pub fn service_name(&self) -> &'static str {
        if self.gitlab() { "GitLab" } else { "GitHub" }
    }
}

pub enum PrEvent {
    OpenDiff(DiffSource),
    /// A PR file's diff, with its review comments; line clicks comment.
    OpenReviewDiff(DiffSource, crate::ui::diff_view::Review),
    OpenTimeline(PrTarget, PullRequest),
}

impl EventEmitter<PrEvent> for PullRequestsView {}

struct Details {
    pr: PullRequest,
    files: Vec<PrFile>,
    /// The fork point the files are compared from, once fetched.
    base: Option<String>,
    /// Review comments on lines, for the diff's markers.
    comments: Vec<Comment>,
    loading: bool,
}

pub struct PullRequestsView {
    model: Entity<RepoModel>,
    target: Option<PrTarget>,
    /// Why there is no target: no GitHub remote, or no account for it.
    problem: Option<String>,
    state: &'static str,
    filters: Filters,
    /// The PRs the Review filter's search found (GitHub searches reviews server-side).
    review_matches: Option<std::collections::HashSet<u64>>,
    search: Entity<InputState>,
    prs: Vec<PullRequest>,
    loading: bool,
    error: Option<String>,
    selected: Option<usize>,
    details: Option<Details>,
    _load: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

/// The list's filters besides State, as IntelliJ's search popup has them.
#[derive(Clone, Default)]
struct Filters {
    author: Option<String>,
    label: Option<String>,
    assignee: Option<String>,
    review: Option<ReviewFilter>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReviewFilter {
    RequestedFromMe,
    AwaitingReview,
    Approved,
    ChangesRequested,
    ReviewedByMe,
}

impl ReviewFilter {
    const ALL: [Self; 5] = [Self::AwaitingReview, Self::Approved, Self::ChangesRequested, Self::ReviewedByMe, Self::RequestedFromMe];

    fn label(self) -> &'static str {
        match self {
            Self::RequestedFromMe => "Review requested from you",
            Self::AwaitingReview => "Awaiting review",
            Self::Approved => "Approved",
            Self::ChangesRequested => "Changes requested",
            Self::ReviewedByMe => "Reviewed by you",
        }
    }

    /// GitHub's search qualifier; `None` is decided from the list itself.
    fn qualifier(self) -> Option<&'static str> {
        match self {
            Self::RequestedFromMe => None,
            Self::AwaitingReview => Some("review:none"),
            Self::Approved => Some("review:approved"),
            Self::ChangesRequested => Some("review:changes_requested"),
            Self::ReviewedByMe => Some("reviewed-by:@me"),
        }
    }
}

/// The account and repository for the active repository's GitHub remote.
fn resolve(model: &RepoModel) -> Result<PrTarget, String> {
    use crate::git::hosting::Host;
    let web = model.web_repo().ok_or("This repository has no GitHub or GitLab remote")?;
    let host = web.base.trim_start_matches("https://").split('/').next().unwrap_or_default().to_owned();
    let account = match account::for_host(&host) {
        Some(account) => account,
        None if web.host == Host::GitLab => return Err(format!("Log in to {host} to see merge requests")),
        None if web.host == Host::GitHub => return Err(format!("Log in to {host} to see pull requests")),
        // A self-managed server is known by its account.
        None => return Err(format!("Pull and merge requests are shown for GitHub and GitLab remotes ({} is not one)", web.base)),
    };
    let repo = match account.service {
        account::Service::GitLab => crate::hosting::gitlab::repo_path(&web.base),
        account::Service::GitHub => github::repo_path(&web.base),
    }
    .ok_or("Cannot tell the repository from the remote URL")?;
    let remote = model
        .repository()
        .and_then(|r| r.run(["remote"]).ok())
        .and_then(|list| {
            let names: Vec<String> = list.lines().map(str::to_owned).collect();
            names.iter().find(|n| *n == "origin").or(names.first()).cloned()
        })
        .unwrap_or_else(|| "origin".into());
    Ok(PrTarget { account, repo, remote })
}

/// The review comments on one file, by new-side line.
fn review_for(comments: &[Comment], path: &str) -> crate::ui::diff_view::Review {
    let mut review = crate::ui::diff_view::Review::default();
    for c in comments.iter().filter(|c| c.path.as_deref() == Some(path)) {
        if let Some(line) = c.line {
            review.comments.entry(line as usize).or_default().push(format!("{}: {}", c.user.login, c.body.as_deref().unwrap_or_default()));
        }
    }
    review
}

fn source_of(details: &Details, path: &str) -> DiffSource {
    let file = details.files.iter().find(|f| f.filename == path);
    DiffSource::Between {
        old: details.base.clone().unwrap_or_else(|| details.pr.base.sha.clone()),
        new: Some(details.pr.head.sha.clone()),
        path: path.to_owned(),
        old_path: file.and_then(|f| f.previous_filename.clone()),
    }
}

impl PullRequestsView {
    pub fn new(model: Entity<RepoModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let subscriptions = vec![
            cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            // A new repository (or remote) may mean another target.
            cx.subscribe(&model, |this, model, event, cx| {
                if matches!(event, RepoEvent::Reloaded) {
                    let target = resolve(model.read(cx));
                    let changed = match (&target, &this.target) {
                        (Ok(new), Some(old)) => new.repo != old.repo || new.account != old.account,
                        (Ok(_), None) => true,
                        (Err(problem), _) => this.problem.as_ref() != Some(problem),
                    };
                    if changed {
                        this.refresh(cx);
                    }
                }
            }),
        ];
        let mut this = Self {
            model,
            target: None,
            problem: None,
            state: "open",
            filters: Filters::default(),
            review_matches: None,
            search,
            prs: Vec::new(),
            loading: false,
            error: None,
            selected: None,
            details: None,
            _load: None,
            _subscriptions: subscriptions,
        };
        this.refresh(cx);
        this
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        match resolve(self.model.read(cx)) {
            Ok(target) => {
                self.problem = None;
                self.target = Some(target.clone());
                self.loading = true;
                self.error = None;
                let state = self.state;
                let open_number = self.details.as_ref().map(|d| d.pr.number);
                let qualifier = if target.gitlab() { None } else { self.filters.review.and_then(ReviewFilter::qualifier) };
                self._load = Some(cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_spawn(async move {
                            let client = Client::new(&target.account);
                            let prs = client.pulls(&target.repo, state)?;
                            // The open details may have changed state (merged, closed) and left this filter.
                            let open = open_number.and_then(|n| client.pull(&target.repo, n).ok());
                            let matches = match qualifier {
                                Some(q) => Some(client.search_pulls(&target.repo, q)?.into_iter().collect()),
                                None => None,
                            };
                            anyhow::Ok((prs, open, matches))
                        })
                        .await;
                    this.update(cx, |this, cx| {
                        this.loading = false;
                        match result {
                            Ok((prs, open, matches)) => {
                                this.prs = prs;
                                this.review_matches = matches;
                                if let Some(pr) = open {
                                    this.selected = this.prs.iter().position(|p| p.number == pr.number);
                                    if let Some(details) = this.details.as_mut().filter(|d| d.pr.number == pr.number) {
                                        details.pr = pr;
                                    }
                                }
                            }
                            Err(error) => this.error = Some(error.to_string()),
                        }
                        cx.notify();
                    })
                    .ok();
                }));
            }
            Err(problem) => {
                self.target = None;
                self.prs.clear();
                self.problem = Some(problem);
            }
        }
        cx.notify();
    }

    /// Create Pull Request (the toolbar's + and Git › GitHub's item); without
    /// a GitHub / GitLab target, says why.
    pub fn create_pull_request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = match resolve(self.model.read(cx)) {
            Ok(target) => target,
            Err(problem) => {
                self.model.update(cx, |_, cx| cx.emit(RepoEvent::Notify { title: "Create Pull Request".into(), message: problem, error: true }));
                return;
            }
        };
        let entity = cx.entity();
        crate::ui::github_dialogs::create_pull_request(
            self.model.clone(),
            target,
            Rc::new(move |cx: &mut App| entity.update(cx, |this, cx| this.refresh(cx))),
            window,
            cx,
        );
    }

    fn visible(&self, cx: &App) -> Vec<(usize, &PullRequest)> {
        let query = self.search.read(cx).value().trim().to_lowercase();
        let me = self.target.as_ref().map(|t| t.account.login.clone()).unwrap_or_default();
        let filters = &self.filters;
        self.prs
            .iter()
            .enumerate()
            .filter(|(_, pr)| {
                filters.author.as_ref().is_none_or(|a| pr.user.login == *a)
                    && filters.label.as_ref().is_none_or(|l| pr.labels.iter().any(|x| x.name == *l))
                    && filters.assignee.as_ref().is_none_or(|a| pr.assignees.iter().any(|x| x.login == *a))
                    && match filters.review {
                        None => true,
                        Some(ReviewFilter::RequestedFromMe) => pr.requested_reviewers.iter().any(|r| r.login == me),
                        Some(_) => self.review_matches.as_ref().is_none_or(|m| m.contains(&pr.number)),
                    }
            })
            .filter(|(_, pr)| {
                query.is_empty()
                    || pr.title.to_lowercase().contains(&query)
                    || pr.user.login.to_lowercase().contains(&query)
                    || format!("#{}", pr.number).contains(&query)
                    || format!("!{}", pr.number).contains(&query)
                    || pr.labels.iter().any(|l| l.name.to_lowercase().contains(&query))
            })
            .collect()
    }

    fn open(&mut self, ix: usize, cx: &mut Context<Self>) {
        let (Some(pr), Some(target)) = (self.prs.get(ix).cloned(), self.target.clone()) else { return };
        let repository = self.model.read(cx).repository().cloned();
        self.selected = Some(ix);
        self.details = Some(Details { pr: pr.clone(), files: Vec::new(), base: None, comments: Vec::new(), loading: true });
        self._load = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let client = Client::new(&target.account);
                    let files = client.pull_files(&target.repo, pr.number)?;
                    let comments: Vec<Comment> = client.timeline(&target.repo, pr.number).unwrap_or_default().into_iter().filter(|c| c.path.is_some()).collect();
                    // The PR's commits, so its files can be diffed locally.
                    let base = repository.and_then(|repo| {
                        repo.run(["fetch", "--quiet", &target.remote, &format!("+refs/pull/{}/head:refs/remotes/{}/pr/{}", pr.number, target.remote, pr.number)]).ok();
                        repo.run(["merge-base", &pr.base.sha, &pr.head.sha]).ok().map(|s| s.trim().to_owned())
                    });
                    anyhow::Ok((files, base, comments))
                })
                .await;
            this.update(cx, |this, cx| {
                if let Some(details) = this.details.as_mut() {
                    details.loading = false;
                    match result {
                        Ok((files, base, comments)) => {
                            details.files = files;
                            details.base = base;
                            details.comments = comments;
                        }
                        Err(error) => this.error = Some(error.to_string()),
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn open_file(&mut self, file: &PrFile, cx: &mut Context<Self>) {
        let Some(details) = &self.details else { return };
        let old = details.base.clone().unwrap_or_else(|| details.pr.base.sha.clone());
        let source = DiffSource::Between {
            old,
            new: Some(details.pr.head.sha.clone()),
            path: file.filename.clone(),
            old_path: file.previous_filename.clone(),
        };
        cx.emit(PrEvent::OpenReviewDiff(source, review_for(&details.comments, &file.filename)));
    }

    /// A click on a line of the open PR's diff: Add Review Comment.
    pub fn comment_line(&mut self, path: String, line: usize, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(target), Some(details)) = (self.target.clone(), self.details.as_ref()) else { return };
        let (number, commit) = (details.pr.number, details.pr.head.sha.clone());
        let (entity, model) = (cx.entity(), self.model.clone());
        review_dialog("Add Review Comment", "Comment", window, cx, move |body, cx| {
            if body.trim().is_empty() {
                return;
            }
            let (target, commit, path, entity, model) = (target.clone(), commit.clone(), path.clone(), entity.clone(), model.clone());
            cx.spawn(async move |cx| {
                let posted_path = path.clone();
                let result = cx
                    .background_spawn(async move { Client::new(&target.account).add_line_comment(&target.repo, number, &commit, &posted_path, line as u64, &body) })
                    .await;
                cx.update(|cx| match result {
                    Ok(comment) => entity.update(cx, |this, cx| {
                        let Some(details) = this.details.as_mut().filter(|d| d.pr.number == number) else { return };
                        details.comments.push(comment);
                        let review = review_for(&details.comments, &path);
                        cx.emit(PrEvent::OpenReviewDiff(source_of(details, &path), review));
                    }),
                    Err(error) => model.update(cx, |_, cx| {
                        cx.emit(RepoEvent::Notify { title: "Add Review Comment failed".into(), message: error.to_string(), error: true })
                    }),
                });
            })
            .detach();
        });
    }

    fn api_op(&self, title: &'static str, op: impl FnOnce(&Client, &str, u64) -> anyhow::Result<String> + Send + 'static, cx: &mut Context<Self>) {
        let (Some(target), Some(details)) = (self.target.clone(), self.details.as_ref()) else { return };
        let number = details.pr.number;
        let model = self.model.clone();
        let label = target.number(number);
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { op(&Client::new(&target.account), &target.repo, number) }).await;
            let (message, error) = match result {
                Ok(message) => (message.replace(&format!("#{number}"), &label), false),
                Err(error) => (error.to_string(), true),
            };
            model.update(cx, |_, cx| cx.emit(RepoEvent::Notify { title: title.into(), message, error }));
            this.update(cx, |this, cx| this.refresh(cx)).ok();
        })
        .detach();
    }

    fn checkout(&self, cx: &mut Context<Self>) {
        let (Some(target), Some(details)) = (self.target.clone(), self.details.as_ref()) else { return };
        let pr = details.pr.clone();
        self.model.update(cx, |m, cx| {
            let label = target.number(pr.number);
            m.run_operation(if target.gitlab() { "Checkout Merge Request" } else { "Checkout Pull Request" }, move |repo| {
                let branch = pr.head.name.clone();
                let exists = repo.run(["rev-parse", "--verify", "-q", &format!("refs/heads/{branch}")]).is_ok();
                if exists {
                    repo.run(["checkout", &branch])?;
                    repo.run(["merge", "--ff-only", &pr.head.sha]).ok();
                } else {
                    // GitLab keeps each merge request's head under merge-requests/.
                    let head = if target.gitlab() { format!("merge-requests/{}/head", pr.number) } else { format!("pull/{}/head", pr.number) };
                    repo.run(["fetch", &target.remote, &format!("{head}:{branch}")])?;
                    repo.run(["checkout", &branch])?;
                }
                Ok(format!("Checked out {label} as {branch}"))
            }, cx)
        });
    }

    fn request_changes(&self, window: &mut Window, cx: &mut Context<Self>) {
        let entity = cx.entity();
        review_dialog("Request Changes", "Request Changes", window, cx, move |body, cx| {
            entity.update(cx, |this, cx| {
                this.api_op("Request Changes", move |c, repo, n| {
                    c.submit_review(repo, n, ReviewEvent::RequestChanges, &body)?;
                    Ok(format!("Requested changes on #{n}"))
                }, cx)
            })
        });
    }
}

impl PullRequestsView {
    /// Merge… / Squash and Merge… edit the merge commit's message first, as
    /// IntelliJ's dialogs do; Rebase and Merge asks to confirm.
    fn merge(&mut self, method: MergeMethod, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(target), Some(details)) = (self.target.clone(), self.details.as_ref()) else { return };
        let pr = details.pr.clone();
        let number = target.number(pr.number);
        let entity = cx.entity();
        let run = move |message: Option<String>, cx: &mut App| {
            entity.update(cx, |this, cx| {
                let title = if this.title() == "Merge Requests" { "Merge Merge Request" } else { "Merge Pull Request" };
                this.api_op(title, move |c, repo, n| {
                    c.merge(repo, n, method, message.as_deref())?;
                    Ok(format!("Merged #{n}"))
                }, cx)
            })
        };
        let from = if pr.head.label.is_empty() { pr.head.name.clone() } else { pr.head.label.replacen(':', "/", 1) };
        match method {
            MergeMethod::Rebase => crate::ui::dialogs::confirm(
                "Rebase and Merge",
                format!("Rebase the commits of {number} onto {} and merge them?", pr.base.name),
                "Rebase and Merge",
                move |cx| run(None, cx),
                window,
                cx,
            ),
            MergeMethod::Merge => message_dialog(
                "Merge Pull Request",
                "Merge",
                format!("Merge pull request {number} from {from}\n\n{}", pr.title),
                window,
                cx,
                move |message, cx| run(Some(message), cx),
            ),
            MergeMethod::Squash => {
                // The squashed commit: the PR's title, then its commits' subjects.
                let subjects = self
                    .model
                    .read(cx)
                    .repository()
                    .and_then(|repo| repo.run(["log", "--reverse", "--format=* %s", &format!("{}..{}", pr.base.sha, pr.head.sha)]).ok())
                    .unwrap_or_default();
                message_dialog(
                    "Squash and Merge Pull Request",
                    "Squash and Merge",
                    format!("{} ({number})\n\n{}", pr.title, subjects.trim()),
                    window,
                    cx,
                    move |message, cx| run(Some(message), cx),
                )
            }
        }
    }
}

/// A label as GitHub shows it: its colour behind the name.
fn label_chip(label: &github::Label, palette: &crate::theme::Palette) -> gpui_kit::Div {
    let chip = div().px_1().rounded(px(3.)).border_1().text_xs();
    match u32::from_str_radix(label.color.trim_start_matches('#'), 16) {
        Ok(rgb) if label.color.len() >= 6 => {
            let color: gpui_kit::Hsla = gpui_kit::rgb(rgb).into();
            chip.border_color(color).bg(color.opacity(0.18)).text_color(palette.text).child(label.name.clone())
        }
        _ => chip.border_color(palette.border).text_color(palette.text_secondary).child(label.name.clone()),
    }
}

/// A dialog with a text area, for review bodies and comments; an empty
/// one says so instead of closing.
pub fn review_dialog(title: &'static str, ok: &'static str, window: &mut Window, cx: &mut App, on_ok: impl Fn(String, &mut App) + 'static) {
    text_dialog(title, ok, String::new(), "Leave a comment", window, cx, on_ok)
}

/// The merge commit message, editable before merging.
fn message_dialog(title: &'static str, ok: &'static str, message: String, window: &mut Window, cx: &mut App, on_ok: impl Fn(String, &mut App) + 'static) {
    text_dialog(title, ok, message, "Commit message", window, cx, on_ok)
}

fn text_dialog(
    title: &'static str,
    ok: &'static str,
    initial: String,
    placeholder: &'static str,
    window: &mut Window,
    cx: &mut App,
    on_ok: impl Fn(String, &mut App) + 'static,
) {
    let text = cx.new(|cx| TextareaState::new(window, cx).rows(6).default_value(initial).placeholder(placeholder));
    let on_ok = Rc::new(on_ok);
    let focus = text.clone();
    let empty = Rc::new(std::cell::Cell::new(false));
    window.open_dialog(cx, move |dialog, _, cx| {
        let (text_ok, on_ok, empty_ok) = (text.clone(), on_ok.clone(), empty.clone());
        let error = cx.palette().status_conflict;
        dialog
            .title(title)
            .w(px(520.))
            .child(
                v_flex()
                    .gap_1()
                    .child(Textarea::new(&text))
                    .when(empty.get(), |el| el.child(div().text_xs().text_color(error).child(if placeholder == "Commit message" { "Enter a commit message" } else { "Enter a comment" }))),
            )
            .on_ok(move |_, window, cx| {
                let body = text_ok.read(cx).value().trim().to_owned();
                if body.is_empty() {
                    empty_ok.set(true);
                    window.refresh();
                    return false;
                }
                on_ok(body, cx);
                true
            })
            .footer(crate::ui::dialogs::footer(ok))
    });
    window.defer(cx, move |window, cx| focus.update(cx, |s, cx| s.focus(window, cx)));
}

fn state_color(status: &str, palette: &crate::theme::Palette) -> gpui_kit::Hsla {
    match status {
        "Open" => palette.status_added,
        "Merged" => palette.ref_remote,
        "Draft" => palette.text_secondary,
        _ => palette.status_deleted,
    }
}

impl PullRequestsView {
    /// The tool window's name: Merge Requests for a GitLab repository.
    pub fn title(&self) -> &'static str {
        if self.target.as_ref().is_some_and(PrTarget::gitlab) { "Merge Requests" } else { "Pull Requests" }
    }

    fn number(&self, n: u64) -> String {
        self.target.as_ref().map_or_else(|| format!("#{n}"), |t| t.number(n))
    }
}

impl Render for PullRequestsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        if let Some(problem) = self.problem.clone() {
            let entity = cx.entity();
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .p_4()
                .child(Icon::new(IconName::GitPullRequest).large().text_color(palette.text_secondary))
                .child(div().text_sm().text_color(palette.text_secondary).text_center().child(problem.clone()))
                .when(problem.starts_with("Log in"), |el| {
                    el.child(Button::new("pr-login").small().primary().label("Log In…").on_click(move |_, window, cx| {
                        let entity = entity.clone();
                        crate::ui::accounts_dialog::accounts(
                            Some(Rc::new(move |cx: &mut App| entity.update(cx, |this, cx| this.refresh(cx)))),
                            window,
                            cx,
                        )
                    }))
                })
                .into_any_element();
        }
        if let Some(details) = &self.details {
            return self.render_details(details, &palette, cx).into_any_element();
        }

        let mut rows = v_flex();
        let visible = self.visible(cx);
        if visible.is_empty() && !self.prs.is_empty() {
            rows = rows.child(div().p_3().text_sm().text_color(palette.text_secondary).child("Nothing matches the search and filters"));
        }
        for (ix, pr) in visible {
            let status = pr.status();
            rows = rows.child(
                v_flex()
                    .id(SharedString::from(format!("pr-{}", pr.number)))
                    .px_2()
                    .py_1()
                    .gap_0p5()
                    .border_b_1()
                    .border_color(palette.border)
                    .cursor_pointer()
                    .hover(|s| s.bg(palette.hover))
                    .on_click(cx.listener(move |this, _, _, cx| this.open(ix, cx)))
                    .child(
                        h_flex()
                            .gap_1p5()
                            .text_sm()
                            .child(Icon::new(IconName::GitPullRequest).small().text_color(state_color(status, &palette)))
                            .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().font_weight(gpui_kit::FontWeight::MEDIUM).child(pr.title.clone()))
                            .children(pr.labels.iter().take(2).map(|l| label_chip(l, &palette))),
                    )
                    .child(
                        div()
                            .pl(px(20.))
                            .text_xs()
                            .text_color(palette.text_secondary)
                            .child(format!("{} · {} · {} · {}", self.number(pr.number), status, pr.user.login, pr.updated_at.get(..10).unwrap_or_default())),
                    ),
            );
        }
        if !self.loading && self.prs.is_empty() && self.error.is_none() {
            rows = rows.child(div().p_3().text_sm().text_color(palette.text_secondary).child(if self.title() == "Merge Requests" { "No merge requests" } else { "No pull requests" }));
        }
        let state = self.state;
        let entity = cx.entity();
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(px(crate::ui::common::toolbar_height()))
                    .px_1()
                    .gap_1()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(div().flex_1().child(Input::new(&self.search).xsmall().cleanable(true)))
                    .child(
                        Button::new("pr-state")
                            .ghost()
                            .xsmall()
                            .label(match state {
                                "open" => "Open",
                                "closed" => "Closed",
                                _ => "All",
                            })
                            .dropdown_menu(move |mut menu, _, _| {
                                for (value, label) in [("open", "Open"), ("closed", "Closed"), ("all", "All")] {
                                    let entity = entity.clone();
                                    menu = menu.item(PopupMenuItem::new(label).checked(state == value).on_click(move |_, _, cx| {
                                        entity.update(cx, |this, cx| {
                                            this.state = value;
                                            this.refresh(cx)
                                        })
                                    }));
                                }
                                menu
                            }),
                    )
                    .child(tool_button("pr-create", IconName::Plus, if self.title() == "Merge Requests" { "Create Merge Request" } else { "Create Pull Request" }).on_click(cx.listener(|this, _, window, cx| this.create_pull_request(window, cx))))
                    .child(tool_button("pr-refresh", IconName::RefreshCw, "Refresh").on_click(cx.listener(|this, _, _, cx| this.refresh(cx)))),
            )
            .child(self.filter_bar(&palette, cx))
            .when(self.loading, |el| el.child(div().px_2().py_1().text_xs().text_color(palette.text_secondary).child("Loading…")))
            .when_some(self.error.clone(), |el, e| el.child(div().px_2().py_1().text_sm().text_color(palette.status_conflict).child(e)))
            .child(div().id("pr-list").flex_1().min_h_0().overflow_y_scrollbar().child(rows))
            .into_any_element()
    }
}

impl PullRequestsView {
    /// Author, Label, Assignee and Review filters under the search field;
    /// the choices come from the listed PRs, as IntelliJ's popups offer them.
    fn filter_bar(&self, palette: &crate::theme::Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let mut authors: Vec<String> = self.prs.iter().map(|p| p.user.login.clone()).collect();
        let mut labels: Vec<String> = self.prs.iter().flat_map(|p| p.labels.iter().map(|l| l.name.clone())).collect();
        let mut assignees: Vec<String> = self.prs.iter().flat_map(|p| p.assignees.iter().map(|a| a.login.clone())).collect();
        for list in [&mut authors, &mut labels, &mut assignees] {
            list.sort_by_key(|v| v.to_lowercase());
            list.dedup();
        }
        let entity = cx.entity();
        let chooser = |id: &'static str, name: &'static str, current: Option<String>, values: Vec<String>, set: fn(&mut Filters, Option<String>)| {
            let entity = entity.clone();
            let label = match &current {
                Some(value) => format!("{name}: {value}"),
                None => name.to_owned(),
            };
            Button::new(id).ghost().xsmall().label(label).selected(current.is_some()).dropdown_menu(move |mut menu, _, _| {
                let pick = |value: Option<String>| {
                    let entity = entity.clone();
                    move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
                        entity.update(cx, |this, cx| {
                            set(&mut this.filters, value.clone());
                            cx.notify();
                        })
                    }
                };
                menu = menu.item(PopupMenuItem::new("Any").checked(current.is_none()).on_click(pick(None)));
                if values.is_empty() {
                    menu = menu.item(PopupMenuItem::new(format!("No {}s in the list", name.to_lowercase())).disabled(true));
                }
                for value in &values {
                    menu = menu.item(PopupMenuItem::new(value.clone()).checked(current.as_ref() == Some(value)).on_click(pick(Some(value.clone()))));
                }
                menu
            })
        };
        let review = self.filters.review;
        let gitlab = self.target.as_ref().is_some_and(PrTarget::gitlab);
        let review_entity = entity.clone();
        let any_filter = self.filters.author.is_some() || self.filters.label.is_some() || self.filters.assignee.is_some() || review.is_some();
        h_flex()
            .px_1()
            .gap_0p5()
            .flex_wrap()
            .border_b_1()
            .border_color(palette.border)
            .child(chooser("pr-author", "Author", self.filters.author.clone(), authors, |f, v| f.author = v))
            .child(chooser("pr-label", "Label", self.filters.label.clone(), labels, |f, v| f.label = v))
            .child(chooser("pr-assignee", "Assignee", self.filters.assignee.clone(), assignees, |f, v| f.assignee = v))
            .child(
                Button::new("pr-review")
                    .ghost()
                    .xsmall()
                    .label(review.map_or_else(|| "Review".to_owned(), |r| format!("Review: {}", r.label())))
                    .selected(review.is_some())
                    .dropdown_menu(move |mut menu, _, _| {
                        let pick = |value: Option<ReviewFilter>| {
                            let entity = review_entity.clone();
                            move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
                                entity.update(cx, |this, cx| {
                                    this.filters.review = value;
                                    this.review_matches = None;
                                    this.refresh(cx)
                                })
                            }
                        };
                        menu = menu.item(PopupMenuItem::new("Any").checked(review.is_none()).on_click(pick(None)));
                        // GitLab has no review-state search: only the reviewers list.
                        for filter in ReviewFilter::ALL.into_iter().filter(|f| !gitlab || f.qualifier().is_none()) {
                            menu = menu.item(PopupMenuItem::new(filter.label()).checked(review == Some(filter)).on_click(pick(Some(filter))));
                        }
                        menu
                    }),
            )
            .when(any_filter, |el| {
                el.child(tool_button("pr-clear-filters", IconName::Close, "Clear Filters").on_click(cx.listener(|this, _, _, cx| {
                    let reload = this.filters.review.is_some();
                    this.filters = Filters::default();
                    this.review_matches = None;
                    if reload { this.refresh(cx) } else { cx.notify() }
                })))
            })
    }

    fn render_details(&self, details: &Details, palette: &crate::theme::Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let pr = &details.pr;
        let status = pr.status();
        let open = pr.state == "open";
        let web_url = pr.html_url.clone();
        let entity = cx.entity();
        let mut files = v_flex();
        for (ix, file) in details.files.iter().enumerate() {
            let file_for_click = file.clone();
            let (dir, name) = file.filename.rsplit_once('/').map_or(("", file.filename.as_str()), |(d, n)| (d, n));
            let color = match file.status.as_str() {
                "added" => palette.status_added,
                "removed" => palette.status_deleted,
                "renamed" => palette.status_renamed,
                _ => palette.status_modified,
            };
            files = files.child(
                h_flex()
                    .id(("pr-file", ix))
                    .h(px(row_height()))
                    .px_2()
                    .gap_1p5()
                    .text_sm()
                    .cursor_pointer()
                    .hover(|s| s.bg(palette.hover))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_file(&file_for_click, cx)))
                    .child(Icon::new(crate::ui::common::file_icon(&file.filename)).small().text_color(palette.text_secondary))
                    .child(div().text_color(color).child(name.to_owned()))
                    .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().text_xs().text_color(palette.text_secondary).child(dir.to_owned()))
                    .child(div().text_xs().text_color(palette.status_added).child(format!("+{}", file.additions)))
                    .child(div().text_xs().text_color(palette.status_deleted).child(format!("−{}", file.deletions))),
            );
        }
        if details.loading {
            files = files.child(div().p_2().text_xs().text_color(palette.text_secondary).child("Loading files…"));
        }
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(px(crate::ui::common::toolbar_height()))
                    .px_1()
                    .gap_1()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(tool_button("pr-back", IconName::ChevronLeft, "Back to List").on_click(cx.listener(|this, _, _, cx| {
                        this.details = None;
                        cx.notify();
                    })))
                    .child(div().flex_1().text_sm().text_color(palette.text_secondary).child(self.number(pr.number)))
                    .child(tool_button("pr-web", IconName::Globe, if self.title() == "Merge Requests" { "Open on GitLab" } else { "Open on GitHub" }).on_click(move |_, _, cx| cx.open_url(&web_url))),
            )
            .child(
                div().id("pr-details").flex_1().min_h_0().overflow_y_scrollbar().child(
                    v_flex()
                        .p_2()
                        .gap_2()
                        .child(div().text_base().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(pr.title.clone()))
                        .child(
                            h_flex()
                                .gap_2()
                                .text_xs()
                                .child(div().px_1().rounded(px(3.)).bg(state_color(status, palette)).text_color(gpui_kit::white()).child(status))
                                .child(div().text_color(palette.text_secondary).child(format!("{} wants to merge {} into {}", pr.user.login, pr.head.name, pr.base.name))),
                        )
                        .when(!pr.labels.is_empty(), |el| {
                            el.child(h_flex().gap_1().flex_wrap().children(pr.labels.iter().map(|l| label_chip(l, palette))))
                        })
                        .when(!pr.assignees.is_empty() || !pr.requested_reviewers.is_empty(), |el| {
                            let names = |users: &[github::User]| users.iter().map(|u| u.login.clone()).collect::<Vec<_>>().join(", ");
                            el.child(
                                v_flex()
                                    .text_xs()
                                    .text_color(palette.text_secondary)
                                    .when(!pr.requested_reviewers.is_empty(), |el| el.child(format!("Reviewers: {}", names(&pr.requested_reviewers))))
                                    .when(!pr.assignees.is_empty(), |el| el.child(format!("Assignees: {}", names(&pr.assignees)))),
                            )
                        })
                        .when_some(pr.body.clone().filter(|b| !b.trim().is_empty()), |el, body| {
                            el.child(div().text_sm().child(TextView::markdown(SharedString::from(format!("pr-body-{}", pr.number)), body)))
                        })
                        .child(
                            h_flex()
                                .gap_1()
                                .flex_wrap()
                                .child(Button::new("pr-timeline").xsmall().outline().icon(IconName::MessageSquare).label("Timeline").on_click(cx.listener(|this, _, _, cx| {
                                    if let (Some(target), Some(details)) = (this.target.clone(), this.details.as_ref()) {
                                        cx.emit(PrEvent::OpenTimeline(target, details.pr.clone()));
                                    }
                                })))
                                .child(Button::new("pr-checkout").xsmall().outline().icon(IconName::GitBranch).label("Checkout").on_click(cx.listener(|this, _, _, cx| this.checkout(cx))))
                                .when(open, |el| {
                                    el.child(Button::new("pr-approve").xsmall().outline().icon(IconName::Check).label("Approve").on_click(cx.listener(|this, _, _, cx| {
                                        this.api_op("Approve", |c, repo, n| {
                                            c.submit_review(repo, n, ReviewEvent::Approve, "")?;
                                            Ok(format!("Approved #{n}"))
                                        }, cx)
                                    })))
                                    .child(Button::new("pr-changes").xsmall().outline().label("Request Changes").on_click(cx.listener(|this, _, window, cx| this.request_changes(window, cx))))
                                    .child(Button::new("pr-merge").xsmall().primary().label("Merge").dropdown_menu(move |menu, _, _| {
                                        let item = |label: &'static str, method: MergeMethod| {
                                            let entity = entity.clone();
                                            PopupMenuItem::new(label).on_click(move |_, window, cx| {
                                                entity.update(cx, |this, cx| this.merge(method, window, cx))
                                            })
                                        };
                                        menu.item(item("Merge…", MergeMethod::Merge))
                                            .item(item("Squash and Merge…", MergeMethod::Squash))
                                            .item(item("Rebase and Merge…", MergeMethod::Rebase))
                                    }))
                                }),
                        )
                        .child(div().pt_1().text_xs().text_color(palette.text_secondary).child(format!("{} file{} changed", details.files.len(), if details.files.len() == 1 { "" } else { "s" })))
                        .child(div().rounded(px(4.)).border_1().border_color(palette.border).child(files)),
                ),
            )
    }
}

/// The conversation of a PR in the editor area: description, comments,
/// reviews and line comments, with a box to add a comment.
pub struct PrTimelineView {
    target: PrTarget,
    pr: PullRequest,
    comments: Vec<Comment>,
    input: Entity<TextareaState>,
    loading: bool,
    error: Option<String>,
    _load: Option<Task<()>>,
}

impl PrTimelineView {
    pub fn new(target: PrTarget, pr: PullRequest, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextareaState::new(window, cx).rows(3).placeholder("Leave a comment"));
        let mut this = Self { target, pr, comments: Vec::new(), input, loading: true, error: None, _load: None };
        this.reload(cx);
        this
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let (target, number) = (self.target.clone(), self.pr.number);
        self.loading = true;
        self._load = Some(cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { Client::new(&target.account).timeline(&target.repo, number) }).await;
            this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(comments) => this.comments = comments,
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn post(&mut self, event: Option<ReviewEvent>, window: &mut Window, cx: &mut Context<Self>) {
        let body = self.input.read(cx).value().trim().to_owned();
        if body.is_empty() && event != Some(ReviewEvent::Approve) {
            // GitHub needs a body to request changes; say so, as IntelliJ does.
            self.error = Some(if event == Some(ReviewEvent::RequestChanges) {
                "Enter a comment to request changes".into()
            } else {
                "Enter a comment".into()
            });
            self.input.update(cx, |s, cx| s.focus(window, cx));
            cx.notify();
            return;
        }
        self.error = None;
        self.input.update(cx, |s, cx| s.set_value("", window, cx));
        let (target, number) = (self.target.clone(), self.pr.number);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let client = Client::new(&target.account);
                    match event {
                        Some(event) => client.submit_review(&target.repo, number, event, &body),
                        None => client.add_comment(&target.repo, number, &body),
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                if let Err(error) = result {
                    this.error = Some(error.to_string());
                }
                this.reload(cx);
            })
            .ok();
        })
        .detach();
    }
}

impl PrTimelineView {
    /// The editor tab's title: "Merge Request !5" / "Pull Request #7".
    pub fn title(&self) -> String {
        format!("{} {}", self.target.noun(), self.target.number(self.pr.number))
    }
}

impl Render for PrTimelineView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mut items = v_flex().gap_3();
        let entry = |id: String, author: &str, time: &str, badge: Option<(&str, gpui_kit::Hsla)>, context: Option<String>, body: &str| {
            v_flex()
                .gap_1()
                .p_2()
                .rounded(px(6.))
                .border_1()
                .border_color(palette.border)
                .child(
                    h_flex()
                        .gap_2()
                        .text_sm()
                        .child(div().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(author.to_owned()))
                        .when_some(badge, |el, (label, color)| el.child(div().px_1().rounded(px(3.)).bg(color).text_xs().text_color(gpui_kit::white()).child(label.to_owned())))
                        .when_some(context, |el, c| el.child(div().text_xs().text_color(palette.link).child(c)))
                        .child(div().flex_1())
                        .child(div().text_xs().text_color(palette.text_secondary).child(time.get(..16).unwrap_or(time).replace('T', " "))),
                )
                .when(!body.trim().is_empty(), |el| el.child(div().text_sm().child(TextView::markdown(SharedString::from(id), body.to_owned()))))
        };
        items = items.child(entry(format!("tl-body-{}", self.pr.number), &self.pr.user.login, &self.pr.created_at, Some(("opened", palette.status_added)), None, self.pr.body.as_deref().unwrap_or("No description provided.")));
        for c in &self.comments {
            let badge = match c.state.as_deref() {
                Some("APPROVED") => Some(("approved", palette.status_added)),
                Some("CHANGES_REQUESTED") => Some(("requested changes", palette.status_conflict)),
                Some("COMMENTED") => Some(("reviewed", palette.text_secondary)),
                Some("DISMISSED") => Some(("dismissed", palette.text_secondary)),
                _ => None,
            };
            let context = c.path.as_ref().map(|p| match c.line {
                Some(line) => format!("{p}:{line}"),
                None => p.clone(),
            });
            if badge.is_some() && c.body.as_deref().unwrap_or_default().trim().is_empty() && context.is_none() {
                items = items.child(entry(format!("tl-{}", c.id), &c.user.login, c.time(), badge, None, ""));
            } else {
                items = items.child(entry(format!("tl-{}", c.id), &c.user.login, c.time(), badge, context, c.body.as_deref().unwrap_or_default()));
            }
        }
        let open = self.pr.state == "open";
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(px(36.))
                    .px_3()
                    .gap_2()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(Icon::new(IconName::GitPullRequest).small().text_color(state_color(self.pr.status(), &palette)))
                    .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(format!("{} {}", self.pr.title, self.target.number(self.pr.number))))
                    .child(tool_button("tl-refresh", IconName::RefreshCw, "Refresh").on_click(cx.listener(|this, _, _, cx| this.reload(cx)))),
            )
            .when(self.loading, |el| el.child(div().px_3().py_1().text_xs().text_color(palette.text_secondary).child("Loading…")))
            .when_some(self.error.clone(), |el, e| el.child(div().px_3().py_1().text_sm().text_color(palette.status_conflict).child(e)))
            .child(div().id("timeline").flex_1().min_h_0().overflow_y_scrollbar().child(div().p_3().child(items)))
            .child(
                v_flex()
                    .p_3()
                    .gap_2()
                    .border_t_1()
                    .border_color(palette.border)
                    .child(Textarea::new(&self.input))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Button::new("tl-comment").small().primary().label("Comment").on_click(cx.listener(|this, _, window, cx| this.post(None, window, cx))))
                            .when(open, |el| {
                                el.child(Button::new("tl-approve").small().outline().label("Approve").on_click(cx.listener(|this, _, window, cx| this.post(Some(ReviewEvent::Approve), window, cx))))
                                    .child(Button::new("tl-changes").small().outline().label("Request Changes").on_click(cx.listener(|this, _, window, cx| this.post(Some(ReviewEvent::RequestChanges), window, cx))))
                            }),
                    ),
            )
    }
}
