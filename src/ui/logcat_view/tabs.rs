//! Logcat's tabs, as Android Studio's tool window has them: each tab its
//! own device, filter and log; "+" opens a new one ("Logcat (2)"), the
//! tab's × or middle click closes it (not the last), and its context menu
//! renames it or closes it and the others.

use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::{
    WindowExt as _, h_flex,
    input::{Input, InputState},
    menu::{ContextMenuExt as _, PopupMenuItem},
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px,
};

use super::{LogcatEvent, LogcatView};
use crate::domain::logcat::tab_name;
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::common::tool_button;
use crate::ui::device_manager::DeviceManager;
use crate::ui::dialogs;
use crate::ui::run_service::RunService;

struct Tab {
    name: String,
    view: Entity<LogcatView>,
    _subscription: Subscription,
}

pub struct LogcatTabs {
    devices: Entity<DeviceManager>,
    run: Entity<RunService>,
    project: Rc<dyn Fn(&App) -> Option<PathBuf>>,
    tabs: Vec<Tab>,
    active: usize,
    /// The tool window has been shown: new tabs start reading at once.
    shown: bool,
}

impl EventEmitter<LogcatEvent> for LogcatTabs {}

impl Focusable for LogcatTabs {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.tabs[self.active].view.focus_handle(cx)
    }
}

impl LogcatTabs {
    pub fn new(devices: Entity<DeviceManager>, run: Entity<RunService>, project: Rc<dyn Fn(&App) -> Option<PathBuf>>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self { devices, run, project, tabs: Vec::new(), active: 0, shown: false };
        this.add_tab(window, cx);
        this
    }

    fn names(&self) -> Vec<String> {
        self.tabs.iter().map(|t| t.name.clone()).collect()
    }

    /// New Tab ("+"): a log of its own, showing.
    fn add_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (devices, run, project) = (self.devices.clone(), self.run.clone(), self.project.clone());
        let view = cx.new(|cx| {
            let mut view = LogcatView::new(devices, run, project, window, cx);
            view.embedded = true;
            view
        });
        // A tab's frame links open in the editor, as the tool window's do.
        let _subscription = cx.subscribe(&view, |_, _, event: &LogcatEvent, cx| {
            if let LogcatEvent::OpenFrame { package, file, line } = event {
                cx.emit(LogcatEvent::OpenFrame { package: package.clone(), file: file.clone(), line: *line });
            }
        });
        if self.shown {
            view.update(cx, |v, cx| v.set_shown(true, cx));
        }
        self.tabs.push(Tab { name: tab_name(&self.names()), view, _subscription });
        self.active = self.tabs.len() - 1;
        cx.notify();
    }

    fn close_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.tabs.len() <= 1 || ix >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(ix);
        tab.view.update(cx, |v, _| v.shutdown());
        if self.active > ix || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        cx.notify();
    }

    fn close_others(&mut self, keep: usize, cx: &mut Context<Self>) {
        if keep >= self.tabs.len() {
            return;
        }
        let kept = self.tabs.remove(keep);
        for tab in self.tabs.drain(..) {
            tab.view.update(cx, |v, _| v.shutdown());
        }
        self.tabs.push(kept);
        self.active = 0;
        cx.notify();
    }

    /// Rename Tab: a name not empty and not another tab's.
    fn rename_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(ix) else { return };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(tab.name.clone()));
        let this = cx.entity().downgrade();
        let focus = input.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let (input, this) = (input.clone(), this.clone());
            dialog.title("Rename Tab").w(px(360.)).child(Input::new(&input)).footer(dialogs::footer("OK")).on_ok(move |_, _, cx| {
                let name = input.read(cx).value().trim().to_owned();
                let Some(this) = this.upgrade() else { return true };
                let taken = this.read(cx).tabs.iter().enumerate().any(|(i, t)| i != ix && t.name == name);
                if name.is_empty() || taken {
                    return false;
                }
                this.update(cx, |this, cx| {
                    if let Some(tab) = this.tabs.get_mut(ix) {
                        tab.name = name;
                    }
                    cx.notify();
                });
                true
            })
        });
        dialogs::focus_input(&focus, window, cx);
        focus.update(cx, |s, cx| s.select_all(window, cx));
    }

    /// The tool window showed or hid: every tab's log keeps reading.
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        self.shown |= shown;
        for tab in &self.tabs {
            tab.view.update(cx, |v, cx| v.set_shown(shown, cx));
        }
    }

    fn render_tab(&self, ix: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let active = ix == self.active;
        let closable = self.tabs.len() > 1;
        let this = cx.entity().downgrade();
        h_flex()
            .id(("logcat-tab", ix))
            .h_full()
            .pl_2()
            .pr(px(if closable { 2. } else { 8. }))
            .gap_1()
            .text_sm()
            .cursor_pointer()
            .border_b_2()
            .border_color(if active { palette.accent } else { gpui_kit::transparent_black() })
            .when(!active, |el| el.hover(|s| s.bg(palette.hover)))
            .child(SharedString::from(self.tabs[ix].name.clone()))
            .when(closable, |el| {
                el.child(tool_button(("logcat-tab-close", ix), icons::CLOSE_SMALL, "Close Tab").on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.close_tab(ix, cx);
                })))
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                this.active = ix;
                this.tabs[ix].view.focus_handle(cx).focus(window, cx);
                cx.notify();
            }))
            .on_mouse_up(MouseButton::Middle, cx.listener(move |this, _, _, cx| this.close_tab(ix, cx)))
            .context_menu(move |menu, _, _| {
                let (r, c, o) = (this.clone(), this.clone(), this.clone());
                menu.item(PopupMenuItem::new("Rename Tab…").on_click(move |_, window, cx| {
                    r.update(cx, |this, cx| this.rename_tab(ix, window, cx)).ok();
                }))
                .separator()
                .item(PopupMenuItem::new("Close Tab").disabled(!closable).on_click(move |_, _, cx| {
                    c.update(cx, |this, cx| this.close_tab(ix, cx)).ok();
                }))
                .item(PopupMenuItem::new("Close Other Tabs").disabled(!closable).on_click(move |_, _, cx| {
                    o.update(cx, |this, cx| this.close_others(ix, cx)).ok();
                }))
            })
    }
}

impl Render for LogcatTabs {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let tabs: Vec<_> = (0..self.tabs.len()).map(|ix| self.render_tab(ix, cx).into_any_element()).collect();
        gpui_kit::component::v_flex()
            .size_full()
            .bg(palette.panel)
            .child(
                h_flex()
                    .h(px(30.))
                    .px_1()
                    .gap_0p5()
                    .flex_none()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(div().px_1().text_sm().font_weight(FontWeight::SEMIBOLD).child("Logcat"))
                    .children(tabs)
                    .child(tool_button("logcat-new-tab", icons::ADD, "New Tab").on_click(cx.listener(|this, _, window, cx| this.add_tab(window, cx))))
                    .child(div().flex_1())
                    .child(tool_button("logcat-hide", icons::HIDE, "Hide").on_click(cx.listener(|_, _, _, cx| cx.emit(LogcatEvent::Hide)))),
            )
            .child(div().flex_1().min_h_0().child(self.tabs[self.active].view.clone()))
    }
}
