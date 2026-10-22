//! Android Studio's resource previews in the editor gutter: a color's
//! swatch and a bitmap drawable's thumbnail on the lines that use them
//! (`R.color.primary`, `@drawable/ic_logo`, `#FF6200EE`), a drawable
//! XML (vector, shape, selector, layer list) drawn from its XML.

use gpui_kit::component::color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState};
use gpui_kit::component::input::RopeExt as _;
use gpui_kit::{
    AppContext as _, Bounds, Context, DispatchPhase, Entity, IntoElement, MouseButton, MouseDownEvent, ParentElement as _, Pixels, Point, Styled as _, Subscription, Window,
    anchored, canvas, deferred, div, point, px, size,
};

use super::{FileEditor, GUTTER_ICON, GUTTER_ICON_GAP, GUTTER_ICON_SLOT};
use crate::domain::drawable;
use crate::usecase::android_resources::{self, Preview};

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
        let root = index.read(cx).root().map(|r| r.to_path_buf()).unwrap_or_default();
        self.previews_task = Some(cx.spawn(async move |this, cx| {
            if debounce {
                cx.background_executor().timer(std::time::Duration::from_millis(400)).await;
            }
            let (marks, vectors) = cx
                .background_spawn(async move {
                    let table = table.read().unwrap();
                    let marks: Vec<(u32, Preview)> = android_resources::previews(&table, &path, &text)
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
                    (marks.into_iter().filter(|(_, p)| !matches!(p, Preview::Vector(f) if vectors.get(f).is_none_or(Option::is_none))).collect::<Vec<_>>(), vectors)
                })
                .await;
            this.update(cx, |this, cx| {
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

    /// A gutter preview clicked, as in Android Studio: a color literal's
    /// swatch opens a color picker that rewrites it, any other preview goes
    /// to the resource's declaration.
    fn preview_clicked(&mut self, line: u32, preview: Preview, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.line_text(line as usize, cx);
        let start = self.state.read(cx).text().line_start_offset(line as usize);
        if let Preview::Color(rgba) = preview
            && !self.read_only()
            && self.path.ends_with(".xml")
            && android_resources::color_literal(&text).is_some()
        {
            let picker = cx.new(|cx| ColorPickerState::new(window, cx).default_value(gpui_kit::rgba(rgba)));
            picker.update(cx, |p, cx| p.set_open(true, cx));
            let changed = cx.subscribe_in(&picker, window, move |this, _, event: &ColorPickerEvent, window, cx| {
                let ColorPickerEvent::Change(Some(color)) = event else { return };
                this.set_color_literal(line, *color, window, cx);
            });
            // Kept after it closes: committing closes it before the change
            // is delivered.
            let closed = cx.observe(&picker, |_, _, cx| cx.notify());
            self.color_edit = Some(ColorEdit { picker, at, _subscriptions: [changed, closed] });
            cx.notify();
            return;
        }
        let Some(column) = android_resources::preview_reference(&self.path, &text) else { return };
        let offset = start + column;
        self.state.update(cx, |state, cx| state.set_selected_range(offset..offset, cx));
        self.goto_declaration(&super::GotoDeclaration, window, cx);
    }

    /// Writes a picked color over the line's literal, in its format.
    fn set_color_literal(&mut self, line: u32, color: gpui_kit::Hsla, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.line_text(line as usize, cx);
        let Some(range) = android_resources::color_literal(&text) else { return };
        let rgb = color.to_rgb();
        let channel = |v: f32| (v.clamp(0., 1.) * 255.).round() as u32;
        let rgba = (channel(rgb.r) << 24) | (channel(rgb.g) << 16) | (channel(rgb.b) << 8) | channel(rgb.a);
        let new = crate::domain::android_resources::format_color(rgba, &text[range.clone()]);
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

    pub(super) fn render_color_edit(&self, cx: &gpui_kit::App) -> Option<impl IntoElement + use<>> {
        let edit = self.color_edit.as_ref().filter(|e| e.picker.read(cx).is_open())?;
        Some(deferred(anchored().position(edit.at).child(div().size(px(1.)).child(ColorPicker::new(&edit.picker)))).with_priority(1))
    }
}

pub(super) struct ColorEdit {
    picker: Entity<ColorPickerState>,
    at: Point<Pixels>,
    _subscriptions: [Subscription; 2],
}
