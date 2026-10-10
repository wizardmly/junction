//! The commit details panel: changed files and the message.

use super::*;
use crate::ui::as_icons as icons;

impl LogView {
    pub(super) fn rebuild_changes(&mut self, cx: &mut Context<Self>) {
        let details = self.model.read(cx).details().cloned();
        self.change_kinds.clear();
        self.combined = None;
        // Several commits selected: IntelliJ shows their changes merged. Each
        // file compares its state before the oldest selected commit that
        // touched it with its state after the newest one.
        let selected = if self.extra_selection.is_empty() { Vec::new() } else { self.selected_commits(cx) };
        let combined = match self.model.read(cx).repository() {
            // Many commits (Ctrl+A): one diff from before the oldest to the
            // newest, rather than a git call per commit.
            Some(repo) if selected.len() > MAX_COMBINED => {
                let (oldest, newest) = (&selected[0], &selected[selected.len() - 1]);
                let old = if oldest.parents.is_empty() { EMPTY_TREE.to_owned() } else { format!("{}^", oldest.hash) };
                let mut changes = crate::git::diff::changed_files(repo, &old, Some(&newest.hash)).unwrap_or_default();
                changes.sort_by(|a, b| a.path.cmp(&b.path));
                let ranges = changes.iter().map(|c| (c.path.clone(), (old.clone(), newest.hash.clone()))).collect();
                Some((ranges, changes))
            }
            Some(repo) if selected.len() > 1 => {
                let mut merged: Vec<crate::git::log::FileChange> = Vec::new();
                let mut ranges: HashMap<String, (String, String)> = HashMap::new();
                for commit in &selected {
                    let parent = format!("{}^", commit.hash);
                    let old = if commit.parents.is_empty() { EMPTY_TREE.to_owned() } else { parent };
                    let Ok(changes) = crate::git::diff::changed_files(repo, &old, Some(&commit.hash)) else { continue };
                    for change in changes {
                        match ranges.get_mut(&change.path) {
                            Some(range) => range.1 = commit.hash.clone(),
                            None => {
                                ranges.insert(change.path.clone(), (old.clone(), commit.hash.clone()));
                                merged.push(change);
                            }
                        }
                    }
                }
                merged.sort_by(|a, b| a.path.cmp(&b.path));
                Some((ranges, merged))
            }
            _ => None,
        };
        let paths: Vec<String> = match (&combined, &details) {
            (Some((ranges, changes)), _) => {
                for change in changes {
                    self.change_kinds.insert(change.path.clone(), (change.kind, change.old_path.clone()));
                }
                self.combined = Some(ranges.clone());
                changes.iter().map(|c| c.path.clone()).collect()
            }
            (None, Some(details)) => {
                for change in &details.changes {
                    self.change_kinds.insert(change.path.clone(), (change.kind, change.old_path.clone()));
                }
                details.changes.iter().map(|c| c.path.clone()).collect()
            }
            (None, None) => Vec::new(),
        };
        // View Options › Group By: Directory and / or Module, else a flat list.
        let settings = crate::settings::Settings::get(cx).log.clone();
        let root = self.model.read(cx).repository().map(|r| (r.root().to_path_buf(), r.name()));
        let mut cache = HashMap::new();
        let modules: HashMap<String, String> = match (&root, settings.changes_by_module) {
            (Some((root, _)), true) => paths.iter().map(|p| (p.clone(), common::module_of(root, p, &mut cache))).collect(),
            _ => HashMap::new(),
        };
        let module_of = |path: &str| modules.get(path).cloned().unwrap_or_default();
        let root_name = root.map(|(_, name)| name).unwrap_or_default();
        let items = common::grouped_file_tree(
            paths,
            "",
            self.changes_expanded,
            settings.changes_by_directory,
            settings.changes_by_module.then_some((root_name.as_str(), &module_of as &dyn Fn(&str) -> String)),
            None,
        );
        self.change_counts.clear();
        common::count_files(&items, &mut self.change_counts);
        self.last_change_selection = None;
        self.changes.update(cx, |tree, cx| {
            tree.set_items(items, cx);
            tree.set_selected_index(None, cx);
        });
    }

    pub(super) fn diff_source_for(&self, id: &str, cx: &App) -> Option<DiffSource> {
        let path = id.strip_prefix(FILE_PREFIX)?;
        let old_path = self.change_kinds.get(path).and_then(|(_, old)| old.clone());
        if let Some(ranges) = &self.combined {
            let (old, new) = ranges.get(path)?.clone();
            return Some(DiffSource::Between { old, new: Some(new), path: path.to_owned(), old_path });
        }
        let hash = self.model.read(cx).selected_hash()?.to_owned();
        Some(DiffSource::Commit { hash, path: path.to_owned(), old_path })
    }

    pub(super) fn render_details(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let submodules = self.model.read(cx).submodule_paths().clone();
        let palette = cx.palette().clone();
        let model = self.model.read(cx);
        let details = model.details().cloned();
        let refs = model.log_refs().clone();
        // A multi-root project: which repository the commit is in.
        let root = model.project_root().filter(|_| model.is_multi_root()).and_then(|project| {
            let active = model.repository()?.root();
            let ix = model.root_states().iter().position(|r| r.path == active)?;
            Some((ix, crate::git::roots::label(project, active), active.display().to_string()))
        });
        let kinds = self.change_kinds.clone();
        let counts = self.change_counts.clone();
        let entity = cx.entity();

        let tree_palette = palette.clone();
        let changes = v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(px(crate::ui::common::toolbar_height()))
                    .px_2()
                    .border_b_1()
                    .border_color(palette.border)
                    .text_sm()
                    .text_color(palette.text_secondary)
                    .child(match &details {
                        _ if self.combined.is_some() => {
                            let n = self.combined.as_ref().map_or(0, |c| c.len());
                            format!("{} commits selected · {n} {} changed", self.extra_selection.len() + 1, if n == 1 { "file" } else { "files" })
                        }
                        Some(d) => format!("{} {} changed", d.changes.len(), if d.changes.len() == 1 { "file" } else { "files" }),
                        None => "No commit selected".into(),
                    })
                    .child(div().flex_1())
                    .child(tool_button("log-changes-expand", icons::EXPAND_ALL, "Expand All").on_click(cx.listener(|this, _, _, cx| {
                        this.changes_expanded = true;
                        this.rebuild_changes(cx);
                        cx.notify();
                    })))
                    .child(tool_button("log-changes-collapse", icons::COLLAPSE_ALL, "Collapse All").on_click(cx.listener(|this, _, _, cx| {
                        this.changes_expanded = false;
                        this.rebuild_changes(cx);
                        cx.notify();
                    })))
                    .child({
                        let settings = Settings::get(cx).log.clone();
                        let entity = cx.entity();
                        Button::new("log-changes-options").ghost().xsmall().icon(icons::SHOW).tooltip("View Options").dropdown_menu(move |menu, _, _| {
                            let toggle = |label: &'static str, on: bool, set: fn(&mut crate::settings::LogSettings, bool)| {
                                let entity = entity.clone();
                                PopupMenuItem::new(label).checked(on).on_click(move |_, _, cx| {
                                    Settings::update(cx, |s| set(&mut s.log, !on));
                                    entity.update(cx, |this, cx| {
                                        this.rebuild_changes(cx);
                                        cx.notify();
                                    });
                                })
                            };
                            menu.label("Group By")
                                .item(toggle("Directory", settings.changes_by_directory, |s, v| s.changes_by_directory = v))
                                .item(toggle("Module", settings.changes_by_module, |s, v| s.changes_by_module = v))
                        })
                    }),
            )
            .child(
                div().flex_1().min_h_0().child(
                    tree(&self.changes, move |ix, entry, _, _, _| {
                        let palette = &tree_palette;
                        let item = entry.item();
                        let id = item.id.to_string();
                        let path = id.strip_prefix(FILE_PREFIX).map(str::to_owned);
                        let kind = path.as_ref().and_then(|p| kinds.get(p)).cloned();
                        let color = kind.as_ref().map_or(palette.text, |(k, _)| common::change_color(*k, &palette));
                        let entity = entity.clone();
                        let open_path = path.clone();
                        let (menu_entity, menu_path) = (entity.clone(), path.clone());
                        ListItem::new(ix)
                            .py_0()
                            .px_1()
                            .h(px(row_height()))

                            .on_click(move |event, _, cx| {
                                if event.click_count() == 2 {
                                    if let Some(path) = open_path.clone() {
                                        entity.update(cx, |this, cx| {
                                            if let Some(source) = this.diff_source_for(&format!("{FILE_PREFIX}{path}"), cx) {
                                                cx.emit(LogEvent::OpenDiff(source));
                                            }
                                        });
                                    }
                                }
                            })
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_1()
                                    .pl(px(entry.depth() as f32 * 14.))
                                    .text_sm()
                                    .when(entry.is_folder(), |el| {
                                        el.child(
                                            Icon::new(if entry.is_expanded() { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT })
                                                .xsmall()
                                                .text_color(palette.text_secondary),
                                        )
                                    })
                                    .child(
                                        Icon::new(match &path {
                                            Some(p) if submodules.contains(p.as_str()) => Icon::from(icons::MODULE),
                                            Some(p) => common::file_icon(p),
                                            None if id.starts_with(common::MODULE_PREFIX) => Icon::from(icons::MODULE),
                                            None => Icon::from(icons::FOLDER),
                                        })
                                        .small()
                                        .text_color(palette.text_secondary),
                                    )
                                    .child(div().text_color(color).child(item.label.clone()))
                                    .when_some(kind.and_then(|(_, old)| old), |el, old| {
                                        el.child(div().text_xs().text_color(palette.text_secondary).child(format!("from {old}")))
                                    })
                                    .when(id.contains(DIR_PREFIX) || id.starts_with(common::MODULE_PREFIX), |el| {
                                        let n = counts.get(&item.id).copied().unwrap_or(0);
                                        el.child(
                                            div()
                                                .text_xs()
                                                .text_color(palette.text_secondary)
                                                .child(format!("{n} {}", if n == 1 { "file" } else { "files" })),
                                        )
                                    })
                                    .context_menu(move |menu, _, cx| match &menu_path {
                                        Some(path) => {
                                            let hash = menu_entity.read(cx).model.read(cx).selected_hash().map(str::to_owned);
                                            match hash {
                                                Some(hash) => change_menu(menu, &menu_entity, path, &hash, cx),
                                                None => menu,
                                            }
                                        }
                                        None => menu,
                                    }),
                            )
                    })
                    .size_full(),
                ),
            );

        let entity_for_links = cx.entity();
        let info = div()
            .id("commit-details")
            .size_full()
            .p_3()
            .text_sm()
            .overflow_y_scrollbar()
            .when_some(details, |el, d| {
                let labels = refs.for_commit(&d.hash);
                el.child(
                    v_flex()
                        .gap_2()
                        // The text is selectable and copies (Ctrl+C), as IntelliJ's details pane.
                        .child(message_text(&d.message, palette.link, &entity_for_links))
                        .child(
                            h_flex()
                                .gap_1()
                                .flex_wrap()
                                .child(selectable("commit-hash", 1, &d.hash[..d.hash.len().min(10)], palette.link).font_family(Some(cx.theme().mono_font_family.clone())))
                                .child(selectable(
                                    "commit-author",
                                    2,
                                    &format!("{} <{}> on {}", d.author_name, d.author_email, common::format_full_date(d.author_time)),
                                    palette.text_secondary,
                                )),
                        )
                        .when(d.committer_email != d.author_email || d.committer_time != d.author_time, |el| {
                            el.child(selectable(
                                "commit-committer",
                                3,
                                &format!(
                                    "committed by {} <{}> on {}",
                                    d.committer_name,
                                    d.committer_email,
                                    common::format_full_date(d.committer_time)
                                ),
                                palette.text_secondary,
                            ))
                        })
                        .when_some(d.signature.clone(), |el, signature| {
                            let color = if signature.is_good() {
                                palette.status_added
                            } else if signature.status == 'B' {
                                palette.status_conflict
                            } else {
                                palette.text_secondary
                            };
                            el.child(
                                h_flex()
                                    .gap_1()
                                    .text_color(color)
                                    .items_start()
                                    .child(Icon::new(if signature.is_good() { icons::STATUS_SUCCESS } else { icons::STATUS_WARNING }).small())
                                    .child(div().flex_1().min_w_0().child(selectable("commit-signature", 4, &signature.describe(), color))),
                            )
                        })
                        .when(!labels.is_empty(), |el| {
                            let mut row = h_flex().gap_1().flex_wrap();
                            for label in labels {
                                row = row.child(ref_label(label, refs.current_branch.as_deref(), &palette));
                            }
                            el.child(row)
                        })
                        .when_some(root.clone(), |el, (ix, name, path)| {
                            el.child(
                                h_flex()
                                    .gap_1p5()
                                    .text_color(palette.text_secondary)
                                    .child(div().size(px(9.)).rounded_sm().bg(common::root_color(ix)))
                                    .child(selectable("commit-root", 6, &format!("Root: {name}  ({path})"), palette.text_secondary)),
                            )
                        })
                        .when(d.complete, |el| el.child({
                            let n = d.containing_branches.len();
                            let all = self.show_all_branches || n <= 5;
                            let shown = if all { &d.containing_branches[..] } else { &d.containing_branches[..5] };
                            let text = match n {
                                0 => "Not in any branch".to_owned(),
                                1 => format!("In 1 branch: {}", shown[0]),
                                n => format!("In {n} branches: {}", shown.join(", ")),
                            };
                            // More than five: "Show all" lists the rest, as in IntelliJ.
                            v_flex().child(selectable("commit-branches", 5, &text, palette.text_secondary)).when(!all, |el| {
                                el.child(
                                    div()
                                        .id("commit-branches-all")
                                        .text_color(palette.link)
                                        .cursor_pointer()
                                        .child(format!("Show all {n}"))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.show_all_branches = true;
                                            cx.notify();
                                        })),
                                )
                            })
                        })),
                )
            });

        v_resizable("log-details-split")
            // Text measures itself unwrapped; out of the flow (absolute), a
            // long message or path stays inside the pane instead of widening
            // the window.
            .child(resizable_panel().child(div().relative().size_full().child(div().absolute().inset_0().child(changes))))
            .child(resizable_panel().size(px(220.)).child(div().relative().size_full().child(div().absolute().inset_0().child(info))))
    }
}

/// The commit message as selectable text: URLs and hash-like words are
/// links (a hash goes to that commit in the Log), as in IntelliJ.
pub(super) fn message_text(message: &str, link_color: gpui_kit::Hsla, entity: &Entity<LogView>) -> impl IntoElement {
    let (ranges, links): (Vec<_>, Vec<_>) = common::find_links(message).into_iter().unzip();
    let entity = entity.clone();
    selectable_text("commit-message", message.to_owned()).order(0).links(ranges, link_color, move |ix, window, cx| {
        match &links[ix] {
            common::Link::Url(url) => cx.open_url(url),
            common::Link::Commit(hash) => {
                let hash = hash.clone();
                entity.update(cx, |this, cx| this.go_to(&hash, window, cx));
            }
        }
    })
}

/// One line of the details pane; `order` places it for a drag across lines.
pub(super) fn selectable(id: &'static str, order: u64, text: &str, color: gpui_kit::Hsla) -> SelectableText {
    selectable_text(id, text.to_owned()).order(order).color(color)
}

/// Right-click on a file in the commit details, after IntelliJ's.
pub(super) fn change_menu(
    menu: gpui_kit::component::menu::PopupMenu,
    entity: &Entity<LogView>,
    path: &str,
    hash: &str,
    cx: &mut App,
) -> gpui_kit::component::menu::PopupMenu {
    let model = entity.read(cx).model.clone();
    let hash = hash.to_owned();
    let web_file = model.read(cx).web_repo().map(|w| (w.host.name(), w.file_url(&hash, path, None)));
    let short = hash[..hash.len().min(8)].to_owned();
    let path = path.to_owned();
    // What the commit did to the file: nothing to compare on a side it lacks.
    let (kind, old_path) = entity.read(cx).change_kinds.get(&path).cloned().unzip();
    let old_path = old_path.flatten();
    let added = kind == Some(FileChangeKind::Added);
    let no_local = !entity.read(cx).model.read(cx).repository().is_some_and(|r| r.root().join(&path).exists());
    let (e_diff, e_blame, e_history, e_here) = (entity.clone(), entity.clone(), entity.clone(), entity.clone());
    let (p_diff, p_blame, p_history, p_here, p_copy) = (path.clone(), path.clone(), path.clone(), path.clone(), path.clone());
    let (h_blame, h_here) = (hash.clone(), hash.clone());
    let (m_get, m_revert) = (model.clone(), model.clone());
    let (p_get, h_get, p_revert, h_revert) = (path.clone(), hash.clone(), path.clone(), hash.clone());
    menu.item(PopupMenuItem::new("Show Diff").on_click(move |_, _, cx| {
        e_diff.update(cx, |this, cx| {
            if let Some(source) = this.diff_source_for(&format!("{FILE_PREFIX}{p_diff}"), cx) {
                cx.emit(LogEvent::OpenDiff(source));
            }
        })
    }))
    .item(PopupMenuItem::new("Compare with Local").disabled(no_local).on_click({
        let (entity, path, hash) = (entity.clone(), path.clone(), hash.clone());
        move |_, _, cx| {
            let source = DiffSource::Between { old: hash.clone(), new: None, path: path.clone(), old_path: None };
            entity.update(cx, |_, cx| cx.emit(LogEvent::OpenDiff(source)))
        }
    }))
    // The file as it was before the commit, at its old path if renamed.
    .item(PopupMenuItem::new("Compare Before with Local").disabled(added || no_local).on_click({
        let (entity, path, hash, old_path) = (entity.clone(), path.clone(), hash.clone(), old_path.clone());
        move |_, _, cx| {
            let source = DiffSource::Between { old: format!("{hash}^"), new: None, path: path.clone(), old_path: old_path.clone() };
            entity.update(cx, |_, cx| cx.emit(LogEvent::OpenDiff(source)))
        }
    }))
    .item(PopupMenuItem::new("Open Repository Version").on_click({
        let (entity, path, hash) = (entity.clone(), path.clone(), hash.clone());
        move |_, _, cx| {
            let (path, revision) = (path.clone(), Some(hash.clone()));
            entity.update(cx, |_, cx| cx.emit(LogEvent::OpenFile { path, revision }))
        }
    }))
    .item(PopupMenuItem::new("Edit Source").on_click({
        let (entity, path) = (entity.clone(), path.clone());
        move |_, _, cx| {
            let path = path.clone();
            entity.update(cx, |_, cx| cx.emit(LogEvent::OpenFile { path, revision: None }))
        }
    }))
    .item(PopupMenuItem::new("Annotate Revision").on_click(move |_, _, cx| {
        e_blame.update(cx, |_, cx| cx.emit(LogEvent::Annotate { path: p_blame.clone(), revision: Some(h_blame.clone()) }))
    }))
    .separator()
    .item(PopupMenuItem::new("Show History").on_click(move |_, _, cx| {
        let path = p_history.clone();
        e_history.update(cx, |this, cx| this.open_history_tab(path, None, cx))
    }))
    .item(PopupMenuItem::new("History Up to Here").on_click(move |_, _, cx| {
        let (path, hash) = (p_here.clone(), h_here.clone());
        e_here.update(cx, |this, cx| this.open_history_tab(path, Some(hash), cx))
    }))
    .separator()
    .item(PopupMenuItem::new(format!("Get from Revision {short}")).on_click(move |_, _, cx| {
        let (path, hash) = (p_get.clone(), h_get.clone());
        m_get.update(cx, |model, cx| {
            model.run_operation("Get from Revision", move |repo| {
                repo.run(["checkout", hash.as_str(), "--", path.as_str()])?;
                Ok(format!("{path} restored from {}", &hash[..hash.len().min(8)]))
            }, cx)
        })
    }))
    .item(PopupMenuItem::new("Revert Selected Changes").on_click(move |_, _, cx| {
        let (path, hash) = (p_revert.clone(), h_revert.clone());
        m_revert.update(cx, |model, cx| {
            model.run_operation("Revert Changes", move |repo| {
                let patch = repo.run(["show", "--format=", "--binary", hash.as_str(), "--", path.as_str()])?;
                repo.run_with_input(["apply", "-R", "--3way", "-"], Some(&patch))?;
                Ok(format!("Changes to {path} reverted in the working tree"))
            }, cx)
        })
    }))
    .separator()
    .item(PopupMenuItem::new("Copy Path").on_click(move |_, _, cx| {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(p_copy.clone()))
    }))
    .when_some(web_file, |menu, (name, url)| {
        menu.item(PopupMenuItem::new(format!("Open on {name}")).on_click(move |_, _, cx| cx.open_url(&url)))
    })
}
