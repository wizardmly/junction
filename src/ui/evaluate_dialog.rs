//! IntelliJ's Evaluate Expression dialog (Alt+F8 while paused): the
//! expression (the selection or the expression at the caret), its history,
//! and the result as a tree that expands like the Variables view; Add to
//! Watches keeps it in the Debug window. The field is a code editor in the
//! paused file's language: completion offers what's in scope at the paused
//! line, Shift+Enter starts another line (a code fragment), Enter evaluates.

use gpui_kit::component::{
    Disableable as _, Icon, Sizable as _, WindowExt as _, h_flex,
    button::{Button, ButtonVariants as _},
    dialog::{DialogAction, DialogClose, DialogFooter},
    input::{Editor, EditorState, InputEvent},
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _, px,
};

use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::debug_service::{self, DebugService, NodeKind, VarNode};
use crate::ui::debug_view::{VarRow, flatten, var_row};

pub struct EvaluateView {
    service: Entity<DebugService>,
    expression: Entity<EditorState>,
    /// The result's row, once evaluated.
    result: Vec<VarNode>,
    evaluating: bool,
    /// Why there's no result: not paused.
    message: Option<&'static str>,
    _subscriptions: Vec<Subscription>,
}

impl EvaluateView {
    fn text(&self, cx: &App) -> String {
        self.expression.read(cx).value().trim().to_owned()
    }

    /// Evaluate: the expression's value in the selected frame.
    fn evaluate(&mut self, cx: &mut Context<Self>) {
        let text = self.text(cx);
        if text.is_empty() {
            return;
        }
        let task = self.service.update(cx, |s, cx| {
            s.remember_evaluated(&text);
            s.evaluate_task(&text, cx)
        });
        let Some(task) = task else {
            self.message = Some("The debugger is not paused");
            cx.notify();
            return;
        };
        self.evaluating = true;
        self.message = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.service.update(cx, |s, cx| s.evaluated(&text, cx));
                this.evaluating = false;
                // IntelliJ names the root "result".
                let mut node = VarNode::expression("result".into(), NodeKind::Evaluation, result);
                node.kind = NodeKind::Variable;
                this.result = vec![node];
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn toggle(&mut self, path: &[usize], cx: &mut Context<Self>) {
        let Some(node) = debug_service::node_at(&mut self.result, path) else { return };
        node.expanded = !node.expanded;
        let load = (node.expanded && node.children.is_none()).then(|| node.var.value.clone());
        cx.notify();
        let Some(value) = load else { return };
        let Some(task) = self.service.update(cx, |s, cx| s.children_task(value, cx)) else { return };
        let path = path.to_vec();
        cx.spawn(async move |this, cx| {
            let children = task.await;
            this.update(cx, |this, cx| {
                if let Some(node) = debug_service::node_at(&mut this.result, &path) {
                    node.children = Some(children);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn add_to_watches(&mut self, cx: &mut Context<Self>) {
        let text = self.text(cx);
        self.service.update(cx, |s, cx| s.add_watch(text, cx));
    }
}

impl Render for EvaluateView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let history = self.service.read(cx).evaluated.clone();
        let input = self.expression.clone();
        let mut rows: Vec<VarRow> = Vec::new();
        flatten(&self.result, 0, &mut Vec::new(), &mut rows);
        let note = |text: &'static str| div().p_2().text_sm().text_color(palette.text_secondary).child(text);
        let body = if self.evaluating {
            note("Evaluating…").into_any_element()
        } else if let Some(message) = self.message {
            note(message).into_any_element()
        } else if rows.is_empty() {
            note(if cfg!(target_os = "macos") { "Press ↩ to evaluate" } else { "Press Enter to evaluate" }).into_any_element()
        } else {
            v_flex()
                .children(rows.iter().enumerate().map(|(ix, row)| {
                    let path = row.path.clone();
                    var_row(("evaluate-row", ix), row, &palette)
                        .when(row.expandable, |el| el.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| this.toggle(&path, cx))))
                }))
                .into_any_element()
        };
        v_flex()
            .gap_2()
            .child(div().text_sm().child("Expression:"))
            .child(
                h_flex()
                    .gap_1()
                    .child({
                        // One line, growing with a code fragment's.
                        let lines = self.expression.read(cx).value().lines().count().clamp(1, 6) as f32;
                        div().flex_1().h(px(lines * 20. + 12.)).px_1().border_1().border_color(palette.border).rounded_md().child(Editor::new(&self.expression).bordered(false).appearance(false).py_1().h_full())
                    })
                    .child(
                        Button::new("evaluate-history")
                            .ghost()
                            .small()
                            .icon(Icon::new(icons::HISTORY))
                            .tooltip("History")
                            .disabled(history.is_empty())
                            .dropdown_menu(move |mut menu, _, _| {
                                for text in &history {
                                    let (text, input) = (text.clone(), input.clone());
                                    menu = menu.item(PopupMenuItem::new(text.clone()).on_click(move |_, window, cx| {
                                        input.update(cx, |s, cx| s.set_value(text.clone(), window, cx));
                                    }));
                                }
                                menu
                            }),
                    ),
            )
            .child(div().text_sm().child("Result:"))
            .child(div().id("evaluate-result").h(px(240.)).overflow_y_scroll().py_1().border_1().border_color(palette.border).rounded_md().bg(palette.panel).child(body))
    }
}

/// Opens Evaluate Expression with `initial` selected in its field.
pub fn open(initial: String, window: &mut Window, cx: &mut App) {
    let service = DebugService::global(cx);
    let completion = fragment_completion(&service, cx);
    let language = completion.as_ref().map_or("text", |c| crate::ui::file_editor::language_for(&c.inner.path));
    let view = cx.new(|cx| {
        let expression = cx.new(|cx| {
            let mut state = EditorState::new(window, cx).language(language).line_number(false).soft_wrap(false).submit_on_enter(true).placeholder("Expression to evaluate").default_value(initial);
            if let Some(completion) = completion {
                state.lsp_mut().completion_provider = Some(std::rc::Rc::new(completion));
            }
            state
        });
        let _subscriptions = vec![
            cx.observe(&service, |_, _, cx| cx.notify()),
            cx.subscribe(&expression, |this: &mut EvaluateView, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) && this.message.is_some() {
                    this.message = None;
                    cx.notify();
                }
            }),
        ];
        EvaluateView { service, expression, result: Vec::new(), evaluating: false, message: None, _subscriptions }
    });
    let focus = view.read(cx).expression.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let (view_ok, view_watch) = (view.clone(), view.clone());
        let footer = DialogFooter::new()
            .gap_2()
            .child(div().size_full().child(Button::new("evaluate-add-watch").label("Add to Watches").outline().w_full().on_click(move |_, _, cx| view_watch.update(cx, |v, cx| v.add_to_watches(cx)))))
            .child(DialogClose::new().child(Button::new("evaluate-close").label("Close").outline()))
            .child(DialogAction::new().child(Button::new("evaluate-ok").label("Evaluate").primary()));
        // Evaluate keeps the dialog open, as IntelliJ's.
        dialog.title("Evaluate").w(px(620.)).child(view.clone()).footer(footer).on_ok(move |_, _, cx| {
            view_ok.update(cx, |v, cx| v.evaluate(cx));
            false
        })
    });
    let editor = focus.clone();
    window.defer(cx, move |window, cx| editor.update(cx, |state, cx| state.focus(window, cx)));
    focus.update(cx, |s, cx| s.select_all(window, cx));
}

/// Completion as if the expression were written at the start of the paused
/// line, over the index (none when not paused in a project file).
fn fragment_completion(service: &Entity<DebugService>, cx: &mut App) -> Option<crate::ui::completion::FragmentCompletion> {
    let (file, line) = service.read(cx).execution_point(cx)?;
    let index = service.read(cx).code_index.as_ref()?.upgrade()?;
    let root = index.read(cx).root()?.to_path_buf();
    let path = file.strip_prefix(&root).ok()?.to_string_lossy().replace('\\', "/");
    let context = std::fs::read_to_string(&file).ok()?;
    let at = crate::domain::expression::line_start(&context, line as usize);
    Some(crate::ui::completion::FragmentCompletion { inner: crate::ui::completion::IndexCompletion { index, path, index_only: true }, context, at })
}
