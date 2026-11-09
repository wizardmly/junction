//! Android Studio's manifest editor tabs: "Text" and "Merged Manifest"
//! under an `AndroidManifest.xml`. The merged manifest is read-only, each
//! line tinted by the file it came from, with "Manifest Sources" (those
//! files and their colors) and "Merging Errors" beside it; it follows the
//! text as it's edited. A click selects a line and shows its "Merging
//! Log" (who added it, who merged into it, what the build file injected),
//! each entry opening its source; a double click opens the line's source.

use std::hash::{Hash as _, Hasher as _};
use std::path::{Path, PathBuf};

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, StatefulInteractiveElement as _, Styled as _, div, hsla, prelude::FluentBuilder as _, px};

use super::FileEditor;
use crate::domain::manifest_merge::Merged;
use crate::theme::ActivePalette as _;

pub(super) struct MergedManifest {
    hash: u64,
    result: Result<Merged, String>,
    /// The line whose Merging Log is shown.
    selected: Option<usize>,
}

/// Where a source's line is, to open it; none for a remote library's
/// ("group:artifact:version"), which has no file.
fn target(sources: &[String], source: usize, line: Option<u32>) -> Option<crate::index::nav::Target> {
    sources.get(source).filter(|path| !path.contains(':')).map(|path| crate::index::nav::Target {
        path: path.clone(),
        line: line.unwrap_or(0),
        col: 0,
        name: path.rsplit('/').next().unwrap_or(path).to_owned(),
        label: "Manifest source".into(),
        container: None,
    })
}

/// A source's tint: Android Studio's distinct pastel hues.
fn tint(source: usize, dark: bool) -> gpui_kit::Hsla {
    const HUES: [f32; 8] = [210., 120., 35., 280., 0., 170., 320., 60.];
    let hue = HUES[source % HUES.len()] / 360.;
    if dark { hsla(hue, 0.5, 0.45, 0.22) } else { hsla(hue, 0.7, 0.6, 0.2) }
}

impl FileEditor {
    fn is_manifest(&self) -> bool {
        self.path.ends_with("AndroidManifest.xml") && self.revision.is_none()
    }

    /// The Merged Manifest tab is showing (the text hidden).
    pub(super) fn manifest_merged_shown(&self) -> bool {
        self.is_manifest() && self.manifest_merged
    }

    /// The merge for the current text, redone when it changed.
    fn merged_manifest(&mut self, cx: &mut Context<Self>) -> &MergedManifest {
        let text = self.text(cx);
        let path = self.absolute_path();
        let root = self.code_index.as_ref().and_then(|i| i.read(cx).root().map(Path::to_path_buf)).unwrap_or_else(|| self.repository.root().to_path_buf());
        // The picked build variant's source sets.
        let sets = crate::usecase::merged_manifest::module_of(&path)
            .and_then(|(module, _)| {
                let rel = module.strip_prefix(&root).ok()?.to_string_lossy().replace('\\', "/");
                let index = self.code_index.as_ref()?.read(cx).index.clone();
                let modules = index.read().ok()?.modules.clone();
                Some(modules.variant_source_sets(&rel))
            })
            .unwrap_or_default();
        // The remote libraries' (AARs') manifests.
        let external = self.code_index.as_ref().and_then(|i| i.read(cx).index.read().ok().map(|i| i.external.clone()));
        let aars: Vec<(String, PathBuf)> = external
            .iter()
            .flat_map(|e| e.libraries.iter())
            .filter_map(|l| Some((l.name.trim_start_matches("Gradle: ").trim_end_matches("@aar").to_owned(), l.aar.clone()?)))
            .collect();
        let mut hasher = std::hash::DefaultHasher::new();
        (&text, &sets, &aars).hash(&mut hasher);
        let hash = hasher.finish();
        if self.merged.as_ref().map(|m| m.hash) != Some(hash) {
            // This file as edited, the others from disk.
            let read = |p: &Path| if p == path.as_path() { Some(text.clone()) } else { std::fs::read_to_string(p).ok() };
            let aars: Vec<(String, String)> = aars.into_iter().filter_map(|(name, aar)| Some((name, crate::index::libraries::aar_manifest(&aar)?))).collect();
            let result = crate::usecase::merged_manifest::merged_manifest(&root, &path, &sets, &aars, &read);
            self.merged = Some(MergedManifest { hash, result, selected: None });
        }
        self.merged.as_ref().expect("just set")
    }

    /// "Text" / "Merged Manifest" under a manifest.
    pub(super) fn manifest_tabs(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.is_manifest() {
            return None;
        }
        let palette = cx.palette().clone();
        let tab = |id: &'static str, label: &'static str, merged: bool, cx: &mut Context<Self>| {
            let active = self.manifest_merged == merged;
            div()
                .id(id)
                .px_3()
                .h_full()
                .flex()
                .items_center()
                .text_sm()
                .cursor_pointer()
                .border_t_2()
                .border_color(if active { palette.accent } else { gpui_kit::transparent_black() })
                .when(!active, |el| el.hover(|s| s.bg(palette.hover)))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.manifest_merged = merged;
                    cx.notify();
                }))
        };
        Some(
            h_flex()
                .h(px(28.))
                .flex_none()
                .border_t_1()
                .border_color(palette.border)
                .bg(palette.toolbar)
                .child(tab("manifest-text", "Text", false, cx))
                .child(tab("manifest-merged", "Merged Manifest", true, cx))
                .into_any_element(),
        )
    }

    /// The merged manifest, its sources and errors.
    pub(super) fn manifest_pane(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.manifest_merged_shown() {
            return None;
        }
        let palette = cx.palette().clone();
        let dark = palette.dark;
        let mono = {
            use gpui_kit::component::ActiveTheme as _;
            cx.theme().mono_font_family.clone()
        };
        let state = self.merged_manifest(cx);
        let selected = state.selected;
        let merged = match &state.result {
            Ok(m) => m.clone(),
            Err(e) => {
                return Some(div().flex_1().p_4().text_sm().text_color(palette.status_conflict).child(format!("Cannot merge the manifest: {e}")).into_any_element());
            }
        };
        // A click selects the line; a double click opens its source there
        // (the build file at its top).
        let lines = v_flex().children(merged.lines.iter().enumerate().map(|(ix, l)| {
            let target = target(&merged.sources, l.source, l.line);
            div()
                .id(("merged-line", ix))
                .px_2()
                .whitespace_nowrap()
                .bg(if selected == Some(ix) { palette.selection } else { tint(l.source, dark) })
                .hover(|s| s.bg(palette.hover))
                .child(if l.text.is_empty() { " ".to_owned() } else { l.text.clone() })
                .on_click(cx.listener(move |this, event: &gpui_kit::ClickEvent, _, cx| {
                    if let Some(m) = this.merged.as_mut() {
                        m.selected = Some(ix);
                    }
                    if let (2, Some(target)) = (event.click_count(), target.clone()) {
                        cx.emit(super::FileEditorEvent::Navigate { targets: vec![target], at: None });
                    }
                    cx.notify();
                }))
        }));
        let heading = |text: &'static str| div().px_2().py_1().text_sm().font_weight(FontWeight::SEMIBOLD).child(text);
        let sources = v_flex().gap_1().px_2().children(merged.sources.iter().enumerate().map(|(i, s)| {
            h_flex().gap_2().text_sm().child(div().size(px(12.)).flex_none().rounded_sm().bg(tint(i, dark)).border_1().border_color(palette.border)).child(div().overflow_hidden().text_ellipsis().child(s.clone()))
        }));
        // The selected line's Merging Log, each entry opening its source.
        let log = selected.and_then(|ix| merged.lines.get(ix)).filter(|l| !l.log.is_empty()).map(|l| {
            v_flex().pb_1().children(std::iter::once(heading("Merging Log").into_any_element()).chain(l.log.iter().enumerate().map(|(i, e)| {
                let place = match (merged.sources.get(e.source), e.line) {
                    (Some(s), Some(line)) => format!("{s}:{}", line + 1),
                    (Some(s), None) => s.clone(),
                    (None, _) => String::new(),
                };
                let target = target(&merged.sources, e.source, e.line);
                let clickable = target.is_some();
                h_flex()
                    .id(("merging-log", i))
                    .px_2()
                    .gap_1()
                    .text_xs()
                    .items_start()
                    .child(div().flex_none().font_weight(FontWeight::SEMIBOLD).child(e.action.label()))
                    .child(div().flex_none().text_color(palette.text_secondary).child("from"))
                    .child(div().min_w_0().text_color(if clickable { palette.link } else { palette.text }).when(clickable, |el| el.underline()).child(place))
                    .when_some(target, |el, target| {
                        el.cursor_pointer().hover(|s| s.bg(palette.hover)).on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(super::FileEditorEvent::Navigate { targets: vec![target.clone()], at: None });
                        }))
                    })
                    .into_any_element()
            })))
        });
        let errors = (!merged.errors.is_empty()).then(|| {
            v_flex().children(std::iter::once(heading("Merging Errors").into_any_element()).chain(
                merged.errors.iter().map(|e| div().px_2().pb_1().text_xs().text_color(palette.status_conflict).child(e.clone()).into_any_element()),
            ))
        });
        Some(
            h_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .items_start()
                .child(div().id("merged-manifest").flex_1().min_w_0().h_full().overflow_scroll().py_1().font_family(mono).text_size(px(crate::ui::diff_panes::font_size())).child(lines))
                .child(
                    v_flex()
                        .id("merged-manifest-side")
                        .w(px(320.))
                        .flex_none()
                        .h_full()
                        .overflow_y_scroll()
                        .border_l_1()
                        .border_color(palette.border)
                        .bg(palette.panel)
                        .child(heading("Manifest Sources"))
                        .child(sources)
                        .children(log)
                        .children(errors),
                )
                .into_any_element(),
        )
    }
}
