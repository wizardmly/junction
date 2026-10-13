//! Device Manager actions: start and stop emulators, Cold Boot, Wipe Data,
//! Delete, View Details, Wi-Fi pairing and connecting, the ⋮ menu.

use super::*;

impl DeviceManager {
    /// ▶ Start (Quick Boot), or Cold Boot.
    pub(super) fn start(&mut self, id: &str, cold_boot: bool, cx: &mut Context<Self>) {
        let Some(device) = self.device(id).cloned() else { return };
        let Some(avd) = device.avd.clone() else { return };
        self.transitions.insert(device.id.clone(), (Transition::Starting, Instant::now()));
        cx.notify();
        let tools = self.tools.clone();
        let id = device.id.clone();
        self._tasks.push(cx.spawn(async move |this, cx| {
            let launched = cx.background_spawn({
                let name = avd.name.clone();
                async move { android::launch(&tools, &name, cold_boot) }
            });
            let (child, log) = match launched.await {
                Ok(started) => started,
                Err(e) => {
                    this.update(cx, |this, cx| {
                        this.transitions.remove(&id);
                        this.notify_error(format!("Failed to start {}", device.name), e.to_string(), cx);
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };
            this.update(cx, |this, cx| this.refresh(cx)).ok();
            // The emulator runs until closed; a thread of its own waits for it.
            let (tx, rx) = futures::channel::oneshot::channel();
            std::thread::spawn(move || {
                let mut child = child;
                let _ = tx.send(child.wait().map(|s| s.success()).unwrap_or(false));
            });
            let success = rx.await.unwrap_or(true);
            this.update(cx, |this, cx| {
                let stopping = this.transitions.get(&id).is_some_and(|(t, _)| *t == Transition::Stopping);
                this.transitions.remove(&id);
                if !success && !stopping {
                    let reason = android::launch_error(&log);
                    this.notify_error(format!("Failed to start {}", device.name), if reason.is_empty() { "The emulator exited".to_owned() } else { reason }, cx);
                }
                this.refresh(cx);
            })
            .ok();
        }));
    }

    /// ■ Stop: the emulator console's `kill`.
    pub(super) fn stop(&mut self, id: &str, cx: &mut Context<Self>) -> Option<Task<()>> {
        let device = self.device(id)?.clone();
        let (adb, serial) = (self.tools.adb.clone()?, device.serial.clone()?);
        self.transitions.insert(device.id.clone(), (Transition::Stopping, Instant::now()));
        cx.notify();
        let task = cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { adb.emu_kill(&serial) }).await;
            this.update(cx, |this, cx| {
                if let Err(e) = result {
                    this.transitions.remove(&device.id);
                    this.notify_error(format!("Failed to stop {}", device.name), e.to_string(), cx);
                }
                this.refresh(cx);
            })
            .ok();
        });
        Some(task)
    }

    /// Wipe Data and Delete stop a running emulator first and wait for it
    /// to go before touching its files.
    pub(super) fn stop_then(&mut self, id: &str, title: &'static str, action: fn(&avd::Avd) -> std::io::Result<()>, cx: &mut Context<Self>) {
        let Some(device) = self.device(id).cloned() else { return };
        let Some(avd) = device.avd.clone() else { return };
        let running = device.serial.is_some();
        if running {
            if let Some(task) = self.stop(id, cx) {
                task.detach();
            }
        }
        let id = id.to_owned();
        self._tasks.push(cx.spawn(async move |this, cx| {
            let start = Instant::now();
            while running && start.elapsed() < Duration::from_secs(60) {
                cx.background_executor().timer(Duration::from_millis(500)).await;
                let gone = this.read_with(cx, |this, _| this.device(&id).is_none_or(|d| d.serial.is_none())).unwrap_or(true);
                if gone {
                    // The emulator lets go of its files a moment after adb loses it.
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    break;
                }
            }
            let result = cx.background_spawn(async move { action(&avd) }).await;
            this.update(cx, |this, cx| {
                if let Err(e) = result {
                    this.notify_error(format!("{title} failed for {}", device.name), e.to_string(), cx);
                }
                this.refresh(cx);
            })
            .ok();
        }));
    }

    pub(super) fn confirm_wipe(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(device) = self.device(id) else { return };
        let suffix = if device.serial.is_some() { " This will stop the device." } else { "" };
        let message = format!("Do you really want to wipe user files from {}?{suffix}", device.name);
        let (entity, id) = (cx.entity(), id.to_owned());
        crate::ui::dialogs::confirm(
            "Confirm Data Wipe",
            message,
            "Yes",
            move |cx| entity.update(cx, |this, cx| this.stop_then(&id, "Wipe Data", avd::wipe_data, cx)),
            window,
            cx,
        );
    }

    pub(super) fn confirm_delete(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(device) = self.device(id) else { return };
        let suffix = if device.serial.is_some() { " This will stop the device." } else { "" };
        let message = format!("Do you really want to delete {}?{suffix}", device.name);
        let (entity, id) = (cx.entity(), id.to_owned());
        crate::ui::dialogs::confirm(
            "Confirm Deletion",
            message,
            "Yes",
            move |cx| entity.update(cx, |this, cx| this.stop_then(&id, "Delete", avd::delete, cx)),
            window,
            cx,
        );
    }

    pub(super) fn view_details(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(device) = self.device(id).cloned() else { return };
        let adb = self.tools.adb.clone();
        let load = cx.spawn(async move |this, cx| {
            let summary = cx.background_spawn(async move { android::summary(&device, adb.as_ref()) }).await;
            this.update(cx, |this, cx| {
                if let Some(details) = &mut this.details {
                    details.summary = Some(summary);
                }
                cx.notify();
            })
            .ok();
        });
        self.details = Some(Details { id: id.to_owned(), summary: None, _load: load });
        cx.notify();
    }

    pub(super) fn pair_over_wifi(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::ui::wifi_pairing::open(cx.entity().downgrade(), self.tools.adb.clone(), window, cx);
    }

    /// The Wi-Fi button of a device available for Wi-Fi pairing: connects
    /// it when this computer paired with it before, else opens the pairing
    /// dialog.
    pub(super) fn connect_wifi(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(device) = self.device(id).cloned() else { return };
        let (Some(address), Some(adb)) = (device.wifi.clone(), self.tools.adb.clone()) else { return };
        self.transitions.insert(device.id.clone(), (Transition::Connecting, Instant::now()));
        cx.notify();
        let window_handle = window.window_handle();
        self._tasks.push(cx.spawn(async move |this, cx| {
            let connected = cx
                .background_spawn(async move { android::wifi::connect(&adb, &address) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.transitions.remove(&device.id);
                if connected {
                    this.paired(device.name.clone(), cx);
                }
                cx.notify();
            });
            if !connected {
                // Not paired with this computer yet.
                let _ = window_handle.update(cx, |_, window, cx| this.update(cx, |this, cx| this.pair_over_wifi(window, cx)));
            }
        }));
    }

    /// The ⋮ menu (also the row's context menu).
    pub(super) fn device_menu(entity: Entity<Self>, device: &Device, transitioning: bool, menu: PopupMenu) -> PopupMenu {
        let id = device.id.clone();
        let action = |f: fn(&mut Self, &str, &mut Window, &mut Context<Self>)| {
            let (entity, id) = (entity.clone(), id.clone());
            move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| entity.update(cx, |this, cx| f(this, &id, window, cx))
        };
        let mut menu = menu;
        if device.wifi.is_some() {
            menu = menu
                .item(PopupMenuItem::new("Pair device over Wi-Fi").icon(Icon::new(icons::PAIR_OVER_WIFI)).disabled(transitioning).on_click(action(Self::connect_wifi)))
                .separator();
        }
        // Open in Device Explorer: any device adb can reach.
        if let Some(serial) = device.serial.clone().filter(|_| device.online) {
            let entity = entity.clone();
            menu = menu
                .item(PopupMenuItem::new("Open in Device Explorer").icon(Icon::new(icons::TW_DEVICE_EXPLORER)).on_click(move |_, _, cx| {
                    entity.update(cx, |_, cx| cx.emit(DeviceManagerEvent::Explore(serial.clone())))
                }))
                .separator();
        }
        if let Some(avd) = &device.avd {
            let offline = device.serial.is_none() && !transitioning;
            let dir = avd.dir.clone();
            menu = menu
                .item(PopupMenuItem::new("Cold Boot").disabled(!offline).on_click(action(|this, id, _, cx| this.start(id, true, cx))))
                .separator()
                .item(PopupMenuItem::new("Wipe Data").icon(Icon::new(icons::CLEAR)).disabled(transitioning).on_click(action(Self::confirm_wipe)))
                .item(PopupMenuItem::new("Delete").icon(Icon::new(icons::DELETE)).disabled(transitioning).on_click(action(Self::confirm_delete)))
                .separator()
                .item(PopupMenuItem::new("View Details").on_click(action(|this, id, _, cx| this.view_details(id, cx))))
                .item(PopupMenuItem::new("Show on Disk").icon(Icon::new(icons::SHOW)).on_click(move |_, _, cx| cx.reveal_path(&dir)));
        } else {
            menu = menu.item(PopupMenuItem::new("View Details").on_click(action(|this, id, _, cx| this.view_details(id, cx))));
        }
        menu
    }
}
