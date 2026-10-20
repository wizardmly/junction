//! Logcat's tabs, as Android Studio's tool window has them: each tab its
//! own device, filter and log; "+" opens a new one ("Logcat (2)"), the
//! tab's × or middle click closes it (not the last), and its context menu
//! renames it or closes it and the others. A tab's panel splits (Split
//! Panels › Split Right / Split Down, AS's `SplittingPanel`) into panels
//! each with its own device, filter and log; Close takes one away.

use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::{
    WindowExt as _, h_flex,
    input::{Input, InputState},
    menu::{ContextMenuExt as _, PopupMenuItem},
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EntityId, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render, SharedString,
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

/// A tab's panels: one log, or two side by side (`down`: one above the
/// other), as AS's nested `OnePixelSplitter`s.
enum Pane {
    Log(Entity<LogcatView>),
    Split { id: usize, down: bool, first: Box<Pane>, second: Box<Pane> },
}

impl Pane {
    fn first_log(&self) -> &Entity<LogcatView> {
        match self {
            Pane::Log(view) => view,
            Pane::Split { first, .. } => first.first_log(),
        }
    }

    fn logs(&self, out: &mut Vec<Entity<LogcatView>>) {
        match self {
            Pane::Log(view) => out.push(view.clone()),
            Pane::Split { first, second, .. } => {
                first.logs(out);
                second.logs(out);
            }
        }
    }

    fn contains(&self, id: EntityId) -> bool {
        match self {
            Pane::Log(view) => view.entity_id() == id,
            Pane::Split { first, second, .. } => first.contains(id) || second.contains(id),
        }
    }

    /// Puts `new` beside the log `id`.
    fn split(&mut self, id: EntityId, down: bool, new: Entity<LogcatView>, split_id: usize) -> bool {
        match self {
            Pane::Log(view) if view.entity_id() == id => {
                let old = std::mem::replace(self, Pane::Log(new.clone()));
                *self = Pane::Split { id: split_id, down, first: Box::new(old), second: Box::new(Pane::Log(new)) };
                true
            }
            Pane::Log(_) => false,
            Pane::Split { first, second, .. } => first.split(id, down, new.clone(), split_id) || second.split(id, down, new, split_id),
        }
    }

    /// Takes the log `id` away, its neighbour taking the split's place.
    fn remove(&mut self, id: EntityId) -> bool {
        let Pane::Split { first, second, .. } = self else { return false };
        let keep = if matches!(&**first, Pane::Log(v) if v.entity_id() == id) {
            std::mem::replace(&mut **second, Pane::Log(first.first_log().clone()))
        } else if matches!(&**second, Pane::Log(v) if v.entity_id() == id) {
            std::mem::replace(&mut **first, Pane::Log(second.first_log().clone()))
        } else {
            return first.remove(id) || second.remove(id);
        };
        *self = keep;
        true
    }
}

struct Tab {
    name: String,
    pane: Pane,
}

pub struct LogcatTabs {
    devices: Entity<DeviceManager>,
    run: Entity<RunService>,
    project: Rc<dyn Fn(&App) -> Option<PathBuf>>,
    tabs: Vec<Tab>,
    active: usize,
    /// The tool window has been shown: new tabs start reading at once.
    shown: bool,
    /// The panel last clicked in: it takes the focus.
    focused: Option<EntityId>,
    subscriptions: HashMap<EntityId, Subscription>,
    next_split: usize,
}

impl EventEmitter<LogcatEvent> for LogcatTabs {}

impl Focusable for LogcatTabs {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        let pane = &self.tabs[self.active].pane;
        let view = self.focused.and_then(|id| { let mut all = Vec::new(); pane.logs(&mut all); all.into_iter().find(|v| v.entity_id() == id) });
        view.unwrap_or_else(|| pane.first_log().clone()).focus_handle(cx)
    }
}

impl LogcatTabs {
    pub fn new(devices: Entity<DeviceManager>, run: Entity<RunService>, project: Rc<dyn Fn(&App) -> Option<PathBuf>>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self { devices, run, project, tabs: Vec::new(), active: 0, shown: false, focused: None, subscriptions: HashMap::new(), next_split: 0 };
        this.add_tab(window, cx);
        this
    }

    fn names(&self) -> Vec<String> {
        self.tabs.iter().map(|t| t.name.clone()).collect()
    }

    /// A new log panel, reading once the tool window has shown.
    fn new_log(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<LogcatView> {
        let (devices, run, project) = (self.devices.clone(), self.run.clone(), self.project.clone());
        let view = cx.new(|cx| {
            let mut view = LogcatView::new(devices, run, project, window, cx);
            view.embedded = true;
            view
        });
        // A panel's frame links open in the editor, as the tool window's do.
        let subscription = cx.subscribe_in(&view, window, |this, view, event: &LogcatEvent, window, cx| match event {
            LogcatEvent::OpenFrame { package, file, line } => cx.emit(LogcatEvent::OpenFrame { package: package.clone(), file: file.clone(), line: *line }),
            LogcatEvent::Split { down } => this.split(view.entity_id(), *down, window, cx),
            LogcatEvent::ClosePanel => this.close_panel(view.entity_id(), cx),
            LogcatEvent::Hide => {}
        });
        self.subscriptions.insert(view.entity_id(), subscription);
        if self.shown {
            view.update(cx, |v, cx| v.set_shown(true, cx));
        }
        view
    }

    /// New Tab ("+"): a log of its own, showing.
    fn add_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = self.new_log(window, cx);
        self.tabs.push(Tab { name: tab_name(&self.names()), pane: Pane::Log(view) });
        self.active = self.tabs.len() - 1;
        cx.notify();
    }

    /// Split Right / Split Down: a new panel beside the log.
    fn split(&mut self, id: EntityId, down: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter().position(|t| t.pane.contains(id)) else { return };
        let view = self.new_log(window, cx);
        self.next_split += 1;
        self.tabs[tab].pane.split(id, down, view.clone(), self.next_split);
        self.active = tab;
        self.focused = Some(view.entity_id());
        view.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    /// Close (Split Panels): the panel goes; a tab's only panel closes
    /// the tab.
    fn close_panel(&mut self, id: EntityId, cx: &mut Context<Self>) {
        let Some(ix) = self.tabs.iter().position(|t| t.pane.contains(id)) else { return };
        if matches!(self.tabs[ix].pane, Pane::Log(_)) {
            self.close_tab(ix, cx);
            return;
        }
        let mut logs = Vec::new();
        self.tabs[ix].pane.logs(&mut logs);
        if self.tabs[ix].pane.remove(id) {
            if let Some(view) = logs.into_iter().find(|v| v.entity_id() == id) {
                self.drop_log(&view, cx);
            }
        }
        cx.notify();
    }

    fn drop_log(&mut self, view: &Entity<LogcatView>, cx: &mut Context<Self>) {
        self.subscriptions.remove(&view.entity_id());
        if self.focused == Some(view.entity_id()) {
            self.focused = None;
        }
        view.update(cx, |v, _| v.shutdown());
    }

    fn drop_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        let mut logs = Vec::new();
        tab.pane.logs(&mut logs);
        for view in logs {
            self.drop_log(&view, cx);
        }
    }

    fn close_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.tabs.len() <= 1 || ix >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(ix);
        self.drop_tab(tab, cx);
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
        for tab in std::mem::take(&mut self.tabs) {
            self.drop_tab(tab, cx);
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
        let mut logs = Vec::new();
        for tab in &self.tabs {
            tab.pane.logs(&mut logs);
        }
        for view in logs {
            view.update(cx, |v, cx| v.set_shown(shown, cx));
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
                this.focused = None;
                this.tabs[ix].pane.first_log().focus_handle(cx).focus(window, cx);
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
            .child(div().flex_1().min_h_0().child(render_pane(&self.tabs[self.active].pane, palette.border)))
    }
}

/// A tab's panels, split with a one-pixel divider that drags.
fn render_pane(pane: &Pane, border: gpui_kit::Hsla) -> gpui_kit::AnyElement {
    use gpui_kit::component::resizable::{h_resizable, resizable_panel, v_resizable};
    match pane {
        Pane::Log(view) => div().size_full().child(view.clone()).into_any_element(),
        Pane::Split { id, down, first, second } => {
            let (a, b) = (render_pane(first, border), render_pane(second, border));
            let id = SharedString::from(format!("logcat-split-{id}"));
            let group = if *down { v_resizable(id) } else { h_resizable(id) };
            group.child(resizable_panel().child(a)).child(resizable_panel().child(div().size_full().when(*down, |el| el.border_t_1()).when(!*down, |el| el.border_l_1()).border_color(border).child(b))).into_any_element()
        }
    }
}
