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
use crate::domain::drawable::{self, Lookup};
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
    /// A selector's items: their states and pictures.
    states: Vec<(String, Arc<gpui_kit::Image>)>,
}

/// A selector item's picture, its longer side.
const STATE: f32 = 48.;

/// Color, value and drawable lookups in the project's resources, for
/// `drawable::Lookup`.
#[allow(clippy::type_complexity)]
pub(super) fn resource_lookups(
    table: Option<std::sync::Arc<std::sync::RwLock<crate::usecase::android_resources::ResourceTable>>>,
    root: std::path::PathBuf,
) -> (impl Fn(&str) -> Option<u32>, impl Fn(&str) -> Option<String>, impl Fn(&str) -> Option<String>) {
    let reference = |value: &str| -> Option<ResourceRef> {
        let (kind, name) = value.strip_prefix('@')?.split_once('/')?;
        Some(ResourceRef::new(kind, name))
    };
    let (t1, t2, t3) = (table.clone(), table.clone(), table);
    (
        move |value: &str| t1.as_ref()?.read().ok()?.color(&reference(value)?),
        move |value: &str| t2.as_ref()?.read().ok()?.value(&reference(value)?),
        move |value: &str| {
            let path = t3.as_ref()?.read().ok()?.drawable_xml(&reference(value)?)?.path.to_string();
            std::fs::read_to_string(root.join(path)).ok()
        },
    )
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
            if !drawable::is_previewable(&text) {
                self.vector = None;
                return None;
            }
            let index = self.code_index.as_ref().map(|i| i.read(cx));
            let table = index.as_ref().map(|i| i.resources.clone());
            let root = index.and_then(|i| i.root().map(|r| r.to_path_buf())).unwrap_or_default();
            let lookups = resource_lookups(table, root);
            let lookup = Lookup { color: &lookups.0, value: &lookups.1, drawable: &lookups.2 };
            // Measure first, then draw at the preview's size.
            let result = drawable::render(&text, 1., &lookup).and_then(|first| {
                let scale = PREVIEW / first.width.max(first.height).max(1.);
                let image = drawable::render(&text, scale, &lookup)?;
                Ok((Arc::new(gpui_kit::Image::from_bytes(gpui_kit::ImageFormat::Svg, image.svg.into_bytes())), image.width, image.height))
            });
            let states = drawable::states(&text, STATE, &lookup)
                .into_iter()
                .map(|(label, svg)| (label, Arc::new(gpui_kit::Image::from_bytes(gpui_kit::ImageFormat::Svg, svg.into_bytes()))))
                .collect();
            self.vector = Some(VectorPreview { hash, result, states });
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
            Err(e) => div().text_sm().text_color(palette.status_conflict).child(format!("Cannot render the drawable: {e}")).into_any_element(),
        };
        // A selector's state list under its default state.
        let states = (!preview.states.is_empty()).then(|| {
            h_flex().flex_wrap().justify_center().gap_3().pt_4().children(preview.states.iter().map(|(label, image)| {
                v_flex()
                    .items_center()
                    .gap_1()
                    .child(gpui_kit::img(image.clone()).max_w(px(STATE)).max_h(px(STATE)).flex_none())
                    .child(div().text_xs().text_color(palette.text_secondary).child(label.clone()))
            }))
        });
        let body = v_flex().items_center().child(body).children(states).into_any_element();
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
