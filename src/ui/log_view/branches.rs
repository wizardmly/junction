//! The branches tree beside the Log.

use super::*;

impl LogView {
    /// Runs the branch popup's action (matched by label) on the branch
    /// selected in the Branches panel, so the toolbar and menus agree.
    pub(super) fn run_selected_branch_action(&mut self, matches: fn(&str) -> bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.branches.read(cx).selected_item().map(|i| i.id.to_string()) else { return };
        let Some(full) = id.strip_prefix(BRANCH_PREFIX) else { return };
        let refs = self.model.read(cx).log_refs().clone();
        let Some(reference) = refs.find(full).cloned() else { return };
        let remotes = crate::ui::branches_popup::remote_names(&refs);
        let actions = crate::ui::branches_popup::branch_actions(&self.model, &reference, refs.current_branch.as_deref(), &remotes);
        if let Some(action) = actions.into_iter().find(|a| a.enabled && matches(&a.label)) {
            (action.run)(window, cx);
        }
    }

    pub(super) fn rebuild_branches(&mut self, cx: &mut Context<Self>) {
        // What the user opened or closed, unless it was search results.
        if !self.branch_searching {
            collect_expanded(&self.branch_items, &mut self.branch_expansion);
        }
        let mut refs = (**self.model.read(cx).log_refs()).clone();
        // Speed search: keep matching refs only, with every folder open.
        let query = self.branch_search.read(cx).value().trim().to_lowercase();
        let searching = !query.is_empty();
        if searching {
            refs.refs.retain(|r| r.name.to_lowercase().contains(&query));
        }
        if self.my_branches {
            let mine = self.my_refs(cx);
            refs.refs.retain(|r| r.kind == RefKind::Tag || mine.contains(&r.full_name));
        }
        // Favorites first within each group, as IntelliJ pins them.
        refs.refs.sort_by_key(|r| (r.kind, !refs.favorites.contains(&r.full_name)));
        let mut items = Vec::new();
        if let Some(branch) = &refs.current_branch {
            items.push(TreeItem::new(format!("{BRANCH_PREFIX}refs/heads/{branch}"), "HEAD (Current Branch)"));
        } else if refs.head_commit.is_some() {
            items.push(TreeItem::new("head", "HEAD (Detached)"));
        }
        let local: Vec<&RefName> = refs.local_branches().collect();
        items.push(
            TreeItem::new("group:local", "Local")
                .expanded(true)
                .children(group_branches(&local, "local", |r| r.name.clone())),
        );
        let remote: Vec<&RefName> = refs.remote_branches().collect();
        let mut remotes: Vec<String> = remote.iter().filter_map(|r| r.remote().map(str::to_owned)).collect();
        remotes.dedup();
        let remote_items: Vec<TreeItem> = remotes
            .iter()
            .map(|name| {
                let branches: Vec<&RefName> = remote.iter().copied().filter(|r| r.remote() == Some(name)).collect();
                TreeItem::new(format!("group:remote/{name}"), name.clone())
                    .expanded(true)
                    .children(group_branches(&branches, &format!("remote/{name}"), |r| r.branch_without_remote().to_owned()))
            })
            .collect();
        // No remotes, no Remote node, as in IntelliJ.
        if !remote_items.is_empty() {
            items.push(TreeItem::new("group:remote", "Remote").expanded(true).children(remote_items));
        }
        let tags: Vec<&RefName> = refs.tags().collect();
        if !tags.is_empty() {
            items.push(
                TreeItem::new("group:tags", "Tags").expanded(searching).children(
                    tags.iter().map(|t| TreeItem::new(format!("{BRANCH_PREFIX}{}", t.full_name), t.name.clone())),
                ),
            );
        }
        if searching {
            items = items.into_iter().map(expand_all).collect();
        } else if let Some(expand) = self.branch_tree_expanded.take() {
            // Expand All / Collapse All, once.
            items = items.into_iter().map(if expand { expand_all } else { collapse_all }).collect();
        } else {
            // A reload or a filter keeps the folders the user opened or closed.
            restore_expanded(&items, &self.branch_expansion);
        }
        self.branch_items = items.clone();
        self.branch_searching = searching;
        let selected = self.branches.read(cx).selected_item().map(|item| item.id.clone());
        self.branches.update(cx, |tree, cx| {
            tree.set_items(items, cx);
            // The selection stays on the same branch, wherever it now is.
            let mut ix = selected.and_then(|id| tree.index_of(&id));
            // Speed search selects the first match unless the selection is one.
            if searching {
                let is_match = |ix: usize| {
                    tree.entry(ix).is_some_and(|e| {
                        !e.is_folder()
                            && e.item().id.starts_with(BRANCH_PREFIX)
                            && !e.item().label.starts_with("HEAD (")
                    })
                };
                if !ix.is_some_and(is_match) {
                    ix = (0..).take_while(|i| tree.entry(*i).is_some()).find(|i| is_match(*i)).or(ix);
                }
                if let Some(ix) = ix {
                    tree.scroll_to_item(ix, ScrollStrategy::Nearest);
                }
            }
            tree.set_selected_index(ix, cx);
        });
    }

    pub(super) fn set_branch_search(&mut self, value: String, window: &mut Window, cx: &mut Context<Self>) {
        self.branch_search.update(cx, |state, cx| state.set_value(value, window, cx));
        self.rebuild_branches(cx);
        cx.notify();
    }

    /// Opens or closes a folder of the branches tree, keeping the selection.
    pub(super) fn toggle_branch_folder(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(entry) = self.branches.read(cx).entry(ix).cloned() else { return };
        if !entry.is_folder() {
            return;
        }
        entry.item().clone().expanded(!entry.is_expanded());
        let (items, id) = (self.branch_items.clone(), entry.item().id.clone());
        self.branches.update(cx, |tree, cx| {
            tree.set_items(items, cx);
            let ix = tree.index_of(&id);
            tree.set_selected_index(ix, cx);
        });
    }

    /// Enter: opens or closes a folder; on a branch, filters the Log by it
    /// (as a double click).
    pub(super) fn on_branch_confirm(&mut self, _: &BranchConfirm, _: &mut Window, cx: &mut Context<Self>) {
        let tree = self.branches.read(cx);
        let Some(ix) = tree.selected_index() else { return };
        let Some(entry) = tree.entry(ix) else { return };
        if entry.is_folder() {
            return self.toggle_branch_folder(ix, cx);
        }
        let full = entry.item().id.strip_prefix(BRANCH_PREFIX).map(str::to_owned);
        if let Some(name) = full.and_then(|full| self.model.read(cx).log_refs().find(&full).map(|r| r.name.clone())) {
            self.update_filter(cx, |f| f.branches = vec![name]);
        }
    }

    /// ←: on a branch or a closed folder, goes to the parent folder (the tree
    /// itself closes an open one).
    pub(super) fn on_branch_left(&mut self, _: &gpui_kit::base::actions::SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        let tree = self.branches.read(cx);
        let Some(ix) = tree.selected_index() else { return };
        let Some(entry) = tree.entry(ix) else { return };
        if entry.is_folder() && entry.is_expanded() {
            return;
        }
        let depth = entry.depth();
        let parent = (0..ix).rev().find(|&i| tree.entry(i).is_some_and(|e| e.depth() + 1 == depth));
        if let Some(parent) = parent {
            cx.stop_propagation();
            self.branches.update(cx, |tree, cx| {
                tree.set_selected_index(Some(parent), cx);
                tree.scroll_to_item(parent, ScrollStrategy::Nearest);
            });
        }
    }

    /// →: on an open folder, goes to its first child (the tree itself opens
    /// a closed one).
    pub(super) fn on_branch_right(&mut self, _: &gpui_kit::base::actions::SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        let tree = self.branches.read(cx);
        let Some(ix) = tree.selected_index() else { return };
        if tree.entry(ix).is_some_and(|e| e.is_folder() && e.is_expanded()) {
            cx.stop_propagation();
            self.branches.update(cx, |tree, cx| {
                tree.set_selected_index(Some(ix + 1), cx);
                tree.scroll_to_item(ix + 1, ScrollStrategy::Nearest);
            });
        }
    }

    /// Refs holding a commit I authored (Show My Branches), computed once
    /// per set of refs.
    pub(super) fn my_refs(&mut self, cx: &App) -> HashSet<String> {
        let model = self.model.read(cx);
        let key: Vec<(String, String)> = model.log_refs().refs.iter().map(|r| (r.full_name.clone(), r.target.clone())).collect();
        if let Some((k, mine)) = &self.my_refs {
            if *k == key {
                return mine.clone();
            }
        }
        let me = model.user_email().map(str::to_owned);
        let mine: HashSet<String> = match (model.repository(), me) {
            (Some(repo), Some(me)) => model
                .log_refs()
                .refs
                .iter()
                .filter(|r| r.kind != RefKind::Tag)
                .filter(|r| {
                    let author = format!("--author=<{me}>");
                    repo.run(["log", "-1", "--format=%H", "--regexp-ignore-case", "--fixed-strings", &author, &r.full_name])
                        .is_ok_and(|out| !out.trim().is_empty())
                })
                .map(|r| r.full_name.clone())
                .collect(),
            _ => HashSet::new(),
        };
        self.my_refs = Some((key, mine.clone()));
        mine
    }

    pub(super) fn render_branches(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let refs = self.model.read(cx).log_refs().clone();
        let query = self.branch_search.read(cx).value().trim().to_owned();
        let entity = cx.entity();
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(px(crate::ui::common::toolbar_height()))
                    .px_1()
                    .gap_0p5()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(tool_button("branches-new", IconName::Plus, "New Branch…").on_click(cx.listener(
                        |this, _, window, cx| {
                            let start = this.model.read(cx).selected_hash().unwrap_or("HEAD").to_owned();
                            dialogs::new_branch(this.model.clone(), start, window, cx);
                        },
                    )))
                    .child(tool_button("branches-fetch", IconName::CloudDownload, "Fetch").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.model.update(cx, |model, cx| {
                                model.run_operation("Fetch", |repo| {
                                    repo.run(["fetch", "--all", "--prune"])?;
                                    Ok("Fetched all remotes".into())
                                }, cx)
                            });
                        },
                    )))
                    .child(tool_button("branches-update", IconName::ArrowDownToLine, "Update Selected").on_click(cx.listener(
                        |this, _, window, cx| this.run_selected_branch_action(|label| label == "Update", window, cx),
                    )))
                    .child(tool_button("branches-delete", IconName::Delete, "Delete").on_click(cx.listener(
                        |this, _, window, cx| this.run_selected_branch_action(|label| label == "Delete", window, cx),
                    )))
                    .child(tool_button("branches-compare", IconName::GitCompare, "Compare with Current").on_click(cx.listener(
                        |this, _, window, cx| this.run_selected_branch_action(|label| label.starts_with("Compare with"), window, cx),
                    )))
                    .child(
                        tool_button("branches-mine", IconName::User, "Show My Branches")
                            .selected(self.my_branches)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.my_branches = !this.my_branches;
                                this.rebuild_branches(cx);
                                cx.notify();
                            })),
                    )
                    .child(tool_button("branches-expand", IconName::ChevronsUpDown, "Expand All").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.branch_tree_expanded = Some(true);
                            this.rebuild_branches(cx);
                        },
                    )))
                    .child(tool_button("branches-collapse", IconName::ChevronsDownUp, "Collapse All").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.branch_tree_expanded = Some(false);
                            this.rebuild_branches(cx);
                        },
                    )))
                    .child(tool_button("branches-filter", IconName::ListFilter, "Filter Log by Selected Branch").on_click(
                        cx.listener(|this, _, _, cx| {
                            let selected = this.branches.read(cx).selected_item().map(|i| i.id.clone());
                            let name = selected
                                .as_deref()
                                .and_then(|id| id.strip_prefix(BRANCH_PREFIX))
                                .and_then(|full| this.model.read(cx).log_refs().find(full).map(|r| r.name.clone()));
                            this.update_filter(cx, |f| f.branches = name.into_iter().collect());
                        }),
                    ))
            )
            .child(
                div()
                    .px_1()
                    .py_1()
                    .border_b_1()
                    .border_color(palette.border)
                    // Escape clears the search first, as IntelliJ's search fields do.
                    .capture_action(cx.listener(|this, _: &gpui_kit::component::input::Escape, window, cx| {
                        if !this.branch_search.read(cx).value().is_empty() {
                            cx.stop_propagation();
                            this.set_branch_search(String::new(), window, cx);
                        }
                    }))
                    .child(Input::new(&self.branch_search).xsmall().cleanable(true)),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .key_context(BRANCHES_CONTEXT)
                    .on_action(cx.listener(Self::on_branch_confirm))
                    .capture_action(cx.listener(Self::on_branch_left))
                    .capture_action(cx.listener(Self::on_branch_right))
                    // Typing in the tree starts the speed search.
                    .on_key_down(cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, cx| {
                        let keystroke = &event.keystroke;
                        let modifiers = keystroke.modifiers;
                        if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
                            return;
                        }
                        let Some(text) = keystroke.key_char.clone().filter(|t| t.chars().all(|c| !c.is_control())) else { return };
                        cx.stop_propagation();
                        let value = format!("{}{text}", this.branch_search.read(cx).value());
                        this.set_branch_search(value, window, cx);
                        let input = this.branch_search.clone();
                        input.update(cx, |state, cx| state.focus(window, cx));
                    }))
                    .child(
                    tree(&self.branches, move |ix, entry, _selected, _, _| {
                        let item = entry.item();
                        let id = item.id.to_string();
                        let full_name = id.strip_prefix(BRANCH_PREFIX).map(str::to_owned);
                        let reference = full_name.as_deref().and_then(|full| refs.find(full)).cloned();
                        let is_current = reference.as_ref().is_some_and(|r| {
                            r.kind == RefKind::LocalBranch && Some(&r.name) == refs.current_branch.as_ref()
                        });
                        let icon_name = if entry.is_folder() {
                            if id.starts_with("group:") { IconName::FolderGit2 } else { IconName::Folder }
                        } else {
                            match reference.as_ref().map(|r| r.kind) {
                                Some(RefKind::Tag) => IconName::Tag,
                                _ => IconName::GitBranch,
                            }
                        };
                        let icon_color = match reference.as_ref().map(|r| r.kind) {
                            _ if is_current || id == "head" => palette.ref_head,
                            Some(RefKind::LocalBranch) => palette.ref_local,
                            Some(RefKind::RemoteBranch) => palette.ref_remote,
                            Some(RefKind::Tag) => palette.ref_tag,
                            _ => palette.text_secondary,
                        };
                        let is_favorite = full_name.as_ref().is_some_and(|f| refs.favorites.contains(f));
                        let icon_name = if is_favorite && !entry.is_folder() { IconName::Star } else { icon_name };
                        let menu_reference = reference.clone();
                        let (menu_entity, menu_refs) = (entity.clone(), refs.clone());
                        let entity = entity.clone();
                        let double_click_name = reference.as_ref().map(|r| r.name.clone());
                        ListItem::new(ix)
                            .py_0()
                            .px_1()
                            .h(px(row_height()))
                            .on_click(move |event, _, cx| {
                                if event.click_count() == 2 {
                                    // Double click filters the log by this branch.
                                    if let Some(name) = double_click_name.clone() {
                                        entity.update(cx, |this, cx| this.update_filter(cx, |f| f.branches = vec![name]));
                                    }
                                }
                            })
                            .child(
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
                                    .child(Icon::new(icon_name).small().text_color(icon_color))
                                    .child(match label_match(&item.label, &query) {
                                        Some(range) => crate::ui::find_popup::highlighted(&item.label, &[range], &palette).into_any_element(),
                                        None => item.label.clone().into_any_element(),
                                    })
                                    .when_some(reference.filter(|r| r.ahead > 0 || r.behind > 0), |el, r| {
                                        el.child(
                                            h_flex()
                                                .gap_1()
                                                .text_xs()
                                                .when(r.behind > 0, |el| {
                                                    el.child(div().text_color(palette.link).child(format!("↓{}", r.behind)))
                                                })
                                                .when(r.ahead > 0, |el| {
                                                    el.child(div().text_color(palette.status_added).child(format!("↑{}", r.ahead)))
                                                }),
                                        )
                                    })
                                    .context_menu(move |menu, _, cx| {
                                        let Some(reference) = menu_reference.clone() else { return menu };
                                        branch_menu(menu, &menu_entity, &menu_refs, &reference, cx)
                                    }),
                            )
                    })
                    .size_full(),
                ),
            )
    }
}

/// Right-click on a branch or tag in the branches panel: the branch popup's
/// actions plus Add to / Remove from Favorites.
pub(super) fn branch_menu(
    menu: gpui_kit::component::menu::PopupMenu,
    entity: &Entity<LogView>,
    refs: &crate::git::RepositoryRefs,
    reference: &RefName,
    cx: &mut App,
) -> gpui_kit::component::menu::PopupMenu {
    use crate::ui::branches_popup;
    let model = entity.read(cx).model.clone();
    let remotes = branches_popup::remote_names(refs);
    let actions = branches_popup::branch_actions(&model, reference, refs.current_branch.as_deref(), &remotes);
    let mut menu = menu;
    for action in actions {
        let run = action.run.clone();
        menu = menu.item(PopupMenuItem::new(action.label).disabled(!action.enabled).on_click(move |_, window, cx| run(window, cx)));
    }
    let favorite = refs.favorites.contains(&reference.full_name);
    let full_name = reference.full_name.clone();
    menu.separator().item(PopupMenuItem::new(if favorite { "Remove from Favorites" } else { "Add to Favorites" }).on_click(
        move |_, _, cx| {
            model.update(cx, |model, cx| {
                if let Some(repo) = model.repository() {
                    let _ = crate::git::refs::set_favorite(repo, &full_name, !favorite);
                }
                model.reload(cx);
            })
        },
    ))
}

/// Where the speed search query appears in a tree label (case-insensitive).
pub(super) fn label_match(label: &str, query: &str) -> Option<std::ops::Range<usize>> {
    if query.is_empty() {
        return None;
    }
    let start = label.to_ascii_lowercase().find(&query.to_ascii_lowercase())?;
    Some(start..start + query.len())
}

/// Folder ids of a tree and whether each is open.
pub(super) fn collect_expanded(items: &[TreeItem], out: &mut HashMap<SharedString, bool>) {
    for item in items.iter().filter(|i| i.is_folder()) {
        out.insert(item.id.clone(), item.is_expanded());
        collect_expanded(&item.children, out);
    }
}

/// Opens or closes the folders that were known before, by id.
pub(super) fn restore_expanded(items: &[TreeItem], expanded: &HashMap<SharedString, bool>) {
    for item in items.iter().filter(|i| i.is_folder()) {
        if let Some(&open) = expanded.get(&item.id) {
            item.clone().expanded(open);
        }
        restore_expanded(&item.children, expanded);
    }
}

pub(super) fn collapse_all(mut item: TreeItem) -> TreeItem {
    if item.children.is_empty() {
        return item;
    }
    item.children = std::mem::take(&mut item.children).into_iter().map(collapse_all).collect();
    item.expanded(false)
}

pub(super) fn expand_all(mut item: TreeItem) -> TreeItem {
    if item.children.is_empty() {
        return item;
    }
    item.children = std::mem::take(&mut item.children).into_iter().map(expand_all).collect();
    item.expanded(true)
}

/// Groups branches by their `/`-separated path into folders.
pub(super) fn group_branches(branches: &[&RefName], scope: &str, short: impl Fn(&RefName) -> String) -> Vec<TreeItem> {
    #[derive(Default)]
    struct Node {
        dirs: std::collections::BTreeMap<String, Node>,
        leaves: Vec<(String, String)>,
    }
    let mut root = Node::default();
    for branch in branches {
        let name = short(branch);
        let mut parts: Vec<&str> = name.split('/').collect();
        let leaf = parts.pop().unwrap_or_default().to_owned();
        let mut node = &mut root;
        for part in parts {
            node = node.dirs.entry(part.to_owned()).or_default();
        }
        node.leaves.push((leaf, branch.full_name.clone()));
    }
    fn build(node: &Node, path: &str, scope: &str) -> Vec<TreeItem> {
        let mut items: Vec<TreeItem> = node
            .dirs
            .iter()
            .map(|(name, child)| {
                let path = format!("{path}/{name}");
                TreeItem::new(format!("dir:{scope}{path}"), name.clone()).children(build(child, &path, scope))
            })
            .collect();
        items.extend(node.leaves.iter().map(|(leaf, full)| TreeItem::new(format!("{BRANCH_PREFIX}{full}"), leaf.clone())));
        items
    }
    build(&root, "", scope)
}

pub(super) fn ref_label(reference: &RefName, current_branch: Option<&str>, palette: &crate::theme::Palette) -> impl IntoElement {
    let is_current = reference.kind == RefKind::LocalBranch && Some(reference.name.as_str()) == current_branch;
    let (icon, color) = match reference.kind {
        _ if is_current => (IconName::GitBranch, palette.ref_head),
        RefKind::LocalBranch => (IconName::GitBranch, palette.ref_local),
        RefKind::RemoteBranch => (IconName::GitBranch, palette.ref_remote),
        RefKind::Tag => (IconName::Tag, palette.ref_tag),
    };
    h_flex()
        .flex_shrink_0()
        .gap_0p5()
        .mr_1p5()
        .text_xs()
        .child(Icon::new(icon).xsmall().text_color(color))
        .child(div().text_color(color).child(reference.name.clone()))
}
