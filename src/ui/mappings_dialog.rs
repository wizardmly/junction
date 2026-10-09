//! Settings › Version Control › Directory Mappings: the project's Git
//! roots, detected ones plus folders added by hand, each removable.

use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Icon, Sizable as _, WindowExt as _, h_flex,
    button::{Button, ButtonVariants as _},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, PathPromptOptions, Render, Styled as _, Window,
    div, prelude::FluentBuilder as _, px,
};

use crate::git::roots::{self, Mappings};
use crate::model::RepoModel;
use crate::theme::ActivePalette as _;

struct MappingsView {
    model: Entity<RepoModel>,
    project: PathBuf,
    detected: Vec<PathBuf>,
    mappings: Mappings,
    error: Option<String>,
}

impl MappingsView {
    fn relative(&self, path: &PathBuf) -> PathBuf {
        path.strip_prefix(&self.project).map(PathBuf::from).unwrap_or_else(|_| path.clone())
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let result = crate::git::Repository::discover(&self.project, Default::default())
            .and_then(|repo| roots::save_mappings(&repo, &self.mappings));
        self.error = result.err().map(|e| e.to_string());
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
                let relative = this.relative(&path);
                this.mappings.removed.retain(|p| *p != relative);
                if !this.detected.contains(&path) && !this.mappings.added.contains(&relative) {
                    this.mappings.added.push(relative);
                }
                this.save(cx);
            })
            .ok();
        })
        .detach();
    }
}

impl Render for MappingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mut rows = v_flex().gap_px();
        let added: Vec<PathBuf> = self.mappings.added.iter().map(|p| self.project.join(p)).collect();
        for (ix, path) in self.detected.iter().chain(added.iter()).enumerate() {
            let relative = self.relative(path);
            let is_project = *path == self.project;
            let removed = self.mappings.removed.contains(&relative);
            let source = if is_project { "Project" } else if ix < self.detected.len() { "Detected" } else { "Added" };
            let toggle_relative = relative.clone();
            let is_added = ix >= self.detected.len();
            rows = rows.child(
                h_flex()
                    .h(px(28.))
                    .px_2()
                    .gap_2()
                    .text_sm()
                    .rounded(px(3.))
                    .child(Icon::new(IconName::FolderGit2).small().text_color(palette.text_secondary))
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .when(removed, |el| el.line_through().text_color(palette.text_secondary))
                            .child(if is_project { "<Project>".to_owned() } else { relative.display().to_string() }),
                    )
                    .child(div().w(px(70.)).text_xs().text_color(palette.text_secondary).child(source))
                    .when(!is_project, |el| {
                        el.child(
                            Button::new(("mapping-toggle", ix))
                                .xsmall()
                                .outline()
                                .label(if removed { "Restore" } else { "Remove" })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if is_added {
                                        this.mappings.added.retain(|p| *p != toggle_relative);
                                    } else if !this.mappings.removed.contains(&toggle_relative) {
                                        this.mappings.removed.push(toggle_relative.clone());
                                    } else {
                                        this.mappings.removed.retain(|p| *p != toggle_relative);
                                    }
                                    this.save(cx);
                                })),
                        )
                    }),
            );
        }
        v_flex()
            .gap_2()
            .child(div().text_sm().text_color(palette.text_secondary).child(format!("Git roots of {}", self.project.display())))
            .child(div().p_1().rounded(px(4.)).border_1().border_color(palette.border).child(rows))
            .child(h_flex().child(Button::new("mapping-add").small().outline().icon(IconName::Plus).label("Add Root…").on_click(cx.listener(|this, _, _, cx| this.add(cx)))))
            .when_some(self.error.clone(), |el, e| el.child(div().text_sm().text_color(palette.status_conflict).child(e)))
    }
}

pub fn directory_mappings(model: Entity<RepoModel>, window: &mut Window, cx: &mut App) {
    let Some(project) = model.read(cx).project_root().map(PathBuf::from) else { return };
    let Ok(repository) = crate::git::Repository::discover(&project, Default::default()) else { return };
    let mut detected = vec![project.clone()];
    detected.extend(roots::scan_nested(&project));
    let mappings = roots::load_mappings(&repository);
    let view = cx.new(|_| MappingsView { model, project, detected, mappings, error: None });
    window.open_dialog(cx, move |dialog, _, _| {
        dialog.title("Directory Mappings").w(px(600.)).child(view.clone()).footer(
            gpui_kit::component::dialog::DialogFooter::new()
                .child(gpui_kit::component::dialog::DialogClose::new().child(Button::new("mappings-close").label("Close").primary())),
        )
    });
}
