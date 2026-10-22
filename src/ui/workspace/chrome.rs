//! Window chrome: title bar, welcome screen, editor groups, notifications
//! and status bar.

use super::*;
use crate::ui::as_icons as icons;

impl Workspace {
    /// One tab group: its tab bar and what its selected tab shows.
    pub(super) fn render_group(&self, focused: bool, has_repo: bool, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let group = self.active_group;
        let entity = cx.entity();
        v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .on_mouse_down(gpui_kit::MouseButton::Left, move |_, _, cx| entity.update(cx, |this, cx| this.focus_group(group, cx)))
            .children(self.render_tab_bar(focused, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    // A wide toolbar (the diff's) stays inside its group.
                    .overflow_hidden()
                    .when(focused && matches!(self.front, Front::Editor(_)), |el| el.key_context(TABS_CONTEXT))
                    .map(|el| {
                        if !has_repo {
                            return el.child(self.render_welcome(cx));
                        }
                        match self.front {
                            Front::Editor(ix) if ix < self.editors.len() => el.child(crate::ui::common::cached(&self.editors[ix].view)),
                            _ if group != 0 => el,
                            Front::DiffTab(ix) if ix < self.diff_tabs.len() => el.child(crate::ui::common::cached(&self.diff_tabs[ix].0)),
                            Front::Timeline if self.timeline.is_some() => el.child(crate::ui::common::cached(self.timeline.as_ref().unwrap())),
                            Front::Translations if self.translations.is_some() => el.child(crate::ui::common::cached(&self.translations.as_ref().unwrap().0)),
                            Front::Apk if self.apk.is_some() => el.child(crate::ui::common::cached(self.apk.as_ref().unwrap())),
                            Front::Merge if self.merge.is_some() => el.child(crate::ui::common::cached(&self.merge.as_ref().unwrap().0)),
                            _ => el.child(crate::ui::common::cached(&self.diff)),
                        }
                    }),
            )
            .into_any_element()
    }

    /// The editor area: one tab group, or two after Split Right / Down.
    pub(super) fn render_editor_groups(&mut self, has_repo: bool, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let Some(vertical) = self.split.as_ref().map(|s| s.vertical) else {
            return div().flex_1().min_w_0().min_h_0().flex().child(self.render_group(true, has_repo, cx)).into_any_element();
        };
        let focused = self.active_group;
        let mine = self.render_group(true, has_repo, cx);
        self.swap_split();
        let other = self.render_group(false, has_repo, cx);
        self.swap_split();
        let (first, second) = if focused == 0 { (mine, other) } else { (other, mine) };
        let palette = cx.palette().clone();
        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .when(vertical, |el| el.flex_col())
            .child(first)
            .child(if vertical { div().h(px(1.)).w_full().bg(palette.border) } else { div().w(px(1.)).h_full().bg(palette.border) })
            .child(second)
            .into_any_element()
    }

    /// The Welcome screen shown when no repository is open.
    pub(super) fn render_welcome(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let recent = crate::settings::recent_projects();
        let error = self.model.read(cx).error().map(str::to_owned);
        let action = |id: &'static str, icon: Icon, label: &'static str| {
            Button::new(id).small().icon(Icon::new(icon)).label(label)
        };
        let mut list = v_flex().gap_px();
        for (ix, path) in recent.iter().enumerate() {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let open = path.clone();
            let forget = path.clone();
            let model = self.model.clone();
            list = list.child(
                h_flex()
                    .id(("recent", ix))
                    .px_2()
                    .py_1()
                    .gap_2()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|s| s.bg(palette.hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let open = open.clone();
                        this.model.update(cx, |m, cx| m.open(open, cx))
                    }))
                    .context_menu(move |menu, _, _| {
                        let forget = forget.clone();
                        let model = model.clone();
                        menu.item(PopupMenuItem::new("Remove from Recent Projects").on_click(move |_, window, cx| {
                            crate::settings::forget_project(&forget);
                            model.update(cx, |_, cx| cx.notify());
                            window.refresh();
                        }))
                    })
                    .child(
                        div()
                            .size(px(28.))
                            .rounded(px(6.))
                            .bg(palette.graph[ix % palette.graph.len()])
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(gpui_kit::white())
                            .child(name.chars().take(2).collect::<String>().to_uppercase()),
                    )
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(name))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(palette.text_secondary)
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(path.display().to_string()),
                            ),
                    ),
            );
        }
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_4()
            .child(div().text_xl().font_weight(FontWeight::BOLD).child(format!("Welcome to {APP_NAME}")))
            .child(
                h_flex()
                    .gap_2()
                    .child(action("welcome-open", Icon::from(icons::FOLDER), "Open").on_click(cx.listener(
                        |this, _, window, cx| this.open_repository(window, cx),
                    )))
                    .child(action("welcome-clone", Icon::from(icons::VCS_UPDATE), "Get from VCS").on_click(cx.listener(
                        |this, _, window, cx| clone_dialog::clone(this.model.clone(), window, cx),
                    )))
                    .child(action("welcome-init", Icon::from(icons::ADD), "New Repository").on_click(cx.listener(
                        |this, _, _, cx| clone_dialog::init(this.model.clone(), cx),
                    ))),
            )
            .when_some(error, |el, error| {
                let problem = self.model.read(cx).open_problem().cloned();
                let (title, hint) = match &problem {
                    Some(OpenProblem::GitMissing) => (
                        "Git is not installed",
                        "Junction Studio runs the git command-line tool. Install Git for Windows (or point Settings › Git to an existing git.exe), then retry.",
                    ),
                    Some(OpenProblem::Unsafe(_)) => (
                        "The folder is owned by another user",
                        "Git only opens repositories you own unless you trust them. Trusting adds the folder to safe.directory in your global git config.",
                    ),
                    None => ("Cannot open the folder", ""),
                };
                let mut buttons = h_flex().gap_2();
                match problem {
                    Some(OpenProblem::GitMissing) => {
                        buttons = buttons
                            .child(Button::new("welcome-get-git").small().primary().label("Download Git").on_click(|_, _, cx| {
                                cx.open_url(if cfg!(windows) { "https://git-scm.com/download/win" } else { "https://git-scm.com/downloads" })
                            }))
                            .child(Button::new("welcome-git-settings").small().label("Set Path to Git…").on_click(cx.listener(|this, _, window, cx| crate::ui::settings_dialog::open(Some(this.model.clone()), window, cx))))
                            .child(Button::new("welcome-retry").small().label("Retry").on_click(cx.listener(|this, _, _, cx| this.model.update(cx, |m, cx| m.retry_open(cx)))));
                    }
                    Some(OpenProblem::Unsafe(root)) => {
                        buttons = buttons.child(Button::new("welcome-trust").small().primary().label("Trust Directory and Open").on_click(cx.listener(
                            move |this, _, window, cx| match crate::git::trust_directory(&root) {
                                Ok(()) => this.model.update(cx, |m, cx| m.retry_open(cx)),
                                Err(e) => window.push_notification(Notification::error(e.to_string()), cx),
                            },
                        )));
                    }
                    None => {}
                }
                el.child(
                    v_flex()
                        .max_w(px(560.))
                        .p_3()
                        .gap_2()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(palette.status_conflict)
                        .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).text_color(palette.status_conflict).child(title))
                        .child(div().text_xs().text_color(palette.text_secondary).child(error))
                        .when(!hint.is_empty(), |el| el.child(div().text_xs().child(hint)))
                        .child(buttons),
                )
            })
            .when(!recent.is_empty(), |el| {
                el.child(
                    v_flex()
                        .w(px(460.))
                        .gap_1()
                        .child(div().text_xs().text_color(palette.text_secondary).child("Recent Projects"))
                        .child(list),
                )
            })
    }

    pub(super) fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let model = self.model.read(cx);
        // The project keeps its name while another of its roots is active.
        let project = model
            .project_root()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .or_else(|| model.repository().map(|r| r.name()))
            .unwrap_or_else(|| "No Project".into());
        let branch = branches_popup::branch_widget_label(model);
        // Incoming / outgoing commits of the current branch, next to its name.
        let track = model
            .refs()
            .current_branch
            .as_ref()
            .and_then(|name| model.refs().find(&format!("refs/heads/{name}")))
            .map(|r| (r.behind, r.ahead))
            .filter(|&(behind, ahead)| behind > 0 || ahead > 0);
        let busy = model.busy().map(str::to_owned);
        let gradle = crate::ui::sync_service::SyncService::global(cx).read(cx).has_builds();
        let entity = cx.entity();
        let popup = self.branches_popup.clone();
        TitleBar::new().child(
            h_flex()
                .w_full()
                .pr_2()
                .gap_1()
                .child(
                    Button::new("main-menu")
                        .ghost()
                        .small()
                        .icon(Icon::new(IconName::Menu))
                        .tooltip("Main Menu")
                        .dropdown_menu({
                            let entity = entity.clone();
                            let focus = self.focus.clone();
                            move |menu, window, cx| super::menu::main_menu(menu, entity.clone(), focus.clone(), window, cx)
                        }),
                )
                .child(
                    Button::new("project-widget")
                        .ghost()
                        .small()
                        .child(
                            h_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .size(px(18.))
                                        .rounded(px(4.))
                                        .bg(palette.accent)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .text_xs()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(gpui_kit::white())
                                        .child(project.chars().take(2).collect::<String>().to_uppercase()),
                                )
                                .child(div().font_weight(FontWeight::SEMIBOLD).child(project))
                                .child(Icon::new(icons::CHEVRON_DOWN).xsmall()),
                        )
                        .dropdown_menu({
                            let entity = entity.clone();
                            move |menu, _, cx| {
                                let current = { let m = entity.read(cx).model.read(cx); m.project_root().map(std::path::Path::to_path_buf).or_else(|| m.repository().map(|r| r.root().to_path_buf())) };
                                let mut menu = menu.label("Recent Projects");
                                for path in crate::settings::recent_projects().into_iter().take(10) {
                                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                                    let entity = entity.clone();
                                    let is_current = current.as_deref() == Some(path.as_path());
                                    menu = menu.item(PopupMenuItem::new(name).checked(is_current).on_click(move |_, _, cx| {
                                        let path = path.clone();
                                        let model = entity.read(cx).model.clone();
                                        model.update(cx, |m, cx| m.open(path, cx));
                                    }));
                                }
                                let (open, clone, init) = (entity.clone(), entity.clone(), entity.clone());
                                menu.separator()
                                    .item(PopupMenuItem::new("Open…").on_click(move |_, window, cx| {
                                        open.update(cx, |this, cx| this.open_repository(window, cx))
                                    }))
                                    .item(PopupMenuItem::new("Get from Version Control…").on_click(move |_, window, cx| {
                                        clone_dialog::clone(clone.read(cx).model.clone(), window, cx)
                                    }))
                                    .item(PopupMenuItem::new("Create Git Repository…").on_click(move |_, _, cx| {
                                        clone_dialog::init(init.read(cx).model.clone(), cx)
                                    }))
                            }
                        }),
                )
                .child(
                    Popover::new("branches-popover")
                        .anchor(gpui_kit::Anchor::TopLeft)
                        .open(self.branches_open)
                        .on_open_change({
                            let entity = entity.clone();
                            move |open, window, cx| {
                                let open = *open;
                                entity.update(cx, |this, cx| {
                                    if open {
                                        this.open_branches(window, cx);
                                    } else {
                                        this.branches_open = false;
                                        cx.notify();
                                    }
                                })
                            }
                        })
                        .trigger(
                            Button::new("vcs-widget")
                                .ghost()
                                .small()
                                .icon(Icon::new(icons::BRANCH).small())
                                .label(branch)
                                .when_some(track, |el, (behind, ahead)| {
                                    el.child(
                                        h_flex()
                                            .gap_1()
                                            .text_xs()
                                            .when(behind > 0, |el| el.child(div().text_color(palette.link).child(format!("↓{behind}"))))
                                            .when(ahead > 0, |el| el.child(div().text_color(palette.status_added).child(format!("↑{ahead}")))),
                                    )
                                })
                                .child(Icon::new(icons::CHEVRON_DOWN).xsmall()),
                        )
                        .child(popup),
                )
                .child(div().flex_1())
                .when_some(busy, |el, busy| {
                    el.child(div().text_xs().text_color(palette.text_secondary).child(format!("{busy}…")))
                })
                // (Windows: the title bar is the window's caption, where a press
                // starts a window drag and the click never ends; the toolbar
                // occludes it so its buttons get their clicks.)
                .child(h_flex().occlude().gap_1().child(tool_button("tb-update", icons::VCS_UPDATE, if cfg!(target_os = "macos") { "Update Project…  ⌘T" } else { "Update Project…  Ctrl+T" }).on_click(cx.listener(
                    |this, _, window, cx| dialogs::update_project(this.model.clone(), window, cx),
                )))
                .child(tool_button("tb-commit", icons::CHECKED, if cfg!(target_os = "macos") { "Commit…  ⌘K" } else { "Commit…  Ctrl+K" }).on_click(cx.listener(
                    |this, _, window, cx| this.on_commit(&CommitChanges, window, cx),
                )))
                .child(tool_button("tb-push", icons::VCS_PUSH, if cfg!(target_os = "macos") { "Push…  ⇧⌘K" } else { "Push…  Ctrl+Shift+K" }).on_click(cx.listener(
                    |this, _, window, cx| dialogs::push(this.model.clone(), window, cx),
                )))
                .child(tool_button("tb-fetch", icons::VCS_FETCH, "Fetch").on_click(cx.listener(
                    |this, _, _, cx| this.model.update(cx, |m, cx| m.fetch_all(cx)),
                )))
                // Android Studio's, for Gradle projects: Make, the device and
                // run configuration pickers, Run / Rerun, Stop, Sync.
                .when(gradle, |el| {
                    el.child(div().w(px(1.)).h(px(16.)).mx_1().bg(palette.border))
                        .child(self.render_run_widget(cx))
                        .child(
                            tool_button("tb-gradle-sync", icons::GRADLE, if cfg!(target_os = "macos") { "Sync Project with Gradle Files  ⇧⌘O" } else { "Sync Project with Gradle Files  Ctrl+Shift+O" })
                                .on_click(|_, _, cx| crate::ui::sync_service::SyncService::global(cx).update(cx, |s, cx| s.sync_all(cx))),
                        )
                })
                // The new UI's right corner: Search Everywhere and Settings.
                .child(div().w(px(1.)).h(px(16.)).mx_1().bg(palette.border))
                .child(tool_button("tb-search", icons::SEARCH, "Search Everywhere  Double Shift").on_click(cx.listener(
                    |this, _, window, cx| this.open_search_everywhere(SeTab::All, window, cx),
                )))
                .child(tool_button("tb-settings", icons::SETTINGS, if cfg!(target_os = "macos") { "Settings…  ⌘," } else { "Settings…  Ctrl+Alt+S" }).on_click(
                    cx.listener(|this, _, window, cx| crate::ui::settings_dialog::open(Some(this.model.clone()), window, cx)),
                ))),
        )
    }

    /// Android Studio's run widget: Make Project, the target device, the run
    /// configuration, Run (Rerun while it runs) and Stop.
    fn render_run_widget(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let run = self.run.clone();
        let service = run.read(cx);
        let config = service.configuration(cx);
        let configs = service.configurations(cx);
        let device = service.device(cx);
        let devices = service.devices(cx);
        let (busy, can_stop) = (service.is_busy(), service.can_stop_any(cx));
        let tests = service.test_configurations(cx);
        let selected_test = service.selected_test(cx);
        let running = service.console.as_ref().is_some_and(|c| c.active);
        let multiple = service.multiple(cx);
        let device_label = match multiple {
            Some(n) => format!("Multiple Devices ({n})"),
            None => device.as_ref().map(|d| d.name.clone()).unwrap_or_else(|| "No Devices".into()),
        };
        let device_icon = match device.as_ref() {
            Some(d) if d.kind == crate::android::Kind::Virtual => icons::VIRTUAL_DEVICE_PHONE,
            _ => icons::PHYSICAL_DEVICE_PHONE,
        };
        let config_name = selected_test.clone().or_else(|| config.as_ref().map(|c| c.name.clone()));
        let config_icon = if selected_test.is_some() { icons::TEST_ROOT } else { icons::MODULE_ANDROID };
        let (make, pick_device, pick_config, start, stop, debug, cover) = (run.clone(), run.clone(), run.clone(), run.clone(), run.clone(), run.clone(), run.clone());
        let cover_label = format!("Run '{}' with Coverage", config_name.as_deref().unwrap_or("app"));
        let selected_device = device.map(|d| d.id).filter(|_| multiple.is_none());
        let pick_several = run.clone();
        let selected_config = config.filter(|_| selected_test.is_none()).map(|c| (c.build_root, c.name));
        let pick_test = run.clone();
        let edit_configs = run.clone();
        h_flex()
            .gap_1()
            .child(
                tool_button("tb-make", icons::TW_BUILD, if cfg!(target_os = "macos") { "Make Project  ⌘F9" } else { "Make Project  Ctrl+F9" })
                    .disabled(busy)
                    .on_click(move |_, _, cx| make.update(cx, |r, cx| r.make_project(cx))),
            )
            .child(
                Button::new("tb-device")
                    .ghost()
                    .small()
                    .icon(Icon::new(device_icon).small())
                    .label(device_label)
                    .child(Icon::new(icons::CHEVRON_DOWN).xsmall())
                    .tooltip("Available Devices")
                    .dropdown_menu(move |mut menu, _, _| {
                        if devices.is_empty() {
                            return menu.item(PopupMenuItem::new("No devices").disabled(true));
                        }
                        let mut virtual_label = false;
                        menu = menu.label("Running Devices");
                        for d in &devices {
                            if !d.online && !virtual_label {
                                virtual_label = true;
                                menu = menu.separator().label("Available Devices");
                            }
                            let (id, run) = (d.id.clone(), pick_device.clone());
                            menu = menu.item(PopupMenuItem::new(d.name.clone()).checked(selected_device.as_deref() == Some(d.id.as_str())).on_click(move |_, _, cx| {
                                let id = id.clone();
                                run.update(cx, |r, cx| r.pick_device(id, cx))
                            }));
                        }
                        let several = pick_several.clone();
                        menu.separator().item(PopupMenuItem::new("Select Multiple Devices…").checked(multiple.is_some()).on_click(move |_, window, cx| {
                            crate::ui::multi_device_dialog::open(several.clone(), window, cx)
                        }))
                    }),
            )
            .child(
                Button::new("tb-config")
                    .ghost()
                    .small()
                    .icon(Icon::new(config_icon).small())
                    .label(config_name.clone().unwrap_or_else(|| "Add Configuration…".into()))
                    .child(Icon::new(icons::CHEVRON_DOWN).xsmall())
                    .tooltip("Run / Debug Configurations")
                    .dropdown_menu(move |mut menu, _, _| {
                        let edit = edit_configs.clone();
                        menu = menu.item(PopupMenuItem::new("Edit Configurations…").on_click(move |_, window, cx| edit.update(cx, |r, cx| r.edit_configurations(window, cx)))).separator();
                        if configs.is_empty() {
                            menu = menu.item(PopupMenuItem::new("No application modules").disabled(true));
                        }
                        for c in &configs {
                            let checked = selected_config.as_ref().is_some_and(|(b, n)| *b == c.build_root && *n == c.name);
                            let (c2, run) = (c.clone(), pick_config.clone());
                            menu = menu.item(PopupMenuItem::new(c.name.clone()).checked(checked).on_click(move |_, _, cx| {
                                let c = c2.clone();
                                run.update(cx, |r, cx| r.pick_configuration(&c, cx))
                            }));
                        }
                        // IntelliJ lists the temporary configurations (the
                        // tests run from the gutter or the Project window)
                        // after the saved ones, newest first.
                        if !tests.is_empty() {
                            menu = menu.separator();
                        }
                        for name in tests.iter().rev() {
                            let (n, run) = (name.clone(), pick_test.clone());
                            menu = menu.item(PopupMenuItem::new(name.clone()).icon(icons::TEST_ROOT).checked(selected_test.as_ref() == Some(name)).on_click(move |_, _, cx| {
                                let n = n.clone();
                                run.update(cx, |r, cx| r.pick_test(n, cx))
                            }));
                        }
                        menu
                    }),
            )
            .child(
                tool_button("tb-run", if running { icons::RERUN } else { icons::RUN }, crate::ui::run_views::run_label(config_name.as_deref().unwrap_or("app")))
                    .on_click(move |_, _, cx| start.update(cx, |r, cx| r.run(cx))),
            )
            .child(
                tool_button("tb-debug", icons::DEBUG, crate::ui::run_views::debug_label(config_name.as_deref().unwrap_or("app")))
                    .on_click(move |_, _, cx| debug.update(cx, |r, cx| r.debug(cx))),
            )
            // The new UI shows Stop only while something runs.
            .when(can_stop, |el| el.child(tool_button("tb-stop", icons::STOP, crate::ui::run_views::stop_label()).on_click(move |_, _, cx| stop.update(cx, |r, cx| r.stop(cx)))))
            // More Actions: the other executors of the configuration.
            .child(tool_button("tb-more", icons::MORE_VERTICAL, "More Actions").dropdown_menu(move |menu, _, _| {
                let cover = cover.clone();
                menu.item(PopupMenuItem::new(cover_label.clone()).icon(icons::RUN_WITH_COVERAGE).on_click(move |_, _, cx| cover.update(cx, |r, cx| r.run_with_coverage(cx))))
            }))
    }

    /// The Notifications tool window: every balloon of this session, newest first.
    pub(super) fn render_notifications(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mut list = v_flex().id("notification-list").flex_1().min_h_0().overflow_y_scroll().p_2().gap_2();
        if self.notifications.is_empty() {
            list = list.child(div().pt_8().w_full().text_center().text_sm().text_color(palette.text_secondary).child("No notifications"));
        }
        for record in &self.notifications {
            list = list.child(
                v_flex()
                    .gap_0p5()
                    .p_2()
                    .rounded_md()
                    .border_1()
                    .border_color(palette.border)
                    .text_sm()
                    .child(
                        h_flex()
                            .gap_1()
                            .child(match (record.error, record.warning) {
                                (true, _) => Icon::new(icons::STATUS_ERROR).xsmall().text_color(palette.status_conflict),
                                (_, true) => Icon::new(icons::STATUS_WARNING).xsmall().text_color(palette.ref_head),
                                _ => Icon::new(icons::STATUS_SUCCESS).xsmall().text_color(palette.status_added),
                            })
                            .child(div().flex_1().font_weight(FontWeight::SEMIBOLD).child(record.title.clone()))
                            .child(div().text_xs().text_color(palette.text_secondary).child(record.time.format("%H:%M").to_string())),
                    )
                    .child(div().text_color(palette.text_secondary).child(record.message.lines().take(6).collect::<Vec<_>>().join("\n"))),
            );
        }
        v_flex()
            .size_full()
            .bg(palette.panel)
            .child(
                h_flex()
                    .h(px(crate::ui::common::toolbar_height()))
                    .px_2()
                    .gap_1()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(div().flex_1().text_sm().font_weight(FontWeight::SEMIBOLD).child("Notifications"))
                    .child(tool_button("notifications-clear", icons::DELETE, "Clear All").on_click(cx.listener(|this, _, window, cx| {
                        this.notifications.clear();
                        // The balloons still showing go too.
                        gpui_kit::component::WindowExt::clear_notifications(window, cx);
                        cx.notify();
                    })))
                    .child(tool_button("notifications-hide", icons::HIDE, "Hide").on_click(cx.listener(|this, _, _, cx| {
                        this.tools.hide(ToolWindow::Notifications);
                        cx.notify();
                    }))),
            )
            .child(list)
    }

    /// IntelliJ's status bar: the file's path on the left; background task,
    /// caret, line separator, encoding, indent and the branch on the right.
    pub(super) fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let compiling = crate::ui::compile_service::CompileService::global(cx).read(cx).status();
        let syncing = crate::ui::sync_service::SyncService::global(cx).read(cx).status();
        let model = self.model.read(cx);
        let project = model.project_root().or_else(|| model.repository().map(|r| r.root())).and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
        let file = self.editor().map(|e| e.read(cx).path().to_owned());
        let branch = branches_popup::branch_widget_label(model);
        let task = model.busy().map(|b| format!("{b}…")).or_else(|| model.is_loading().then(|| "Refreshing VCS history…".to_owned())).or(syncing).or(compiling);
        let index = self.code_index.read(cx).summary();
        let compact = Settings::get(cx).compact;
        // Breadcrumbs: project › folders › file.
        let mut crumbs: Vec<String> = project.into_iter().collect();
        if let Some(file) = &file {
            // A library file: External Libraries › library › path inside it.
            let library = crate::index::store::ProjectIndex::is_external(file)
                .then(|| {
                    let index = self.code_index.read(cx).index.read().ok()?;
                    let library = index.external.library_of(file)?;
                    let rel = std::path::Path::new(file).strip_prefix(&library.root).ok()?.to_string_lossy().replace('\\', "/");
                    Some((library.root_label.clone(), library.name.clone(), rel))
                })
                .flatten();
            match library {
                // IntelliJ's: the archive, its packages, then the class ("core-1.13.1.jar › androidx › core › Foo").
                Some((Some(archive), _, rel)) => {
                    crumbs = vec![archive];
                    crumbs.extend(rel.split('/').map(|p| p.strip_suffix(".java").or_else(|| p.strip_suffix(".kt")).unwrap_or(p).to_owned()));
                }
                Some((None, name, rel)) => {
                    crumbs = vec!["External Libraries".to_owned(), name];
                    crumbs.extend(rel.split('/').map(str::to_owned));
                }
                None => crumbs.extend(file.split(['/', '\\']).filter(|p| !p.is_empty()).map(str::to_owned)),
            }
        }
        let crumb_count = crumbs.len();
        let mut path = h_flex().gap_0p5().min_w_0().overflow_hidden();
        for (ix, crumb) in crumbs.into_iter().enumerate() {
            if ix > 0 {
                path = path.child(Icon::new(icons::CHEVRON_RIGHT).xsmall());
            }
            path = path.child(div().when(ix + 1 == crumb_count && file.is_some(), |el| el.text_color(palette.text)).child(crumb));
        }
        h_flex()
            .h(px(if compact { 20. } else { 24. }))
            .px_3()
            .gap_3()
            .border_t_1()
            .border_color(palette.border)
            .bg(palette.toolbar)
            .text_xs()
            .text_color(palette.text_secondary)
            .child(path)
            .child(div().flex_1())
            .when_some(task, |el, task| {
                el.child(
                    h_flex()
                        .gap_1()
                        .child(Icon::new(IconName::LoaderCircle).xsmall().text_color(palette.accent))
                        .child(task),
                )
            })
            .child(index)
            .child(self.caret.clone())
            .child(
                div()
                    .id("status-branch")
                    .px_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .hover(|s| s.bg(palette.hover))
                    .child(h_flex().gap_1().child(Icon::new(icons::BRANCH).xsmall()).child(branch))
                    .on_click(cx.listener(|this, _, window, cx| this.open_branches(window, cx))),
            )
    }
}
