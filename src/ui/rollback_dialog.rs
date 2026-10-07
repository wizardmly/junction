//! IntelliJ's Rollback Changes dialog (Ctrl+Alt+Z in the Commit tool window).

use std::collections::HashSet;

use gpui_kit::component::{
    Sizable as _, WindowExt as _, h_flex,
    checkbox::Checkbox,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};

use crate::git::StatusKind;
use crate::model::RepoModel;
use crate::theme::ActivePalette as _;
use crate::ui::common;
use crate::ui::dialogs::footer;

/// Where unstaged changes come back from (staging-area mode).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RollbackFrom {
    Head,
    Index,
}

pub struct RollbackView {
    files: Vec<(String, StatusKind)>,
    checked: HashSet<String>,
    delete_added: bool,
}

impl RollbackView {
    fn summary(&self) -> String {
        let count = |kinds: &[StatusKind]| self.files.iter().filter(|(p, k)| self.checked.contains(p) && kinds.contains(k)).count();
        let parts = [
            (count(&[StatusKind::Modified, StatusKind::Renamed, StatusKind::Conflicted]), "modified"),
            (count(&[StatusKind::Added]), "added"),
            (count(&[StatusKind::Deleted]), "deleted"),
        ];
        parts.iter().filter(|(n, _)| *n > 0).map(|(n, what)| format!("{n} {what}")).collect::<Vec<_>>().join(", ")
    }
}

impl Render for RollbackView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let has_added = self.files.iter().any(|(_, k)| *k == StatusKind::Added);
        let mut list = v_flex().max_h(px(260.)).min_h(px(80.)).p_1().border_1().border_color(palette.border).rounded_md();
        for (ix, (path, kind)) in self.files.iter().enumerate() {
            let p = path.clone();
            list = list.child(
                h_flex()
                    .gap_1()
                    .child(Checkbox::new(SharedString::from(format!("rb-{ix}"))).checked(self.checked.contains(path)).on_change(cx.listener(
                        move |this, v: &bool, _, cx| {
                            if *v {
                                this.checked.insert(p.clone());
                            } else {
                                this.checked.remove(&p);
                            }
                            cx.notify();
                        },
                    )))
                    .child(gpui_kit::component::Icon::new(common::file_icon(path)).xsmall().text_color(palette.text_secondary))
                    .child(div().text_sm().text_color(common::status_color(*kind, &palette)).child(path.clone())),
            );
        }
        v_flex()
            .gap_2()
            .child(list)
            .child(div().text_xs().text_color(palette.text_secondary).child(self.summary()))
            .when(has_added, |el| {
                el.child(Checkbox::new("rb-delete-added").label("Delete local copies of added files").checked(self.delete_added).on_change(
                    cx.listener(|this, v: &bool, _, cx| {
                        this.delete_added = *v;
                        cx.notify();
                    }),
                ))
            })
    }
}

/// Opens Rollback Changes for `files`; unversioned files are left out.
pub fn rollback(model: Entity<RepoModel>, files: Vec<(String, StatusKind)>, from: RollbackFrom, window: &mut Window, cx: &mut App) {
    let files: Vec<_> = files.into_iter().filter(|(_, k)| *k != StatusKind::Unversioned).collect();
    if files.is_empty() {
        model.update(cx, |m, cx| m.notify("Rollback", "No changes to roll back", true, cx));
        return;
    }
    let checked = files.iter().map(|(p, _)| p.clone()).collect();
    let view = cx.new(|_| RollbackView { files, checked, delete_added: false });
    window.open_dialog(cx, move |dialog, _, _| {
        let (view_ok, model) = (view.clone(), model.clone());
        dialog
            .title(if from == RollbackFrom::Index { "Rollback Unstaged Changes" } else { "Rollback Changes" })
            .w(px(520.))
            .child(view.clone())
            .footer(footer("Rollback"))
            .on_ok(move |_, _, cx| {
                let view = view_ok.read(cx);
                let paths: Vec<String> = view.files.iter().filter(|(p, _)| view.checked.contains(p)).map(|(p, _)| p.clone()).collect();
                if paths.is_empty() {
                    return false;
                }
                let delete_added = view.delete_added;
                model.update(cx, |m, cx| {
                    m.run_operation("Rollback", move |repo| {
                        match from {
                            RollbackFrom::Index => {
                                let mut args = vec!["restore".to_owned(), "--worktree".into(), "--".into()];
                                args.extend(paths.iter().cloned());
                                repo.run(&args)?;
                            }
                            RollbackFrom::Head => crate::git::patch::rollback_with(repo, &paths, delete_added)?,
                        }
                        Ok(format!("Rolled back {} file{}", paths.len(), if paths.len() == 1 { "" } else { "s" }))
                    }, cx)
                });
                true
            })
    });
}
