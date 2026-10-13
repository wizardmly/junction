//! Device Explorer's header, device picker, tabs, package filter button
//! and the busy overlay.

use super::*;

impl DeviceExplorer {
    pub(super) fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let (sync, filter) = (cx.entity(), cx.entity());
        let filtering = self.package_filter;
        h_flex()
            .h(px(crate::ui::common::header_height()))
            .px_2()
            .gap_1()
            .flex_shrink_0()
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Device Explorer"))
            .child(div().flex_1())
            .child(tool_button("de-options", icons::MORE_VERTICAL, "Options").dropdown_menu(move |menu, _, _| {
                let (sync, filter) = (sync.clone(), filter.clone());
                menu.item(PopupMenuItem::new("Synchronize").on_click(move |_, _, cx| sync.update(cx, |this, cx| this.synchronize(cx))))
                    .item(PopupMenuItem::new("Package Filter").checked(filtering).on_click(move |_, _, cx| filter.update(cx, |this, cx| this.toggle_package_filter(cx))))
            }))
            .child(tool_button("de-hide", icons::HIDE, "Hide").on_click(cx.listener(|_, _, _, cx| cx.emit(DeviceExplorerEvent::Hide))))
            .border_b_1()
            .border_color(palette.border)
    }

    pub(super) fn render_device_picker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let active = self.active.as_ref().and_then(|s| self.devices.iter().find(|d| &d.serial == s)).cloned();
        let devices = self.devices.clone();
        let entity = cx.entity();
        let grey = if palette.dark { rgb(0x6f737a) } else { rgb(0x818594) };
        let label: gpui_kit::AnyElement = match &active {
            Some(device) => h_flex()
                .flex_1()
                .min_w_0()
                .gap_1p5()
                .child(Icon::new(device_icon(device.kind)).small())
                .child(div().whitespace_nowrap().child(device.name.clone()))
                .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(grey).child(device.version.clone()))
                .into_any_element(),
            None => div().flex_1().text_color(palette.status_deleted).child("No Connected Devices").into_any_element(),
        };
        h_flex().px_2().py_1().flex_shrink_0().child(
            Button::new("de-device")
                .outline()
                .small()
                .w_full()
                .child(h_flex().w_full().gap_1().text_sm().child(label).child(Icon::new(icons::CHEVRON_DOWN).xsmall().text_color(palette.text_secondary)))
                .dropdown_menu(move |mut menu, _, _| {
                    for device in &devices {
                        let (entity, serial) = (entity.clone(), device.serial.clone());
                        let (name, version) = (device.name.clone(), device.version.clone());
                        menu = menu.item(
                            PopupMenuItem::element(move |_, cx| {
                                let palette = cx.palette().clone();
                                let grey = if palette.dark { rgb(0x6f737a) } else { rgb(0x818594) };
                                h_flex().gap_1p5().child(div().child(name.clone())).child(div().text_color(grey).child(version.clone()))
                            })
                            .icon(Icon::new(device_icon(device.kind)))
                            .on_click(move |_, _, cx| entity.update(cx, |this, cx| this.show_device(&serial, cx))),
                        );
                    }
                    menu
                }),
        )
    }

    pub(super) fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let current = self.tab;
        let tab = |id: &'static str, label: &'static str, value: Tab| {
            div()
                .id(id)
                .px_2()
                .h_full()
                .flex()
                .items_center()
                .text_sm()
                .cursor_pointer()
                .when(value == current, |el| el.border_b_2().border_color(palette.accent).text_color(palette.text))
                .when(value != current, |el| el.text_color(palette.text_secondary))
                .child(label)
        };
        let select = |value: Tab| {
            move |this: &mut Self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>| {
                this.tab = value;
                this.poll_processes(cx);
                cx.notify();
            }
        };
        h_flex()
            .h(px(28.))
            .px_1()
            .flex_shrink_0()
            .border_b_1()
            .border_color(palette.border)
            .child(tab("de-tab-files", "Files", Tab::Files).on_click(cx.listener(select(Tab::Files))))
            .child(tab("de-tab-processes", "Processes", Tab::Processes).on_click(cx.listener(select(Tab::Processes))))
    }

    pub(super) fn filter_button(&self, id: &'static str, cx: &mut Context<Self>) -> Button {
        let enabled = !self.app_ids.is_empty();
        let text = format!("Turn {} package filter", if self.package_filter { "off" } else { "on" });
        let tooltip = if enabled { text } else { format!("{text}\nDisabled due to no application IDs found") };
        tool_button(id, icons::LOCATE, tooltip)
            .selected(self.package_filter && enabled)
            .disabled(!enabled)
            .on_click(cx.listener(|this, _, _, cx| this.toggle_package_filter(cx)))
    }

    pub(super) fn render_busy(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let text = self.busy.clone()?;
        let palette = cx.palette().clone();
        Some(
            h_flex()
                .h(px(26.))
                .px_2()
                .gap_2()
                .flex_shrink_0()
                .border_t_1()
                .border_color(palette.border)
                .text_sm()
                .child(Spinner::new().icon(Icon::new(IconName::LoaderCircle)).small().color(palette.text_secondary))
                .child(div().min_w_0().whitespace_nowrap().overflow_hidden().text_ellipsis().child(text)),
        )
    }
}
