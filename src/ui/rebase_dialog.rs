//! IntelliJ's "Rebasing Commits" dialog for interactive rebase: the commits
//! (newest first, as in the Log), an action per commit, reordering, and a
//! message editor for Reword / Squash.

use gpui_kit::component::{
    Disableable as _, Sizable as _, WindowExt as _, h_flex,
    button::{Button, ButtonVariants as _},
    dialog::{DialogAction, DialogClose, DialogFooter},
    input::{InputEvent, Textarea, TextareaState},
    v_flex,
};
use gpui_kit::assets::IconName;
use gpui_kit::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _,
    px,
};

use crate::git::rebase::{self, Action, Entry};
use crate::git::Repository;
use crate::model::RepoModel;
use crate::theme::ActivePalette as _;
use crate::ui::common::tool_button;

pub struct RebaseEditor {
    repository: Repository,
    /// Newest first, as displayed.
    entries: Vec<Entry>,
    original: Vec<Entry>,
    selected: usize,
    message: Entity<TextareaState>,
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
                    }
                }
            }
        });
        let mut this = Self { repository, original: entries.clone(), entries, selected: 0, message, _subscription: subscription };
        this.load_message(window, cx);
        this
    }

    fn load_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.entries.get(self.selected) else { return };
        let text = entry.message.clone().unwrap_or_else(|| rebase::message_of(&self.repository, &entry.commit.hash));
        self.message.update(cx, |state, cx| state.set_value(text, window, cx));
    }

    fn select(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = ix.min(self.entries.len().saturating_sub(1));
        self.load_message(window, cx);
        cx.notify();
    }

    fn set_action(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        // The oldest commit has nothing below it to squash into.
        let oldest = self.selected + 1 == self.entries.len();
        if oldest && matches!(action, Action::Squash | Action::Fixup) {
            return;
        }
        if let Some(entry) = self.entries.get_mut(self.selected) {
            entry.action = action;
            if action == Action::Squash && entry.message.is_none() {
                // Squash proposes both messages, like git's default.
                let below = &self.entries[self.selected + 1];
                let joined = format!(
                    "{}\n\n{}",
                    rebase::message_of(&self.repository, &below.commit.hash),
                    rebase::message_of(&self.repository, &self.entries[self.selected].commit.hash)
                );
                self.entries[self.selected].message = Some(joined);
            }
        }
        self.load_message(window, cx);
        cx.notify();
    }

    fn move_selected(&mut self, up: bool, cx: &mut Context<Self>) {
        let ix = self.selected;
        let target = if up { ix.checked_sub(1) } else { (ix + 1 < self.entries.len()).then_some(ix + 1) };
        if let Some(target) = target {
            self.entries.swap(ix, target);
            self.selected = target;
            cx.notify();
        }
    }

    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.entries = self.original.clone();
        self.load_message(window, cx);
        cx.notify();
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
        for (ix, entry) in self.entries.iter().enumerate() {
            let dropped = entry.action == Action::Drop;
            rows = rows.child(
                h_flex()
                    .id(SharedString::from(format!("rebase-row-{ix}")))
                    .h(px(24.))
                    .px_2()
                    .gap_2()
                    .text_sm()
                    .when(ix == selected, |el| el.bg(palette.selection))
                    .on_click(cx.listener(move |this, _, window, cx| this.select(ix, window, cx)))
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
                    .child(div().w(px(90.)).text_color(palette.text_secondary).child(entry.commit.author_name.clone())),
            );
        }
        let current = self.entries.get(selected).map(|e| e.action);
        let editable = matches!(current, Some(Action::Reword | Action::Squash));
        let oldest = selected + 1 == self.entries.len();
        let action_button = |id: &'static str, action: Action, cx: &mut Context<Self>| {
            Button::new(id)
                .xsmall()
                .label(action.label())
                .when(current == Some(action), |b| b.primary())
                .when(current != Some(action), |b| b.ghost())
                .disabled(oldest && matches!(action, Action::Squash | Action::Fixup))
                .on_click(cx.listener(move |this, _, window, cx| this.set_action(action, window, cx)))
        };

        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_0p5()
                    .child(action_button("rb-pick", Action::Pick, cx))
                    .child(action_button("rb-edit", Action::Edit, cx))
                    .child(action_button("rb-reword", Action::Reword, cx))
                    .child(action_button("rb-squash", Action::Squash, cx))
                    .child(action_button("rb-fixup", Action::Fixup, cx))
                    .child(action_button("rb-drop", Action::Drop, cx))
                    .child(div().w(px(1.)).h(px(16.)).mx_1().bg(palette.border))
                    .child(tool_button("rb-up", IconName::ChevronUp, "Move Up").on_click(cx.listener(|this, _, _, cx| this.move_selected(true, cx))))
                    .child(tool_button("rb-down", IconName::ChevronDown, "Move Down").on_click(cx.listener(|this, _, _, cx| this.move_selected(false, cx))))
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
            .child(div().text_xs().text_color(palette.text_secondary).child(if editable {
                "Commit message"
            } else {
                "Commit message (choose Reword or Squash to edit)"
            }))
            .child(Textarea::new(&self.message).h(px(110.)).disabled(!editable))
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
    let commits = match rebase::commits_since(&repository, &base) {
        Ok(commits) if !commits.is_empty() => commits,
        Ok(_) => return notify("Nothing to rebase".into(), window, cx),
        Err(error) => return notify(error.to_string(), window, cx),
    };
    let entries: Vec<Entry> = commits.into_iter().rev().map(|commit| Entry { commit, action: Action::Pick, message: None }).collect();
    let count = entries.len();
    let editor = cx.new(|cx| RebaseEditor::new(repository, entries, window, cx));
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
    let count_above = |h: &String| {
        repository.run(["rev-list", "--count", &format!("{h}..HEAD")]).ok().and_then(|n| n.trim().parse::<usize>().ok()).unwrap_or(0)
    };
    let Some(oldest) = hashes.iter().max_by_key(|h| count_above(h)).cloned() else { return };
    model.update(cx, |m, cx| {
        m.run_operation(title, move |repo| {
            let base = rebase::base_of(repo, &oldest);
            let entries = rebase::commits_since(repo, &base)?
                .into_iter()
                .map(|commit| Entry { commit, action: Action::Pick, message: None })
                .collect();
            rebase::run_interactive(repo, &base, &plan(entries))
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
pub fn drop_commits(model: Entity<RepoModel>, hashes: Vec<String>, window: &mut Window, cx: &mut App) {
    let targets = hashes.clone();
    rewrite(&model, "Drop Commits", &hashes, window, cx, move |mut entries| {
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
    ordered.sort_by_key(|h| {
        std::cmp::Reverse(
            repository.run(["rev-list", "--count", &format!("{h}..HEAD")]).ok().and_then(|n| n.trim().parse::<usize>().ok()).unwrap_or(0),
        )
    });
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
