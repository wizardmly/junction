//! Annotate with Git Blame in the diff viewer, as IntelliJ shows it: each
//! side's gutter gets its own annotation column by its line numbers, the
//! hovered commit's lines are marked on both sides, and the column has the
//! annotation actions on right-click.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::{
    AnyElement, App, AppContext as _, ClipboardItem, Context, ElementId, Entity, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, Task, Window, div, prelude::FluentBuilder as _, px,
};

use super::{DiffSource, DiffView};
use crate::git::Repository;
use crate::git::blame::{self, Blame, BlameCommit};
use crate::theme::Palette;
use crate::ui::annotation_gutter::{Annotation, AnnotationView};
use crate::ui::common;

/// Padding on each side of an annotation.
const PADDING: f32 = 6.;

/// What a diff side is blamed as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum BlameOf {
    /// The file at a revision.
    Revision { rev: String, path: String },
    /// The pane's own text as the work tree's `path` (`--contents`), so
    /// edits and staged text show as "Not Committed Yet".
    Text { path: String },
}

/// Each side of a diff, as it is blamed (`None`: a side Git doesn't know,
/// such as the clipboard or another file on disk).
pub(super) fn blame_targets(source: &DiffSource) -> [Option<BlameOf>; 2] {
    let rev = |rev: &str, path: &str| Some(BlameOf::Revision { rev: rev.to_owned(), path: path.to_owned() });
    let text = |path: &str| Some(BlameOf::Text { path: path.to_owned() });
    match source {
        DiffSource::Commit { hash, path, old_path } => [rev(&format!("{hash}^"), old_path.as_ref().unwrap_or(path)), rev(hash, path)],
        DiffSource::WorkingTree { path, unversioned } => [rev("HEAD", path), if *unversioned { None } else { text(path) }],
        DiffSource::Staged { path } => [rev("HEAD", path), text(path)],
        DiffSource::Unstaged { path } => [text(path), text(path)],
        DiffSource::Between { old, new, path, old_path } => {
            [rev(old, old_path.as_ref().unwrap_or(path)), match new {
                Some(new) => rev(new, path),
                None => text(path),
            }]
        }
        DiffSource::Files { path, .. } => [text(path), None],
        DiffSource::Clipboard { path, .. } => [None, text(path)],
        DiffSource::Texts { .. } => [None, None],
    }
}

fn run_blame(repository: &Repository, target: &BlameOf, text: &str) -> anyhow::Result<Blame> {
    match target {
        BlameOf::Revision { rev, path } => blame::blame(repository, path, Some(rev)),
        BlameOf::Text { path } => blame::blame_contents(repository, path, text),
    }
}

/// The diff's annotations: on or off, and each side's blame.
#[derive(Default)]
pub(super) struct DiffAnnotations {
    pub on: bool,
    pub sides: [Option<Annotation>; 2],
    pub view: AnnotationView,
    /// The commit under the mouse, marked on both sides.
    pub hovered: Option<String>,
    pub error: Option<String>,
    /// A right-click opened the column's menu: no commit tooltip over it
    /// until the mouse moves to another line.
    pub menu_open: bool,
    task: Option<Task<()>>,
}

impl DiffAnnotations {
    /// The commit annotating a pane's 0-based line.
    pub fn commit_at(&self, pane: usize, line: usize) -> Option<&BlameCommit> {
        self.sides.get(pane)?.as_ref()?.commit_at(line)
    }
}

impl DiffView {
    /// Whether this diff has a side Git can annotate.
    pub(super) fn can_annotate(&self) -> bool {
        self.source.as_ref().is_some_and(|s| blame_targets(s).iter().any(Option::is_some)) && self.loaded.is_some() && !self.diff.binary
    }

    /// Annotate with Git Blame / Close Annotations.
    pub(super) fn toggle_annotations(&mut self, cx: &mut Context<Self>) {
        if self.annotations.on {
            self.close_annotations(cx);
        } else {
            self.annotations.on = true;
            self.load_annotations(None, cx);
        }
    }

    pub(super) fn close_annotations(&mut self, cx: &mut Context<Self>) {
        let view = self.annotations.view;
        self.annotations = DiffAnnotations { view, ..Default::default() };
        cx.notify();
    }

    /// Blames the shown sides (only the panes' own texts with `texts_only`,
    /// after an edit), a moment later when `delay` is given.
    pub(super) fn load_annotations(&mut self, delay: Option<Duration>, cx: &mut Context<Self>) {
        let texts_only = delay.is_some();
        let (Some(repository), Some(source)) = (self.repository.clone(), self.source.clone()) else { return };
        if !self.annotations.on {
            return;
        }
        let targets = blame_targets(&source);
        let jobs: Vec<(usize, BlameOf, String)> = targets
            .into_iter()
            .enumerate()
            .filter_map(|(pane, target)| Some((pane, target?)))
            .filter(|(_, target)| !texts_only || matches!(target, BlameOf::Text { .. }))
            .map(|(pane, target)| (pane, target, self.panes.buffers[pane].text().to_owned()))
            .collect();
        if !texts_only {
            for side in self.annotations.sides.iter_mut().filter(|s| s.is_none()) {
                *side = Some(Annotation::loading());
            }
        }
        cx.notify();
        self.annotations.task = Some(cx.spawn(async move |this, cx| {
            if let Some(delay) = delay {
                cx.background_executor().timer(delay).await;
            }
            let results = cx
                .background_spawn(async move {
                    jobs.into_iter().map(|(pane, target, text)| (pane, run_blame(&repository, &target, &text))).collect::<Vec<_>>()
                })
                .await;
            this.update(cx, |this, cx| {
                // Still the same file, annotated.
                if !this.annotations.on || this.source.as_ref() != Some(&source) {
                    return;
                }
                let view = this.annotations.view;
                let mut errors = Vec::new();
                let mut blamed = [false; 2];
                for (pane, result) in results {
                    blamed[pane] = true;
                    this.annotations.sides[pane] = match result {
                        Ok(blame) => {
                            let mut annotation = Annotation::default();
                            annotation.set_blame(blame);
                            annotation.view = view;
                            Some(annotation)
                        }
                        // A side the revision doesn't have (an added or
                        // deleted file) has no annotations.
                        Err(error) => {
                            errors.push(error.to_string());
                            None
                        }
                    };
                }
                if !texts_only {
                    for pane in 0..2 {
                        if !blamed[pane] {
                            this.annotations.sides[pane] = None;
                        }
                    }
                }
                let annotated = this.annotations.sides.iter().any(|s| s.as_ref().is_some_and(|a| !a.loading));
                this.annotations.error = (!annotated).then(|| errors.first().cloned()).flatten();
                cx.notify();
            })
            .ok();
        }));
    }

    /// The annotation column's width for a pane (0: none).
    pub(super) fn annotation_width(&self, pane: usize, window: &Window, cx: &App) -> f32 {
        match self.annotations.sides.get(pane).and_then(Option::as_ref) {
            Some(a) if self.annotations.on && !a.loading => {
                f32::from(a.pixel_width(cx.theme().font_family.clone(), px(crate::ui::diff_panes::font_size()), window))
            }
            _ => 0.,
        }
    }

    /// One line's annotation: date and author on a tint by age, the
    /// hovered commit marked; a click shows the commit in the Log.
    pub(super) fn annotation_cell(&self, pane: usize, line: usize, palette: &Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let annotation = self.annotations.sides.get(pane)?.as_ref().filter(|_| self.annotations.on)?;
        let blamed = annotation.blame.lines.get(line)?;
        let commit = annotation.blame.commits.get(blamed.commit)?;
        let uncommitted = blame::is_uncommitted(&commit.hash);
        let hovered = self.annotations.hovered.as_deref() == Some(commit.hash.as_str());
        let tint = if hovered {
            palette.selection.opacity(0.6)
        } else if uncommitted {
            palette.status_modified.opacity(0.18)
        } else {
            palette.accent.opacity(0.04 + 0.22 * annotation.age_of(blamed.commit))
        };
        let hash = commit.hash.clone();
        let tooltip = (!uncommitted && !self.annotations.menu_open).then(|| {
            let mut text = format!("{}\n{} <{}>  {}", commit.hash, commit.author, commit.author_email, common::format_full_date(commit.author_time));
            if !commit.summary.is_empty() {
                text.push_str(&format!("\n\n{}", commit.summary));
            }
            text
        });
        let (hover_hash, click_hash) = (hash.clone(), hash);
        Some(
            div()
                .id(ElementId::NamedInteger("diff-blame".into(), (pane * 10_000_000 + line) as u64))
                .size_full()
                .px(px(PADDING))
                .font_family(cx.theme().font_family.clone())
                .overflow_hidden()
                .whitespace_nowrap()
                .bg(tint)
                .text_color(if hovered { palette.text } else { palette.text_secondary })
                .cursor_pointer()
                .child(annotation.label(commit))
                .on_hover(cx.listener(move |this: &mut DiffView, on: &bool, _, cx| {
                    let annotations = &mut this.annotations;
                    if *on {
                        annotations.menu_open = false;
                        annotations.hovered = Some(hover_hash.clone());
                    } else if annotations.hovered.as_deref() == Some(hover_hash.as_str()) {
                        annotations.hovered = None;
                    } else {
                        return;
                    }
                    cx.notify();
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this: &mut DiffView, _, _, cx| {
                        this.annotations.menu_open = true;
                        cx.notify();
                    }),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |_, _, _, cx| {
                        cx.stop_propagation();
                        if !blame::is_uncommitted(&click_hash) {
                            cx.emit(super::DiffEvent::SelectInLog(click_hash.clone()));
                        }
                    }),
                )
                .when_some(tooltip, |el, tip| el.tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx)))
                .into_any_element(),
        )
    }

    /// The annotation column's menu for the commit on a line, as the
    /// editor's: Annotate Revision, Show Diff, Select in Git Log, Copy
    /// Revision Number, View, Close Annotations.
    pub(super) fn annotation_menu(entity: &Entity<Self>, line: Option<(usize, usize)>, menu: PopupMenu, window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
        let view = entity.read(cx);
        let commit = line.and_then(|(pane, l)| view.annotations.commit_at(pane, l)).cloned();
        let committed = commit.filter(|c| !blame::is_uncommitted(&c.hash));
        let hash = committed.as_ref().map(|c| c.hash.clone()).unwrap_or_default();
        let path = committed.as_ref().map(|c| c.path.clone()).unwrap_or_default();
        let previous = committed.as_ref().and_then(|c| c.previous.clone());
        let shown = view.annotations.view;
        let emit = |event: super::DiffEvent| {
            let entity = entity.clone();
            let event = Rc::new(Cell::new(Some(event)));
            move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
                if let Some(event) = event.take() {
                    entity.update(cx, |_, cx| cx.emit(event));
                }
            }
        };
        let view_entity = entity.clone();
        let set_view = move |label: &'static str, on: bool, edit: fn(&mut AnnotationView)| {
            let entity = view_entity.clone();
            let mut next = shown;
            edit(&mut next);
            PopupMenuItem::new(label).checked(on).on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| {
                    this.annotations.view = next;
                    for side in this.annotations.sides.iter_mut().flatten() {
                        side.view = next;
                    }
                    cx.notify();
                })
            })
        };
        let close = entity.clone();
        let copy = hash.clone();
        menu.item(
            PopupMenuItem::new("Annotate Revision")
                .disabled(committed.is_none())
                .on_click(emit(super::DiffEvent::Annotate { path: path.clone(), revision: Some(hash.clone()) })),
        )
        .item(PopupMenuItem::new("Annotate Previous Revision").disabled(previous.is_none()).on_click({
            let (hash, path) = previous.clone().unwrap_or_default();
            emit(super::DiffEvent::Annotate { path, revision: Some(hash) })
        }))
        .separator()
        .item(
            PopupMenuItem::new("Show Diff")
                .disabled(committed.is_none())
                .on_click(emit(super::DiffEvent::OpenDiff(DiffSource::Commit { hash: hash.clone(), path: path.clone(), old_path: None }))),
        )
        .item(PopupMenuItem::new("Select in Git Log").disabled(committed.is_none()).on_click(emit(super::DiffEvent::SelectInLog(hash.clone()))))
        .item(
            PopupMenuItem::new("Copy Revision Number")
                .disabled(committed.is_none())
                .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))),
        )
        .separator()
        .submenu("View", window, cx, move |m, _, _| {
            m.item(set_view("Date", shown.date, |v| v.date = !v.date))
                .item(set_view("Author", shown.author, |v| v.author = !v.author))
                .item(set_view("Commit Hash", shown.hash, |v| v.hash = !v.hash))
        })
        .separator()
        .item(PopupMenuItem::new("Close Annotations").on_click(move |_, _, cx| close.update(cx, |this, cx| this.close_annotations(cx))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_side_is_blamed_as_its_revision() {
        let commit = DiffSource::Commit { hash: "abc".into(), path: "b.kt".into(), old_path: Some("a.kt".into()) };
        assert_eq!(
            blame_targets(&commit),
            [Some(BlameOf::Revision { rev: "abc^".into(), path: "a.kt".into() }), Some(BlameOf::Revision { rev: "abc".into(), path: "b.kt".into() })]
        );
        let local = DiffSource::WorkingTree { path: "a.kt".into(), unversioned: false };
        assert_eq!(blame_targets(&local)[1], Some(BlameOf::Text { path: "a.kt".into() }));
        let new_file = DiffSource::WorkingTree { path: "a.kt".into(), unversioned: true };
        assert_eq!(blame_targets(&new_file)[1], None);
        let texts = DiffSource::Texts { path: "a".into(), old: String::new(), new: String::new(), old_title: String::new(), new_title: String::new() };
        assert_eq!(blame_targets(&texts), [None, None]);
    }
}
