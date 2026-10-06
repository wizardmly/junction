//! Repository state shared by every view, loaded off the UI thread.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use gpui_kit::{AppContext as _, Context, EventEmitter, Task};

use crate::git::{
    self, Commit, CommitDetails, GitConsole, GraphLayout, LogFilter, Repository, RepositoryRefs,
    RepositoryState, WorkingTreeStatus,
};

#[derive(Clone, Debug)]
pub enum RepoEvent {
    /// Refs, log, or status changed.
    Reloaded,
    SelectionChanged,
    DetailsLoaded,
    /// An operation finished; shown as a balloon notification.
    Notify { title: String, message: String, error: bool },
}

pub struct RepoModel {
    repository: Option<Repository>,
    console: GitConsole,
    refs: RepositoryRefs,
    commits: Arc<Vec<Commit>>,
    graph: Arc<GraphLayout>,
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
        };
        if let Some(path) = path {
            this.open(path, cx);
        }
        this
    }

    pub fn open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        match Repository::discover(&path, self.console.clone()) {
            Ok(repository) => {
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
                    let commits = git::log::load_log(&repository, &filter)?;
                    let graph = GraphLayout::build(&commits);
                    let status = WorkingTreeStatus::load(&repository)?;
                    anyhow::Ok(Snapshot { refs, commits, graph, status, state: repository.state() })
                })
                .await;
            this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(snapshot) => {
                        this.refs = snapshot.refs;
                        this.commits = Arc::new(snapshot.commits);
                        this.graph = Arc::new(snapshot.graph);
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
        }));
    }

    pub fn select_index(&mut self, ix: usize, cx: &mut Context<Self>) {
        let hash = self.commits.get(ix).map(|c| c.hash.clone());
        self.select_hash(hash, cx);
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
