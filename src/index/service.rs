//! The project's code index as an entity: background (re)indexing with
//! progress, and the navigation queries the editor and tool windows run —
//! bridges first, then the language server, then the built-in index.

use std::future::Future;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Task};

use crate::domain::rename::Occurrence;
use crate::domain::version_catalog::{Catalog, Entry};
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
    /// The language servers published other problems (`server_problems`).
    ServerProblems,
    /// Another build variant was picked: other source sets count.
    Variants,
}

pub struct CodeIndex {
    root: Option<PathBuf>,
    pub index: Arc<RwLock<ProjectIndex>>,
    pub lsp: Arc<LspManager>,
    /// The Android resources of the project's `res` folders.
    pub resources: Arc<RwLock<crate::usecase::android_resources::ResourceTable>>,
    /// The Gradle version catalogs (`gradle/libs.versions.toml`).
    pub catalogs: Arc<RwLock<Vec<Catalog>>>,
    /// The Android framework's resources (`android.R`, `@android:`) of the
    /// compileSdk platform, and the folder they were read from.
    pub framework: Arc<RwLock<Option<(PathBuf, Arc<crate::usecase::android_resources::ResourceTable>)>>>,
    /// The res files' stamp the resource table was built from.
    resources_stamp: Arc<std::sync::atomic::AtomicU64>,
    /// (done, total) while files are being indexed.
    pub progress: Option<(usize, usize)>,
    updating: bool,
    /// Set while the libraries are being indexed.
    library_phase: Arc<std::sync::atomic::AtomicBool>,
    /// Another refresh was asked for while one ran.
    again: bool,
    /// The next refresh scans the libraries again (sources were downloaded).
    rescan_libraries: Arc<std::sync::atomic::AtomicBool>,
    /// The variant each Android module builds, by its folder.
    variants: Vec<(PathBuf, String)>,
    _task: Option<Task<()>>,
    _poll: Option<Task<()>>,
    /// What the language servers report for the project's files, open or
    /// not, for the Problems window's Project Errors; the revision of
    /// their diagnostics it was read at, and the read under way.
    pub server_problems: Vec<(PathBuf, Vec<crate::domain::diagnostics::Problem>)>,
    server_revision: u64,
    server_task: Option<Task<()>>,
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
                let alive = this.update(cx, |this, cx| {
                    if !this.lsp.status().is_empty() {
                        cx.emit(IndexEvent::Progress);
                        this.read_server_problems(cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        Self {
            root: None,
            variants: Vec::new(),
            index: Arc::default(),
            lsp: Arc::default(),
            resources: Arc::default(),
            catalogs: Arc::default(),
            framework: Arc::default(),
            resources_stamp: Arc::default(),
            progress: None,
            updating: false,
            library_phase: Arc::default(),
            again: false,
            rescan_libraries: Arc::default(),
            _task: None,
            _poll: Some(poll),
            server_problems: Vec::new(),
            server_revision: 0,
            server_task: None,
            counts: Default::default(),
        }
    }

    /// Reads what the servers published again when it changed: their
    /// errors and warnings in the project's files, columns counted on the
    /// text on disk.
    fn read_server_problems(&mut self, cx: &mut Context<Self>) {
        let revision = self.lsp.diagnostics_revision();
        let Some(root) = self.root.clone() else { return };
        if revision == self.server_revision || self.server_task.is_some() {
            return;
        }
        self.server_revision = revision;
        let lsp = self.lsp.clone();
        let read = cx.background_spawn(async move {
            let mut files: Vec<(PathBuf, Vec<crate::domain::diagnostics::Problem>)> = lsp
                .all_diagnostics()
                .into_iter()
                .filter(|(path, list)| !list.is_empty() && path.starts_with(&root))
                .filter_map(|(path, list)| {
                    let text = std::fs::read_to_string(&path).ok()?;
                    let problems: Vec<_> = list.iter().filter_map(|d| crate::domain::diagnostics::from_server(&text, d.start, d.end, d.severity, &d.message)).collect();
                    (!problems.is_empty()).then_some((path, problems))
                })
                .collect();
            files.sort_by(|a, b| a.0.cmp(&b.0));
            files
        });
        self.server_task = Some(cx.spawn(async move |this, cx| {
            let files = read.await;
            this.update(cx, |this, cx| {
                this.server_task = None;
                if this.server_problems != files {
                    this.server_problems = files;
                    cx.emit(IndexEvent::ServerProblems);
                }
            })
            .ok();
        }));
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
        self.apply_variants();
        self.resources = Arc::default();
        self.catalogs = Arc::default();
        self.framework = Arc::default();
        self.resources_stamp = Arc::default();
        self.lsp = Arc::new(LspManager::new(root.clone().unwrap_or_default()));
        self.server_problems.clear();
        self.server_revision = 0;
        self.server_task = None;
        cx.emit(IndexEvent::ServerProblems);
        self.progress = None;
        self.updating = false;
        self._task = None;
        this_changed!(self, cx);
        if root.is_some() {
            self.refresh(cx);
        }
    }

    /// The variants picked in Build Variants, by module folder: names
    /// resolve to their source sets (`src/free/`, `src/debug/`) and not the
    /// other variants'.
    pub fn set_variants(&mut self, variants: Vec<(PathBuf, String)>, cx: &mut Context<Self>) {
        self.variants = variants;
        if self.apply_variants() {
            cx.emit(IndexEvent::Variants);
        }
    }

    /// Whether the index's variants changed.
    fn apply_variants(&self) -> bool {
        let Some(root) = self.root.as_deref() else { return false };
        let picked: Vec<(String, String)> = self
            .variants
            .iter()
            .filter_map(|(dir, variant)| {
                let rel = dir.strip_prefix(root).ok()?.to_string_lossy().replace('\\', "/");
                Some((rel, variant.clone()))
            })
            .collect();
        let mut index = self.index.write().unwrap();
        if index.picked == picked {
            return false;
        }
        index.picked = picked;
        index.refresh_modules();
        true
    }

    /// Re-indexes what changed since the last pass.
    /// Scans the libraries again: their sources changed (Download Sources).
    pub fn rescan_libraries(&mut self, cx: &mut Context<Self>) {
        self.rescan_libraries.store(true, std::sync::atomic::Ordering::Relaxed);
        self.refresh(cx);
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else { return };
        if self.updating {
            self.again = true;
            return;
        }
        self.updating = true;
        let index = self.index.clone();
        let resources = self.resources.clone();
        let catalogs = self.catalogs.clone();
        let framework = self.framework.clone();
        let resources_stamp = self.resources_stamp.clone();
        let phase = self.library_phase.clone();
        let rescan = self.rescan_libraries.clone();
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
                // `changed`: the index views read was replaced or edited;
                // `dirty`: it differs from the cache on disk. Loading an
                // up-to-date cache is the first, not the second: rewriting
                // it took seconds at every start on big projects.
                let (changed, dirty) = match fresh.as_mut() {
                    Some(fresh) => {
                        let dirty = fresh.update(&|done, total| {
                            tx.send((done, total)).ok();
                        });
                        (true, dirty)
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
                        (changed, changed)
                    }
                };
                if let Some(mut fresh) = fresh {
                    let mut index = index.write().unwrap();
                    if !index.picked.is_empty() {
                        fresh.picked = std::mem::take(&mut index.picked);
                        fresh.refresh_modules();
                    }
                    *index = fresh;
                }
                if changed {
                    content.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                if dirty {
                    if let Ok(index) = index.read() {
                        index.save();
                    }
                }
                rebuild_resources(&root, &resources, &resources_stamp);
                // Then the libraries, so project navigation is ready first.
                let (files, previous) = {
                    let index = index.read().unwrap();
                    (index.all_files.clone(), index.external.clone())
                };
                *catalogs.write().unwrap() = crate::infra::catalog_files::load(&root, &files);
                load_framework(&root, &files, &framework);
                let stamp = super::libraries::inputs_stamp(&root, &files);
                if rescan.swap(false, std::sync::atomic::Ordering::Relaxed) || !previous.scanned || previous.stamp != stamp {
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
        let (resources, root) = (self.resources.clone(), self.root.clone().unwrap_or_default());
        let catalogs = self.catalogs.clone();
        let resources_stamp = self.resources_stamp.clone();
        cx.spawn(async move |this, cx| {
            cx.background_spawn(async move {
                if let Ok(mut index) = index.write() {
                    index.refresh_file(&rel);
                }
                if rel.ends_with(".versions.toml") || rel.rsplit('/').next().is_some_and(|n| n.starts_with("settings.gradle")) {
                    let files = index.read().map(|i| i.all_files.clone()).unwrap_or_default();
                    *catalogs.write().unwrap() = crate::infra::catalog_files::load(&root, &files);
                }
                if rel.contains("/res/") || rel.starts_with("res/") {
                    rebuild_resources(&root, &resources, &resources_stamp);
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
        let task = self.go_to(path, text, offset, cx);
        cx.background_spawn(async move {
            match task.await {
                Nav::Targets(targets) => targets,
                Nav::Declaration => Vec::new(),
            }
        })
    }

    /// The framework's resource table, once read.
    pub fn framework_table(&self) -> Option<Arc<crate::usecase::android_resources::ResourceTable>> {
        self.framework.read().ok()?.as_ref().map(|(_, t)| t.clone())
    }

    /// Go to Declaration / Ctrl+click: where the identifier at `offset` is
    /// declared, or that it is a declaration itself (IntelliJ then shows
    /// its usages). Locals first, then bridges, the language server and the
    /// index. Off the main thread: it parses the file.
    pub fn go_to(&self, path: String, text: String, offset: usize, cx: &App) -> Task<Nav> {
        // A version catalog entry: its line in the catalog.
        if let Some(nav) = catalog_nav(&self.catalogs.read().unwrap(), &path, &text, offset) {
            return Task::ready(nav);
        }
        // An Android resource: its declarations in the res folders.
        if let Some(nav) = resource_nav(&self.resources.read().unwrap(), &path, &text, offset) {
            return Task::ready(nav);
        }
        // The framework's (`android.R.string.ok`, `@android:color/white`): in the SDK.
        if let Some(nav) = self.framework_table().and_then(|t| framework_nav(&t, &path, &text, offset)) {
            return Task::ready(nav);
        }
        // A Groovy build script's `buildSrc` names (`Deps.coreKtx`).
        if super::groovy_scripts::is_groovy_script(&path) {
            let index = self.index.clone();
            return cx.background_spawn(async move { Nav::Targets(super::groovy_scripts::targets(&index.read().unwrap(), &text, offset)) });
        }
        let (Some(root), Some(lang)) = (self.root.clone(), Self::lang_of(&path, &text)) else { return Task::ready(Nav::Targets(Vec::new())) };
        let (index, lsp) = (self.index.clone(), self.lsp.clone());
        cx.background_spawn(async move {
            // A parameter or local variable: its declaration in this file.
            if let Some(local) = super::locals::resolve(lang, &text, offset) {
                if local.on_decl {
                    return Nav::Declaration;
                }
                let (line, col) = nav::position(&text, local.decl);
                let name = nav::word_at(&text, offset).map(|(w, _)| w).unwrap_or_default();
                return Nav::Targets(vec![Target { path, line, col, name, label: format!("local · {}", lang.name()), container: None }]);
            }
            // `this`, `super`, `it`, `field`, a named argument.
            if let Some(special) = super::special::classify(lang, &text, offset) {
                let targets = super::special::targets(&index.read().unwrap(), &root, &path, lang, &text, offset, special);
                if !targets.is_empty() {
                    return Nav::Targets(targets);
                }
            }
            let local = {
                let index = index.read().unwrap();
                let bridged = nav::bridge_definitions(&index, &path, &text, offset);
                if !bridged.is_empty() {
                    return Nav::Targets(bridged);
                }
                if symbol_declared_at(&index, &path, &text, offset, false) {
                    return Nav::Declaration;
                }
                nav::definitions(&index, &path, lang, &text, offset)
            };
            // The server is exact but may still be loading the project; when
            // the index already has an answer, don't wait long for it.
            let patience = if local.is_empty() { Duration::from_secs(10) } else { Duration::from_millis(600) };
            let file = root.join(&path);
            // A server still starting doesn't hold up an answer the index
            // has; without one, it gets the same patience to come up.
            let client = match ProjectIndex::is_external(&path) {
                true => None,
                false if local.is_empty() => lsp.client_within(lang, &file, patience),
                false => lsp.client(lang, &file),
            };
            // Nor does one still loading the project (rust-analyzer on a big
            // workspace answers only once it has).
            let loading = client.as_ref().is_some_and(|c| c.progress.lock().is_ok_and(|p| p.is_some()));
            if let Some(client) = client.filter(|_| local.is_empty() || !loading) {
                let (line, col) = nav::position(&text, offset);
                if let Ok(locations) = client.definition(&file, lang, &text, line, col, patience) {
                    if !locations.is_empty() {
                        return Nav::Targets(locations.into_iter().map(|l| server_target(&root, l, lang)).collect());
                    }
                }
            }
            // A C prototype nothing else defines: its usages, too.
            if local.is_empty() && symbol_declared_at(&index.read().unwrap(), &path, &text, offset, true) {
                return Nav::Declaration;
            }
            Nav::Targets(local)
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
        if let Some((catalog, entry, _)) = catalog_entry_at(&self.catalogs.read().unwrap(), &path, &text, offset) {
            let value = catalog.describe(&entry);
            let value = if value.is_empty() { String::new() } else { format!("\n\n```\n{value}\n```") };
            return Task::ready(Some(format!("`{}`{value}\n\n{}", catalog.accessor_of(&entry), catalog.path)));
        }
        if let Some((r, _)) = resource_at(&path, &text, offset) {
            let table = self.resources.read().unwrap();
            if let Some(first) = table.declarations(&r).first() {
                let value = table.value(&r).map(|v| format!(" = {v}")).unwrap_or_default();
                return Task::ready(Some(format!("`{}`{value}\n\n{} · {}", r.xml(), first.folder, first.path)));
            }
        }
        if let (Some(r), Some(table)) = (crate::domain::android_resources::framework_reference_at(&path, &text, offset), self.framework_table()) {
            if let Some(first) = table.declarations(&r).first() {
                let value = table.value(&r).map(|v| format!(" = {v}")).unwrap_or_default();
                return Task::ready(Some(format!("`@android:{}/{}`{value}\n\n{} · {}", r.kind, r.name, first.folder, first.path)));
            }
        }
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
                // On a declaration's name: that declaration, not others of its name.
                if symbol_declared_at(&index, &path, &text, offset, true) {
                    let (word, range) = nav::word_at(&text, offset)?;
                    let (line, col) = nav::position(&text, range.start);
                    let s = index.file(&path)?.symbols.iter().find(|s| s.line == line && s.col == col && s.name == word)?;
                    vec![Target { path: path.clone(), line, col, name: word, label: format!("{} · {}", s.kind.label(), lang.name()), container: s.container.clone() }]
                } else {
                    nav::definitions(&index, &path, lang, &text, offset)
                }
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
        if let Some((catalog, entry, _)) = catalog_entry_at(&self.catalogs.read().unwrap(), &path, &text, offset) {
            let catalogs = self.catalogs.read().unwrap().clone();
            return cx.background_spawn(async move { catalog_usages(&root, &catalogs, &catalog, &entry) });
        }
        if let Some((r, _)) = resource_at(&path, &text, offset) {
            let declarations = self.resources.read().unwrap().declarations(&r).to_vec();
            if !declarations.is_empty() {
                return cx.background_spawn(async move { resource_usages(&root, &r, declarations) });
            }
        }
        let lang = Self::lang_of(&path, &text);
        let (index, lsp) = (self.index.clone(), self.lsp.clone());
        cx.background_spawn(async move {
            let Some((word, _)) = nav::word_at(&text, offset) else { return (String::new(), Vec::new()) };
            // A parameter or local variable: its uses in its scope.
            if let Some(local) = lang.and_then(|lang| super::locals::resolve(lang, &text, offset)).filter(|l| !l.member) {
                let line_text = |at: usize| {
                    let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
                    let end = text[at..].find('\n').map_or(text.len(), |i| at + i);
                    text[start..end].trim().to_owned()
                };
                let group = lang.map(|l| l.name()).unwrap_or_default().to_owned();
                let mut found = Vec::new();
                for (at, group) in std::iter::once((local.decl, "Declarations".to_owned())).chain(local.uses.iter().map(|u| (*u, group.clone()))) {
                    let (line, col) = nav::position(&text, at);
                    found.push(Usage { path: path.clone(), line, col, text: line_text(at), group });
                }
                return (word, found);
            }
            let mut found = {
                let index = index.read().unwrap();
                let mut found = nav::usages(&index, &root, &word, lang);
                // From a library's source: that library's usages too.
                if let Some(library) = ProjectIndex::is_external(&path).then(|| index.external.library_of(&path)).flatten() {
                    found.extend(nav::library_usages(&index, &library.files, &word));
                }
                found
            };
            // Only the symbol's: uses resolving to another of its name go
            // (another class's `add`, a parameter), as IntelliJ finds references.
            if let Some(lang) = lang {
                let index = index.read().unwrap();
                let declaration = if symbol_declared_at(&index, &path, &text, offset, true) {
                    Some((path.clone(), nav::position(&text, nav::word_at(&text, offset).map_or(offset, |(_, r)| r.start)).0))
                } else {
                    let targets = nav::definitions(&index, &path, lang, &text, offset);
                    match &targets[..] {
                        [only] if !ProjectIndex::is_external(&only.path) => Some((only.path.clone(), only.line)),
                        _ => None,
                    }
                };
                if let Some(mut same) = declaration.and_then(|(p, l)| nav::SameSymbol::new(&index, &p, l, &word)) {
                    let mut texts: std::collections::HashMap<String, Option<String>> = std::collections::HashMap::new();
                    found.retain(|u| {
                        let file = texts.entry(u.path.clone()).or_insert_with(|| if u.path == path { Some(text.clone()) } else { std::fs::read_to_string(root.join(&u.path)).ok() });
                        let at = file.as_deref().and_then(|t| crate::domain::rename::offset_of(t, u.line, u.col).map(|o| (t, o)));
                        at.is_none_or(|(t, o)| same.other_at(&u.path, t, o).is_none())
                    });
                }
            }
            // `buildSrc` names used in Groovy build scripts (not indexed).
            let declared: Vec<(String, u32)> = found.iter().filter(|u| u.group == "Declarations" && !super::groovy_scripts::is_groovy_script(&u.path)).map(|u| (u.path.clone(), u.line)).collect();
            if !declared.is_empty() {
                let index = index.read().unwrap();
                found.extend(super::groovy_scripts::usages(&index, &root, &word, &declared));
            }
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

    /// What Rename (Shift+F6) at `offset` renames: a parameter or local's
    /// uses in its scope, or the symbol's whole-word occurrences across the
    /// project, as Android Studio's Rename dialog lists them. Files are
    /// read from disk: save them first.
    pub fn rename_target(&self, path: String, text: String, offset: usize, cx: &App) -> Task<Result<RenameTarget, String>> {
        const NOT_A_NAME: &str = "Cannot perform refactoring.\nCaret should be positioned at the name of element to be refactored.";
        if ProjectIndex::is_external(&path) {
            return Task::ready(Err("Cannot perform refactoring.\nThe file is read-only.".into()));
        }
        let Some((word, _)) = nav::word_at(&text, offset) else { return Task::ready(Err(NOT_A_NAME.into())) };
        let (Some(root), Some(lang)) = (self.root.clone(), Self::lang_of(&path, &text)) else { return Task::ready(Err(NOT_A_NAME.into())) };
        if let Some(local) = super::locals::resolve(lang, &text, offset).filter(|l| !l.member) {
            let occurrences = std::iter::once(local.decl)
                .chain(local.uses.iter().copied())
                .map(|at| {
                    let (line, col) = nav::position(&text, at);
                    let line_text = text.split('\n').nth(line as usize).unwrap_or_default().trim().to_owned();
                    Occurrence { path: path.clone(), line, col, text: line_text, code: true, other: None }
                })
                .collect();
            return Task::ready(Ok(RenameTarget { name: word, kind: "variable".into(), declaration: path, occurrences }));
        }
        let go_to = self.go_to(path.clone(), text.clone(), offset, cx);
        let index = self.index.clone();
        let lsp = self.lsp.clone();
        cx.background_spawn(async move {
            // A language server's references are the symbol's, exactly.
            let server_hits = server_references(&lsp, &root, lang, &path, &text, offset);
            let (declaration, line) = match go_to.await {
                Nav::Declaration => (path, nav::position(&text, offset).0),
                Nav::Targets(targets) => {
                    let Some(target) = targets.first() else { return Err(format!("Cannot perform refactoring.\nCannot find declaration of '{word}'.")) };
                    if ProjectIndex::is_external(&target.path) {
                        return Err(format!("Cannot perform refactoring.\n'{word}' is declared in a library."));
                    }
                    (target.path.clone(), target.line)
                }
            };
            let index = index.read().unwrap();
            let kind = index
                .file(&declaration)
                .and_then(|e| e.symbols.iter().find(|s| s.name == word && s.line == line))
                .map_or("symbol", |s| s.kind.label())
                .to_owned();
            let mut occurrences = nav::rename_hits(&index, &root, &word);
            if let Some(hits) = server_hits.filter(|h| !h.is_empty()) {
                // The server's for code; the index's in comments and strings.
                occurrences.retain(|o| !o.code);
                occurrences.extend(hits);
            } else {
                nav::mark_other_symbols(&index, &root, &declaration, line, &word, &mut occurrences);
            }
            // A `buildSrc` name's uses in Groovy build scripts (not indexed).
            let groovy = super::groovy_scripts::usages(&index, &root, &word, &[(declaration.clone(), line)]);
            occurrences.extend(groovy.into_iter().map(|u| crate::domain::rename::Occurrence { path: u.path, line: u.line, col: u.col, text: u.text, code: true, other: None }));
            occurrences.sort_by(|a, b| (&a.path, a.line, a.col).cmp(&(&b.path, b.line, b.col)));
            if occurrences.is_empty() {
                return Err(NOT_A_NAME.into());
            }
            Ok(RenameTarget { name: word, kind, declaration, occurrences })
        })
    }

    /// Keeps a file's language server up to date for the editor's
    /// problems: sends `text` when given (the server starts on first use),
    /// then, when the server has published anything since revision
    /// `since`, reads what it last published for the file. `None` while no
    /// server runs for the file (the text wasn't sent); otherwise the
    /// server's revision and the file's diagnostics when they may have
    /// changed (`None`: unchanged, or nothing published for it yet).
    pub fn server_diagnostics(&self, path: String, text: Option<String>, since: u64, cx: &App) -> Task<Option<(u64, Option<Vec<super::lsp::ServerDiagnostic>>)>> {
        let (Some(root), Some(lang)) = (self.root.clone(), Self::lang_of(&path, text.as_deref().unwrap_or(""))) else { return Task::ready(None) };
        if ProjectIndex::is_external(&path) {
            return Task::ready(None);
        }
        let lsp = self.lsp.clone();
        cx.background_spawn(async move {
            let file = root.join(&path);
            let client = lsp.client(lang, &file)?;
            if let Some(text) = &text {
                client.sync(&file, lang, text).ok()?;
            }
            let revision = client.diagnostics_revision();
            Some((revision, (revision != since).then(|| client.published_diagnostics(&file)).flatten()))
        })
    }

    /// A language server's completions at `offset`; `None` without a
    /// running server for the file or when it doesn't answer in time, so
    /// the caller completes from the index instead.
    pub fn server_completions(&self, path: String, text: String, offset: usize, cx: &App) -> Task<Option<Vec<lsp_types::CompletionItem>>> {
        let (Some(root), Some(lang)) = (self.root.clone(), Self::lang_of(&path, &text)) else { return Task::ready(None) };
        if ProjectIndex::is_external(&path) || super::lsp::command_for(lang).is_none() {
            return Task::ready(None);
        }
        let lsp = self.lsp.clone();
        cx.background_spawn(async move {
            let file = root.join(&path);
            let client = lsp.client(lang, &file)?;
            let (line, col) = nav::position(&text, offset);
            let word = text[..offset].trim_end_matches(|c: char| c.is_alphanumeric() || c == '_' || c == '$');
            client.completion(&file, lang, &text, line, col, word.ends_with('.'), Duration::from_millis(1500)).ok()
        })
    }

    /// The edits a picked server completion brings once resolved.
    pub fn resolve_completion(&self, path: String, text: &str, item: lsp_types::CompletionItem, cx: &App) -> Task<Vec<super::lsp::TextEdit>> {
        let (Some(root), Some(lang)) = (self.root.clone(), Self::lang_of(&path, text)) else { return Task::ready(Vec::new()) };
        let lsp = self.lsp.clone();
        cx.background_spawn(async move {
            let Some(client) = lsp.client(lang, &root.join(&path)) else { return Vec::new() };
            client.resolve_completion(&item, Duration::from_secs(3)).unwrap_or_default()
        })
    }

    /// A language server's code actions (quick fixes, refactorings) for a
    /// range of the file; none without a running server.
    pub fn code_actions(&self, path: String, text: String, range: std::ops::Range<usize>, cx: &App) -> Task<Vec<super::lsp::CodeAction>> {
        let (Some(root), Some(lang)) = (self.root.clone(), Self::lang_of(&path, &text)) else { return Task::ready(Vec::new()) };
        if ProjectIndex::is_external(&path) {
            return Task::ready(Vec::new());
        }
        let lsp = self.lsp.clone();
        cx.background_spawn(async move {
            let file = root.join(&path);
            let Some(client) = lsp.client_within(lang, &file, Duration::from_secs(2)) else { return Vec::new() };
            let start = nav::position(&text, range.start);
            let end = nav::position(&text, range.end);
            client.code_actions(&file, lang, &text, start, end, Duration::from_secs(3)).unwrap_or_default()
        })
    }

    /// The language server's Organize Imports for the file: its edits to
    /// the file, or why there are none.
    pub fn organize_imports(&self, path: String, text: String, cx: &App) -> Task<Result<Vec<super::lsp::TextEdit>, String>> {
        let (Some(root), Some(lang)) = (self.root.clone(), Self::lang_of(&path, &text)) else { return Task::ready(Err("No language server for this file".into())) };
        if ProjectIndex::is_external(&path) || super::lsp::command_for(lang).is_none() {
            return Task::ready(Err("No language server for this file".into()));
        }
        let lsp = self.lsp.clone();
        cx.background_spawn(async move {
            let file = root.join(&path);
            let client = lsp.client_within(lang, &file, Duration::from_secs(10)).ok_or("The language server isn't running")?;
            if client.offers(super::lsp::ORGANIZE_IMPORTS) == Some(false) {
                return Err("The language server can't optimize imports".into());
            }
            // A server that can offers nothing when the imports are in order.
            let Some(action) = client.organize_imports(&file, lang, &text, Duration::from_secs(5)).map_err(|e| e.to_string())? else { return Ok(Vec::new()) };
            let edits = client.code_action_edits(&action).map_err(|e| e.to_string())?;
            Ok(edits.into_iter().filter(|f| f.path == file).flat_map(|f| f.edits).collect())
        })
    }

    /// A code action's edits: its own, resolved, or its command's.
    pub fn code_action_edits(&self, path: String, text: String, action: super::lsp::CodeAction, cx: &App) -> Task<Result<Vec<super::lsp::FileEdit>, String>> {
        let (Some(root), Some(lang)) = (self.root.clone(), Self::lang_of(&path, &text)) else { return Task::ready(Err("No language server".into())) };
        let lsp = self.lsp.clone();
        cx.background_spawn(async move {
            let file = root.join(&path);
            let client = lsp.client(lang, &file).ok_or("The language server has stopped")?;
            client.code_action_edits(&action).map_err(|e| e.to_string())
        })
    }

    /// A file's text, its own path or one relative to the project.
    fn read_file(root: Option<&Path>, path: &str) -> Option<String> {
        let file = Path::new(path);
        let file = if file.is_absolute() { file.to_path_buf() } else { root?.join(file) };
        std::fs::read_to_string(file).ok()
    }

    /// Call Hierarchy: the functions calling the one named at `offset` of
    /// `path`, by name. Off the main thread: it reads the files.
    pub fn callers_of(this: &Entity<Self>, path: String, offset: usize, ancestors: Vec<String>, cx: &mut App) -> Task<Vec<super::call_hierarchy::CallNode>> {
        let root = this.read(cx).root.clone();
        let this = this.clone();
        cx.spawn(async move |cx| {
            let read_root = root.clone();
            let file = path.clone();
            let Some(text) = cx.background_spawn(async move { Self::read_file(read_root.as_deref(), &file) }).await else { return Vec::new() };
            let usages = this.read_with(cx, |index, cx| index.usages(path, text, offset, cx));
            let (_, usages) = usages.await;
            cx.background_spawn(async move {
                let places = usages.into_iter().filter(|u| u.group != "Declarations").map(|u| (u.path, u.line, u.col, (u.line, u.col))).collect();
                let mut nodes = super::call_hierarchy::group_by_function(places, &ancestors, &|p| Self::read_file(root.as_deref(), p));
                nodes.sort_by(|a, b| a.name.cmp(&b.name));
                nodes
            })
            .await
        })
    }

    /// Callee Hierarchy: the functions the one named at `offset` of `path`
    /// calls, in order, each resolved to its declaration.
    pub fn callees_of(this: &Entity<Self>, path: String, offset: usize, ancestors: Vec<String>, cx: &mut App) -> Task<Vec<super::call_hierarchy::CallNode>> {
        let root = this.read(cx).root.clone();
        let this = this.clone();
        cx.spawn(async move |cx| {
            let (read_root, file) = (root.clone(), path.clone());
            let Some(text) = cx.background_spawn(async move { Self::read_file(read_root.as_deref(), &file) }).await else { return Vec::new() };
            let lang = Lang::from_path(&path);
            let calls: Vec<usize> = lang.map(|lang| super::call_hierarchy::calls_in(lang, &text, offset)).unwrap_or_default();
            let tasks: Vec<_> = this.read_with(cx, |index, cx| calls.iter().map(|&o| index.definitions(path.clone(), text.clone(), o, cx)).collect());
            let mut places = Vec::new();
            for (task, &call) in tasks.into_iter().zip(&calls) {
                if let Some(t) = task.await.into_iter().next() {
                    places.push((t.path, t.line, t.col, super::call_hierarchy::position_of(&text, call)));
                }
            }
            cx.background_spawn(async move { super::call_hierarchy::group_by_function(places, &ancestors, &|p| Self::read_file(root.as_deref(), p)) }).await
        })
    }

    /// The type a Type Hierarchy (Ctrl+H) at `offset` is for.
    pub fn type_at(&self, path: &str, text: &str, offset: usize) -> Option<String> {
        let lang = Self::lang_of(path, text)?;
        let index = self.index.read().ok()?;
        super::call_hierarchy::type_at(lang, text, offset, |w| index.symbols_named(w).any(|(_, _, s)| s.kind.is_type()))
    }

    pub fn type_hierarchy(&self, name: String, view: super::hierarchy::TypeView, cx: &App) -> Task<super::hierarchy::TypeNode> {
        let index = self.index.clone();
        cx.background_spawn(async move { super::hierarchy::type_hierarchy(&index.read().unwrap(), &name, view) })
    }

    /// Analyze Dependencies… of project files (project-relative).
    pub fn dependencies(&self, files: Vec<String>, cx: &App) -> Task<Vec<super::dependencies::FileDeps>> {
        let (index, root) = (self.index.clone(), self.root.clone().unwrap_or_default());
        cx.background_spawn(async move {
            let read = |p: &str| std::fs::read_to_string(root.join(p)).ok();
            super::dependencies::dependencies(&index.read().unwrap(), &files, &read)
        })
    }

    pub fn method_hierarchy(&self, owner: String, method: String, cx: &App) -> Task<super::hierarchy::TypeNode> {
        let index = self.index.clone();
        cx.background_spawn(async move { super::hierarchy::method_hierarchy(&index.read().unwrap(), &owner, &method) })
    }

    /// The editor gutter's implementation markers for a file's current text.
    pub fn gutter_marks(&self, path: String, text: String, cx: &App) -> Task<Vec<super::hierarchy::GutterMark>> {
        let Some(lang) = Self::lang_of(&path, &text) else { return Task::ready(Vec::new()) };
        let index = self.index.clone();
        cx.background_spawn(async move {
            let symbols = super::symbols::extract(lang, &text);
            let mut marks = super::hierarchy::gutter_marks(&index.read().unwrap(), &path, &text, &symbols);
            for mark in &mut marks {
                if symbols.iter().any(|s| s.line == mark.line && s.name == mark.name && s.kind.is_callable()) {
                    mark.signature = super::hierarchy::signature(&text, mark.line, &mark.name);
                }
            }
            marks
        })
    }

    pub fn search_symbols(&self, query: String, types_only: bool, libraries: bool, cx: &App) -> Task<Vec<nav::SymbolMatch>> {
        let index = self.index.clone();
        cx.background_spawn(async move { nav::search_symbols(&index.read().unwrap(), &query, types_only, libraries, 200) })
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

/// Where Go to Declaration leads.
/// What a Rename renames.
#[derive(Clone, Debug)]
pub struct RenameTarget {
    pub name: String,
    /// "class", "method", "variable"…
    pub kind: String,
    /// The declaring file.
    pub declaration: String,
    pub occurrences: Vec<Occurrence>,
}

pub enum Nav {
    Targets(Vec<Target>),
    /// On a declaration: IntelliJ shows its usages.
    Declaration,
}

/// The references a language server knows for the symbol at `offset`,
/// as Rename's occurrences (project files only); none without a server.
fn server_references(lsp: &LspManager, root: &Path, lang: Lang, path: &str, text: &str, offset: usize) -> Option<Vec<Occurrence>> {
    let file = root.join(path);
    let client = lsp.client_within(lang, &file, Duration::from_secs(10))?;
    // A server still indexing answers with what it has so far.
    let waited = std::time::Instant::now();
    while client.progress.lock().is_ok_and(|p| p.is_some()) && waited.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(200));
    }
    let (line, col) = nav::position(text, offset);
    let locations = client.references(&file, lang, text, line, col).ok()?;
    let mut sources: HashMap<PathBuf, String> = HashMap::new();
    let out = locations
        .into_iter()
        .filter_map(|l| {
            let rel = l.path.strip_prefix(root).ok()?.to_string_lossy().replace('\\', "/");
            let source = if rel == path { text.to_owned() } else { sources.entry(l.path.clone()).or_insert_with(|| std::fs::read_to_string(&l.path).unwrap_or_default()).clone() };
            let line_text = source.split('\n').nth(l.line as usize)?.trim().to_owned();
            Some(Occurrence { path: rel, line: l.line, col: l.col, text: line_text, code: true, other: None })
        })
        .collect();
    Some(out)
}

/// The identifier at `offset` is the name of a symbol the index has for
/// this file. A C / C++ / Objective-C prototype counts only when
/// `prototypes` (otherwise it goes on to its definition).
fn symbol_declared_at(index: &ProjectIndex, path: &str, text: &str, offset: usize, prototypes: bool) -> bool {
    let Some((word, range)) = nav::word_at(text, offset) else { return false };
    let (line, col) = nav::position(text, range.start);
    index.file(path).is_some_and(|e| {
        let c_like = matches!(e.lang, Lang::C | Lang::Cpp | Lang::ObjC);
        e.symbols.iter().any(|s| s.line == line && s.col == col && s.name == word && (prototypes || !(c_like && s.decl)))
    })
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

/// The Android resource at `offset`: a reference in Kotlin / Java
/// (`R.string.x`) or XML (`@string/x`), or a declaration's name in a values
/// file. The bool tells a declaration (`<string name=…>`, `@+id/`).
fn resource_at(path: &str, text: &str, offset: usize) -> Option<(crate::domain::android_resources::ResourceRef, bool)> {
    use crate::domain::android_resources as res;
    if path.ends_with(".kt") || path.ends_with(".java") || path.ends_with(".kts") {
        return res::code_reference_at(text, offset).map(|r| (r, false));
    }
    if path.ends_with(".xml") {
        if let Some(r) = res::declaration_at(path, text, offset) {
            return Some((r, true));
        }
        return res::xml_reference_at(text, offset);
    }
    None
}

/// Go to Declaration on a resource: its declarations (a values entry, a
/// file per folder); on a declaration, its usages.
fn resource_nav(table: &crate::usecase::android_resources::ResourceTable, path: &str, text: &str, offset: usize) -> Option<Nav> {
    let (r, declaration) = resource_at(path, text, offset)?;
    let decls = table.declarations(&r);
    if decls.is_empty() {
        return None;
    }
    if declaration {
        return Some(Nav::Declaration);
    }
    Some(Nav::Targets(decls.iter().map(|d| Target { path: d.path.to_string(), line: d.line, col: d.col, name: r.name.to_string(), label: d.folder.to_string(), container: None }).collect()))
}

/// Go to Declaration on a framework resource: its declarations in the SDK.
fn framework_nav(table: &crate::usecase::android_resources::ResourceTable, path: &str, text: &str, offset: usize) -> Option<Nav> {
    let r = crate::domain::android_resources::framework_reference_at(path, text, offset)?;
    let decls = table.declarations(&r);
    (!decls.is_empty()).then(|| Nav::Targets(decls.iter().map(|d| Target { path: d.path.to_string(), line: d.line, col: d.col, name: r.name.to_string(), label: d.folder.to_string(), container: None }).collect()))
}

/// Reads the framework's resources when the project's platform changed
/// (a few thousand values and drawables; read once).
fn load_framework(root: &Path, files: &[String], framework: &RwLock<Option<(PathBuf, Arc<crate::usecase::android_resources::ResourceTable>)>>) {
    let dir = super::libraries::android_platform_res(root, files);
    let current = framework.read().unwrap().as_ref().map(|(d, _)| d.clone());
    if current == dir {
        return;
    }
    let table = dir.map(|res| {
        let table = crate::usecase::android_resources::ResourceTable::build(&crate::infra::resource_scanner::FrameworkRes { res: res.clone() });
        (res, Arc::new(table))
    });
    *framework.write().unwrap() = table;
}

/// Find Usages of a resource: `R.type.name` in code and `@type/name` in
/// XML across the project, after its declarations.
fn resource_usages(root: &Path, r: &crate::domain::android_resources::ResourceRef, declarations: Vec<crate::domain::android_resources::Declaration>) -> (String, Vec<Usage>) {
    use super::text_search::{SearchContext, TextQuery};
    let line_of = |path: &str, line: u32| std::fs::read_to_string(root.join(path)).ok().and_then(|t| t.lines().nth(line as usize).map(|l| l.trim().to_owned())).unwrap_or_default();
    let mut found: Vec<Usage> = declarations.iter().map(|d| Usage { path: d.path.to_string(), line: d.line, col: d.col, text: line_of(&d.path, d.line), group: "Declarations".into() }).collect();
    let query = TextQuery { text: crate::domain::android_resources::usage_pattern(r), case_sensitive: true, whole_words: false, regex: true, mask: None, context: SearchContext::Anywhere };
    if let Ok(re) = super::text_search::compile(&query) {
        let files = super::store::list_files(root);
        let result = super::text_search::search(root, &files, &query, &re, 5000, &std::sync::atomic::AtomicBool::new(false));
        for m in result.matches {
            if found.iter().any(|u| u.path == m.path && u.line == m.line) {
                continue;
            }
            let group = if m.path.ends_with(".xml") { "Resource references in XML" } else { "Resource references in code" };
            found.push(Usage { path: m.path, line: m.line, col: m.col, text: m.text, group: group.into() });
        }
    }
    (r.name.to_string(), found)
}

/// Safe Delete of string resources (the Translations Editor's Remove
/// Keys): each use of the keys, `R.string.x` in code or `@string/x` in XML,
/// across the project as "path:line".
pub fn string_uses(root: &Path, keys: &[String]) -> Vec<String> {
    use super::text_search::{SearchContext, TextQuery};
    use crate::domain::android_resources::{ResourceRef, usage_pattern};
    if keys.is_empty() {
        return Vec::new();
    }
    let pattern = keys.iter().map(|k| usage_pattern(&ResourceRef { kind: "string".into(), name: k.as_str().into() })).collect::<Vec<_>>().join("|");
    let query = TextQuery { text: pattern, case_sensitive: true, whole_words: false, regex: true, mask: None, context: SearchContext::Anywhere };
    let Ok(re) = super::text_search::compile(&query) else { return Vec::new() };
    let files = super::store::list_files(root);
    let result = super::text_search::search(root, &files, &query, &re, 1000, &std::sync::atomic::AtomicBool::new(false));
    result.matches.into_iter().map(|m| format!("{}:{}", m.path, m.line + 1)).collect()
}

/// The version catalog entry at `offset`: an accessor in a build script
/// (`libs.androidx.core.ktx`), or in a catalog an entry's key (a
/// declaration, `true`) or a reference to one (`version.ref`, a bundle
/// member). A catalog being edited is read from its text.
fn catalog_entry_at(catalogs: &[Catalog], path: &str, text: &str, offset: usize) -> Option<(Catalog, Entry, bool)> {
    use crate::domain::version_catalog as vc;
    if vc::is_build_script(path) {
        let (name, accessor, _) = vc::accessor_at(text, offset, catalogs.iter().map(|c| c.name.as_str()))?;
        let catalog = catalogs.iter().find(|c| c.name == name)?;
        let entry = catalog.resolve(&accessor)?.clone();
        return Some((catalog.clone(), entry, false));
    }
    let name = vc::name_of(path.rsplit('/').next()?)?;
    let name = catalogs.iter().find(|c| c.path == path).map_or(name, |c| c.name.as_str());
    let catalog = Catalog::parse(name, path, text);
    let (entry, declaration) = catalog.reference_at(offset)?;
    let entry = entry.clone();
    Some((catalog, entry, declaration))
}

/// Go to Declaration on a catalog accessor or reference: the entry's key;
/// on the key itself, its usages.
fn catalog_nav(catalogs: &[Catalog], path: &str, text: &str, offset: usize) -> Option<Nav> {
    let (catalog, entry, declaration) = catalog_entry_at(catalogs, path, text, offset)?;
    if declaration {
        return Some(Nav::Declaration);
    }
    let label = catalog.path.rsplit('/').next().unwrap_or(&catalog.path).to_owned();
    Some(Nav::Targets(vec![Target { path: catalog.path.clone(), line: entry.line, col: entry.col, name: entry.key.clone(), label, container: None }]))
}

/// Find Usages of a catalog entry: its key, the references inside the
/// catalog, and the build scripts' accessors.
fn catalog_usages(root: &Path, catalogs: &[Catalog], catalog: &Catalog, entry: &Entry) -> (String, Vec<Usage>) {
    use super::text_search::{SearchContext, TextQuery};
    use crate::domain::version_catalog as vc;
    let catalog_text = std::fs::read_to_string(root.join(&catalog.path)).unwrap_or_default();
    let line_of = |text: &str, line: u32| text.lines().nth(line as usize).map(|l| l.trim().to_owned()).unwrap_or_default();
    let mut found = vec![Usage { path: catalog.path.clone(), line: entry.line, col: entry.col, text: line_of(&catalog_text, entry.line), group: "Declarations".into() }];
    // The saved catalog's references (the edited text may differ).
    let saved = Catalog::parse(&catalog.name, &catalog.path, &catalog_text);
    if let Some(target) = saved.by_key(entry.section, &entry.key) {
        for range in saved.references_to(target) {
            let p = crate::domain::diagnostics::Pos::of(&catalog_text, range.start);
            found.push(Usage { path: catalog.path.clone(), line: p.line, col: p.col, text: line_of(&catalog_text, p.line), group: "Version catalog references".into() });
        }
    }
    let accessor = catalog.accessor_of(entry);
    let query = TextQuery { text: format!(r"\b{}\b", regex::escape(&accessor)), case_sensitive: true, whole_words: false, regex: true, mask: None, context: SearchContext::Anywhere };
    if let Ok(re) = super::text_search::compile(&query) {
        let files: Vec<String> = super::store::list_files(root).into_iter().filter(|f| vc::is_build_script(f)).collect();
        let result = super::text_search::search(root, &files, &query, &re, 5000, &std::sync::atomic::AtomicBool::new(false));
        for m in result.matches {
            // `libs.androidx.core` also starts `libs.androidx.core.ktx`: keep the ones naming this entry.
            let names = vc::accessor_at(&m.text, m.text.find(&accessor).unwrap_or(0), catalogs.iter().map(|c| c.name.as_str()));
            let same = names.and_then(|(name, path, _)| catalogs.iter().find(|c| c.name == name)?.resolve(&path).map(|e| e.section == entry.section && e.key == entry.key));
            if same != Some(true) {
                continue;
            }
            found.push(Usage { path: m.path, line: m.line, col: m.col, text: m.text, group: "Build script references".into() });
        }
    }
    (entry.key.clone(), found)
}

/// Rebuilds the resource table when a res file changed since it was built
/// (checking costs a directory walk; a big app's build, half a second).
fn rebuild_resources(root: &Path, resources: &RwLock<crate::usecase::android_resources::ResourceTable>, built: &std::sync::atomic::AtomicU64) {
    let folders = crate::infra::resource_scanner::ResFolders { root: root.to_path_buf() };
    let stamp = folders.stamp();
    if built.swap(stamp, std::sync::atomic::Ordering::Relaxed) == stamp && stamp != 0 {
        return;
    }
    let table = crate::usecase::android_resources::ResourceTable::build(&folders);
    *resources.write().unwrap() = table;
}
