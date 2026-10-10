//! Window chrome: title bar, welcome screen, editor groups, notifications
//! and status bar.

use super::*;

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
                    .when(focused && matches!(self.front, Front::Editor(_)), |el| el.key_context(TABS_CONTEXT))
                    .map(|el| {
                        if !has_repo {
                            return el.child(self.render_welcome(cx));
                        }
                        match self.front {
                            Front::Editor(ix) if ix < self.editors.len() => el.child(crate::ui::common::cached(&self.editors[ix].view)),
                            _ if group != 0 => el,
                            Front::Timeline if self.timeline.is_some() => el.child(crate::ui::common::cached(self.timeline.as_ref().unwrap())),
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
            return div().flex_1().min_h_0().flex().child(self.render_group(true, has_repo, cx)).into_any_element();
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
        let action = |id: &'static str, icon: IconName, label: &'static str| {
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
                    .child(action("welcome-open", IconName::FolderOpen, "Open").on_click(cx.listener(
                        |this, _, window, cx| this.open_repository(window, cx),
                    )))
                    .child(action("welcome-clone", IconName::ArrowDownToLine, "Get from VCS").on_click(cx.listener(
                        |this, _, window, cx| clone_dialog::clone(this.model.clone(), window, cx),
                    )))
                    .child(action("welcome-init", IconName::Plus, "New Repository").on_click(cx.listener(
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
        let entity = cx.entity();
        let popup = self.branches_popup.clone();
        let operation_model = self.model.clone();
        let op = move |title: &'static str, args: &'static [&'static str], done: &'static str| {
            let model = operation_model.clone();
            move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut gpui_kit::App| {
                model.update(cx, |model, cx| {
                    model.run_operation(title, move |repo| {
                        repo.run(args)?;
                        Ok(done.to_owned())
                    }, cx)
                });
            }
        };

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
                                .child(Icon::new(IconName::ChevronDown).xsmall()),
                        )
                        .dropdown_menu({
                            let entity = entity.clone();
                            move |menu, _, cx| {
                                let current = entity.read(cx).model.read(cx).repository().map(|r| r.root().to_path_buf());
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
                                .icon(Icon::new(IconName::GitBranch).small())
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
                                .child(Icon::new(IconName::ChevronDown).xsmall()),
                        )
                        .child(popup),
                )
                .child(div().flex_1())
                .when_some(busy, |el, busy| {
                    el.child(div().text_xs().text_color(palette.text_secondary).child(format!("{busy}…")))
                })
                .child(tool_button("tb-update", IconName::ArrowDownToLine, if cfg!(target_os = "macos") { "Update Project…  ⌘T" } else { "Update Project…  Ctrl+T" }).on_click(cx.listener(
                    |this, _, window, cx| dialogs::update_project(this.model.clone(), window, cx),
                )))
                .child(tool_button("tb-commit", IconName::Check, if cfg!(target_os = "macos") { "Commit…  ⌘K" } else { "Commit…  Ctrl+K" }).on_click(cx.listener(
                    |this, _, window, cx| this.on_commit(&CommitChanges, window, cx),
                )))
                .child(tool_button("tb-push", IconName::ArrowUpFromLine, if cfg!(target_os = "macos") { "Push…  ⇧⌘K" } else { "Push…  Ctrl+Shift+K" }).on_click(cx.listener(
                    |this, _, window, cx| dialogs::push(this.model.clone(), window, cx),
                )))
                .child(tool_button("tb-fetch", IconName::CloudDownload, "Fetch").on_click(op(
                    "Fetch",
                    &["fetch", "--all", "--prune"],
                    "Fetched all remotes",
                )))
                // The new UI's right corner: Search Everywhere and Settings.
                .child(div().w(px(1.)).h(px(16.)).mx_1().bg(palette.border))
                .child(tool_button("tb-search", IconName::Search, "Search Everywhere  Double Shift").on_click(cx.listener(
                    |this, _, window, cx| this.open_search_everywhere(SeTab::All, window, cx),
                )))
                .child(tool_button("tb-settings", IconName::Settings, if cfg!(target_os = "macos") { "Settings…  ⌘," } else { "Settings…  Ctrl+Alt+S" }).on_click(
                    cx.listener(|this, _, window, cx| crate::ui::settings_dialog::open(Some(this.model.clone()), window, cx)),
                )),
        )
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
                                (true, _) => Icon::new(IconName::CircleX).xsmall().text_color(palette.status_conflict),
                                (_, true) => Icon::new(IconName::TriangleAlert).xsmall().text_color(palette.ref_head),
                                _ => Icon::new(IconName::CircleCheck).xsmall().text_color(palette.status_added),
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
                    .child(tool_button("notifications-clear", IconName::Delete, "Clear All").on_click(cx.listener(|this, _, _, cx| {
                        this.notifications.clear();
                        cx.notify();
                    })))
                    .child(tool_button("notifications-hide", IconName::Minus, "Hide").on_click(cx.listener(|this, _, _, cx| {
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
        let model = self.model.read(cx);
        let project = model.project_root().or_else(|| model.repository().map(|r| r.root())).and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
        let file = self.editor().map(|e| e.read(cx).path().to_owned());
        let branch = branches_popup::branch_widget_label(model);
        let task = model.busy().map(|b| format!("{b}…")).or_else(|| model.is_loading().then(|| "Refreshing VCS history…".to_owned()));
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
                    Some((library.name.clone(), rel))
                })
                .flatten();
            match library {
                Some((name, rel)) => {
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
                path = path.child(Icon::new(IconName::ChevronRight).xsmall());
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
                    .child(h_flex().gap_1().child(Icon::new(IconName::GitBranch).xsmall()).child(branch))
                    .on_click(cx.listener(|this, _, window, cx| this.open_branches(window, cx))),
            )
    }
}
