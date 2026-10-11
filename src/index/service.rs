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
use super::libraries::ExternalIndex;
use super::store::ProjectIndex;

pub enum IndexEvent {
    /// The index's content changed (files, symbols, libraries).
    Changed,
    /// Only the progress or the language servers' status did.
    Progress,
}

pub struct CodeIndex {
    root: Option<PathBuf>,
    pub index: Arc<RwLock<ProjectIndex>>,
    pub lsp: Arc<LspManager>,
    /// (done, total) while files are being indexed.
    pub progress: Option<(usize, usize)>,
    updating: bool,
    /// Set while the libraries are being indexed.
    library_phase: Arc<std::sync::atomic::AtomicBool>,
    /// Another refresh was asked for while one ran.
    again: bool,
    _task: Option<Task<()>>,
    _poll: Option<Task<()>>,
    /// "N files, M symbols indexed…", counted once per change rather than
    /// on every frame of the status bar.
    counts: std::cell::RefCell<Option<String>>,
}

impl EventEmitter<IndexEvent> for CodeIndex {}

/// The index's content changed: recount and tell the views.
macro_rules! this_changed {
    ($this:expr, $cx:expr) => {{
        $this.counts.replace(None);
        $cx.emit(IndexEvent::Changed);
    }};
}

impl CodeIndex {
    pub fn new(cx: &mut Context<Self>) -> Self {
        // Language server progress shows in the status bar; poll it while
        // any server runs.
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this.update(cx, |this, cx| if !this.lsp.status().is_empty() { cx.emit(IndexEvent::Progress) }).is_err() {
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
            library_phase: Arc::default(),
            again: false,
            _task: None,
            _poll: Some(poll),
            counts: Default::default(),
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
        this_changed!(self, cx);
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
        let phase = self.library_phase.clone();
        // Set when the work replaced part of the index, so views reload
        // the file list only then, not on every progress tick.
        let changed_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let content = changed_flag.clone();
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
                        // Incremental: stat everything and re-parse the changed
                        // files while queries keep using the index, then apply.
                        let changes = index.read().unwrap().scan(&|done, total| {
                            tx.send((done, total)).ok();
                        });
                        let changed = changes.is_some();
                        if let Some(changes) = changes {
                            index.write().unwrap().apply(changes);
                        }
                        changed
                    }
                };
                if let Some(fresh) = fresh {
                    *index.write().unwrap() = fresh;
                }
                if changed {
                    content.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                if changed {
                    if let Ok(index) = index.read() {
                        index.save();
                    }
                }
                // Then the libraries, so project navigation is ready first.
                let (files, previous) = {
                    let index = index.read().unwrap();
                    (index.all_files.clone(), index.external.clone())
                };
                let stamp = super::libraries::inputs_stamp(&root, &files);
                if !previous.scanned || previous.stamp != stamp {
                    phase.store(true, std::sync::atomic::Ordering::Relaxed);
                    tx.send((0, 0)).ok();
                    let libraries = super::libraries::discover(&root, &files);
                    let external = ExternalIndex::build(libraries, stamp, Some(&previous), &|done, total| {
                        tx.send((done, total)).ok();
                    });
                    index.write().unwrap().external = Arc::new(external);
                    content.store(true, std::sync::atomic::Ordering::Relaxed);
                    phase.store(false, std::sync::atomic::Ordering::Relaxed);
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
                        let changed = changed_flag.swap(false, std::sync::atomic::Ordering::Relaxed);
                        if changed || finished {
                            this_changed!(this, cx);
                        } else {
                            cx.emit(IndexEvent::Progress);
                        }
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
        this_changed!(self, cx);
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
            this.update(cx, |this, cx| this_changed!(this, cx)).ok();
        })
        .detach();
    }

    pub fn lang_of(path: &str, text: &str) -> Option<Lang> {
        let lang = if ProjectIndex::is_external(path) { super::libraries::library_lang(path)? } else { Lang::from_path(path)? };
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
            if let Some(client) = (!ProjectIndex::is_external(&path)).then(|| lsp.client(lang, &file)).flatten() {
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
            if let Some(client) = (!ProjectIndex::is_external(&path)).then(|| lsp.client(lang, &root.join(&path))).flatten() {
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
                if let Some(client) = (!ProjectIndex::is_external(&path)).then(|| lsp.client(lang, &root.join(&path))).flatten() {
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

    /// The editor gutter's implementation markers for a file's current text.
    pub fn gutter_marks(&self, path: String, text: String, cx: &App) -> Task<Vec<super::hierarchy::GutterMark>> {
        let Some(lang) = Self::lang_of(&path, &text) else { return Task::ready(Vec::new()) };
        let index = self.index.clone();
        cx.background_spawn(async move {
            let symbols = super::symbols::extract(lang, &text);
            super::hierarchy::gutter_marks(&index.read().unwrap(), &path, &symbols)
        })
    }

    pub fn search_symbols(&self, query: String, types_only: bool, libraries: bool, cx: &App) -> Task<Vec<nav::SymbolMatch>> {
        let index = self.index.clone();
        cx.background_spawn(async move { nav::search_symbols(&index.read().unwrap(), &query, types_only, libraries, 200) })
    }

    /// Whether the identifier at `offset` is a declaration the index knows.
    pub fn declared_at(&self, path: &str, text: &str, offset: usize) -> bool {
        let Some((word, range)) = nav::word_at(text, offset) else { return false };
        let (line, col) = nav::position(text, range.start);
        self.index.read().is_ok_and(|index| index.files.get(path).is_some_and(|e| e.symbols.iter().any(|s| s.line == line && s.col == col && s.name == word)))
    }

    /// On a declaration's name, where Go to Declaration shows its usages
    /// instead, as IntelliJ does. A C / C++ / Objective-C prototype still
    /// goes to its definition.
    pub fn on_declaration(&self, path: &str, text: &str, offset: usize) -> bool {
        let Some((word, range)) = nav::word_at(text, offset) else { return false };
        let (line, col) = nav::position(text, range.start);
        self.index.read().is_ok_and(|index| {
            index.files.get(path).is_some_and(|e| {
                let prototypes = matches!(e.lang, Lang::C | Lang::Cpp | Lang::ObjC);
                e.symbols.iter().any(|s| s.line == line && s.col == col && s.name == word && !(prototypes && s.decl))
            })
        })
    }

    pub fn file_symbols(&self, path: &str) -> Vec<nav::SymbolMatch> {
        self.index.read().map(|index| nav::file_symbols(&index, path)).unwrap_or_default()
    }

    pub fn search_files(&self, query: String, libraries: bool, cx: &App) -> Task<Vec<(String, i32)>> {
        let index = self.index.clone();
        cx.background_spawn(async move { nav::search_files(&index.read().unwrap(), &query, libraries, 200) })
    }

    /// Find in Files over `files` (all project files when `None`), in file order.
    pub fn find_text(
        &self,
        query: super::text_search::TextQuery,
        scope: super::text_search::FileScope,
        limit: usize,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
        cx: &App,
    ) -> Task<Result<super::text_search::SearchResult, String>> {
        let root = self.root.clone();
        cx.background_spawn(async move {
            let root = root.ok_or_else(String::new)?;
            let re = super::text_search::compile(&query)?;
            let files = scope.select(|| super::store::list_files(&root));
            Ok(super::text_search::search(&root, &files, &query, &re, limit, &cancel))
        })
    }

    /// Files git ignores, for "Include non-project items".
    pub fn ignored_files(&self, cx: &App) -> Task<Vec<String>> {
        let root = self.root.clone();
        cx.background_spawn(async move {
            let Some(root) = root else { return Vec::new() };
            let Ok(output) = crate::git::git_process()
                .args(["ls-files", "-z", "--others", "--ignored", "--exclude-standard", "--directory", "--no-empty-directory"])
                .current_dir(&root)
                .output()
            else {
                return Vec::new();
            };
            let mut out = Vec::new();
            for p in output.stdout.split(|b| *b == 0).filter(|p| !p.is_empty()) {
                let p = String::from_utf8_lossy(p).into_owned();
                if let Some(dir) = p.strip_suffix('/') {
                    // Expand ignored folders, but not huge build trees past a cap.
                    for entry in walk(&root.join(dir), 20_000) {
                        if let Ok(rel) = entry.strip_prefix(&root) {
                            out.push(rel.to_string_lossy().replace('\\', "/"));
                        }
                    }
                } else {
                    out.push(p);
                }
                if out.len() > 50_000 {
                    break;
                }
            }
            out
        })
    }

    /// For the status bar.
    pub fn summary(&self) -> String {
        let libraries = self.library_phase.load(std::sync::atomic::Ordering::Relaxed);
        match self.progress {
            Some((_, 0)) if libraries => return "Scanning external libraries…".into(),
            Some((done, total)) if libraries => return format!("Indexing external libraries… {done}/{total}"),
            Some((done, total)) => return format!("Indexing… {done}/{total}"),
            None => {}
        }
        if self.updating {
            return "Indexing…".into();
        }
        let mut out = self
            .counts
            .borrow_mut()
            .get_or_insert_with(|| {
                let Ok(index) = self.index.read() else { return String::new() };
                if index.files.is_empty() {
                    return String::new();
                }
                let mut out = format!("{} files, {} symbols indexed", index.files.len(), index.symbol_count());
                if !index.external.is_empty() {
                    out.push_str(&format!(" · {} libraries", index.external.libraries.len()));
                }
                out
            })
            .clone();
        if out.is_empty() {
            return out;
        }
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

fn walk(dir: &Path, cap: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(path),
                Ok(t) if t.is_file() => out.push(path),
                _ => {}
            }
            if out.len() >= cap {
                return out;
            }
        }
    }
    out
}
