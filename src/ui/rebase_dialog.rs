//! IntelliJ's "Rebasing Commits" dialog for interactive rebase: the commits
//! (newest first, as in the Log), an action per commit, reordering, and a
//! message editor for Reword / Squash.

use gpui_kit::component::{
    Disableable as _, Icon, Sizable as _, WindowExt as _, h_flex,
    button::{Button, ButtonVariants as _},
    dialog::{DialogAction, DialogClose, DialogFooter},
    input::{InputEvent, Textarea, TextareaState},
    v_flex,
};
use gpui_kit::assets::IconName;
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _,
    px,
};

use crate::git::rebase::{self, Action, Entry};
use crate::git::Repository;
use crate::model::RepoModel;
use crate::theme::ActivePalette as _;
use crate::ui::common::tool_button;

/// A row being dragged to a new position.
#[derive(Clone)]
struct DraggedRow {
    ix: usize,
    subject: SharedString,
}

impl Render for DraggedRow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        div()
            .px_2()
            .py_1()
            .rounded(px(4.))
            .bg(palette.selection)
            .border_1()
            .border_color(palette.accent)
            .text_sm()
            .text_color(palette.text)
            .child(self.subject.clone())
    }
}

pub struct RebaseEditor {
    repository: Repository,
    /// Newest first, as displayed.
    entries: Vec<Entry>,
    original: Vec<Entry>,
    /// The row the message editor and the changes show (the last clicked).
    selected: usize,
    /// Selected rows: Ctrl/Cmd-click adds one, Shift-click a range from
    /// `anchor`. Actions apply to all of them, as in IntelliJ.
    selection: std::collections::BTreeSet<usize>,
    anchor: usize,
    message: Entity<TextareaState>,
    focus: FocusHandle,
    /// The changed files of each commit looked at, for the details panel.
    files: std::collections::HashMap<String, Vec<crate::git::log::FileChange>>,
    _subscription: Subscription,
}

impl RebaseEditor {
    fn new(repository: Repository, entries: Vec<Entry>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let message = cx.new(|cx| TextareaState::new(window, cx).rows(6));
        let subscription = cx.subscribe(&message, |this: &mut Self, message, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let text = message.read(cx).value().to_string();
                if let Some(entry) = this.entries.get_mut(this.selected) {
                    if matches!(entry.action, Action::Reword | Action::Squash) {
                        entry.message = Some(text);
                        cx.notify();
                    }
                }
            }
        });
        let mut this = Self {
            repository,
            original: entries.clone(),
            entries,
            selected: 0,
            selection: [0].into(),
            anchor: 0,
            message,
            focus: cx.focus_handle(),
            files: Default::default(),
            _subscription: subscription,
        };
        this.load_message(window, cx);
        this
    }

    fn load_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.entries.get(self.selected) else { return };
        let hash = entry.commit.hash.clone();
        if !self.files.contains_key(&hash) {
            let changes = crate::git::log::load_details(&self.repository, &hash).map(|d| d.changes).unwrap_or_default();
            self.files.insert(hash, changes);
        }
        let text = entry.message.clone().unwrap_or_else(|| rebase::message_of(&self.repository, &entry.commit.hash));
        self.message.update(cx, |state, cx| state.set_value(text, window, cx));
    }

    fn rows(&self) -> Vec<usize> {
        self.selection.iter().copied().filter(|&ix| ix < self.entries.len()).collect()
    }

    fn select(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = ix.min(self.entries.len().saturating_sub(1));
        self.anchor = self.selected;
        self.selection = [self.selected].into();
        self.load_message(window, cx);
        cx.notify();
    }

    /// A click on a row: plain selects it, Ctrl/Cmd adds or removes it,
    /// Shift selects the range from the last plain click.
    fn click(&mut self, ix: usize, modifiers: gpui_kit::Modifiers, window: &mut Window, cx: &mut Context<Self>) {
        if modifiers.shift {
            let (from, to) = (self.anchor.min(ix), self.anchor.max(ix));
            self.selection = (from..=to).collect();
        } else if modifiers.secondary() {
            if !self.selection.remove(&ix) {
                self.selection.insert(ix);
            }
            if self.selection.is_empty() {
                self.selection.insert(ix);
            }
            self.anchor = ix;
        } else {
            return self.select(ix, window, cx);
        }
        self.selected = ix;
        self.load_message(window, cx);
        cx.notify();
    }

    /// The toolbar's actions on every selected row. Squash and Fixup with
    /// several rows selected unite them; on one row they fold it into the
    /// commit below.
    fn set_action(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.rows();
        if rows.len() > 1 && matches!(action, Action::Squash | Action::Fixup) {
            return self.unite(action, window, cx);
        }
        for ix in rows {
            // The oldest commit has nothing below it to squash into.
            let oldest = ix + 1 == self.entries.len();
            if oldest && matches!(action, Action::Squash | Action::Fixup) {
                continue;
            }
            self.entries[ix].action = action;
            if action == Action::Squash && self.entries[ix].message.is_none() {
                // Squash proposes both messages, like git's default.
                let joined = format!(
                    "{}\n\n{}",
                    rebase::message_of(&self.repository, &self.entries[ix + 1].commit.hash),
                    rebase::message_of(&self.repository, &self.entries[ix].commit.hash)
                );
                self.entries[ix].message = Some(joined);
            }
        }
        self.load_message(window, cx);
        cx.notify();
    }

    /// Reword (also a double-click): the message editor takes the new message.
    fn reword(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.selection = [self.selected].into();
        self.set_action(Action::Reword, window, cx);
        self.message.update(cx, |state, cx| state.focus(window, cx));
    }

    /// IntelliJ's Unite: the selected commits become one, folded into the
    /// oldest of them, which keeps its place.
    fn unite(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.rows();
        let Some(range) = rebase::unite(&mut self.entries, &rows, action) else { return };
        if action == Action::Squash {
            // The united commit's message: every message, oldest first.
            let messages: Vec<String> = self.entries[range.clone()]
                .iter()
                .rev()
                .map(|e| e.message.clone().unwrap_or_else(|| rebase::message_of(&self.repository, &e.commit.hash)))
                .collect();
            self.entries[range.start].message = Some(messages.join("\n\n"));
        }
        self.selected = range.start;
        self.anchor = range.start;
        self.selection = range.collect();
        self.load_message(window, cx);
        cx.notify();
    }

    /// Drag and drop: moves the dragged row (with the rest of the selection
    /// when it is part of it) onto row `to`.
    fn drop_rows(&mut self, from: usize, to: usize, window: &mut Window, cx: &mut Context<Self>) {
        let rows = if self.selection.contains(&from) { self.rows() } else { vec![from] };
        let moved = rebase::move_rows(&mut self.entries, &rows, to);
        self.after_move(moved, window, cx);
    }

    /// Move Up / Move Down (Alt+Up / Alt+Down): the selected rows by one.
    fn move_selected(&mut self, up: bool, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.rows();
        let (Some(&first), Some(&last)) = (rows.first(), rows.last()) else { return };
        let to = if up { first.checked_sub(1) } else { (last + 1 < self.entries.len()).then_some(last + 1) };
        if let Some(to) = to {
            let moved = rebase::move_rows(&mut self.entries, &rows, to);
            self.after_move(moved, window, cx);
        }
    }

    fn after_move(&mut self, moved: Vec<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(&first) = moved.first() else { return };
        self.selected = first;
        self.anchor = first;
        self.selection = moved.into_iter().collect();
        self.load_message(window, cx);
        cx.notify();
    }

    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.entries = self.original.clone();
        self.select(0, window, cx);
    }

    /// The dialog's shortcuts, as in IntelliJ: Alt+P / E / R / S / F / D
    /// (Delete drops too), Alt+Up / Alt+Down, and Up / Down to move the selection.
    fn on_key(&mut self, event: &gpui_kit::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Typing in the message editor is its own.
        if !self.focus.is_focused(window) {
            return;
        }
        let k = &event.keystroke;
        let alt = k.modifiers.alt && !k.modifiers.control && !k.modifiers.platform;
        let action = match k.key.as_str() {
            "p" if alt => Some(Action::Pick),
            "e" if alt => Some(Action::Edit),
            "s" if alt => Some(Action::Squash),
            "f" if alt => Some(Action::Fixup),
            "d" if alt => Some(Action::Drop),
            "delete" | "backspace" if !alt => Some(Action::Drop),
            _ => None,
        };
        match (action, k.key.as_str()) {
            (Some(action), _) => self.set_action(action, window, cx),
            (None, "r") if alt => self.reword(window, cx),
            (None, "up") if alt => self.move_selected(true, window, cx),
            (None, "down") if alt => self.move_selected(false, window, cx),
            (None, "up") if self.selected > 0 => {
                let ix = self.selected - 1;
                self.click(ix, k.modifiers, window, cx)
            }
            (None, "down") if self.selected + 1 < self.entries.len() => {
                let ix = self.selected + 1;
                self.click(ix, k.modifiers, window, cx)
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    /// Entries in git's order (oldest first). Messages are kept only where
    /// they were edited, so untouched commits keep their messages exactly.
    fn todo_entries(&self) -> Vec<Entry> {
        self
            .entries
            .iter()
            .rev()
            .map(|e| {
                let mut e = e.clone();
                if matches!(e.action, Action::Fixup | Action::Drop) {
                    e.message = None;
                }
                e
            })
            .collect()
    }
}

/// The graph column: a commit dot on the branch line; a squashed or fixed
/// up commit hangs off the line and joins the commit it folds into, and a
/// dropped one leaves the line, as in IntelliJ's dialog.
fn graph_cell(entries: &[Entry], ix: usize, palette: &crate::theme::Palette) -> impl IntoElement {
    let action = entries[ix].action;
    let folds = |a: Action| matches!(a, Action::Squash | Action::Fixup);
    // This row is part of a group folding into a commit below.
    let joins_below = folds(action);
    // A folded commit sits right above: join it.
    let joins_above = ix > 0 && folds(entries[ix - 1].action) && action != Action::Drop;
    let line = palette.text_secondary;
    let accent = palette.status_renamed;
    let first = ix == 0;
    let last = ix + 1 == entries.len();
    let x_main = px(9.);
    let x_side = px(19.);
    div()
        .relative()
        .w(px(28.))
        .h_full()
        .flex_shrink_0()
        // The branch line through every commit.
        .child(div().absolute().left(x_main).top(if first { px(12.) } else { px(0.) }).bottom(if last { px(12.) } else { px(0.) }).w(px(1.)).bg(line))
        .when(joins_below, |el| {
            // From this commit down into the next row.
            el.child(div().absolute().left(x_side).top(px(12.)).bottom(px(0.)).w(px(2.)).bg(accent))
        })
        .when(joins_above && !joins_below, |el| {
            el.child(div().absolute().left(x_side).top(px(0.)).h(px(12.)).w(px(2.)).bg(accent))
                .child(div().absolute().left(x_main).top(px(11.)).w(px(11.)).h(px(2.)).bg(accent))
        })
        .when(joins_above && joins_below, |el| el.child(div().absolute().left(x_side).top(px(0.)).h(px(12.)).w(px(2.)).bg(accent)))
        .child(match action {
            Action::Drop => div()
                .absolute()
                .left(x_side - px(3.))
                .top(px(8.))
                .size(px(8.))
                .rounded_full()
                .border_1()
                .border_color(palette.status_deleted),
            Action::Squash | Action::Fixup => {
                div().absolute().left(x_side - px(3.)).top(px(9.)).size(px(8.)).rounded_full().bg(accent)
            }
            _ => div()
                .absolute()
                .left(x_main - px(4.))
                .top(px(8.))
                .size(px(9.))
                .rounded_full()
                .bg(if matches!(action, Action::Reword | Action::Edit) { palette.accent } else { line }),
        })
}

impl Render for RebaseEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let selected = self.selected;
        let action_color = |action: Action| match action {
            Action::Pick => palette.text_secondary,
            Action::Reword | Action::Edit => palette.accent,
            Action::Squash | Action::Fixup => palette.status_renamed,
            Action::Drop => palette.status_deleted,
        };
        let mut rows = v_flex();
        let count = self.selection.len();
        for (ix, entry) in self.entries.iter().enumerate() {
            let dropped = entry.action == Action::Drop;
            let accent = palette.accent;
            let dragging = if self.selection.contains(&ix) && count > 1 { count } else { 1 };
            let label = if dragging > 1 { format!("{} commits", dragging) } else { entry.commit.subject.clone() };
            let dragged = DraggedRow { ix, subject: label.into() };
            let moving_down = move |row: &DraggedRow| row.ix < ix;
            let is_selected = self.selection.contains(&ix);
            rows = rows.child(
                h_flex()
                    .id(SharedString::from(format!("rebase-row-{ix}")))
                    .h(px(24.))
                    .pr_2()
                    .gap_2()
                    .text_sm()
                    .when(is_selected, |el| el.bg(palette.selection))
                    // Drag rows to reorder, as in IntelliJ's dialog; the line
                    // shows where they land.
                    .on_drag(dragged, |row, _, _, cx| cx.new(|_| row.clone()))
                    .drag_over::<DraggedRow>(move |style, row, _, _| {
                        if moving_down(row) { style.border_b_2().border_color(accent) } else { style.border_t_2().border_color(accent) }
                    })
                    .on_drop(cx.listener(move |this, row: &DraggedRow, window, cx| this.drop_rows(row.ix, ix, window, cx)))
                    .on_click(cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                        window.focus(&this.focus, cx);
                        if event.click_count() == 2 {
                            this.select(ix, window, cx);
                            this.reword(window, cx);
                        } else {
                            this.click(ix, event.modifiers(), window, cx)
                        }
                    }))
                    .child(graph_cell(&self.entries, ix, &palette))
                    .child(div().w(px(56.)).text_color(action_color(entry.action)).child(entry.action.label()))
                    .child(div().w(px(64.)).text_color(palette.text_secondary).child(entry.commit.short_hash().to_owned()))
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .when(dropped, |el| el.line_through().text_color(palette.text_disabled))
                            .child(
                                entry
                                    .message
                                    .as_deref()
                                    .and_then(|m| m.lines().next())
                                    .filter(|_| entry.action == Action::Reword)
                                    .unwrap_or(&entry.commit.subject)
                                    .to_owned(),
                            ),
                    )
                    .child(div().w(px(90.)).text_color(palette.text_secondary).child(entry.commit.author_name.to_string())),
            );
        }
        let current = self.entries.get(selected).map(|e| e.action);
        let editable = matches!(current, Some(Action::Reword | Action::Squash));
        let multi = count > 1;
        // On the oldest row alone there is nothing below to fold into.
        let oldest_only = !multi && selected + 1 == self.entries.len();
        let action_button = |id: &'static str, label: &'static str, action: Action, shortcut: &'static str, cx: &mut Context<Self>| {
            Button::new(id)
                .xsmall()
                .label(label)
                .tooltip(shortcut)
                .when(current == Some(action) && !multi, |b| b.primary())
                .when(current != Some(action) || multi, |b| b.ghost())
                .disabled(oldest_only && matches!(action, Action::Squash | Action::Fixup))
                .on_click(cx.listener(move |this, _, window, cx| {
                    window.focus(&this.focus, cx);
                    if action == Action::Reword { this.reword(window, cx) } else { this.set_action(action, window, cx) }
                }))
        };

        v_flex()
            .gap_2()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .child(
                h_flex()
                    .gap_0p5()
                    .child(action_button("rb-pick", "Pick", Action::Pick, "Pick  Alt+P", cx))
                    .child(
                        action_button("rb-edit", "Stop to Edit", Action::Edit, "Stop to Edit  Alt+E", cx).icon(Icon::new(IconName::Pause)),
                    )
                    .child(action_button("rb-reword", "Reword", Action::Reword, "Reword  Alt+R (or double-click)", cx))
                    // With several commits selected, Squash and Fixup unite them.
                    .child(action_button(
                        "rb-squash",
                        if multi { "Unite" } else { "Squash" },
                        Action::Squash,
                        if multi { "Unite the selected commits, combining their messages  Alt+S" } else { "Squash into the commit below  Alt+S" },
                        cx,
                    ))
                    .child(action_button(
                        "rb-fixup",
                        if multi { "Unite (Fixup)" } else { "Fixup" },
                        Action::Fixup,
                        if multi { "Unite the selected commits, keeping the oldest message  Alt+F" } else { "Fixup into the commit below  Alt+F" },
                        cx,
                    ))
                    .child(action_button("rb-drop", "Drop", Action::Drop, "Drop  Alt+D / Delete", cx))
                    .child(div().w(px(1.)).h(px(16.)).mx_1().bg(palette.border))
                    .child(tool_button("rb-up", IconName::ChevronUp, "Move Up  Alt+Up").on_click(cx.listener(|this, _, window, cx| this.move_selected(true, window, cx))))
                    .child(tool_button("rb-down", IconName::ChevronDown, "Move Down  Alt+Down").on_click(cx.listener(|this, _, window, cx| this.move_selected(false, window, cx))))
                    .child(div().flex_1())
                    .child(Button::new("rb-reset").ghost().xsmall().label("Reset").on_click(cx.listener(|this, _, window, cx| this.reset(window, cx)))),
            )
            .child(
                div()
                    .id("rebase-rows")
                    .h(px(220.))
                    .overflow_y_scroll()
                    .border_1()
                    .border_color(palette.border)
                    .rounded_md()
                    .child(rows),
            )
            .child(
                // IntelliJ's details: the message beside the commit's changed files.
                h_flex()
                    .gap_2()
                    .items_start()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(div().text_xs().text_color(palette.text_secondary).child(if editable {
                                "Commit message"
                            } else {
                                "Commit message (choose Reword or Squash to edit)"
                            }))
                            .child(Textarea::new(&self.message).h(px(110.)).disabled(!editable)),
                    )
                    .child(self.render_files(cx)),
            )
    }
}

impl RebaseEditor {
    /// The selected commit's changed files.
    fn render_files(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let files = self.entries.get(self.selected).and_then(|e| self.files.get(&e.commit.hash)).cloned().unwrap_or_default();
        let mut list = v_flex().gap_px().child(div().pb_1().text_xs().text_color(palette.text_secondary).child(format!(
            "{} file{} changed",
            files.len(),
            if files.len() == 1 { "" } else { "s" }
        )));
        for file in &files {
            let (dir, name) = file.path.rsplit_once('/').map_or(("", file.path.as_str()), |(d, n)| (d, n));
            list = list.child(
                h_flex()
                    .h(px(20.))
                    .gap_1p5()
                    .text_sm()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(gpui_kit::component::Icon::new(IconName::File).xsmall().text_color(palette.text_secondary))
                    .child(div().flex_shrink_0().text_color(crate::ui::common::change_color(file.kind, &palette)).child(name.to_owned()))
                    .child(div().min_w_0().text_xs().text_color(palette.text_secondary).text_ellipsis().child(dir.to_owned())),
            );
        }
        div()
            .id("rebase-files")
            .w(px(260.))
            .flex_shrink_0()
            .h(px(130.))
            .p_2()
            .border_1()
            .border_color(palette.border)
            .rounded_md()
            .overflow_y_scroll()
            .child(list)
    }
}

/// Opens the dialog for commits from `hash` (inclusive) up to HEAD.
pub fn open(model: Entity<RepoModel>, hash: String, window: &mut Window, cx: &mut App) {
    let Some(repository) = model.read(cx).repository().cloned() else { return };
    let notify = |message: String, window: &mut Window, cx: &mut App| {
        window.push_notification(gpui_kit::component::notification::Notification::warning(message), cx);
    };
    if !rebase::is_on_current_branch(&repository, &hash) {
        return notify("The commit isn't on the current branch".into(), window, cx);
    }
    let base = rebase::base_of(&repository, &hash);
    open_onto(model, base, window, cx);
}

/// Interactive rebase of the current branch onto `base` (a commit or branch).
pub fn open_onto(model: Entity<RepoModel>, base: String, window: &mut Window, cx: &mut App) {
    let Some(repository) = model.read(cx).repository().cloned() else { return };
    let notify = |message: String, window: &mut Window, cx: &mut App| {
        window.push_notification(gpui_kit::component::notification::Notification::warning(message), cx);
    };
    let commits = match rebase::commits_since(&repository, &base) {
        Ok(commits) if !commits.is_empty() => commits,
        Ok(_) => return notify("Nothing to rebase".into(), window, cx),
        Err(error) => return notify(error.to_string(), window, cx),
    };
    // fixup! / squash! commits start out attached to their targets, as with --autosquash.
    let entries: Vec<Entry> = rebase::autosquash(commits).into_iter().rev().collect();
    let count = entries.len();
    let editor = cx.new(|cx| RebaseEditor::new(repository, entries, window, cx));
    let focus = editor.read(cx).focus.clone();
    window.defer(cx, move |window, cx| window.focus(&focus, cx));
    let branch = model.read(cx).refs().current_branch.clone().unwrap_or_else(|| "HEAD".into());
    window.open_dialog(cx, move |dialog, _, _| {
        let editor_ok = editor.clone();
        let model = model.clone();
        let base = base.clone();
        dialog
            .title(format!("Rebasing {count} Commits on {branch}"))
            .w(px(760.))
            .child(editor.clone())
            .footer(
                DialogFooter::new()
                    .gap_2()
                    .child(DialogClose::new().child(Button::new("rb-cancel").label("Cancel").outline()))
                    .child(DialogAction::new().child(Button::new("rb-start").label("Start Rebasing").primary())),
            )
            .on_ok(move |_, _, cx| {
                let entries = editor_ok.read(cx).todo_entries();
                let base = base.clone();
                model.update(cx, |m, cx| m.run_operation("Rebase", move |repo| rebase::run_interactive(repo, &base, &entries), cx));
                true
            })
    });
}

/// Rewrites history from the oldest of `hashes`: every commit is picked
/// except where `plan` says otherwise. Runs in the background.
fn rewrite(
    model: &Entity<RepoModel>,
    title: &'static str,
    hashes: &[String],
    window: &mut Window,
    cx: &mut App,
    plan: impl FnOnce(Vec<Entry>) -> Vec<Entry> + Send + 'static,
) {
    let Some(repository) = model.read(cx).repository().cloned() else { return };
    let warn = |message: &str, window: &mut Window, cx: &mut App| {
        window.push_notification(gpui_kit::component::notification::Notification::warning(message.to_owned()), cx);
    };
    if hashes.iter().any(|h| !rebase::is_on_current_branch(&repository, h)) {
        return warn("The selected commits aren't all on the current branch", window, cx);
    }
    // The oldest selected commit is the one with the most commits above it.
    let Some(oldest) = hashes.iter().max_by_key(|h| rebase::commits_above(&repository, h)).cloned() else { return };
    let count = hashes.len();
    model.update(cx, |m, cx| {
        m.run_operation(title, move |repo| {
            let base = rebase::base_of(repo, &oldest);
            let entries = rebase::commits_since(repo, &base)?
                .into_iter()
                .map(|commit| Entry { commit, action: Action::Pick, message: None })
                .collect();
            let message = rebase::run_interactive(repo, &base, &plan(entries))?;
            // IntelliJ's balloon: "Dropped 1 commit", with Undo (the workspace adds it).
            Ok(if title == DROP_TITLE && repo.state() != crate::git::RepositoryState::Rebasing {
                format!("{DROPPED} {count} commit{}", if count == 1 { "" } else { "s" })
            } else {
                message
            })
        }, cx)
    });
}

/// A dialog with a message editor, used by Edit Commit Message and Squash.
fn message_dialog(title: &'static str, ok: &'static str, initial: String, window: &mut Window, cx: &mut App, on_ok: impl Fn(String, &mut Window, &mut App) + 'static) {
    let state = cx.new(|cx| TextareaState::new(window, cx).rows(8));
    state.update(cx, |s, cx| s.set_value(initial, window, cx));
    let on_ok = std::rc::Rc::new(on_ok);
    let focus = state.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let state_ok = state.clone();
        let on_ok = on_ok.clone();
        dialog
            .title(title)
            .w(px(560.))
            .child(Textarea::new(&state).h(px(160.)))
            .footer(
                DialogFooter::new()
                    .gap_2()
                    .child(DialogClose::new().child(Button::new("msg-cancel").label("Cancel").outline()))
                    .child(DialogAction::new().child(Button::new("msg-ok").label(ok).primary())),
            )
            .on_ok(move |_, window, cx| {
                let message = state_ok.read(cx).value().trim().to_owned();
                if message.is_empty() {
                    return false;
                }
                on_ok(message, window, cx);
                true
            })
    });
    window.defer(cx, move |window, cx| focus.update(cx, |s, cx| s.focus(window, cx)));
}

/// Log › Edit Commit Message… (reword one commit).
pub fn reword(model: Entity<RepoModel>, hash: String, window: &mut Window, cx: &mut App) {
    let Some(repository) = model.read(cx).repository().cloned() else { return };
    let current = rebase::message_of(&repository, &hash);
    message_dialog("Edit Commit Message", "OK", current, window, cx, move |message, window, cx| {
        let target = hash.clone();
        rewrite(&model, "Edit Commit Message", &[hash.clone()], window, cx, move |mut entries| {
            for entry in &mut entries {
                if entry.commit.hash == target {
                    entry.action = Action::Reword;
                    entry.message = Some(message.clone());
                }
            }
            entries
        });
    });
}

/// Log › Drop Commits.
/// The title of Drop Commits' notification, and how its message starts.
pub const DROP_TITLE: &str = "Drop Commits";
pub const DROPPED: &str = "Dropped";

pub fn drop_commits(model: Entity<RepoModel>, hashes: Vec<String>, window: &mut Window, cx: &mut App) {
    let targets = hashes.clone();
    rewrite(&model, DROP_TITLE, &hashes, window, cx, move |mut entries| {
        for entry in &mut entries {
            if targets.contains(&entry.commit.hash) {
                entry.action = Action::Drop;
            }
        }
        entries
    });
}

/// Log › Squash Commits… (several selected): the later commits move up
/// behind the oldest one and are squashed into it with a new message.
pub fn squash(model: Entity<RepoModel>, hashes: Vec<String>, window: &mut Window, cx: &mut App) {
    let Some(repository) = model.read(cx).repository().cloned() else { return };
    // Messages oldest first, as git would combine them.
    let mut ordered = hashes.clone();
    ordered.sort_by_key(|h| std::cmp::Reverse(rebase::commits_above(&repository, h)));
    let combined = ordered.iter().map(|h| rebase::message_of(&repository, h)).collect::<Vec<_>>().join("\n\n");
    message_dialog("Squash Commits", "Squash", combined, window, cx, move |message, window, cx| {
        let targets = ordered.clone();
        rewrite(&model, "Squash Commits", &ordered, window, cx, move |entries| {
            let (selected, others): (Vec<Entry>, Vec<Entry>) =
                entries.into_iter().partition(|e| targets.contains(&e.commit.hash));
            // The rewrite starts at the oldest selected commit, so the squash
            // group comes first and the other commits keep their order after it.
            let count = selected.len();
            let mut out: Vec<Entry> = selected
                .into_iter()
                .enumerate()
                .map(|(ix, mut e)| {
                    e.action = if ix == 0 { Action::Pick } else { Action::Squash };
                    e.message = (ix + 1 == count).then(|| message.clone());
                    e
                })
                .collect();
            out.extend(others);
            out
        });
    });
}
