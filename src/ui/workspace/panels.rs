//! Tool windows: stripes, show / hide / focus, and the left (Commit, Stash,
//! Shelf, Pull Requests) and bottom (Git: Log, Console, …) panels.

use super::*;

impl Workspace {
    /// The open tool window that holds the focus.
    pub(super) fn focused_tool(&self, window: &Window, cx: &App) -> Option<ToolWindow> {
        ToolWindow::ALL
            .into_iter()
            .filter(|w| self.tools.is_open(*w))
            .find(|w| self.tool_focus.get(w).is_some_and(|f| f.contains_focused(window, cx)))
    }

    /// Shift+Escape: hides the active tool window, the one with the focus
    /// (else the one activated last), and gives the focus back.
    pub(super) fn hide_active_tool_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tool = self
            .focused_tool(window, cx)
            .or(self.last_tool.filter(|w| self.tools.is_open(*w)))
            .or_else(|| [Side::Bottom, Side::Left, Side::Right].into_iter().find_map(|s| self.tools.active(s)));
        if let Some(tool) = tool {
            self.tools.hide(tool);
            self.last_tool = None;
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    /// Opens or hides a tool window from its stripe button or shortcut; an
    /// opened window takes the focus, as IntelliJ activates it.
    pub(super) fn toggle_tool(&mut self, tool: ToolWindow, window: &mut Window, cx: &mut Context<Self>) {
        self.tools.toggle(tool);
        if tool == ToolWindow::PullRequests && self.tools.is_open(tool) {
            self.prs.update(cx, |prs, cx| prs.refresh(cx));
        }
        if tool == ToolWindow::Notifications {
            self.unread_notifications = 0;
        }
        if self.tools.is_open(tool) {
            self.focus_tool(tool, window, cx);
        } else {
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    /// A tool window's shortcut (Alt+1, Alt+9, Alt+0): as in IntelliJ, a
    /// window that is shown but doesn't hold the focus is activated; only
    /// the active one is hidden.
    pub(super) fn shortcut_tool(&mut self, tool: ToolWindow, window: &mut Window, cx: &mut Context<Self>) {
        let focused = self.tool_focus.get(&tool).is_some_and(|f| f.contains_focused(window, cx));
        if self.tools.is_open(tool) && !focused {
            self.tools.open(tool);
            self.focus_tool(tool, window, cx);
            cx.notify();
        } else {
            self.toggle_tool(tool, window, cx);
        }
    }

    /// Moves the focus into a tool window: the Log's table for Git, else the
    /// window itself.
    pub(super) fn focus_tool(&mut self, tool: ToolWindow, window: &mut Window, cx: &mut Context<Self>) {
        self.last_tool = Some(tool);
        let handle = match tool {
            ToolWindow::Git if self.bottom_tab == BottomTab::Log => gpui_kit::Focusable::focus_handle(self.log.read(cx), cx),
            ToolWindow::Project => gpui_kit::Focusable::focus_handle(self.project.read(cx), cx),
            _ => match self.tool_focus.get(&tool) {
                Some(handle) => handle.clone(),
                None => return,
            },
        };
        window.focus(&handle, cx);
    }

    pub(super) fn tool_window_info(&self, window: ToolWindow, cx: &App) -> (IconName, SharedString, &'static str) {
        match window {
            ToolWindow::Project => (IconName::FolderTree, "Project".into(), "Alt+1"),
            ToolWindow::Commit => (IconName::GitCommitVertical, "Commit".into(), "Alt+0"),
            ToolWindow::PullRequests => (IconName::GitPullRequest, self.prs.read(cx).title().into(), ""),
            ToolWindow::Changes => (IconName::FileDiff, "Changes".into(), ""),
            ToolWindow::Git => (IconName::GitGraph, "Git".into(), "Alt+9"),
            ToolWindow::Notifications => {
                (if self.unread_notifications > 0 { IconName::BellDot } else { IconName::Bell }, "Notifications".into(), "")
            }
        }
    }

    /// One stripe button: click toggles the window, drag moves it (drop on
    /// another button to go before it), right-click offers Move To and Hide.
    pub(super) fn stripe_button(&self, window: ToolWindow, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let (icon, title, shortcut) = self.tool_window_info(window, cx);
        let tooltip: SharedString = if shortcut.is_empty() {
            title.clone()
        } else {
            format!("{title} ({})", crate::ui::file_menus::shortcut_label(shortcut)).into()
        };
        let side = self.tools.side(window);
        let entity = cx.entity();
        div()
            .id(SharedString::from(format!("stripe-{window:?}")))
            .on_drag(DraggedToolWindow(window, icon), |dragged, _, _, cx| cx.new(|_| *dragged))
            .drag_over::<DraggedToolWindow>(move |el, _, _, _| el.border_t_2().border_color(palette.accent))
            .on_drop(cx.listener(move |this, dragged: &DraggedToolWindow, _, cx| {
                this.tools.move_to(dragged.0, side, Some(window), cx);
                cx.notify();
            }))
            .child(
                Button::new(SharedString::from(format!("stripe-button-{window:?}")))
                    .ghost()
                    .icon(Icon::new(icon))
                    .tooltip(tooltip)
                    .when(self.tools.is_open(window), |b| b.selected(true))
                    .on_click(cx.listener(move |this, _, w, cx| this.toggle_tool(window, w, cx))),
            )
            .context_menu(move |menu, _, _| {
                let mut menu = menu.label(title.clone());
                for target in Side::ALL {
                    let entity = entity.clone();
                    menu = menu.item(
                        PopupMenuItem::new(format!("Move to {}", target.label()))
                            .disabled(target == side)
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.tools.move_to(window, target, None, cx);
                                    cx.notify();
                                })
                            }),
                    );
                }
                let entity = entity.clone();
                menu.separator().item(PopupMenuItem::new("Hide").on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.tools.hide(window);
                        cx.notify();
                    })
                }))
            })
    }

    /// The left stripe holds the left windows, then the bottom ones at its
    /// foot, as in the new UI; the right stripe holds the right windows.
    pub(super) fn render_stripe(&self, right: bool, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let palette = cx.palette().clone();
        // Non-modal commit off: no Commit tool window, as in IntelliJ.
        let non_modal = Settings::get(cx).non_modal_commit;
        let visible = |w: &ToolWindow| match w {
            ToolWindow::Changes => !self.changes.read(cx).is_empty(),
            ToolWindow::Commit => non_modal,
            _ => true,
        };
        let top: Vec<ToolWindow> = self.tools.on_side(if right { Side::Right } else { Side::Left }).into_iter().filter(visible).collect();
        let bottom: Vec<ToolWindow> = if right { Vec::new() } else { self.tools.on_side(Side::Bottom).into_iter().filter(visible).collect() };
        if right && top.is_empty() && !cx.has_active_drag() {
            return None;
        }
        let compact = Settings::get(cx).compact;
        let drop_zone = |id: &'static str, side: Side, cx: &mut Context<Self>| {
            div()
                .id(id)
                .flex_1()
                .w_full()
                .min_h(px(24.))
                .drag_over::<DraggedToolWindow>(move |el, _, _, _| el.bg(palette.selection))
                .on_drop(cx.listener(move |this, dragged: &DraggedToolWindow, _, cx| {
                    this.tools.move_to(dragged.0, side, None, cx);
                    cx.notify();
                }))
        };
        let mut stripe = v_flex()
            .w(px(if compact { 32. } else { 40. }))
            .h_full()
            .py_1()
            .gap_1()
            .items_center()
            .when(right, |el| el.border_l_1())
            .when(!right, |el| el.border_r_1())
            .border_color(palette.border)
            .bg(palette.toolbar);
        for window in top {
            stripe = stripe.child(self.stripe_button(window, cx));
        }
        if right {
            stripe = stripe.child(drop_zone("stripe-drop-right", Side::Right, cx));
        } else {
            stripe = stripe
                .child(drop_zone("stripe-drop-left", Side::Left, cx))
                .child(div().w(px(20.)).h(px(1.)).bg(palette.border))
                .child(drop_zone("stripe-drop-bottom", Side::Bottom, cx).flex_none().h(px(24.)));
            for window in bottom {
                stripe = stripe.child(self.stripe_button(window, cx));
            }
        }
        Some(stripe)
    }

    /// The content of a tool window, shown in the panel of its side.
    pub(super) fn tool_window_content(&self, window: ToolWindow, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        // Out of the flow (absolute), wide content (a long filter label, a
        // narrow side panel) is clipped inside the panel instead of pushing
        // the stripes and the other panels out of the window.
        div()
            .id(SharedString::from(format!("tool-window-{window:?}")))
            .relative()
            .size_full()
            .when_some(self.tool_focus.get(&window), |el, focus| el.track_focus(focus))
            .child(div().absolute().inset_0().overflow_hidden().child(self.tool_window_view(window, cx)))
            .into_any_element()
    }

    /// Redraws every cached panel (see [`crate::ui::common::cached`]).
    pub(super) fn redraw_panels(&self, cx: &mut Context<Self>) {
        fn redraw<V: 'static>(view: &Entity<V>, cx: &mut Context<Workspace>) {
            view.update(cx, |_, cx| cx.notify());
        }
        redraw(&self.commit, cx);
        redraw(&self.stash, cx);
        redraw(&self.shelf, cx);
        redraw(&self.diff, cx);
        redraw(&self.worktrees, cx);
        redraw(&self.submodules, cx);
        redraw(&self.find, cx);
        redraw(&self.project, cx);
        redraw(&self.prs, cx);
        redraw(&self.changes, cx);
        for tab in &self.editors {
            redraw(&tab.view, cx);
        }
        if let Some((merge, _)) = &self.merge {
            redraw(merge, cx);
        }
        if let Some(timeline) = &self.timeline {
            redraw(timeline, cx);
        }
    }

    pub(super) fn tool_window_view(&self, window: ToolWindow, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        match window {
            ToolWindow::Project => crate::ui::common::cached(&self.project),
            ToolWindow::Commit => self.render_left(cx).into_any_element(),
            ToolWindow::PullRequests => crate::ui::common::cached(&self.prs),
            ToolWindow::Changes => crate::ui::common::cached(&self.changes),
            ToolWindow::Git => self.render_bottom(cx).into_any_element(),
            ToolWindow::Notifications => self.render_notifications(cx).into_any_element(),
        }
    }

    pub(super) fn render_left(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let current = self.left_tab;
        let tab = |id: &'static str, label: &'static str, value: LeftTab| {
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
        v_flex()
            .size_full()
            .bg(palette.panel)
            .child(
                h_flex()
                    .h(px(crate::ui::common::header_height()))
                    .px_2()
                    .gap_1()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(tab("left-commit", "Commit", LeftTab::Commit).on_click(cx.listener(|this, _, _, cx| {
                        this.left_tab = LeftTab::Commit;
                        cx.notify();
                    })))
                    .child(tab("left-stash", "Stash", LeftTab::Stash).on_click(cx.listener(|this, _, _, cx| {
                        this.left_tab = LeftTab::Stash;
                        cx.notify();
                    })))
                    .child(tab("left-shelf", "Shelf", LeftTab::Shelf).on_click(cx.listener(|this, _, _, cx| {
                        this.left_tab = LeftTab::Shelf;
                        cx.notify();
                    })))
                    .child(div().flex_1())
                    .child(tool_button("commit-hide", IconName::Minus, "Hide").on_click(cx.listener(|this, _, _, cx| {
                        this.tools.hide(ToolWindow::Commit);
                        cx.notify();
                    }))),
            )
            .child(div().flex_1().min_h_0().map(|el| match current {
                LeftTab::Commit => el.child(crate::ui::common::cached(&self.commit)),
                LeftTab::Stash => el.child(crate::ui::common::cached(&self.stash)),
                LeftTab::Shelf => el.child(crate::ui::common::cached(&self.shelf)),
            }))
    }

    /// Shows a tab of the Commit tool window. With non-modal commit off
    /// there is no Commit tool window: Commit opens the Commit Changes
    /// dialog and Shelf and Stash are tabs of the Git tool window.
    pub(super) fn show_left_tab(&mut self, tab: LeftTab, window: &mut Window, cx: &mut Context<Self>) {
        if Settings::get(cx).non_modal_commit {
            self.tools.open(ToolWindow::Commit);
            self.left_tab = tab;
        } else if tab == LeftTab::Commit {
            return self.open_modal_commit(window, cx);
        } else {
            self.tools.open(ToolWindow::Git);
            self.bottom_tab = if tab == LeftTab::Shelf { BottomTab::Shelf } else { BottomTab::Stash };
        }
        cx.notify();
    }

    /// Alt+0: the Commit tool window, or (non-modal commit off) the Git
    /// tool window's Local Changes tab.
    pub(super) fn toggle_commit_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if Settings::get(cx).non_modal_commit {
            return self.shortcut_tool(ToolWindow::Commit, window, cx);
        }
        let focused = self.tool_focus.get(&ToolWindow::Git).is_some_and(|f| f.contains_focused(window, cx));
        if self.tools.is_open(ToolWindow::Git) && self.bottom_tab == BottomTab::LocalChanges && focused {
            self.tools.hide(ToolWindow::Git);
            window.focus(&self.focus, cx);
        } else {
            self.tools.open(ToolWindow::Git);
            self.bottom_tab = BottomTab::LocalChanges;
            self.focus_tool(ToolWindow::Git, window, cx);
        }
        cx.notify();
    }

    pub(super) fn on_toggle_git(&mut self, _: &ToggleGitWindow, window: &mut Window, cx: &mut Context<Self>) {
        self.shortcut_tool(ToolWindow::Git, window, cx);
    }

    pub(super) fn render_bottom(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let tab = |id: &'static str, _label: &'static str, value: BottomTab, current: BottomTab| {
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
        };
        let current = self.bottom_tab;
        let non_modal = Settings::get(cx).non_modal_commit;
        let has_submodules = self.model.read(cx).repository().is_some_and(|r| r.root().join(".gitmodules").exists());
        v_flex()
            .size_full()
            .bg(palette.panel)
            .child(
                h_flex()
                    .h(px(crate::ui::common::header_height()))
                    .px_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Git"))
                    .when(!non_modal, |el| {
                        el.child(tab("tab-local-changes", "Local Changes", BottomTab::LocalChanges, current).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.bottom_tab = BottomTab::LocalChanges;
                                cx.notify();
                            },
                        )).child("Local Changes"))
                    })
                    .children(self.log_tabs.iter().enumerate().map(|(ix, log_tab)| {
                        let active = current == BottomTab::Log && ix == self.active_log;
                        div()
                            .id(("tab-log", ix))
                            .px_2()
                            .h_full()
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_sm()
                            .cursor_pointer()
                            .when(active, |el| el.border_b_2().border_color(palette.accent).text_color(palette.text))
                            .when(!active, |el| el.text_color(palette.text_secondary))
                            .on_click(cx.listener(move |this, _, _, cx| this.switch_log_tab(ix, cx)))
                            .child(log_tab.title.clone())
                            .when(ix > 0, |el| {
                                el.child(
                                    tool_button(("tab-log-close", ix), IconName::X, "Close Tab")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.close_log_tab(ix, cx)
                                        })),
                                )
                            })
                    }))
                    .child(tool_button("tab-log-new", IconName::Plus, "New Log Tab").on_click(cx.listener(|this, _, _, cx| {
                        let title = format!("Log {}", this.log_tabs.len() + 1);
                        this.open_log_tab(title, Default::default(), cx);
                    })))
                    .when(!non_modal, |el| {
                        el.child(tab("tab-shelf", "Shelf", BottomTab::Shelf, current).on_click(cx.listener(|this, _, _, cx| {
                            this.bottom_tab = BottomTab::Shelf;
                            cx.notify();
                        })).child("Shelf"))
                        .child(tab("tab-stash", "Stash", BottomTab::Stash, current).on_click(cx.listener(|this, _, _, cx| {
                            this.bottom_tab = BottomTab::Stash;
                            cx.notify();
                        })).child("Stash"))
                    })
                    .child(tab("tab-worktrees", "Worktrees", BottomTab::Worktrees, current).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.bottom_tab = BottomTab::Worktrees;
                            cx.notify();
                        },
                    )).child("Worktrees"))
                    .when(has_submodules, |el| {
                        el.child(tab("tab-submodules", "Submodules", BottomTab::Submodules, current).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.bottom_tab = BottomTab::Submodules;
                                cx.notify();
                            },
                        )).child("Submodules"))
                    })
                    .child(tab("tab-find", "Find", BottomTab::Find, current).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.bottom_tab = BottomTab::Find;
                            cx.notify();
                        },
                    )).child("Find"))
                    .child(tab("tab-console", "Console", BottomTab::Console, current).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.bottom_tab = BottomTab::Console;
                            cx.notify();
                        },
                    )).child("Console"))
                    .child(div().flex_1())
                    .child(tool_button("git-hide", IconName::Minus, "Hide").on_click(cx.listener(|this, _, _, cx| {
                        this.tools.hide(ToolWindow::Git);
                        cx.notify();
                    }))),
            )
            .child(div().flex_1().min_h_0().map(|el| match current {
                // The view is shown in one place at a time: the dialog has it while open.
                BottomTab::LocalChanges if self.modal_commit.get() => el.child(
                    div().size_full().flex().items_center().justify_center().text_sm().text_color(palette.text_secondary).child("Shown in the Commit Changes dialog"),
                ),
                BottomTab::LocalChanges => el.child(crate::ui::common::cached(&self.commit)),
                BottomTab::Shelf => el.child(crate::ui::common::cached(&self.shelf)),
                BottomTab::Stash => el.child(crate::ui::common::cached(&self.stash)),
                // Not cached itself: a cached view redraws everything inside it when
                // it changes, while the Log's parts are cached one by one.
                BottomTab::Log => el.child(self.log.clone()),
                BottomTab::Worktrees => el.child(crate::ui::common::cached(&self.worktrees)),
                BottomTab::Submodules => el.child(crate::ui::common::cached(&self.submodules)),
                BottomTab::Find => el.child(crate::ui::common::cached(&self.find)),
                BottomTab::Console => el.child(self.render_console(cx)),
            }))
    }

    /// The Git console, as IntelliJ's: "14:05:02.123: [root] git …" per
    /// command with its output below, failures in red; a side toolbar with
    /// Soft-Wrap, Scroll to the End and Clear All.
    pub(super) fn render_console(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let console = self.model.read(cx).console().clone();
        let entries = console.entries();
        let mono = cx.theme().mono_font_family.clone();
        let wrap = self.console_wrap;
        let mut list = v_flex().p_2().gap_0p5().font_family(mono).text_size(px(12.));
        for entry in entries.iter().rev().take(500).rev() {
            let line = format!("{}: [{}] {}", entry.time.format("%H:%M:%S%.3f"), entry.root, entry.command_line);
            list = list
                .child(
                    div()
                        .text_color(if entry.success { palette.text } else { palette.status_conflict })
                        .when(!wrap, |el| el.whitespace_nowrap())
                        .child(line),
                )
                .when(!entry.output.is_empty(), |el| {
                    el.child(
                        div()
                            .text_color(if entry.success { palette.text_secondary } else { palette.status_conflict })
                            .when(wrap, |el| el.whitespace_normal())
                            .when(!wrap, |el| el.whitespace_nowrap())
                            .child(entry.output.clone()),
                    )
                });
        }
        // Scroll to the End: follow new commands while it is on.
        if self.console_autoscroll && entries.len() != self.console_seen.get() {
            self.console_seen.set(entries.len());
            self.console_scroll.scroll_to_bottom();
        }
        let toggle = |id: &'static str, icon: IconName, tip: &'static str, on: bool| {
            tool_button(id, icon, tip).when(on, |b| b.selected(true))
        };
        h_flex()
            .size_full()
            .child(
                v_flex()
                    .h_full()
                    .w(px(28.))
                    .flex_shrink_0()
                    .py_1()
                    .gap_0p5()
                    .items_center()
                    .border_r_1()
                    .border_color(palette.border)
                    .child(toggle("console-wrap", IconName::TextWrap, "Soft-Wrap", wrap).on_click(cx.listener(|this, _, _, cx| {
                        this.console_wrap = !this.console_wrap;
                        cx.notify();
                    })))
                    .child(
                        toggle("console-end", IconName::ArrowDownToLine, "Scroll to the End", self.console_autoscroll).on_click(cx.listener(|this, _, _, cx| {
                            this.console_autoscroll = !this.console_autoscroll;
                            if this.console_autoscroll {
                                this.console_scroll.scroll_to_bottom();
                            }
                            cx.notify();
                        })),
                    )
                    .child(tool_button("console-clear", IconName::Delete, "Clear All").on_click(cx.listener(move |this, _, _, cx| {
                        console.clear();
                        this.console_seen.set(0);
                        cx.notify();
                    }))),
            )
            .child(
                div()
                    .id("git-console")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .track_scroll(&self.console_scroll)
                    .overflow_y_scroll()
                    .when(!wrap, |el| el.overflow_x_scroll())
                    .child(list),
            )
    }
}
