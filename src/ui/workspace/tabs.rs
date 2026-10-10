//! Editor tabs, after IntelliJ's: one tab per open file plus the diff,
//! merge and pull request views shown in the editor area.
//! Pinned tabs come first; closing a tab activates its left neighbour;
//! past the tab limit the least recently used unpinned tab closes.
//!
//! Split Right / Down adds a second tab group. The focused group's tabs live
//! in `Workspace::editors` / `front` and the other group's in `split`; focusing
//! a group swaps them, so every tab operation works on the focused group.

use gpui_kit::assets::IconName;
use gpui_kit::component::{h_flex, menu::{ContextMenuExt as _, PopupMenuItem}};
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px,
};

use super::Workspace;
use crate::theme::ActivePalette as _;
use crate::ui::common;
use crate::ui::file_editor::FileEditor;

/// IntelliJ's default "Tab limit".
const TAB_LIMIT: usize = 10;

pub(super) struct EditorTab {
    pub view: Entity<FileEditor>,
    pub _subscription: Subscription,
    pub pinned: bool,
    /// IntelliJ's preview tab (italic): the next preview replaces it.
    pub preview: bool,
    /// When the tab was last active, for the tab limit.
    pub used: u64,
}

/// What the editor area shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Front {
    Editor(usize),
    Diff,
    Merge,
    Timeline,
}

/// The tab group not focused, after Split Right / Down.
pub(super) struct SplitGroup {
    pub editors: Vec<EditorTab>,
    pub front: Front,
    /// Split Down (one above the other) rather than Split Right.
    pub vertical: bool,
}

/// A tab being dragged to a new place.
#[derive(Clone)]
struct DraggedTab {
    group: usize,
    ix: usize,
    title: SharedString,
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        div().px_2().py_1().rounded(px(4.)).bg(palette.selection).border_1().border_color(palette.accent).text_sm().text_color(palette.text).child(self.title.clone())
    }
}

/// Runs `f` on the workspace with tab group `group` focused.
fn grp(entity: &Entity<Workspace>, group: usize, cx: &mut gpui_kit::App, f: impl FnOnce(&mut Workspace, &mut Context<Workspace>)) {
    entity.update(cx, |this, cx| {
        this.focus_group(group, cx);
        f(this, cx)
    })
}

impl Workspace {
    /// Swaps the focused and the other tab group.
    pub(super) fn swap_split(&mut self) {
        if let Some(split) = &mut self.split {
            std::mem::swap(&mut self.editors, &mut split.editors);
            std::mem::swap(&mut self.front, &mut split.front);
            self.active_group = 1 - self.active_group;
        }
    }

    /// Focuses tab group 0 (left / top) or 1.
    pub(super) fn focus_group(&mut self, group: usize, cx: &mut Context<Self>) {
        if group != self.active_group && self.split.is_some() {
            self.swap_split();
            cx.notify();
        }
    }

    /// Split Right / Down (`move_it`: Split and Move), or Move to Opposite Group.
    fn split_tab(&mut self, ix: usize, vertical: bool, move_it: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.editors.get(ix) else { return };
        let path = tab.view.read(cx).path().to_owned();
        let revision = tab.view.read(cx).revision().map(str::to_owned);
        let from = self.active_group;
        if self.split.is_none() {
            self.split = Some(SplitGroup { editors: Vec::new(), front: Front::Diff, vertical });
        }
        self.swap_split();
        self.open_tab(path.clone(), revision.clone(), window, cx);
        let to = self.active_group;
        if move_it {
            self.focus_group(from, cx);
            if let Some(ix) = self.editors.iter().position(|t| t.view.read(cx).path() == path && t.view.read(cx).revision() == revision.as_deref()) {
                self.close_tabs(vec![ix], cx);
            }
            // The source group may have closed and the groups renumbered.
            let target = if self.split.is_some() { to } else { 0 };
            self.focus_group(target, cx);
        }
        if let Some(ix) = self.editors.iter().position(|t| t.view.read(cx).path() == path && t.view.read(cx).revision() == revision.as_deref()) {
            self.activate(Front::Editor(ix), window, cx);
        }
        cx.notify();
    }

    /// Unsplit: the other group's tabs join this one.
    fn unsplit(&mut self, cx: &mut Context<Self>) {
        let Some(split) = self.split.take() else { return };
        for tab in split.editors {
            let (path, rev) = (tab.view.read(cx).path().to_owned(), tab.view.read(cx).revision().map(str::to_owned));
            if !self.editors.iter().any(|t| t.view.read(cx).path() == path && t.view.read(cx).revision() == rev.as_deref()) {
                self.editors.push(tab);
            }
        }
        self.active_group = 0;
        self.fix_front(cx);
        cx.notify();
    }

    /// A tab group left empty goes away.
    fn collapse_empty_group(&mut self, cx: &mut Context<Self>) {
        if self.split.is_none() || !self.editors.is_empty() {
            return;
        }
        if self.active_group == 0 && self.has_specials(cx) {
            return;
        }
        let split = self.split.take().unwrap();
        self.editors = split.editors;
        self.front = split.front;
        self.active_group = 0;
    }

    fn has_specials(&self, cx: &gpui_kit::App) -> bool {
        self.diff.read(cx).title().is_some() || self.merge.is_some() || self.timeline.is_some()
    }

    /// The file editor the editor area shows, if it shows one.
    pub(super) fn editor(&self) -> Option<&Entity<FileEditor>> {
        match self.front {
            Front::Editor(ix) => self.editors.get(ix).map(|t| &t.view),
            _ => None,
        }
    }

    fn tab_paths(&self, cx: &gpui_kit::App) -> Vec<String> {
        let other = self.split.iter().flat_map(|s| s.editors.iter());
        self.editors.iter().chain(other).filter(|t| t.view.read(cx).revision().is_none()).map(|t| t.view.read(cx).path().to_owned()).collect()
    }

    /// The tabs in display order.
    fn tab_list(&self, cx: &gpui_kit::App) -> Vec<Front> {
        let mut tabs: Vec<Front> = (0..self.editors.len()).map(Front::Editor).collect();
        // The diff, merge, annotate and PR views live in the first group.
        if self.active_group != 0 {
            return tabs;
        }
        if self.diff.read(cx).title().is_some() {
            tabs.push(Front::Diff);
        }
        if self.merge.is_some() {
            tabs.push(Front::Merge);
        }
        if self.timeline.is_some() {
            tabs.push(Front::Timeline);
        }
        tabs
    }

    /// Opens `path` in a tab, or switches to its tab.
    /// Enable Preview Tab: shows the file in the preview tab, replacing the
    /// file previewed before unless it was edited; an open file just activates.
    pub(super) fn open_preview(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.editors.iter().position(|t| t.view.read(cx).path() == path && t.view.read(cx).revision().is_none()) {
            self.activate_tab(ix, cx);
            return;
        }
        let old = self.editors.iter().position(|t| t.preview && !t.pinned && !t.view.read(cx).is_dirty(cx));
        if let Some(old) = old {
            self.remove_tab(old, cx);
        }
        self.open_tab(path.clone(), None, window, cx);
        let Some(ix) = self.editors.iter().position(|t| t.view.read(cx).path() == path && t.view.read(cx).revision().is_none()) else { return };
        self.editors[ix].preview = true;
        // The new preview takes the old one's place.
        if let Some(old) = old.filter(|old| *old != ix && *old < self.editors.len()) {
            self.move_tab(ix, old, cx);
        }
        if old.is_some() {
            // Previewing doesn't fill Reopen Closed Tab.
            self.closed_tabs.pop();
        }
        cx.notify();
    }

    /// A preview tab that was edited or opened for real stays.
    pub(super) fn keep_tab(&mut self, view: &Entity<FileEditor>, cx: &mut Context<Self>) {
        if let Some(tab) = self.editors.iter_mut().find(|t| t.view == *view && t.preview) {
            tab.preview = false;
            cx.notify();
        }
    }

    pub(super) fn open_tab(&mut self, path: String, revision: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        // Working-tree files are relative to the project; a revision comes
        // from the active repository (the Log's selected root).
        let model = self.model.read(cx);
        let repository = if revision.is_some() { model.repository() } else { model.project_repository() };
        let Some(repository) = repository.cloned() else { return };
        let existing = self.editors.iter().position(|t| t.view.read(cx).path() == path && t.view.read(cx).revision() == revision.as_deref());
        let ix = match existing {
            Some(ix) => {
                // Opening a previewed file for real (double click) keeps its tab.
                self.editors[ix].preview = false;
                ix
            }
            None => {
                let view = cx.new(|cx| FileEditor::new(repository, path, revision, window, cx));
                let subscription = cx.subscribe_in(&view, window, Self::on_file_editor_event);
                let index = self.code_index.clone();
                view.update(cx, |editor, cx| editor.attach_index(index, cx));
                // New tabs open right of the current one, after the pinned ones.
                let pinned = self.editors.iter().filter(|t| t.pinned).count();
                let at = match self.front {
                    Front::Editor(ix) => ix + 1,
                    _ => self.editors.len(),
                }
                .max(pinned)
                .min(self.editors.len());
                let id = view.entity_id();
                self.editors.insert(at, EditorTab { view, _subscription: subscription, pinned: false, preview: false, used: 0 });
                if let Front::Editor(active) = &mut self.front {
                    if *active >= at {
                        *active += 1;
                    }
                }
                self.enforce_limit(at, cx);
                self.editors.iter().position(|t| t.view.entity_id() == id).unwrap_or(0)
            }
        };
        self.activate_tab(ix, cx);
    }

    /// Past the tab limit, closes the least recently used unpinned, unmodified tab.
    fn enforce_limit(&mut self, keep: usize, cx: &mut Context<Self>) {
        while self.editors.len() > TAB_LIMIT {
            let victim = self
                .editors
                .iter()
                .enumerate()
                .filter(|(i, t)| *i != keep && !t.pinned && !t.view.read(cx).is_dirty(cx))
                .min_by_key(|(_, t)| t.used)
                .map(|(i, _)| i);
            let Some(victim) = victim else { break };
            self.remove_tab(victim, cx);
        }
    }

    pub(super) fn activate_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.editors.get_mut(ix) else { return };
        self.tab_clock += 1;
        tab.used = self.tab_clock;
        self.front = Front::Editor(ix);
        let view = tab.view.clone();
        if view.read(cx).revision().is_none() {
            let path = view.read(cx).path().to_owned();
            let open = self.tab_paths(cx);
            self.project.update(cx, |project, cx| project.file_opened(&path, open, cx));
        }
        cx.notify();
    }

    fn activate(&mut self, front: Front, window: &mut Window, cx: &mut Context<Self>) {
        match front {
            Front::Editor(ix) => {
                self.activate_tab(ix, cx);
                if let Some(tab) = self.editors.get(ix) {
                    let view = tab.view.clone();
                    view.update(cx, |editor, cx| editor.focus(window, cx));
                }
            }
            other => {
                self.front = other;
                cx.notify();
            }
        }
    }

    /// Removes a tab without saving; keeps `front` pointing at the same tab.
    pub(super) fn remove_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.editors.len() {
            return;
        }
        let tab = self.editors.remove(ix);
        if tab.view.read(cx).revision().is_none() {
            let path = tab.view.read(cx).path().to_owned();
            self.closed_tabs.retain(|p| *p != path);
            self.closed_tabs.push(path);
        }
        if let Front::Editor(active) = self.front {
            if active > ix {
                self.front = Front::Editor(active - 1);
            } else if active == ix {
                // IntelliJ activates the tab on the left.
                self.front = if self.editors.is_empty() { Front::Diff } else { Front::Editor(ix.saturating_sub(1).min(self.editors.len() - 1)) };
            }
        }
        let open = self.tab_paths(cx);
        self.project.update(cx, |project, cx| project.set_open_files(open, cx));
    }

    /// Closes tabs (saving their text first, as IntelliJ's autosave does).
    pub(super) fn close_tabs(&mut self, mut tabs: Vec<usize>, cx: &mut Context<Self>) {
        tabs.sort_unstable();
        tabs.dedup();
        for ix in tabs.into_iter().rev() {
            if let Some(tab) = self.editors.get(ix) {
                tab.view.update(cx, |editor, cx| editor.save_now(cx));
            }
            self.remove_tab(ix, cx);
        }
        self.collapse_empty_group(cx);
        self.fix_front(cx);
        if let Front::Editor(ix) = self.front {
            self.activate_tab(ix, cx);
        }
        cx.notify();
    }

    /// After a view closes: show the last active tab instead.
    pub(super) fn fix_front(&mut self, cx: &mut Context<Self>) {
        let ok = match self.front {
            Front::Editor(ix) => ix < self.editors.len(),
            _ if self.active_group != 0 => self.editors.is_empty(),
            Front::Diff => self.diff.read(cx).title().is_some() || self.editors.is_empty(),
            Front::Merge => self.merge.is_some(),
            Front::Timeline => self.timeline.is_some(),
        };
        if ok {
            return;
        }
        let recent = self.editors.iter().enumerate().max_by_key(|(_, t)| t.used).map(|(i, _)| i);
        self.front = match recent {
            Some(ix) => Front::Editor(ix),
            None => self.tab_list(cx).last().copied().unwrap_or(Front::Diff),
        };
    }

    fn close_front(&mut self, front: Front, cx: &mut Context<Self>) {
        match front {
            Front::Editor(ix) => return self.close_tabs(vec![ix], cx),
            Front::Diff => self.diff.update(cx, |diff, cx| diff.clear(cx)),
            Front::Merge => self.merge = None,
            Front::Timeline => self.timeline = None,
        }
        self.fix_front(cx);
        cx.notify();
    }

    pub(super) fn close_active_tab(&mut self, cx: &mut Context<Self>) {
        let front = self.front;
        if self.tab_list(cx).contains(&front) {
            self.close_front(front, cx);
        }
    }

    pub(super) fn step_tab(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let tabs = self.tab_list(cx);
        let Some(pos) = tabs.iter().position(|t| *t == self.front) else { return };
        let n = tabs.len() as isize;
        let next = tabs[((pos as isize + delta).rem_euclid(n)) as usize];
        self.activate(next, window, cx);
    }

    pub(super) fn reopen_closed_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        while let Some(path) = self.closed_tabs.pop() {
            if self.editors.iter().any(|t| t.view.read(cx).path() == path && t.view.read(cx).revision().is_none()) {
                continue;
            }
            self.open_file(path, None, window, cx);
            return;
        }
    }

    fn toggle_pin(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.editors.get_mut(ix) else { return };
        tab.pinned = !tab.pinned;
        let active = match self.front {
            Front::Editor(a) => Some(self.editors[a].view.entity_id()),
            _ => None,
        };
        // Pinned tabs move to the front, keeping their order.
        let mut tabs = std::mem::take(&mut self.editors);
        tabs.sort_by_key(|t| !t.pinned);
        self.editors = tabs;
        if let Some(id) = active {
            if let Some(ix) = self.editors.iter().position(|t| t.view.entity_id() == id) {
                self.front = Front::Editor(ix);
            }
        }
        cx.notify();
    }

    fn move_tab(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        if from == to || from >= self.editors.len() || to >= self.editors.len() {
            return;
        }
        let active = match self.front {
            Front::Editor(a) => Some(self.editors[a].view.entity_id()),
            _ => None,
        };
        let mut tab = self.editors.remove(from);
        // Dropped among the pinned tabs it becomes pinned, and the other way round.
        let pinned_before = self.editors.iter().filter(|t| t.pinned).count();
        tab.pinned = to < pinned_before || (tab.pinned && to == pinned_before);
        self.editors.insert(to, tab);
        if let Some(id) = active {
            if let Some(ix) = self.editors.iter().position(|t| t.view.entity_id() == id) {
                self.front = Front::Editor(ix);
            }
        }
        cx.notify();
    }

    fn tab_title(&self, front: Front, cx: &gpui_kit::App) -> (String, IconName, Option<String>) {
        match front {
            Front::Editor(ix) => {
                let editor = self.editors[ix].view.read(cx);
                let path = editor.path();
                let name = path.rsplit('/').next().unwrap_or(path);
                // Same names get their folder, as "build.gradle (app)".
                let clash = self.editors.iter().enumerate().any(|(i, t)| i != ix && t.view.read(cx).path().rsplit('/').next() == Some(name));
                let mut title = name.to_owned();
                if clash {
                    let parent = path.rsplit('/').nth(1).unwrap_or("");
                    if !parent.is_empty() {
                        title = format!("{title} ({parent})");
                    }
                }
                if let Some(rev) = editor.revision() {
                    title = format!("{title} [{}]", &rev[..rev.len().min(8)]);
                }
                (title, common::file_icon(path), Some(path.to_owned()))
            }
            Front::Diff => (self.diff.read(cx).title().unwrap_or_default(), IconName::FileDiff, None),
            Front::Merge => {
                let path = self.merge.as_ref().map(|(m, _)| m.read(cx).path().to_owned()).unwrap_or_default();
                (format!("Merge: {}", path.rsplit('/').next().unwrap_or(&path)), IconName::GitMerge, None)
            }
            Front::Timeline => {
                let title = self.timeline.as_ref().map(|t| t.read(cx).title()).unwrap_or_else(|| "Pull Request".to_owned());
                (title, IconName::GitPullRequest, None)
            }
        }
    }

    /// The focused group's tab bar; `focused` is false while drawing the other group swapped in.
    pub(super) fn render_tab_bar(&self, focused: bool, cx: &mut Context<Self>) -> Option<gpui_kit::AnyElement> {
        let tabs = self.tab_list(cx);
        if tabs.is_empty() {
            return None;
        }
        let group = self.active_group;
        let split = self.split.as_ref().map(|s| s.vertical);
        let palette = cx.palette().clone();
        let status: std::collections::HashMap<String, crate::git::status::StatusKind> =
            self.model.read(cx).project_status().entries.iter().map(|e| (e.path.clone(), e.kind)).collect();
        let entity = cx.entity();
        let count = self.editors.len();
        let row = h_flex()
            .id(("editor-tabs", group))
            .h(px(crate::ui::common::header_height()))
            .flex_shrink_0()
            .overflow_x_scroll()
            .border_b_1()
            .border_color(palette.border)
            .bg(palette.toolbar)
            .children(tabs.into_iter().enumerate().map(|(n, front)| {
                let active = front == self.front;
                let (title, icon, path) = self.tab_title(front, cx);
                let editor_ix = match front {
                    Front::Editor(ix) => Some(ix),
                    _ => None,
                };
                let pinned = editor_ix.is_some_and(|ix| self.editors[ix].pinned);
                let preview = editor_ix.is_some_and(|ix| self.editors[ix].preview);
                let dirty = editor_ix.is_some_and(|ix| self.editors[ix].view.read(cx).is_dirty(cx));
                let color = path.as_ref().and_then(|p| status.get(p)).map(|k| common::status_color(*k, &palette));
                let title_s: SharedString = title.clone().into();
                h_flex()
                    .id(("editor-tab", n))
                    .group(SharedString::from(format!("editor-tab-{group}")))
                    .h_full()
                    .pl_2()
                    .pr_1()
                    .gap_1()
                    .flex_shrink_0()
                    .text_sm()
                    .cursor_pointer()
                    .border_r_1()
                    .border_color(palette.border)
                    .relative()
                    .when(active, |el| {
                        el.bg(palette.panel).child(
                            div().absolute().left_0().right_0().bottom_0().h(px(2.)).bg(if focused { palette.accent } else { palette.text_secondary }),
                        )
                    })
                    .when(!active, |el| el.hover(|el| el.bg(palette.hover)))
                    .child(common::icon(icon).text_color(palette.text_secondary))
                    .child(
                        div()
                            .whitespace_nowrap()
                            // Linux's default UI font has no italic face; Inter does.
                            .when(preview, |el| el.italic().when(cfg!(target_os = "linux"), |el| el.font_family("Inter")))
                            .when_some(color, |el, c| el.text_color(c))
                            .child(if dirty { format!("{title} •") } else { title.clone() }),
                    )
                    .when(pinned, |el| el.child(common::icon(IconName::Pin).text_color(palette.text_secondary)))
                    .child(
                        div()
                            .id(("editor-tab-close", n))
                            .p(px(2.))
                            .rounded(px(3.))
                            .hover(|el| el.bg(palette.hover))
                            .when(!active, |el| el.invisible().group_hover(SharedString::from(format!("editor-tab-{group}")), |el| el.visible()))
                            .child(common::icon(IconName::X).text_color(palette.text_secondary))
                            .on_click({
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    cx.stop_propagation();
                                    grp(&entity, group, cx, |this, cx| this.close_front(front, cx));
                                }
                            }),
                    )
                    .on_click({
                        let entity = entity.clone();
                        move |event: &gpui_kit::ClickEvent, window, cx| {
                            let keep = event.click_count() >= 2;
                            grp(&entity, group, cx, |this, cx| {
                                // Double-clicking a preview tab keeps it.
                                if let (true, Some(ix)) = (keep, editor_ix) {
                                    this.editors[ix].preview = false;
                                }
                                this.activate(front, window, cx)
                            })
                        }
                    })
                    .on_mouse_down(MouseButton::Middle, {
                        let entity = entity.clone();
                        move |_, _, cx| grp(&entity, group, cx, |this, cx| this.close_front(front, cx))
                    })
                    .when_some(editor_ix, |el, ix| {
                        el.on_drag(DraggedTab { group, ix, title: title_s.clone() }, |tab, _, _, cx| cx.new(|_| tab.clone()))
                            .drag_over::<DraggedTab>(move |el, _, _, _| el.bg(palette.selection))
                            .on_drop({
                                let entity = entity.clone();
                                move |tab: &DraggedTab, window, cx| {
                                    let tab = tab.clone();
                                    if tab.group == group {
                                        grp(&entity, group, cx, |this, cx| this.move_tab(tab.ix, ix, cx))
                                    } else {
                                        // Dropped on the other group: the tab moves there.
                                        grp(&entity, tab.group, cx, |this, cx| {
                                            let vertical = this.split.as_ref().is_some_and(|s| s.vertical);
                                            this.split_tab(tab.ix, vertical, true, window, cx)
                                        })
                                    }
                                }
                            })
                    })
                    .context_menu({
                        let entity = entity.clone();
                        let path = path.clone();
                        move |menu, _, _| {
                            let close = |label: &'static str, enabled: bool, which: fn(usize, usize) -> bool| {
                                let entity = entity.clone();
                                PopupMenuItem::new(label).disabled(!enabled).on_click(move |_, _, cx| {
                                    grp(&entity, group, cx, |this, cx| {
                                        let ix = editor_ix.unwrap_or(usize::MAX);
                                        let tabs: Vec<usize> = (0..this.editors.len()).filter(|&i| which(i, ix) && !(this.editors[i].pinned && i != ix && label != "Close All Tabs")).collect();
                                        this.close_tabs(tabs, cx);
                                    })
                                })
                            };
                            let Some(ix) = editor_ix else {
                                let entity = entity.clone();
                                return menu.item(PopupMenuItem::new("Close").on_click(move |_, _, cx| grp(&entity, group, cx, |this, cx| this.close_front(front, cx))));
                            };
                            let (e1, e2, e3, e4) = (entity.clone(), entity.clone(), entity.clone(), entity.clone());
                            let path = path.clone().unwrap_or_default();
                            let (p1, p2) = (path.clone(), path.clone());
                            menu.item(close("Close", true, |i, ix| i == ix))
                                .item(close("Close Other Tabs", count > 1, |i, ix| i != ix))
                                .item(close("Close Tabs to the Left", ix > 0, |i, ix| i < ix))
                                .item(close("Close Tabs to the Right", ix + 1 < count, |i, ix| i > ix))
                                .item(close("Close All Tabs", true, |_, _| true))
                                .item({
                                    let entity = e4.clone();
                                    PopupMenuItem::new("Close All but Pinned").on_click(move |_, _, cx| {
                                        grp(&entity, group, cx, |this, cx| {
                                            let tabs = (0..this.editors.len()).filter(|&i| !this.editors[i].pinned).collect();
                                            this.close_tabs(tabs, cx);
                                        })
                                    })
                                })
                                .separator()
                                .item(PopupMenuItem::new(if pinned { "Unpin Tab" } else { "Pin Tab" }).on_click(move |_, _, cx| grp(&e1, group, cx, |this, cx| this.toggle_pin(ix, cx))))
                                .separator()
                                .map(|menu| {
                                    let item = |label: &'static str, vertical: bool, move_it: bool, enabled: bool| {
                                        let entity = entity.clone();
                                        PopupMenuItem::new(label).disabled(!enabled).on_click(move |_, window, cx| {
                                            grp(&entity, group, cx, |this, cx| this.split_tab(ix, vertical, move_it, window, cx))
                                        })
                                    };
                                    match split {
                                        None => menu
                                            .item(item("Split Right", false, false, true))
                                            .item(item("Split Down", true, false, true))
                                            .item(item("Split and Move Right", false, true, count > 1))
                                            .item(item("Split and Move Down", true, true, count > 1)),
                                        Some(vertical) => {
                                            let entity = entity.clone();
                                            menu.item(item("Open in Opposite Group", vertical, false, true))
                                                .item(item("Move to Opposite Group", vertical, true, true))
                                                .item(PopupMenuItem::new("Unsplit").on_click(move |_, _, cx| grp(&entity, group, cx, |this, cx| this.unsplit(cx))))
                                        }
                                    }
                                })
                                .separator()
                                .item(PopupMenuItem::new("Copy Path").on_click(move |_, _, cx| {
                                    let full = e2.read(cx).code_index.read(cx).root().map(|r| r.join(&p1).display().to_string()).unwrap_or(p1.clone());
                                    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(full));
                                }))
                                .item(PopupMenuItem::new("Copy Path From Repository Root").on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(p2.clone()))
                                }))
                                .item(PopupMenuItem::new("Select in Project View").on_click(move |_, window, cx| {
                                    grp(&e3, group, cx, |this, cx| {
                                        this.activate(front, window, cx);
                                        this.select_in_project(&super::SelectInProject, window, cx);
                                    })
                                }))
                                .separator()
                                .item({
                                    let entity = entity.clone();
                                    PopupMenuItem::new("Reopen Closed Tab").on_click(move |_, window, cx| grp(&entity, group, cx, |this, cx| this.reopen_closed_tab(window, cx)))
                                })
                        }
                    })
                    .into_any_element()
            }));
        Some(row.into_any_element())
    }
}
