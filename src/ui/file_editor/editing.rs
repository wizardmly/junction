//! IntelliJ's basic editing actions in the editor: Duplicate Line or
//! Selection (Ctrl+D), Comment with Line / Block Comment (Ctrl+/,
//! Ctrl+Shift+/), Move Line Up / Down (Alt+Shift+↑/↓), Extend / Shrink
//! Selection (Ctrl+W, Ctrl+Shift+W), Move Statement Up / Down
//! (Ctrl+Shift+↑/↓), Move Element Left / Right (Ctrl+Alt+Shift+←/→), Join Lines (Ctrl+Shift+J), Start New
//! Line (Shift+Enter) and Toggle Case (Ctrl+Shift+U). The edits are
//! `domain::line_edit`'s; each is one undoable change.

use std::ops::Range;

use gpui_kit::{Context, KeyBinding, Window, actions};

use super::FileEditor;
use crate::domain::line_edit::{self, Edited};

actions!(editing, [DuplicateLine, CommentLine, CommentBlock, MoveLineUp, MoveLineDown, MoveStatementUp, MoveStatementDown, MoveElementLeft, MoveElementRight, Inline, UnwrapRemove, ExtendSelection, ShrinkSelection, JoinLines, StartNewLine, ToggleCase]);

/// In the editor's input, over the input's own bindings for these keys.
const INPUT: &str = "FileEditor > Input";

pub fn init(cx: &mut gpui_kit::App) {
    let mac = cfg!(target_os = "macos");
    cx.bind_keys([
        KeyBinding::new(if mac { "cmd-d" } else { "ctrl-d" }, DuplicateLine, Some(INPUT)),
        KeyBinding::new(if mac { "cmd-/" } else { "ctrl-/" }, CommentLine, Some(INPUT)),
        KeyBinding::new(if mac { "cmd-alt-/" } else { "ctrl-shift-/" }, CommentBlock, Some(INPUT)),
        KeyBinding::new(if mac { "cmd-alt-n" } else { "ctrl-alt-n" }, Inline, Some(INPUT)),
        KeyBinding::new(if mac { "cmd-alt-shift-left" } else { "ctrl-alt-shift-left" }, MoveElementLeft, Some(INPUT)),
        KeyBinding::new(if mac { "cmd-alt-shift-right" } else { "ctrl-alt-shift-right" }, MoveElementRight, Some(INPUT)),
        KeyBinding::new(if mac { "cmd-shift-delete" } else { "ctrl-shift-delete" }, UnwrapRemove, Some(INPUT)),
        KeyBinding::new("alt-shift-up", MoveLineUp, Some(INPUT)),
        KeyBinding::new("alt-shift-down", MoveLineDown, Some(INPUT)),
        KeyBinding::new(if mac { "cmd-shift-up" } else { "ctrl-shift-up" }, MoveStatementUp, Some(INPUT)),
        KeyBinding::new(if mac { "cmd-shift-down" } else { "ctrl-shift-down" }, MoveStatementDown, Some(INPUT)),
        KeyBinding::new(if mac { "alt-up" } else { "ctrl-w" }, ExtendSelection, Some(INPUT)),
        KeyBinding::new(if mac { "alt-down" } else { "ctrl-shift-w" }, ShrinkSelection, Some(INPUT)),
        KeyBinding::new("ctrl-shift-j", JoinLines, Some(INPUT)),
        KeyBinding::new("shift-enter", StartNewLine, Some(INPUT)),
        KeyBinding::new(if mac { "cmd-shift-u" } else { "ctrl-shift-u" }, ToggleCase, Some(INPUT)),
    ]);
}

impl FileEditor {
    fn extension(&self) -> String {
        self.path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
    }

    /// Applies an edit as one change, then its selection.
    fn apply_edited(&mut self, edited: Edited, window: &mut Window, cx: &mut Context<Self>) {
        let old = self.text(cx);
        self.replace_text(&old, &edited.text, edited.selection.start, window, cx);
        let selection = edited.selection;
        self.state.update(cx, |state, cx| state.set_selected_range(selection, cx));
        cx.notify();
    }

    fn edit_with(&mut self, edit: impl FnOnce(&str, Range<usize>) -> Option<Edited>, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only() {
            return;
        }
        let text = self.text(cx);
        let selection = self.state.read(cx).selected_range();
        if let Some(edited) = edit(&text, selection) {
            self.apply_edited(edited, window, cx);
        }
    }

    pub(super) fn duplicate_line(&mut self, _: &DuplicateLine, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_with(|t, s| Some(line_edit::duplicate(t, s)), window, cx);
    }

    pub(super) fn comment_line(&mut self, _: &CommentLine, window: &mut Window, cx: &mut Context<Self>) {
        let (line, block) = line_edit::comment_tokens(&self.extension());
        self.edit_with(|t, s| Some(line_edit::toggle_line_comment(t, s, line, block)), window, cx);
    }

    pub(super) fn comment_block(&mut self, _: &CommentBlock, window: &mut Window, cx: &mut Context<Self>) {
        let (_, block) = line_edit::comment_tokens(&self.extension());
        let Some(block) = block else { return };
        self.edit_with(|t, s| Some(line_edit::toggle_block_comment(t, s, block)), window, cx);
    }

    pub(super) fn move_line_up(&mut self, _: &MoveLineUp, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_with(|t, s| line_edit::move_lines(t, s, true), window, cx);
    }

    pub(super) fn move_line_down(&mut self, _: &MoveLineDown, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_with(|t, s| line_edit::move_lines(t, s, false), window, cx);
    }

    pub(super) fn move_statement_up(&mut self, _: &MoveStatementUp, window: &mut Window, cx: &mut Context<Self>) {
        let lang = crate::index::lang::Lang::from_path(&self.path);
        self.edit_with(|t, s| crate::index::selection::move_statement(lang, t, s, true), window, cx);
    }

    pub(super) fn move_statement_down(&mut self, _: &MoveStatementDown, window: &mut Window, cx: &mut Context<Self>) {
        let lang = crate::index::lang::Lang::from_path(&self.path);
        self.edit_with(|t, s| crate::index::selection::move_statement(lang, t, s, false), window, cx);
    }

    pub(super) fn move_element_left(&mut self, _: &MoveElementLeft, window: &mut Window, cx: &mut Context<Self>) {
        let lang = crate::index::lang::Lang::from_path(&self.path);
        self.edit_with(|t, s| crate::index::selection::move_element(lang, t, s, true), window, cx);
    }

    pub(super) fn move_element_right(&mut self, _: &MoveElementRight, window: &mut Window, cx: &mut Context<Self>) {
        let lang = crate::index::lang::Lang::from_path(&self.path);
        self.edit_with(|t, s| crate::index::selection::move_element(lang, t, s, false), window, cx);
    }

    /// Inline (Ctrl+Alt+N): a local variable into its uses, or a function
    /// into its calls.
    pub(super) fn inline(&mut self, _: &Inline, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only() {
            return;
        }
        let Some(lang) = crate::index::lang::Lang::from_path(&self.path) else { return };
        let text = self.text(cx);
        let offset = self.state.read(cx).cursor();
        match crate::index::inline::inline_variable(lang, &text, offset) {
            Ok(edited) => self.apply_edited(edited, window, cx),
            // Not a local variable: a function?
            Err(variable) => match crate::index::inline_function::plan_at(lang, &text, offset) {
                Ok((plan, declaration)) => self.inline_function(lang, text, offset, plan, declaration, window, cx),
                Err(_) if !variable.contains("Caret should be positioned") => Self::hint("Inline Variable", variable, window, cx),
                Err(function) => Self::hint("Inline", function, window, cx),
            },
        }
    }

    pub(super) fn join_lines(&mut self, _: &JoinLines, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_with(line_edit::join_lines, window, cx);
    }

    pub(super) fn start_new_line(&mut self, _: &StartNewLine, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_with(|t, s| Some(line_edit::start_new_line(t, s)), window, cx);
    }

    pub(super) fn toggle_case(&mut self, _: &ToggleCase, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_with(|t, s| Some(line_edit::toggle_case(t, s)), window, cx);
    }

    /// Ctrl+W: the next larger syntax around the selection; the steps are
    /// kept for Shrink Selection.
    pub(super) fn extend_selection(&mut self, _: &ExtendSelection, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.text(cx);
        let current = self.state.read(cx).selected_range();
        if self.selection_steps.last() != Some(&current) {
            self.selection_steps.clear();
        }
        let lang = crate::index::lang::Lang::from_path(&self.path);
        let Some(next) = crate::index::selection::extend(lang, &text, current.clone()) else { return };
        if self.selection_steps.is_empty() {
            self.selection_steps.push(current);
        }
        self.selection_steps.push(next.clone());
        self.state.update(cx, |state, cx| state.set_selected_range(next, cx));
        cx.notify();
    }

    /// Ctrl+Shift+W: back one Extend Selection step.
    pub(super) fn shrink_selection(&mut self, _: &ShrinkSelection, _: &mut Window, cx: &mut Context<Self>) {
        let current = self.state.read(cx).selected_range();
        if self.selection_steps.last() != Some(&current) || self.selection_steps.len() < 2 {
            self.selection_steps.clear();
            return;
        }
        self.selection_steps.pop();
        let previous = self.selection_steps.last().cloned().unwrap_or(current);
        self.state.update(cx, |state, cx| state.set_selected_range(previous, cx));
        cx.notify();
    }
}
