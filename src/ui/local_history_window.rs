//! Local History › Show History, as IntelliJ's: a window with the recorded
//! versions (and the project's labels) on the left, newest first, and the
//! selected version against the current text on the right. For a folder
//! (or the whole project) the middle lists the files that changed since
//! the selected moment. Revert puts the selected version back.

use std::path::PathBuf;

use gpui_kit::component::{
    Icon, Sizable as _, TitleBar, h_flex,
    button::{Button, ButtonVariants as _},
    v_flex,
};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _,
    WeakEntity, Window, WindowBounds, WindowOptions, div, prelude::FluentBuilder as _, px, size,
};

use crate::domain::local_history::{HistoryStore as _, Kind, Revision};
use crate::infra::local_history_store::DiskHistory;
use crate::model::RepoModel;
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::diff_view::{DiffSource, DiffView};
use crate::ui::workspace::{APP_NAME, Workspace};
use crate::usecase::local_history;

/// What a history window shows.
#[derive(Clone)]
pub enum Scope {
    File(String),
    /// A folder relative to the project; empty for the whole project.
    Folder(String),
}

impl Scope {
    fn path(&self) -> &str {
        match self {
            Scope::File(p) | Scope::Folder(p) => p,
        }
    }
}

pub struct LocalHistoryWindow {
    root: PathBuf,
    scope: Scope,
    model: Entity<RepoModel>,
    revisions: Vec<Revision>,
    selected: Option<usize>,
    /// A folder's files changed since the selected moment, with their text
    /// then; and the one whose diff shows.
    changes: Vec<(String, String)>,
    shown: Option<usize>,
    diff: Entity<DiffView>,
    workspace: WeakEntity<Workspace>,
    home: AnyWindowHandle,
}

/// Opens the history of a file or folder of the project at `root`, with
/// the version recorded at `select` picked, when given.
pub fn open(root: PathBuf, scope: Scope, select: Option<i64>, model: Entity<RepoModel>, workspace: WeakEntity<Workspace>, home: AnyWindowHandle, cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(1200.), px(760.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(600.), px(300.))),
        app_id: Some("junction".into()),
        ..TitleBar::window_options()
    };
    let opened = gpui_kit::open_window(options, cx, move |window, cx| {
        window.set_window_title(&format!("Local History for {} – {APP_NAME}", name_of(&root, &scope)));
        cx.new(|cx| {
            let diff = cx.new(DiffView::new);
            diff.update(cx, |view, _| view.set_in_window());
            let mut this = LocalHistoryWindow { root, scope, model, revisions: Vec::new(), selected: None, changes: Vec::new(), shown: None, diff, workspace, home };
            this.load(select, cx);
            this
        })
    });
    if let Err(error) = opened {
        eprintln!("cannot open Local History: {error}");
    }
}

fn name_of(root: &std::path::Path, scope: &Scope) -> String {
    match scope.path() {
        "" => root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        path => path.rsplit('/').next().unwrap_or(path).to_owned(),
    }
}

impl LocalHistoryWindow {
    fn store(&self) -> Option<DiskHistory> {
        DiskHistory::for_project(&self.root)
    }

    fn folder(&self) -> Option<&str> {
        match &self.scope {
            Scope::Folder(dir) => Some(dir),
            Scope::File(_) => None,
        }
    }

    fn load(&mut self, select: Option<i64>, cx: &mut Context<Self>) {
        self.revisions = self
            .store()
            .map(|s| match &self.scope {
                Scope::File(path) => local_history::history(&s, path),
                Scope::Folder(dir) => local_history::folder_history(&s, dir),
            })
            .unwrap_or_default();
        // The newest version is the current text; IntelliJ selects the one before.
        let first = self.revisions.iter().position(|r| r.kind != Kind::Label);
        let before = self.revisions.iter().enumerate().skip(first.map_or(0, |i| i + 1)).find(|(_, r)| r.kind != Kind::Label).map(|(i, _)| i).or(first);
        let pick = select.and_then(|t| self.revisions.iter().position(|r| r.time == t)).or(before);
        if let Some(ix) = pick {
            self.select(ix, cx);
        }
        cx.notify();
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(row) = self.revisions.get(ix).cloned() else { return };
        match self.folder().map(str::to_owned) {
            Some(dir) => {
                self.selected = Some(ix);
                self.changes = self.store().map(|s| local_history::changed_since(&s, &dir, row.time)).unwrap_or_default();
                self.shown = None;
                if self.changes.is_empty() {
                    self.diff.update(cx, |view, cx| view.clear(cx));
                } else {
                    self.show(0, cx);
                }
            }
            None => {
                // A label shows the file as it was then: the next older version.
                let Some(version) = self.revisions.iter().skip(ix).find(|r| r.kind != Kind::Label).cloned() else { return };
                self.selected = Some(ix);
                let text = self.store().and_then(|s| s.content(&version.content)).unwrap_or_default();
                self.compare(version.path, text, version.time, cx);
            }
        }
        cx.notify();
    }

    /// A folder's changed file against its current text.
    fn show(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some((path, text)) = self.changes.get(ix).cloned() else { return };
        let Some(time) = self.selected.and_then(|s| self.revisions.get(s)).map(|r| r.time) else { return };
        self.shown = Some(ix);
        self.compare(path, text, time, cx);
        cx.notify();
    }

    fn compare(&mut self, path: String, text: String, time: i64, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let Some((repository, own)) = model.route(&path).or_else(|| model.project_repository().cloned().map(|r| (r, path.clone()))) else { return };
        let title = time_label(time);
        self.diff.update(cx, |view, cx| view.show(repository, DiffSource::Clipboard { path: own, text, title }, cx));
    }

    /// Revert: the selected version replaces the file (a folder's changed
    /// files); the text each had is kept first.
    fn revert(&mut self, cx: &mut Context<Self>) {
        let Some(ix) = self.selected else { return };
        let Some(store) = self.store() else { return };
        let (time, files): (i64, Vec<(String, String)>) = match &self.scope {
            Scope::Folder(_) => (self.revisions[ix].time, self.changes.clone()),
            Scope::File(path) => {
                let Some(version) = self.revisions.iter().skip(ix).find(|r| r.kind != Kind::Label).cloned() else { return };
                let Some(text) = store.content(&version.content) else { return };
                (version.time, vec![(path.clone(), text)])
            }
        };
        let label = format!("Before reverting to {}", time_label(time));
        let mut reverted = Vec::new();
        for (path, text) in files {
            let file = self.root.join(&path);
            if let Ok(current) = std::fs::read_to_string(&file) {
                let _ = local_history::record(&store, &path, &current, Kind::Before, &label);
            }
            if std::fs::write(&file, &text).is_ok() {
                let _ = local_history::record(&store, &path, &text, Kind::Saved, "");
                reverted.push(path);
            }
        }
        if reverted.is_empty() {
            return;
        }
        let workspace = self.workspace.clone();
        self.home
            .update(cx, |_, window, cx| {
                workspace.update(cx, |ws, cx| ws.files_changed_on_disk(reverted, window, cx)).ok();
            })
            .ok();
        self.load(None, cx);
    }

    fn label(r: &Revision) -> String {
        match r.kind {
            Kind::Saved => String::new(),
            Kind::Before | Kind::Label => r.label.clone(),
        }
    }

    /// A folder row's file, relative to the folder.
    fn file_label(&self, path: &str) -> String {
        match self.folder() {
            Some(dir) if !dir.is_empty() => path.strip_prefix(dir).map(|p| p.trim_start_matches('/')).unwrap_or(path).to_owned(),
            _ => path.to_owned(),
        }
    }
}

impl Render for LocalHistoryWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let name = name_of(&self.root, &self.scope);
        let folder = self.folder().is_some();
        let mut list = v_flex().id("lh-list").size_full().overflow_y_scroll().text_sm();
        for (ix, r) in self.revisions.iter().enumerate() {
            let selected = self.selected == Some(ix);
            let label = Self::label(r);
            let is_label = r.kind == Kind::Label;
            let file = (folder && !is_label).then(|| self.file_label(&r.path));
            list = list.child(
                v_flex()
                    .id(("lh-row", ix))
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .when(selected, |el| el.bg(palette.selection))
                    .when(!selected, |el| el.hover(|el| el.bg(palette.hover)))
                    .child(
                        h_flex()
                            .gap_1()
                            .when(is_label, |el| el.child(Icon::new(icons::TAG).xsmall()))
                            .child(time_label(r.time))
                            .when(ix == 0 && !is_label, |el| el.child(div().text_color(palette.text_secondary).child("Current"))),
                    )
                    .when_some(file, |el, file| el.child(div().text_color(palette.text_secondary).overflow_hidden().whitespace_nowrap().text_ellipsis().child(file)))
                    .when(!label.is_empty(), |el| el.child(div().text_color(if is_label { palette.text } else { palette.text_secondary }).when(is_label, |el| el.font_weight(FontWeight::SEMIBOLD)).child(label)))
                    .on_click(cx.listener(move |this, _, _, cx| this.select(ix, cx))),
            );
        }
        if self.revisions.is_empty() {
            let empty = if folder { "No local changes recorded here yet. Versions are kept each time a file is saved." } else { "No local changes recorded for this file yet. Versions are kept each time it is saved." };
            list = list.child(div().p_4().text_color(palette.text_secondary).child(empty));
        }
        let files = folder.then(|| {
            let mut files = v_flex().id("lh-files").size_full().overflow_y_scroll().text_sm();
            for (ix, (path, _)) in self.changes.iter().enumerate() {
                let shown = self.shown == Some(ix);
                files = files.child(
                    h_flex()
                        .id(("lh-file", ix))
                        .px_2()
                        .h(px(24.))
                        .gap_1()
                        .cursor_pointer()
                        .when(shown, |el| el.bg(palette.selection))
                        .when(!shown, |el| el.hover(|el| el.bg(palette.hover)))
                        .child(Icon::new(icons::FILE_TEXT).xsmall())
                        .child(div().text_color(palette.status_modified).overflow_hidden().whitespace_nowrap().text_ellipsis().child(self.file_label(path)))
                        .on_click(cx.listener(move |this, _, _, cx| this.show(ix, cx))),
                );
            }
            if self.changes.is_empty() && self.selected.is_some() {
                files = files.child(div().p_3().text_color(palette.text_secondary).child("No differences"));
            }
            div().w(px(240.)).flex_none().border_r_1().border_color(palette.border).child(files)
        });
        let revert_tip = if folder { "Revert the changed files to the selected version" } else { "Revert the file to the selected version" };
        v_flex()
            .size_full()
            .bg(palette.window)
            .text_color(palette.text)
            .child(TitleBar::new().child(div().text_sm().child(format!("Local History for {name}"))))
            .child(
                h_flex()
                    .h(px(32.))
                    .px_2()
                    .gap_1()
                    .bg(palette.panel)
                    .border_b_1()
                    .border_color(palette.border)
                    .child(Button::new("lh-revert").ghost().small().icon(Icon::new(icons::DIFF_REVERT)).label("Revert").tooltip(revert_tip).on_click(cx.listener(|this, _, _, cx| this.revert(cx)))),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .bg(palette.panel)
                    .child(div().w(px(260.)).flex_none().border_r_1().border_color(palette.border).child(list))
                    .children(files)
                    .child(div().flex_1().min_w_0().child(self.diff.clone())),
            )
    }
}

/// Recent Changes (Alt+Shift+C): the project's latest recorded changes;
/// picking one opens its history at that moment (a label: the project's).
pub fn recent_changes(root: PathBuf, model: Entity<RepoModel>, workspace: WeakEntity<Workspace>, window: &mut Window, cx: &mut App) {
    use gpui_kit::component::WindowExt as _;
    use gpui_kit::SharedString;
    let rows: Vec<Revision> = DiskHistory::for_project(&root).map(|s| local_history::folder_history(&s, "")).unwrap_or_default().into_iter().take(40).collect();
    let home = window.window_handle();
    window.open_dialog(cx, move |dialog, _, cx| {
        let palette = cx.palette().clone();
        let mut list = v_flex().id("recent-changes").max_h(px(440.)).overflow_y_scroll().text_sm();
        for (ix, r) in rows.iter().enumerate() {
            let (root, model, workspace, row) = (root.clone(), model.clone(), workspace.clone(), r.clone());
            let what = match r.kind {
                Kind::Label => r.label.clone(),
                _ => r.path.rsplit('/').next().unwrap_or(&r.path).to_owned(),
            };
            let detail = match r.kind {
                Kind::Label => String::new(),
                _ => r.path.rsplit_once('/').map(|(dir, _)| dir.to_owned()).unwrap_or_default(),
            };
            list = list.child(
                h_flex()
                    .id(SharedString::from(format!("recent-{ix}")))
                    .px_2()
                    .h(px(26.))
                    .gap_2()
                    .cursor_pointer()
                    .hover(|el| el.bg(palette.hover))
                    .child(div().w(px(150.)).flex_none().text_color(palette.text_secondary).child(time_label(r.time)))
                    .when(r.kind == Kind::Label, |el| el.child(Icon::new(icons::TAG).xsmall()))
                    .child(div().when(r.kind == Kind::Label, |el| el.font_weight(FontWeight::SEMIBOLD)).child(what))
                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(palette.text_secondary).child(detail))
                    .on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        let scope = if row.kind == Kind::Label { Scope::Folder(String::new()) } else { Scope::File(row.path.clone()) };
                        open(root.clone(), scope, Some(row.time), model.clone(), workspace.clone(), home, cx);
                    }),
            );
        }
        if rows.is_empty() {
            list = list.child(div().p_3().text_color(palette.text_secondary).child("No local changes recorded yet. Versions are kept each time a file is saved."));
        }
        dialog.title("Recent Changes").w(px(640.)).child(list)
    });
}

/// Local History › Put Label…: names this moment of the whole project.
pub fn put_label(root: PathBuf, window: &mut Window, cx: &mut App) {
    use gpui_kit::component::WindowExt as _;
    use gpui_kit::component::input::{Input, InputState};
    let name = cx.new(|cx| InputState::new(window, cx).placeholder("Label name"));
    let focus = name.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let (name, root) = (name.clone(), root.clone());
        dialog
            .title("Put Label")
            .w(px(420.))
            .child(Input::new(&name))
            .on_ok(move |_, _, cx| {
                let label = name.read(cx).value().trim().to_owned();
                if label.is_empty() {
                    return false;
                }
                if let Some(store) = DiskHistory::for_project(&root) {
                    let _ = local_history::put_label(&store, &label);
                }
                true
            })
            .footer(crate::ui::dialogs::footer("OK"))
    });
    crate::ui::dialogs::focus_input(&focus, window, cx);
}

/// Records a file's text on disk as its newest version (in the background):
/// after a save, or when it opens with no history yet (`only_new`), so the
/// first save has something to compare with.
pub fn record_file(root: PathBuf, path: String, only_new: bool) {
    std::thread::spawn(move || {
        let Some(store) = DiskHistory::for_project(&root) else { return };
        if only_new && local_history::has_history(&store, &path) {
            return;
        }
        if let Ok(text) = std::fs::read_to_string(root.join(&path)) {
            let _ = local_history::record(&store, &path, &text, Kind::Saved, "");
        }
    });
}

/// Once per project opening: drops versions older than the kept working days.
pub fn prune(root: PathBuf) {
    std::thread::spawn(move || {
        if let Some(store) = DiskHistory::for_project(&root) {
            let _ = local_history::prune(&store);
        }
    });
}

/// "Today 14:32:05", "Yesterday 09:15:40", or the date: saves come seconds apart.
fn time_label(millis: i64) -> String {
    let Some(time) = chrono::DateTime::from_timestamp_millis(millis).map(|t| t.with_timezone(&chrono::Local)) else { return String::new() };
    let today = chrono::Local::now().date_naive();
    match (today - time.date_naive()).num_days() {
        0 => format!("Today {}", time.format("%H:%M:%S")),
        1 => format!("Yesterday {}", time.format("%H:%M:%S")),
        _ => time.format("%Y-%m-%d %H:%M:%S").to_string(),
    }
}
