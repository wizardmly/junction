//! Settings › Version Control › Directory Mappings: the project's Git
//! roots, detected ones plus folders added by hand, each removable.

use std::path::PathBuf;
use crate::ui::as_icons as icons;

use gpui_kit::component::{
    Disableable as _, Sizable as _, WindowExt as _, h_flex, checkbox::Checkbox,
    button::{Button, ButtonVariants as _},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, PathPromptOptions, Render, Styled as _, Window,
    InteractiveElement as _, StatefulInteractiveElement as _, div, prelude::FluentBuilder as _, px,
};

use crate::git::roots;
use crate::model::RepoModel;
use crate::theme::ActivePalette as _;

pub struct MappingsView {
    model: Entity<RepoModel>,
    project: PathBuf,
    /// The mapped roots (the project's own repository first), each with
    /// whether it is a repository at all (a mapping can go stale).
    registered: Vec<(PathBuf, bool)>,
    /// Repositories under the project that aren't roots (detection off, or removed).
    unregistered: Vec<PathBuf>,
    auto: bool,
    /// The selected row: (unregistered, its index in that list).
    selected: Option<(bool, usize)>,
    error: Option<String>,
}

impl MappingsView {
    fn scan(&mut self) {
        let roots = roots::detect(&self.project);
        let mut registered: Vec<(PathBuf, bool)> = roots.iter().map(|p| (p.clone(), true)).collect();
        registered.extend(roots::invalid(&self.project).into_iter().map(|p| (p, false)));
        self.unregistered = roots::scan_nested(&self.project).into_iter().filter(|p| !roots.contains(p)).collect();
        self.registered = registered;
        self.auto = roots::load_mappings(&self.project).auto;
        self.selected = None;
    }

    fn changed(&mut self, result: anyhow::Result<()>, cx: &mut Context<Self>) {
        self.error = result.err().map(|e| e.to_string());
        self.scan();
        self.model.update(cx, |m, cx| m.rescan_roots(cx));
        cx.notify();
    }

    /// IntelliJ's "Directory (project)" form: the project's own root as an
    /// absolute path, a nested one relative to it.
    fn label(&self, path: &PathBuf) -> String {
        match path.strip_prefix(&self.project) {
            Ok(rel) if rel.as_os_str().is_empty() => self.project.display().to_string(),
            Ok(rel) => format!("{} ({})", rel.display(), self.project.display()),
            Err(_) => path.display().to_string(),
        }
    }

    /// `+`: registers the selected unregistered root, else picks a folder.
    fn add(&mut self, cx: &mut Context<Self>) {
        if let Some((true, ix)) = self.selected {
            if let Some(path) = self.unregistered.get(ix).cloned() {
                let result = roots::register(&self.project, &[path], true);
                return self.changed(result, cx);
            }
        }
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
                if !this.registered.iter().any(|(p, _)| *p == path) {
                    let result = roots::register(&this.project, &[path], true);
                    this.changed(result, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// `−`: unmaps the selected root (the project's own one stays).
    fn remove(&mut self, cx: &mut Context<Self>) {
        let Some((false, ix)) = self.selected else { return };
        let Some((path, _)) = self.registered.get(ix).cloned() else { return };
        if path == self.project {
            return;
        }
        let result = roots::register(&self.project, &[path], false);
        self.changed(result, cx);
    }
}

impl Render for MappingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let selected = self.selected;
        let row = |id: usize, key: (bool, usize), label: String, vcs: &'static str, dim: bool, broken: bool, cx: &mut Context<Self>| {
            let is_selected = selected == Some(key);
            h_flex()
                .id(("mapping-row", id))
                .h(px(28.))
                .px_2()
                .text_sm()
                .when(is_selected, |el| el.bg(palette.selection))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.selected = Some(key);
                    cx.notify();
                }))
                .child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .when(dim, |el| el.text_color(palette.text_secondary))
                        .when(broken, |el| el.text_color(palette.status_conflict))
                        .child(label),
                )
                .child(div().w(px(90.)).when(dim, |el| el.text_color(palette.text_secondary)).child(vcs))
        };
        let mut table = v_flex().child(
            h_flex()
                .h(px(26.))
                .px_2()
                .text_sm()
                .border_b_1()
                .border_color(palette.border)
                .child(div().flex_1().child("Directory"))
                .child(div().w(px(90.)).border_l_1().border_color(palette.border).pl_2().child("VCS")),
        );
        for (ix, (path, valid)) in self.registered.clone().into_iter().enumerate() {
            let label = self.label(&path);
            table = table.child(row(ix, (false, ix), label, "Git", false, !valid, cx));
        }
        if !self.unregistered.is_empty() {
            table = table.child(div().h(px(26.)).px_2().pt_1().text_sm().text_color(palette.text_secondary).child("Unregistered roots:"));
            for (ix, path) in self.unregistered.clone().into_iter().enumerate() {
                let label = self.label(&path);
                table = table.child(row(1000 + ix, (true, ix), label, "Git", true, false, cx));
            }
        }
        let can_remove = matches!(selected, Some((false, ix)) if self.registered.get(ix).is_some_and(|(p, _)| *p != self.project));
        let auto = self.auto;
        v_flex()
            .gap_2()
            .child(
                v_flex()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(palette.border)
                    .child(
                        h_flex()
                            .gap_1()
                            .p_1()
                            .border_b_1()
                            .border_color(palette.border)
                            .child(Button::new("mapping-add").ghost().xsmall().icon(icons::ADD).tooltip("Add").on_click(cx.listener(|this, _, _, cx| this.add(cx))))
                            .child(
                                Button::new("mapping-remove")
                                    .ghost()
                                    .xsmall()
                                    .icon(icons::REMOVE)
                                    .tooltip("Remove")
                                    .disabled(!can_remove)
                                    .on_click(cx.listener(|this, _, _, cx| this.remove(cx))),
                            ),
                    )
                    .child(div().id("mappings-table").min_h(px(220.)).max_h(px(360.)).overflow_y_scroll().child(table)),
            )
            .child(
                Checkbox::new("mapping-auto").label("Enable automatic mapping detection").checked(auto).on_change(cx.listener(|this, value: &bool, _, cx| {
                    let result = roots::set_auto(&this.project, *value);
                    this.changed(result, cx);
                })),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(palette.text_secondary)
                    .child("Git repositories found under the project become roots automatically; when off, they are offered in a notification."),
            )
            .when_some(self.error.clone(), |el, e| el.child(div().text_sm().text_color(palette.status_conflict).child(e)))
    }
}

impl MappingsView {
    /// The project's roots (Settings › Version Control › Directory Mappings).
    pub fn new(model: Entity<RepoModel>, cx: &App) -> Option<Self> {
        let project = model.read(cx).project_root().map(PathBuf::from)?;
        let mut view = MappingsView { model, project, registered: Vec::new(), unregistered: Vec::new(), auto: true, selected: None, error: None };
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
