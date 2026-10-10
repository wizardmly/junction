//! The commit table: rows, graph, columns and keyboard selection.

use super::*;

impl LogView {
    /// Mouse selection: plain click selects one commit, Ctrl/Cmd-click
    /// toggles, Shift-click selects the range from the last plain click.
    pub(super) fn click_row(&mut self, ix: usize, toggle: bool, range: bool, cx: &mut Context<Self>) {
        self.update_selection(ix, toggle, range, cx);
        // The details pane shows the selection's combined changes.
        self.rebuild_changes(cx);
    }

    pub(super) fn update_selection(&mut self, ix: usize, toggle: bool, range: bool, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let commits = model.commits().clone();
        let lead = model.selected_hash().map(str::to_owned);
        let Some(hash) = commits.get(ix).map(|c| c.hash.clone()) else { return };
        if range {
            let anchor = self.anchor.or(model.selected_index()).unwrap_or(ix);
            let (from, to) = (anchor.min(ix), anchor.max(ix));
            self.extra_selection = commits[from..=to].iter().map(|c| c.hash.clone()).filter(|h| *h != hash).collect();
        } else if toggle {
            if lead.as_deref() == Some(hash.as_str()) {
                // Deselect the lead: another selected commit takes its place.
                if let Some(next) = self.extra_selection.iter().next().cloned() {
                    self.extra_selection.remove(&next);
                    self.model.update(cx, |m, cx| m.select_hash(Some(next), cx));
                }
                cx.notify();
                return;
            }
            if !self.extra_selection.remove(&hash) {
                self.extra_selection.extend(lead);
            } else {
                cx.notify();
                return;
            }
            self.anchor = Some(ix);
        } else {
            self.extra_selection.clear();
            self.anchor = Some(ix);
        }
        self.model.update(cx, |m, cx| m.select_index(ix, cx));
        cx.notify();
    }

    /// Selected commits, oldest first (the order cherry-pick applies them).
    pub(super) fn selected_commits(&self, cx: &App) -> Vec<Commit> {
        let model = self.model.read(cx);
        let lead = model.selected_hash();
        model
            .commits()
            .iter()
            .rev()
            .filter(|c| Some(c.hash.as_str()) == lead || self.extra_selection.contains(&c.hash))
            .cloned()
            .collect()
    }

    /// Arrow keys, Page Up / Down, Home / End: one commit selected, `delta`
    /// rows from the lead one (clamped to the list).
    pub(super) fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let count = model.commits().len();
        if count == 0 {
            return;
        }
        let current = model.selected_index().map(|ix| ix as isize).unwrap_or(-1);
        let next = (current + delta).clamp(0, count as isize - 1) as usize;
        self.extra_selection.clear();
        self.anchor = Some(next);
        self.model.update(cx, |model, cx| model.select_index(next, cx));
        self.rebuild_changes(cx);
        cx.notify();
    }

    /// Shift with the arrows, Home or End: the range from the anchor grows
    /// or shrinks, as a Shift-click does.
    pub(super) fn extend_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let count = model.commits().len();
        let Some(current) = model.selected_index() else { return self.move_selection(delta.signum(), cx) };
        if self.anchor.is_none() {
            self.anchor = Some(current);
        }
        let next = (current as isize + delta).clamp(0, count as isize - 1) as usize;
        self.click_row(next, false, true, cx);
    }

    /// Rows visible in the table, for Page Up / Page Down.
    pub(super) fn page_rows(&self) -> isize {
        let state = self.scroll.0.borrow();
        let height = f32::from(state.base_handle.bounds().size.height);
        ((height / row_height()).floor() as isize - 1).max(1)
    }

    pub(super) fn on_select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(-1, cx);
    }

    pub(super) fn on_select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(1, cx);
    }

    pub(super) fn on_select_first(&mut self, _: &SelectFirst, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(isize::MIN / 2, cx);
    }

    pub(super) fn on_select_last(&mut self, _: &SelectLast, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(isize::MAX / 2, cx);
    }

    pub(super) fn on_page_up(&mut self, _: &SelectPageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(-self.page_rows(), cx);
    }

    pub(super) fn on_page_down(&mut self, _: &SelectPageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(self.page_rows(), cx);
    }

    pub(super) fn on_extend_previous(&mut self, _: &ExtendPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.extend_selection(-1, cx);
    }

    pub(super) fn on_extend_next(&mut self, _: &ExtendNext, _: &mut Window, cx: &mut Context<Self>) {
        self.extend_selection(1, cx);
    }

    pub(super) fn on_extend_first(&mut self, _: &ExtendFirst, _: &mut Window, cx: &mut Context<Self>) {
        self.extend_selection(isize::MIN / 2, cx);
    }

    pub(super) fn on_extend_last(&mut self, _: &ExtendLast, _: &mut Window, cx: &mut Context<Self>) {
        self.extend_selection(isize::MAX / 2, cx);
    }

    /// Ctrl+A: every loaded commit, the lead one staying where it is.
    pub(super) fn on_select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let lead = model.selected_hash().map(str::to_owned).or_else(|| model.commits().first().map(|c| c.hash.clone()));
        let Some(lead) = lead else { return };
        self.extra_selection = model.commits().iter().map(|c| c.hash.clone()).filter(|h| *h != lead).collect();
        if model.selected_hash() != Some(lead.as_str()) {
            self.model.update(cx, |m, cx| m.select_hash(Some(lead), cx));
        }
        self.rebuild_changes(cx);
        cx.notify();
    }

    pub(super) fn on_copy_revision(&mut self, _: &CopyRevision, _: &mut Window, cx: &mut Context<Self>) {
        // Newest first, like IntelliJ's Copy Revision Number with several selected.
        let hashes: Vec<String> = self.selected_commits(cx).into_iter().rev().map(|c| c.hash).collect();
        if !hashes.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(hashes.join("\n")));
        }
    }

    /// Which rows are reachable from HEAD within the loaded log (cached per load).
    pub(super) fn head_reachable(&mut self, cx: &App) -> Rc<Vec<bool>> {
        let model = self.model.read(cx);
        let commits = model.commits().clone();
        let head = model.refs().head_commit.clone();
        let key = Arc::as_ptr(&commits) as usize;
        if let Some((k, h, set)) = &self.head_reachable {
            if *k == key && *h == head {
                return set.clone();
            }
        }
        let mut reachable = vec![false; commits.len()];
        let mut stack: Vec<usize> = head.as_deref().and_then(|h| model.row_of(h)).into_iter().collect();
        while let Some(ix) = stack.pop() {
            if std::mem::replace(&mut reachable[ix], true) {
                continue;
            }
            stack.extend(commits[ix].parents.iter().filter_map(|p| model.row_of(p)).filter(|&p| !reachable[p]));
        }
        let set = Rc::new(reachable);
        self.head_reachable = Some((key, head, set.clone()));
        set
    }

    /// Follows a column edge drag anywhere in the window; the widths are
    /// saved when the mouse is released.
    pub(super) fn column_drag_tracker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        gpui_kit::canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let moving = entity.clone();
                window.on_mouse_event(move |e: &gpui_kit::MouseMoveEvent, phase, _, cx| {
                    if phase != gpui_kit::DispatchPhase::Bubble {
                        return;
                    }
                    moving.update(cx, |this, cx| {
                        let (Some((column, start, width)), Some(mut widths)) = (this.column_drag, this.columns) else { return };
                        // The edge is on the column's left: dragging left widens it.
                        let w = (width as f32 + start - f32::from(e.position.x)).clamp(40., 600.) as u32;
                        if widths[column] != w {
                            widths[column] = w;
                            this.columns = Some(widths);
                            cx.notify();
                        }
                    });
                });
                let released = entity.clone();
                window.on_mouse_event(move |_: &gpui_kit::MouseUpEvent, phase, _, cx| {
                    if phase != gpui_kit::DispatchPhase::Bubble {
                        return;
                    }
                    released.update(cx, |this, cx| {
                        this.column_drag = None;
                        if let Some(widths) = this.columns.take() {
                            Settings::update(cx, |s| s.log.columns = widths);
                        }
                        cx.notify();
                    });
                });
            },
        )
        .absolute()
        .size_full()
    }

    pub(super) fn render_rows(&mut self, range: std::ops::Range<usize>, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let palette = cx.palette().clone();
        let focused = self.focus.contains_focused(window, cx);
        let model = self.model.read(cx);
        let commits = model.commits().clone();
        let graph = model.graph().clone();
        let refs = model.refs().clone();
        let selected = model.selected_index();
        let show_long_edges = Settings::get(cx).log.show_long_edges;
        let graph_rows = graph.rows(range.clone(), show_long_edges);
        let model = self.model.read(cx);
        let extra: Vec<bool> = range.clone().map(|ix| commits.get(ix).is_some_and(|c| self.extra_selection.contains(&c.hash))).collect();
        let me = model.user_email().map(str::to_owned);
        let log = Settings::get(cx).log.clone();
        let show_hash = log.show_hash;
        let entity = cx.entity();
        let widths = self.columns.unwrap_or(log.columns).map(|w| px(w as f32));
        // A column's left edge: drag it to resize the column, as in IntelliJ's log table.
        let edge = |column: usize, ix: usize, entity: &Entity<Self>| {
            let entity = entity.clone();
            div()
                .id(("log-column-edge", column * 10_000_000 + ix))
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(px(5.))
                .cursor(gpui_kit::CursorStyle::ResizeLeftRight)
                .on_mouse_down(gpui_kit::MouseButton::Left, move |event: &gpui_kit::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    entity.update(cx, |this, cx| {
                        let widths = this.columns.unwrap_or(Settings::get(cx).log.columns);
                        this.columns = Some(widths);
                        this.column_drag = Some((column, f32::from(event.position.x), widths[column]));
                        cx.notify();
                    });
                })
        };
        let reachable = if log.highlight_current_branch || log.highlight_not_merged { Some(self.head_reachable(cx)) } else { None };
        // Clicking a long edge's arrow goes to the edge's other end.
        let on_arrow: crate::ui::graph_paint::OnArrow = {
            let entity = entity.clone();
            Rc::new(move |target, window, cx| {
                entity.update(cx, |this, cx| {
                    window.focus(&this.focus, cx);
                    this.click_row(target, false, false, cx);
                })
            })
        };
        let model = self.model.read(cx);

        let range_start = range.start;
        range
            .filter_map(|ix| {
                let commit = commits.get(ix)?;
                let row = graph_rows.get(ix - range_start).cloned().unwrap_or_default();
                let is_selected = selected == Some(ix) || extra[ix - range_start];
                let is_head = refs.head_commit.as_deref() == Some(commit.hash.as_str());
                let mine = log.highlight_mine && me.as_deref().is_some_and(|me| *me == *commit.author_email);
                let labels = refs.for_commit(&commit.hash);
                let is_merge = commit.parents.len() > 1;
                let on_head = reachable.as_ref().map(|r| r.get(ix).copied().unwrap_or(false));
                // Merge commits and commits not merged into HEAD are greyed, as IntelliJ's highlighters do.
                let dim = (log.highlight_merges && is_merge) || (log.highlight_not_merged && on_head == Some(false));
                let text_color = if dim { palette.text_secondary } else { palette.text };
                let branch_tint = log.highlight_current_branch && on_head == Some(true);

                // The subject keeps some width in a narrow table; the other
                // columns are clipped instead.
                let mut subject = h_flex()
                    .flex_1()
                    .min_w(px(160.))
                    .h_full()
                    .overflow_hidden()
                    .child(graph_canvas(row, &palette, is_head, ix, &on_arrow));
                let shown = if log.compact_refs { 1 } else { 4 };
                let mut ref_labels = h_flex().flex_shrink_0();
                // A detached HEAD gets its own label, as in IntelliJ.
                if is_head && refs.current_branch.is_none() {
                    ref_labels = ref_labels.child(
                        h_flex()
                            .flex_shrink_0()
                            .gap_0p5()
                            .mr_1p5()
                            .text_xs()
                            .child(Icon::new(IconName::GitBranch).xsmall().text_color(palette.ref_head))
                            .child(div().text_color(palette.ref_head).child("HEAD")),
                    );
                }
                for label in labels.iter().take(shown) {
                    ref_labels = ref_labels.child(ref_label(label, refs.current_branch.as_deref(), &palette));
                }
                if labels.len() > shown {
                    ref_labels = ref_labels.child(
                        div().mr_1().text_xs().text_color(palette.text_secondary).child(format!("+{}", labels.len() - shown)),
                    );
                }
                if log.refs_on_left {
                    subject = subject.child(ref_labels);
                    ref_labels = h_flex();
                }
                subject = subject.child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_color(text_color)
                        .when(mine, |el| el.font_weight(FontWeight::SEMIBOLD))
                        .child(commit.subject.clone()),
                );
                if !log.refs_on_left && !labels.is_empty() {
                    subject = subject.child(div().flex_1()).child(ref_labels);
                }
                if let Some(count) = model.hidden_below(&commit.hash) {
                    let run = commit.hash.clone();
                    let model_entity = self.model.clone();
                    subject = subject.child(
                        div()
                            .id(SharedString::from(format!("expand-{}", commit.hash)))
                            .ml_2()
                            .px_1()
                            .rounded_sm()
                            .text_xs()
                            .text_color(palette.text_secondary)
                            .bg(palette.text_secondary.opacity(0.12))
                            .hover(|el| el.bg(palette.text_secondary.opacity(0.25)))
                            .cursor_pointer()
                            .child(format!("⋯ {count} commits"))
                            .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                model_entity.update(cx, |model, cx| model.expand_run(run.clone(), cx));
                            }),
                    );
                }

                let hash = commit.hash.clone();
                let menu_commit = commit.clone();
                let menu_entity = entity.clone();
                Some(
                    h_flex()
                        .id(SharedString::from(format!("commit-{}", commit.hash)))
                        .h(px(row_height()))
                        .w_full()
                        .pr_2()
                        .text_sm()
                        .when(is_selected, |el| el.bg(if focused { palette.selection } else { palette.selection_inactive }))
                        .when(!is_selected && branch_tint, |el| el.bg(palette.accent.opacity(0.08)))
                        .when(!is_selected, |el| el.hover(|s| s.bg(palette.hover)))
                        .on_mouse_down(gpui_kit::MouseButton::Left, {
                            let entity = entity.clone();
                            move |event: &gpui_kit::MouseDownEvent, window, cx| {
                                let (toggle, range) = (event.modifiers.secondary(), event.modifiers.shift);
                                entity.update(cx, |this, cx| {
                                    window.focus(&this.focus, cx);
                                    this.click_row(ix, toggle, range, cx);
                                });
                            }
                        })
                        .on_mouse_down(gpui_kit::MouseButton::Right, {
                            let entity = entity.clone();
                            let hash = hash.clone();
                            move |_, _, cx| {
                                let hash = hash.clone();
                                entity.update(cx, |this, cx| {
                                    // Right-clicking inside a multi-selection keeps it.
                                    let in_selection = this.extra_selection.contains(&hash)
                                        || this.model.read(cx).selected_hash() == Some(hash.as_str());
                                    if !in_selection {
                                        this.extra_selection.clear();
                                        this.model.update(cx, |model, cx| model.select_hash(Some(hash), cx));
                                    }
                                });
                            }
                        })
                        .context_menu(move |menu, window, cx| commit_menu(menu, &menu_entity, &menu_commit, window, cx))
                        .child(subject)
                        .when(log.show_author, |el| {
                            el.child(
                                div()
                                    .relative()
                                    .w(widths[0])
                                    .flex_shrink_0()
                                    .pl_2()
                                    .child(edge(0, ix, &entity))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_color(palette.text_secondary)
                                    .when(mine, |el| el.font_weight(FontWeight::SEMIBOLD))
                                    .child(commit.author_name.to_string()),
                            )
                        })
                        .when(log.show_date, |el| {
                            el.child(
                                div()
                                    .relative()
                                    .w(widths[1])
                                    .flex_shrink_0()
                                    .pl_2()
                                    .child(edge(1, ix, &entity))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_color(palette.text_secondary)
                                    .child(if log.relative_dates {
                                        common::format_relative_date(commit.author_time)
                                    } else {
                                        common::format_date(commit.author_time)
                                    }),
                            )
                        })
                        .when(show_hash, |el| {
                            el.child(
                                div()
                                    .relative()
                                    .w(widths[2])
                                    .flex_shrink_0()
                                    .pl_2()
                                    .child(edge(2, ix, &entity))
                                    .overflow_hidden()
                                    .font_family(cx.theme().mono_font_family.clone())
                                    .text_color(palette.text_secondary)
                                    .child(commit.short_hash().to_owned()),
                            )
                        })
                        .into_any_element(),
                )
            })
            .collect()
    }

    /// The commit table (its own view, see [`LogPart`]).
    pub(super) fn render_table(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = cx.palette().clone();
        let count = self.model.read(cx).commits().len();
        let empty = count == 0 && !self.model.read(cx).is_loading();

        div()
            .id("log-table")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_select_previous))
            .on_action(cx.listener(Self::on_select_next))
            .on_action(cx.listener(Self::on_select_first))
            .on_action(cx.listener(Self::on_select_last))
            .on_action(cx.listener(Self::on_page_up))
            .on_action(cx.listener(Self::on_page_down))
            .on_action(cx.listener(Self::on_extend_previous))
            .on_action(cx.listener(Self::on_extend_next))
            .on_action(cx.listener(Self::on_extend_first))
            .on_action(cx.listener(Self::on_extend_last))
            .on_action(cx.listener(Self::on_select_all))
            .on_action(cx.listener(Self::on_copy_revision))
            .on_action(cx.listener(Self::on_go_to_hash))
            .flex_1()
            .min_h_0()
            .relative()
            .when(empty, |el| {
                el.child(
                    v_flex()
                        .size_full()
                        .items_center()
                        .justify_center()
                        .text_color(palette.text_secondary)
                        .child("No commits matching filters"),
                )
            })
            .when(!empty, |el| {
                el.child(
                    uniform_list("log-rows", count, cx.processor(Self::render_rows))
                        .track_scroll(&self.scroll)
                        .size_full(),
                )
                .vertical_scrollbar(&self.scroll)
                .when(self.column_drag.is_some(), |el| el.child(self.column_drag_tracker(cx)))
            })
            .size_full()
            .into_any_element()
    }
}
