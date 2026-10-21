//! The Layout Inspector tool window, as Android Studio's standalone one: a
//! device (and window) picker, Refresh and Live Updates; the Component Tree,
//! the screen with each view's outline (hover and click pick a view), and
//! the selected view's attributes. The snapshot comes from `dumpsys
//! activity top`, or from UI Automator, which also sees Compose and text.

use std::cell::Cell;
use std::collections::HashSet;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::{
    Icon, Sizable as _, h_flex,
    button::Button,
    checkbox::Checkbox,
    menu::{DropdownMenu as _, PopupMenuItem},
    scroll::ScrollableElement as _,
    v_flex,
};
use gpui_kit::{
    AppContext as _, Bounds, Context, Entity, EventEmitter, FontWeight, Image, ImageFormat, InteractiveElement as _, IntoElement, MouseButton, MouseMoveEvent, ParentElement as _, Pixels, Render,
    ScrollStrategy, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, UniformListScrollHandle, Window, canvas, div, point, prelude::FluentBuilder as _, px, size,
    uniform_list,
};

use crate::domain::view_hierarchy::LayoutWindow;
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::common::tool_button;
use crate::ui::device_manager::DeviceManager;

pub enum LayoutInspectorEvent {
    Hide,
}

const LIVE_INTERVAL: Duration = Duration::from_millis(2500);
const ROW_H: f32 = 22.;

#[derive(Clone)]
struct Target {
    serial: String,
    name: String,
}

struct Snapshot {
    windows: Vec<LayoutWindow>,
    image: Option<Arc<Image>>,
    size: (u32, u32),
}

pub struct LayoutInspector {
    device_manager: Entity<DeviceManager>,
    devices: Vec<Target>,
    active: Option<String>,
    uiautomator: bool,
    live: bool,
    shown: bool,
    loading: bool,
    error: Option<String>,
    snapshot: Option<Snapshot>,
    window: usize,
    selected: Option<usize>,
    hovered: Option<usize>,
    collapsed: HashSet<usize>,
    tree_scroll: UniformListScrollHandle,
    /// Where the screen was painted: its bounds and device pixels per pixel.
    painted: Rc<Cell<Option<(Bounds<Pixels>, f32)>>>,
    _observe: Subscription,
}

impl EventEmitter<LayoutInspectorEvent> for LayoutInspector {}

impl LayoutInspector {
    pub fn new(device_manager: Entity<DeviceManager>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&device_manager, |this, dm, cx| {
            let devices: Vec<Target> = dm.read(cx).devices().iter().filter(|d| d.online).filter_map(|d| Some(Target { serial: d.serial.clone()?, name: d.name.clone() })).collect();
            this.devices = devices;
            if this.active.as_ref().is_none_or(|s| !this.devices.iter().any(|d| &d.serial == s)) {
                this.active = this.devices.first().map(|d| d.serial.clone());
                // A device arriving while the window shows is inspected.
                if this.shown && this.active.is_some() && this.snapshot.is_none() {
                    this.refresh(cx);
                }
            }
            cx.notify();
        });
        Self {
            device_manager,
            devices: Vec::new(),
            active: None,
            uiautomator: false,
            live: false,
            shown: false,
            loading: false,
            error: None,
            snapshot: None,
            window: 0,
            selected: None,
            hovered: None,
            collapsed: HashSet::new(),
            tree_scroll: UniformListScrollHandle::new(),
            painted: Rc::default(),
            _observe: observe,
        }
    }

    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if self.shown == shown {
            return;
        }
        self.shown = shown;
        if shown && self.snapshot.is_none() && self.active.is_some() {
            self.refresh(cx);
        }
    }

    fn current(&self) -> Option<&LayoutWindow> {
        self.snapshot.as_ref()?.windows.get(self.window)
    }

    /// Takes a new snapshot; the selected view stays picked if it is still there.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let (Some(serial), Some(adb)) = (self.active.clone(), self.device_manager.read(cx).adb()) else {
            self.error = Some("No device: connect one, or start a virtual device in Device Manager".into());
            cx.notify();
            return;
        };
        if self.loading {
            return;
        }
        self.loading = true;
        let uiautomator = self.uiautomator;
        let task = cx.background_spawn(async move { crate::usecase::inspect_layout::capture(&crate::infra::adb_layout::AdbLayout { adb }, &serial, uiautomator) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(capture) => {
                        let picked = this.selected.and_then(|i| this.current()?.nodes.get(i)).map(|n| (n.class.clone(), n.id.clone(), n.bounds));
                        let name = this.current().map(|w| w.name.clone());
                        this.window = name.and_then(|n| capture.windows.iter().position(|w| w.name == n)).unwrap_or(0);
                        let image = (!capture.png.is_empty()).then(|| Arc::new(Image::from_bytes(ImageFormat::Png, capture.png)));
                        this.snapshot = Some(Snapshot { windows: capture.windows, image, size: capture.size });
                        this.selected = picked.and_then(|(class, id, bounds)| this.current()?.nodes.iter().position(|n| n.class == class && n.id == id && n.bounds == bounds));
                        this.hovered = None;
                        this.error = None;
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn set_live(&mut self, live: bool, cx: &mut Context<Self>) {
        self.live = live;
        if live {
            self.refresh(cx);
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(LIVE_INTERVAL).await;
                    let go = this
                        .update(cx, |this, cx| {
                            if this.live && this.shown && !this.loading {
                                this.refresh(cx);
                            }
                            this.live
                        })
                        .unwrap_or(false);
                    if !go {
                        break;
                    }
                }
            })
            .detach();
        }
        cx.notify();
    }

    fn show_device(&mut self, serial: String, cx: &mut Context<Self>) {
        self.active = Some(serial);
        self.snapshot = None;
        self.selected = None;
        self.collapsed.clear();
        self.refresh(cx);
    }

    /// The tree's rows: the nodes not inside a collapsed one.
    fn rows(&self) -> Vec<usize> {
        let Some(window) = self.current() else { return Vec::new() };
        let mut rows = Vec::new();
        let mut skip_below: Option<usize> = None;
        for (ix, node) in window.nodes.iter().enumerate() {
            if skip_below.is_some_and(|d| node.depth > d) {
                continue;
            }
            skip_below = None;
            rows.push(ix);
            if self.collapsed.contains(&ix) {
                skip_below = Some(node.depth);
            }
        }
        rows
    }

    /// Picks a view: its ancestors open and its row scrolled to.
    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.selected = Some(ix);
        if let Some(window) = self.current() {
            let mut ancestors = Vec::new();
            let mut at = window.parent(ix);
            while let Some(p) = at {
                ancestors.push(p);
                at = window.parent(p);
            }
            for p in ancestors {
                self.collapsed.remove(&p);
            }
        }
        if let Some(row) = self.rows().iter().position(|&r| r == ix) {
            self.tree_scroll.scroll_to_item(row, ScrollStrategy::Center);
        }
        cx.notify();
    }

    /// The view under a point of the painted screen.
    fn view_at(&self, position: gpui_kit::Point<Pixels>) -> Option<usize> {
        let (bounds, scale) = self.painted.get()?;
        if !bounds.contains(&position) {
            return None;
        }
        let x = (f32::from(position.x - bounds.origin.x) * scale) as i32;
        let y = (f32::from(position.y - bounds.origin.y) * scale) as i32;
        self.current()?.view_at(x, y)
    }

    fn render_tree_rows(&mut self, range: Range<usize>, _: &mut Window, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let palette = cx.palette().clone();
        let rows = self.rows();
        let Some(window) = self.current() else { return Vec::new() };
        rows[range.start.min(rows.len())..range.end.min(rows.len())]
            .iter()
            .map(|&ix| {
                let node = &window.nodes[ix];
                let parent = !window.subtree(ix).is_empty();
                let open = !self.collapsed.contains(&ix);
                let selected = self.selected == Some(ix);
                let hovered = self.hovered == Some(ix);
                let label = node.short_class().to_owned();
                let detail = match (node.id_name(), &node.text) {
                    (Some(id), Some(text)) => format!("@id/{id}  \"{text}\""),
                    (Some(id), None) => format!("@id/{id}"),
                    (None, Some(text)) => format!("\"{text}\""),
                    (None, None) => String::new(),
                };
                h_flex()
                    .id(("li-row", ix))
                    .w_full()
                    .h(px(ROW_H))
                    .pl(px(4. + node.depth as f32 * 14.))
                    .pr_2()
                    .gap_1()
                    .text_sm()
                    .whitespace_nowrap()
                    .when(selected, |el| el.bg(palette.selection))
                    .when(!selected && hovered, |el| el.bg(palette.hover))
                    .when(!node.visible, |el| el.text_color(palette.text_secondary))
                    .child(
                        div()
                            .id(("li-toggle", ix))
                            .w(px(14.))
                            .flex_none()
                            .when(parent, |el| {
                                el.child(Icon::new(if open { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT }).xsmall()).on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    if !this.collapsed.remove(&ix) {
                                        this.collapsed.insert(ix);
                                    }
                                    cx.notify();
                                }))
                            }),
                    )
                    .child(label)
                    .when(!detail.is_empty(), |el| el.child(div().text_color(palette.text_secondary).child(detail)))
                    .on_mouse_move(cx.listener(move |this, _: &MouseMoveEvent, _, cx| {
                        if this.hovered != Some(ix) {
                            this.hovered = Some(ix);
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| this.select(ix, cx)))
                    .into_any_element()
            })
            .collect()
    }

    fn render_screen(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let Some(snapshot) = &self.snapshot else {
            return div().size_full().flex().items_center().justify_center().text_sm().text_color(palette.text_secondary).child(if self.loading { "Taking a snapshot…" } else { "" }).into_any_element();
        };
        let image = snapshot.image.clone();
        let (width, height) = (snapshot.size.0.max(1) as f32, snapshot.size.1.max(1) as f32);
        let boxes: Vec<(crate::domain::view_hierarchy::Rect, bool)> = self.current().map(|w| (0..w.nodes.len()).filter(|&i| w.shown(i)).map(|i| (w.nodes[i].bounds, false)).collect()).unwrap_or_default();
        let picked: Vec<(crate::domain::view_hierarchy::Rect, bool)> = [self.hovered.map(|h| (h, false)), self.selected.map(|s| (s, true))]
            .into_iter()
            .flatten()
            .filter_map(|(ix, selected)| Some((self.current()?.nodes.get(ix)?.bounds, selected)))
            .collect();
        let painted = self.painted.clone();
        let (outline, hover, accent, background) = (palette.border, palette.text_secondary, palette.accent, palette.panel);
        div()
            .id("li-screen")
            .size_full()
            .p_2()
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        // Fit the screen, centered.
                        let fit = (f32::from(bounds.size.width) / width).min(f32::from(bounds.size.height) / height);
                        let shown = size(px(width * fit), px(height * fit));
                        let origin = point(bounds.origin.x + (bounds.size.width - shown.width) / 2., bounds.origin.y + (bounds.size.height - shown.height) / 2.);
                        let screen = Bounds::new(origin, shown);
                        painted.set(Some((screen, 1. / fit)));
                        window.paint_quad(gpui_kit::fill(screen, background));
                        if let Some(render) = image.as_ref().and_then(|i| i.clone().use_render_image(window, cx)) {
                            let _ = window.paint_image(screen, screen, Default::default(), render, 0, false);
                        }
                        let rect = |r: &crate::domain::view_hierarchy::Rect| {
                            Bounds::new(point(origin.x + px(r.left as f32 * fit), origin.y + px(r.top as f32 * fit)), size(px(r.width().max(0) as f32 * fit), px(r.height().max(0) as f32 * fit)))
                        };
                        for (r, _) in &boxes {
                            window.paint_quad(gpui_kit::quad(rect(r), px(0.), gpui_kit::transparent_black(), px(1.), outline, Default::default()));
                        }
                        for (r, selected) in &picked {
                            let color = if *selected { accent } else { hover };
                            window.paint_quad(gpui_kit::quad(rect(r), px(0.), gpui_kit::transparent_black(), px(if *selected { 2. } else { 1. }), color, Default::default()));
                        }
                    },
                )
                .size_full(),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                let at = this.view_at(event.position);
                if at != this.hovered {
                    this.hovered = at;
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui_kit::MouseDownEvent, _, cx| {
                    if let Some(ix) = this.view_at(event.position) {
                        this.select(ix, cx);
                    }
                }),
            )
            .into_any_element()
    }

    fn render_attributes(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let node = self.selected.and_then(|ix| self.current()?.nodes.get(ix));
        let mut list = v_flex().gap_px();
        if let Some(node) = node {
            list = list.child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(node.class.clone())).when_some(node.id.clone(), |el, id| el.child(div().text_xs().text_color(palette.text_secondary).child(id)));
            for (key, value) in &node.attributes {
                list = list.child(
                    h_flex()
                        .gap_2()
                        .text_xs()
                        .child(div().w(px(110.)).flex_none().text_color(palette.text_secondary).child(key.clone()))
                        .child(div().flex_1().min_w_0().overflow_hidden().child(value.clone())),
                );
            }
        } else {
            list = list.child(div().text_xs().text_color(palette.text_secondary).child("Select a view to see its attributes"));
        }
        div().id("li-attributes").size_full().p_2().overflow_y_scrollbar().child(list)
    }
}

impl Render for LayoutInspector {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let entity = cx.entity();
        let devices = self.devices.clone();
        let device_label = self.active.as_ref().and_then(|s| self.devices.iter().find(|d| &d.serial == s)).map(|d| d.name.clone()).unwrap_or_else(|| "No Connected Devices".into());
        let device_picker = Button::new("li-device").outline().xsmall().label(device_label).child(Icon::new(icons::CHEVRON_DOWN).xsmall()).dropdown_menu({
            let entity = entity.clone();
            move |mut menu, _, _| {
                for d in &devices {
                    let (entity, serial) = (entity.clone(), d.serial.clone());
                    menu = menu.item(PopupMenuItem::new(d.name.clone()).on_click(move |_, _, cx| entity.update(cx, |this, cx| this.show_device(serial.clone(), cx))));
                }
                menu
            }
        });
        let windows: Vec<String> = self.snapshot.as_ref().map(|s| s.windows.iter().map(|w| w.name.clone()).collect()).unwrap_or_default();
        let window_picker = (windows.len() > 1).then(|| {
            let entity = entity.clone();
            let current = windows.get(self.window).cloned().unwrap_or_default();
            Button::new("li-window").outline().xsmall().label(current).child(Icon::new(icons::CHEVRON_DOWN).xsmall()).dropdown_menu(move |mut menu, _, _| {
                for (ix, name) in windows.iter().enumerate() {
                    let entity = entity.clone();
                    menu = menu.item(PopupMenuItem::new(name.clone()).on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.window = ix;
                            this.selected = None;
                            this.hovered = None;
                            this.collapsed.clear();
                            cx.notify();
                        })
                    }));
                }
                menu
            })
        });
        let (live_entity, ui_entity) = (entity.clone(), entity.clone());
        let header = h_flex()
            .h(px(crate::ui::common::header_height()))
            .px_2()
            .gap_2()
            .flex_shrink_0()
            .border_b_1()
            .border_color(palette.border)
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Layout Inspector"))
            .child(device_picker)
            .children(window_picker)
            .child(tool_button("li-refresh", icons::REFRESH, "Refresh Layout").on_click(cx.listener(|this, _, _, cx| this.refresh(cx))))
            .child(Checkbox::new("li-live").label("Live Updates").checked(self.live).on_change(move |v, _, cx| {
                let v = *v;
                live_entity.update(cx, |this, cx| this.set_live(v, cx))
            }))
            .child(Checkbox::new("li-uiautomator").label("UI Automator").checked(self.uiautomator).on_change(move |v, _, cx| {
                let v = *v;
                ui_entity.update(cx, |this, cx| {
                    this.uiautomator = v;
                    this.refresh(cx);
                })
            }))
            .child(div().flex_1())
            .when(self.loading, |el| el.child(div().text_xs().text_color(palette.text_secondary).child("Refreshing…")))
            .child(tool_button("li-hide", icons::HIDE, "Hide").on_click(cx.listener(|_, _, _, cx| cx.emit(LayoutInspectorEvent::Hide))));
        let count = self.rows().len();
        let tree = v_flex()
            .w(px(260.))
            .flex_none()
            .h_full()
            .overflow_hidden()
            .border_r_1()
            .border_color(palette.border)
            .child(div().px_2().py_1().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(palette.text_secondary).child("Component Tree"))
            .child(uniform_list("li-tree", count, cx.processor(Self::render_tree_rows)).track_scroll(&self.tree_scroll).flex_1().min_h_0())
            .on_mouse_move(cx.listener(|_, _: &MouseMoveEvent, _, _| {}));
        let body = h_flex()
            .flex_1()
            .min_h_0()
            .child(tree)
            .child(div().flex_1().min_w_0().h_full().child(self.render_screen(cx)))
            .child(
                v_flex()
                    .w(px(260.))
                    .flex_none()
                    .h_full()
                    .border_l_1()
                    .border_color(palette.border)
                    .child(div().px_2().py_1().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(palette.text_secondary).child("Attributes"))
                    .child(div().flex_1().min_h_0().child(self.render_attributes(cx))),
            );
        v_flex()
            .size_full()
            .child(header)
            .when_some(self.error.clone(), |el, e| el.child(div().px_2().py_1().text_xs().text_color(palette.status_conflict).child(SharedString::from(e))))
            .when(self.snapshot.is_none() && !self.loading && self.error.is_none(), |el| {
                el.child(div().p_4().text_sm().text_color(palette.text_secondary).child("Pick a device and press Refresh Layout to inspect what is on its screen."))
            })
            .child(body)
    }
}
