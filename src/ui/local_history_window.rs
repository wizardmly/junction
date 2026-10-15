//! Local History › Show History, as IntelliJ's: a window with the file's
//! recorded versions (and the project's labels) on the left, newest first,
//! and the selected version against the current file on the right. Revert
//! puts the selected version back.

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
use crate::git::Repository;
use crate::infra::local_history_store::DiskHistory;
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::diff_view::{DiffSource, DiffView};
use crate::ui::workspace::{APP_NAME, Workspace};
use crate::usecase::local_history;

pub struct LocalHistoryWindow {
    root: PathBuf,
    /// The file relative to the project, and inside its repository.
    path: String,
    own: String,
    repository: Repository,
    revisions: Vec<Revision>,
    selected: Option<usize>,
    diff: Entity<DiffView>,
    workspace: WeakEntity<Workspace>,
    home: AnyWindowHandle,
}

/// Opens the history of `path` (relative to `root`; `own` inside `repository`).
pub fn open(root: PathBuf, path: String, repository: Repository, own: String, workspace: WeakEntity<Workspace>, home: AnyWindowHandle, cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(1200.), px(760.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(600.), px(300.))),
        app_id: Some("junction".into()),
        ..TitleBar::window_options()
    };
    let opened = gpui_kit::open_window(options, cx, move |window, cx| {
        let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
        window.set_window_title(&format!("Local History for {name} – {APP_NAME}"));
        cx.new(|cx| {
            let diff = cx.new(DiffView::new);
            diff.update(cx, |view, _| view.set_in_window());
            let mut this = LocalHistoryWindow { root, path, own, repository, revisions: Vec::new(), selected: None, diff, workspace, home };
            this.load(cx);
            this
        })
    });
    if let Err(error) = opened {
        eprintln!("cannot open Local History: {error}");
    }
}

impl LocalHistoryWindow {
    fn store(&self) -> Option<DiskHistory> {
        DiskHistory::for_project(&self.root)
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        self.revisions = self.store().map(|s| local_history::history(&s, &self.path)).unwrap_or_default();
        // The newest version is the current text; IntelliJ selects the one before.
        let first = self.revisions.iter().position(|r| r.kind != Kind::Label);
        let pick = self.revisions.iter().enumerate().skip(first.map_or(0, |i| i + 1)).find(|(_, r)| r.kind != Kind::Label).map(|(i, _)| i).or(first);
        if let Some(ix) = pick {
            self.select(ix, cx);
        }
        cx.notify();
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        // A label shows the file as it was then: the next older version.
        let Some(version) = self.revisions.iter().skip(ix).find(|r| r.kind != Kind::Label).cloned() else { return };
        self.selected = Some(ix);
        let text = self.store().and_then(|s| s.content(&version.content)).unwrap_or_default();
        let title = time_label(version.time);
        let (repository, own) = (self.repository.clone(), self.own.clone());
        self.diff.update(cx, |view, cx| view.show(repository, DiffSource::Clipboard { path: own, text, title }, cx));
        cx.notify();
    }

    /// Revert: the selected version replaces the file; the text it had is kept first.
    fn revert(&mut self, cx: &mut Context<Self>) {
        let Some(ix) = self.selected else { return };
        let Some(version) = self.revisions.iter().skip(ix).find(|r| r.kind != Kind::Label).cloned() else { return };
        let Some(store) = self.store() else { return };
        let Some(text) = store.content(&version.content) else { return };
        let file = self.root.join(&self.path);
        if let Ok(current) = std::fs::read_to_string(&file) {
            let label = format!("Before reverting to {}", time_label(version.time));
            let _ = local_history::record(&store, &self.path, &current, Kind::Before, &label);
        }
        if std::fs::write(&file, &text).is_err() {
            return;
        }
        let _ = local_history::record(&store, &self.path, &text, Kind::Saved, "");
        let path = self.path.clone();
        let workspace = self.workspace.clone();
        self.home
            .update(cx, |_, window, cx| {
                workspace.update(cx, |ws, cx| ws.files_changed_on_disk(vec![path], window, cx)).ok();
            })
            .ok();
        self.load(cx);
    }

    fn label(r: &Revision) -> String {
        match r.kind {
            Kind::Saved => String::new(),
            Kind::Before | Kind::Label => r.label.clone(),
        }
    }
}

impl Render for LocalHistoryWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let name = self.path.rsplit('/').next().unwrap_or(&self.path).to_owned();
        let mut list = v_flex().id("lh-list").size_full().overflow_y_scroll().text_sm();
        for (ix, r) in self.revisions.iter().enumerate() {
            let selected = self.selected == Some(ix);
            let label = Self::label(r);
            let is_label = r.kind == Kind::Label;
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
                    .when(!label.is_empty(), |el| el.child(div().text_color(if is_label { palette.text } else { palette.text_secondary }).when(is_label, |el| el.font_weight(FontWeight::SEMIBOLD)).child(label)))
                    .on_click(cx.listener(move |this, _, _, cx| this.select(ix, cx))),
            );
        }
        if self.revisions.is_empty() {
            list = list.child(div().p_4().text_color(palette.text_secondary).child("No local changes recorded for this file yet. Versions are kept each time it is saved."));
        }
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
                    .child(Button::new("lh-revert").ghost().small().icon(Icon::new(icons::DIFF_REVERT)).label("Revert").tooltip("Revert the file to the selected version").on_click(cx.listener(|this, _, _, cx| this.revert(cx)))),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .bg(palette.panel)
                    .child(div().w(px(260.)).flex_none().border_r_1().border_color(palette.border).child(list))
                    .child(div().flex_1().min_w_0().child(self.diff.clone())),
            )
    }
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
