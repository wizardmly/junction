//! A vector drawable's preview, as Android Studio's drawable editor shows
//! it: Code / Split / Design at the top right, the picture beside the XML
//! (Split, the default) or alone (Design), its size in dp, redrawn as the
//! XML is edited; `@color/…` from the project's resources.

use std::hash::{Hash as _, Hasher as _};
use std::sync::Arc;

use gpui_kit::component::{Selectable as _, h_flex, v_flex};
use gpui_kit::{AnyElement, Context, IntoElement, ParentElement as _, Styled as _, div, prelude::FluentBuilder as _, px};

use super::FileEditor;
use crate::domain::android_resources::ResourceRef;
use crate::domain::vector_drawable;
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::common::tool_button;

/// The drawable editor's view mode.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum VectorMode {
    Code,
    #[default]
    Split,
    Design,
}

/// The picture is drawn this big at most, its longer side.
const PREVIEW: f32 = 256.;

pub(super) struct VectorPreview {
    hash: u64,
    /// The picture and its size in dp, or why there's none.
    result: Result<(Arc<gpui_kit::Image>, f32, f32), String>,
}

impl FileEditor {
    /// A drawable or mipmap XML file: the ones that may be vectors.
    fn is_drawable_xml(&self) -> bool {
        let folder = std::path::Path::new(&self.path).parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.path.ends_with(".xml") && (folder.starts_with("drawable") || folder.starts_with("mipmap")) && self.revision.is_none()
    }

    /// The preview for the current text, redrawn when it changed; none for
    /// files that aren't vectors.
    fn vector_preview(&mut self, cx: &mut Context<Self>) -> Option<&VectorPreview> {
        if !self.is_drawable_xml() {
            return None;
        }
        let text = self.state.read(cx).value().to_string();
        let mut hasher = std::hash::DefaultHasher::new();
        text.hash(&mut hasher);
        let hash = hasher.finish();
        if self.vector.as_ref().map(|v| v.hash) != Some(hash) {
            if !text.contains("<vector") {
                self.vector = None;
                return None;
            }
            let table = self.code_index.as_ref().map(|i| i.read(cx).resources.clone());
            let color = move |value: &str| -> Option<u32> {
                let (kind, name) = value.strip_prefix('@')?.split_once('/')?;
                table.as_ref()?.read().ok()?.color(&ResourceRef::new(kind, name))
            };
            // Measure first, then draw at the preview's size.
            let result = vector_drawable::to_svg(&text, 1., &color).and_then(|first| {
                let scale = PREVIEW / first.width.max(first.height).max(1.);
                let image = vector_drawable::to_svg(&text, scale, &color)?;
                Ok((Arc::new(gpui_kit::Image::from_bytes(gpui_kit::ImageFormat::Svg, image.svg.into_bytes())), image.width, image.height))
            });
            self.vector = Some(VectorPreview { hash, result });
        }
        self.vector.as_ref()
    }

    /// Code / Split / Design, for the editor's header.
    pub(super) fn vector_mode_buttons(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.vector_preview(cx)?;
        let mode = self.vector_mode;
        let button = |id: &'static str, icon, tip: &'static str, m: VectorMode, cx: &mut Context<Self>| {
            tool_button(id, icon, tip).selected(mode == m).on_click(cx.listener(move |this, _, _, cx| {
                this.vector_mode = m;
                cx.notify();
            }))
        };
        Some(
            h_flex()
                .gap_0p5()
                .child(button("vector-code", icons::LIST_FILES, "Code", VectorMode::Code, cx))
                .child(button("vector-split", icons::SPLIT_VERTICALLY, "Split", VectorMode::Split, cx))
                .child(button("vector-design", icons::PREVIEW, "Design", VectorMode::Design, cx))
                .into_any_element(),
        )
    }

    /// Whether the XML is hidden behind the preview (Design).
    pub(super) fn vector_design_only(&self) -> bool {
        self.vector.is_some() && self.is_drawable_xml() && self.vector_mode == VectorMode::Design
    }

    /// The preview pane beside the XML (Split) or instead of it (Design).
    pub(super) fn vector_pane(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.vector_mode == VectorMode::Code {
            return None;
        }
        let palette = cx.palette().clone();
        let design = self.vector_mode == VectorMode::Design;
        let preview = self.vector_preview(cx)?;
        let body = match &preview.result {
            Ok((image, w, h)) => {
                let scale = PREVIEW / w.max(*h).max(1.);
                v_flex()
                    .items_center()
                    .gap_2()
                    .child(gpui_kit::img(image.clone()).w(px(w * scale)).h(px(h * scale)).flex_none().border_1().border_color(palette.border))
                    .child(div().text_xs().text_color(palette.text_secondary).child(format!("{} × {} dp", trim(*w), trim(*h))))
                    .into_any_element()
            }
            Err(e) => div().text_sm().text_color(palette.status_conflict).child(format!("Cannot render the vector: {e}")).into_any_element(),
        };
        Some(
            div()
                .when(design, |el| el.flex_1())
                .when(!design, |el| el.w(px(PREVIEW + 80.)).flex_none().border_l_1().border_color(palette.border))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .p_4()
                .bg(palette.panel)
                .child(body)
                .into_any_element(),
        )
    }
}

/// `24` rather than `24.0`.
fn trim(v: f32) -> String {
    if v.fract() == 0. { format!("{}", v as i64) } else { format!("{v}") }
}
