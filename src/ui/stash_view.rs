//! The Stash tab of the Commit tool window: stashes, their files, and
//! Apply / Pop / Drop / Clear / Unstash as Branch.

use std::collections::HashMap;

use gpui_kit::component::{
    Disableable as _, Icon, Sizable as _, h_flex,
    list::ListItem,
    menu::{ContextMenuExt as _, PopupMenuItem},
    scroll::ScrollableElement as _,
    tree::{TreeState, tree},
    v_flex, v_resizable, resizable_panel,
};
use gpui_kit::assets::IconName;
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div,
    prelude::FluentBuilder as _, px,
};

use crate::git::ops::{self, Stash};
use crate::git::FileChangeKind;
use crate::model::{RepoEvent, RepoModel};
use crate::theme::ActivePalette as _;
use crate::ui::common::{self, FILE_PREFIX, row_height, tool_button};
use crate::ui::dialogs;
use crate::ui::diff_view::DiffSource;

pub enum StashEvent {
    OpenDiff(DiffSource),
}

impl EventEmitter<StashEvent> for StashView {}

pub struct StashView {
    model: Entity<RepoModel>,
    stashes: Vec<Stash>,
    selected: Option<usize>,
    files: Entity<TreeState>,
    kinds: HashMap<String, FileChangeKind>,
    last_file: Option<SharedString>,
    _load: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl StashView {
    pub fn new(model: Entity<RepoModel>, cx: &mut Context<Self>) -> Self {
        let files = cx.new(|cx| TreeState::new(cx));
        let subscriptions = vec![
            cx.subscribe(&model, |this, _, event, cx| {
                if matches!(event, RepoEvent::Reloaded) {
                    this.reload(cx);
                }
            }),
            cx.observe(&files, |this, files, cx| {
                let selected = files.read(cx).selected_item().map(|i| i.id.clone());
                if selected != this.last_file {
                    this.last_file = selected.clone();
                    let path = selected.as_deref().and_then(|id| id.strip_prefix(FILE_PREFIX)).map(str::to_owned);
                    if let (Some(path), Some(stash)) = (path, this.selected.and_then(|ix| this.stashes.get(ix))) {
                        cx.emit(StashEvent::OpenDiff(DiffSource::Commit { hash: stash.hash.clone(), path, old_path: None }));
                    }
                }
            }),
        ];
        let mut this = Self {
            model,
            stashes: Vec::new(),
            selected: None,
            files,
            kinds: HashMap::new(),
            last_file: None,
            _load: None,
            _subscriptions: subscriptions,
        };
        this.reload(cx);
        this
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(repository) = self.model.read(cx).repository().cloned() else { return };
        self._load = Some(cx.spawn(async move |this, cx| {
            let stashes = cx.background_spawn(async move { ops::stash_list(&repository).unwrap_or_default() }).await;
            this.update(cx, |this, cx| {
                let previous = this.selected.and_then(|ix| this.stashes.get(ix)).map(|s| s.hash.clone());
                this.stashes = stashes;
                let ix = previous.and_then(|h| this.stashes.iter().position(|s| s.hash == h));
                this.select(ix.or(if this.stashes.is_empty() { None } else { Some(0) }), cx);
            })
            .ok();
        }));
    }

    fn select(&mut self, ix: Option<usize>, cx: &mut Context<Self>) {
        self.selected = ix;
        self.kinds.clear();
        self.last_file = None;
        let Some(stash) = ix.and_then(|ix| self.stashes.get(ix)).cloned() else {
            self.files.update(cx, |tree, cx| tree.set_items(Vec::new(), cx));
            cx.notify();
            return;
        };
        let Some(repository) = self.model.read(cx).repository().cloned() else { return };
        self._load = Some(cx.spawn(async move |this, cx| {
            let changes = cx
                .background_spawn(async move { ops::stash_files(&repository, &stash.name).unwrap_or_default() })
                .await;
            this.update(cx, |this, cx| {
                this.kinds = changes.iter().map(|c| (c.path.clone(), c.kind)).collect();
                let items = common::file_tree(changes.into_iter().map(|c| c.path), "");
                this.files.update(cx, |tree, cx| tree.set_items(items, cx));
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn drop_stash(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(stash) = self.selected.and_then(|ix| self.stashes.get(ix)).cloned() else { return };
        let entity = cx.entity();
        dialogs::confirm(
            "Drop Stash",
            format!("Do you want to remove {}?\n{}", stash.name, stash.message),
            "Drop",
            move |cx| entity.update(cx, |this, cx| this.stash_op("Drop Stash", "drop", cx)),
            window,
            cx,
        );
    }

    fn clear(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.stashes.is_empty() {
            return;
        }
        let model = self.model.clone();
        dialogs::confirm(
            "Clear Stashes",
            format!("Do you want to remove all {} stashes? This can't be undone.", self.stashes.len()),
            "Clear",
            move |cx| {
                model.update(cx, |m, cx| {
                    m.run_operation("Clear Stashes", |repo| {
                        repo.run(["stash", "clear"])?;
                        Ok("All stashes removed".into())
                    }, cx)
                })
            },
            window,
            cx,
        );
    }

    fn unstash_as(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(stash) = self.selected.and_then(|ix| self.stashes.get(ix)).cloned() else { return };
        dialogs::unstash_as(self.model.clone(), stash.name, stash.message, window, cx);
    }

    fn stash_op(&self, title: &'static str, verb: &'static str, cx: &mut Context<Self>) {
        let Some(stash) = self.selected.and_then(|ix| self.stashes.get(ix)).cloned() else { return };
        self.model.update(cx, |model, cx| {
            model.run_operation(title, move |repo| {
                repo.run(["stash", verb, &stash.name])?;
                Ok(format!("{title}: {}", stash.message))
            }, cx)
        });
    }
}

impl Render for StashView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let has_selection = self.selected.is_some();
        let entity = cx.entity();

        let mut list = v_flex();
        for (ix, stash) in self.stashes.iter().enumerate() {
            let selected = self.selected == Some(ix);
            let menu_entity = entity.clone();
            list = list.child(
                h_flex()
                    .id(SharedString::from(format!("stash-{}", stash.hash)))
                    .h(px(row_height() + 4.))
                    .px_2()
                    .gap_2()
                    .text_sm()
                    .when(selected, |el| el.bg(palette.selection))
                    .when(!selected, |el| el.hover(|s| s.bg(palette.hover)))
                    .on_click(cx.listener(move |this, _, _, cx| this.select(Some(ix), cx)))
                    .context_menu(move |menu, _, _| {
                        let (a, p, d) = (menu_entity.clone(), menu_entity.clone(), menu_entity.clone());
                        let (u, c) = (menu_entity.clone(), menu_entity.clone());
                        menu.item(PopupMenuItem::new("Apply").on_click(move |_, _, cx| {
                            a.update(cx, |this, cx| {
                                this.selected = Some(ix);
                                this.stash_op("Apply Stash", "apply", cx)
                            })
                        }))
                        .item(PopupMenuItem::new("Pop").on_click(move |_, _, cx| {
                            p.update(cx, |this, cx| {
                                this.selected = Some(ix);
                                this.stash_op("Pop Stash", "pop", cx)
                            })
                        }))
                        .item(PopupMenuItem::new("Unstash As…").on_click(move |_, window, cx| {
                            u.update(cx, |this, cx| {
                                this.selected = Some(ix);
                                this.unstash_as(window, cx)
                            })
                        }))
                        .separator()
                        .item(PopupMenuItem::new("Drop…").on_click(move |_, window, cx| {
                            d.update(cx, |this, cx| {
                                this.selected = Some(ix);
                                this.drop_stash(window, cx)
                            })
                        }))
                        .item(PopupMenuItem::new("Clear…").on_click(move |_, window, cx| c.update(cx, |this, cx| this.clear(window, cx))))
                    })
                    .child(Icon::new(IconName::Archive).small().text_color(palette.text_secondary))
                    .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().child(stash.message.clone()))
                    .when_some(stash.branch.clone(), |el, branch| {
                        el.child(div().text_xs().text_color(palette.ref_local).child(branch))
                    })
                    .child(div().text_xs().text_color(palette.text_secondary).child(common::format_date(stash.time))),
            );
        }
        if self.stashes.is_empty() {
            list = list.child(div().p_3().text_sm().text_color(palette.text_secondary).child("No stashes"));
        }

        let kinds = self.kinds.clone();
        let tree_palette = palette.clone();
        let files = tree(&self.files, move |ix, entry, _, _, _| {
            let palette = &tree_palette;
            let item = entry.item();
            let path = item.id.strip_prefix(FILE_PREFIX).map(str::to_owned);
            let color = path.as_ref().and_then(|p| kinds.get(p)).map_or(palette.text, |k| common::change_color(*k, palette));
            ListItem::new(ix).py_0().px_1().h(px(row_height())).child(
                h_flex()
                    .gap_1()
                    .pl(px(entry.depth() as f32 * 14.))
                    .text_sm()
                    .child(
                        Icon::new(match &path {
                            Some(p) => common::file_icon(p),
                            None => IconName::Folder,
                        })
                        .small()
                        .text_color(palette.text_secondary),
                    )
                    .child(div().text_color(color).child(item.label.clone())),
            )
        });

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(px(crate::ui::common::toolbar_height()))
                    .px_1()
                    .gap_0p5()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(tool_button("stash-new", IconName::Plus, "Stash Changes…").on_click(cx.listener(
                        |this, _, window, cx| dialogs::stash(this.model.clone(), window, cx),
                    )))
                    .child(tool_button("stash-apply", IconName::Check, "Apply").disabled(!has_selection).on_click(
                        cx.listener(|this, _, _, cx| this.stash_op("Apply Stash", "apply", cx)),
                    ))
                    .child(tool_button("stash-pop", IconName::ArrowUpFromLine, "Pop").disabled(!has_selection).on_click(
                        cx.listener(|this, _, _, cx| this.stash_op("Pop Stash", "pop", cx)),
                    ))
                    .child(tool_button("stash-unstash-as", IconName::GitBranch, "Unstash As…").disabled(!has_selection).on_click(
                        cx.listener(|this, _, window, cx| this.unstash_as(window, cx)),
                    ))
                    .child(tool_button("stash-drop", IconName::X, "Drop…").disabled(!has_selection).on_click(
                        cx.listener(|this, _, window, cx| this.drop_stash(window, cx)),
                    ))
                    .child(tool_button("stash-clear", IconName::Delete, "Clear…").disabled(self.stashes.is_empty()).on_click(
                        cx.listener(|this, _, window, cx| this.clear(window, cx)),
                    ))
                    .child(tool_button("stash-refresh", IconName::RefreshCw, "Refresh").on_click(cx.listener(
                        |this, _, _, cx| this.reload(cx),
                    ))),
            )
            .child(
                div().flex_1().min_h_0().child(
                    v_resizable("stash-split")
                        .child(resizable_panel().child(div().id("stash-list").size_full().overflow_y_scrollbar().child(list)))
                        .child(resizable_panel().size(px(220.)).child(files.size_full())),
                ),
            )
    }
}
