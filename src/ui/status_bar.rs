//! The status bar's caret widgets (line:column, line separator, encoding,
//! indent), redrawn on their own as the caret moves, and the notification
//! history behind the Notifications tool window.

use gpui_kit::component::{Icon, Sizable as _, h_flex};
use gpui_kit::assets::IconName;
use gpui_kit::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, div, prelude::FluentBuilder as _,
};

use crate::theme::ActivePalette as _;
use crate::ui::file_editor::FileEditor;

pub struct CaretStatus {
    editor: Option<Entity<FileEditor>>,
    _subscriptions: Vec<Subscription>,
}

impl CaretStatus {
    pub fn new() -> Self {
        Self { editor: None, _subscriptions: Vec::new() }
    }

    /// Follows the active editor (None while the diff, merge or no file is in front).
    pub fn set_editor(&mut self, editor: Option<Entity<FileEditor>>, cx: &mut Context<Self>) {
        if self.editor.as_ref().map(|e| e.entity_id()) == editor.as_ref().map(|e| e.entity_id()) {
            return;
        }
        self._subscriptions.clear();
        if let Some(editor) = &editor {
            let input = editor.read(cx).input().clone();
            self._subscriptions.push(cx.observe(&input, |_, _, cx| cx.notify()));
            self._subscriptions.push(cx.observe(editor, |_, _, cx| cx.notify()));
        }
        self.editor = editor;
        cx.notify();
    }
}

impl Render for CaretStatus {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let Some(editor) = &self.editor else { return h_flex() };
        let editor = editor.read(cx);
        let (line, column) = editor.caret(cx);
        let format = editor.format().clone();
        let read_only = editor.revision().is_some();
        let widget = |id: &'static str, text: SharedString, tooltip: &'static str| {
            div()
                .id(id)
                .px_1()
                .rounded_sm()
                .cursor_pointer()
                .hover(|s| s.bg(palette.hover))
                .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tooltip).build(window, cx))
                .child(text)
        };
        h_flex()
            .gap_1()
            .child(widget("status-caret", format!("{line}:{column}").into(), "Go to Line/Column").on_click(|_, window, cx| {
                window.dispatch_action(Box::new(crate::ui::workspace::GotoLine), cx)
            }))
            .child(widget("status-separator", if format.crlf { "CRLF" } else { "LF" }.into(), "Line Separator"))
            .child(widget("status-encoding", "UTF-8".into(), "File Encoding"))
            .child(widget("status-indent", format.indent.into(), "Indent"))
            .when(read_only, |el| el.child(Icon::new(IconName::Lock).xsmall().text_color(palette.text_secondary)))
    }
}

/// One entry of the notification history.
#[derive(Clone)]
pub struct NotificationRecord {
    pub title: String,
    pub message: String,
    pub error: bool,
    /// Finished with some failures (a warning).
    pub warning: bool,
    pub time: chrono::DateTime<chrono::Local>,
}
