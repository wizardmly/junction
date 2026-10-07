//! Repository state shared by every view, loaded off the UI thread.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use gpui_kit::{AppContext as _, Context, EventEmitter, Task};

use crate::git::{
    self, Commit, CommitDetails, GitConsole, GraphLayout, LogFilter, Repository, RepositoryRefs,
    RepositoryState, WorkingTreeStatus,
};

/// Partial commits: change blocks unchecked in the diff viewer, per file,
/// shared by the diff viewer (which edits them) and the Commit tool window.
#[derive(Clone, Debug, Default)]
pub struct ExcludedHunks(pub HashMap<String, HashSet<u64>>);

impl gpui_kit::Global for ExcludedHunks {}

impl ExcludedHunks {
    pub fn get(cx: &gpui_kit::App) -> &HashMap<String, HashSet<u64>> {
        cx.try_global::<Self>().map(|e| &e.0).unwrap_or_else(|| {
            static EMPTY: std::sync::OnceLock<HashMap<String, HashSet<u64>>> = std::sync::OnceLock::new();
            EMPTY.get_or_init(HashMap::new)
        })
    }

    pub fn update(cx: &mut gpui_kit::App, f: impl FnOnce(&mut HashMap<String, HashSet<u64>>)) {
        let mut value = cx.try_global::<Self>().cloned().unwrap_or_default();
        f(&mut value.0);
        value.0.retain(|_, set| !set.is_empty());
        cx.set_global(value);
    }
}

#[derive(Clone, Debug)]
pub enum RepoEvent {
    /// Refs, log, or status changed.
    Reloaded,
    SelectionChanged,
    DetailsLoaded,
    /// An operation finished; shown as a balloon notification.
    Notify { title: String, message: String, error: bool },
    /// Show the files that differ between two revisions (the working tree when `new` is `None`).
    Compare { old: String, new: Option<String> },
    /// Open another Log tab (File History, Compare with Current, …).
    OpenLogTab { title: String, filter: LogFilter },
    /// Fixup… / Squash Into…: put a message in the commit box.
    PrefillCommitMessage(String),
}

pub struct RepoModel {
    repository: Option<Repository>,
    console: GitConsole,
    refs: RepositoryRefs,
    commits: Arc<Vec<Commit>>,
    graph: Arc<GraphLayout>,
    /// The log as loaded, before Collapse Linear Branches folds it.
    all_commits: Arc<Vec<Commit>>,
    all_graph: Arc<GraphLayout>,
    collapse_linear: bool,
    expanded_runs: HashSet<String>,
    hidden: HashMap<String, usize>,
    status: WorkingTreeStatus,
    state: RepositoryState,
    filter: LogFilter,
    selected: Option<String>,
    details: Option<CommitDetails>,
    user_email: Option<String>,
    loading: bool,
    busy: Option<String>,
    error: Option<String>,
    _reload_task: Option<Task<()>>,
    _details_task: Option<Task<()>>,
    _fetch_task: Option<Task<()>>,
}

impl EventEmitter<RepoEvent> for RepoModel {}

struct Snapshot {
    refs: RepositoryRefs,
    commits: Vec<Commit>,
    graph: GraphLayout,
    status: WorkingTreeStatus,
    state: RepositoryState,
}

impl RepoModel {
    pub fn new(path: Option<PathBuf>, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            repository: None,
            console: GitConsole::default(),
            refs: RepositoryRefs::default(),
            commits: Arc::default(),
            graph: Arc::default(),
            all_commits: Arc::default(),
            all_graph: Arc::default(),
            collapse_linear: false,
            expanded_runs: HashSet::new(),
            hidden: HashMap::new(),
            status: WorkingTreeStatus::default(),
            state: RepositoryState::Normal,
            filter: LogFilter::default(),
            selected: None,
            details: None,
            user_email: None,
            loading: false,
            busy: None,
            error: None,
            _reload_task: None,
            _details_task: None,
            _fetch_task: None,
        };
        this.start_background_fetch(cx);
        if let Some(path) = path {
            this.open(path, cx);
        }
        this
    }

    pub fn open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        match Repository::discover(&path, self.console.clone()) {
            Ok(repository) => {
                crate::settings::remember_project(repository.root());
                self.user_email = repository.current_user().1;
                self.repository = Some(repository);
                self.error = None;
                self.selected = None;
                self.details = None;
                self.reload(cx);
            }
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
            }
        }
    }

    /// Get from Version Control: `git clone` into `dir`, then open it.
    pub fn clone_repository(&mut self, url: String, dir: PathBuf, cx: &mut Context<Self>) {
        self.busy = Some(format!("Cloning {url}"));
        cx.notify();
        let console = self.console.clone();
        cx.spawn(async move |this, cx| {
            let target = dir.clone();
            let result = cx
                .background_spawn(async move {
                    let parent = target.parent().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
                    std::fs::create_dir_all(&parent)?;
                    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    git::run_in(std::path::Path::new("git"), &parent, &console, ["clone", "--progress", url.as_str(), name.as_str()], None, &[])
                        .map(|_| url)
                })
                .await;
            this.update(cx, |this, cx| {
                this.busy = None;
                match result {
                    Ok(url) => {
                        cx.emit(RepoEvent::Notify { title: "Clone".into(), message: format!("Cloned {url}"), error: false });
                        this.open(dir, cx);
                    }
                    Err(error) => cx.emit(RepoEvent::Notify { title: "Clone failed".into(), message: error.to_string(), error: true }),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Create Git Repository: `git init` in `dir`, then open it.
    pub fn init_repository(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        match git::run_in(std::path::Path::new("git"), &dir, &self.console, ["init"], None, &[]) {
            Ok(_) => self.open(dir, cx),
            Err(error) => cx.emit(RepoEvent::Notify { title: "Create Git Repository failed".into(), message: error.to_string(), error: true }),
        }
    }

    pub fn repository(&self) -> Option<&Repository> {
        self.repository.as_ref()
    }

    pub fn console(&self) -> &GitConsole {
        &self.console
    }

    pub fn refs(&self) -> &RepositoryRefs {
        &self.refs
    }

    pub fn commits(&self) -> &Arc<Vec<Commit>> {
        &self.commits
    }

    pub fn graph(&self) -> &Arc<GraphLayout> {
        &self.graph
    }

    pub fn status(&self) -> &WorkingTreeStatus {
        &self.status
    }

    pub fn state(&self) -> RepositoryState {
        self.state
    }

    pub fn filter(&self) -> &LogFilter {
        &self.filter
    }

    pub fn details(&self) -> Option<&CommitDetails> {
        self.details.as_ref()
    }

    pub fn selected_hash(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    pub fn selected_index(&self) -> Option<usize> {
        let hash = self.selected.as_deref()?;
        self.commits.iter().position(|c| c.hash == hash)
    }

    pub fn user_email(&self) -> Option<&str> {
        self.user_email.as_deref()
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn busy(&self) -> Option<&str> {
        self.busy.as_deref()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn set_filter(&mut self, filter: LogFilter, cx: &mut Context<Self>) {
        if self.filter != filter {
            self.filter = filter;
            self.reload(cx);
        }
    }

    /// Reloads refs, the log, and working tree status in the background.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.clone() else { return };
        let filter = self.filter.clone();
        self.loading = true;
        cx.notify();
        self._reload_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let refs = RepositoryRefs::load(&repository)?;
                    // The first page shows quickly; the rest follows below.
                    let commits = git::log::load_log(&repository, &filter, Some(git::log::FIRST_PAGE))?;
                    let graph = GraphLayout::build(&commits);
                    let status = WorkingTreeStatus::load(&repository)?;
                    let complete = commits.len() < git::log::FIRST_PAGE;
                    anyhow::Ok((Snapshot { refs, commits, graph, status, state: repository.state() }, complete, repository, filter))
                })
                .await;
            let (result, rest) = match result {
                Ok((snapshot, complete, repository, filter)) => (Ok(snapshot), (!complete).then_some((repository, filter))),
                Err(error) => (Err(error), None),
            };
            this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(snapshot) => {
                        this.refs = snapshot.refs;
                        this.set_log(snapshot.commits, snapshot.graph);
                        this.status = snapshot.status;
                        this.state = snapshot.state;
                        this.error = None;
                        // Keep the selection if the commit is still listed, else
                        // select HEAD the way the Log does on first open.
                        let keep = this.selected.as_ref().is_some_and(|h| this.commits.iter().any(|c| &c.hash == h));
                        if !keep {
                            let head = this.refs.head_commit.clone();
                            let target = head
                                .filter(|h| this.commits.iter().any(|c| &c.hash == h))
                                .or_else(|| this.commits.first().map(|c| c.hash.clone()));
                            this.select_hash(target, cx);
                        } else if let Some(hash) = this.selected.clone() {
                            this.load_details(hash, cx);
                        }
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.emit(RepoEvent::Reloaded);
                cx.notify();
            })
            .ok();

            let Some((repository, filter)) = rest else { return };
            let full = cx
                .background_spawn(async move {
                    let commits = git::log::load_log(&repository, &filter, None)?;
                    let graph = GraphLayout::build(&commits);
                    anyhow::Ok((commits, graph))
                })
                .await;
            this.update(cx, |this, cx| {
                if let Ok((commits, graph)) = full {
                    this.set_log(commits, graph);
                    cx.emit(RepoEvent::Reloaded);
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    fn set_log(&mut self, commits: Vec<Commit>, graph: GraphLayout) {
        self.all_commits = Arc::new(commits);
        self.all_graph = Arc::new(graph);
        self.apply_collapse();
    }

    fn apply_collapse(&mut self) {
        if !self.collapse_linear {
            self.commits = self.all_commits.clone();
            self.graph = self.all_graph.clone();
            self.hidden.clear();
            return;
        }
        let refs = &self.refs;
        let (commits, hidden) =
            git::log::collapse_linear(&self.all_commits, |h| !refs.for_commit(h).is_empty(), &self.expanded_runs);
        self.graph = Arc::new(GraphLayout::build(&commits));
        self.commits = Arc::new(commits);
        self.hidden = hidden;
    }

    pub fn collapse_linear(&self) -> bool {
        self.collapse_linear
    }

    /// How many commits are folded below `hash`, if any.
    pub fn hidden_below(&self, hash: &str) -> Option<usize> {
        self.hidden.get(hash).copied()
    }

    pub fn set_collapse_linear(&mut self, collapse: bool, cx: &mut Context<Self>) {
        self.collapse_linear = collapse;
        self.expanded_runs.clear();
        self.apply_collapse();
        cx.emit(RepoEvent::Reloaded);
        cx.notify();
    }

    pub fn expand_run(&mut self, hash: String, cx: &mut Context<Self>) {
        self.expanded_runs.insert(hash);
        self.apply_collapse();
        cx.emit(RepoEvent::Reloaded);
        cx.notify();
    }

    pub fn select_index(&mut self, ix: usize, cx: &mut Context<Self>) {
        let hash = self.commits.get(ix).map(|c| c.hash.clone());
        self.select_hash(hash, cx);
    }

    /// Shows a balloon notification without running an operation.
    pub fn notify(&mut self, title: impl Into<String>, message: impl Into<String>, error: bool, cx: &mut Context<Self>) {
        cx.emit(RepoEvent::Notify { title: title.into(), message: message.into(), error });
    }

    pub fn open_log_tab(&mut self, title: String, filter: LogFilter, cx: &mut Context<Self>) {
        cx.emit(RepoEvent::OpenLogTab { title, filter });
    }

    pub fn prefill_commit_message(&mut self, message: String, cx: &mut Context<Self>) {
        cx.emit(RepoEvent::PrefillCommitMessage(message));
    }

    pub fn compare(&mut self, old: String, new: Option<String>, cx: &mut Context<Self>) {
        cx.emit(RepoEvent::Compare { old, new });
    }

    pub fn select_hash(&mut self, hash: Option<String>, cx: &mut Context<Self>) {
        if self.selected == hash {
            return;
        }
        self.selected = hash.clone();
        self.details = None;
        cx.emit(RepoEvent::SelectionChanged);
        cx.notify();
        if let Some(hash) = hash {
            self.load_details(hash, cx);
        }
    }

    fn load_details(&mut self, hash: String, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.clone() else { return };
        self._details_task = Some(cx.spawn(async move |this, cx| {
            let details = cx
                .background_spawn(async move { git::log::load_details(&repository, &hash) })
                .await;
            this.update(cx, |this, cx| {
                if let Ok(details) = details {
                    if this.selected.as_deref() == Some(details.hash.as_str()) {
                        this.details = Some(details);
                        cx.emit(RepoEvent::DetailsLoaded);
                        cx.notify();
                    }
                }
            })
            .ok();
        }));
    }

    /// IntelliJ's "Update branch info": a quiet `git fetch` every few minutes
    /// so incoming commits show up as ↓N in the branches popup and the Log.
    /// Failures (offline, credentials) stay silent, as in IntelliJ.
    fn start_background_fetch(&mut self, cx: &mut Context<Self>) {
        self._fetch_task = Some(cx.spawn(async move |this, cx| {
            let mut last_fetch = std::time::Instant::now();
            loop {
                cx.background_executor().timer(std::time::Duration::from_secs(30)).await;
                let Ok(repository) = this.update(cx, |this, cx| {
                    let minutes = crate::settings::Settings::get(cx).fetch_interval_minutes;
                    let due = minutes > 0 && last_fetch.elapsed().as_secs() >= minutes as u64 * 60;
                    // Never race a user-started operation.
                    (due && this.busy.is_none()).then(|| this.repository.clone()).flatten()
                }) else {
                    break;
                };
                let Some(repository) = repository else { continue };
                last_fetch = std::time::Instant::now();
                let changed = cx
                    .background_spawn(async move {
                        let remotes = repository.run(["remote"]).unwrap_or_default();
                        if remotes.trim().is_empty() {
                            return false;
                        }
                        let before = repository.run(["for-each-ref", "refs/remotes"]).unwrap_or_default();
                        let _ = repository.run(["fetch", "--all", "--prune", "--quiet"]);
                        repository.run(["for-each-ref", "refs/remotes"]).unwrap_or_default() != before
                    })
                    .await;
                if changed {
                    this.update(cx, |this, cx| this.reload(cx)).ok();
                }
            }
        }));
    }

    /// Runs a git operation in the background, then reloads and reports the
    /// result as a notification, the way IntelliJ reports VCS operations.
    pub fn run_operation(
        &mut self,
        title: impl Into<String>,
        operation: impl FnOnce(&Repository) -> Result<String> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(repository) = self.repository.clone() else { return };
        let title = title.into();
        self.busy = Some(title.clone());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { operation(&repository) }).await;
            this.update(cx, |this, cx| {
                this.busy = None;
                let event = match result {
                    Ok(message) => RepoEvent::Notify { title, message, error: false },
                    Err(error) => RepoEvent::Notify { title: format!("{title} failed"), message: error.to_string(), error: true },
                };
                cx.emit(event);
                this.reload(cx);
            })
            .ok();
        })
        .detach();
    }
}
