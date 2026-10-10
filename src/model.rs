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
    /// Only the log changed (a filter, its full history arriving after the
    /// first page): the Log redraws, nothing else needs to reload.
    LogLoaded,
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
    /// Every root's repository, refs and changes in a multi-root project
    /// (empty with a single root), in the order of `roots`.
    root_states: Arc<Vec<RootState>>,
    /// The changes of every root, paths relative to the project: what the
    /// Commit tool window lists.
    project_status: Arc<WorkingTreeStatus>,
    /// The branches and tags of every root the Log shows, for its labels,
    /// branches tree and Branch filter.
    log_refs: Arc<RepositoryRefs>,
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
    _stale_details_task: Option<Task<()>>,
    _read_ahead_task: Option<Task<()>>,
    /// Details of commits seen or read ahead; cleared when the refs may have
    /// changed (their branch lists).
    details_cache: HashMap<String, git::log::CommitDetails>,
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

/// One root of a multi-root project, loaded with every reload: the Commit
/// tool window lists its changes and the Log its commits next to the
/// other roots', as IntelliJ does.
#[derive(Clone)]
pub struct RootState {
    pub path: PathBuf,
    /// The root's folder relative to the project, `/`-separated; empty for
    /// the project's own repository. Commit window paths start with it.
    pub prefix: String,
    pub repository: Repository,
    pub refs: Arc<RepositoryRefs>,
    pub status: Arc<WorkingTreeStatus>,
    pub state: RepositoryState,
    pub web: Option<git::hosting::WebRepo>,
    pub user_email: Option<String>,
    pub submodules: HashSet<String>,
}

impl RootState {
    fn load(path: &Path, project: Option<&Path>, console: &GitConsole) -> Option<Self> {
        let repository = Repository::discover(path, console.clone()).ok()?;
        let root = repository.root().to_path_buf();
        let refs = RepositoryRefs::load(&repository).ok()?;
        let status = WorkingTreeStatus::load(&repository).ok()?;
        let submodules = if root.join(".gitmodules").exists() {
            git::submodule::gitlink_paths(&repository).into_iter().collect()
        } else {
            HashSet::new()
        };
        let prefix = match project.and_then(|p| root.strip_prefix(p).ok()) {
            Some(rel) => rel.to_string_lossy().replace('\\', "/"),
            None => root.to_string_lossy().replace('\\', "/"),
        };
        Some(Self {
            path: root,
            prefix,
            web: git::hosting::web_repo(&repository),
            user_email: repository.current_user().1,
            state: repository.state(),
            refs: Arc::new(refs),
            status: Arc::new(status),
            submodules,
            repository,
        })
    }

    /// Every root at once, each on its own thread; roots git can't read are left out.
    fn load_all(paths: &[PathBuf], project: Option<&Path>, console: &GitConsole) -> Vec<Self> {
        std::thread::scope(|scope| {
            let jobs: Vec<_> = paths.iter().map(|path| scope.spawn(move || Self::load(path, project, console))).collect();
            jobs.into_iter().filter_map(|job| job.join().ok().flatten()).collect()
        })
    }
}

/// The root a project-relative path belongs to: the deepest root whose
/// folder holds it, and the path inside that root.
fn route_path<'a>(states: &[RootState], path: &'a str) -> Option<(usize, &'a str)> {
    states
        .iter()
        .enumerate()
        .filter_map(|(ix, state)| {
            if state.prefix.is_empty() {
                return Some((ix, 0, path));
            }
            let rest = path.strip_prefix(state.prefix.as_str())?;
            let rest = if rest.is_empty() { rest } else { rest.strip_prefix('/')? };
            Some((ix, state.prefix.len(), rest))
        })
        .max_by_key(|(_, depth, _)| *depth)
        .map(|(ix, _, rest)| (ix, rest))
}

/// One repository the Log reads, with the filter adjusted to it.
#[derive(Clone)]
struct LogSource {
    root: u16,
    repository: Repository,
    filter: LogFilter,
}

/// What the Log reads for `filter`: the active repository alone with a
/// single root; otherwise every root the Paths filter shows, each with the
/// filter's paths that lie in it and the filter's branches it has.
fn log_sources(states: &[RootState], active: &Repository, filter: &LogFilter) -> Vec<LogSource> {
    if states.len() < 2 {
        let root = states.iter().position(|s| s.path == active.root()).unwrap_or(0) as u16;
        return vec![LogSource { root, repository: active.clone(), filter: filter.clone() }];
    }
    let mut sources = Vec::new();
    for (ix, state) in states.iter().enumerate() {
        if !filter.roots.is_empty() && !filter.roots.contains(&state.path) {
            continue;
        }
        let mut own = filter.clone();
        own.roots.clear();
        if !filter.paths.is_empty() {
            own.paths = filter
                .paths
                .iter()
                .filter_map(|p| route_path(states, p).filter(|(root, _)| *root == ix).map(|(_, rest)| rest.to_owned()))
                .collect();
            if own.paths.is_empty() {
                continue;
            }
            // A path that is the root itself means all of it.
            if own.paths.iter().any(String::is_empty) {
                own.paths.clear();
                own.lines = None;
            }
        }
        if !filter.branches.is_empty() {
            let active_root = state.path == active.root();
            own.branches = filter
                .branches
                .iter()
                .filter(|b| {
                    *b == "HEAD"
                        || if b.contains("..") { active_root } else { state.repository.run(["rev-parse", "--verify", "-q", b.as_str()]).is_ok() }
                })
                .cloned()
                .collect();
            if own.branches.is_empty() {
                continue;
            }
        }
        sources.push(LogSource { root: ix as u16, repository: state.repository.clone(), filter: own });
    }
    sources
}

/// A failed open the user can fix from the Welcome screen.
#[derive(Clone, Debug)]
pub enum OpenProblem {
    GitMissing,
    /// The repository root git refused (`safe.directory`).
    Unsafe(PathBuf),
}

/// The log's inputs: every ref tip (the log lists `--all`), HEAD, the
/// repositories and the filter.
#[derive(Clone, Debug, PartialEq)]
struct LogKey {
    roots: Vec<(u16, PathBuf, String)>,
    filter: LogFilter,
}

impl LogKey {
    fn load(sources: &[LogSource], filter: &LogFilter) -> Self {
        let roots = sources
            .iter()
            .map(|source| {
                let repository = &source.repository;
                let tips = repository.run(["for-each-ref", "--format=%(objectname) %(refname)"]).unwrap_or_default();
                let head = repository.run(["rev-parse", "-q", "--verify", "HEAD"]).unwrap_or_default();
                // A checkout of a branch only moves HEAD among commits the log
                // already lists: unless the log follows HEAD itself, that keeps it
                // (rather than loading a large history again).
                let follows_head = source.filter.branches.iter().any(|b| b == "HEAD");
                let listed = !head.trim().is_empty() && tips.contains(head.trim());
                let tips = if follows_head || !listed { tips + &head } else { tips };
                (source.root, repository.root().to_path_buf(), tips)
            })
            .collect();
        Self { roots, filter: filter.clone() }
    }
}

struct LoadedLog {
    commits: Vec<Commit>,
    graph: GraphLayout,
    rows: git::log::CommitRows,
    /// Every repository's whole log is in (not just its first page).
    complete: bool,
}

impl LoadedLog {
    fn load(sources: &[LogSource], first_page: bool) -> Result<Self> {
        fn one(repository: &Repository, filter: &LogFilter, first_page: bool) -> Result<(Vec<Commit>, bool)> {
            let commits = if first_page { git::log::load_first_page(repository, filter)? } else { Vec::new() };
            // The first page leaves out tags; a short log is loaded whole right away.
            if commits.len() < git::log::FIRST_PAGE {
                return Ok((git::log::load_log(repository, filter, None)?, true));
            }
            Ok((commits, false))
        }
        let results: Vec<Result<(Vec<Commit>, bool)>> = std::thread::scope(|scope| {
            let jobs: Vec<_> =
                sources.iter().map(|source| scope.spawn(move || one(&source.repository, &source.filter, first_page))).collect();
            jobs.into_iter().map(|job| job.join().unwrap_or_else(|_| Err(anyhow::anyhow!("git log failed")))).collect()
        });
        let mut complete = true;
        let mut logs = Vec::new();
        for (source, result) in sources.iter().zip(results) {
            match result {
                Ok((commits, done)) => {
                    complete &= done;
                    logs.push((source.root, commits));
                }
                // One root failing (a filter it can't take) leaves the others.
                Err(error) if sources.len() == 1 => return Err(error),
                Err(_) => {}
            }
        }
        let commits = git::log::merge_logs(logs);
        let graph = GraphLayout::build(&commits);
        let rows = git::log::CommitRows::build(&commits);
        Ok(Self { commits, graph, rows, complete })
    }
}

struct Snapshot {
    roots: Vec<RootInfo>,
    root_states: Vec<RootState>,
    web: Option<git::hosting::WebRepo>,
    refs: RepositoryRefs,
    /// The first page of the log when it changed, and its key.
    log: Option<Result<(LoadedLog, LogKey)>>,
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
            root_states: Arc::default(),
            project_status: Arc::default(),
            log_refs: Arc::default(),
            web_repo: None,
            loading: false,
            busy: None,
            conflicts_pending: false,
            error: None,
            open_problem: None,
            failed_path: None,
            _reload_task: None,
            _log_task: None,
            _stale_details_task: None,
            _read_ahead_task: None,
            details_cache: HashMap::new(),
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
                self.root_states = Arc::default();
                self.user_email = repository.current_user().1;
                self.repository = Some(repository);
                self.error = None;
                self.selected = None;
                self.details = None;
                self.forget_repository_filters();
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

    /// Branches and paths name things of the previous repository: a log
    /// filtered by them would fail (or be empty) in another.
    fn forget_repository_filters(&mut self) {
        self.filter.branches.clear();
        self.filter.paths.clear();
        self.filter.lines = None;
        self.filter.roots.clear();
        self.log_key = None;
    }

    /// Makes another root of the project the active one: the Log, Commit
    /// window and branch widget then show that repository.
    pub fn switch_root(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.repository.as_ref().is_some_and(|r| r.root() == path) {
            return;
        }
        // A multi-root project has every root loaded: the Log and the Commit
        // window already list them all and stay as they are.
        if let Some(state) = self.root_states.iter().find(|s| s.path == path).cloned() {
            self.focus_root(state, cx);
            cx.emit(RepoEvent::Reloaded);
            return;
        }
        match Repository::discover(&path, self.console.clone()) {
            Ok(repository) => {
                self.user_email = repository.current_user().1;
                self.repository = Some(repository);
                self.selected = None;
                self.details = None;
                self.forget_repository_filters();
                self.reload(cx);
            }
            Err(error) => cx.emit(RepoEvent::Notify { title: "Switch Repository".into(), message: error.to_string(), error: true }),
        }
    }

    /// Makes a loaded root the active one, without reloading anything: the
    /// branch widget, the operations and the selected commit's details then
    /// work on it.
    fn focus_root(&mut self, state: RootState, cx: &mut Context<Self>) {
        self.repository = Some(state.repository);
        self.refs = state.refs;
        self.status = (*state.status).clone();
        self.state = state.state;
        self.web_repo = state.web;
        self.user_email = state.user_email;
        self.submodule_paths = state.submodules;
        self.details_cache.clear();
        cx.notify();
    }

    /// Every root of a multi-root project with its changes and refs; empty
    /// with a single root.
    pub fn root_states(&self) -> &Arc<Vec<RootState>> {
        &self.root_states
    }

    /// The changes the Commit tool window lists: every root's in a
    /// multi-root project (paths relative to the project), else the
    /// repository's own.
    pub fn project_status(&self) -> &WorkingTreeStatus {
        if self.root_states.len() > 1 { &self.project_status } else { &self.status }
    }

    /// The repository and its own path for a path of `project_status` (in
    /// a multi-root project, the root that holds it).
    pub fn route(&self, path: &str) -> Option<(Repository, String)> {
        if self.root_states.len() > 1 {
            let (ix, rest) = route_path(&self.root_states, path)?;
            return Some((self.root_states[ix].repository.clone(), rest.to_owned()));
        }
        Some((self.repository.clone()?, path.to_owned()))
    }

    /// Paths of `project_status` grouped by the root that holds them, each
    /// with its root's own paths, in root order.
    pub fn route_all(&self, paths: &[String]) -> Vec<(Repository, Vec<String>)> {
        let mut groups: Vec<(Repository, Vec<String>)> = Vec::new();
        for path in paths {
            let Some((repository, own)) = self.route(path) else { continue };
            match groups.iter_mut().find(|(r, _)| r.root() == repository.root()) {
                Some((_, list)) => list.push(own),
                None => groups.push((repository, vec![own])),
            }
        }
        groups
    }

    /// The prefix a root's own paths get in `project_status` ("" for the project's repository).
    pub fn prefix_of(&self, root: &Path) -> String {
        self.root_states.iter().find(|s| s.path == root).map(|s| s.prefix.clone()).unwrap_or_default()
    }

    /// The project's own repository (files of the Project tool window and
    /// the editor are relative to it), whichever root is active.
    pub fn project_repository(&self) -> Option<&Repository> {
        self.root_states.iter().find(|s| s.prefix.is_empty()).map(|s| &s.repository).or(self.repository.as_ref())
    }

    /// Refs of every root the Log shows (the active repository's alone with one root).
    pub fn log_refs(&self) -> &Arc<RepositoryRefs> {
        if self.root_states.len() > 1 { &self.log_refs } else { &self.refs }
    }

    /// Runs `operation` in each root holding some of `paths` (project
    /// paths), with that root's own paths, then reloads and reports the
    /// result: one commit per repository, as IntelliJ commits a
    /// multi-root change set.
    pub fn run_in_roots(
        &mut self,
        title: impl Into<String>,
        paths: Vec<String>,
        operation: impl Fn(&Repository, Vec<String>) -> Result<String> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let groups = self.route_all(&paths);
        if groups.len() <= 1 && self.root_states.len() <= 1 {
            let own = groups.into_iter().next().map(|(_, own)| own).unwrap_or(paths);
            return self.run_operation(title, move |repo| operation(repo, own), cx);
        }
        let project = self.project_root.clone();
        let title = title.into();
        self.busy = Some(title.clone());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let mut messages = Vec::new();
                    for (repository, own) in groups {
                        let name = project.as_deref().map(|p| git::roots::label(p, repository.root())).unwrap_or_default();
                        let message = operation(&repository, own).map_err(|e| anyhow::anyhow!("{name}: {e}"))?;
                        if !message.is_empty() {
                            messages.push(if messages.is_empty() && name.is_empty() { message } else { format!("{name}: {message}") });
                        }
                    }
                    anyhow::Ok(messages.join("\n"))
                })
                .await;
            this.update(cx, |this, cx| {
                this.busy = None;
                cx.emit(match result {
                    Ok(message) => RepoEvent::Notify { title, message, error: false },
                    Err(error) => RepoEvent::Notify { title: format!("{title} failed"), message: error.to_string(), error: true },
                });
                this.reload(cx);
            })
            .ok();
        })
        .detach();
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
            self.reload_log(cx);
        }
    }

    /// The filter the log is loaded with (its order comes from the settings).
    fn effective_filter(&self, cx: &gpui_kit::App) -> LogFilter {
        let mut filter = self.filter.clone();
        filter.date_order = crate::settings::Settings::get(cx).log.sort_by_date;
        filter
    }

    /// Loads the log alone, for a new filter: refs and the working tree
    /// haven't changed, and `git status` is the slow part of a reload.
    fn reload_log(&mut self, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.clone() else { return };
        let filter = self.effective_filter(cx);
        let states = self.root_states.clone();
        self.loading = true;
        cx.notify();
        self._log_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn({
                    let (repository, filter) = (repository.clone(), filter.clone());
                    async move {
                        let sources = log_sources(&states, &repository, &filter);
                        let key = LogKey::load(&sources, &filter);
                        let l = LoadedLog::load(&sources, true)?;
                        anyhow::Ok((l, key, sources))
                    }
                })
                .await;
            let full = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok((log, key, sources)) => {
                        let complete = log.complete;
                        this.set_log(log);
                        this.log_key = Some(key);
                        this.log_changed(cx);
                        (!complete).then(|| this.load_full_log(sources, cx))
                    }
                    Err(error) => {
                        this.error = Some(error.to_string());
                        this.log_key = None;
                        cx.notify();
                        None
                    }
                }
            });
            // Keeps loading the rest under this same task, so a newer
            // filter cancels it.
            if let Ok(Some(full)) = full {
                full.await;
            }
        }));
    }

    /// After a new log: keep the selection if the commit is still listed,
    /// else select HEAD as the Log does on first open.
    fn keep_selection(&mut self, cx: &mut Context<Self>) {
        let keep = self.selected.as_deref().is_some_and(|h| self.row_of(h).is_some());
        if !keep {
            let head = self.refs.head_commit.clone();
            let target = head.filter(|h| self.row_of(h).is_some()).or_else(|| self.commits.first().map(|c| c.hash.clone()));
            self.select_hash(target, cx);
        } else if let Some(hash) = self.selected.clone() {
            self.load_details(hash, cx);
        }
    }

    fn log_changed(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        self.keep_selection(cx);
        cx.emit(RepoEvent::LogLoaded);
        cx.notify();
    }

    /// Reloads refs, the log, and working tree status in the background
    /// (of every root, in a multi-root project).
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.clone() else { return };
        let filter = self.effective_filter(cx);
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
                    let paths = if known_roots.is_empty() {
                        project_root
                            .as_ref()
                            .and_then(|p| Repository::discover(p, console.clone()).ok())
                            .map(|project| git::roots::detect(&project))
                            .unwrap_or_else(|| vec![repository.root().to_path_buf()])
                    } else {
                        known_roots
                    };
                    // Several roots: all of them load, the active one among them.
                    let root_states = if paths.len() > 1 { RootState::load_all(&paths, project_root.as_deref(), &console) } else { Vec::new() };
                    let active = root_states.iter().find(|s| s.path == repository.root()).cloned();
                    let (refs, status, submodules, web, state) = match &active {
                        Some(a) => ((*a.refs).clone(), (*a.status).clone(), a.submodules.clone(), a.web.clone(), a.state),
                        None => {
                            let refs = RepositoryRefs::load(&repository)?;
                            let status = WorkingTreeStatus::load(&repository)?;
                            let submodules = if repository.root().join(".gitmodules").exists() {
                                git::submodule::gitlink_paths(&repository).into_iter().collect()
                            } else {
                                HashSet::new()
                            };
                            (refs, status, submodules, git::hosting::web_repo(&repository), repository.state())
                        }
                    };
                    // The log only changes with the refs (or the filter); a
                    // reload after staging or editing keeps it.
                    let sources = log_sources(&root_states, &repository, &filter);
                    let key = LogKey::load(&sources, &filter);
                    let log = if previous_key.as_ref() != Some(&key) {
                        // The first page shows quickly; the rest follows below.
                        // (A failing log, say for a branch filter that no longer
                        // matches, still lets the refs and the status refresh.)
                        Some(LoadedLog::load(&sources, true).map(|log| (log, key)))
                    } else {
                        None
                    };
                    let roots = if root_states.is_empty() {
                        paths
                            .into_iter()
                            .map(|path| {
                                let branch = git::run_in(&git::executable(), &path, &GitConsole::default(), ["symbolic-ref", "--short", "-q", "HEAD"], None, &[])
                                    .ok()
                                    .map(|b| b.trim().to_owned())
                                    .filter(|b| !b.is_empty());
                                RootInfo { path, branch }
                            })
                            .collect()
                    } else {
                        root_states.iter().map(|s| RootInfo { path: s.path.clone(), branch: s.refs.current_branch.clone() }).collect()
                    };
                    anyhow::Ok((Snapshot { roots, root_states, web, refs, log, status, state, submodules }, repository, filter, sources))
                })
                .await;
            this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok((snapshot, repository, filter, sources)) => {
                        let mut log_error = None;
                        this.details_cache.clear();
                        this.refs = Arc::new(snapshot.refs);
                        // (Unless the filter changed meanwhile: its own load wins.)
                        let log = match snapshot.log.filter(|_| this.effective_filter(cx) == filter) {
                            Some(Err(error)) => {
                                this.log_key = None;
                                log_error = Some(error.to_string());
                                None
                            }
                            other => other.map(|log| log.ok()).flatten(),
                        };
                        if let Some((log, key)) = log {
                            let complete = log.complete;
                            // (A short log came whole; a long one was the first page.)
                            this.set_log(log);
                            this.log_key = Some(key);
                            // A new log replaces one still loading.
                            this._log_task = (!complete).then(|| this.load_full_log(sources, cx));
                        }
                        let _ = repository;
                        this.status = snapshot.status;
                        this.state = snapshot.state;
                        this.submodule_paths = snapshot.submodules;
                        this.roots = snapshot.roots;
                        this.set_root_states(snapshot.root_states);
                        this.web_repo = snapshot.web;
                        this.error = log_error;
                        this.keep_selection(cx);
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

    /// Keeps every root's state and what is built from it: the Commit
    /// window's combined changes and the Log's combined refs.
    fn set_root_states(&mut self, states: Vec<RootState>) {
        if states.len() > 1 {
            self.project_status = Arc::new(WorkingTreeStatus::combined(states.iter().map(|s| (s.prefix.as_str(), &*s.status))));
            let active = self.repository.as_ref().map(|r| r.root().to_path_buf());
            let others: Vec<&RepositoryRefs> = states.iter().filter(|s| Some(&s.path) != active.as_ref()).map(|s| &*s.refs).collect();
            self.log_refs = Arc::new(RepositoryRefs::merged(&self.refs, &others));
        } else {
            self.project_status = Arc::default();
            self.log_refs = Arc::default();
        }
        self.root_states = Arc::new(states);
    }

    /// Loads the whole log after its first page, then refreshes git's
    /// commit-graph so the next first page comes sorted at once.
    fn load_full_log(&mut self, sources: Vec<LogSource>, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            let full = cx.background_spawn({
                let sources = sources.clone();
                async move { LoadedLog::load(&sources, false) }
            });
            let full = full.await;
            this.update(cx, |this, cx| match full {
                Ok(log) => {
                    this.set_log(log);
                    cx.emit(RepoEvent::LogLoaded);
                    cx.notify();
                }
                // Load the log again on the next reload.
                Err(_) => this.log_key = None,
            })
            .ok();
            cx.background_spawn(async move {
                for source in &sources {
                    git::log::write_commit_graph(&source.repository);
                }
            })
            .await;
        })
    }

    fn set_log(&mut self, log: LoadedLog) {
        let old = (
            std::mem::replace(&mut self.all_commits, Arc::new(log.commits)),
            std::mem::replace(&mut self.all_graph, Arc::new(log.graph)),
            std::mem::replace(&mut self.all_rows, Arc::new(log.rows)),
            self.commits.clone(),
            self.graph.clone(),
            self.rows.clone(),
        );
        self.apply_collapse();
        // Freeing a big log (a million commits) takes most of a second:
        // not on the main thread. (Views still holding a copy free theirs
        // when they next draw, and only drop a reference then.)
        std::thread::spawn(move || drop(old));
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
        cx.emit(RepoEvent::LogLoaded);
        cx.notify();
    }

    pub fn expand_run(&mut self, hash: String, cx: &mut Context<Self>) {
        self.expanded_runs.insert(hash);
        self.apply_collapse();
        cx.emit(RepoEvent::LogLoaded);
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
        // In a multi-root project the commit's own repository becomes the
        // active one, so its details and the Log's actions work on it.
        let root = hash.as_deref().and_then(|h| self.row_of(h)).and_then(|row| self.commits.get(row)).map(|c| c.root as usize);
        if let Some(state) = root.filter(|_| self.root_states.len() > 1).and_then(|ix| self.root_states.get(ix)) {
            if self.repository.as_ref().is_none_or(|r| r.root() != state.path) {
                self.focus_root(state.clone(), cx);
            }
        }
        // The previous commit's details stay up while the next ones load
        // (usually a few ms), so the details pane doesn't flash empty; a
        // slow load clears them after a moment.
        if hash.is_none() {
            self.details = None;
        }
        cx.emit(RepoEvent::SelectionChanged);
        cx.notify();
        if let Some(hash) = hash {
            self.load_details(hash.clone(), cx);
            self._stale_details_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(std::time::Duration::from_millis(400)).await;
                this.update(cx, |this, cx| {
                    if this.details.as_ref().is_some_and(|d| d.hash != hash) {
                        this.details = None;
                        cx.emit(RepoEvent::DetailsLoaded);
                        cx.notify();
                    }
                })
                .ok();
            }));
        }
    }

    fn load_details(&mut self, hash: String, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.clone() else { return };
        // Seen (or read ahead) before: shown at once.
        if let Some(details) = self.details_cache.get(&hash).cloned() {
            self.details = Some(details);
            cx.emit(RepoEvent::DetailsLoaded);
            cx.notify();
            self.read_ahead(&hash, cx);
            return;
        }
        self._details_task = Some(cx.spawn(async move |this, cx| {
            // The header and files first, then the branches and signature.
            let basic = cx
                .background_spawn({
                    let (repository, hash) = (repository.clone(), hash.clone());
                    async move { git::log::load_details_basic(&repository, &hash) }
                })
                .await;
            let Ok(basic) = basic else { return };
            let full_hash = basic.hash.clone();
            let shown = this.update(cx, |this, cx| {
                if this.selected.as_deref() != Some(full_hash.as_str()) {
                    return false;
                }
                // (A reload of the shown commit keeps its branches meanwhile.)
                if !this.details.as_ref().is_some_and(|d| d.hash == full_hash) {
                    this.details = Some(basic.clone());
                    cx.emit(RepoEvent::DetailsLoaded);
                    cx.notify();
                }
                true
            });
            if !matches!(shown, Ok(true)) {
                return;
            }
            let (branches, signature) = cx
                .background_spawn({
                    let (repository, hash) = (repository.clone(), full_hash.clone());
                    async move { git::log::load_details_extra(&repository, &hash) }
                })
                .await;
            let mut details = basic;
            (details.containing_branches, details.signature, details.complete) = (branches, signature, true);
            this.update(cx, |this, cx| {
                this.remember_details(details.clone());
                if this.selected.as_deref() == Some(details.hash.as_str()) {
                    let hash = details.hash.clone();
                    this.details = Some(details);
                    cx.emit(RepoEvent::DetailsLoaded);
                    cx.notify();
                    this.read_ahead(&hash, cx);
                }
            })
            .ok();
        }));
    }

    fn remember_details(&mut self, details: git::log::CommitDetails) {
        if self.details_cache.len() >= 256 {
            self.details_cache.clear();
        }
        self.details_cache.insert(details.hash.clone(), details);
    }

    /// Loads the commits above and below the selected one in the
    /// background, so moving through the Log with the arrow keys shows
    /// their details at once.
    fn read_ahead(&mut self, hash: &str, cx: &mut Context<Self>) {
        let (Some(repository), Some(row)) = (self.repository.clone(), self.row_of(hash)) else { return };
        let next: Vec<String> = [row.checked_sub(1), Some(row + 1)]
            .into_iter()
            .flatten()
            .filter_map(|r| self.commits.get(r))
            .map(|c| c.hash.to_string())
            .filter(|h| !self.details_cache.contains_key(h))
            .collect();
        if next.is_empty() {
            return;
        }
        self._read_ahead_task = Some(cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_spawn(async move {
                    std::thread::scope(|scope| {
                        let jobs: Vec<_> = next.iter().map(|h| scope.spawn(|| git::log::load_details(&repository, h))).collect();
                        jobs.into_iter().filter_map(|j| j.join().ok()?.ok()).collect::<Vec<_>>()
                    })
                })
                .await;
            this.update(cx, |this, _| {
                for details in loaded {
                    this.remember_details(details);
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
