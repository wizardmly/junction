//! The side-by-side viewer's context menu, and partial commits down to
//! single lines ("Include Lines into Commit" / "Exclude Lines from Commit").

use std::collections::HashSet;
use std::ops::Range;

use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::{App, Context, Entity, Window};

use super::DiffView;
use crate::git::diff::{self, Hunk};
use crate::ui::text_panes;

/// Whether a block's line range on one side is touched by selected lines
/// (an empty range by lines on either side of it).
fn touches(range: &Range<usize>, lines: &Range<usize>) -> bool {
    if range.is_empty() { lines.start <= range.start && range.start <= lines.end } else { range.start < lines.end && lines.start < range.end }
}

impl DiffView {
    /// Per change, the lines left out of the commit (old side, new side);
    /// empty when the diff takes no partial commits.
    pub(super) fn exclusions(&self, cx: &App) -> Vec<(Vec<bool>, Vec<bool>)> {
        let Some(path) = self.partial_path(cx) else { return Vec::new() };
        let excluded = crate::model::ExcludedHunks::get(cx).get(&path).cloned().unwrap_or_default();
        (0..self.diff.hunks.len())
            .map(|c| match self.signature(c) {
                Some(signature) => diff::excluded_lines(signature, &self.diff.hunks[c], &excluded),
                None => Default::default(),
            })
            .collect()
    }

    /// The changes the caret's selected lines touch, with those lines.
    fn selected_changes(&self) -> Vec<(usize, Range<usize>)> {
        let Some((pane, lines)) = self.panes.selected_lines() else { return Vec::new() };
        let side = |h: &Hunk| if pane == 0 { h.old.clone() } else { h.new.clone() };
        self.diff.hunks.iter().enumerate().filter(|(_, h)| touches(&side(h), &lines)).map(|(c, _)| (c, lines.clone())).collect()
    }

    /// Includes or excludes the selected lines of their changes: lines of
    /// the left pane are deletions, of the right pane insertions.
    fn set_lines_included(&mut self, include: bool, cx: &mut Context<Self>) {
        let (Some(path), Some((pane, _))) = (self.partial_path(cx), self.panes.selected_lines()) else { return };
        let new_side = pane == 1;
        let mut updates = Vec::new();
        for (change, lines) in self.selected_changes() {
            let (Some(signature), Some(hunk)) = (self.signature(change), self.diff.hunks.get(change)) else { continue };
            let range = if new_side { hunk.new.clone() } else { hunk.old.clone() };
            let offsets: Vec<usize> = range.clone().filter(|l| lines.contains(l)).map(|l| l - range.start).collect();
            updates.push((signature, hunk.clone(), offsets));
        }
        crate::model::ExcludedHunks::update(cx, |map| {
            let set = map.entry(path).or_default();
            for (signature, hunk, offsets) in updates {
                let (mut old, mut new) = diff::excluded_lines(signature, &hunk, set);
                let side = if new_side { &mut new } else { &mut old };
                for k in offsets {
                    side[k] = !include;
                }
                // Store the block whole when it is all in or all out.
                set.remove(&signature);
                for (k, _) in old.iter().enumerate() {
                    set.remove(&diff::line_id(signature, false, k));
                }
                for (k, _) in new.iter().enumerate() {
                    set.remove(&diff::line_id(signature, true, k));
                }
                if old.iter().chain(&new).all(|out| *out) {
                    set.insert(signature);
                } else {
                    set.extend(old.iter().enumerate().filter(|(_, out)| **out).map(|(k, _)| diff::line_id(signature, false, k)));
                    set.extend(new.iter().enumerate().filter(|(_, out)| **out).map(|(k, _)| diff::line_id(signature, true, k)));
                }
            }
        });
        cx.notify();
    }

    /// Reverts every change the selection touches, bottom-up so the
    /// earlier changes keep their numbers.
    fn revert_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let changes: HashSet<usize> = self.selected_changes().into_iter().map(|(c, _)| c).collect();
        let mut changes: Vec<usize> = changes.into_iter().collect();
        changes.sort_unstable_by(|a, b| b.cmp(a));
        for change in changes {
            self.revert_change(change, false, window, cx);
        }
    }

    /// The panes' context menu.
    pub(super) fn context_menu(entity: &Entity<Self>, menu: PopupMenu, cx: &mut Context<PopupMenu>) -> PopupMenu {
        let view = entity.read(cx);
        let selected = !view.selected_changes().is_empty();
        let revert = view.editable() && selected;
        let partial = view.partial_path(cx).is_some() && selected;
        let jump = view.jump_target().is_some();
        let on = |f: fn(&mut DiffView, &mut Window, &mut Context<DiffView>)| {
            let entity = entity.clone();
            move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| entity.update(cx, |view, cx| f(view, window, cx))
        };
        text_panes::edit_menu(menu, entity, cx)
            .separator()
            .item(PopupMenuItem::new("Revert Selected Changes").disabled(!revert).on_click(on(|v, w, cx| v.revert_selected(w, cx))))
            .item(PopupMenuItem::new("Include Lines into Commit").disabled(!partial).on_click(on(|v, _, cx| v.set_lines_included(true, cx))))
            .item(PopupMenuItem::new("Exclude Lines from Commit").disabled(!partial).on_click(on(|v, _, cx| v.set_lines_included(false, cx))))
            .separator()
            .item(PopupMenuItem::new("Jump to Source").disabled(!jump).on_click(on(|v, _, cx| v.jump_to_source(cx))))
    }
}
