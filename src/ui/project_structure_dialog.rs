//! File › Project Structure… (Android Studio's Project Structure dialog)
//! with its SDK Location, Project and Modules pages: the Android SDK
//! location, the Android Gradle Plugin and Gradle versions, and each
//! module's Properties and Default Config. OK writes the changed values
//! back where they're written (the build scripts, the version catalog,
//! `gradle-wrapper.properties`, `local.properties`) and syncs, as AS does.
//! A value written as an expression the dialog can't rewrite is shown
//! greyed out. The Dependencies page lists each module's declared
//! dependencies, edits their version and configuration, removes them and
//! adds library and module dependencies.

use std::path::PathBuf;

use gpui_kit::component::{
    Disableable as _, Icon, Sizable as _, WindowExt as _, h_flex,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::{App, AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, PathPromptOptions, Render, StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px};

use crate::domain::dependencies::{Addition, CONFIGURATIONS, Change, Notation};
use crate::domain::project_structure::{BUILD_TYPE, DEFAULT_CONFIG, FLAVOR, MODULE_PROPERTIES, Property, Variant};
use crate::domain::run::ModuleInfo;
use crate::infra::project_files::DiskFiles;
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::common::tool_button;
use crate::ui::dialogs;
use crate::usecase::project_structure::{self as usecase, DependencyDraft, Edits, Structure, VariantKind};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    SdkLocation,
    Project,
    Modules,
    Dependencies,
    BuildVariants,
}

const PAGES: [(Page, &str); 5] =
    [(Page::SdkLocation, "SDK Location"), (Page::Project, "Project"), (Page::Modules, "Modules"), (Page::Dependencies, "Dependencies"), (Page::BuildVariants, "Build Variants")];

/// A tab's rows.
fn variant_rows(kind: VariantKind) -> &'static [(&'static str, &'static str)] {
    match kind {
        VariantKind::BuildType => BUILD_TYPE,
        VariantKind::Flavor => FLAVOR,
    }
}

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
    /// The modules' paths and directories, to read the structure again
    /// after the dependencies are written.
    paths: Vec<(String, String)>,
    draft: DependencyDraft,
    /// The selected row of the Dependencies page.
    dependency: Option<usize>,
    /// Its Requested Version.
    version: Entity<InputState>,
    /// Add Library Dependency: the coordinate and its configuration.
    library: Entity<InputState>,
    configuration: String,
    /// Build Variants: the tab, the selected build type or flavor, its
    /// rows' fields, and every one's values as edited (by module, tab and
    /// name, in the tab's row order).
    variant_kind: VariantKind,
    variant: usize,
    variant_fields: Vec<Entity<InputState>>,
    variant_values: std::collections::HashMap<(usize, VariantKind, String), Vec<String>>,
    error: Option<String>,
    _subscriptions: Vec<gpui_kit::Subscription>,
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
            variants: Vec::new(),
        };
        self.store_variant(cx);
        for ((module, kind, name), values) in &self.variant_values {
            for ((label, _), value) in variant_rows(*kind).iter().zip(values) {
                let Some(p) = self.structure.variant_property(*module, *kind, name, label).filter(|p| p.site.is_some()) else { continue };
                if value.trim() != p.value {
                    edits.variants.push((*module, *kind, name.clone(), label, value.trim().to_owned()));
                }
            }
        }
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
        if edits.is_empty() && !self.draft.is_changed() {
            return true;
        }
        let files = DiskFiles { root: self.build_root.clone(), repository: None };
        let written = if self.draft.is_changed() {
            // The properties are found again in the scripts as written now.
            self.draft.write(&files).map(|_| usecase::read(&files, &self.paths, crate::infra::catalog_files::load(&self.build_root, &catalog_candidates(&self.build_root))))
        } else {
            Ok(self.structure.clone())
        };
        match written.and_then(|structure| if edits.is_empty() { Ok(Vec::new()) } else { usecase::apply(&files, &structure, &edits) }) {
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

    fn variants_of(&self, module: usize, kind: VariantKind) -> &[Variant] {
        match (self.structure.modules.get(module), kind) {
            (Some(m), VariantKind::BuildType) => &m.build_types,
            (Some(m), VariantKind::Flavor) => &m.flavors,
            (None, _) => &[],
        }
    }

    fn current_variant(&self) -> Option<&Variant> {
        self.variants_of(self.module, self.variant_kind).get(self.variant)
    }

    /// The selected build type's or flavor's rows into the fields.
    fn load_variant(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let variant = self.current_variant().cloned();
        let key = variant.as_ref().map(|v| (self.module, self.variant_kind, v.name.clone()));
        for (ix, field) in self.variant_fields.clone().iter().enumerate() {
            let row = variant.as_ref().and_then(|v| v.properties.get(ix));
            let editable = row.is_some_and(|p| p.site.is_some());
            let text = match row {
                Some(p) if p.site.is_none() => p.expression.clone().unwrap_or_default(),
                Some(_) => key.as_ref().and_then(|k| self.variant_values.get(k)).and_then(|v| v.get(ix)).cloned().unwrap_or_default(),
                None => String::new(),
            };
            field.update(cx, |s, cx| {
                s.set_value(text, window, cx);
                s.set_placeholder(if editable { "" } else { "Not set" }, window, cx);
            });
        }
    }

    fn store_variant(&mut self, cx: &App) {
        let Some(name) = self.current_variant().map(|v| v.name.clone()) else { return };
        let n = variant_rows(self.variant_kind).len();
        let values: Vec<String> = self.variant_fields.iter().take(n).map(|f| f.read(cx).value().to_string()).collect();
        self.variant_values.insert((self.module, self.variant_kind, name), values);
    }

    fn select_variant(&mut self, module: usize, kind: VariantKind, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.store_variant(cx);
        if module != self.module {
            self.select_module(module, window, cx);
        }
        self.variant_kind = kind;
        self.variant = index;
        self.load_variant(window, cx);
        cx.notify();
    }

    fn render_variants(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let palette = cx.palette().clone();
        let entity = cx.entity();
        let kind = self.variant_kind;
        let modules = v_flex()
            .id("psd-var-modules")
            .w(px(130.))
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .border_r_1()
            .border_color(palette.border)
            .children(self.structure.modules.iter().enumerate().map(|(ix, m)| {
                let e = entity.clone();
                div()
                    .id(("psd-var-module", ix))
                    .px_2()
                    .py_0p5()
                    .text_sm()
                    .when(ix == self.module, |el| el.bg(palette.selection))
                    .hover(|el| el.bg(palette.hover))
                    .child(m.path.trim_start_matches(':').to_owned().replace(':', "/"))
                    .on_click(move |_, window, cx| e.update(cx, |this, cx| this.select_variant(ix, kind, 0, window, cx)))
            }));
        let module = self.module;
        let tabs = h_flex().gap_3().px_2().pb_1().border_b_1().border_color(palette.border).children([(VariantKind::BuildType, "Build Types"), (VariantKind::Flavor, "Flavors")].into_iter().map(|(k, name)| {
            let e = entity.clone();
            div()
                .id(name)
                .text_sm()
                .when(k == kind, |el| el.font_weight(FontWeight::SEMIBOLD).border_b_2().border_color(palette.accent))
                .child(name)
                .on_click(move |_, window, cx| e.update(cx, |this, cx| this.select_variant(module, k, 0, window, cx)))
        }));
        let list = self.variants_of(self.module, kind);
        let names = v_flex().id("psd-var-names").w(px(150.)).flex_none().h_full().overflow_y_scroll().border_r_1().border_color(palette.border).children(list.iter().enumerate().map(|(ix, v)| {
            let e = entity.clone();
            div()
                .id(("psd-var", ix))
                .px_2()
                .py_0p5()
                .text_sm()
                .when(ix == self.variant, |el| el.bg(palette.selection))
                .hover(|el| el.bg(palette.hover))
                .child(v.name.clone())
                .on_click(move |_, window, cx| e.update(cx, |this, cx| this.select_variant(module, kind, ix, window, cx)))
        }));
        let label = |text: &'static str| div().w(px(170.)).flex_none().text_sm().child(text);
        let fields = match self.current_variant() {
            Some(v) => v_flex()
                .id("psd-var-fields")
                .flex_1()
                .h_full()
                .overflow_y_scroll()
                .px_3()
                .gap_2()
                .children(variant_rows(kind).iter().enumerate().map(|(ix, (name, _))| {
                    let editable = v.properties.get(ix).is_some_and(|p| p.site.is_some());
                    h_flex().gap_2().child(label(name)).child(div().flex_1().child(Input::new(&self.variant_fields[ix]).small().disabled(!editable)))
                }))
                .into_any_element(),
            None => div()
                .px_3()
                .text_sm()
                .text_color(palette.text_secondary)
                .child(match kind {
                    VariantKind::BuildType => "This module's build script declares no build types (debug and release are Gradle's defaults).",
                    VariantKind::Flavor => "This module's build script declares no product flavors.",
                })
                .into_any_element(),
        };
        let body = h_flex().flex_1().min_h_0().items_start().child(names).child(fields);
        h_flex().size_full().items_start().child(modules).child(v_flex().flex_1().min_w_0().h_full().gap_1().child(tabs).child(body)).into_any_element()
    }

    /// A row of the Dependencies page picked: its version into the field.
    fn select_dependency(&mut self, index: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.dependency = index;
        let version = index.and_then(|ix| self.draft.declared(self.module).into_iter().nth(ix)).and_then(|d| d.version_site.is_some().then_some(d.version).flatten()).unwrap_or_default();
        self.version.update(cx, |s, cx| s.set_value(version, window, cx));
        cx.notify();
    }

    fn select_dependency_module(&mut self, module: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.select_module(module, window, cx);
        self.select_dependency(None, window, cx);
    }

    fn change_dependency(&mut self, change: Change, cx: &mut Context<Self>) {
        let Some(ix) = self.dependency else { return };
        match self.draft.change(self.module, ix, &change) {
            Ok(()) => self.error = None,
            Err(e) => self.error = Some(e),
        }
        if change == Change::Remove {
            self.dependency = None;
        }
        cx.notify();
    }

    /// The Requested Version as typed, once it reads as a version.
    fn version_changed(&mut self, cx: &mut Context<Self>) {
        let Some(dep) = self.dependency.and_then(|ix| self.draft.declared(self.module).into_iter().nth(ix)) else { return };
        let typed = self.version.read(cx).value().trim().to_owned();
        if dep.version_site.is_none() || dep.version.as_deref() == Some(typed.as_str()) || crate::domain::dependencies::check(&Change::Version(typed.clone())).is_err() {
            return;
        }
        self.change_dependency(Change::Version(typed), cx);
    }

    fn add_dependency(&mut self, addition: Addition, window: &mut Window, cx: &mut Context<Self>) {
        match self.draft.add(self.module, &self.configuration, &addition) {
            Ok(()) => {
                self.error = None;
                if matches!(addition, Addition::Library(_)) {
                    self.library.update(cx, |s, cx| s.set_value("", window, cx));
                }
                let last = self.draft.declared(self.module).len().checked_sub(1);
                self.select_dependency(last, window, cx);
            }
            Err(e) => self.error = Some(e),
        }
        cx.notify();
    }

    fn render_dependencies(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let palette = cx.palette().clone();
        let entity = cx.entity();
        let modules = v_flex()
            .id("psd-dep-modules")
            .w(px(130.))
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .border_r_1()
            .border_color(palette.border)
            .children(self.structure.modules.iter().enumerate().map(|(ix, m)| {
                let e = entity.clone();
                div()
                    .id(("psd-dep-module", ix))
                    .px_2()
                    .py_0p5()
                    .text_sm()
                    .when(ix == self.module, |el| el.bg(palette.selection))
                    .hover(|el| el.bg(palette.hover))
                    .child(m.path.trim_start_matches(':').to_owned().replace(':', "/"))
                    .on_click(move |_, window, cx| e.update(cx, |this, cx| this.select_dependency_module(ix, window, cx)))
            }));
        let declared = self.draft.declared(self.module);
        let has_script = self.draft.has_script(self.module);
        let others: Vec<String> = self.structure.modules.iter().enumerate().filter(|(ix, _)| *ix != self.module).map(|(_, m)| m.path.clone()).collect();
        let (add, remove) = (entity.clone(), entity.clone());
        let toolbar = h_flex()
            .gap_1()
            .child(div().flex_1().text_sm().font_weight(FontWeight::SEMIBOLD).child("Declared Dependencies"))
            .child(
                Button::new("psd-dep-add-module")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(icons::ADD))
                    .tooltip("Add Module Dependency")
                    .disabled(!has_script || others.is_empty())
                    .dropdown_menu(move |mut menu, _, _| {
                        menu = menu.label("Module Dependency");
                        for path in &others {
                            let (p, e) = (path.clone(), add.clone());
                            menu = menu.item(PopupMenuItem::new(path.clone()).on_click(move |_, window, cx| {
                                let p = p.clone();
                                e.update(cx, |this, cx| this.add_dependency(Addition::Module(p), window, cx))
                            }));
                        }
                        menu
                    }),
            )
            .child(tool_button("psd-dep-remove", icons::REMOVE, "Remove Dependency").disabled(self.dependency.is_none()).on_click(move |_, _, cx| remove.update(cx, |this, cx| this.change_dependency(Change::Remove, cx))));
        let rows = v_flex().id("psd-dep-rows").flex_1().min_h_0().overflow_y_scroll().border_1().border_color(palette.border).children(declared.iter().enumerate().map(|(ix, d)| {
            let e = entity.clone();
            let icon = match d.notation {
                Notation::Module(_) => icons::MODULE_ANDROID,
                _ => icons::LIBRARY,
            };
            h_flex()
                .id(("psd-dep", ix))
                .px_2()
                .py_0p5()
                .gap_2()
                .text_sm()
                .when(self.dependency == Some(ix), |el| el.bg(palette.selection))
                .hover(|el| el.bg(palette.hover))
                .child(Icon::new(icon).small())
                .child(div().flex_1().truncate().child(d.text.clone()))
                .child(div().w(px(150.)).flex_none().text_color(palette.text_secondary).truncate().child(d.configuration.clone()))
                .on_click(move |_, window, cx| e.update(cx, |this, cx| this.select_dependency(Some(ix), window, cx)))
        }));
        let label = |text: &'static str| div().w(px(130.)).flex_none().text_sm().child(text);
        let selected = self.dependency.and_then(|ix| declared.get(ix));
        let pick = entity.clone();
        let configuration_menu = |id: &'static str, current: String, on_pick: std::rc::Rc<dyn Fn(String, &mut Window, &mut App)>| {
            Button::new(id).small().label(current.clone()).child(Icon::new(icons::CHEVRON_DOWN).xsmall()).dropdown_menu(move |mut menu, _, _| {
                for name in CONFIGURATIONS {
                    let pick = on_pick.clone();
                    menu = menu.item(PopupMenuItem::new(*name).checked(current == *name).on_click(move |_, window, cx| pick(name.to_string(), window, cx)));
                }
                menu
            })
        };
        let details = v_flex()
            .gap_2()
            .when_some(selected, |el, d| {
                let e = pick.clone();
                el.child(div().text_sm().font_weight(FontWeight::SEMIBOLD).truncate().child(d.written.clone()))
                    .child(h_flex().gap_2().child(label("Requested Version")).child(div().flex_1().child(Input::new(&self.version).small().disabled(d.version_site.is_none()))))
                    .child(h_flex().gap_2().child(label("Configuration")).child(configuration_menu(
                        "psd-dep-configuration",
                        d.configuration.clone(),
                        std::rc::Rc::new(move |name, _, cx| e.update(cx, |this, cx| this.change_dependency(Change::Configuration(name), cx))),
                    )))
            })
            .when(selected.is_none(), |el| el.child(div().text_sm().text_color(palette.text_secondary).child(if has_script { "Select a dependency to edit its version and configuration." } else { "This module has no build script." })));
        let (add_library, set_configuration) = (entity.clone(), entity.clone());
        let add_row = v_flex()
            .gap_1()
            .pt_2()
            .border_t_1()
            .border_color(palette.border)
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Add Library Dependency"))
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.library).small().disabled(!has_script)))
                    .child(configuration_menu(
                        "psd-dep-new-configuration",
                        self.configuration.clone(),
                        std::rc::Rc::new(move |name, _, cx| {
                            set_configuration.update(cx, |this, cx| {
                                this.configuration = name;
                                cx.notify();
                            })
                        }),
                    ))
                    .child(Button::new("psd-dep-add").small().label("Add").disabled(!has_script).on_click(move |_, window, cx| {
                        add_library.update(cx, |this, cx| {
                            let coordinate = this.library.read(cx).value().to_string();
                            this.add_dependency(Addition::Library(coordinate), window, cx)
                        })
                    })),
            );
        let middle = v_flex().flex_1().min_w_0().overflow_hidden().h_full().px_2().gap_1().child(toolbar).child(rows);
        let right = v_flex().w(px(280.)).flex_none().h_full().pl_2().gap_2().border_l_1().border_color(palette.border).child(div().flex_1().child(details)).child(add_row);
        h_flex().size_full().items_start().child(modules).child(middle).child(right).into_any_element()
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
            Page::Dependencies => self.render_dependencies(cx),
            Page::BuildVariants => self.render_variants(cx),
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
            .child(h_flex().h(px(420.)).items_start().border_1().border_color(palette.border).child(nav).child(div().flex_1().min_w_0().overflow_hidden().h_full().p_2().child(self.render_page(cx))))
            .child(div().h(px(16.)).text_xs().text_color(palette.status_conflict).children(self.error.clone()))
    }
}

/// Opens Project Structure over a build's modules.
pub fn open(build_root: PathBuf, modules: Vec<ModuleInfo>, window: &mut Window, cx: &mut App) {
    let rel = |dir: &PathBuf| dir.strip_prefix(&build_root).map(|d| d.to_string_lossy().replace('\\', "/")).unwrap_or_default();
    let paths: Vec<(String, String)> = modules.iter().filter(|m| m.path != ":").map(|m| (m.path.clone(), rel(&m.dir))).collect();
    let files = DiskFiles { root: build_root.clone(), repository: None };
    let catalogs = crate::infra::catalog_files::load(&build_root, &catalog_candidates(&build_root));
    let structure = usecase::read(&files, &paths, catalogs.clone());
    let draft = DependencyDraft::read(&files, &structure, catalogs);
    let view = cx.new(|cx| {
        let mut input = |cx: &mut Context<ProjectStructureView>, value: String| cx.new(|cx| InputState::new(window, cx).default_value(value));
        let sdk_dir = input(cx, structure.sdk_dir.clone());
        let agp = input(cx, structure.agp.clone().unwrap_or_default());
        let gradle = input(cx, structure.gradle.clone().unwrap_or_default());
        let fields = labels().map(|_| input(cx, String::new())).collect();
        let values = structure.modules.iter().map(|m| m.properties.iter().chain(&m.default_config).map(|p| p.value.clone()).collect()).collect();
        let version = input(cx, String::new());
        let variant_fields = (0..BUILD_TYPE.len().max(FLAVOR.len())).map(|_| input(cx, String::new())).collect::<Vec<_>>();
        let library = cx.new(|cx| InputState::new(window, cx).placeholder("group:artifact:version"));
        let subscriptions = vec![cx.subscribe(&version, |this: &mut ProjectStructureView, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.version_changed(cx);
            }
        })];
        let variant_values = structure
            .modules
            .iter()
            .enumerate()
            .flat_map(|(ix, m)| {
                let tab = |kind, list: &Vec<Variant>| list.iter().map(move |v| ((ix, kind, v.name.clone()), v.properties.iter().map(|p| p.value.clone()).collect::<Vec<_>>())).collect::<Vec<_>>();
                let mut out = tab(VariantKind::BuildType, &m.build_types);
                out.extend(tab(VariantKind::Flavor, &m.flavors));
                out
            })
            .collect();
        let mut view = ProjectStructureView {
            build_root,
            structure,
            page: Page::SdkLocation,
            module: 0,
            sdk_dir,
            agp,
            gradle,
            fields,
            values,
            paths,
            draft,
            dependency: None,
            version,
            library,
            configuration: "implementation".into(),
            variant_kind: VariantKind::BuildType,
            variant: 0,
            variant_fields,
            variant_values,
            error: None,
            _subscriptions: subscriptions,
        };
        view.load_fields(window, cx);
        view.load_variant(window, cx);
        view
    });
    let focus = view.read(cx).sdk_dir.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let view_ok = view.clone();
        dialog.title("Project Structure").w(px(980.)).child(view.clone()).footer(dialogs::footer("OK")).on_ok(move |_, _, cx| view_ok.update(cx, |v, cx| v.save(cx)))
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
