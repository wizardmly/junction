//! IntelliJ's Reformat File dialog (Code › Reformat File…, Ctrl+Alt+Shift+L,
//! `LayoutCodeDialog`): the scope (only changes uncommitted to VCS, the
//! selected text, the whole file) and what runs with the reformat
//! (optimize imports, rearrange code, code cleanup); the last run's choices come back next
//! time, as `LastRunReformatCodeOptionsProvider` keeps them.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::{
    WindowExt as _,
    checkbox::Checkbox,
    radio::{Radio, RadioGroup},
    v_flex,
};
use gpui_kit::{App, AppContext as _, Context, IntoElement, ParentElement as _, Render, SharedString, Styled as _, Window, div, prelude::FluentBuilder as _, px};

use crate::theme::ActivePalette as _;
use crate::ui::dialogs;

/// The text Reformat File works on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReformatScope {
    VcsChanged,
    Selected,
    WholeFile,
}

/// What Run asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReformatOptions {
    pub scope: ReformatScope,
    pub optimize_imports: bool,
    /// Java's members in the default arrangement first.
    pub rearrange: bool,
    pub code_cleanup: bool,
}

thread_local! {
    /// The last run's options: optimize imports, code cleanup, whether it
    /// processed the changed text, and rearrange code.
    static LAST: Cell<(bool, bool, bool, bool)> = const { Cell::new((false, false, false, false)) };
}

/// Optimize imports, code cleanup and rearrange code as the last Reformat
/// File run chose them, for Reformat Code without the dialog.
pub fn last_options() -> (bool, bool, bool) {
    let (imports, cleanup, _, rearrange) = LAST.with(Cell::get);
    (imports, cleanup, rearrange)
}

struct ReformatView {
    scope: ReformatScope,
    selected: bool,
    changed: bool,
    imports: bool,
    optimize_imports: bool,
    /// Java: Rearrange code is offered.
    java: bool,
    rearrange: bool,
    code_cleanup: bool,
}

impl ReformatView {
    fn options(&self) -> ReformatOptions {
        ReformatOptions { scope: self.scope, optimize_imports: self.imports && self.optimize_imports, rearrange: self.java && self.rearrange, code_cleanup: self.code_cleanup }
    }
}

impl Render for ReformatView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let scopes = [ReformatScope::VcsChanged, ReformatScope::Selected, ReformatScope::WholeFile];
        let selected = scopes.iter().position(|s| *s == self.scope);
        let heading = |text: &'static str| div().text_sm().text_color(palette.text_secondary).child(text);
        v_flex()
            .gap_2()
            .child(heading("Scope"))
            .child(
                RadioGroup::vertical("reformat-scope")
                    .child(Radio::new("reformat-vcs").label("Only changes uncommitted to VCS").disabled(!self.changed))
                    .child(Radio::new("reformat-selected").label("Selected text").disabled(!self.selected).when(!self.selected, |r| r.tooltip("No text selected in editor")))
                    .child(Radio::new("reformat-whole").label("Whole file"))
                    .selected_index(selected)
                    .on_change(cx.listener(move |this, ix: &usize, _, cx| {
                        this.scope = scopes[*ix];
                        cx.notify();
                    })),
            )
            .child(heading("Options"))
            .when(self.imports, |el| {
                el.child(Checkbox::new("reformat-imports").label("Optimize imports").checked(self.optimize_imports).on_change(cx.listener(|this, v: &bool, _, cx| {
                    this.optimize_imports = *v;
                    cx.notify();
                })))
            })
            .when(self.java, |el| {
                el.child(Checkbox::new("reformat-rearrange").label("Rearrange code").checked(self.rearrange).on_change(cx.listener(|this, v: &bool, _, cx| {
                    this.rearrange = *v;
                    cx.notify();
                })))
            })
            .child(Checkbox::new("reformat-cleanup").label("Code cleanup").checked(self.code_cleanup).on_change(cx.listener(|this, v: &bool, _, cx| {
                this.code_cleanup = *v;
                cx.notify();
            })))
    }
}

/// Opens "Reformat File: `name`". `selected`: the editor has a selection;
/// `changed`: the file has changes against VCS; `imports`: the language has
/// imports to optimize; `java`: its code can be rearranged.
#[allow(clippy::too_many_arguments)]
pub fn open(name: &str, selected: bool, changed: bool, imports: bool, java: bool, on_run: impl Fn(ReformatOptions, &mut Window, &mut App) + 'static, window: &mut Window, cx: &mut App) {
    let (optimize_imports, code_cleanup, last_changed, rearrange) = LAST.with(Cell::get);
    // A selection is what's reformatted; else the changed text when the
    // last run took it and there is some; else the file.
    let scope = if selected {
        ReformatScope::Selected
    } else if last_changed && changed {
        ReformatScope::VcsChanged
    } else {
        ReformatScope::WholeFile
    };
    let view = cx.new(|_| ReformatView { scope, selected, changed, imports, optimize_imports, java, rearrange, code_cleanup });
    let on_run = Rc::new(on_run);
    let title = SharedString::from(format!("Reformat File: {name}"));
    window.open_dialog(cx, move |dialog, _, _| {
        let (view, on_run) = (view.clone(), on_run.clone());
        dialog.title(title.clone()).w(px(380.)).child(view.clone()).footer(dialogs::footer("Run")).on_ok(move |_, window, cx| {
            let view = view.read(cx);
            let options = view.options();
            LAST.with(|l| {
                let last = l.get();
                l.set((if view.imports { view.optimize_imports } else { last.0 }, view.code_cleanup, options.scope == ReformatScope::VcsChanged, if view.java { view.rearrange } else { last.3 }))
            });
            on_run(options, window, cx);
            true
        })
    });
}
