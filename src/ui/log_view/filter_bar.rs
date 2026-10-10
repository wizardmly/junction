//! The Log toolbar's filters: text, Branch, User, Date, Paths, and the
//! compare banner.

use super::*;

impl LogView {
    /// View and Reset Filters: every filter off, then the commit selected.
    pub(super) fn reset_filters_and_select(&mut self, hash: String, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |state, cx| state.set_value("", window, cx));
        self.select_after_reload = Some(hash);
        self.update_filter(cx, |f| *f = LogFilter { date_order: f.date_order, ..Default::default() });
    }

    pub(super) fn apply_text_filter(&mut self, cx: &mut Context<Self>) {
        let text = self.search.read(cx).value().to_string();
        let mut filter = self.model.read(cx).filter().clone();
        filter.text = text;
        self.model.update(cx, |model, cx| model.set_filter(filter, cx));
    }

    pub(super) fn update_filter(&mut self, cx: &mut Context<Self>, edit: impl FnOnce(&mut LogFilter)) {
        let mut filter = self.model.read(cx).filter().clone();
        edit(&mut filter);
        // Filter history: each filter remembers its last few values.
        fn remember<T: PartialEq + Clone>(list: &mut Vec<T>, value: &T) {
            list.retain(|v| v != value);
            list.insert(0, value.clone());
            list.truncate(5);
        }
        if filter.branches.len() > 1 || filter.branches.first().is_some_and(|b| b != "HEAD") {
            remember(&mut self.recent_branch_filters, &filter.branches);
        }
        if !filter.authors.is_empty() {
            remember(&mut self.recent_user_filters, &filter.authors);
        }
        if !filter.paths.is_empty() && filter.lines.is_none() {
            remember(&mut self.recent_path_filters, &filter.paths);
        }
        self.model.update(cx, |model, cx| model.set_filter(filter, cx));
    }

    /// Branch › Select…: several branches at once.
    pub(super) fn select_branches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let refs = self.model.read(cx).refs().clone();
        let names: Vec<String> = refs.local_branches().chain(refs.remote_branches()).chain(refs.tags()).map(|r| r.name.clone()).collect();
        let checked: Rc<std::cell::RefCell<HashSet<String>>> =
            Rc::new(std::cell::RefCell::new(self.model.read(cx).filter().branches.iter().cloned().collect()));
        let entity = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let mut list = v_flex().gap_1().max_h(px(360.)).overflow_y_scrollbar().id("select-branches");
            for (ix, name) in names.iter().enumerate() {
                let (state, name_c) = (checked.clone(), name.clone());
                list = list.child(
                    gpui_kit::component::checkbox::Checkbox::new(SharedString::from(format!("sel-branch-{ix}")))
                        .label(name.clone())
                        .checked(checked.borrow().contains(name))
                        .on_change(move |v, window, _| {
                            if *v {
                                state.borrow_mut().insert(name_c.clone());
                            } else {
                                state.borrow_mut().remove(&name_c);
                            }
                            window.refresh();
                        }),
                );
            }
            let (checked, entity, names) = (checked.clone(), entity.clone(), names.clone());
            dialog
                .title("Select Branches")
                .w(px(380.))
                .child(list)
                .footer(dialogs::footer("OK"))
                .on_ok(move |_, _, cx| {
                    let chosen = checked.borrow();
                    let branches: Vec<String> = names.iter().filter(|n| chosen.contains(*n)).cloned().collect();
                    entity.update(cx, |this, cx| this.update_filter(cx, |f| f.branches = branches));
                    true
                })
        });
    }

    /// User › Select…: several users, checked from the log's authors or
    /// typed (names or emails, comma-separated), as IntelliJ's dialog.
    pub(super) fn select_users(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let names: Vec<String> = self.known_authors.1.iter().cloned().collect();
        let current = self.model.read(cx).filter().authors.clone();
        let checked: Rc<std::cell::RefCell<HashSet<String>>> =
            Rc::new(std::cell::RefCell::new(current.iter().filter(|a| names.contains(a)).cloned().collect()));
        let others: Vec<String> = current.iter().filter(|a| !names.contains(a)).cloned().collect();
        let typed = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Other users: name or email, comma-separated").default_value(others.join(", "))
        });
        let entity = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let mut list = v_flex().gap_1().max_h(px(320.)).overflow_y_scrollbar().id("select-users");
            for (ix, name) in names.iter().enumerate() {
                let (state, name_c) = (checked.clone(), name.clone());
                list = list.child(
                    gpui_kit::component::checkbox::Checkbox::new(SharedString::from(format!("sel-user-{ix}")))
                        .label(name.clone())
                        .checked(checked.borrow().contains(name))
                        .on_change(move |v, window, _| {
                            if *v {
                                state.borrow_mut().insert(name_c.clone());
                            } else {
                                state.borrow_mut().remove(&name_c);
                            }
                            window.refresh();
                        }),
                );
            }
            let (checked, entity, names, typed_ok) = (checked.clone(), entity.clone(), names.clone(), typed.clone());
            dialog
                .title("Select Users")
                .w(px(380.))
                .child(v_flex().gap_2().child(list).child(Input::new(&typed).small()))
                .footer(dialogs::footer("OK"))
                .on_ok(move |_, _, cx| {
                    let chosen = checked.borrow();
                    let mut authors: Vec<String> = names.iter().filter(|n| chosen.contains(*n)).cloned().collect();
                    let text = typed_ok.read(cx).value().to_string();
                    authors.extend(text.split(',').map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned));
                    authors.dedup();
                    entity.update(cx, |this, cx| this.update_filter(cx, |f| f.authors = authors));
                    true
                })
        });
    }

    /// Compare with Current shows `current..branch`; this says what the list
    /// means and offers Swap Branches, as IntelliJ's compare tab does.
    pub(super) fn render_compare_banner(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let filter = self.model.read(cx).filter().clone();
        let [range] = filter.branches.as_slice() else { return None };
        if range.contains("...") {
            return None;
        }
        let (base, branch) = range.split_once("..")?;
        let is_hash = |r: &str| r.len() == 40 && r.bytes().all(|b| b.is_ascii_hexdigit());
        let updated = is_hash(base) && is_hash(branch);
        let (diff_base, diff_branch) = (base.to_owned(), branch.to_owned());
        let shorten = |r: &str| if is_hash(r) { r[..8].to_owned() } else { r.to_owned() };
        let (base, branch) = (shorten(base), shorten(branch));
        let palette = cx.palette().clone();
        let count = self.model.read(cx).commits().len();
        let swapped = format!("{diff_branch}..{diff_base}");
        Some(
            h_flex()
                .h(px(28.))
                .px_2()
                .gap_2()
                .text_sm()
                .bg(palette.diff_header)
                .border_b_1()
                .border_color(palette.border)
                .child(Icon::new(IconName::GitCompare).small().text_color(palette.text_secondary))
                .child(div().child(match count {
                    // Update Project's "View Commits".
                    n if updated => format!("{n} commit{} received by Update Project ({base}..{branch})", if n == 1 { "" } else { "s" }),
                    0 => format!("'{branch}' has no commits that '{base}' doesn't have"),
                    n => format!("{n} commit{} in '{branch}' that {} not in '{base}'", if n == 1 { "" } else { "s" }, if n == 1 { "is" } else { "are" }),
                }))
                .child(div().flex_1())
                .when(!updated, |el| el.child(Button::new("compare-swap").xsmall().ghost().label("Swap Branches").on_click(cx.listener(move |this, _, _, cx| {
                    let swapped = swapped.clone();
                    this.update_filter(cx, |f| f.branches = vec![swapped]);
                }))))
                .child(Button::new("compare-files").xsmall().ghost().label("Show Files").on_click(cx.listener(move |this, _, _, cx| {
                    let (old, new) = (diff_base.clone(), diff_branch.clone());
                    this.model.update(cx, |m, cx| m.compare(old, Some(new), cx));
                })))
                .child(
                    tool_button("compare-close", IconName::Close, "Close Comparison")
                        .on_click(cx.listener(|this, _, _, cx| this.update_filter(cx, |f| f.branches.clear()))),
                ),
        )
    }

    pub(super) fn render_filter_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let model = self.model.read(cx);
        let filter = model.filter().clone();
        let refs = model.refs().clone();
        let user_email = model.user_email().map(str::to_owned);
        let authors: Vec<String> = self.known_authors.1.iter().take(40).cloned().collect();

        let branch_label = match filter.branches.as_slice() {
            [] => "Branch".to_owned(),
            [one] => format!("Branch: {one}"),
            many => format!("Branch: {} selected", many.len()),
        };
        // My email shows as "me", as IntelliJ labels it.
        let user_name = |a: &String| if Some(a) == user_email.as_ref() { "me".to_owned() } else { a.clone() };
        let user_label = match filter.authors.as_slice() {
            [] => "User".to_owned(),
            authors => format!("User: {}", authors.iter().map(user_name).collect::<Vec<_>>().join(", ")),
        };
        let date_label = match (&filter.since, &filter.until) {
            (Some(since), Some(until)) => format!("Date: {since} – {until}"),
            (Some(since), None) => format!("Date: since {since}"),
            (None, Some(until)) => format!("Date: until {until}"),
            (None, None) => "Date".to_owned(),
        };
        let entity = cx.entity();

        // Long values (several users, a date range, a path) are cut short
        // so the bar keeps its size, as IntelliJ elides its filter labels.
        let filter_button = |id: &'static str, label: String, active: bool| {
            let label = if label.chars().count() > 28 { format!("{}…", label.chars().take(27).collect::<String>()) } else { label };
            Button::new(id)
                .ghost()
                .xsmall()
                .label(label)
                .when(active, |b| b.selected(true))
                .icon(Icon::new(IconName::ChevronDown).xsmall())
        };

        h_flex()
            .h(px(crate::ui::common::toolbar_height()))
            .px_1()
            .gap_1()
            .overflow_hidden()
            .border_b_1()
            .border_color(palette.border)
            .child(
                div().w(px(260.)).min_w(px(120.)).flex_shrink(1.).child(
                    Input::new(&self.search)
                        .xsmall()
                        .cleanable(true)
                        .prefix(Icon::new(IconName::Search).xsmall().text_color(palette.text_secondary))
                        .suffix(
                            h_flex()
                                .gap_0p5()
                                .child(
                                    Button::new("search-case")
                                        .ghost()
                                        .xsmall()
                                        .label("Cc")
                                        .tooltip("Match Case")
                                        .selected(filter.match_case)
                                        .on_click(cx.listener(|this, _, _, cx| this.update_filter(cx, |f| f.match_case = !f.match_case))),
                                )
                                .child(
                                    Button::new("search-regex")
                                        .ghost()
                                        .xsmall()
                                        .label(".*")
                                        .tooltip("Regex")
                                        .selected(filter.regex)
                                        .on_click(cx.listener(|this, _, _, cx| this.update_filter(cx, |f| f.regex = !f.regex))),
                                ),
                        ),
                ),
            )
            .child(filter_button("filter-branch", branch_label, !filter.branches.is_empty()).dropdown_menu(self.branch_filter_menu(&entity, &refs, &filter)))
            .child(filter_button("filter-user", user_label, !filter.authors.is_empty()).dropdown_menu(self.user_filter_menu(&entity, &filter, user_email, authors)))
            .child(filter_button("filter-date", date_label, filter.since.is_some() || filter.until.is_some()).dropdown_menu(Self::date_filter_menu(&entity, &filter)))
            .child({
                let label = match filter.paths.as_slice() {
                    [] => "Paths".to_owned(),
                    [one] => format!("Path: {one}"),
                    many => format!("Paths: {}", many.len()),
                };
                let entity = entity.clone();
                let current = filter.paths.clone();
                let recent_paths = self.recent_path_filters.clone();
                filter_button("filter-path", label, !filter.paths.is_empty()).dropdown_menu(move |mut menu, _, _| {
                    let clear = entity.clone();
                    menu = menu.item(PopupMenuItem::new("All").checked(current.is_empty()).on_click(move |_, _, cx| {
                        clear.update(cx, |this, cx| this.update_filter(cx, |f| f.paths.clear()))
                    }));
                    let pick = entity.clone();
                    menu = menu.item(PopupMenuItem::new("Select Folders…").on_click(move |_, window, cx| {
                        pick.update(cx, |this, cx| this.select_paths(window, cx))
                    }));
                    if !recent_paths.is_empty() {
                        menu = menu.separator().label("Recent");
                        for paths in &recent_paths {
                            let (entity, paths_c) = (entity.clone(), paths.clone());
                            menu = menu.item(PopupMenuItem::new(paths.join(", ")).checked(&current == paths).on_click(move |_, _, cx| {
                                let paths = paths_c.clone();
                                entity.update(cx, |this, cx| this.update_filter(cx, |f| f.paths = paths))
                            }));
                        }
                    }
                    menu
                })
            })
            .child(div().flex_1())
            .when(model.is_loading(), |el| {
                el.child(div().text_xs().text_color(palette.text_secondary).child("Loading…"))
            })
            .child(tool_button("log-refresh", IconName::RefreshCw, "Refresh").on_click(cx.listener(|this, _, _, cx| {
                this.model.update(cx, |model, cx| model.reload(cx));
            })))
            .child(
                tool_button("log-toggle-branches", IconName::PanelLeft, "Show Branches")
                    .selected(self.show_branches)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_branches = !this.show_branches;
                        cx.notify();
                    })),
            )
            .child(
                tool_button("log-toggle-details", IconName::Rows3, "Show Details")
                    .selected(self.show_details)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_details = !this.show_details;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("log-options")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::Eye))
                    .tooltip("View Options")
                    .dropdown_menu(self.view_options_menu(&entity, cx)),
            )
    }

    /// Branch filter: laid out as IntelliJ's.
    fn branch_filter_menu(&self, entity: &Entity<LogView>, refs: &std::sync::Arc<crate::git::RepositoryRefs>, filter: &LogFilter) -> impl Fn(gpui_kit::component::menu::PopupMenu, &mut Window, &mut Context<gpui_kit::component::menu::PopupMenu>) -> gpui_kit::component::menu::PopupMenu + 'static {
        let entity = entity.clone();
        let refs = refs.clone();
        let selected = filter.branches.clone();
        let recent_branches = self.recent_branch_filters.clone();
        // Laid out as IntelliJ's branch filter: Select…, Favorites,
        // recent filters, HEAD and the favorite refs at the top, every
        // branch in a Local submenu and one submenu per remote.
        move |mut menu, window, cx| {
            fn set(entity: &Entity<LogView>, branches: Vec<String>) -> impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static {
                let entity = entity.clone();
                move |_, _, cx| {
                    let branches = branches.clone();
                    entity.update(cx, |this, cx| this.update_filter(cx, |f| f.branches = branches));
                }
            }
            fn ref_row(selected: &[String], refs: &crate::git::refs::RepositoryRefs, branch: &crate::git::refs::RefName, label: &str) -> crate::ui::ref_menu::RefRow {
                crate::ui::ref_menu::RefRow {
                    label: label.to_owned().into(),
                    value: branch.name.clone(),
                    favorite: refs.favorites.contains(&branch.full_name),
                    checked: selected == [branch.name.clone()],
                }
            }
            fn pick(entity: &Entity<LogView>) -> std::rc::Rc<dyn Fn(String, &mut Window, &mut App)> {
                let entity = entity.clone();
                std::rc::Rc::new(move |name, _, cx| entity.update(cx, |this, cx| this.update_filter(cx, |f| f.branches = vec![name])))
            }
            fn branch_item(entity: &Entity<LogView>, selected: &[String], refs: &crate::git::refs::RepositoryRefs, branch: &crate::git::refs::RefName, label: &str) -> PopupMenuItem {
                let item = PopupMenuItem::new(label.to_owned())
                    .checked(selected == [branch.name.clone()])
                    .on_click(set(entity, vec![branch.name.clone()]));
                if refs.favorites.contains(&branch.full_name) { item.icon(IconName::Star) } else { item }
            }
            if !selected.is_empty() {
                menu = menu.item(PopupMenuItem::new("All").on_click(set(&entity, vec![])));
            }
            let select_entity = entity.clone();
            menu = menu.item(PopupMenuItem::new("Select…").on_click(move |_, window, cx| {
                select_entity.update(cx, |this, cx| this.select_branches(window, cx))
            }));
            let favorites: Vec<&crate::git::refs::RefName> = refs.refs.iter().filter(|r| refs.favorites.contains(&r.full_name)).collect();
            let favorite_names: Vec<String> = favorites.iter().map(|r| r.name.clone()).collect();
            if favorites.len() > 1 {
                menu = menu.item(PopupMenuItem::new("Favorites").checked(selected == favorite_names).on_click(set(&entity, favorite_names.clone())));
            }
            for recent in &recent_branches {
                // Single branches already have their own row below.
                if recent.len() == 1 && (recent[0] == "HEAD" || favorite_names.contains(&recent[0])) || *recent == favorite_names {
                    continue;
                }
                menu = menu.item(PopupMenuItem::new(recent.join(", ")).checked(&selected == recent).on_click(set(&entity, recent.clone())));
            }
            if refs.current_branch.is_some() || refs.head_commit.is_some() {
                menu = menu.item(
                    PopupMenuItem::new("HEAD").icon(IconName::Star).checked(selected == ["HEAD"]).on_click(set(&entity, vec!["HEAD".into()])),
                );
            }
            for branch in &favorites {
                menu = menu.item(branch_item(&entity, &selected, &refs, branch, &branch.name));
            }
            menu = menu.separator();
            if refs.local_branches().next().is_some() {
                let (entity, refs, selected) = (entity.clone(), refs.clone(), selected.clone());
                menu = menu.submenu("Local", window, cx, move |sub, _, cx| {
                    let rows = refs.local_branches().map(|b| ref_row(&selected, &refs, b, &b.name)).collect();
                    crate::ui::ref_menu::add_rows(sub, rows, pick(&entity), cx)
                });
            }
            for remote in &refs.remotes {
                if !refs.remote_branches().any(|b| b.remote() == Some(remote.as_str())) {
                    continue;
                }
                let (entity, refs, selected, remote) = (entity.clone(), refs.clone(), selected.clone(), remote.clone());
                menu = menu.submenu(format!("{remote}/..."), window, cx, move |sub, _, cx| {
                    let rows = refs
                        .remote_branches()
                        .filter(|b| b.remote() == Some(remote.as_str()))
                        .map(|b| ref_row(&selected, &refs, b, b.branch_without_remote()))
                        .collect();
                    crate::ui::ref_menu::add_rows(sub, rows, pick(&entity), cx)
                });
            }
            menu
        }
    }

    /// User filter: all, me, recent selections and known authors.
    fn user_filter_menu(&self, entity: &Entity<LogView>, filter: &LogFilter, user_email: Option<String>, authors: Vec<String>) -> impl Fn(gpui_kit::component::menu::PopupMenu, &mut Window, &mut Context<gpui_kit::component::menu::PopupMenu>) -> gpui_kit::component::menu::PopupMenu + 'static {
        let entity = entity.clone();
        let current = filter.authors.clone();
        let recent_users = self.recent_user_filters.clone();
        move |mut menu, _, _| {
            let set = |authors: Vec<String>| {
                let entity = entity.clone();
                move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
                    let authors = authors.clone();
                    entity.update(cx, |this, cx| this.update_filter(cx, |f| f.authors = authors));
                }
            };
            menu = menu.item(PopupMenuItem::new("All").checked(current.is_empty()).on_click(set(Vec::new())));
            if let Some(email) = &user_email {
                menu = menu.item(PopupMenuItem::new("me").checked(current == [email.clone()]).on_click(set(vec![email.clone()])));
            }
            let select = entity.clone();
            menu = menu.item(PopupMenuItem::new("Select…").on_click(move |_, window, cx| {
                select.update(cx, |this, cx| this.select_users(window, cx))
            }));
            let recent: Vec<&Vec<String>> =
                recent_users.iter().filter(|u| user_email.as_ref().is_none_or(|me| **u != [me.clone()])).collect();
            if !recent.is_empty() {
                menu = menu.separator().label("Recent");
                for authors in recent {
                    let label = authors
                        .iter()
                        .map(|a| if Some(a) == user_email.as_ref() { "me" } else { a.as_str() })
                        .collect::<Vec<_>>()
                        .join(", ");
                    menu = menu.item(PopupMenuItem::new(label).checked(&current == authors).on_click(set(authors.clone())));
                }
            }
            menu = menu.separator();
            for author in &authors {
                menu = menu.item(
                    PopupMenuItem::new(author.clone())
                        .checked(current == [author.clone()])
                        .on_click(set(vec![author.clone()])),
                );
            }
            menu.max_h(px(420.))
        }
    }

    /// Date filter: preset ranges and Select….
    fn date_filter_menu(entity: &Entity<LogView>, filter: &LogFilter) -> impl Fn(gpui_kit::component::menu::PopupMenu, &mut Window, &mut Context<gpui_kit::component::menu::PopupMenu>) -> gpui_kit::component::menu::PopupMenu + 'static {
        let entity = entity.clone();
        let current = filter.since.clone();
        move |mut menu, _, _| {
            for (label, since) in [
                ("All", None),
                ("Last 24 hours", Some("24 hours ago")),
                ("Last 7 days", Some("7 days ago")),
                ("Last 30 days", Some("30 days ago")),
                ("Last year", Some("1 year ago")),
            ] {
                let entity = entity.clone();
                let since = since.map(str::to_owned);
                menu = menu.item(PopupMenuItem::new(label).checked(current == since).on_click(move |_, _, cx| {
                    let since = since.clone();
                    entity.update(cx, |this, cx| this.update_filter(cx, |f| {
                        f.since = since;
                        f.until = None;
                    }));
                }));
            }
            let entity = entity.clone();
            menu.separator().item(PopupMenuItem::new("Select…").on_click(move |_, window, cx| {
                entity.update(cx, |this, cx| this.select_date_range(window, cx))
            }))
        }
    }

    /// View Options (eye): collapse, sorting, highlights, references, columns.
    fn view_options_menu(&self, entity: &Entity<LogView>, cx: &App) -> impl Fn(gpui_kit::component::menu::PopupMenu, &mut Window, &mut Context<gpui_kit::component::menu::PopupMenu>) -> gpui_kit::component::menu::PopupMenu + 'static {
        let entity = entity.clone();
        let log = Settings::get(cx).log.clone();
        let collapse = self.model.read(cx).collapse_linear();
        move |menu, _, _| {
            let collapse_entity = entity.clone();
            // Each toggle flips one View Options flag and reloads when it changes the order.
            let toggle = |label: &'static str, on: bool, set: fn(&mut crate::settings::LogSettings, bool), reload: bool| {
                let entity = entity.clone();
                PopupMenuItem::new(label).checked(on).on_click(move |_, _, cx| {
                    Settings::update(cx, |s| set(&mut s.log, !on));
                    entity.update(cx, |this, cx| {
                        if reload {
                            this.model.update(cx, |model, cx| model.reload(cx));
                        }
                        cx.notify();
                    });
                })
            };
            menu.item(PopupMenuItem::new("Collapse Linear Branches").checked(collapse).on_click(
                move |_, _, cx| {
                    collapse_entity.update(cx, |this, cx| {
                        this.model.update(cx, |model, cx| model.set_collapse_linear(!collapse, cx));
                    })
                },
            ))
            .item(toggle("Show Long Edges", log.show_long_edges, |l, v| l.show_long_edges = v, false))
            .separator()
            .label("Sort")
            .item(toggle("IntelliSort", !log.sort_by_date, |l, _| l.sort_by_date = false, true))
            .item(toggle("By Date", log.sort_by_date, |l, _| l.sort_by_date = true, true))
            .separator()
            .label("Highlight")
            .item(toggle("My Commits", log.highlight_mine, |l, v| l.highlight_mine = v, false))
            .item(toggle("Merge Commits", log.highlight_merges, |l, v| l.highlight_merges = v, false))
            .item(toggle("Current Branch", log.highlight_current_branch, |l, v| l.highlight_current_branch = v, false))
            .item(toggle("Not Merged into Current Branch", log.highlight_not_merged, |l, v| l.highlight_not_merged = v, false))
            .separator()
            .label("References")
            .item(toggle("Compact References View", log.compact_refs, |l, v| l.compact_refs = v, false))
            .item(toggle("Show References on the Left", log.refs_on_left, |l, v| l.refs_on_left = v, false))
            .separator()
            .label("Show Columns")
            .item(toggle("Author", log.show_author, |l, v| l.show_author = v, false))
            .item(toggle("Date", log.show_date, |l, v| l.show_date = v, false))
            .item(toggle("Hash", log.show_hash, |l, v| l.show_hash = v, false))
            .separator()
            .item(toggle("Relative Dates", log.relative_dates, |l, v| l.relative_dates = v, false))
        }
    }

    /// Paths › Select Folders…: the repository's folders and files as a
    /// tree to check, as IntelliJ's structure filter shows the project.
    pub(super) fn select_paths(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = self.model.read(cx).repository().cloned() else { return };
        let files: Vec<String> = repo.run(["ls-files"]).unwrap_or_default().lines().map(str::to_owned).collect();
        let tree_state = cx.new(|cx| TreeState::new(cx).items(common::file_tree_with(files, "", false)));
        let checked: Rc<std::cell::RefCell<HashSet<String>>> =
            Rc::new(std::cell::RefCell::new(self.model.read(cx).filter().paths.iter().cloned().collect()));
        let entity = cx.entity();
        let palette = cx.palette().clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let rows_checked = checked.clone();
            let palette = palette.clone();
            let list = tree(&tree_state, move |ix, entry, _, _, _| {
                let id = entry.item().id.to_string();
                let path = id.strip_prefix(DIR_PREFIX).or_else(|| id.strip_prefix(FILE_PREFIX)).unwrap_or_default().to_owned();
                let set = rows_checked.borrow();
                // A path under a checked folder is part of the filter too.
                let inherited = set.iter().any(|p| path.starts_with(&format!("{p}/")));
                let state = rows_checked.clone();
                let toggle_path = path.clone();
                ListItem::new(ix).py_0().px_1().h(px(row_height())).child(
                    h_flex()
                        .w_full()
                        .gap_1()
                        .pl(px(entry.depth() as f32 * 14.))
                        .text_sm()
                        .child(if entry.is_folder() {
                            Icon::new(if entry.is_expanded() { IconName::ChevronDown } else { IconName::ChevronRight })
                                .xsmall()
                                .text_color(palette.text_secondary)
                        } else {
                            Icon::new(IconName::Circle).xsmall().text_color(gpui_kit::transparent_black())
                        })
                        .child(
                            div().on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(
                                gpui_kit::component::checkbox::Checkbox::new(SharedString::from(format!("path-{ix}")))
                                    .checked(inherited || set.contains(&path))
                                    .disabled(inherited)
                                    .on_change(move |v, window, _| {
                                        let mut set = state.borrow_mut();
                                        if *v {
                                            set.retain(|p| !p.starts_with(&format!("{toggle_path}/")));
                                            set.insert(toggle_path.clone());
                                        } else {
                                            set.remove(&toggle_path);
                                        }
                                        window.refresh();
                                    }),
                            ),
                        )
                        .child(
                            Icon::new(if entry.is_folder() { IconName::Folder } else { common::file_icon(&path) })
                                .small()
                                .text_color(palette.text_secondary),
                        )
                        .child(entry.item().label.clone()),
                )
            });
            let (checked, entity) = (checked.clone(), entity.clone());
            dialog
                .title("Select Folders and Files")
                .w(px(460.))
                .child(div().h(px(380.)).border_1().border_color(palette.border).child(list.size_full()))
                .footer(dialogs::footer("OK"))
                .on_ok(move |_, _, cx| {
                    let mut paths: Vec<String> = checked.borrow().iter().cloned().collect();
                    paths.sort();
                    entity.update(cx, |this, cx| this.update_filter(cx, |f| f.paths = paths));
                    true
                })
        });
    }

    /// Date › Select…: a from / to range (`--since` / `--until`).
    pub(super) fn select_date_range(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let filter = self.model.read(cx).filter().clone();
        let from = cx.new(|cx| InputState::new(window, cx).placeholder("YYYY-MM-DD").default_value(filter.since.clone().unwrap_or_default()));
        let to = cx.new(|cx| InputState::new(window, cx).placeholder("YYYY-MM-DD").default_value(filter.until.clone().unwrap_or_default()));
        let entity = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let (from_ok, to_ok, entity) = (from.clone(), to.clone(), entity.clone());
            dialog
                .title("Select Period")
                .w(px(360.))
                .child(
                    v_flex()
                        .gap_2()
                        .child(h_flex().gap_2().child(div().w(px(40.)).text_sm().child("From:")).child(div().flex_1().child(Input::new(&from).small())))
                        .child(h_flex().gap_2().child(div().w(px(40.)).text_sm().child("To:")).child(div().flex_1().child(Input::new(&to).small()))),
                )
                .footer(dialogs::footer("OK"))
                .on_ok(move |_, _, cx| {
                    let value = |input: &Entity<InputState>, cx: &App| {
                        let v = input.read(cx).value().trim().to_owned();
                        (!v.is_empty()).then_some(v)
                    };
                    let (since, until) = (value(&from_ok, cx), value(&to_ok, cx));
                    entity.update(cx, |this, cx| this.update_filter(cx, |f| {
                        f.since = since;
                        f.until = until;
                    }));
                    true
                })
        });
    }
}
