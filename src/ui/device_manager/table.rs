//! The Device Manager table: sorting, grouping, the toolbar, rows, the
//! empty state, the adb banner and the details panel.

use super::*;

impl DeviceManager {
    pub(super) fn group_key(&self, device: &Device) -> &'static str {
        match self.grouping {
            Grouping::None => "",
            Grouping::FormFactor => device.form.label(),
            Grouping::Status => {
                if device.online { "Online" } else { "Offline" }
            }
            Grouping::Type => device.kind.label(),
        }
    }

    pub(super) fn sorted(&self) -> Vec<&Device> {
        let mut devices: Vec<&Device> = self.devices.iter().collect();
        let (column, ascending) = self.sort;
        devices.sort_by(|a, b| {
            let order = match column {
                SortColumn::Status => b.online.cmp(&a.online),
                SortColumn::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortColumn::Api => a.api.cmp(&b.api).then_with(|| a.codename.cmp(&b.codename)),
                SortColumn::Type => a.kind.label().cmp(b.kind.label()),
            };
            let order = if ascending { order } else { order.reverse() };
            order.then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        devices
    }

    pub(super) fn toggle_sort(&mut self, column: SortColumn, cx: &mut Context<Self>) {
        self.sort = if self.sort.0 == column { (column, !self.sort.1) } else { (column, true) };
        cx.notify();
    }

    pub(super) fn grouping_menu(entity: Entity<Self>, current: Grouping, menu: PopupMenu) -> PopupMenu {
        let mut menu = menu.label("Group By");
        for (label, grouping) in [("None", Grouping::None), ("Form Factor", Grouping::FormFactor), ("Status", Grouping::Status), ("Type", Grouping::Type)] {
            let entity = entity.clone();
            menu = menu.item(PopupMenuItem::new(label).checked(current == grouping).on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| {
                    this.grouping = grouping;
                    this.collapsed.clear();
                    cx.notify();
                })
            }));
        }
        menu
    }

    pub(super) fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let entity = cx.entity();
        let grouping = self.grouping;
        h_flex()
            .h(px(crate::ui::common::header_height()))
            .px_2()
            .gap_1()
            .flex_shrink_0()
            .border_b_1()
            .border_color(palette.border)
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Device Manager"))
            .child(div().flex_1())
            .child(tool_button("dm-options", icons::MORE_VERTICAL, "Options").dropdown_menu(move |menu, _, _| {
                let refresh = entity.clone();
                Self::grouping_menu(entity.clone(), grouping, menu)
                    .separator()
                    .item(PopupMenuItem::new("Refresh").on_click(move |_, _, cx| refresh.update(cx, |this, cx| this.refresh(cx))))
            }))
            .child(tool_button("dm-hide", icons::HIDE, "Hide").on_click(cx.listener(|_, _, _, cx| cx.emit(DeviceManagerEvent::Hide))))
    }

    pub(super) fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let (group_entity, add_entity) = (cx.entity(), cx.entity());
        let grouping = self.grouping;
        h_flex()
            .h(px(crate::ui::common::toolbar_height()))
            .px_1()
            .gap_0p5()
            .flex_shrink_0()
            .border_b_1()
            .border_color(palette.border)
            .child(tool_button("dm-group", icons::GROUP, "Group").dropdown_menu(move |menu, _, _| Self::grouping_menu(group_entity.clone(), grouping, menu)))
            .child(div().w(px(1.)).h(px(16.)).mx_1().bg(palette.border))
            .child(tool_button("dm-add", icons::ADD, "Add Device").dropdown_menu(move |menu, _, _| {
                let entity = add_entity.clone();
                menu.item(PopupMenuItem::new("Pair Devices Using Wi-Fi").icon(Icon::new(icons::PAIR_OVER_WIFI)).on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| this.pair_over_wifi(window, cx))
                }))
            }))
            .child(tool_button("dm-pair", icons::PAIR_OVER_WIFI, "Pair Devices Using Wi-Fi").on_click(cx.listener(|this, _, window, cx| this.pair_over_wifi(window, cx))))
    }

    pub(super) fn render_column_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let (sorted_by, ascending) = self.sort;
        let column = |id: &'static str, label: &'static str, sort: SortColumn, cx: &mut Context<Self>| {
            h_flex()
                .id(id)
                .h_full()
                .px_1()
                .gap_1()
                .border_r_1()
                .border_color(palette.border)
                .cursor_pointer()
                .hover(|el| el.bg(palette.hover))
                .when(sorted_by == sort, |el| {
                    el.child(Icon::new(if ascending { icons::CHEVRON_UP } else { icons::CHEVRON_DOWN }).xsmall().text_color(palette.text_secondary))
                })
                .child(div().whitespace_nowrap().overflow_hidden().child(label))
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_sort(sort, cx)))
        };
        h_flex()
            .h(px(26.))
            .flex_shrink_0()
            .text_sm()
            .border_b_1()
            .border_color(palette.border)
            .child(column("dm-col-status", "", SortColumn::Status, cx).w(px(STATUS_W)).flex_shrink_0())
            .child(column("dm-col-name", "Name", SortColumn::Name, cx).flex_1().min_w_0())
            .child(column("dm-col-api", "API", SortColumn::Api, cx).w(px(API_W)).flex_shrink_0())
            .child(column("dm-col-type", "Type", SortColumn::Type, cx).w(px(TYPE_W)).flex_shrink_0())
            .child(div().w(px(ACTIONS_W)).flex_shrink_0())
    }

    pub(super) fn render_row(&self, device: &Device, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let transition = self.transitions.get(&device.id).map(|(t, _)| *t);
        let busy = transition.is_some() || device.booting;
        let selected = self.selected.as_deref() == Some(device.id.as_str());
        let line2_color = if selected { palette.text } else if palette.dark { rgb(0x6f737a).into() } else { rgb(0x818594).into() };
        let line2 = transition
            .map(|t| t.label().to_owned())
            .or_else(|| device.booting.then(|| "Booting".to_owned()))
            .or_else(|| device.error.clone())
            .unwrap_or_else(|| device.version_label());
        let status: gpui_kit::AnyElement = if busy {
            Spinner::new().icon(Icon::new(IconName::LoaderCircle)).small().color(palette.text_secondary).into_any_element()
        } else if device.error.is_some() {
            Icon::new(icons::STATUS_WARNING).small().into_any_element()
        } else if device.online {
            Icon::new(icons::AVD_STATUS_ONLINE).small().into_any_element()
        } else {
            div().into_any_element()
        };
        let icon = match (device.kind, device.form) {
            (Kind::Virtual, android::FormFactor::Wear) => icons::VIRTUAL_DEVICE_WEAR,
            (Kind::Virtual, android::FormFactor::Tv) => icons::VIRTUAL_DEVICE_TV,
            (Kind::Virtual, android::FormFactor::Automotive) => icons::VIRTUAL_DEVICE_CAR,
            (Kind::Virtual, android::FormFactor::Desktop) => icons::VIRTUAL_DEVICE_DESKTOP,
            (Kind::Virtual, _) => icons::VIRTUAL_DEVICE_PHONE,
            (Kind::Physical, android::FormFactor::Wear) => icons::PHYSICAL_DEVICE_WEAR,
            (Kind::Physical, android::FormFactor::Tv) => icons::PHYSICAL_DEVICE_TV,
            (Kind::Physical, android::FormFactor::Automotive) => icons::PHYSICAL_DEVICE_CAR,
            (Kind::Physical, _) => icons::PHYSICAL_DEVICE_PHONE,
        };
        let id = device.id.clone();
        let entity = cx.entity();
        let menu_device = device.clone();
        let overflow_device = device.clone();
        let overflow_entity = entity.clone();
        let start_stop = device.avd.is_some().then(|| {
            let running = device.serial.is_some() && transition != Some(Transition::Starting);
            let (icon, tooltip) = if running || transition == Some(Transition::Starting) { (icons::AVD_STOP, "Stop") } else { (icons::AVD_RUN, "Start") };
            let id = id.clone();
            Button::new(SharedString::from(format!("dm-run-{id}")))
                .ghost()
                .xsmall()
                .icon(Icon::new(icon))
                .tooltip(tooltip)
                .disabled(transition.is_some())
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    if running {
                        if let Some(task) = this.stop(&id, cx) {
                            task.detach();
                        }
                    } else {
                        this.start(&id, false, cx);
                    }
                }))
        });
        // Physical devices: Start / Stop Mirroring, as Android Studio's row.
        let mirroring = (device.kind == Kind::Physical && device.online).then(|| {
            let serial = device.serial.clone().unwrap_or_default();
            let active = self.mirrored.contains(&serial);
            let title = crate::ui::running_devices::title_for(device);
            Button::new(SharedString::from(format!("dm-mirror-{id}")))
                .ghost()
                .xsmall()
                .icon(Icon::new(if active { icons::AVD_STOP_MIRROR } else { icons::AVD_START_MIRROR }))
                .tooltip(if active { "Stop Mirroring" } else { "Start Mirroring" })
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.stop_propagation();
                    cx.emit(DeviceManagerEvent::Mirror { serial: serial.clone(), title: title.clone(), start: !active });
                }))
        });
        // Available for Wi-Fi pairing: AS's Wi-Fi button.
        let wifi = device.wifi.is_some().then(|| {
            let id = id.clone();
            Button::new(SharedString::from(format!("dm-wifi-{id}")))
                .ghost()
                .xsmall()
                .icon(Icon::new(icons::PAIR_OVER_WIFI))
                .tooltip("Pair device over Wi-Fi")
                .disabled(transition.is_some())
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.connect_wifi(&id, window, cx);
                }))
        });
        h_flex()
            .id(SharedString::from(format!("dm-row-{id}")))
            .h(px(if crate::ui::common::compact() { 34. } else { 40. }))
            .flex_shrink_0()
            .text_sm()
            .when(selected, |el| el.bg(palette.selection))
            .when(!selected, |el| el.hover(|s| s.bg(palette.hover)))
            .on_click(cx.listener({
                let id = id.clone();
                move |this, _, _, cx| {
                    this.selected = Some(id.clone());
                    cx.notify();
                }
            }))
            .on_mouse_down(
                gpui_kit::MouseButton::Right,
                cx.listener({
                    let id = id.clone();
                    move |this, _, _, cx| {
                        this.selected = Some(id.clone());
                        cx.notify();
                    }
                }),
            )
            .context_menu(move |menu, _, _| Self::device_menu(entity.clone(), &menu_device, busy, menu))
            .child(h_flex().w(px(STATUS_W)).flex_shrink_0().justify_center().child(status))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .px_1()
                    .gap_2()
                    .child(Icon::new(icon).small())
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(div().whitespace_nowrap().overflow_hidden().text_ellipsis().child(device.name.clone()))
                            .child(div().text_xs().text_color(line2_color).whitespace_nowrap().overflow_hidden().text_ellipsis().child(line2)),
                    ),
            )
            .child(div().w(px(API_W)).flex_shrink_0().px_1().child(device.api_label()))
            .child(div().w(px(TYPE_W)).flex_shrink_0().px_1().child(device.kind.label()))
            .child(
                h_flex()
                    .w(px(ACTIONS_W))
                    .flex_shrink_0()
                    .justify_end()
                    .pr_1()
                    .children(start_stop)
                    .children(mirroring)
                    .children(wifi)
                    .child(
                        Button::new(SharedString::from(format!("dm-more-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(icons::OVERFLOW))
                            .tooltip("More Actions")
                            .dropdown_menu(move |menu, _, _| Self::device_menu(overflow_entity.clone(), &overflow_device, busy, menu)),
                    ),
            )
    }

    pub(super) fn render_rows(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let palette = cx.palette().clone();
        let devices = self.sorted();
        let mut rows = v_flex();
        if self.grouping == Grouping::None {
            for device in devices {
                rows = rows.child(self.render_row(device, cx));
            }
            return rows.into_any_element();
        }
        let mut keys: Vec<&'static str> = devices.iter().map(|d| self.group_key(d)).collect();
        keys.sort_unstable();
        keys.dedup();
        for key in keys {
            let collapsed = self.collapsed.contains(key);
            let members: Vec<&Device> = devices.iter().copied().filter(|d| self.group_key(d) == key).collect();
            rows = rows.child(
                h_flex()
                    .id(SharedString::from(format!("dm-group-{key}")))
                    .h(px(26.))
                    .px_1()
                    .gap_1()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .cursor_pointer()
                    .hover(|el| el.bg(palette.hover))
                    .child(Icon::new(if collapsed { icons::CHEVRON_RIGHT } else { icons::CHEVRON_DOWN }).xsmall().text_color(palette.text_secondary))
                    .child(format!("{key} ({})", members.len()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.collapsed.remove(key) {
                            this.collapsed.insert(key);
                        }
                        cx.notify();
                    })),
            );
            if !collapsed {
                for device in members {
                    rows = rows.child(self.render_row(device, cx));
                }
            }
        }
        rows.into_any_element()
    }

    pub(super) fn render_empty(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_2()
            .text_sm()
            .child(div().text_color(palette.text_secondary).child(if self.loaded { "No devices connected." } else { "Loading devices…" }))
            .when(self.loaded, |el| {
                el.child(
                    div()
                        .id("dm-add-new")
                        .text_color(palette.link)
                        .cursor_pointer()
                        .hover(|el| el.underline())
                        .child("Add a new device...")
                        .on_click(cx.listener(|this, _, window, cx| this.pair_over_wifi(window, cx))),
                )
            })
    }

    pub(super) fn render_banner(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let palette = cx.palette().clone();
        let message = if !self.loaded {
            return None;
        } else if self.tools.adb.is_none() {
            "Android SDK not found: set ANDROID_HOME to the SDK folder, or install the SDK Platform-Tools.".to_owned()
        } else {
            self.adb_error.clone()?
        };
        let (bg, border) = if palette.dark { (rgb(0x3d3223), rgb(0x5e4d33)) } else { (rgb(0xfff6de), rgb(0xfed277)) };
        Some(
            h_flex()
                .m_1()
                .p_2()
                .gap_2()
                .rounded(px(4.))
                .border_1()
                .border_color(border)
                .bg(bg)
                .text_sm()
                .child(Icon::new(icons::STATUS_WARNING).small())
                .child(div().flex_1().min_w_0().child(message)),
        )
    }

    pub(super) fn render_details(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let details = self.details.as_ref()?;
        let device = self.device(&details.id)?;
        let palette = cx.palette().clone();
        let mut text = format!("{}\n", device.name);
        let row = |label: &str, value: &str| {
            h_flex()
                .gap_2()
                .text_sm()
                .child(div().w(px(150.)).flex_shrink_0().text_color(palette.text_secondary).child(format!("{label}:")))
                .child(div().min_w_0().child(value.to_owned()))
        };
        let mut body = v_flex().p_2().gap_1();
        body = body.child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Summary"));
        match &details.summary {
            None => body = body.child(div().text_sm().text_color(palette.text_secondary).child("Loading…")),
            Some(summary) => {
                for (label, value) in summary {
                    text.push_str(&format!("{label}: {value}\n"));
                    body = body.child(row(label, value));
                }
            }
        }
        if let Some(avd) = &device.avd {
            body = body.child(div().pt_2().text_sm().font_weight(FontWeight::SEMIBOLD).child("Properties"));
            text.push_str("\nProperties\n");
            for (key, value) in &avd.config {
                text.push_str(&format!("{key}: {value}\n"));
                body = body.child(row(key, value));
            }
        }
        Some(
            // Half the window, as Android Studio's splitter opens it.
            v_flex()
                .flex_1()
                .min_h(px(120.))
                .border_t_1()
                .border_color(palette.border)
                .child(
                    h_flex()
                        .h(px(crate::ui::common::header_height()))
                        .px_2()
                        .gap_1()
                        .flex_shrink_0()
                        .child(div().flex_1().min_w_0().text_sm().font_weight(FontWeight::SEMIBOLD).whitespace_nowrap().text_ellipsis().child(device.name.clone()))
                        .child(tool_button("dm-details-copy", icons::COPY, "Copy properties to clipboard").on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
                        }))
                        .child(tool_button("dm-details-close", icons::CLOSE, "Close").on_click(cx.listener(|this, _, _, cx| {
                            this.details = None;
                            cx.notify();
                        }))),
                )
                .child(
                    div()
                        .h(px(26.))
                        .px_2()
                        .flex()
                        .items_center()
                        .text_sm()
                        .border_b_1()
                        .border_color(palette.border)
                        .child(div().h_full().flex().items_center().border_b_2().border_color(palette.accent).child("Device Info")),
                )
                .child(div().id("dm-details-body").flex_1().min_h_0().overflow_y_scrollbar().child(body)),
        )
    }
}
