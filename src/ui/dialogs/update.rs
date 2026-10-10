//! Update Project.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::{
    WindowExt as _,
    checkbox::Checkbox,
    radio::RadioGroup,
    v_flex,
};
use gpui_kit::{
    App, Entity, ParentElement as _,
    Styled as _, Window, div, px,
};

use crate::model::RepoModel;
use crate::settings::{Settings, UpdateMethod};
use crate::theme::ActivePalette as _;

use super::footer;

/// Update Project (`Ctrl+T`): fetch, then merge or rebase, cleaning the
/// working tree with stash or shelve. Both choices are remembered; after
/// "Don't show again" Ctrl+T updates with them straight away.
pub fn update_project(model: Entity<RepoModel>, window: &mut Window, cx: &mut App) {
    let settings = Settings::get(cx);
    if !settings.update_dialog {
        let clean = if settings.update_shelve { crate::git::ops::CleanWith::Shelve } else { crate::git::ops::CleanWith::Stash };
        run_update(model, settings.update_method == UpdateMethod::Rebase, clean, cx);
        return;
    }
    let rebase = Rc::new(Cell::new(settings.update_method == UpdateMethod::Rebase));
    let shelve = Rc::new(Cell::new(settings.update_shelve));
    let hide = Rc::new(Cell::new(false));
    window.open_dialog(cx, move |dialog, _, cx| {
        let secondary = cx.palette().text_secondary;
        let (rebase_cell, rebase_ok) = (rebase.clone(), rebase.clone());
        let (shelve_cell, shelve_ok) = (shelve.clone(), shelve.clone());
        let (hide_cell, hide_ok) = (hide.clone(), hide.clone());
        let model = model.clone();
        dialog
            .title("Update Project")
            .w(px(440.))
            .child(
                v_flex()
                    .gap_3()
                    .child(div().text_sm().text_color(secondary).child("Update type"))
                    .child(
                        RadioGroup::new("update-type")
                            .children(["Merge incoming changes into the current branch", "Rebase the current branch on top of incoming changes"])
                            .selected_index(Some(if rebase.get() { 1 } else { 0 }))
                            .on_change(move |ix, window, _| {
                                rebase_cell.set(*ix == 1);
                                window.refresh();
                            }),
                    )
                    .child(div().text_sm().text_color(secondary).child("Clean working tree before update"))
                    .child(
                        RadioGroup::new("update-clean")
                            .children(["Using Stash", "Using Shelve"])
                            .selected_index(Some(if shelve.get() { 1 } else { 0 }))
                            .on_change(move |ix, window, _| {
                                shelve_cell.set(*ix == 1);
                                window.refresh();
                            }),
                    )
                    .child(Checkbox::new("update-hide").label("Don't show again").checked(hide.get()).on_change(move |v, window, _| {
                        hide_cell.set(*v);
                        window.refresh();
                    })),
            )
            .on_ok(move |_, _, cx| {
                let rebase = rebase_ok.get();
                let shelve = shelve_ok.get();
                let method = if rebase { UpdateMethod::Rebase } else { UpdateMethod::Merge };
                let settings = Settings::get(cx);
                if settings.update_method != method || settings.update_shelve != shelve || hide_ok.get() {
                    let hide = hide_ok.get();
                    Settings::update(cx, |s| {
                        s.update_method = method;
                        s.update_shelve = shelve;
                        if hide {
                            s.update_dialog = false;
                        }
                    });
                }
                let clean = if shelve { crate::git::ops::CleanWith::Shelve } else { crate::git::ops::CleanWith::Stash };
                run_update(model.clone(), rebase, clean, cx);
                true
            })
            .footer(footer("OK"))
    });
}

/// Starts an operation's message when it partly failed (some roots of
/// Update Project): the balloon is a warning, not a success.
pub const PARTIAL_FAILURE: char = '\u{1d}';

/// Runs Update Project; multi-root projects update every root, as IntelliJ does.
fn run_update(model: Entity<RepoModel>, rebase: bool, clean: crate::git::ops::CleanWith, cx: &mut App) {
    model.update(cx, |model, cx| {
        let roots: Vec<std::path::PathBuf> = model.roots().iter().map(|r| r.path.clone()).collect();
        let project = model.project_root().map(std::path::Path::to_path_buf);
        model.run_operation("Update Project", move |repo| {
            if roots.len() < 2 {
                return crate::git::ops::update_project(repo, rebase, clean);
            }
            let project = project.unwrap_or_else(|| repo.root().to_path_buf());
            let mut lines = Vec::new();
            let mut failed = Vec::new();
            for root in &roots {
                let label = crate::git::roots::label(&project, root);
                let result = repo.nested(&root.to_string_lossy()).and_then(|r| crate::git::ops::update_project(&r, rebase, clean));
                match result {
                    Ok(message) => lines.push(format!("{label}: {}", message.split('\u{1f}').next().unwrap_or_default())),
                    Err(error) => failed.push(format!("{label}: {error}")),
                }
            }
            if !failed.is_empty() && lines.is_empty() {
                anyhow::bail!("{}", failed.join("\n"));
            }
            let partial = !failed.is_empty();
            lines.extend(failed.into_iter().map(|f| format!("{f} (failed)")));
            let message = lines.join("\n");
            Ok(if partial { format!("{PARTIAL_FAILURE}{message}") } else { message })
        }, cx)
    });
}
