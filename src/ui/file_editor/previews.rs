//! Android Studio's resource previews in the editor gutter: a color's
//! swatch and a bitmap drawable's thumbnail on the lines that use them
//! (`R.color.primary`, `@drawable/ic_logo`, `#FF6200EE`).

use gpui_kit::component::input::RopeExt as _;
use gpui_kit::{AppContext as _, Bounds, Context, IntoElement, Pixels, Styled as _, canvas, point, px, size};

use super::{FileEditor, GUTTER_ICON, GUTTER_ICON_GAP, GUTTER_ICON_SLOT};
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
            let marks = cx.background_spawn(async move { android_resources::previews(&table.read().unwrap(), &path, &text) }).await;
            this.update(cx, |this, cx| {
                let marks: Vec<(u32, Preview)> = marks
                    .into_iter()
                    .map(|(line, p)| match p {
                        Preview::Image(rel) => (line, Preview::Image(root.join(rel).to_string_lossy().into_owned())),
                        other => (line, other),
                    })
                    .collect();
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
