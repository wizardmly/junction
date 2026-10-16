//! Android Studio's resource previews in the editor gutter: a color's
//! swatch and a bitmap drawable's thumbnail on the lines that use them
//! (`R.color.primary`, `@drawable/ic_logo`, `#FF6200EE`), a drawable
//! XML (vector, shape, selector, layer list) drawn from its XML.

use gpui_kit::component::input::RopeExt as _;
use gpui_kit::{AppContext as _, Bounds, Context, IntoElement, Pixels, Styled as _, canvas, point, px, size};

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
    pub(super) fn preview_strip(&self, slot: usize) -> Option<impl IntoElement + use<>> {
        if self.previews.is_empty() {
            return None;
        }
        let marks = self.previews.clone();
        let state = self.state.clone();
        let vectors = self.vector_icons.clone();
        Some(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    let placed: Vec<(Bounds<Pixels>, Preview)> = {
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
                                bounds.contains(&rect.center()).then(|| (rect, preview.clone()))
                            })
                            .collect()
                    };
                    for (rect, preview) in placed {
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
                },
            )
            .absolute()
            .inset_0(),
        )
    }
}
