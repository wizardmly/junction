//! Android Studio's Select Multiple Devices dialog (the device dropdown's
//! "Select Multiple Devices…"): a check box per device; OK makes Run
//! target the checked ones together.

use gpui_kit::component::{Icon, Sizable as _, WindowExt as _, checkbox::Checkbox, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, Window, div, px};

use crate::android::{Device, Kind};
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::dialogs;
use crate::ui::run_service::RunService;

struct DeviceList {
    devices: Vec<(Device, bool)>,
}

impl Render for DeviceList {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mut list = v_flex().gap_1();
        for (ix, (device, checked)) in self.devices.iter().enumerate() {
            let icon = if device.kind == Kind::Virtual { icons::VIRTUAL_DEVICE_PHONE } else { icons::PHYSICAL_DEVICE_PHONE };
            let this = cx.entity().downgrade();
            list = list.child(
                h_flex()
                    .gap_2()
                    .child(Checkbox::new(("md-device", ix)).checked(*checked).on_change(move |value: &bool, _, cx| {
                        let value = *value;
                        this.update(cx, |v, cx| {
                            v.devices[ix].1 = value;
                            cx.notify();
                        })
                        .ok();
                    }))
                    .child(Icon::new(icon).small())
                    .child(div().text_sm().child(device.name.clone()))
                    .when_some((!device.online).then_some("Not running"), |el, note| el.child(div().text_xs().text_color(palette.text_secondary).child(note))),
            );
        }
        if self.devices.is_empty() {
            list = list.child(div().text_sm().text_color(palette.text_secondary).child("No devices"));
        }
        list
    }
}


/// Opens the dialog over Run's devices, the current targets checked.
pub fn open(service: Entity<RunService>, window: &mut Window, cx: &mut App) {
    let (all, picked) = {
        let s = service.read(cx);
        let picked: Vec<String> = if s.multiple(cx).is_some() { s.targets(cx).into_iter().map(|d| d.id).collect() } else { Vec::new() };
        (s.devices(cx), picked)
    };
    let view = cx.new(|_| DeviceList { devices: all.into_iter().map(|d| { let on = picked.contains(&d.id); (d, on) }).collect() });
    window.open_dialog(cx, move |dialog, _, _| {
        let (view, service) = (view.clone(), service.clone());
        let list = view.clone();
        dialog
            .title("Select Multiple Devices")
            .w(px(420.))
            .child(div().text_sm().pb_2().child("Available devices"))
            .child(list)
            .footer(dialogs::footer("OK"))
            .on_ok(move |_, _, cx| {
                let ids: Vec<String> = view.read(cx).devices.iter().filter(|(_, on)| *on).map(|(d, _)| d.id.clone()).collect();
                if ids.is_empty() {
                    return false;
                }
                service.update(cx, |s, cx| s.pick_devices(ids, cx));
                true
            })
    });
}
