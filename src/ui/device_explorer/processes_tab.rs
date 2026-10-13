//! The Processes tab: debuggable processes over JDWP, Kill / Force stop /
//! Clear app data / Uninstall, its menu and its table.

use super::*;

impl DeviceExplorer {
    pub(super) fn poll_processes(&mut self, cx: &mut Context<Self>) {
        if self.process_poll.is_some() || !self.shown || self.tab != Tab::Processes {
            return;
        }
        self.process_poll = Some(cx.spawn(async move |this, cx| {
            loop {
                let job = this
                    .update(cx, |this, cx| {
                        if !this.shown || this.tab != Tab::Processes {
                            return None;
                        }
                        let processes = this.processes.as_mut()?;
                        Some((this.device_manager.read(cx).adb()?, processes.serial.clone(), std::mem::take(&mut processes.known)))
                    })
                    .ok()
                    .flatten();
                let Some((adb, serial, mut known)) = job else { break };
                let (result, known) = cx
                    .background_spawn(async move {
                        let result = jdwp::list(&adb, &serial, &mut known);
                        (result, known)
                    })
                    .await;
                let alive = this
                    .update(cx, |this, cx| {
                        if let Some(processes) = this.processes.as_mut() {
                            processes.known = known;
                            processes.loaded = true;
                            match result {
                                Ok(list) => {
                                    processes.error = None;
                                    processes.selected.retain(|pid| list.iter().any(|p| p.pid == *pid));
                                    if processes.list != list {
                                        processes.list = list;
                                    }
                                }
                                Err(e) => processes.error = Some(e.to_string()),
                            }
                            cx.notify();
                        }
                    })
                    .is_ok();
                if !alive {
                    break;
                }
                cx.background_executor().timer(PROCESS_POLL).await;
            }
            this.update(cx, |this, _| this.process_poll = None).ok();
        }));
    }

    pub(super) fn refresh_processes(&mut self, cx: &mut Context<Self>) {
        if let Some(processes) = self.processes.as_mut() {
            processes.known.clear();
        }
        self.process_poll = None;
        self.poll_processes(cx);
    }

    pub(super) fn visible_processes(&self) -> Vec<Process> {
        let Some(processes) = self.processes.as_ref() else { return Vec::new() };
        let filtering = self.filtering();
        processes.list.iter().filter(|p| !filtering || p.package_name().is_some_and(|n| self.app_ids.iter().any(|id| id == n))).cloned().collect()
    }

    pub(super) fn select_process(&mut self, pid: u32, modifiers: Modifiers, cx: &mut Context<Self>) {
        let visible = self.visible_processes();
        let Some(processes) = self.processes.as_mut() else { return };
        if modifiers.shift {
            let anchor = processes.anchor.unwrap_or(pid);
            let (Some(a), Some(b)) = (visible.iter().position(|p| p.pid == anchor), visible.iter().position(|p| p.pid == pid)) else { return };
            processes.selected = visible[a.min(b)..=a.max(b)].iter().map(|p| p.pid).collect();
        } else if modifiers.secondary() {
            if let Some(ix) = processes.selected.iter().position(|p| *p == pid) {
                processes.selected.remove(ix);
            } else {
                processes.selected.push(pid);
            }
            processes.anchor = Some(pid);
        } else {
            processes.selected = vec![pid];
            processes.anchor = Some(pid);
        }
        cx.notify();
    }

    pub(super) fn selected_processes(&self) -> Vec<Process> {
        self.processes.as_ref().map(Processes::selected_processes).unwrap_or_default()
    }

    /// Runs an action on each selected process in the background, then refreshes.
    pub(super) fn process_action(&mut self, what: &'static str, run: fn(&crate::android::adb::Adb, &str, &Process) -> Result<(), String>, cx: &mut Context<Self>) {
        let selected = self.selected_processes();
        let Some(adb) = self.adb(cx) else { return };
        let Some(serial) = self.processes.as_ref().map(|p| p.serial.clone()) else { return };
        if selected.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx| {
            let errors: Vec<String> = cx.background_spawn(async move { selected.iter().filter_map(|p| run(&adb, &serial, p).err()).collect() }).await;
            this.update(cx, |this, cx| {
                for error in errors {
                    this.notify(&format!("Unable to {what}"), error, true, cx);
                }
                this.refresh_processes(cx);
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn kill_processes(&mut self, cx: &mut Context<Self>) {
        self.process_action("kill process", |adb, serial, p| jdwp::kill(adb, serial, p.pid).map_err(|e| e.to_string()), cx);
    }

    pub(super) fn force_stop(&mut self, cx: &mut Context<Self>) {
        self.process_action(
            "force stop",
            |adb, serial, p| {
                let package = p.package_name().ok_or("Couldn't find package name for process.")?;
                adb.shell(serial, &format!("am force-stop {package}")).map(|_| ()).map_err(|e| e.to_string())
            },
            cx,
        );
    }

    pub(super) fn clear_app_data(&mut self, cx: &mut Context<Self>) {
        self.process_action(
            "clear app data",
            |adb, serial, p| {
                let package = p.package_name().ok_or("Couldn't find package name for process.")?;
                let out = adb.shell(serial, &format!("pm clear {package}")).map_err(|e| e.to_string())?;
                if out.contains("Success") { Ok(()) } else { Err("Failed to clear app data.".into()) }
            },
            cx,
        );
    }

    pub(super) fn uninstall_app(&mut self, cx: &mut Context<Self>) {
        self.process_action(
            "uninstall app",
            |adb, serial, p| {
                let package = p.package_name().ok_or("Couldn't find package name for process.")?;
                let out = adb.run(&["-s", serial, "uninstall", package], Duration::from_secs(60)).map_err(|e| e.to_string())?;
                if out.stdout.contains("Success") { Ok(()) } else { Err("Failed to uninstall app.".into()) }
            },
            cx,
        );
    }

    pub(super) fn process_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &event.keystroke;
        let m = k.modifiers;
        let visible = self.visible_processes();
        let handled = match k.key.as_str() {
            "up" | "down" if !m.alt && !m.control && !m.platform => {
                let current = self.processes.as_ref().and_then(|p| p.selected.last().copied()).and_then(|pid| visible.iter().position(|p| p.pid == pid));
                let next = match current {
                    Some(ix) if k.key == "down" => (ix + 1 < visible.len()).then_some(ix + 1),
                    Some(ix) => ix.checked_sub(1),
                    None => (!visible.is_empty()).then_some(0),
                };
                if let Some(ix) = next {
                    self.select_process(visible[ix].pid, Modifiers { shift: m.shift, ..Default::default() }, cx);
                    self.process_scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
                }
                true
            }
            "c" if (m.control || m.platform) && !m.shift => {
                // Copy: "name: pid" per selected process, as Android Studio.
                let text: String = self.selected_processes().iter().map(|p| format!("{}: {}\n", p.display_name(), p.pid)).collect();
                if !text.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
                true
            }
            _ => false,
        };
        if handled {
            cx.stop_propagation();
        }
    }

    pub(super) fn process_menu(this: Entity<Self>, menu: PopupMenu, cx: &mut Context<PopupMenu>) -> PopupMenu {
        let selected = this.read(cx).selected_processes();
        if selected.is_empty() {
            return menu;
        }
        let many = selected.len() > 1;
        let known = selected.iter().any(|p| !p.pid_only());
        let packages = selected.iter().all(|p| p.package_name().is_some());
        let act = |f: fn(&mut Self, &mut Context<Self>)| {
            let this = this.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| this.update(cx, |this, cx| f(this, cx))
        };
        let mut menu = menu;
        if known {
            menu = menu
                .item(PopupMenuItem::new(if many { "Kill processes" } else { "Kill process" }).icon(Icon::new(icons::KILL_PROCESS)).on_click(act(Self::kill_processes)))
                .item(PopupMenuItem::new(if many { "Force stop processes" } else { "Force stop process" }).icon(Icon::new(icons::STOP)).on_click(act(Self::force_stop)));
        }
        if packages {
            menu = menu
                .item(PopupMenuItem::new(if many { "Clear apps data" } else { "Clear app data" }).icon(Icon::new(icons::CLEAR)).on_click(act(Self::clear_app_data)))
                .item(PopupMenuItem::new(if many { "Uninstall apps" } else { "Uninstall app" }).icon(Icon::new(icons::DELETE)).on_click(act(Self::uninstall_app)));
        }
        menu
    }

    pub(super) fn render_processes_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let selected = self.selected_processes();
        let many = selected.len() > 1;
        let known = selected.iter().any(|p| !p.pid_only());
        let packages = !selected.is_empty() && selected.iter().all(|p| p.package_name().is_some());
        h_flex()
            .h(px(crate::ui::common::toolbar_height()))
            .px_1()
            .gap_0p5()
            .flex_shrink_0()
            .border_b_1()
            .border_color(palette.border)
            .child(
                tool_button("de-kill", icons::KILL_PROCESS, if many { "Kill processes\nTerminates the target VM" } else { "Kill process\nTerminates the target VM" })
                    .disabled(!known)
                    .on_click(cx.listener(|this, _, _, cx| this.kill_processes(cx))),
            )
            .child(
                tool_button(
                    "de-force-stop",
                    icons::STOP,
                    if many { "Force stop processes\nExecutes command: am force-stop" } else { "Force stop process\nExecutes command: am force-stop" },
                )
                .disabled(!known)
                .on_click(cx.listener(|this, _, _, cx| this.force_stop(cx))),
            )
            .child(
                tool_button("de-clear", icons::CLEAR, if many { "Clear apps data\nClears application data" } else { "Clear app data\nClears application data" })
                    .disabled(!packages)
                    .on_click(cx.listener(|this, _, _, cx| this.clear_app_data(cx))),
            )
            .child(
                tool_button("de-uninstall", icons::DELETE, if many { "Uninstall apps\nUninstalls application" } else { "Uninstall app\nUninstalls application" })
                    .disabled(!packages)
                    .on_click(cx.listener(|this, _, _, cx| this.uninstall_app(cx))),
            )
            .child(tool_button("de-refresh", icons::STUDIO_REFRESH, "Refresh (Ctrl+F5)").on_click(cx.listener(|this, _, _, cx| this.refresh_processes(cx))))
            .child(self.filter_button("de-process-filter", cx))
    }

    pub(super) fn render_process_rows(&mut self, range: Range<usize>, _: &mut Window, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let palette = cx.palette().clone();
        let entity = cx.entity();
        let visible = self.visible_processes();
        let selected: Vec<u32> = self.processes.as_ref().map(|p| p.selected.clone()).unwrap_or_default();
        range
            .filter_map(|ix| {
                let process = visible.get(ix)?.clone();
                let is_selected = selected.contains(&process.pid);
                let pid = process.pid;
                let dash = |v: Option<String>| if process.pid_only() { "-".to_owned() } else { v.unwrap_or_else(|| "-".into()) };
                let debugger = if process.pid_only() { "-" } else if process.waiting { "Waiting" } else { "No" };
                let user = if process.pid_only() { "-".to_owned() } else { process.user_id.map(|u| u.to_string()).unwrap_or_else(|| "n/a".into()) };
                let cell = |text: String| div().px_1().whitespace_nowrap().overflow_hidden().text_ellipsis().child(text);
                let menu_entity = entity.clone();
                Some(
                    h_flex()
                        .id(("de-process", pid as usize))
                        .h(px(ROW_H))
                        .w_full()
                        .text_sm()
                        .when(is_selected, |el| el.bg(palette.selection))
                        .when(!is_selected, |el| el.hover(|s| s.bg(palette.hover)))
                        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                            window.focus(&this.focus, cx);
                            this.select_process(pid, e.modifiers(), cx)
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, _, window, cx| {
                                window.focus(&this.focus, cx);
                                if !this.processes.as_ref().is_some_and(|p| p.selected.contains(&pid)) {
                                    this.select_process(pid, Modifiers::default(), cx);
                                }
                            }),
                        )
                        .context_menu(move |menu, _, cx| Self::process_menu(menu_entity.clone(), menu, cx))
                        .child(cell(process.display_name()).flex_1().min_w(px(NAME_MIN)))
                        .child(cell(pid.to_string()).w(px(PID_W)).flex_shrink_0())
                        .child(cell(dash(process.abi.clone())).w(px(ABI_W)).flex_shrink_0())
                        .child(cell(dash(process.vm.clone())).w(px(VM_W)).flex_shrink_0())
                        .child(cell(user).w(px(USER_W)).flex_shrink_0())
                        .child(cell(debugger.into()).w(px(DEBUGGER_W)).flex_shrink_0())
                        .into_any_element(),
                )
            })
            .collect()
    }

    pub(super) fn render_processes(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let palette = cx.palette().clone();
        let column = |label: &'static str| div().px_1().border_l_1().border_color(palette.border).whitespace_nowrap().overflow_hidden().child(label);
        let header = h_flex()
            .h(px(24.))
            .flex_shrink_0()
            .text_sm()
            .text_color(palette.text_secondary)
            .border_b_1()
            .border_color(palette.border)
            .child(div().flex_1().min_w(px(NAME_MIN)).px_1().child("Process Name"))
            .child(column("PID").w(px(PID_W)).flex_shrink_0())
            .child(column("ABI").w(px(ABI_W)).flex_shrink_0())
            .child(column("VM").w(px(VM_W)).flex_shrink_0())
            .child(column("User ID").w(px(USER_W)).flex_shrink_0())
            .child(column("Debugger").w(px(DEBUGGER_W)).flex_shrink_0());
        let processes = self.processes.as_ref();
        let count = self.visible_processes().len();
        let body: gpui_kit::AnyElement = match processes {
            Some(p) if !p.loaded => message_layer("Loading...".into(), palette.text_secondary).into_any_element(),
            Some(Processes { error: Some(error), .. }) if count == 0 => message_layer(error.clone(), palette.status_deleted).into_any_element(),
            _ if count == 0 => message_layer("No debuggable process on device".into(), palette.text_secondary).into_any_element(),
            _ => uniform_list("de-processes", count, cx.processor(Self::render_process_rows)).track_scroll(&self.process_scroll).flex_1().min_h_0().into_any_element(),
        };
        v_flex()
            .id("de-processes-tab")
            .flex_1()
            .min_h_0()
            .child(self.render_processes_toolbar(cx))
            .child(table("de-processes-scroll", NAME_MIN + PID_W + ABI_W + VM_W + USER_W + DEBUGGER_W, header, body))
            .into_any_element()
    }
}
