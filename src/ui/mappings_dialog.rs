//! Settings › Version Control › Directory Mappings: the project's Git
//! roots, detected ones plus folders added by hand, each removable.

use std::path::PathBuf;
use crate::ui::as_icons as icons;

use gpui_kit::component::{
    Icon, Sizable as _, WindowExt as _, h_flex,
    button::{Button, ButtonVariants as _},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, PathPromptOptions, Render, Styled as _, Window,
    div, prelude::FluentBuilder as _, px,
};

use crate::git::roots;
use crate::model::RepoModel;
use crate::theme::ActivePalette as _;

pub struct MappingsView {
    model: Entity<RepoModel>,
    project: PathBuf,
    /// Registered roots (the project's own repository first).
    registered: Vec<PathBuf>,
    /// Repositories under the project that aren't registered, and whether
    /// each was ignored (the balloon no longer offers it).
    unregistered: Vec<(PathBuf, bool)>,
    error: Option<String>,
}

impl MappingsView {
    fn relative(&self, path: &PathBuf) -> PathBuf {
        path.strip_prefix(&self.project).map(PathBuf::from).unwrap_or_else(|_| path.clone())
    }

    fn scan(&mut self) {
        self.registered = roots::detect(&self.project);
        let offered = roots::unregistered(&self.project);
        self.unregistered = roots::scan_nested(&self.project)
            .into_iter()
            .filter(|p| !self.registered.contains(p))
            .map(|p| {
                let ignored = !offered.contains(&p);
                (p, ignored)
            })
            .collect();
    }

    fn set(&mut self, path: PathBuf, add: bool, cx: &mut Context<Self>) {
        self.error = roots::register(&self.project, &[path], add).err().map(|e| e.to_string());
        self.scan();
        self.model.update(cx, |m, cx| m.rescan_roots(cx));
        cx.notify();
    }

    fn add(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Add Git Root".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            this.update(cx, |this, cx| {
                if !path.join(".git").exists() {
                    this.error = Some(format!("{} is not a Git repository root", path.display()));
                    cx.notify();
                    return;
                }
                if !this.registered.contains(&path) {
                    this.set(path, true, cx);
                }
            })
            .ok();
        })
        .detach();
    }
}

impl Render for MappingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let row = |ix: usize, label: String, note: &'static str, button: Option<(&'static str, PathBuf, bool)>, cx: &mut Context<Self>| {
            h_flex()
                .h(px(28.))
                .px_2()
                .gap_2()
                .text_sm()
                .rounded(px(3.))
                .child(Icon::new(icons::MODULE).small().text_color(palette.text_secondary))
                .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().child(label))
                .child(div().w(px(70.)).text_xs().text_color(palette.text_secondary).child(note))
                .when_some(button, |el, (label, path, add)| {
                    el.child(
                        Button::new(("mapping-toggle", ix))
                            .xsmall()
                            .outline()
                            .label(label)
                            .on_click(cx.listener(move |this, _, _, cx| this.set(path.clone(), add, cx))),
                    )
                })
        };
        let mut registered = v_flex().gap_px();
        for (ix, path) in self.registered.clone().into_iter().enumerate() {
            let is_project = path == self.project;
            let label = if is_project { "<Project>".to_owned() } else { self.relative(&path).display().to_string() };
            registered = registered.child(row(ix, label, if is_project { "Project" } else { "Git" }, (!is_project).then(|| ("Remove", path.clone(), false)), cx));
        }
        let mut unregistered = v_flex().gap_px();
        for (ix, (path, ignored)) in self.unregistered.clone().into_iter().enumerate() {
            let label = self.relative(&path).display().to_string();
            unregistered = unregistered.child(row(1000 + ix, label, if ignored { "Ignored" } else { "Unregistered" }, Some(("Add", path.clone(), true)), cx));
        }
        v_flex()
            .gap_2()
            .child(div().text_sm().text_color(palette.text_secondary).child(format!("Git roots of {}", self.project.display())))
            .child(div().p_1().rounded(px(4.)).border_1().border_color(palette.border).child(registered))
            .when(!self.unregistered.is_empty(), |el| {
                el.child(div().text_sm().text_color(palette.text_secondary).child("Unregistered roots"))
                    .child(div().p_1().rounded(px(4.)).border_1().border_color(palette.border).child(unregistered))
            })
            .child(h_flex().child(Button::new("mapping-add").small().outline().icon(icons::ADD).label("Add Root…").on_click(cx.listener(|this, _, _, cx| this.add(cx)))))
            .when_some(self.error.clone(), |el, e| el.child(div().text_sm().text_color(palette.status_conflict).child(e)))
    }
}

impl MappingsView {
    /// The project's roots (Settings › Version Control › Directory Mappings).
    pub fn new(model: Entity<RepoModel>, cx: &App) -> Option<Self> {
        let project = model.read(cx).project_root().map(PathBuf::from)?;
        let mut view = MappingsView { model, project, registered: Vec::new(), unregistered: Vec::new(), error: None };
        view.scan();
        Some(view)
    }
}

pub fn directory_mappings(model: Entity<RepoModel>, window: &mut Window, cx: &mut App) {
    let Some(view) = MappingsView::new(model, cx) else { return };
    let view = cx.new(|_| view);
    window.open_dialog(cx, move |dialog, _, _| {
        dialog.title("Directory Mappings").w(px(600.)).child(view.clone()).footer(
            gpui_kit::component::dialog::DialogFooter::new()
                .child(gpui_kit::component::dialog::DialogClose::new().child(Button::new("mappings-close").label("Close").primary())),
        )
    });
}
