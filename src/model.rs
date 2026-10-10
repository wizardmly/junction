//! Repository state shared by every view, loaded off the UI thread.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
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
        // Both the diff and the Commit tool window show these.
        cx.refresh_windows();
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
    /// An operation stopped on conflicts: open the Conflicts dialog, as
    /// IntelliJ does after a merge, rebase or cherry-pick (sent once the
    /// status has been reloaded).
    ShowConflicts,
    /// Get from Version Control finished cloning into this directory; the
    /// workspace opens it, asking This Window / New Window when a project
    /// is open, as IntelliJ does.
    Cloned(PathBuf),
}

pub struct RepoModel {
    repository: Option<Repository>,
    console: GitConsole,
    /// Shared, so the views' per-frame copies are cheap.
    refs: Arc<RepositoryRefs>,
    commits: Arc<Vec<Commit>>,
    graph: Arc<GraphLayout>,
    /// The log as loaded, before Collapse Linear Branches folds it.
    all_commits: Arc<Vec<Commit>>,
    all_graph: Arc<GraphLayout>,
    /// Row lookup by hash, for `commits` and `all_commits`.
    rows: Arc<git::log::CommitRows>,
    all_rows: Arc<git::log::CommitRows>,
    /// What the loaded log shows: reloads keep it while refs and filter stay the same.
    log_key: Option<LogKey>,
    collapse_linear: bool,
    expanded_runs: HashSet<String>,
    hidden: HashMap<String, usize>,
    status: WorkingTreeStatus,
    state: RepositoryState,
    filter: LogFilter,
    selected: Option<String>,
    details: Option<CommitDetails>,
    user_email: Option<String>,
    /// Paths of submodules (gitlinks), drawn with a repository icon.
    submodule_paths: HashSet<String>,
    /// The project's own repository; other roots are switched to in place.
    project_root: Option<PathBuf>,
    roots: Vec<RootInfo>,
    /// Where "Open on GitHub / GitLab" points, from the tracked remote.
    web_repo: Option<git::hosting::WebRepo>,
    loading: bool,
    busy: Option<String>,
    /// An operation just stopped on conflicts; the next reload opens the
    /// Conflicts dialog.
    conflicts_pending: bool,
    error: Option<String>,
    open_problem: Option<OpenProblem>,
    failed_path: Option<PathBuf>,
    _reload_task: Option<Task<()>>,
    _log_task: Option<Task<()>>,
    _details_task: Option<Task<()>>,
    _fetch_task: Option<Task<()>>,
}

impl EventEmitter<RepoEvent> for RepoModel {}

/// One VCS root of the project and the branch checked out there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RootInfo {
    pub path: PathBuf,
    pub branch: Option<String>,
}

/// A failed open the user can fix from the Welcome screen.
#[derive(Clone, Debug)]
pub enum OpenProblem {
    GitMissing,
    /// The repository root git refused (`safe.directory`).
    Unsafe(PathBuf),
}

/// The log's inputs: every ref tip (the log lists `--all`), HEAD, the
/// repository and the filter.
#[derive(Clone, Debug, PartialEq)]
struct LogKey {
    root: PathBuf,
    tips: String,
    filter: LogFilter,
}

impl LogKey {
    fn load(repository: &Repository, filter: &LogFilter) -> Self {
        let tips = repository.run(["for-each-ref", "--format=%(objectname) %(refname)"]).unwrap_or_default();
        let head = repository.run(["rev-parse", "-q", "--verify", "HEAD"]).unwrap_or_default();
        Self { root: repository.root().to_path_buf(), tips: tips + &head, filter: filter.clone() }
    }
}

struct LoadedLog {
    commits: Vec<Commit>,
    graph: GraphLayout,
    rows: git::log::CommitRows,
}

impl LoadedLog {
    fn load(repository: &Repository, filter: &LogFilter, first_page: bool) -> Result<Self> {
        let mut commits = if first_page { git::log::load_first_page(repository, filter)? } else { Vec::new() };
        // The first page leaves out tags; a short log is loaded whole right away.
        if commits.len() < git::log::FIRST_PAGE {
            commits = git::log::load_log(repository, filter, None)?;
        }
        let graph = GraphLayout::build(&commits);
        let rows = git::log::CommitRows::build(&commits);
        Ok(Self { commits, graph, rows })
    }
}

struct Snapshot {
    roots: Vec<RootInfo>,
    web: Option<git::hosting::WebRepo>,
    refs: RepositoryRefs,
    /// The first page of the log when it changed, and its key.
    log: Option<(LoadedLog, LogKey)>,
    status: WorkingTreeStatus,
    state: RepositoryState,
    submodules: HashSet<String>,
}

impl RepoModel {
    pub fn new(path: Option<PathBuf>, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            repository: None,
            console: GitConsole::default(),
            refs: Arc::default(),
            commits: Arc::default(),
            graph: Arc::default(),
            all_commits: Arc::default(),
            all_graph: Arc::default(),
            rows: Arc::default(),
            all_rows: Arc::default(),
            log_key: None,
            collapse_linear: false,
            expanded_runs: HashSet::new(),
            hidden: HashMap::new(),
            status: WorkingTreeStatus::default(),
            state: RepositoryState::Normal,
            filter: LogFilter::default(),
            selected: None,
            details: None,
            user_email: None,
            submodule_paths: HashSet::new(),
            project_root: None,
            roots: Vec::new(),
            web_repo: None,
            loading: false,
            busy: None,
            conflicts_pending: false,
            error: None,
            open_problem: None,
            failed_path: None,
            _reload_task: None,
            _log_task: None,
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
                git::migrate_legacy_data(&repository);
                self.open_problem = None;
                self.failed_path = None;
                crate::settings::remember_project(repository.root());
                self.project_root = Some(repository.root().to_path_buf());
                self.roots.clear();
                self.user_email = repository.current_user().1;
                self.repository = Some(repository);
                self.error = None;
                self.selected = None;
                self.details = None;
                self.reload(cx);
            }
            Err(error) => {
                self.open_problem = match error.downcast_ref::<git::OpenError>() {
                    Some(git::OpenError::GitMissing(_)) => Some(OpenProblem::GitMissing),
                    Some(git::OpenError::Unsafe(root)) => Some(OpenProblem::Unsafe(root.clone())),
                    None => None,
                };
                self.failed_path = Some(path);
                self.error = Some(error.to_string());
                cx.notify();
            }
        }
    }

    /// What the Welcome screen can offer for the last failed open.
    pub fn open_problem(&self) -> Option<&OpenProblem> {
        self.open_problem.as_ref()
    }

    /// Opens the folder that failed last time again (after installing git
    /// or trusting the directory).
    pub fn retry_open(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self.failed_path.clone() {
            git::redetect_executable();
            self.open(path, cx);
        }
    }

    /// Makes another root of the project the active one: the Log, Commit
    /// window and branch widget then show that repository.
    pub fn switch_root(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.repository.as_ref().is_some_and(|r| r.root() == path) {
            return;
        }
        match Repository::discover(&path, self.console.clone()) {
            Ok(repository) => {
                self.user_email = repository.current_user().1;
                self.repository = Some(repository);
                self.selected = None;
                self.details = None;
                self.reload(cx);
            }
            Err(error) => cx.emit(RepoEvent::Notify { title: "Switch Repository".into(), message: error.to_string(), error: true }),
        }
    }

    pub fn web_repo(&self) -> Option<&git::hosting::WebRepo> {
        self.web_repo.as_ref()
    }

    pub fn project_root(&self) -> Option<&Path> {
        self.project_root.as_deref()
    }

    /// The project's roots (the project repository first); a single entry
    /// unless it has nested repositories or mapped directories.
    pub fn roots(&self) -> &[RootInfo] {
        &self.roots
    }

    /// Directory Mappings changed: scan for roots again.
    pub fn rescan_roots(&mut self, cx: &mut Context<Self>) {
        self.roots.clear();
        self.reload(cx);
    }

    /// Roots other than the active one, for synchronous branch control.
    pub fn other_roots(&self) -> Vec<PathBuf> {
        let active = self.repository.as_ref().map(|r| r.root().to_path_buf());
        self.roots.iter().map(|r| r.path.clone()).filter(|p| Some(p) != active.as_ref()).collect()
    }

    /// The label of the active root ("app", "libs/core").
    pub fn active_root_label(&self) -> Option<String> {
        let (project, repo) = (self.project_root.as_ref()?, self.repository.as_ref()?);
        Some(git::roots::label(project, repo.root()))
    }

    pub fn is_multi_root(&self) -> bool {
        self.roots.len() > 1
    }

    /// Get from Version Control: `git clone` into `dir`, then open it (see `RepoEvent::Cloned`).
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
                    git::run_in(&git::executable(), &parent, &console, ["clone", "--progress", url.as_str(), name.as_str()], None, &[])
                        .map(|_| url)
                })
                .await;
            this.update(cx, |this, cx| {
                this.busy = None;
                match result {
                    Ok(url) => {
                        cx.emit(RepoEvent::Notify { title: "Clone".into(), message: format!("Cloned {url}"), error: false });
                        cx.emit(RepoEvent::Cloned(dir));
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
        match git::run_in(&git::executable(), &dir, &self.console, ["init"], None, &[]) {
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

    pub fn refs(&self) -> &Arc<RepositoryRefs> {
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
        self.row_of(self.selected.as_deref()?)
    }

    /// The Log row showing a commit.
    pub fn row_of(&self, hash: &str) -> Option<usize> {
        self.rows.get(&self.commits, hash)
    }

    pub fn submodule_paths(&self) -> &HashSet<String> {
        &self.submodule_paths
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
        let mut filter = self.filter.clone();
        filter.date_order = crate::settings::Settings::get(cx).log.sort_by_date;
        let project_root = self.project_root.clone();
        let console = self.console.clone();
        // Roots are scanned once per project (and after Directory Mappings
        // change); each reload only refreshes their branches.
        let known_roots: Vec<PathBuf> = self.roots.iter().map(|r| r.path.clone()).collect();
        let previous_key = self.log_key.clone();
        self.loading = true;
        cx.notify();
        self._reload_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let refs = RepositoryRefs::load(&repository)?;
                    // The log only changes with the refs (or the filter); a
                    // reload after staging or editing keeps it.
                    let key = LogKey::load(&repository, &filter);
                    let log = if previous_key.as_ref() != Some(&key) {
                        // The first page shows quickly; the rest follows below.
                        Some((LoadedLog::load(&repository, &filter, true)?, key))
                    } else {
                        None
                    };
                    let status = WorkingTreeStatus::load(&repository)?;
                    let submodules = if repository.root().join(".gitmodules").exists() {
                        git::submodule::gitlink_paths(&repository).into_iter().collect()
                    } else {
                        HashSet::new()
                    };
                    let paths = if known_roots.is_empty() {
                        project_root
                            .as_ref()
                            .and_then(|p| Repository::discover(p, console.clone()).ok())
                            .map(|project| git::roots::detect(&project))
                            .unwrap_or_else(|| vec![repository.root().to_path_buf()])
                    } else {
                        known_roots
                    };
                    let roots = paths
                        .into_iter()
                        .map(|path| {
                            let branch = git::run_in(&git::executable(), &path, &GitConsole::default(), ["symbolic-ref", "--short", "-q", "HEAD"], None, &[])
                                .ok()
                                .map(|b| b.trim().to_owned())
                                .filter(|b| !b.is_empty());
                            RootInfo { path, branch }
                        })
                        .collect();
                    let web = git::hosting::web_repo(&repository);
                    anyhow::Ok((Snapshot { roots, web, refs, log, status, state: repository.state(), submodules }, repository, filter))
                })
                .await;
            this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok((snapshot, repository, filter)) => {
                        this.refs = Arc::new(snapshot.refs);
                        if let Some((log, key)) = snapshot.log {
                            let complete = log.commits.len() < git::log::FIRST_PAGE;
                            // (A short log came whole; a long one was the first page.)
                            this.set_log(log);
                            this.log_key = Some(key);
                            // A new log replaces one still loading.
                            this._log_task = (!complete).then(|| this.load_full_log(repository, filter, cx));
                        }
                        this.status = snapshot.status;
                        this.state = snapshot.state;
                        this.submodule_paths = snapshot.submodules;
                        this.roots = snapshot.roots;
                        this.web_repo = snapshot.web;
                        this.error = None;
                        // Keep the selection if the commit is still listed, else
                        // select HEAD the way the Log does on first open.
                        let keep = this.selected.as_deref().is_some_and(|h| this.row_of(h).is_some());
                        if !keep {
                            let head = this.refs.head_commit.clone();
                            let target = head
                                .filter(|h| this.row_of(h).is_some())
                                .or_else(|| this.commits.first().map(|c| c.hash.clone()));
                            this.select_hash(target, cx);
                        } else if let Some(hash) = this.selected.clone() {
                            this.load_details(hash, cx);
                        }
                    }
                    Err(error) => {
                        this.error = Some(error.to_string());
                        this.log_key = None;
                    }
                }
                cx.emit(RepoEvent::Reloaded);
                if std::mem::take(&mut this.conflicts_pending) && !git::merge::conflicts(&this.status).is_empty() {
                    cx.emit(RepoEvent::ShowConflicts);
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Loads the whole log after its first page, then refreshes git's
    /// commit-graph so the next first page comes sorted at once.
    fn load_full_log(&mut self, repository: Repository, filter: LogFilter, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            let full = cx.background_spawn({
                let repository = repository.clone();
                async move { LoadedLog::load(&repository, &filter, false) }
            });
            let full = full.await;
            this.update(cx, |this, cx| match full {
                Ok(log) => {
                    this.set_log(log);
                    cx.emit(RepoEvent::Reloaded);
                    cx.notify();
                }
                // Load the log again on the next reload.
                Err(_) => this.log_key = None,
            })
            .ok();
            cx.background_spawn(async move { git::log::write_commit_graph(&repository) }).await;
        })
    }

    fn set_log(&mut self, log: LoadedLog) {
        self.all_commits = Arc::new(log.commits);
        self.all_graph = Arc::new(log.graph);
        self.all_rows = Arc::new(log.rows);
        self.apply_collapse();
    }

    fn apply_collapse(&mut self) {
        if !self.collapse_linear {
            self.commits = self.all_commits.clone();
            self.graph = self.all_graph.clone();
            self.rows = self.all_rows.clone();
            self.hidden.clear();
            return;
        }
        let refs = &self.refs;
        let (commits, hidden) =
            git::log::collapse_linear(&self.all_commits, |h| !refs.for_commit(h).is_empty(), &self.expanded_runs);
        self.graph = Arc::new(GraphLayout::build(&commits));
        self.rows = Arc::new(git::log::CommitRows::build(&commits));
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

    /// Undo Commit: `reset --soft` to the parent of `hash` if it is still
    /// HEAD, putting its message back into the commit message editor.
    pub fn undo_commit(&mut self, hash: String, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.clone() else { return };
        let head = repository.run(["rev-parse", "HEAD"]).unwrap_or_default();
        if head.trim() != hash {
            self.notify("Undo Commit", "The commit is no longer HEAD; use Reset or Revert from the Log", true, cx);
            return;
        }
        let message = repository.run(["log", "-1", "--format=%B", &hash]).unwrap_or_default();
        self.prefill_commit_message(message.trim_end().to_owned(), cx);
        self.run_operation("Undo Commit", |repo| {
            repo.run(["reset", "--soft", "HEAD~1"])?;
            Ok("Commit undone; changes kept".into())
        }, cx);
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

    /// Like `run_operation`, but hands the result to `done` (which reports
    /// it) instead of notifying, for operations that may need to ask next.
    pub fn run_task<T: Send + 'static>(
        &mut self,
        title: impl Into<String>,
        operation: impl FnOnce(&Repository) -> Result<T> + Send + 'static,
        done: impl FnOnce(&mut Self, Result<T>, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(repository) = self.repository.clone() else { return };
        self.busy = Some(title.into());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { operation(&repository) }).await;
            this.update(cx, |this, cx| {
                this.busy = None;
                done(this, result, cx);
                this.reload(cx);
            })
            .ok();
        })
        .detach();
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
        // Conflicts that were already there are the Conflicts dialog's own
        // business (Accept Yours, Merge…); only new ones open it.
        let had_conflicts = !git::merge::conflicts(&self.status).is_empty();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let result = operation(&repository);
                    let conflicted = result.is_err()
                        && !had_conflicts
                        && repository.run(["diff", "--name-only", "--diff-filter=U"]).is_ok_and(|o| !o.trim().is_empty());
                    (result, conflicted)
                })
                .await;
            this.update(cx, |this, cx| {
                this.busy = None;
                let (result, conflicted) = result;
                let event = match result {
                    Ok(message) => RepoEvent::Notify { title, message, error: false },
                    // The Conflicts dialog says it all; no error balloon.
                    Err(_) if conflicted => {
                        this.conflicts_pending = true;
                        RepoEvent::Notify { title, message: String::new(), error: false }
                    }
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
