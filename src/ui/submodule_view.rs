//! The Submodules tab of the Git tool window (shown when the repository has
//! a `.gitmodules`): each submodule's commit and state, with Update,
//! Update All, Sync, Open and Open in New Window.

use gpui_kit::assets::IconName;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, h_flex, menu::{ContextMenuExt as _, PopupMenuItem}, scroll::ScrollableElement as _, v_flex};
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div, prelude::FluentBuilder as _, px,
};

use crate::git::submodule::{self, Submodule, SubmoduleState};
use crate::model::{RepoEvent, RepoModel};
use crate::theme::ActivePalette as _;
use crate::ui::common::{row_height, tool_button};

pub struct SubmoduleView {
    model: Entity<RepoModel>,
    submodules: Vec<Submodule>,
    selected: Option<usize>,
    _load: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl SubmoduleView {
    pub fn new(model: Entity<RepoModel>, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![cx.subscribe(&model, |this, _, event, cx| {
            if matches!(event, RepoEvent::Reloaded) {
                this.reload(cx);
            }
        })];
        let mut this = Self { model, submodules: Vec::new(), selected: None, _load: None, _subscriptions: subscriptions };
        this.reload(cx);
        this
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(repository) = self.model.read(cx).repository().cloned() else { return };
        self._load = Some(cx.spawn(async move |this, cx| {
            let list = cx.background_spawn(async move { submodule::list(&repository).unwrap_or_default() }).await;
            this.update(cx, |this, cx| {
                let previous = this.selected.and_then(|ix| this.submodules.get(ix)).map(|s| s.path.clone());
                this.submodules = list;
                this.selected = previous.and_then(|p| this.submodules.iter().position(|s| s.path == p));
                cx.notify();
            })
            .ok();
        }));
    }

    fn selected(&self) -> Option<Submodule> {
        self.selected.and_then(|ix| self.submodules.get(ix)).cloned()
    }

    fn update(&self, all: bool, cx: &mut Context<Self>) {
        let paths: Vec<String> = if all { Vec::new() } else { self.selected().map(|s| vec![s.path]).unwrap_or_default() };
        if !all && paths.is_empty() {
            return;
        }
        self.model.update(cx, |m, cx| {
            m.run_operation("Update Submodules", move |repo| {
                submodule::update(repo, &paths)?;
                Ok(if paths.is_empty() { "Submodules updated".into() } else { format!("Updated submodule {}", paths.join(", ")) })
            }, cx)
        });
    }

    fn sync(&self, cx: &mut Context<Self>) {
        self.model.update(cx, |m, cx| {
            m.run_operation("Sync Submodules", |repo| {
                submodule::sync(repo)?;
                Ok("Submodule URLs synchronized".into())
            }, cx)
        });
    }

    fn open(&self, new_window: bool, cx: &mut Context<Self>) {
        let Some(sub) = self.selected().filter(|s| s.state != SubmoduleState::Uninitialized) else { return };
        let Some(root) = self.model.read(cx).repository().map(|r| r.root().join(&sub.path)) else { return };
        if new_window {
            crate::open_project_window(Some(root), cx);
        } else {
            self.model.update(cx, |m, cx| m.open(root, cx));
        }
    }
}

impl Render for SubmoduleView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let selected = self.selected();
        let can_open = selected.as_ref().is_some_and(|s| s.state != SubmoduleState::Uninitialized);
        let entity = cx.entity();

        let mut rows = v_flex();
        for (ix, sub) in self.submodules.iter().enumerate() {
            let is_selected = self.selected == Some(ix);
            let initialized = sub.state != SubmoduleState::Uninitialized;
            let (state, state_color) = match sub.state {
                SubmoduleState::UpToDate => ("up to date", palette.text_secondary),
                SubmoduleState::Changed => ("checked out at another commit", palette.status_modified),
                SubmoduleState::Uninitialized => ("not initialized", palette.status_deleted),
                SubmoduleState::Conflict => ("conflict", palette.status_conflict),
            };
            let menu_entity = entity.clone();
            rows = rows.child(
                h_flex()
                    .id(SharedString::from(format!("submodule-{ix}")))
                    .h(px(row_height() + 4.))
                    .px_2()
                    .gap_2()
                    .text_sm()
                    .when(is_selected, |el| el.bg(palette.selection))
                    .when(!is_selected, |el| el.hover(|s| s.bg(palette.hover)))
                    .on_click(cx.listener(move |this, event: &gpui_kit::ClickEvent, _, cx| {
                        this.selected = Some(ix);
                        if event.click_count() == 2 {
                            this.open(true, cx);
                        }
                        cx.notify();
                    }))
                    .context_menu(move |menu, _, _| {
                        let (u, o, n) = (menu_entity.clone(), menu_entity.clone(), menu_entity.clone());
                        menu.item(PopupMenuItem::new(if initialized { "Update" } else { "Initialize and Update" }).on_click(move |_, _, cx| {
                            u.update(cx, |this, cx| {
                                this.selected = Some(ix);
                                this.update(false, cx)
                            })
                        }))
                        .separator()
                        .item(PopupMenuItem::new("Open").disabled(!initialized).on_click(move |_, _, cx| {
                            o.update(cx, |this, cx| {
                                this.selected = Some(ix);
                                this.open(false, cx)
                            })
                        }))
                        .item(PopupMenuItem::new("Open in New Window").disabled(!initialized).on_click(move |_, _, cx| {
                            n.update(cx, |this, cx| {
                                this.selected = Some(ix);
                                this.open(true, cx)
                            })
                        }))
                    })
                    .child(Icon::new(IconName::FolderGit2).small().text_color(palette.text_secondary))
                    .child(div().w(px(220.)).overflow_hidden().whitespace_nowrap().text_ellipsis().child(sub.path.clone()))
                    .child(div().w(px(90.)).font_family(gpui_kit::component::ActiveTheme::theme(&**cx).mono_font_family.clone()).text_color(palette.text_secondary).child(sub.commit[..sub.commit.len().min(8)].to_owned()))
                    .child(div().w(px(140.)).overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(palette.text_secondary).child(sub.describe.clone().unwrap_or_default()))
                    .child(div().w(px(200.)).text_color(state_color).child(state))
                    .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(palette.text_secondary).child(sub.url.clone().unwrap_or_default())),
            );
        }
        if self.submodules.is_empty() {
            rows = rows.child(div().p_3().text_sm().text_color(palette.text_secondary).child("This repository has no submodules"));
        }

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(px(crate::ui::common::toolbar_height()))
                    .px_1()
                    .gap_0p5()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(tool_button("sm-update-all", IconName::ArrowDownToLine, "Update All Submodules (init, recursive)").on_click(cx.listener(|this, _, _, cx| this.update(true, cx))))
                    .child(
                        tool_button("sm-update", IconName::Download, "Update Selected")
                            .disabled(selected.is_none())
                            .on_click(cx.listener(|this, _, _, cx| this.update(false, cx))),
                    )
                    .child(tool_button("sm-sync", IconName::RefreshCw, "Sync URLs").on_click(cx.listener(|this, _, _, cx| this.sync(cx))))
                    .child(
                        tool_button("sm-open", IconName::FolderOpen, "Open")
                            .disabled(!can_open)
                            .on_click(cx.listener(|this, _, _, cx| this.open(false, cx))),
                    )
                    .child(
                        tool_button("sm-open-new", IconName::PanelLeft, "Open in New Window")
                            .disabled(!can_open)
                            .on_click(cx.listener(|this, _, _, cx| this.open(true, cx))),
                    ),
            )
            .child(div().id("submodules").flex_1().min_h_0().overflow_y_scrollbar().child(rows))
    }
}
