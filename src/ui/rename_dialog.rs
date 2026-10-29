//! Android Studio's Rename dialog (Shift+F6 in the editor): the new name,
//! "Search in comments and strings", "Rename file", and the occurrences
//! the rename changes, each of which can be left out.

use std::rc::Rc;

use gpui_kit::component::{
    Sizable as _, WindowExt as _, h_flex,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    dialog::{DialogAction, DialogClose, DialogFooter},
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px,
};

use crate::domain::rename::{check_name, renamed_file, summary, Occurrence};
use crate::index::service::RenameTarget;
use crate::theme::ActivePalette as _;
use crate::ui::{common, dialogs};

/// What the dialog's Refactor asks for.
pub struct RenameRequest {
    pub old: String,
    pub new: String,
    pub declaration: String,
    pub occurrences: Vec<Occurrence>,
    pub rename_file: bool,
}

pub struct RenameView {
    target: RenameTarget,
    name: Entity<InputState>,
    checked: Vec<bool>,
    comments: bool,
    rename_file: bool,
    _subscription: Subscription,
}

impl RenameView {
    fn extension(&self) -> &str {
        self.target.declaration.rsplit('.').next().unwrap_or_default()
    }

    fn new_name(&self, cx: &App) -> String {
        self.name.read(cx).value().trim().to_owned()
    }

    /// The occurrences the rename changes now.
    fn selected(&self) -> Vec<&Occurrence> {
        self.target.occurrences.iter().zip(&self.checked).filter(|(o, c)| **c && (o.code || self.comments)).map(|(o, _)| o).collect()
    }

    fn request(&self, cx: &App) -> Result<RenameRequest, String> {
        let new = self.new_name(cx);
        check_name(self.extension(), &new)?;
        Ok(RenameRequest {
            old: self.target.name.clone(),
            new,
            declaration: self.target.declaration.clone(),
            occurrences: self.selected().into_iter().cloned().collect(),
            rename_file: self.rename_file && self.file_name().is_some(),
        })
    }

    /// The declaring file, when renaming a type would rename it too.
    fn file_name(&self) -> Option<String> {
        if !matches!(self.target.kind.as_str(), "class" | "interface" | "enum" | "struct" | "protocol" | "trait") {
            return None;
        }
        renamed_file(&self.target.declaration, &self.target.name, "_")?;
        self.target.declaration.rsplit('/').next().map(str::to_owned)
    }
}

impl Render for RenameView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let new = self.new_name(cx);
        let error = (new != self.target.name).then(|| check_name(self.extension(), &new).err()).flatten();
        let has_text = self.target.occurrences.iter().any(|o| !o.code);
        let mut list = v_flex().id("rename-occurrences").max_h(px(240.)).min_h(px(60.)).overflow_y_scroll().p_1().border_1().border_color(palette.border).rounded_md();
        for (ix, o) in self.target.occurrences.iter().enumerate() {
            if !o.code && !self.comments {
                continue;
            }
            list = list.child(
                h_flex()
                    .gap_1()
                    .child(Checkbox::new(SharedString::from(format!("rename-{ix}"))).checked(self.checked[ix]).on_change(cx.listener(move |this, v: &bool, _, cx| {
                        this.checked[ix] = *v;
                        cx.notify();
                    })))
                    .child(gpui_kit::component::Icon::new(common::file_icon(&o.path)).xsmall().text_color(palette.text_secondary))
                    .child(div().flex_none().text_sm().text_color(palette.text_secondary).child(format!("{}:{}", o.path.rsplit('/').next().unwrap_or(&o.path), o.line + 1)))
                    .child(div().text_sm().truncate().when(!o.code, |el| el.text_color(palette.text_secondary)).child(o.text.clone()))
                    .when_some(o.other.as_ref(), |el, other| el.child(div().flex_none().text_xs().text_color(palette.text_secondary).child(format!("(another symbol: {other})")))),
            );
        }
        let local = self.target.kind == "variable";
        v_flex()
            .gap_2()
            .child(div().text_sm().child(format!("Rename {} '{}' and its usages to:", self.target.kind, self.target.name)))
            .child(Input::new(&self.name))
            .when_some(error, |el, e| el.child(div().text_xs().text_color(palette.status_conflict).child(e)))
            .when(!local && has_text, |el| {
                el.child(Checkbox::new("rename-comments").label("Search in comments and strings").checked(self.comments).on_change(cx.listener(|this, v: &bool, _, cx| {
                    this.comments = *v;
                    cx.notify();
                })))
            })
            .when_some(self.file_name(), |el, file| {
                el.child(Checkbox::new("rename-file").label(format!("Rename file '{file}'")).checked(self.rename_file).on_change(cx.listener(|this, v: &bool, _, cx| {
                    this.rename_file = *v;
                    cx.notify();
                })))
            })
            .child(list)
            .child(div().text_xs().text_color(palette.text_secondary).child(summary(&self.selected())))
    }
}

/// Opens the Rename dialog; Refactor hands the request to `on_ok`, Preview
/// to `on_preview` (which shows it before it's done).
pub fn open(
    target: RenameTarget,
    on_ok: impl Fn(RenameRequest, &mut Window, &mut App) + 'static,
    on_preview: impl Fn(RenameRequest, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let title = format!("Rename {}", capitalized(&target.kind));
    let view = cx.new(|cx| {
        let name = cx.new(|cx| InputState::new(window, cx).default_value(target.name.clone()));
        let _subscription = cx.subscribe(&name, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        // Comments and strings count once searched in.
        // Other symbols of the same name start unchecked.
        let checked = target.occurrences.iter().map(|o| o.other.is_none()).collect();
        RenameView { target, name, checked, comments: false, rename_file: true, _subscription }
    });
    let focus = view.read(cx).name.clone();
    let (on_ok, on_preview) = (Rc::new(on_ok), Rc::new(on_preview));
    window.open_dialog(cx, move |dialog, _, _| {
        let (view_ok, on_ok) = (view.clone(), on_ok.clone());
        let (view_preview, on_preview) = (view.clone(), on_preview.clone());
        let preview = Button::new("rename-preview").label("Preview").outline().on_click(move |_, window, cx| {
            let Ok(request) = view_preview.read(cx).request(cx) else { return };
            window.close_dialog(cx);
            on_preview(request, window, cx);
        });
        let footer = DialogFooter::new()
            .gap_2()
            .child(preview)
            .child(div().flex_1())
            .child(DialogClose::new().child(Button::new("cancel").label("Cancel").outline()))
            .child(DialogAction::new().child(Button::new("ok").label("Refactor").primary()));
        dialog.title(title.clone()).w(px(560.)).child(view.clone()).footer(footer).on_ok(move |_, window, cx| {
            let Ok(request) = view_ok.read(cx).request(cx) else { return false };
            on_ok(request, window, cx);
            true
        })
    });
    dialogs::focus_input(&focus, window, cx);
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}
