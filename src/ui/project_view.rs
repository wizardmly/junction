//! The Project tool window, after IntelliJ's: a view selector (Project,
//! Project Files, Production, Tests, Open Files, Changed Files), the header's
//! New / Select Opened File / Expand All / Collapse All buttons, the ⋮ menu
//! (Behavior, Appearance, Sort By), speed search and keyboard navigation.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, Hsla, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, Render, ScrollStrategy, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, UniformListScrollHandle, Window, div, prelude::FluentBuilder as _, px, uniform_list,
};

use crate::git::status::StatusKind;
use crate::index::nav::Target;
use crate::index::service::{CodeIndex, IndexEvent};
use crate::model::{RepoEvent, RepoModel};
use crate::settings::{ProjectSettings, ProjectSort, Settings};
use crate::theme::ActivePalette as _;
use crate::ui::common::{self, row_height, tool_button};
use crate::ui::file_menus::{FileActions, FileClipboard, ProjectTarget};
use crate::ui::navigate::OpenTarget;

/// The view selector's choices ("Project ▾").
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProjectMode {
    Project,
    ProjectFiles,
    Production,
    Tests,
    OpenFiles,
    Changed,
}

impl ProjectMode {
    const ALL: [ProjectMode; 6] = [Self::Project, Self::ProjectFiles, Self::Production, Self::Tests, Self::OpenFiles, Self::Changed];

    pub fn label(self) -> &'static str {
        match self {
            Self::Project => "Project",
            Self::ProjectFiles => "Project Files",
            Self::Production => "Production",
            Self::Tests => "Tests",
            Self::OpenFiles => "Open Files",
            Self::Changed => "Changed Files",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RowKind {
    Root,
    Dir,
    File,
    /// "External Libraries" and one library under it.
    Libraries,
    Library,
}

#[derive(Clone, Debug)]
struct Row {
    depth: usize,
    name: String,
    /// Repository-relative; empty for the root row.
    path: String,
    kind: RowKind,
    /// Ignored by Git: IntelliJ's "excluded", drawn in olive.
    excluded: bool,
}

#[derive(Default)]
struct Dir {
    dirs: BTreeMap<String, Dir>,
    /// (name, excluded)
    files: Vec<(String, bool)>,
    excluded: bool,
    /// An excluded folder's children come from disk on first expand.
    loaded: bool,
}

impl Dir {
    fn insert(&mut self, path: &str, excluded: bool) {
        let mut dir = self;
        let mut parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        let Some(last) = parts.pop() else { return };
        for part in parts {
            dir = dir.dirs.entry(part.to_owned()).or_insert_with(|| Dir { loaded: true, ..Default::default() });
        }
        if path.ends_with('/') {
            let d = dir.dirs.entry(last.to_owned()).or_default();
            d.excluded = excluded;
            d.loaded = !excluded;
        } else {
            dir.files.push((last.to_owned(), excluded));
        }
    }

    fn get_mut(&mut self, path: &str) -> Option<&mut Dir> {
        let mut dir = self;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            dir = dir.dirs.get_mut(part)?;
        }
        Some(dir)
    }
}

/// The Project tool window.
pub struct ProjectView {
    index: Entity<CodeIndex>,
    model: Option<Entity<RepoModel>>,
    actions: Option<FileActions>,
    clipboard: FileClipboard,
    mode: ProjectMode,
    settings: ProjectSettings,
    tree: Dir,
    expanded: HashSet<String>,
    selected: Option<String>,
    rows: Rc<Vec<Row>>,
    files: Rc<Vec<String>>,
    /// `git ls-files --ignored --directory`: folders end with '/'.
    ignored: Rc<Vec<String>>,
    open_files: Vec<String>,
    current_file: Option<String>,
    status: Rc<HashMap<String, StatusKind>>,
    /// Folders holding changed files (VCS-colored like their files).
    changed_dirs: Rc<HashSet<String>>,
    /// Speed search: the typed text while the popup is shown.
    search: Option<String>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    root_dir: Option<PathBuf>,
    /// External Libraries: (name, root, absolute files).
    libraries: Rc<Vec<(String, PathBuf, Vec<String>)>>,
    libraries_from: usize,
    _subscriptions: Vec<Subscription>,
}

/// Row keys under External Libraries (not repository paths).
const LIBRARIES_KEY: &str = "\u{1}libraries";

fn library_key(ix: usize) -> String {
    format!("\u{1}library/{ix}")
}

impl EventEmitter<OpenTarget> for ProjectView {}

/// Show a file in the editor's preview tab (a single click, Enable Preview Tab).
pub struct PreviewFile(pub String);

impl EventEmitter<PreviewFile> for ProjectView {}

impl Focusable for ProjectView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ProjectView {
    pub fn new(index: Entity<CodeIndex>, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            cx.subscribe(&index, |this, _, event: &IndexEvent, cx| {
                if matches!(event, IndexEvent::Changed) {
                    this.reload(cx)
                }
            }),
            cx.observe_global::<Settings>(|this, cx| {
                let settings = Settings::get(cx).project.clone();
                if settings != this.settings {
                    let excluded_changed = settings.show_excluded != this.settings.show_excluded;
                    this.settings = settings;
                    if excluded_changed {
                        this.rebuild_tree(cx);
                    }
                    this.flatten();
                    cx.notify();
                }
            }),
        ];
        let mut expanded = HashSet::new();
        expanded.insert(String::new());
        let mut this = Self {
            index,
            model: None,
            actions: None,
            clipboard: Rc::default(),
            mode: ProjectMode::Project,
            settings: Settings::get(cx).project.clone(),
            tree: Dir::default(),
            expanded,
            selected: None,
            rows: Rc::default(),
            files: Rc::default(),
            ignored: Rc::default(),
            open_files: Vec::new(),
            current_file: None,
            status: Rc::default(),
            changed_dirs: Rc::default(),
            search: None,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            root_dir: None,
            libraries: Rc::default(),
            libraries_from: 0,
            _subscriptions: subscriptions,
        };
        this.reload(cx);
        this
    }

    pub fn set_menu(&mut self, model: Entity<RepoModel>, actions: FileActions, cx: &mut Context<Self>) {
        self._subscriptions.push(cx.subscribe(&model, |this, _, event: &RepoEvent, cx| {
            if matches!(event, RepoEvent::Reloaded) {
                this.reload_status(cx);
            }
        }));
        self.model = Some(model);
        self.actions = Some(actions);
        self.reload_status(cx);
    }

    /// The editor switched files: remembered for Select Opened File, and
    /// selected right away under Always Select Opened File.
    pub fn file_opened(&mut self, path: &str, open_files: Vec<String>, cx: &mut Context<Self>) {
        self.current_file = Some(path.to_owned());
        self.open_files = open_files;
        if self.mode == ProjectMode::OpenFiles {
            self.rebuild_tree(cx);
            self.expand_all(cx);
        }
        if self.settings.autoscroll_from_source {
            self.reveal(path, cx);
        }
        cx.notify();
    }

    /// Tabs closed: the Open Files view follows.
    pub fn set_open_files(&mut self, open_files: Vec<String>, cx: &mut Context<Self>) {
        self.open_files = open_files;
        if self.mode == ProjectMode::OpenFiles {
            self.rebuild_tree(cx);
            self.expand_all(cx);
        }
        cx.notify();
    }

    fn root(&self, cx: &App) -> Option<PathBuf> {
        self.index.read(cx).root().map(Path::to_path_buf)
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let seen = self.libraries_from;
        let (files, libraries) = self
            .index
            .read(cx)
            .index
            .read()
            .map(|i| {
                // The library index is replaced, never changed in place.
                let from = std::sync::Arc::as_ptr(&i.external) as usize;
                let libraries = (from != seen).then(|| {
                    let list: Vec<(String, PathBuf, Vec<String>)> = i.external.libraries.iter().map(|l| (l.name.clone(), l.root.clone(), l.files.clone())).collect();
                    (from, list)
                });
                (i.all_files.clone(), libraries)
            })
            .unwrap_or_default();
        if let Some((from, libraries)) = libraries {
            self.libraries_from = from;
            self.libraries = Rc::new(libraries);
            self.flatten();
            cx.notify();
        }
        if *self.files == files && !self.files.is_empty() {
            return;
        }
        self.files = Rc::new(files);
        self.root_dir = self.root(cx);
        self.rebuild_tree(cx);
        self.flatten();
        cx.notify();
        self.load_ignored(cx);
    }

    fn load_ignored(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root(cx) else { return };
        let task = cx.background_spawn(async move {
            let Ok(output) = crate::git::git_process()
                .args(["ls-files", "-z", "--others", "--ignored", "--exclude-standard", "--directory"])
                .current_dir(&root)
                .output()
            else {
                return Vec::new();
            };
            output.stdout.split(|b| *b == 0).filter(|p| !p.is_empty()).map(|p| String::from_utf8_lossy(p).into_owned()).collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let ignored = task.await;
            let _ = this.update(cx, |this, cx| {
                if *this.ignored != ignored {
                    this.ignored = Rc::new(ignored);
                    this.rebuild_tree(cx);
                    this.flatten();
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn reload_status(&mut self, cx: &mut Context<Self>) {
        let Some(model) = &self.model else { return };
        let mut status = HashMap::new();
        let mut dirs = HashSet::new();
        for entry in &model.read(cx).status().entries {
            status.insert(entry.path.clone(), entry.kind);
            if entry.kind != StatusKind::Unversioned {
                let mut p = entry.path.as_str();
                while let Some(ix) = p.rfind('/') {
                    p = &p[..ix];
                    if !dirs.insert(p.to_owned()) {
                        break;
                    }
                }
            }
        }
        if *self.status == status {
            return;
        }
        self.status = Rc::new(status);
        self.changed_dirs = Rc::new(dirs);
        if self.mode == ProjectMode::Changed {
            self.rebuild_tree(cx);
            self.flatten();
        }
        cx.notify();
    }

    /// The files the current view shows, before folders are built.
    fn rebuild_tree(&mut self, _: &mut Context<Self>) {
        let mut tree = Dir { loaded: true, ..Default::default() };
        let files: Vec<String> = match self.mode {
            ProjectMode::Project | ProjectMode::ProjectFiles => self.files.to_vec(),
            ProjectMode::Production => self.files.iter().filter(|f| !crate::index::text_search::is_test_path(f)).cloned().collect(),
            ProjectMode::Tests => self.files.iter().filter(|f| crate::index::text_search::is_test_path(f)).cloned().collect(),
            ProjectMode::OpenFiles => self.open_files.clone(),
            ProjectMode::Changed => {
                let mut v: Vec<String> = self.status.keys().cloned().collect();
                v.sort();
                v
            }
        };
        for f in &files {
            tree.insert(f, false);
        }
        if self.settings.show_excluded && matches!(self.mode, ProjectMode::Project | ProjectMode::ProjectFiles) {
            for p in self.ignored.iter() {
                tree.insert(p, true);
            }
        }
        self.tree = tree;
    }

    /// Reads an excluded folder's children from disk the first time it opens.
    fn load_dir(&mut self, path: &str, root: &Path) {
        let Some(dir) = self.tree.get_mut(path) else { return };
        if dir.loaded {
            return;
        }
        dir.loaded = true;
        let Ok(entries) = std::fs::read_dir(root.join(path)) else { return };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                dir.dirs.insert(name, Dir { excluded: true, ..Default::default() });
            } else {
                dir.files.push((name, true));
            }
        }
    }

    fn flatten(&mut self) {
        let mut rows = Vec::new();
        let root_dir = self.root_dir.clone();
        let base = if self.mode == ProjectMode::Project {
            let name = root_dir.as_ref().and_then(|r| r.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            rows.push(Row { depth: 0, name, path: String::new(), kind: RowKind::Root, excluded: false });
            if !self.expanded.contains("") {
                self.rows = Rc::new(rows);
                return;
            }
            1
        } else {
            0
        };
        let ctx = WalkCtx { expanded: &self.expanded, settings: &self.settings, root: root_dir.as_deref() };
        walk(&self.tree, "", base, &ctx, &mut rows);
        if self.mode == ProjectMode::Project && !self.libraries.is_empty() {
            self.library_rows(&mut rows);
        }
        self.rows = Rc::new(rows);
    }

    /// External Libraries, after the project as in IntelliJ: each library's
    /// files as a tree; file rows carry absolute paths.
    fn library_rows(&self, rows: &mut Vec<Row>) {
        rows.push(Row { depth: 0, name: "External Libraries".into(), path: LIBRARIES_KEY.into(), kind: RowKind::Libraries, excluded: false });
        if !self.expanded.contains(LIBRARIES_KEY) {
            return;
        }
        let ctx = WalkCtx { expanded: &self.expanded, settings: &self.settings, root: None };
        for (ix, (name, root, files)) in self.libraries.iter().enumerate() {
            let key = library_key(ix);
            let open = self.expanded.contains(&key);
            rows.push(Row { depth: 1, name: name.clone(), path: key.clone(), kind: RowKind::Library, excluded: false });
            if !open {
                continue;
            }
            let mut tree = Dir { loaded: true, ..Default::default() };
            for file in files {
                if let Ok(rel) = Path::new(file).strip_prefix(root) {
                    tree.insert(&rel.to_string_lossy().replace('\\', "/"), false);
                }
            }
            let prefix = format!("{key}/");
            let start = rows.len();
            walk(&tree, &prefix, 2, &ctx, rows);
            for row in &mut rows[start..] {
                if row.kind == RowKind::File {
                    row.path = root.join(&row.path[prefix.len()..]).to_string_lossy().into_owned();
                }
            }
        }
    }

    fn select_row(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.get(ix) {
            self.selected = Some(row.path.clone());
            self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
            cx.notify();
        }
    }

    fn selected_ix(&self) -> Option<usize> {
        let selected = self.selected.as_ref()?;
        self.rows.iter().position(|r| &r.path == selected)
    }

    fn toggle(&mut self, path: &str, cx: &mut Context<Self>) {
        if !self.expanded.remove(path) {
            self.expanded.insert(path.to_owned());
            if let Some(root) = self.root(cx) {
                self.load_dir(path, &root);
            }
        }
        self.flatten();
        cx.notify();
    }

    /// A single click with Enable Preview Tab: the workspace shows the file in its preview tab.
    fn preview(&mut self, path: String, cx: &mut Context<Self>) {
        cx.emit(PreviewFile(path));
    }

    fn open(&mut self, path: String, cx: &mut Context<Self>) {
        let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
        cx.emit(OpenTarget(Target { path, line: 0, col: 0, name, label: String::new(), container: None }));
    }

    fn click(&mut self, ix: usize, count: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(ix).cloned() else { return };
        window.focus(&self.focus, cx);
        self.search = None;
        self.selected = Some(row.path.clone());
        match row.kind {
            RowKind::File if count >= 2 => self.open(row.path, cx),
            RowKind::File if self.settings.preview_tab => self.preview(row.path, cx),
            RowKind::File if self.settings.single_click => self.open(row.path, cx),
            // Folders open on a single click; the second click of a double click is ignored.
            RowKind::Dir | RowKind::Root | RowKind::Libraries | RowKind::Library if count == 1 => self.toggle(&row.path, cx),
            _ => {}
        }
        cx.notify();
    }

    /// Select In › Project View, and the Select Opened File button.
    pub fn reveal(&mut self, path: &str, cx: &mut Context<Self>) {
        if crate::index::store::ProjectIndex::is_external(path) {
            self.reveal_library_file(path, cx);
            return;
        }
        if !self.tree_has(path) && self.mode != ProjectMode::Project {
            self.mode = ProjectMode::Project;
            self.rebuild_tree(cx);
        }
        self.expanded.insert(String::new());
        let parts: Vec<&str> = path.split('/').collect();
        let mut prefix = String::new();
        for part in &parts[..parts.len().saturating_sub(1)] {
            prefix = if prefix.is_empty() { (*part).to_owned() } else { format!("{prefix}/{part}") };
            self.expanded.insert(prefix.clone());
        }
        self.selected = Some(path.to_owned());
        self.flatten();
        if let Some(ix) = self.selected_ix() {
            self.scroll.scroll_to_item(ix, ScrollStrategy::Center);
        }
        cx.notify();
    }

    fn reveal_library_file(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(ix) = self.libraries.iter().position(|(_, _, files)| files.binary_search_by(|f| f.as_str().cmp(path)).is_ok()) else { return };
        if self.mode != ProjectMode::Project {
            self.mode = ProjectMode::Project;
            self.rebuild_tree(cx);
        }
        let key = library_key(ix);
        self.expanded.insert(LIBRARIES_KEY.into());
        self.expanded.insert(key.clone());
        if let Ok(rel) = Path::new(path).strip_prefix(&self.libraries[ix].1) {
            let rel = rel.to_string_lossy().replace('\\', "/");
            let mut prefix = key;
            for part in rel.split('/').collect::<Vec<_>>().iter().rev().skip(1).rev() {
                prefix = format!("{prefix}/{part}");
                self.expanded.insert(prefix.clone());
            }
        }
        self.selected = Some(path.to_owned());
        self.flatten();
        if let Some(ix) = self.selected_ix() {
            self.scroll.scroll_to_item(ix, ScrollStrategy::Center);
        }
        cx.notify();
    }

    fn tree_has(&mut self, path: &str) -> bool {
        let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
        self.tree.get_mut(dir).is_some_and(|d| d.files.iter().any(|(f, _)| f == name))
    }

    fn select_opened_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.current_file.clone() {
            self.reveal(&path, cx);
            window.focus(&self.focus, cx);
        }
    }

    fn expand_all(&mut self, cx: &mut Context<Self>) {
        fn collect(dir: &Dir, prefix: &str, out: &mut HashSet<String>) {
            for (name, sub) in &dir.dirs {
                // Excluded folders stay as they are, as in IntelliJ.
                if sub.excluded {
                    continue;
                }
                let path = format!("{prefix}{name}");
                collect(sub, &format!("{path}/"), out);
                out.insert(path);
            }
        }
        let mut all = HashSet::new();
        collect(&self.tree, "", &mut all);
        // Compacted chains are keyed by their full path, which is in `all` too.
        self.expanded.extend(all);
        self.expanded.insert(String::new());
        self.flatten();
        cx.notify();
    }

    fn collapse_all(&mut self, cx: &mut Context<Self>) {
        self.expanded.clear();
        if self.mode == ProjectMode::Project {
            // IntelliJ keeps the project root open.
            self.expanded.insert(String::new());
        }
        self.flatten();
        if let Some(sel) = &self.selected {
            if !self.rows.iter().any(|r| &r.path == sel) {
                let top = sel.split('/').next().unwrap_or_default().to_owned();
                self.selected = self.rows.iter().find(|r| r.path == top || r.path.starts_with(&format!("{top}/"))).map(|r| r.path.clone());
            }
        }
        cx.notify();
    }

    fn set_mode(&mut self, mode: ProjectMode, cx: &mut Context<Self>) {
        self.mode = mode;
        self.search = None;
        self.rebuild_tree(cx);
        if matches!(mode, ProjectMode::OpenFiles | ProjectMode::Changed) {
            // Short lists: shown fully expanded.
            self.expand_all(cx);
        }
        self.flatten();
        cx.notify();
    }

    fn target(&self, row: &Row, cx: &App) -> Option<ProjectTarget> {
        // Library sources have no file actions (they're read-only).
        if row.path.starts_with('\u{1}') || crate::index::store::ProjectIndex::is_external(&row.path) {
            return None;
        }
        let (model, actions, root) = (self.model.clone()?, self.actions.clone()?, self.root(cx)?);
        let is_dir = row.kind != RowKind::File;
        let prefix = format!("{}/", row.path);
        let files = match row.kind {
            RowKind::Root => self.files.to_vec(),
            RowKind::Dir => self.files.iter().filter(|f| f.starts_with(&prefix)).cloned().collect(),
            _ => Vec::new(),
        };
        Some(ProjectTarget { model, root, path: row.path.clone(), is_dir, files, actions, clipboard: self.clipboard.clone() })
    }

    /// New › File / Directory in the selected folder (the root when nothing is selected).
    fn new_entry(&mut self, directory: bool, window: &mut Window, cx: &mut Context<Self>) {
        let row = self
            .selected_ix()
            .and_then(|ix| self.rows.get(ix).cloned())
            .unwrap_or(Row { depth: 0, name: String::new(), path: String::new(), kind: RowKind::Root, excluded: false });
        if let Some(target) = self.target(&row, cx) {
            crate::ui::file_menus::new_entry(&target, directory, window, cx);
        }
    }

    // ---- Speed search ----------------------------------------------------

    fn matches(&self, row: &Row) -> bool {
        let Some(q) = self.search.as_deref().filter(|q| !q.is_empty()) else { return false };
        crate::index::nav::fuzzy_score(q, &row.name).is_some()
    }

    /// Moves to the next (or previous) row matching the speed search, starting at `from`.
    fn jump_match(&mut self, from: usize, forward: bool, cx: &mut Context<Self>) -> bool {
        let n = self.rows.len();
        for step in 0..n {
            let ix = if forward { (from + step) % n } else { (from + n - step % n) % n };
            if self.matches(&self.rows[ix]) {
                self.select_row(ix, cx);
                return true;
            }
        }
        false
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &event.keystroke;
        let m = k.modifiers;
        let ix = self.selected_ix();
        let row = ix.and_then(|ix| self.rows.get(ix).cloned());
        if (m.control || m.platform) && !m.alt && !m.shift && k.key == "f" {
            self.search.get_or_insert_with(String::new);
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if self.search.is_some() {
            match k.key.as_str() {
                "escape" => self.search = None,
                "backspace" => {
                    if let Some(s) = &mut self.search {
                        s.pop();
                    }
                    let from = ix.unwrap_or(0);
                    self.jump_match(from, true, cx);
                }
                "down" => {
                    let from = ix.map_or(0, |i| i + 1);
                    self.jump_match(from, true, cx);
                }
                "up" => {
                    let from = ix.map_or(0, |i| i + self.rows.len().max(1) - 1);
                    self.jump_match(from, false, cx);
                }
                _ => {
                    if let Some(ch) = k.key_char.as_ref().filter(|_| !m.control && !m.alt && !m.platform) {
                        self.search.get_or_insert_with(String::new).push_str(ch);
                        let from = ix.unwrap_or(0);
                        self.jump_match(from, true, cx);
                    } else if k.key == "enter" {
                        self.search = None;
                        self.activate(row, cx);
                    } else {
                        // Any other key closes the popup and does its usual thing.
                        self.search = None;
                        cx.notify();
                        return self.on_key(event, window, cx);
                    }
                }
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }
        let last = self.rows.len().saturating_sub(1);
        match k.key.as_str() {
            "down" => self.select_row(ix.map_or(0, |i| (i + 1).min(last)), cx),
            "up" => self.select_row(ix.map_or(0, |i| i.saturating_sub(1)), cx),
            "home" => self.select_row(0, cx),
            "end" => self.select_row(last, cx),
            "pagedown" => self.select_row(ix.map_or(0, |i| (i + 20).min(last)), cx),
            "pageup" => self.select_row(ix.map_or(0, |i| i.saturating_sub(20)), cx),
            "right" => {
                let (Some(ix), Some(row)) = (ix, row) else { return };
                if row.kind != RowKind::File {
                    if self.expanded.contains(&row.path) {
                        if self.rows.get(ix + 1).is_some_and(|r| r.depth > row.depth) {
                            self.select_row(ix + 1, cx);
                        }
                    } else {
                        self.toggle(&row.path, cx);
                    }
                }
            }
            "left" => {
                let (Some(ix), Some(row)) = (ix, row) else { return };
                if row.kind != RowKind::File && self.expanded.contains(&row.path) {
                    self.toggle(&row.path, cx);
                } else if let Some(parent) = (0..ix).rev().find(|&i| self.rows[i].depth < row.depth) {
                    self.select_row(parent, cx);
                }
            }
            "enter" | "f4" => self.activate(row, cx),
            _ => {
                let Some(ch) = k.key_char.as_ref().filter(|c| !m.control && !m.alt && !m.platform && !c.trim().is_empty()) else { return };
                self.search = Some(ch.clone());
                self.jump_match(ix.unwrap_or(0), true, cx);
                cx.notify();
            }
        }
        cx.stop_propagation();
    }

    fn activate(&mut self, row: Option<Row>, cx: &mut Context<Self>) {
        match row {
            Some(row) if row.kind == RowKind::File => self.open(row.path, cx),
            Some(row) => self.toggle(&row.path, cx),
            None => {}
        }
    }

    // ---- Menus ---------------------------------------------------------

    fn options_menu(
        menu: gpui_kit::component::menu::PopupMenu,
        entity: Entity<Self>,
        window: &mut Window,
        cx: &mut Context<gpui_kit::component::menu::PopupMenu>,
    ) -> gpui_kit::component::menu::PopupMenu {
        let s = Settings::get(cx).project.clone();
        let toggle = |label: &'static str, on: bool, set: fn(&mut ProjectSettings, bool)| {
            PopupMenuItem::new(label).checked(on).on_click(move |_, _, cx| Settings::update(cx, |st| set(&mut st.project, !on)))
        };
        let sort = |label: &'static str, sort: ProjectSort, current: ProjectSort| {
            PopupMenuItem::new(label).checked(sort == current).on_click(move |_, _, cx| Settings::update(cx, |st| st.project.sort = sort))
        };
        let (s1, s2, s3) = (s.clone(), s.clone(), s.clone());
        let search_entity = entity.clone();
        menu.submenu("Behavior", window, cx, move |menu, _, _| {
            menu.item(toggle("Always Select Opened File", s1.autoscroll_from_source, |p, v| p.autoscroll_from_source = v))
                .item(toggle("Open Files with Single Click", s1.single_click, |p, v| p.single_click = v))
                .item(toggle("Enable Preview Tab", s1.preview_tab, |p, v| p.preview_tab = v))
        })
        .submenu("Appearance", window, cx, move |menu, _, _| {
            menu.item(toggle("Show Excluded Files", s2.show_excluded, |p, v| p.show_excluded = v))
                .item(toggle("Compact Middle Packages", s2.compact_middle, |p, v| p.compact_middle = v))
        })
        .submenu("Sort By", window, cx, move |menu, _, _| {
            menu.item(sort("Name", ProjectSort::Name, s3.sort))
                .item(sort("Type", ProjectSort::Type, s3.sort))
                .item(sort("Modification Time", ProjectSort::Modified, s3.sort))
                .separator()
                .item(toggle("Folders Always on Top", s3.folders_on_top, |p, v| p.folders_on_top = v))
        })
        .separator()
        .item(PopupMenuItem::new("Speed Search  (Ctrl+F or any symbol)").on_click(move |_, window, cx| {
            search_entity.update(cx, |this, cx| {
                this.search = Some(String::new());
                window.focus(&this.focus, cx);
                cx.notify();
            })
        }))
    }
}

struct WalkCtx<'a> {
    expanded: &'a HashSet<String>,
    settings: &'a ProjectSettings,
    root: Option<&'a Path>,
}

fn extension(name: &str) -> String {
    name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default()
}

fn mtime(root: Option<&Path>, path: &str) -> SystemTime {
    root.and_then(|r| std::fs::metadata(r.join(path)).ok()).and_then(|m| m.modified().ok()).unwrap_or(SystemTime::UNIX_EPOCH)
}

fn walk(dir: &Dir, prefix: &str, depth: usize, ctx: &WalkCtx, rows: &mut Vec<Row>) {
    // (row, child folder to descend into)
    let mut entries: Vec<(Row, Option<&Dir>)> = Vec::new();
    for (name, sub) in &dir.dirs {
        let (mut label, mut path, mut node) = (name.clone(), format!("{prefix}{name}"), sub);
        // Compact Middle Packages: single-child folder chains become one row ("src/main/java").
        if ctx.settings.compact_middle {
            while node.loaded && node.files.is_empty() && node.dirs.len() == 1 {
                let (n, s) = node.dirs.iter().next().unwrap();
                label = format!("{label}/{n}");
                path = format!("{path}/{n}");
                node = s;
            }
        }
        entries.push((Row { depth, name: label, path, kind: RowKind::Dir, excluded: node.excluded }, Some(node)));
    }
    for (f, excluded) in &dir.files {
        entries.push((Row { depth, name: f.clone(), path: format!("{prefix}{f}"), kind: RowKind::File, excluded: *excluded }, None));
    }
    let on_top = ctx.settings.folders_on_top;
    match ctx.settings.sort {
        ProjectSort::Name => entries.sort_by_cached_key(|(r, _)| (on_top && r.kind == RowKind::File, r.name.to_lowercase())),
        ProjectSort::Type => entries.sort_by_cached_key(|(r, _)| {
            let ext = if r.kind == RowKind::File { extension(&r.name) } else { String::new() };
            (on_top && r.kind == RowKind::File, ext, r.name.to_lowercase())
        }),
        ProjectSort::Modified => entries.sort_by_cached_key(|(r, _)| {
            (on_top && r.kind == RowKind::File, std::cmp::Reverse(mtime(ctx.root, &r.path)), r.name.to_lowercase())
        }),
    }
    for (row, node) in entries {
        let open = node.is_some() && ctx.expanded.contains(&row.path);
        let child_prefix = format!("{}/", row.path);
        rows.push(row);
        if let (true, Some(node)) = (open, node) {
            walk(node, &child_prefix, depth + 1, ctx, rows);
        }
    }
}

/// IntelliJ's olive for files Git ignores.
fn excluded_color(dark: bool) -> Hsla {
    if dark { gpui_kit::rgb(0x8c8c4a).into() } else { gpui_kit::rgb(0x848504).into() }
}

impl Render for ProjectView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let rows = self.rows.clone();
        let selected = self.selected.clone();
        let expanded = self.expanded.clone();
        let status = self.status.clone();
        let changed_dirs = self.changed_dirs.clone();
        let search = self.search.clone().filter(|s| !s.is_empty());
        let root_path: SharedString = self.root(cx).map(|r| r.display().to_string()).unwrap_or_default().into();
        let entity = cx.entity();
        let list = uniform_list(
            "project-files",
            rows.len(),
            cx.processor(move |_, range: Range<usize>, _, cx| {
                let palette = cx.palette().clone();
                range
                    .map(|ix| {
                        let row = &rows[ix];
                        let is_dir = row.kind != RowKind::File;
                        let open = expanded.contains(&row.path);
                        let color = if row.excluded {
                            Some(excluded_color(palette.dark))
                        } else if row.kind == RowKind::File {
                            status.get(&row.path).map(|k| common::status_color(*k, &palette))
                        } else if row.kind == RowKind::Dir && changed_dirs.contains(&row.path) {
                            Some(palette.status_modified)
                        } else {
                            None
                        };
                        let hit = search.as_deref().filter(|q| crate::index::nav::fuzzy_score(q, &row.name).is_some());
                        let name_el = match hit {
                            Some(q) => {
                                let lower = row.name.to_lowercase();
                                let ranges: Vec<Range<usize>> = lower.find(&q.to_lowercase()).map(|s| vec![s..s + q.len()]).unwrap_or_else(|| vec![0..row.name.len()]);
                                div().child(crate::ui::find_popup::highlighted(&row.name, &ranges, &palette)).into_any_element()
                            }
                            None => div().child(row.name.clone()).into_any_element(),
                        };
                        h_flex()
                            .id(ix)
                            .w_full()
                            .h(px(row_height()))
                            .pl(px(8. + row.depth as f32 * 16.))
                            .gap_1()
                            .text_sm()
                            .cursor_pointer()
                            .whitespace_nowrap()
                            .when(selected.as_deref() == Some(row.path.as_str()), |el| el.bg(palette.selection))
                            .hover(|el| el.bg(palette.hover))
                            .child(div().w(px(14.)).flex_shrink_0().when(is_dir, |el| {
                                el.child(common::icon(if open { IconName::ChevronDown } else { IconName::ChevronRight }).text_color(palette.text_secondary))
                            }))
                            .child(
                                common::icon(match row.kind {
                                    RowKind::Root => IconName::FolderGit2,
                                    RowKind::Dir if open => IconName::FolderOpen,
                                    RowKind::Dir => IconName::FolderClosed,
                                    RowKind::File => common::file_icon(&row.path),
                                    RowKind::Libraries => IconName::Layers,
                                    RowKind::Library => IconName::Archive,
                                })
                                .text_color(if row.excluded { excluded_color(palette.dark) } else { palette.text_secondary }),
                            )
                            .child(div().when_some(color, |el, c| el.text_color(c)).when(row.kind == RowKind::Root, |el| el.font_weight(gpui_kit::FontWeight::SEMIBOLD)).child(name_el))
                            .when(row.kind == RowKind::Root, |el| {
                                el.child(div().ml_1().overflow_hidden().text_ellipsis().text_xs().text_color(palette.text_secondary).child(root_path.clone()))
                            })
                            .on_click(cx.listener(move |this, e: &gpui_kit::ClickEvent, window, cx| this.click(ix, e.click_count(), window, cx)))
                            .on_mouse_down(gpui_kit::MouseButton::Right, {
                                let path = row.path.clone();
                                cx.listener(move |this, _, window, cx| {
                                    window.focus(&this.focus, cx);
                                    this.selected = Some(path.clone());
                                    cx.notify();
                                })
                            })
                            .context_menu({
                                // Built when the menu opens: a folder's target lists
                                // every file under it, too costly for each row drawn.
                                let (view, row) = (cx.entity().downgrade(), row.clone());
                                move |m, window, cx| match view.upgrade().and_then(|view| view.read(cx).target(&row, cx)) {
                                    Some(target) => crate::ui::file_menus::project_menu(m, target, window, cx),
                                    None => m,
                                }
                            })
                            .into_any_element()
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.scroll)
        .size_full();
        let mode = self.mode;
        let mode_entity = entity.clone();
        let new_entity = entity.clone();
        let options_entity = entity.clone();
        let current = self.current_file.is_some();
        v_flex()
            .size_full()
            .bg(palette.panel)
            .key_context("ProjectView")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .child(
                h_flex()
                    .h(px(crate::ui::common::header_height()))
                    .px_1()
                    .gap_0p5()
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(
                        Button::new("project-mode")
                            .ghost()
                            .xsmall()
                            .label(mode.label())
                            .icon(IconName::ChevronDown)
                            .dropdown_menu(move |mut menu, _, _| {
                                for m in ProjectMode::ALL {
                                    let entity = mode_entity.clone();
                                    menu = menu.item(PopupMenuItem::new(m.label()).checked(m == mode).on_click(move |_, _, cx| {
                                        entity.update(cx, |this, cx| this.set_mode(m, cx))
                                    }));
                                }
                                menu
                            }),
                    )
                    .child(div().flex_1())
                    .child(tool_button("project-new", IconName::Plus, "New").dropdown_menu(move |menu, _, _| {
                        let (a, b) = (new_entity.clone(), new_entity.clone());
                        menu.item(PopupMenuItem::new("File").on_click(move |_, window, cx| a.update(cx, |this, cx| this.new_entry(false, window, cx))))
                            .item(PopupMenuItem::new("Directory").on_click(move |_, window, cx| b.update(cx, |this, cx| this.new_entry(true, window, cx))))
                    }))
                    .child(
                        tool_button("project-locate", IconName::Crosshair, "Select Opened File")
                            .disabled(!current)
                            .on_click(cx.listener(|this, _, window, cx| this.select_opened_file(window, cx))),
                    )
                    .child(tool_button("project-expand", IconName::ChevronsUpDown, "Expand All").on_click(cx.listener(|this, _, _, cx| this.expand_all(cx))))
                    .child(tool_button("project-collapse", IconName::ChevronsDownUp, "Collapse All").on_click(cx.listener(|this, _, _, cx| this.collapse_all(cx))))
                    .child(
                        tool_button("project-options", IconName::EllipsisVertical, "Options")
                            .dropdown_menu(move |menu, window, cx| Self::options_menu(menu, options_entity.clone(), window, cx)),
                    ),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(list)
                    .when_some(self.search.clone(), |el, q| {
                        let found = q.is_empty() || self.rows.iter().any(|r| self.matches(r));
                        el.child(
                            h_flex()
                                .absolute()
                                .top_0()
                                .left_0()
                                .right_0()
                                .h(px(24.))
                                .px_2()
                                .gap_1()
                                .text_sm()
                                .bg(Hsla { a: 1.0, ..palette.panel })
                                .border_b_1()
                                .border_color(palette.border)
                                .shadow_sm()
                                .child(common::icon(IconName::Search).text_color(palette.text_secondary))
                                .child(div().text_color(if found { palette.text } else { palette.status_unversioned }).child(if q.is_empty() { "Search for:".to_owned() } else { q })),
                        )
                    }),
            )
    }
}
