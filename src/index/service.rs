//! The project's code index as an entity: background (re)indexing with
//! progress, and the navigation queries the editor and tool windows run —
//! bridges first, then the language server, then the built-in index.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use gpui_kit::{App, AppContext as _, Context, EventEmitter, Task};

use super::lang::Lang;
use super::lsp::LspManager;
use super::nav::{self, Target, Usage};
use super::store::ProjectIndex;

pub enum IndexEvent {
    /// Indexing progressed or finished.
    Changed,
}

pub struct CodeIndex {
    root: Option<PathBuf>,
    pub index: Arc<RwLock<ProjectIndex>>,
    pub lsp: Arc<LspManager>,
    /// (done, total) while files are being indexed.
    pub progress: Option<(usize, usize)>,
    updating: bool,
    /// Another refresh was asked for while one ran.
    again: bool,
    _task: Option<Task<()>>,
    _poll: Option<Task<()>>,
}

impl EventEmitter<IndexEvent> for CodeIndex {}

impl CodeIndex {
    pub fn new(cx: &mut Context<Self>) -> Self {
        // Language server progress shows in the status bar; poll it while
        // any server runs.
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this.update(cx, |this, cx| if !this.lsp.status().is_empty() { cx.emit(IndexEvent::Changed) }).is_err() {
                    break;
                }
            }
        });
        Self {
            root: None,
            index: Arc::default(),
            lsp: Arc::default(),
            progress: None,
            updating: false,
            again: false,
            _task: None,
            _poll: Some(poll),
        }
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// Indexes another project (or none).
    pub fn set_root(&mut self, root: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.root == root {
            return;
        }
        self.lsp.shutdown();
        self.root = root.clone();
        self.index = Arc::default();
        self.lsp = Arc::new(LspManager::new(root.clone().unwrap_or_default()));
        self.progress = None;
        self.updating = false;
        self._task = None;
        cx.emit(IndexEvent::Changed);
        if root.is_some() {
            self.refresh(cx);
        }
    }

    /// Re-indexes what changed since the last pass.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else { return };
        if self.updating {
            self.again = true;
            return;
        }
        self.updating = true;
        let index = self.index.clone();
        let (tx, rx) = std::sync::mpsc::channel::<(usize, usize)>();
        self._task = Some(cx.spawn(async move |this, cx| {
            let work = cx.background_spawn(async move {
                // The first pass loads the cache; work on a copy so queries
                // keep answering from the old index meanwhile.
                let mut fresh = {
                    let current = index.read().ok();
                    match current {
                        Some(current) if !current.files.is_empty() || !current.all_files.is_empty() => None,
                        _ => Some(ProjectIndex::load(&root)),
                    }
                };
                let changed = match fresh.as_mut() {
                    Some(fresh) => {
                        fresh.update(&|done, total| {
                            tx.send((done, total)).ok();
                        });
                        true
                    }
                    None => {
                        // Incremental: stat everything, re-parse the changed files.
                        let mut copy = index.read().unwrap().clone();
                        let changed = copy.update(&|done, total| {
                            tx.send((done, total)).ok();
                        });
                        *index.write().unwrap() = copy;
                        changed
                    }
                };
                if let Some(fresh) = fresh {
                    *index.write().unwrap() = fresh;
                }
                if changed {
                    if let Ok(index) = index.read() {
                        index.save();
                    }
                }
            });
            // Forward progress until the work finishes.
            let mut work = std::pin::pin!(work);
            loop {
                let tick = cx.background_executor().timer(Duration::from_millis(150));
                let finished = futures_lite_poll(&mut work, tick).await;
                let latest = rx.try_iter().last();
                let done = this
                    .update(cx, |this, cx| {
                        if let Some(p) = latest {
                            this.progress = Some(p);
                        }
                        if finished {
                            this.progress = None;
                            this.updating = false;
                        }
                        cx.emit(IndexEvent::Changed);
                    })
                    .is_err();
                if finished || done {
                    break;
                }
            }
            this.update(cx, |this, cx| {
                if std::mem::take(&mut this.again) {
                    this.refresh(cx);
                }
            })
            .ok();
        }));
        cx.emit(IndexEvent::Changed);
    }

    /// Re-indexes one file right away (after saving it).
    pub fn refresh_file(&mut self, rel: &str, cx: &mut Context<Self>) {
        if self.root.is_none() {
            return;
        }
        let (index, rel) = (self.index.clone(), rel.to_owned());
        cx.spawn(async move |this, cx| {
            cx.background_spawn(async move {
                if let Ok(mut index) = index.write() {
                    index.refresh_file(&rel);
                }
            })
            .await;
            this.update(cx, |_, cx| cx.emit(IndexEvent::Changed)).ok();
        })
        .detach();
    }

    pub fn lang_of(path: &str, text: &str) -> Option<Lang> {
        let lang = Lang::from_path(path)?;
        Some(if path.ends_with(".h") { Lang::for_header(text) } else { lang })
    }

    /// Go to Declaration: bridge sites, then the language server, then the
    /// built-in index. Paths in the result are relative to the root when
    /// inside it, absolute otherwise.
    pub fn definitions(&self, path: String, text: String, offset: usize, cx: &App) -> Task<Vec<Target>> {
        let (Some(root), Some(lang)) = (self.root.clone(), Self::lang_of(&path, &text)) else { return Task::ready(Vec::new()) };
        let (index, lsp) = (self.index.clone(), self.lsp.clone());
        cx.background_spawn(async move {
            let local = {
                let index = index.read().unwrap();
                let bridged = nav::bridge_definitions(&index, &path, &text, offset);
                if !bridged.is_empty() {
                    return bridged;
                }
                nav::definitions(&index, &path, lang, &text, offset)
            };
            // The server is exact but may still be loading the project; when
            // the index already has an answer, don't wait long for it.
            let patience = if local.is_empty() { Duration::from_secs(10) } else { Duration::from_millis(600) };
            let file = root.join(&path);
            if let Some(client) = lsp.client(lang, &file) {
                let (line, col) = nav::position(&text, offset);
                if let Ok(locations) = client.definition(&file, lang, &text, line, col, patience) {
                    if !locations.is_empty() {
                        return locations.into_iter().map(|l| server_target(&root, l, lang)).collect();
                    }
                }
            }
            local
        })
    }

    /// Starts the language server for a file as soon as it opens, so the
    /// first navigation doesn't wait for the server to load the project.
    pub fn warm_up(&self, path: &str, text: String, cx: &App) {
        let (Some(root), Some(lang)) = (self.root.clone(), Self::lang_of(path, &text)) else { return };
        let (lsp, file) = (self.lsp.clone(), root.join(path));
        cx.background_spawn(async move {
            if let Some(client) = lsp.client(lang, &file) {
                client.sync(&file, lang, &text).ok();
            }
        })
        .detach();
    }

    /// Quick Documentation: the server's hover, else the declaration line.
    pub fn hover(&self, path: String, text: String, offset: usize, cx: &App) -> Task<Option<String>> {
        let (Some(root), Some(lang)) = (self.root.clone(), Self::lang_of(&path, &text)) else { return Task::ready(None) };
        let (index, lsp) = (self.index.clone(), self.lsp.clone());
        cx.background_spawn(async move {
            nav::word_at(&text, offset)?;
            if let Some(client) = lsp.client(lang, &root.join(&path)) {
                let (line, col) = nav::position(&text, offset);
                if let Ok(Some(markdown)) = client.hover(&root.join(&path), lang, &text, line, col, Duration::from_secs(2)) {
                    return Some(markdown);
                }
            }
            let targets = {
                let index = index.read().unwrap();
                nav::definitions(&index, &path, lang, &text, offset)
            };
            let target = targets.first()?;
            let source = std::fs::read_to_string(root.join(&target.path)).ok()?;
            let line = source.lines().nth(target.line as usize)?.trim();
            let mut out = format!("```{}\n{line}\n```\n", lang.language_id());
            let location = match &target.container {
                Some(c) => format!("{} · {c} · {}", target.label, target.path),
                None => format!("{} · {}", target.label, target.path),
            };
            out.push_str(&location);
            if targets.len() > 1 {
                out.push_str(&format!(" (+{} more)", targets.len() - 1));
            }
            Some(out)
        })
    }

    /// Find Usages of the identifier at `offset`.
    pub fn usages(&self, path: String, text: String, offset: usize, cx: &App) -> Task<(String, Vec<Usage>)> {
        let Some(root) = self.root.clone() else { return Task::ready((String::new(), Vec::new())) };
        let lang = Self::lang_of(&path, &text);
        let (index, lsp) = (self.index.clone(), self.lsp.clone());
        cx.background_spawn(async move {
            let Some((word, _)) = nav::word_at(&text, offset) else { return (String::new(), Vec::new()) };
            let mut found = {
                let index = index.read().unwrap();
                nav::usages(&index, &root, &word, lang)
            };
            // The server's references are exact; mark them and add any the
            // text search missed.
            if let Some(lang) = lang {
                if let Some(client) = lsp.client(lang, &root.join(&path)) {
                    let (line, col) = nav::position(&text, offset);
                    if let Ok(refs) = client.references(&root.join(&path), lang, &text, line, col) {
                        for r in refs {
                            let rel = r.path.strip_prefix(&root).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_else(|_| r.path.to_string_lossy().into_owned());
                            if let Some(u) = found.iter_mut().find(|u| u.path == rel && u.line == r.line) {
                                if u.group != "Declarations" {
                                    u.group = format!("{} (verified)", lang.name());
                                }
                                continue;
                            }
                            let text = std::fs::read_to_string(&r.path).ok().and_then(|t| t.lines().nth(r.line as usize).map(|l| l.trim().to_owned())).unwrap_or_default();
                            found.push(Usage { path: rel, line: r.line, col: r.col, text, group: format!("{} (verified)", lang.name()) });
                        }
                    }
                }
            }
            (word, found)
        })
    }

    pub fn search_symbols(&self, query: String, types_only: bool, cx: &App) -> Task<Vec<nav::SymbolMatch>> {
        let index = self.index.clone();
        cx.background_spawn(async move { nav::search_symbols(&index.read().unwrap(), &query, types_only, 200) })
    }

    pub fn search_files(&self, query: String, cx: &App) -> Task<Vec<(String, i32)>> {
        let index = self.index.clone();
        cx.background_spawn(async move { nav::search_files(&index.read().unwrap(), &query, 200) })
    }

    /// For the status bar.
    pub fn summary(&self) -> String {
        if let Some((done, total)) = self.progress {
            return format!("Indexing… {done}/{total}");
        }
        if self.updating {
            return "Indexing…".into();
        }
        let Ok(index) = self.index.read() else { return String::new() };
        if index.files.is_empty() {
            return String::new();
        }
        let mut out = format!("{} files, {} symbols indexed", index.files.len(), index.symbol_count());
        let servers = self.lsp.status();
        if !servers.is_empty() {
            let list: Vec<String> = servers.iter().map(|(name, state)| format!("{name}: {state}")).collect();
            out.push_str(&format!(" · {}", list.join(", ")));
        }
        out
    }
}

fn server_target(root: &Path, l: super::lsp::Location, lang: Lang) -> Target {
    let path = l.path.strip_prefix(root).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_else(|_| l.path.to_string_lossy().into_owned());
    Target { path, line: l.line, col: l.col, name: String::new(), label: format!("{} server", lang.name()), container: None }
}

/// Polls `work` until it finishes or `tick` fires; true when finished.
async fn futures_lite_poll<F: Future<Output = ()>>(work: &mut std::pin::Pin<&mut F>, tick: impl std::future::Future) -> bool {
    use std::task::Poll;
    let mut tick = std::pin::pin!(tick);
    std::future::poll_fn(|cx| {
        if work.as_mut().poll(cx).is_ready() {
            return Poll::Ready(true);
        }
        if tick.as_mut().poll(cx).is_ready() {
            return Poll::Ready(false);
        }
        Poll::Pending
    })
    .await
}

impl Drop for CodeIndex {
    fn drop(&mut self) {
        self.lsp.shutdown();
    }
}
