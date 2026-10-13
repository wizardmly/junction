//! A file's History tab, laid out as IntelliJ's File History: a toolbar
//! (Branch filter, Refresh, Show Diff, Show All Affected Files, the view
//! options, Open on GitHub and Show in Git Log), a graph-less table of
//! author, date and subject, the commit details under it and the file's
//! diff preview below or to the right.

use super::*;
use crate::git::log::FileAt;
use crate::ui::as_icons as icons;
use crate::ui::diff_view::DiffView;

pub(super) struct FileHistory {
    /// The file as the Log filters it (relative to the project).
    path: String,
    /// The repository holding it, and the file's path there.
    repository: Option<crate::git::Repository>,
    own: String,
    /// A folder's history: the selected commit's changes instead of a diff.
    is_dir: bool,
    /// Each commit's version of the file: its path then, what the commit
    /// did to it and the commit time; for the branches it was loaded for.
    revisions: Arc<HashMap<String, FileAt>>,
    loaded_for: Option<Vec<String>>,
    diff: Entity<DiffView>,
    shown: Option<DiffSource>,
    _task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl LogView {
    /// The Log tab shown is a file's History (`Some(path)`) or a Log.
    pub fn set_history(&mut self, path: Option<String>, cx: &mut Context<Self>) {
        match path {
            None => {
                if self.history.take().is_some() {
                    cx.notify();
                }
            }
            Some(path) if self.history.as_ref().is_some_and(|h| h.path == path) => {}
            Some(path) => {
                let (repository, own) = match self.model.read(cx).route(&path) {
                    Some((repository, own)) => (Some(repository), own),
                    None => (None, path.clone()),
                };
                let is_dir = own.is_empty() || repository.as_ref().is_some_and(|r| r.root().join(&own).is_dir());
                let diff = cx.new(DiffView::new);
                let subscriptions = vec![
                    cx.subscribe(&diff, |this, diff, event: &crate::ui::navigate::OpenTarget, cx| {
                        let mut target = event.0.clone();
                        if let Some(root) = diff.read(cx).repository().map(|r| r.root().to_path_buf()) {
                            target.path = this.model.read(cx).project_path(&root, &target.path);
                        }
                        cx.emit(LogEvent::GoTo(target));
                    }),
                    cx.subscribe(&diff, |this, _, _: &crate::ui::diff_view::FilesChanged, cx| {
                        this.model.update(cx, |m, cx| m.reload(cx));
                    }),
                    cx.subscribe(&diff, |_, diff, event: &crate::ui::diff_view::DiffEvent, cx| cx.emit(LogEvent::Diff(diff, event.clone()))),
                ];
                self.history = Some(FileHistory {
                    path,
                    repository,
                    own,
                    is_dir,
                    revisions: Arc::default(),
                    loaded_for: None,
                    diff,
                    shown: None,
                    _task: None,
                    _subscriptions: subscriptions,
                });
                self.load_history_revisions(cx);
                cx.notify();
            }
        }
    }

    pub(super) fn is_history(&self) -> bool {
        self.history.is_some()
    }

    /// Which version of the file each commit has (renames), off the main thread.
    pub(super) fn load_history_revisions(&mut self, cx: &mut Context<Self>) {
        let branches = self.model.read(cx).filter().branches.clone();
        let Some(history) = self.history.as_mut() else { return };
        if history.is_dir || history.loaded_for.as_ref() == Some(&branches) {
            return;
        }
        let Some(repository) = history.repository.clone() else { return };
        history.loaded_for = Some(branches.clone());
        let own = history.own.clone();
        history._task = Some(cx.spawn(async move |this, cx| {
            let revisions = cx.background_spawn(async move { crate::git::log::file_history_paths(&repository, &own, &branches) }).await;
            this.update(cx, |this, cx| {
                if let Some(history) = this.history.as_mut() {
                    history.revisions = Arc::new(revisions);
                    history.shown = None;
                }
                this.update_history_preview(cx);
                cx.notify();
            })
            .ok();
        }));
    }

    /// The file's path in a commit and, when the commit renamed it, the path before.
    pub(super) fn history_path_at(&self, hash: &str) -> Option<(String, Option<String>)> {
        let history = self.history.as_ref()?;
        Some(match history.revisions.get(hash) {
            Some(at) => (at.change.path.clone(), at.change.old_path.clone()),
            None => (history.own.clone(), None),
        })
    }

    /// The selected commits' diff of the file: one commit against its
    /// parent, two (or more) between the oldest and the newest.
    fn history_diff_source(&self, cx: &App) -> Option<DiffSource> {
        let history = self.history.as_ref()?;
        if history.is_dir {
            return None;
        }
        let selected = self.selected_commits(cx);
        match selected.as_slice() {
            [] => None,
            [one] => {
                let (path, old_path) = self.history_path_at(&one.hash)?;
                Some(DiffSource::Commit { hash: one.hash.clone(), path, old_path })
            }
            [oldest, .., newest] => {
                let (path, _) = self.history_path_at(&newest.hash)?;
                let (old, _) = self.history_path_at(&oldest.hash)?;
                Some(DiffSource::Between { old: oldest.hash.clone(), new: Some(newest.hash.clone()), old_path: (old != path).then_some(old), path })
            }
        }
    }

    /// Shows the selection in the diff preview (when it's on and changed).
    pub(super) fn update_history_preview(&mut self, cx: &mut Context<Self>) {
        if self.history.is_none() || !Settings::get(cx).log.history_diff {
            return;
        }
        let source = self.history_diff_source(cx);
        let Some(history) = self.history.as_mut() else { return };
        if history.shown == source {
            return;
        }
        history.shown = source.clone();
        let repository = history.repository.clone();
        history.diff.update(cx, |diff, cx| match (source, repository) {
            (Some(source), Some(repository)) => diff.show(repository, source, cx),
            _ => diff.clear(cx),
        });
    }

    /// Show Diff (Ctrl+D, double-click): the selection's diff in the editor.
    pub(super) fn history_show_diff(&mut self, new_tab: bool, cx: &mut Context<Self>) {
        if let Some(source) = self.history_diff_source(cx) {
            cx.emit(if new_tab { LogEvent::OpenDiffInNewTab(source) } else { LogEvent::OpenDiff(source) });
        }
    }

    pub(super) fn on_show_diff(&mut self, _: &ShowDiff, _: &mut Window, cx: &mut Context<Self>) {
        self.history_show_diff(false, cx);
    }

    /// Show All Affected Files: every file the commit changed, as IntelliJ's
    /// "Paths Affected by Commit" dialog; double-click shows a file's diff.
    pub(super) fn show_affected(&mut self, hash: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repository) = self.history.as_ref().and_then(|h| h.repository.clone()) else { return };
        let Ok(details) = crate::git::log::load_details_basic(&repository, &hash) else { return };
        let this_path = self.history_path_at(&hash).map(|(path, _)| path);
        let entity = cx.entity();
        let short = hash[..hash.len().min(8)].to_owned();
        let changes = details.changes.clone();
        window.open_dialog(cx, move |dialog, _, cx| {
            let palette = cx.palette().clone();
            let rows = changes.iter().enumerate().map(|(ix, change)| {
                let source = DiffSource::Commit { hash: hash.clone(), path: change.path.clone(), old_path: change.old_path.clone() };
                let entity = entity.clone();
                let (dir, name) = match change.path.rsplit_once('/') {
                    Some((dir, name)) => (dir.to_owned(), name.to_owned()),
                    None => (String::new(), change.path.clone()),
                };
                let current = this_path.as_deref() == Some(change.path.as_str());
                h_flex()
                    .id(("affected", ix))
                    .h(px(row_height()))
                    .px_2()
                    .gap_1()
                    .text_sm()
                    .when(current, |el| el.bg(palette.selection_inactive))
                    .hover(|el| el.bg(palette.hover))
                    .child(Icon::from(common::file_icon(&change.path)).small())
                    .child(div().text_color(common::change_color(change.kind, &palette)).child(name))
                    .child(div().text_color(palette.text_secondary).child(dir))
                    .when_some(change.old_path.clone(), |el, old| el.child(div().text_xs().text_color(palette.text_secondary).child(format!("from {old}"))))
                    .on_click(move |event, _, cx| {
                        if event.click_count() == 2 {
                            let source = source.clone();
                            entity.update(cx, |_, cx| cx.emit(LogEvent::OpenDiff(source)));
                        }
                    })
            });
            dialog
                .title(format!("Paths Affected by Commit {short}"))
                .w(px(640.))
                .child(div().id("affected-list").h(px(360.)).overflow_y_scrollbar().children(rows))
        });
    }

    /// History toolbar, IntelliJ's Vcs.FileHistory.Toolbar.
    fn render_history_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let model = self.model.read(cx);
        let filter = model.filter().clone();
        let refs = model.log_refs().clone();
        let web = model.web_repo().cloned();
        let loading = model.is_loading();
        let selected = self.selected_commits(cx);
        let single = selected.len() == 1;
        let entity = cx.entity();
        let separator = || div().flex_shrink_0().w(px(1.)).h(px(18.)).mx_1().bg(palette.border);
        let branch_label = match filter.branches.as_slice() {
            [] => "Branch".to_owned(),
            [one] => format!("Branch: {}", if one.len() == 40 { &one[..8] } else { one }),
            many => format!("Branch: {} selected", many.len()),
        };
        let settings = Settings::get(cx).log.clone();
        let github = web.zip(selected.first().cloned()).filter(|_| single).and_then(|(web, commit)| {
            let (path, _) = self.history_path_at(&commit.hash)?;
            Some((web.host.name(), web.file_url(&commit.hash, &path, None)))
        });
        h_flex()
            .h(px(crate::ui::common::toolbar_height()))
            .flex_shrink_0()
            .px_1()
            .gap_0p5()
            .overflow_hidden()
            .border_b_1()
            .border_color(palette.border)
            .child(
                h_flex()
                    .flex_shrink_0()
                    .child(
                        Button::new("history-branch")
                            .ghost()
                            .xsmall()
                            .label(branch_label)
                            .when(filter.branches.is_empty(), |b| b.icon(Icon::new(icons::CHEVRON_DOWN).xsmall()))
                            .dropdown_menu(self.branch_filter_menu(&entity, &refs, &filter)),
                    )
                    .when(!filter.branches.is_empty(), |el| {
                        el.child(
                            Button::new("history-branch-clear")
                                .ghost()
                                .with_size(px(18.))
                                .icon(icons::CLOSE_SMALL)
                                .icon_size(px(16.))
                                .tooltip("Clear Filter")
                                .on_click(cx.listener(|this, _, _, cx| this.update_filter(cx, |f| f.branches.clear()))),
                        )
                    }),
            )
            .child(separator())
            .child(tool_button("history-refresh", icons::REFRESH, "Refresh").on_click(cx.listener(|this, _, _, cx| {
                if let Some(history) = this.history.as_mut() {
                    history.loaded_for = None;
                }
                this.load_history_revisions(cx);
                this.model.update(cx, |model, cx| model.reload(cx));
            })))
            .when_some(self.history.as_ref().filter(|h| !h.is_dir), |el, _| {
                el.child(
                    tool_button("history-show-diff", icons::VCS_DIFF, "Show Diff (Ctrl+D)")
                        .disabled(selected.is_empty())
                        .on_click(cx.listener(|this, _, _, cx| this.history_show_diff(false, cx))),
                )
            })
            .child(
                tool_button("history-affected", icons::VCS_CHANGES, "Show All Affected Files").disabled(!single).on_click(cx.listener({
                    let hash = selected.first().map(|c| c.hash.clone());
                    move |this, _, window, cx| {
                        if let Some(hash) = hash.clone() {
                            this.show_affected(hash, window, cx)
                        }
                    }
                })),
            )
            .child(separator())
            .child(
                Button::new("history-options")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(icons::SHOW))
                    .tooltip("View Options")
                    .dropdown_menu({
                        let entity = entity.clone();
                        let is_dir = self.history.as_ref().is_some_and(|h| h.is_dir);
                        move |menu, window, cx| {
                            let toggle = |label: &'static str, on: bool, set: fn(&mut crate::settings::LogSettings, bool)| {
                                let entity = entity.clone();
                                PopupMenuItem::new(label).checked(on).on_click(move |_, _, cx| {
                                    Settings::update(cx, |s| set(&mut s.log, !on));
                                    entity.update(cx, |this, cx| {
                                        this.update_history_preview(cx);
                                        cx.notify();
                                    });
                                })
                            };
                            let s = settings.clone();
                            let columns = settings.clone();
                            let (place_right, place_bottom) = (entity.clone(), entity.clone());
                            menu.label("Show")
                                .item(toggle("Commit Timestamp", s.history_commit_time, |s, v| s.history_commit_time = v))
                                .submenu("Show Columns", window, cx, move |sub, _, _| {
                                    let toggle = |label: &'static str, on: bool, set: fn(&mut crate::settings::LogSettings, bool)| {
                                        PopupMenuItem::new(label).checked(on).on_click(move |_, _, cx| Settings::update(cx, |s| set(&mut s.log, !on)))
                                    };
                                    sub.item(toggle("Author", columns.history_author, |s, v| s.history_author = v))
                                        .item(toggle("Date", columns.history_date, |s, v| s.history_date = v))
                                        .item(toggle("Hash", columns.history_hash, |s, v| s.history_hash = v))
                                })
                                .separator()
                                .label("Layout")
                                .item(toggle("Show Details", s.history_details, |s, v| s.history_details = v))
                                .item(toggle(if is_dir { "Show Changes" } else { "Show Diff Preview" }, s.history_diff, |s, v| s.history_diff = v))
                                .submenu("Diff Preview Location", window, cx, move |sub, _, _| {
                                    let place = |label: &'static str, right: bool, entity: Entity<LogView>| {
                                        PopupMenuItem::new(label).checked(s.history_diff_right == right).on_click(move |_, _, cx| {
                                            Settings::update(cx, |s| {
                                                s.log.history_diff_right = right;
                                                s.log.history_diff = true;
                                            });
                                            entity.update(cx, |this, cx| {
                                                this.update_history_preview(cx);
                                                cx.notify();
                                            });
                                        })
                                    };
                                    sub.item(place("Right", true, place_right.clone())).item(place("Bottom", false, place_bottom.clone()))
                                })
                        }
                    }),
            )
            .when_some(github, |el, (name, url)| {
                el.child(separator()).child(tool_button("history-open-web", icons::GITHUB, format!("Open on {name}")).on_click(move |_, _, cx| cx.open_url(&url)))
            })
            .child(div().flex_1())
            .when(loading, |el| el.child(div().text_xs().text_color(palette.text_secondary).child("Loading…")))
            .child(
                tool_button("history-select-in-log", icons::TW_VCS, "Show in Git Log").disabled(!single).on_click(cx.listener({
                    let hash = selected.first().map(|c| c.hash.clone());
                    move |_, _, _, cx| {
                        if let Some(hash) = hash.clone() {
                            cx.emit(LogEvent::ShowInLog(hash));
                        }
                    }
                })),
            )
    }

    /// The History tab: toolbar, table (with the details under it) and the
    /// diff preview to the right or below, as IntelliJ's FileHistoryPanel.
    pub(super) fn render_history(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let settings = Settings::get(cx).log.clone();
        let Some(history) = self.history.as_ref() else { return div().into_any_element() };
        let table = div().size_full().child(crate::ui::common::cached(&self.parts[0]));
        let table = if settings.history_details {
            let info = self.render_commit_info(cx);
            v_resizable("history-details")
                .child(resizable_panel().child(div().size_full().overflow_hidden().child(table)))
                .child(resizable_panel().size(px(110.)).size_range(px(60.)..px(600.)).child(div().relative().size_full().child(div().absolute().inset_0().child(info))))
                .into_any_element()
        } else {
            table.into_any_element()
        };
        let preview = if history.is_dir {
            crate::ui::common::cached(&self.parts[2]).into_any_element()
        } else {
            div().size_full().child(history.diff.clone()).into_any_element()
        };
        // The toolbar belongs to the table, the diff preview beside them
        // (70 / 30), as IntelliJ's FileHistoryPanel lays them out.
        let left = v_flex()
            .size_full()
            .overflow_hidden()
            .child(self.render_history_toolbar(cx))
            .child(div().flex_1().min_h_0().overflow_hidden().child(table));
        match (settings.history_diff, settings.history_diff_right) {
            (false, _) => left.into_any_element(),
            (true, true) => h_resizable("history-diff-right")
                .child(resizable_panel().child(left))
                .child(resizable_panel().size(px(480.)).size_range(px(200.)..px(1600.)).child(div().size_full().overflow_hidden().child(preview)))
                .into_any_element(),
            (true, false) => v_resizable("history-diff-bottom")
                .child(resizable_panel().child(left))
                .child(resizable_panel().size(px(260.)).size_range(px(100.)..px(1200.)).child(div().size_full().overflow_hidden().child(preview)))
                .into_any_element(),
        }
    }

    /// History rows: author, date and subject (and the hash when shown), no
    /// graph; a commit's branches and tags as icons at the right end.
    pub(super) fn render_history_rows(&mut self, range: std::ops::Range<usize>, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let palette = cx.palette().clone();
        let focused = self.focus.contains_focused(window, cx);
        let settings = Settings::get(cx).log.clone();
        let model = self.model.read(cx);
        let commits = model.commits().clone();
        let refs = model.log_refs().clone();
        let selected = model.selected_index();
        let entity = cx.entity();
        let revisions = self.history.as_ref().map(|h| h.revisions.clone()).unwrap_or_default();
        // The author column fits the longest name, as IntelliJ sizes it.
        let author_width = commits.iter().take(2000).map(|c| c.author_name.chars().count()).max().unwrap_or(8).clamp(6, 28) as f32 * 7.5 + 20.;
        let mono = cx.theme().mono_font_family.clone();
        range
            .filter_map(|ix| {
                let commit = commits.get(ix)?;
                let is_selected = selected == Some(ix) || self.extra_selection.contains(&commit.hash);
                let time = match revisions.get(&commit.hash) {
                    Some(at) if settings.history_commit_time => at.commit_time,
                    _ => commit.author_time,
                };
                let labels = refs.for_commit(&commit.hash);
                let tag_names: Vec<String> = labels.iter().filter(|r| r.kind == RefKind::Tag).map(|r| r.name.clone()).collect();
                let branch_names: Vec<String> = labels.iter().filter(|r| r.kind != RefKind::Tag).map(|r| r.name.clone()).collect();
                let label_icon = |id: &str, icon: icons::AsIcon, names: Vec<String>, color: gpui_kit::Hsla| {
                    let tip: SharedString = names.join(", ").into();
                    div()
                        .id(SharedString::from(format!("history-{id}-{}", commit.hash)))
                        .flex_shrink_0()
                        .child(Icon::new(icon).small().text_color(color))
                        .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx))
                };
                let hash = commit.hash.clone();
                let menu_commit = commit.clone();
                let menu_entity = entity.clone();
                Some(
                    h_flex()
                        .id(SharedString::from(format!("history-{}", commit.hash)))
                        .h(px(row_height()))
                        .w_full()
                        .px_2()
                        .gap_4()
                        .text_sm()
                        .when(is_selected, |el| el.bg(if focused { palette.selection } else { palette.selection_inactive }))
                        .when(!is_selected, |el| el.hover(|s| s.bg(palette.hover)))
                        .on_mouse_down(gpui_kit::MouseButton::Left, {
                            let entity = entity.clone();
                            move |event: &gpui_kit::MouseDownEvent, window, cx| {
                                let (toggle, range) = (event.modifiers.secondary(), event.modifiers.shift);
                                let double = event.click_count == 2;
                                entity.update(cx, |this, cx| {
                                    window.focus(&this.focus, cx);
                                    this.click_row(ix, toggle, range, cx);
                                    if double {
                                        this.history_show_diff(false, cx);
                                    }
                                });
                            }
                        })
                        .on_mouse_down(gpui_kit::MouseButton::Right, {
                            let entity = entity.clone();
                            move |_, _, cx| {
                                let hash = hash.clone();
                                entity.update(cx, |this, cx| {
                                    let in_selection = this.extra_selection.contains(&hash) || this.model.read(cx).selected_hash() == Some(hash.as_str());
                                    if !in_selection {
                                        this.extra_selection.clear();
                                        this.model.update(cx, |model, cx| model.select_hash(Some(hash), cx));
                                    }
                                });
                            }
                        })
                        .context_menu(move |menu, window, cx| history_menu(menu, &menu_entity, &menu_commit, window, cx))
                        .when(settings.history_author, |el| {
                            el.child(div().w(px(author_width)).flex_shrink_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(commit.author_name.to_string()))
                        })
                        .when(settings.history_date, |el| {
                            // A fixed column: "Yesterday 22:20" and "2024/10/12 13:39" line up.
                            el.child(div().w(px(112.)).flex_shrink_0().overflow_hidden().whitespace_nowrap().child(history_date(time)))
                        })
                        .when(settings.history_hash, |el| {
                            el.child(div().flex_shrink_0().font_family(mono.clone()).text_color(palette.text_secondary).child(commit.short_hash().to_owned()))
                        })
                        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(commit.subject.clone()))
                        .when(!branch_names.is_empty(), |el| el.child(label_icon("branches", icons::BRANCH, branch_names, palette.ref_local)))
                        .when(!tag_names.is_empty(), |el| el.child(label_icon("tags", icons::BRANCH_LABEL, tag_names, palette.text_secondary)))
                        .into_any_element(),
                )
            })
            .collect()
    }
}

/// File History dates: "Today 13:39", "Yesterday 09:15" or "2024/6/24 13:39".
fn history_date(unix_seconds: i64) -> String {
    use chrono::{Local, TimeZone as _};
    let Some(time) = Local.timestamp_opt(unix_seconds, 0).single() else { return String::new() };
    let today = Local::now().date_naive();
    match time.date_naive() {
        date if date == today => format!("Today {}", time.format("%H:%M")),
        date if Some(date) == today.pred_opt() => format!("Yesterday {}", time.format("%H:%M")),
        _ => time.format("%Y/%-m/%-d %H:%M").to_string(),
    }
}

/// The History's context menu, IntelliJ's Vcs.FileHistory.ContextMenu.
fn history_menu(
    menu: gpui_kit::component::menu::PopupMenu,
    entity: &Entity<LogView>,
    commit: &Commit,
    _: &mut Window,
    cx: &mut App,
) -> gpui_kit::component::menu::PopupMenu {
    let this = entity.read(cx);
    let model = this.model.clone();
    let selected = this.selected_commits(cx);
    let multi = selected.len() > 1;
    let is_dir = this.history.as_ref().is_some_and(|h| h.is_dir);
    let repository = this.history.as_ref().and_then(|h| h.repository.clone());
    let own = this.history.as_ref().map(|h| h.own.clone()).unwrap_or_default();
    let hash = commit.hash.clone();
    let (path, _) = this.history_path_at(&hash).unwrap_or_else(|| (own.clone(), None));
    let copy_text = selected.iter().rev().map(|c| c.hash.clone()).collect::<Vec<_>>().join("\n");
    let no_local = !repository.as_ref().is_some_and(|r| r.root().join(&own).exists());
    let web = model.read(cx).web_repo().cloned();
    let oldest = selected.first().map(|c| c.hash.clone()).unwrap_or_else(|| hash.clone());
    let newest = selected.last().map(|c| c.hash.clone()).unwrap_or_else(|| hash.clone());
    let patch_paths: Vec<String> = {
        let mut paths: Vec<String> = selected.iter().filter_map(|c| this.history_path_at(&c.hash)).map(|(p, _)| p).collect();
        paths.dedup();
        if is_dir { vec![own.clone()] } else { paths }
    };
    let emit = |event: fn(String, String) -> LogEvent, path: String, hash: String| {
        let entity = entity.clone();
        move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
            let event = event(path.clone(), hash.clone());
            entity.update(cx, |_, cx| cx.emit(event))
        }
    };
    menu.item(
        PopupMenuItem::new("Copy Revision Number")
            .action(Box::new(CopyRevision))
            .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()))),
    )
    .item(PopupMenuItem::new("Create Patch…").on_click({
        let model = model.clone();
        move |_, window, cx| {
            let Some(repository) = model.read(cx).repository().cloned() else { return };
            let old = crate::git::patch::parent_of(&repository, &oldest);
            let source = crate::ui::patch_dialogs::PatchSource::Between { old, new: Some(newest.clone()), paths: patch_paths.clone() };
            crate::ui::patch_dialogs::create_patch(model.clone(), source, window, cx)
        }
    }))
    .separator()
    .when(!is_dir, |menu| {
        menu.item(PopupMenuItem::new("Get").disabled(multi).on_click({
            let (model, repository, path, hash, own) = (model.clone(), repository.clone(), path.clone(), hash.clone(), own.clone());
            move |_, window, cx| {
                let Some(repository) = repository.clone() else { return };
                let get = {
                    let (model, repository) = (model.clone(), repository.clone());
                    let (path, hash, own) = (path.clone(), hash.clone(), own.clone());
                    move |cx: &mut App| {
                        let (path, hash, own) = (path.clone(), hash.clone(), own.clone());
                        model.update(cx, |model, cx| {
                            model.run_operation_in(repository.clone(), "Get", move |repo| {
                                let content = repo.run_bytes(["show", &format!("{hash}:{path}")])?;
                                std::fs::write(repo.root().join(&own), content)?;
                                Ok(format!("{own} set to revision {}", &hash[..hash.len().min(8)]))
                            }, cx)
                        })
                    }
                };
                // Local changes would be lost: IntelliJ asks first.
                let modified = repository.run(["status", "--porcelain", "--", own.as_str()]).is_ok_and(|s| !s.trim().is_empty());
                if modified {
                    dialogs::confirm("Get Revision", format!("{own} has local changes. Replace them with revision {}?", &hash[..hash.len().min(8)]), "Get", get, window, cx);
                } else {
                    get(cx);
                }
            }
        }))
        .item(PopupMenuItem::new("Open Repository Version").disabled(multi).on_click(emit(|path, hash| LogEvent::OpenFile { path, revision: Some(hash) }, path.clone(), hash.clone())))
        .item(PopupMenuItem::new("Compare with Local").disabled(multi || no_local).on_click({
            let (entity, path, hash, own) = (entity.clone(), path.clone(), hash.clone(), own.clone());
            move |_, _, cx| {
                let source = DiffSource::Between { old: hash.clone(), new: None, path: own.clone(), old_path: (path != own).then(|| path.clone()) };
                entity.update(cx, |_, cx| cx.emit(LogEvent::OpenDiff(source)))
            }
        }))
        .item(PopupMenuItem::new("Show Diff").action(Box::new(ShowDiff)).on_click({
            let entity = entity.clone();
            move |_, _, cx| entity.update(cx, |this, cx| this.history_show_diff(false, cx))
        }))
        .item(PopupMenuItem::new("Show Diff in a New Tab").on_click({
            let entity = entity.clone();
            move |_, _, cx| entity.update(cx, |this, cx| this.history_show_diff(true, cx))
        }))
    })
    .item(PopupMenuItem::new("Show All Affected Files").disabled(multi).on_click({
        let (entity, hash) = (entity.clone(), hash.clone());
        move |_, window, cx| entity.update(cx, |this, cx| this.show_affected(hash.clone(), window, cx))
    }))
    .when(!is_dir, |menu| {
        menu.item(PopupMenuItem::new("Annotate Revision").disabled(multi).on_click(emit(|path, hash| LogEvent::Annotate { path, revision: Some(hash) }, path.clone(), hash.clone())))
    })
    .item(PopupMenuItem::new("Show in Git Log").disabled(multi).on_click({
        let (entity, hash) = (entity.clone(), hash.clone());
        move |_, _, cx| entity.update(cx, |_, cx| cx.emit(LogEvent::ShowInLog(hash.clone())))
    }))
    .when_some(web.filter(|_| !multi), |menu, web| {
        let url = if is_dir { web.commit_url(&hash) } else { web.file_url(&hash, &path, None) };
        menu.separator().item(PopupMenuItem::new(format!("Open on {}", web.host.name())).on_click(move |_, _, cx| cx.open_url(&url)))
    })
}
