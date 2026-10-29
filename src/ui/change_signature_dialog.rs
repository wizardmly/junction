//! IntelliJ's Change Signature dialog (Ctrl+F6): the name and return type,
//! the parameters' table (Name, Type, and for an added one the Default value
//! the calls pass) with Add, Remove, Move Up and Move Down, and the
//! signature's preview, the Visibility, and for Java the Exceptions and "Delegate via
//! overloading method".

use std::rc::Rc;

use gpui_kit::component::{
    Disableable as _, Sizable as _, WindowExt as _,
    button::Button,
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::{App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px};

use crate::domain::change_signature::{NewParam, Signature, SignatureRequest, check, visibilities, visibility_name};
use crate::domain::imports::ImportStyle;
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::common::tool_button;
use crate::ui::dialogs;

struct Row {
    old: Option<usize>,
    name: Entity<InputState>,
    ty: Entity<InputState>,
    value: Entity<InputState>,
}

pub struct ChangeSignatureView {
    style: ImportStyle,
    signature: Signature,
    calls: usize,
    name: Entity<InputState>,
    return_type: Entity<InputState>,
    /// Java's exceptions, comma-separated.
    throws: Entity<InputState>,
    rows: Vec<Row>,
    selected: Option<usize>,
    /// The Visibility chosen, as [`visibilities`] names it.
    visibility: String,
    /// "Delegate via overloading method".
    delegate: bool,
    _subscriptions: Vec<Subscription>,
}

impl ChangeSignatureView {
    fn input(&mut self, value: String, placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(value).placeholder(placeholder));
        self._subscriptions.push(cx.subscribe(&input, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        }));
        input
    }

    fn request(&self, cx: &App) -> SignatureRequest {
        let value = |input: &Entity<InputState>| input.read(cx).value().trim().to_owned();
        SignatureRequest {
            name: value(&self.name),
            return_type: value(&self.return_type),
            params: self.rows.iter().map(|r| NewParam { old: r.old, name: value(&r.name), ty: value(&r.ty), value: value(&r.value) }).collect(),
            visibility: self.signature.visibility_range.is_some().then(|| self.visibility.clone()),
            delegate: self.delegate,
            throws: (self.style == ImportStyle::Java).then(|| self.throws.read(cx).value().split(',').map(|t| t.trim().to_owned()).filter(|t| !t.is_empty()).collect()),
        }
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.rows.len();
        let name = self.input(format!("param{}", n + 1), "Name", window, cx);
        let ty = self.input(String::new(), "Type", window, cx);
        let value = self.input(String::new(), "Value at the calls", window, cx);
        self.rows.push(Row { old: None, name: name.clone(), ty, value });
        self.selected = Some(n);
        dialogs::focus_input(&name, window, cx);
        name.update(cx, |s, cx| s.select_all(window, cx));
        cx.notify();
    }

    fn remove(&mut self, cx: &mut Context<Self>) {
        let Some(ix) = self.selected.filter(|ix| *ix < self.rows.len()) else { return };
        self.rows.remove(ix);
        self.selected = (!self.rows.is_empty()).then(|| ix.min(self.rows.len() - 1));
        cx.notify();
    }

    fn shift(&mut self, up: bool, cx: &mut Context<Self>) {
        let Some(ix) = self.selected else { return };
        let to = if up { ix.checked_sub(1) } else { (ix + 1 < self.rows.len()).then_some(ix + 1) };
        if let Some(to) = to {
            self.rows.swap(ix, to);
            self.selected = Some(to);
            cx.notify();
        }
    }

    fn preview(&self, request: &SignatureRequest) -> String {
        let params: Vec<String> = request
            .params
            .iter()
            .map(|p| match self.style {
                ImportStyle::Java => format!("{} {}", p.ty, p.name),
                ImportStyle::Kotlin => format!("{}: {}", p.name, p.ty),
            })
            .collect();
        let signature = match self.style {
            ImportStyle::Java if self.signature.constructor => format!("{}({})", request.name, params.join(", ")),
            ImportStyle::Java => format!("{} {}({})", request.return_type, request.name, params.join(", ")),
            ImportStyle::Kotlin if self.signature.constructor => format!("constructor {}({})", request.name, params.join(", ")),
            ImportStyle::Kotlin if request.return_type.is_empty() => format!("fun {}({})", request.name, params.join(", ")),
            ImportStyle::Kotlin => format!("fun {}({}): {}", request.name, params.join(", "), request.return_type),
        };
        let signature = match request.throws.as_deref() {
            Some(throws) if !throws.is_empty() => format!("{signature} throws {}", throws.join(", ")),
            _ => signature,
        };
        match request.visibility.as_deref() {
            Some(v) if v != "package-private" => format!("{v} {signature}"),
            _ => signature,
        }
    }
}

impl Render for ChangeSignatureView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mono = {
            use gpui_kit::component::ActiveTheme as _;
            cx.theme().mono_font_family.clone()
        };
        let request = self.request(cx);
        let error = check(self.style, &self.signature, &request).err();
        let label = |text: &'static str| div().w(px(90.)).flex_none().text_sm().text_color(palette.text_secondary).child(text);
        let header = |text: &'static str, w: f32| div().w(px(w)).flex_none().text_xs().text_color(palette.text_secondary).child(text);
        let mut table = v_flex().id("signature-params").h(px(180.)).overflow_y_scroll().border_1().border_color(palette.border).rounded_md();
        for (ix, row) in self.rows.iter().enumerate() {
            let selected = self.selected == Some(ix);
            table = table.child(
                h_flex()
                    .id(("signature-param", ix))
                    .gap_2()
                    .px_2()
                    .py_1()
                    .when(selected, |el| el.bg(palette.selection))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = Some(ix);
                        cx.notify();
                    }))
                    .child(div().w(px(160.)).child(Input::new(&row.name).small()))
                    .child(div().w(px(200.)).child(Input::new(&row.ty).small()))
                    .child(div().flex_1().child(Input::new(&row.value).small().disabled(row.old.is_some()))),
            );
        }
        let has = self.selected.is_some();
        v_flex()
            .gap_2()
            .child(h_flex().gap_2().child(label("Name:")).child(div().flex_1().child(Input::new(&self.name).disabled(self.signature.constructor))))
            .child(h_flex().gap_2().child(label("Return type:")).child(div().flex_1().child(Input::new(&self.return_type).disabled(self.signature.constructor))))
            .when(self.style == ImportStyle::Java, |el| el.child(h_flex().gap_2().child(label("Exceptions:")).child(div().flex_1().child(Input::new(&self.throws)))))
            .child(h_flex().gap_2().child(label("Visibility:")).child({
                let (view, style, chosen) = (cx.entity(), self.style, self.visibility.clone());
                Button::new("signature-visibility").small().outline().label(self.visibility.clone()).icon(icons::CHEVRON_DOWN).disabled(self.signature.visibility_range.is_none()).dropdown_menu(move |mut menu, _, _| {
                    for v in visibilities(style) {
                        let view = view.clone();
                        menu = menu.item(PopupMenuItem::new(*v).checked(chosen == *v).on_click(move |_, _, cx| {
                            view.update(cx, |this, cx| {
                                this.visibility = v.to_string();
                                cx.notify();
                            });
                        }));
                    }
                    menu
                })
            }))
            .child(
                h_flex()
                    .gap_1()
                    .child(div().text_sm().text_color(palette.text_secondary).child("Parameters"))
                    .child(div().flex_1())
                    .child(tool_button("signature-add", icons::ADD, "Add").on_click(cx.listener(|this, _, window, cx| this.add(window, cx))))
                    .child(tool_button("signature-remove", icons::REMOVE, "Remove").disabled(!has).on_click(cx.listener(|this, _, _, cx| this.remove(cx))))
                    .child(tool_button("signature-up", icons::MOVE_UP, "Move Up").disabled(!has).on_click(cx.listener(|this, _, _, cx| this.shift(true, cx))))
                    .child(tool_button("signature-down", icons::MOVE_DOWN, "Move Down").disabled(!has).on_click(cx.listener(|this, _, _, cx| this.shift(false, cx)))),
            )
            .child(h_flex().gap_2().px_2().child(header("Name", 160.)).child(header("Type", 200.)).child(header("Default value (for calls)", 200.)))
            .child(table)
            .child(div().text_sm().text_color(palette.text_secondary).child("Signature Preview:"))
            .child(div().p_2().border_1().border_color(palette.border).rounded_md().bg(palette.panel).font_family(mono).text_sm().child(self.preview(&request)))
            .when(self.style == ImportStyle::Java && self.signature.body_start.is_some(), |el| {
                el.child(Checkbox::new("signature-delegate").label("Delegate via overloading method").checked(self.delegate).on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.delegate = *checked;
                    cx.notify();
                })))
            })
            .child(div().text_xs().text_color(palette.text_secondary).child(match self.calls {
                _ if self.delegate => "The calls keep the old signature".to_owned(),
                0 => "No calls found".to_owned(),
                1 => "1 call will be updated".to_owned(),
                n => format!("{n} calls will be updated"),
            }))
            // Always there, so the dialog doesn't move when an error shows.
            .child(div().h(px(16.)).text_xs().text_color(palette.status_conflict).children(error))
    }
}

/// Opens the dialog; Refactor hands the request to `on_ok`.
pub fn open(style: ImportStyle, signature: Signature, calls: usize, on_ok: impl Fn(SignatureRequest, &mut Window, &mut App) + 'static, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| {
        let mut view = ChangeSignatureView {
            style,
            signature: signature.clone(),
            calls,
            name: cx.new(|cx| InputState::new(window, cx)),
            return_type: cx.new(|cx| InputState::new(window, cx)),
            throws: cx.new(|cx| InputState::new(window, cx)),
            rows: Vec::new(),
            selected: None,
            visibility: visibility_name(style, &signature.visibility).to_owned(),
            delegate: false,
            _subscriptions: Vec::new(),
        };
        view.name = view.input(signature.name.clone(), "Name", window, cx);
        view.throws = view.input(signature.throws.join(", "), "e.g. java.io.IOException, IllegalStateException", window, cx);
        view.return_type = view.input(signature.return_type.clone(), if style == ImportStyle::Java { "e.g. void" } else { "Unit when empty" }, window, cx);
        for (i, p) in signature.params.iter().enumerate() {
            let name = view.input(p.name.clone(), "Name", window, cx);
            let ty = view.input(p.ty.clone(), "Type", window, cx);
            let value = view.input(String::new(), "", window, cx);
            view.rows.push(Row { old: Some(i), name, ty, value });
        }
        view.selected = (!view.rows.is_empty()).then_some(0);
        view
    });
    let focus = view.read(cx).name.clone();
    let on_ok = Rc::new(on_ok);
    window.open_dialog(cx, move |dialog, _, _| {
        let (view_ok, on_ok) = (view.clone(), on_ok.clone());
        dialog.title("Change Signature").w(px(720.)).child(view.clone()).footer(dialogs::footer("Refactor")).on_ok(move |_, window, cx| {
            let view = view_ok.read(cx);
            let request = view.request(cx);
            if check(view.style, &view.signature, &request).is_err() {
                return false;
            }
            on_ok(request, window, cx);
            true
        })
    });
    dialogs::focus_input(&focus, window, cx);
}
