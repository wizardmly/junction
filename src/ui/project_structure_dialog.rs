//! File › Project Structure… (Android Studio's Project Structure dialog)
//! with its SDK Location, Project and Modules pages: the Android SDK
//! location, the Android Gradle Plugin and Gradle versions, and each
//! module's Properties and Default Config. OK writes the changed values
//! back where they're written (the build scripts, the version catalog,
//! `gradle-wrapper.properties`, `local.properties`) and syncs, as AS does.
//! A value written as an expression the dialog can't rewrite is shown
//! greyed out.

use std::path::PathBuf;

use gpui_kit::component::{
    Sizable as _, WindowExt as _, h_flex,
    button::Button,
    input::{Input, InputState},
    v_flex,
};
use gpui_kit::{App, AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, PathPromptOptions, Render, StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px};

use crate::domain::project_structure::{DEFAULT_CONFIG, MODULE_PROPERTIES, Property};
use crate::domain::run::ModuleInfo;
use crate::infra::project_files::DiskFiles;
use crate::theme::ActivePalette as _;
use crate::ui::dialogs;
use crate::usecase::project_structure::{self as usecase, Edits, Structure};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    SdkLocation,
    Project,
    Modules,
}

const PAGES: [(Page, &str); 3] = [(Page::SdkLocation, "SDK Location"), (Page::Project, "Project"), (Page::Modules, "Modules")];

/// The labels of a module's rows, Properties then Default Config.
fn labels() -> impl Iterator<Item = &'static str> {
    MODULE_PROPERTIES.iter().chain(DEFAULT_CONFIG).map(|(label, _)| *label)
}

pub struct ProjectStructureView {
    build_root: PathBuf,
    structure: Structure,
    page: Page,
    module: usize,
    sdk_dir: Entity<InputState>,
    agp: Entity<InputState>,
    gradle: Entity<InputState>,
    /// The selected module's rows, in `labels()` order.
    fields: Vec<Entity<InputState>>,
    /// Each module's values as edited, in `labels()` order.
    values: Vec<Vec<String>>,
    error: Option<String>,
}

impl ProjectStructureView {
    fn row(&self, module: usize, ix: usize) -> Option<&Property> {
        let page = self.structure.modules.get(module)?;
        page.properties.iter().chain(&page.default_config).nth(ix)
    }

    /// The module's rows into the fields: its value, or the expression
    /// that can't be edited here.
    fn load_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for ix in 0..self.fields.len() {
            let editable = self.row(self.module, ix).is_some_and(|p| p.site.is_some());
            let text = match self.row(self.module, ix) {
                Some(p) if p.site.is_none() => p.expression.clone().unwrap_or_default(),
                _ => self.values.get(self.module).and_then(|v| v.get(ix)).cloned().unwrap_or_default(),
            };
            self.fields[ix].update(cx, |s, cx| {
                s.set_value(text, window, cx);
                s.set_placeholder(if editable { "" } else { "Not set" }, window, cx);
            });
        }
    }

    fn store_fields(&mut self, cx: &App) {
        if let Some(values) = self.values.get_mut(self.module) {
            for (ix, field) in self.fields.iter().enumerate() {
                values[ix] = field.read(cx).value().to_string();
            }
        }
    }

    fn select_module(&mut self, module: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.store_fields(cx);
        self.module = module;
        self.load_fields(window, cx);
        cx.notify();
    }

    /// What differs from what's written.
    fn edits(&mut self, cx: &App) -> Edits {
        self.store_fields(cx);
        let changed = |input: &Entity<InputState>, old: Option<&str>| {
            let new = input.read(cx).value().trim().to_owned();
            (old.map(str::trim) != Some(new.as_str()) && !(old.is_none() && new.is_empty())).then_some(new)
        };
        let mut edits = Edits {
            sdk_dir: changed(&self.sdk_dir, Some(&self.structure.sdk_dir)),
            agp: self.structure.agp.as_deref().and_then(|old| changed(&self.agp, Some(old))),
            gradle: self.structure.gradle.as_deref().and_then(|old| changed(&self.gradle, Some(old))),
            modules: Vec::new(),
        };
        for (module, values) in self.values.iter().enumerate() {
            for (ix, label) in labels().enumerate() {
                let Some(p) = self.row(module, ix).filter(|p| p.site.is_some()) else { continue };
                if values[ix].trim() != p.value {
                    edits.modules.push((module, label, values[ix].trim().to_owned()));
                }
            }
        }
        edits
    }

    /// Writes the changes and syncs; false (with the problem shown) when
    /// they can't be written.
    fn save(&mut self, cx: &mut Context<Self>) -> bool {
        let edits = self.edits(cx);
        if edits.is_empty() {
            return true;
        }
        let files = DiskFiles { root: self.build_root.clone(), repository: None };
        match usecase::apply(&files, &self.structure, &edits) {
            Ok(_) => {
                let build = self.build_root.clone();
                crate::ui::sync_service::SyncService::global(cx).update(cx, |s, cx| s.sync(build, cx));
                true
            }
            Err(e) => {
                self.error = Some(e);
                cx.notify();
                false
            }
        }
    }

    fn browse_sdk(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Select Android SDK Location".into()) });
        let input = self.sdk_dir.clone();
        cx.spawn(async move |_, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            cx.update(|cx| {
                if let Some(window) = cx.active_window() {
                    window.update(cx, |_, window, cx| input.update(cx, |s, cx| s.set_value(path.display().to_string(), window, cx))).ok();
                }
            });
        })
        .detach();
    }

    fn render_page(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let palette = cx.palette().clone();
        let label = |text: &'static str| div().w(px(190.)).flex_none().text_sm().child(text);
        let hint = |text: &'static str| div().text_xs().text_color(palette.text_secondary).child(text);
        match self.page {
            Page::SdkLocation => v_flex()
                .gap_2()
                .child(
                    h_flex()
                        .gap_2()
                        .child(label("Android SDK location:"))
                        .child(div().flex_1().child(Input::new(&self.sdk_dir).small()))
                        .child(Button::new("psd-sdk-browse").small().label("…").on_click(cx.listener(|this, _, _, cx| this.browse_sdk(cx)))),
                )
                .child(hint("Saved in local.properties (sdk.dir). An empty location removes it."))
                .into_any_element(),
            Page::Project => {
                let version_row = |name: &'static str, input: &Entity<InputState>, known: bool, missing: &'static str| {
                    v_flex()
                        .gap_1()
                        .child(h_flex().gap_2().child(label(name)).child(div().flex_1().child(Input::new(input).small().disabled(!known))))
                        .when(!known, |el| el.child(hint(missing)))
                };
                v_flex()
                    .gap_3()
                    .child(version_row("Android Gradle Plugin Version", &self.agp, self.structure.agp.is_some(), "The plugin's version isn't written in the root build script, the settings script or the version catalog."))
                    .child(version_row("Gradle Version", &self.gradle, self.structure.gradle.is_some(), "The project has no gradle/wrapper/gradle-wrapper.properties."))
                    .into_any_element()
            }
            Page::Modules => {
                let entity = cx.entity();
                let modules = v_flex()
                    .id("psd-modules")
                    .w(px(150.))
                    .flex_none()
                    .h_full()
                    .overflow_y_scroll()
                    .border_r_1()
                    .border_color(palette.border)
                    .children(self.structure.modules.iter().enumerate().map(|(ix, m)| {
                        let selected = ix == self.module;
                        let e = entity.clone();
                        div()
                            .id(("psd-module", ix))
                            .px_2()
                            .py_0p5()
                            .text_sm()
                            .when(selected, |el| el.bg(palette.selection))
                            .hover(|el| el.bg(palette.hover))
                            .child(m.path.trim_start_matches(':').to_owned().replace(':', "/"))
                            .on_click(move |_, window, cx| e.update(cx, |this, cx| this.select_module(ix, window, cx)))
                    }));
                let heading = |text: &'static str| div().pt_1().text_sm().font_weight(FontWeight::SEMIBOLD).child(text);
                let field = |ix: usize, name: &'static str| {
                    let row = self.row(self.module, ix);
                    let editable = row.is_some_and(|p| p.site.is_some());
                    // A catalog version shows where it's written.
                    let reference = row.filter(|p| p.site.is_some()).and_then(|p| p.expression.as_deref()).map(|e| e.split(".get(").next().unwrap_or(e).split(".toInt").next().unwrap_or(e).to_owned());
                    h_flex()
                        .gap_2()
                        .child(label(name))
                        .child(div().flex_1().child(Input::new(&self.fields[ix]).small().disabled(!editable)))
                        .when_some(reference, |el, r| el.child(div().w(px(150.)).flex_none().text_xs().text_color(palette.text_secondary).truncate().child(r)))
                };
                let script = self.structure.modules.get(self.module).and_then(|m| m.script.clone());
                let n = MODULE_PROPERTIES.len();
                let settings = v_flex()
                    .id("psd-module-settings")
                    .flex_1()
                    .h_full()
                    .overflow_y_scroll()
                    .px_3()
                    .gap_2()
                    .when_none(&script, |el| el.child(hint("This module has no build script.")))
                    .child(heading("Properties"))
                    .children(MODULE_PROPERTIES.iter().enumerate().map(|(ix, (name, _))| field(ix, name)))
                    .child(heading("Default Config"))
                    .children(DEFAULT_CONFIG.iter().enumerate().map(|(ix, (name, _))| field(n + ix, name)));
                h_flex().size_full().items_start().child(modules).child(settings).into_any_element()
            }
        }
    }
}

impl Render for ProjectStructureView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let nav = v_flex().w(px(140.)).flex_none().h_full().py_1().border_r_1().border_color(palette.border).children(PAGES.iter().map(|(page, name)| {
            let (page, selected) = (*page, *page == self.page);
            div()
                .id(*name)
                .px_2()
                .py_0p5()
                .text_sm()
                .when(selected, |el| el.bg(palette.selection))
                .hover(|el| el.bg(palette.hover))
                .child(*name)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.page = page;
                    cx.notify();
                }))
        }));
        v_flex()
            .gap_2()
            .child(h_flex().h(px(420.)).items_start().border_1().border_color(palette.border).child(nav).child(div().flex_1().h_full().p_2().child(self.render_page(cx))))
            .child(div().h(px(16.)).text_xs().text_color(palette.status_conflict).children(self.error.clone()))
    }
}

/// Opens Project Structure over a build's modules.
pub fn open(build_root: PathBuf, modules: Vec<ModuleInfo>, window: &mut Window, cx: &mut App) {
    let rel = |dir: &PathBuf| dir.strip_prefix(&build_root).map(|d| d.to_string_lossy().replace('\\', "/")).unwrap_or_default();
    let paths: Vec<(String, String)> = modules.iter().filter(|m| m.path != ":").map(|m| (m.path.clone(), rel(&m.dir))).collect();
    let files = DiskFiles { root: build_root.clone(), repository: None };
    let structure = usecase::read(&files, &paths, crate::infra::catalog_files::load(&build_root, &catalog_candidates(&build_root)));
    let view = cx.new(|cx| {
        let mut input = |cx: &mut Context<ProjectStructureView>, value: String| cx.new(|cx| InputState::new(window, cx).default_value(value));
        let sdk_dir = input(cx, structure.sdk_dir.clone());
        let agp = input(cx, structure.agp.clone().unwrap_or_default());
        let gradle = input(cx, structure.gradle.clone().unwrap_or_default());
        let fields = labels().map(|_| input(cx, String::new())).collect();
        let values = structure.modules.iter().map(|m| m.properties.iter().chain(&m.default_config).map(|p| p.value.clone()).collect()).collect();
        let mut view = ProjectStructureView { build_root, structure, page: Page::SdkLocation, module: 0, sdk_dir, agp, gradle, fields, values, error: None };
        view.load_fields(window, cx);
        view
    });
    let focus = view.read(cx).sdk_dir.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let view_ok = view.clone();
        dialog.title("Project Structure").w(px(820.)).child(view.clone()).footer(dialogs::footer("OK")).on_ok(move |_, _, cx| view_ok.update(cx, |v, cx| v.save(cx)))
    });
    dialogs::focus_input(&focus, window, cx);
}

/// The files a build's catalogs can come from: its settings scripts and
/// what's in `gradle/`.
fn catalog_candidates(build_root: &std::path::Path) -> Vec<String> {
    let mut out: Vec<String> = ["settings.gradle.kts", "settings.gradle"].iter().filter(|f| build_root.join(f).is_file()).map(|f| f.to_string()).collect();
    if let Ok(entries) = std::fs::read_dir(build_root.join("gradle")) {
        out.extend(entries.flatten().map(|e| format!("gradle/{}", e.file_name().to_string_lossy())));
    }
    out
}
