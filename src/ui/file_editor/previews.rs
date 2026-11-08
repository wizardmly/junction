//! Android Studio's resource previews in the editor gutter: a color's
//! swatch and a bitmap drawable's thumbnail on the lines that use them
//! (`R.color.primary`, `@drawable/ic_logo`, `#FF6200EE`), a drawable
//! XML (vector, shape, selector, layer list) drawn from its XML.

use gpui_kit::component::color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState};
use gpui_kit::component::input::{Input, InputState, RopeExt as _};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AppContext as _, Bounds, Context, DispatchPhase, Entity, InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, ParentElement as _, Pixels, Point, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window, anchored, canvas, deferred, div, point, prelude::FluentBuilder as _, px, size,
};

use super::{FileEditor, GUTTER_ICON, GUTTER_ICON_GAP, GUTTER_ICON_SLOT};
use crate::domain::drawable;
use crate::theme::ActivePalette as _;
use crate::usecase::android_resources::{self, ColorTarget, Preview};

impl FileEditor {
    /// Recomputes the previews from the text (a moment after typing) and
    /// the project's resources.
    pub(super) fn update_previews(&mut self, debounce: bool, cx: &mut Context<Self>) {
        let Some(index) = self.code_index.clone() else { return };
        if self.image.is_some() || !(self.path.ends_with(".kt") || self.path.ends_with(".java") || self.path.ends_with(".xml")) {
            return;
        }
        let path = self.path.clone();
        let text = self.text(cx);
        let table = index.read(cx).resources.clone();
        let framework = index.read(cx).framework_table();
        let root = index.read(cx).root().map(|r| r.to_path_buf()).unwrap_or_default();
        let fold = crate::settings::Settings::get(cx).diff.fold_android_strings;
        let revision = self.server_edits;
        self.previews_task = Some(cx.spawn(async move |this, cx| {
            if debounce {
                cx.background_executor().timer(std::time::Duration::from_millis(400)).await;
            }
            let (marks, vectors, folds, text) = cx
                .background_spawn(async move {
                    let table = table.read().unwrap();
                    let folds = if fold { crate::usecase::resource_folding::folds(&table, &path, &text) } else { Vec::new() };
                    let marks: Vec<(u32, Preview)> = android_resources::previews(&table, framework.as_deref(), &path, &text)
                        .into_iter()
                        .map(|(line, p)| match p {
                            Preview::Image(rel) => (line, Preview::Image(root.join(rel).to_string_lossy().into_owned())),
                            Preview::Vector(rel) => (line, Preview::Vector(root.join(rel).to_string_lossy().into_owned())),
                            other => (line, other),
                        })
                        .collect();
                    // Drawable XML (vectors, shapes, selectors…) drawn for the gutter.
                    let color = |value: &str| -> Option<u32> {
                        let (kind, name) = value.strip_prefix('@')?.split_once('/')?;
                        table.color(&crate::domain::android_resources::ResourceRef::new(kind, name))
                    };
                    let value = |value: &str| -> Option<String> {
                        let (kind, name) = value.strip_prefix('@')?.split_once('/')?;
                        table.value(&crate::domain::android_resources::ResourceRef::new(kind, name))
                    };
                    let load = |value: &str| -> Option<String> {
                        let (kind, name) = value.strip_prefix('@')?.split_once('/')?;
                        let path = table.drawable_xml(&crate::domain::android_resources::ResourceRef::new(kind, name))?.path.to_string();
                        std::fs::read_to_string(root.join(path)).ok()
                    };
                    let lookup = drawable::Lookup { color: &color, value: &value, drawable: &load };
                    let mut vectors = std::collections::HashMap::new();
                    for (_, p) in &marks {
                        let Preview::Vector(file) = p else { continue };
                        if vectors.contains_key(file) {
                            continue;
                        }
                        let image = std::fs::read_to_string(file).ok().filter(|t| drawable::is_previewable(t)).and_then(|text| {
                            let size = drawable::render(&text, 1., &lookup).ok()?;
                            let svg = drawable::render(&text, 32. / size.width.max(size.height).max(1.), &lookup).ok()?.svg;
                            Some(std::sync::Arc::new(gpui_kit::Image::from_bytes(gpui_kit::ImageFormat::Svg, svg.into_bytes())))
                        });
                        vectors.insert(file.clone(), image);
                    }
                    (marks.into_iter().filter(|(_, p)| !matches!(p, Preview::Vector(f) if vectors.get(f).is_none_or(Option::is_none))).collect::<Vec<_>>(), vectors, folds, text)
                })
                .await;
            this.update(cx, |this, cx| {
                this.set_resource_folds(folds, &text, revision, cx);
                this.vector_icons = std::rc::Rc::new(vectors.into_iter().filter_map(|(k, v)| Some((k, v?))).collect());
                if this.previews != marks {
                    this.previews = marks;
                    this.apply_gutter_extra(cx);
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// The previews' column, after the implementation markers and bookmarks.
    pub(super) fn preview_strip(&self, slot: usize, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        if self.previews.is_empty() {
            return None;
        }
        let marks = self.previews.clone();
        let state = self.state.clone();
        let this = cx.entity().downgrade();
        let vectors = self.vector_icons.clone();
        Some(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    let placed: Vec<(Bounds<Pixels>, u32, Preview)> = {
                        let state = state.read(cx);
                        let (Some(line_height), Some(visible), Some(left)) = (state.line_height(), state.visible_row_range(), state.gutter_extra_left()) else { return };
                        let rope = state.text();
                        let lines = rope.lines_len().max(1);
                        marks
                            .iter()
                            .filter(|(line, _)| (*line as usize) >= visible.start && (*line as usize) <= visible.end && (*line as usize) < lines)
                            .filter_map(|(line, preview)| {
                                let at = rope.line_start_offset(*line as usize);
                                let top = state.range_to_bounds(&(at..at))?.top();
                                let icon = px(GUTTER_ICON - 2.);
                                let rect = Bounds::new(point(left + px(GUTTER_ICON_GAP + 2. + GUTTER_ICON_SLOT * slot as f32), top + (line_height - icon) / 2.), size(icon, icon));
                                bounds.contains(&rect.center()).then(|| (rect, *line, preview.clone()))
                            })
                            .collect()
                    };
                    let mut hits: Vec<(Bounds<Pixels>, u32, Preview)> = Vec::new();
                    for (rect, line, preview) in placed {
                        hits.push((rect, line, preview.clone()));
                        match preview {
                            Preview::Color(rgba) => {
                                let color: gpui_kit::Hsla = gpui_kit::rgba(rgba).into();
                                let border: gpui_kit::Hsla = gpui_kit::rgba(0x8080_80aa).into();
                                window.paint_quad(gpui_kit::quad(rect, px(2.), color, px(1.), border, Default::default()));
                            }
                            Preview::Vector(path) => {
                                if let Some(image) = vectors.get(&path).and_then(|i| i.clone().use_render_image(window, cx)) {
                                    let _ = window.paint_image(rect, rect, Default::default(), image, 0, false);
                                }
                            }
                            Preview::Image(path) => {
                                let source = gpui_kit::Resource::Path(std::path::PathBuf::from(path).into());
                                if let Some(Ok(image)) = window.use_asset::<gpui_kit::ImageAssetLoader>(&source, cx) {
                                    let _ = window.paint_image(rect, rect, Default::default(), image, 0, false);
                                }
                            }
                        }
                    }
                    let this = this.clone();
                    window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                        if phase != DispatchPhase::Bubble || e.button != MouseButton::Left {
                            return;
                        }
                        let Some((rect, line, preview)) = hits.iter().find(|(r, _, _)| r.dilate(px(2.)).contains(&e.position)) else { return };
                        cx.stop_propagation();
                        let (line, preview, at) = (*line, preview.clone(), rect.bottom_left() + point(px(0.), px(2.)));
                        this.update(cx, |this, cx| this.preview_clicked(line, preview, at, window, cx)).ok();
                    });
                },
            )
            .absolute()
            .inset_0(),
        )
    }

    /// A gutter preview clicked, as in Android Studio: a color's swatch
    /// opens its color popup (Resources and Custom tabs), any other
    /// preview goes to the resource's declaration.
    fn preview_clicked(&mut self, line: u32, preview: Preview, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.line_text(line as usize, cx);
        let start = self.state.read(cx).text().line_start_offset(line as usize);
        if let Preview::Color(rgba) = preview
            && !self.read_only()
            && let Some(target) = android_resources::color_target(&self.path, &text)
        {
            self.open_color_edit(line, rgba, target, at, window, cx);
            return;
        }
        let Some(column) = android_resources::preview_reference(&self.path, &text) else { return };
        let offset = start + column;
        self.state.update(cx, |state, cx| state.set_selected_range(offset..offset, cx));
        self.goto_declaration(&super::GotoDeclaration, window, cx);
    }

    /// Android Studio's color popup (`createAndShowColorPickerPopup`): a
    /// Resources tab listing the project's and the framework's colors
    /// (`CompactResourcePicker`) and a Custom tab with the color picker,
    /// opening on Resources when the line refers to a resource now.
    fn open_color_edit(&mut self, line: u32, rgba: u32, target: ColorTarget, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let picker = cx.new(|cx| ColorPickerState::new(window, cx).default_value(gpui_kit::rgba(rgba)));
        picker.update(cx, |p, cx| p.set_open(true, cx));
        let changed = cx.subscribe_in(&picker, window, move |this, _, event: &ColorPickerEvent, window, cx| {
            let ColorPickerEvent::Change(Some(color)) = event else { return };
            this.set_color_literal(line, *color, window, cx);
        });
        // Committing a color (a palette swatch, Enter in the hex field)
        // closes the picker, and the popup with it.
        let closed = cx.observe(&picker, |this, picker, cx| {
            if !picker.read(cx).is_open() {
                this.color_edit = None;
            }
            cx.notify();
        });
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let typed = cx.observe(&search, |_, _, cx| cx.notify());
        let (project, framework) = match &self.code_index {
            Some(index) => {
                let index = index.read(cx);
                let project = index.resources.read().map(|t| t.colors()).unwrap_or_default();
                (project, index.framework_table().map(|t| t.colors()).unwrap_or_default())
            }
            None => Default::default(),
        };
        let resources = target.resources && !(project.is_empty() && framework.is_empty());
        if !resources && !target.custom {
            return;
        }
        let tab = if !target.custom || (resources && target.reference) { 0 } else { 1 };
        if tab == 0 {
            search.update(cx, |s, cx| s.focus(window, cx));
        }
        // The current reference's name, highlighted in the list.
        let text = self.line_text(line as usize, cx);
        let current = text[target.range.clone()].rsplit(['/', '.']).next().unwrap_or_default().to_owned();
        let framework_now = text[target.range.clone()].contains("android");
        self.color_edit = Some(ColorEdit {
            picker,
            at,
            line,
            custom: target.custom,
            resources,
            tab,
            source: usize::from(framework_now && !framework.is_empty()),
            project,
            framework,
            current,
            search,
            _subscriptions: [changed, closed, typed],
        });
        cx.notify();
    }

    /// Writes a picked color over the line's literal or reference, the
    /// literal in its format.
    fn set_color_literal(&mut self, line: u32, color: gpui_kit::Hsla, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.line_text(line as usize, cx);
        let Some(target) = android_resources::color_target(&self.path, &text).filter(|t| t.custom) else { return };
        let rgb = color.to_rgb();
        let channel = |v: f32| (v.clamp(0., 1.) * 255.).round() as u32;
        let rgba = (channel(rgb.r) << 24) | (channel(rgb.g) << 16) | (channel(rgb.b) << 8) | channel(rgb.a);
        let new = android_resources::format_color_literal(rgba, &text[target.range.clone()]);
        self.replace_on_line(line, target.range, new, window, cx);
    }

    /// A color resource picked in the Resources tab: written over the
    /// line's color as `@color/x` (XML) or `R.color.x` (code); the popup
    /// closes, as Android Studio's does.
    fn set_color_resource(&mut self, name: String, framework: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.color_edit.take() else { return };
        let text = self.line_text(edit.line as usize, cx);
        if let Some(target) = android_resources::color_target(&self.path, &text).filter(|t| t.resources) {
            let new = android_resources::color_reference(&self.path, &name, framework);
            self.replace_on_line(edit.line, target.range, new, window, cx);
        }
        self.focus(window, cx);
        cx.notify();
    }

    fn replace_on_line(&mut self, line: u32, range: std::ops::Range<usize>, new: String, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.line_text(line as usize, cx);
        if new == text[range.clone()] {
            return;
        }
        let start = self.state.read(cx).text().line_start_offset(line as usize);
        self.state.update(cx, |state, cx| {
            let cursor = state.cursor();
            state.set_selected_range(start + range.start..start + range.end, cx);
            state.replace(&new, window, cx);
            state.set_selected_range(cursor..cursor, cx);
        });
    }

    fn close_color_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.color_edit.take().is_some() {
            self.focus(window, cx);
            cx.notify();
        }
    }

    pub(super) fn render_color_edit(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let edit = self.color_edit.as_ref()?;
        let palette = cx.palette().clone();
        let theme = gpui_kit::component::ActiveTheme::theme(&**cx).clone();
        let body = if edit.tab == 0 && edit.resources {
            let query = edit.search.read(cx).value().to_lowercase();
            let framework = edit.source == 1;
            let colors = if framework { &edit.framework } else { &edit.project };
            let rows = colors.iter().filter(|(name, _)| query.is_empty() || name.to_lowercase().contains(&query)).map(|(name, rgba)| {
                let pick = name.clone();
                let selected = *name == edit.current;
                let swatch: gpui_kit::Hsla = gpui_kit::rgba(*rgba).into();
                h_flex()
                    .id(gpui_kit::SharedString::from(format!("color-res-{name}")))
                    .gap_2()
                    .px_2()
                    .py_0p5()
                    .rounded_sm()
                    .cursor_pointer()
                    .when(selected, |el| el.bg(palette.selection))
                    .hover(|el| el.bg(palette.hover))
                    .child(div().size(px(14.)).flex_shrink_0().rounded_sm().bg(swatch).border_1().border_color(gpui_kit::rgba(0x8080_80aa)))
                    .child(div().flex_1().overflow_hidden().text_ellipsis().whitespace_nowrap().child(name.clone()))
                    .child(div().text_xs().text_color(palette.text_secondary).child(crate::domain::android_resources::format_color(*rgba, "#000000")))
                    .on_click(cx.listener(move |this, _, window, cx| this.set_color_resource(pick.clone(), framework, window, cx)))
            });
            let rows: Vec<_> = rows.collect();
            let empty = rows.is_empty();
            v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .gap_1()
                        .child(div().flex_1().child(Input::new(&edit.search).small()))
                        .child(
                            TabBar::new("color-res-source")
                                .segmented()
                                .xsmall()
                                .selected_index(edit.source)
                                .on_click(cx.listener(|this, ix: &usize, _, cx| {
                                    if let Some(edit) = this.color_edit.as_mut() {
                                        edit.source = *ix;
                                    }
                                    cx.notify();
                                }))
                                .child(Tab::new().label("Project").disabled(edit.project.is_empty()))
                                .child(Tab::new().label("Android").disabled(edit.framework.is_empty())),
                        ),
                )
                .child(
                    v_flex()
                        .id("color-res-list")
                        .h(px(260.))
                        .overflow_y_scroll()
                        .text_sm()
                        .children(rows)
                        .when(empty, |el| el.child(div().p_2().text_color(palette.text_secondary).child("Nothing to show"))),
                )
                .into_any_element()
        } else {
            ColorPicker::new(&edit.picker).render_panel(window, cx).into_any_element()
        };
        let tabs = (edit.custom && edit.resources).then(|| {
            TabBar::new("color-edit-tabs")
                .underline()
                .small()
                .selected_index(edit.tab)
                .on_click(cx.listener(|this, ix: &usize, window, cx| {
                    if let Some(edit) = this.color_edit.as_mut() {
                        edit.tab = *ix;
                        if *ix == 0 {
                            edit.search.update(cx, |s, cx| s.focus(window, cx));
                        }
                    }
                    cx.notify();
                }))
                .child(Tab::new().label("Resources"))
                .child(Tab::new().label("Custom"))
        });
        Some(
            deferred(
                anchored().position(edit.at).snap_to_window_with_margin(px(8.)).child(
                    v_flex()
                        .id("color-edit-popup")
                        .occlude()
                        .on_mouse_down_out(cx.listener(|this, _, window, cx| this.close_color_edit(window, cx)))
                        .on_key_down(cx.listener(|this, e: &gpui_kit::KeyDownEvent, window, cx| {
                            if e.keystroke.key == "escape" {
                                cx.stop_propagation();
                                this.close_color_edit(window, cx);
                            }
                        }))
                        .w(px(300.))
                        .p_2()
                        .gap_2()
                        .bg(theme.popover)
                        .border_1()
                        .border_color(palette.border)
                        .rounded_md()
                        .shadow_lg()
                        .children(tabs)
                        .child(body),
                ),
            )
            .with_priority(1),
        )
    }
}

pub(super) struct ColorEdit {
    picker: Entity<ColorPickerState>,
    at: Point<Pixels>,
    line: u32,
    /// The Custom tab is offered (a literal can go there).
    custom: bool,
    /// The Resources tab is offered.
    resources: bool,
    /// 0: Resources, 1: Custom.
    tab: usize,
    /// The Resources tab's source: 0 the project's, 1 the framework's.
    source: usize,
    project: Vec<(String, u32)>,
    framework: Vec<(String, u32)>,
    /// The resource the line refers to now, highlighted.
    current: String,
    search: Entity<InputState>,
    _subscriptions: [Subscription; 3],
}
