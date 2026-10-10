//! The commit context menu.

use super::*;

pub(super) fn commit_menu(
    menu: gpui_kit::component::menu::PopupMenu,
    entity: &Entity<LogView>,
    commit: &Commit,
    _: &mut Window,
    cx: &mut App,
) -> gpui_kit::component::menu::PopupMenu {
    let model = entity.read(cx).model.clone();
    let selected = entity.read(cx).selected_commits(cx);
    let multi = selected.len() > 1;
    let head = model.read(cx).refs().head_commit.clone();
    let is_head = head.as_deref() == Some(commit.hash.as_str());
    let hash = commit.hash.clone();
    let short = commit.short_hash().to_owned();
    // The newest loaded commit that has this one as a parent.
    let child = model.read(cx).commits().iter().find(|c| c.parents.contains(&commit.hash)).map(|c| c.hash.clone());
    let web = model.read(cx).web_repo().cloned();

    let op = |title: &'static str, args: Vec<String>, done: String| {
        let model = model.clone();
        move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
            let args = args.clone();
            let done = done.clone();
            model.update(cx, |model, cx| {
                model.run_operation(title, move |repo| {
                    repo.run(&args)?;
                    Ok(done)
                }, cx)
            });
        }
    };

    let picks: Vec<String> = selected.iter().map(|c| c.hash.clone()).collect();
    let copy_text = picks.iter().rev().cloned().collect::<Vec<_>>().join("\n");
    let mut cherry_pick = vec!["cherry-pick".to_owned()];
    cherry_pick.extend(picks.iter().cloned());
    let picked = if multi { format!("Cherry-picked {} commits", picks.len()) } else { format!("Cherry-picked {short}") };
    // Edit Message, Drop, Squash, Fixup and Interactively Rebase rewrite the
    // current branch: IntelliJ greys them out for commits not on it.
    let on_branch = model.read(cx).repository().is_some_and(|repo| {
        if picks.len() <= 20 {
            picks.iter().all(|pick| crate::git::rebase::is_on_current_branch(repo, pick))
        } else {
            // Many selected (Ctrl+A): one rev-list instead of a check each.
            let on_head: HashSet<String> = repo.run(["rev-list", "HEAD"]).unwrap_or_default().lines().map(str::to_owned).collect();
            picks.iter().all(|pick| on_head.contains(pick))
        }
    });
    // Push All up to Here: commits of the current branch (not detached).
    let can_push_here = model.read(cx).refs().current_branch.is_some()
        && model.read(cx).repository().is_some_and(|repo| crate::git::rebase::is_on_current_branch(repo, &commit.hash));
    let compare_model = model.clone();
    let compare = if selected.len() == 2 { Some((selected[0].hash.clone(), selected[1].hash.clone())) } else { None };
    let local_model = model.clone();
    let local_hash = hash.clone();
    // A file's History tab: IntelliJ puts the file's own actions first
    // (Show Diff, Open Repository Version, Annotate Revision, Get, …).
    let history_path = {
        let model = model.read(cx);
        let filter = model.filter();
        (filter.paths.len() == 1 && filter.lines.is_none() && !multi).then(|| {
            // The file's name in that commit, if it was renamed since (in
            // its own root's terms, in a multi-root project).
            let path = model.route(&filter.paths[0]).map(|(_, own)| own).unwrap_or_else(|| filter.paths[0].clone());
            model
                .details()
                .filter(|d| d.hash == commit.hash && d.changes.len() == 1)
                .map(|d| d.changes[0].path.clone())
                .or_else(|| {
                    // Follow the file back from HEAD to that commit.
                    let output = model.repository()?.run(["log", "--follow", "--name-only", "--format=%H", "HEAD", "--", &path]).ok()?;
                    let mut current = "";
                    for line in output.lines().filter(|l| !l.is_empty()) {
                        if line.len() == 40 && line.bytes().all(|b| b.is_ascii_hexdigit()) {
                            current = line;
                        } else if current == commit.hash {
                            return Some(line.to_owned());
                        }
                    }
                    None
                })
                .unwrap_or(path)
        })
    };
    // The file's actions already compare with the local file; the
    // branch-rewriting actions and navigation stay in the Log's own menu,
    // so the History menu keeps to IntelliJ's shorter list.
    let history = history_path.is_some();
    let menu = match history_path {
        Some(path) => change_menu(menu, entity, &path, &hash, cx).separator(),
        None => menu,
    };
    menu.item(PopupMenuItem::new("Copy Revision Number").on_click(move |_, _, cx| {
        cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()))
    }))
    .when(!history, |menu| {
        menu.item(PopupMenuItem::new("Compare with Local").disabled(multi).on_click(move |_, _, cx| {
            let hash = local_hash.clone();
            local_model.update(cx, |m, cx| m.compare(hash, None, cx))
        }))
    })
    .when_some(compare, |menu, (old, new)| {
        menu.item(PopupMenuItem::new("Compare Versions").on_click(move |_, _, cx| {
            let (old, new) = (old.clone(), new.clone());
            compare_model.update(cx, |m, cx| m.compare(old, Some(new), cx))
        }))
    })
    .item(PopupMenuItem::new("Show Repository at Revision").disabled(multi).on_click({
        let (model, hash, entity) = (model.clone(), hash.clone(), entity.clone());
        move |_, window, cx| {
            let entity = entity.clone();
            let open_file: crate::ui::revision_browser::OpenFile = std::rc::Rc::new(move |revision, path, _, cx| {
                entity.update(cx, |_, cx| cx.emit(LogEvent::OpenFile { path, revision: Some(revision) }))
            });
            crate::ui::revision_browser::open(model.clone(), hash.clone(), open_file, window, cx)
        }
    }))
    .item(PopupMenuItem::new("Create Patch…").on_click({
        let model = model.clone();
        let oldest = picks.first().cloned().unwrap_or_default();
        let newest = picks.last().cloned().unwrap_or_default();
        let label = if multi { format!("{}_{}", &oldest[..oldest.len().min(8)], &newest[..newest.len().min(8)]) } else { short.clone() };
        move |_, window, cx| {
            let Some(repository) = model.read(cx).repository().cloned() else { return };
            let old = crate::git::patch::parent_of(&repository, &oldest);
            let source = crate::ui::patch_dialogs::PatchSource::Commits { old, new: newest.clone(), label: label.clone() };
            crate::ui::patch_dialogs::create_patch(model.clone(), source, window, cx)
        }
    }))
    .separator()
    // Commits already on the current branch have nothing to pick.
    .item(PopupMenuItem::new("Cherry-Pick").disabled(on_branch).on_click({
        let model = model.clone();
        move |_, _, cx| {
            let (mut args, picked) = (cherry_pick.clone(), picked.clone());
            let settings = crate::settings::Settings::get(cx).clone();
            model.update(cx, |model, cx| {
                model.run_operation("Cherry-Pick", move |repo| {
                    // IntelliJ adds "(cherry picked from commit …)" for commits
                    // already pushed to a protected branch.
                    if settings.cherry_pick_suffix && args[1..].iter().any(|hash| crate::git::refs::remote_branches_containing(repo, hash).iter().any(|b| settings.is_protected(b))) {
                        args.insert(1, "-x".into());
                    }
                    crate::git::ops::cherry_pick_skipping_empty(repo, &args, picked)
                }, cx)
            });
        }
    }))
    .item(PopupMenuItem::new("Checkout Revision").disabled(multi).on_click(op(
        "Checkout",
        vec!["checkout".into(), "--detach".into(), hash.clone()],
        format!("Checked out {short}"),
    )))
    .when(!history, |menu| {
        menu
        .separator()
        .item(PopupMenuItem::new("Reset Current Branch to Here…").disabled(multi).on_click({
            let model = model.clone();
            let hash = hash.clone();
            move |_, window, cx| dialogs::reset_to(model.clone(), hash.clone(), window, cx)
        }))
        .item(PopupMenuItem::new(if multi { "Revert Commits" } else { "Revert Commit" }).on_click(op(
            "Revert",
            // Newest first, so each revert applies cleanly on top of the last.
            ["revert".to_owned(), "--no-edit".to_owned()].into_iter().chain(picks.iter().rev().cloned()).collect(),
            if multi { format!("Reverted {} commits", picks.len()) } else { format!("Reverted {short}") },
        )))
        .item(PopupMenuItem::new("Undo Commit…").disabled(!is_head || multi).on_click(op(
            "Undo Commit",
            vec!["reset".into(), "--soft".into(), "HEAD~1".into()],
            "Commit undone; changes kept in the working tree".into(),
        )))
        .item(PopupMenuItem::new("Edit Commit Message…").disabled(multi || !on_branch).on_click({
            let model = model.clone();
            let hash = hash.clone();
            move |_, window, cx| rebase_dialog::reword(model.clone(), hash.clone(), window, cx)
        }))
        .item(PopupMenuItem::new(if multi { "Drop Commits" } else { "Drop Commit" }).disabled(!on_branch).on_click({
            let model = model.clone();
            let picks = picks.clone();
            move |_, window, cx| rebase_dialog::drop_commits(model.clone(), picks.clone(), window, cx)
        }))
        .item(PopupMenuItem::new("Squash Commits…").disabled(!multi || !on_branch).on_click({
            let model = model.clone();
            let picks = picks.clone();
            move |_, window, cx| rebase_dialog::squash(model.clone(), picks.clone(), window, cx)
        }))
        .item(PopupMenuItem::new("Fixup…").disabled(multi || !on_branch).on_click({
            let model = model.clone();
            let message = format!("fixup! {}", commit.subject);
            move |_, _, cx| model.update(cx, |m, cx| m.prefill_commit_message(message.clone(), cx))
        }))
        .item(PopupMenuItem::new("Squash Into…").disabled(multi || !on_branch).on_click({
            let model = model.clone();
            let message = format!("squash! {}", commit.subject);
            move |_, _, cx| model.update(cx, |m, cx| m.prefill_commit_message(message.clone(), cx))
        }))
        .item(PopupMenuItem::new("Interactively Rebase from Here…").disabled(multi || !on_branch).on_click({
            let model = model.clone();
            let hash = hash.clone();
            move |_, window, cx| rebase_dialog::open(model.clone(), hash.clone(), window, cx)
        }))
        .separator()
        // Opens the Push dialog with the commits up to this one, as IntelliJ
        // does, also for a branch without an upstream yet.
        .item(PopupMenuItem::new("Push All up to Here…").disabled(multi || !can_push_here).on_click({
            let model = model.clone();
            let hash = hash.clone();
            move |_, window, cx| dialogs::push_up_to(model.clone(), Some(hash.clone()), window, cx)
        }))
    })
    .separator()
    .item(PopupMenuItem::new("New Branch…").disabled(multi).on_click({
        let model = model.clone();
        let hash = hash.clone();
        move |_, window, cx| dialogs::new_branch(model.clone(), hash.clone(), window, cx)
    }))
    .item(PopupMenuItem::new("New Tag…").disabled(multi).on_click({
        let model = model.clone();
        let hash = hash.clone();
        move |_, window, cx| dialogs::new_tag(model.clone(), hash.clone(), window, cx)
    }))
    .when(!history, |menu| {
        menu
        .separator()
        .item(PopupMenuItem::new("Go to Child Commit").disabled(child.is_none() || multi).on_click({
            let model = model.clone();
            move |_, _, cx| model.update(cx, |m, cx| m.select_hash(child.clone(), cx))
        }))
        .item(PopupMenuItem::new("Go to Parent Commit").disabled(commit.parents.is_empty() || multi).on_click({
            let model = model.clone();
            let parent = commit.parents.first().cloned();
            move |_, _, cx| model.update(cx, |m, cx| m.select_hash(parent.clone(), cx))
        }))
    })
    .when_some(web.filter(|_| !multi), |menu, web| {
        let url = web.commit_url(&hash);
        menu.separator().item(PopupMenuItem::new(format!("Open on {}", web.host.name())).on_click(move |_, _, cx| cx.open_url(&url)))
    })
}
