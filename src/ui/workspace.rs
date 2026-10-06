//! The main window, laid out like IntelliJ's new UI: title bar with project
//! and VCS widgets, a left tool window stripe, the Commit tool window, the
//! editor area (diff), the bottom Git tool window (Log / Console), and the
//! status bar.

use gpui_kit::component::{
    Selectable as _,
    ActiveTheme as _, Icon, Sizable as _, TitleBar, WindowExt as _, h_flex,
    button::{Button, ButtonVariants as _},
    menu::{DropdownMenu as _, PopupMenuItem},
    notification::Notification,
    popover::Popover,
    resizable_panel,
    scroll::ScrollableElement as _,
    v_flex, v_resizable, h_resizable,
};
use gpui_kit::assets::IconName;
use gpui_kit::{
    AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    PathPromptOptions, Render, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _, px,
};

use crate::model::{RepoEvent, RepoModel};
use crate::theme::{self, ActivePalette as _};
use crate::ui::branches_popup::{self, BranchesPopup};
use crate::ui::commit_view::{CommitEvent, CommitView};
use crate::ui::common::tool_button;
use crate::ui::diff_view::DiffView;
use crate::ui::log_view::{LogEvent, LogView};

#[derive(Clone, Copy, PartialEq, Eq)]
enum BottomTab {
    Log,
    Console,
}

pub struct Workspace {
    model: Entity<RepoModel>,
    log: Entity<LogView>,
    commit: Entity<CommitView>,
    diff: Entity<DiffView>,
    branches_popup: Entity<BranchesPopup>,
    show_commit: bool,
    show_git: bool,
    bottom_tab: BottomTab,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(model: Entity<RepoModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let log = cx.new(|cx| LogView::new(model.clone(), window, cx));
        let commit = cx.new(|cx| CommitView::new(model.clone(), window, cx));
        let diff = cx.new(|_| DiffView::new());
        let branches_popup = cx.new(|cx| BranchesPopup::new(model.clone(), window, cx));
        let subscriptions = vec![
            cx.subscribe(&log, |this, _, event: &LogEvent, cx| match event {
                LogEvent::OpenDiff(source) => this.open_diff(source.clone(), cx),
            }),
            cx.subscribe(&commit, |this, _, event: &CommitEvent, cx| match event {
                CommitEvent::OpenDiff(source) => this.open_diff(source.clone(), cx),
            }),
            cx.subscribe_in(&model, window, |_, _, event, window, cx| {
                if let RepoEvent::Notify { title, message, error } = event {
                    let notification = if *error {
                        Notification::error(message.clone()).title(title.clone())
                    } else {
                        Notification::success(message.clone()).title(title.clone())
                    };
                    window.push_notification(notification, cx);
                }
            }),
            cx.observe(&model, |_, _, cx| cx.notify()),
        ];
        Self {
            model,
            log,
            commit,
            diff,
            branches_popup,
            show_commit: true,
            show_git: true,
            bottom_tab: BottomTab::Log,
            _subscriptions: subscriptions,
        }
    }

    fn open_diff(&mut self, source: crate::ui::diff_view::DiffSource, cx: &mut Context<Self>) {
        let Some(repository) = self.model.read(cx).repository().cloned() else { return };
        self.diff.update(cx, |diff, cx| diff.show(repository, source, cx));
    }

    fn open_repository(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open Git Repository".into()),
        });
        let model = self.model.clone();
        cx.spawn(async move |_, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                if let Some(path) = paths.into_iter().next() {
                    model.update(cx, |model, cx| model.open(path, cx));
                }
            }
        })
        .detach();
    }

    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let model = self.model.read(cx);
        let project = model.repository().map(|r| r.name()).unwrap_or_else(|| "No Project".into());
        let branch = branches_popup::branch_widget_label(model);
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
                        .dropdown_menu({
                            let entity = entity.clone();
                            move |menu, _, cx| {
                                let dark = cx.palette().dark;
                                let open = entity.clone();
                                menu.item(PopupMenuItem::new("Open Repository…").on_click(move |_, window, cx| {
                                    open.update(cx, |this, cx| this.open_repository(window, cx))
                                }))
                                .separator()
                                .item(PopupMenuItem::new("Light Theme").checked(!dark).on_click(|_, window, cx| {
                                    theme::apply(false, cx);
                                    window.refresh();
                                }))
                                .item(PopupMenuItem::new("Dark Theme").checked(dark).on_click(|_, window, cx| {
                                    theme::apply(true, cx);
                                    window.refresh();
                                }))
                                .separator()
                                .item(PopupMenuItem::new("Exit").on_click(|_, _, cx| cx.quit()))
                            }
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
                        .on_click(cx.listener(|this, _, window, cx| this.open_repository(window, cx))),
                )
                .child(
                    Popover::new("branches-popover")
                        .anchor(gpui_kit::Anchor::TopLeft)
                        .trigger(
                            Button::new("vcs-widget")
                                .ghost()
                                .small()
                                .icon(Icon::new(IconName::GitBranch).small())
                                .label(branch)
                                .child(Icon::new(IconName::ChevronDown).xsmall()),
                        )
                        .child(popup),
                )
                .child(div().flex_1())
                .when_some(busy, |el, busy| {
                    el.child(div().text_xs().text_color(palette.text_secondary).child(format!("{busy}…")))
                })
                .child(tool_button("tb-update", IconName::ArrowDownToLine, "Update Project…  Ctrl+T").on_click(op(
                    "Update Project",
                    &["pull", "--rebase", "--autostash"],
                    "All files are up to date",
                )))
                .child(tool_button("tb-commit", IconName::Check, "Commit…  Ctrl+K").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.show_commit = true;
                        cx.notify();
                    },
                )))
                .child(tool_button("tb-push", IconName::ArrowUpFromLine, "Push…  Ctrl+Shift+K").on_click(op(
                    "Push",
                    &["push"],
                    "Pushed",
                )))
                .child(tool_button("tb-fetch", IconName::CloudDownload, "Fetch").on_click(op(
                    "Fetch",
                    &["fetch", "--all", "--prune"],
                    "Fetched all remotes",
                ))),
        )
    }

    fn render_stripe(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let stripe_button = |id: &'static str, icon: IconName, tooltip: &'static str, active: bool| {
            Button::new(id)
                .ghost()
                .icon(Icon::new(icon))
                .tooltip(tooltip)
                .when(active, |b| b.selected(true))
        };
        v_flex()
            .w(px(40.))
            .h_full()
            .py_1()
            .gap_1()
            .items_center()
            .border_r_1()
            .border_color(palette.border)
            .bg(palette.toolbar)
            .child(stripe_button("stripe-commit", IconName::GitCommitVertical, "Commit", self.show_commit).on_click(
                cx.listener(|this, _, _, cx| {
                    this.show_commit = !this.show_commit;
                    cx.notify();
                }),
            ))
            .child(div().flex_1())
            .child(stripe_button("stripe-git", IconName::GitGraph, "Git", self.show_git).on_click(cx.listener(
                |this, _, _, cx| {
                    this.show_git = !this.show_git;
                    cx.notify();
                },
            )))
    }

    fn render_bottom(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
        v_flex()
            .size_full()
            .bg(palette.panel)
            .child(
                h_flex()
                    .h(px(30.))
                    .px_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Git"))
                    .child(tab("tab-log", "Log", BottomTab::Log, current).on_click(cx.listener(|this, _, _, cx| {
                        this.bottom_tab = BottomTab::Log;
                        cx.notify();
                    })).child("Log"))
                    .child(tab("tab-console", "Console", BottomTab::Console, current).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.bottom_tab = BottomTab::Console;
                            cx.notify();
                        },
                    )).child("Console"))
                    .child(div().flex_1())
                    .child(tool_button("git-hide", IconName::Minus, "Hide").on_click(cx.listener(|this, _, _, cx| {
                        this.show_git = false;
                        cx.notify();
                    }))),
            )
            .child(div().flex_1().min_h_0().map(|el| match current {
                BottomTab::Log => el.child(self.log.clone()),
                BottomTab::Console => el.child(self.render_console(cx)),
            }))
    }

    fn render_console(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let entries = self.model.read(cx).console().entries();
        let mono = cx.theme().mono_font_family.clone();
        let mut list = v_flex().p_2().gap_1().font_family(mono).text_size(px(12.));
        for entry in entries.iter().rev().take(300).rev() {
            list = list.child(
                v_flex()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().text_color(if entry.success { palette.link } else { palette.status_conflict }).child(entry.command_line.clone()))
                            .child(div().text_color(palette.text_disabled).child(format!("{} ms", entry.duration.as_millis()))),
                    )
                    .when(!entry.output.is_empty(), |el| {
                        el.child(div().pl_4().text_color(palette.text_secondary).whitespace_normal().child(entry.output.clone()))
                    }),
            );
        }
        div().id("git-console").size_full().overflow_y_scrollbar().child(list)
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let model = self.model.read(cx);
        let project = model.repository().map(|r| r.root().display().to_string()).unwrap_or_default();
        let branch = branches_popup::branch_widget_label(model);
        let changes = model.status().entries.len();
        h_flex()
            .h(px(24.))
            .px_3()
            .gap_3()
            .border_t_1()
            .border_color(palette.border)
            .bg(palette.toolbar)
            .text_xs()
            .text_color(palette.text_secondary)
            .child(project)
            .child(div().flex_1())
            .when(model.is_loading(), |el| el.child("Refreshing VCS history…"))
            .child(format!("{changes} changed"))
            .child(h_flex().gap_1().child(Icon::new(IconName::GitBranch).xsmall()).child(branch))
            .child("UTF-8")
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let error = self.model.read(cx).error().map(str::to_owned);
        let has_repo = self.model.read(cx).repository().is_some();

        let editor = v_flex()
            .size_full()
            .bg(palette.panel)
            .when_some(error, |el, error| {
                el.child(div().p_2().text_sm().text_color(palette.status_conflict).child(error))
            })
            .child(div().flex_1().min_h_0().child(self.diff.clone()));

        let top = h_resizable("top-split")
            .child(
                resizable_panel()
                    .size(px(340.))
                    .size_range(px(220.)..px(700.))
                    .visible(self.show_commit && has_repo)
                    .child(div().size_full().bg(palette.panel).child(self.commit.clone())),
            )
            .child(resizable_panel().child(editor));

        let main = v_resizable("main-split")
            .child(resizable_panel().child(top))
            .child(
                resizable_panel()
                    .size(px(380.))
                    .size_range(px(120.)..px(1200.))
                    .visible(self.show_git && has_repo)
                    .child(self.render_bottom(cx)),
            );

        v_flex()
            .size_full()
            .bg(palette.window)
            .text_color(palette.text)
            .text_size(px(13.))
            .child(self.render_title_bar(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_stripe(cx))
                    .child(div().flex_1().h_full().child(main)),
            )
            .child(self.render_status_bar(cx))
    }
}
