//! The main menu (☰).

use super::*;

/// The new UI's main menu (☰), grouped as IntelliJ's menu bar: File, View,
/// Navigate, Git, Window, Help. Items backed by actions show their shortcut.
pub(super) fn main_menu(
    menu: gpui_kit::component::menu::PopupMenu,
    entity: Entity<Workspace>,
    focus: gpui_kit::FocusHandle,
    window: &mut Window,
    cx: &mut Context<gpui_kit::component::menu::PopupMenu>,
) -> gpui_kit::component::menu::PopupMenu {
    let on = |entity: &Entity<Workspace>, f: fn(&mut Workspace, &mut Window, &mut Context<Workspace>)| {
        let entity = entity.clone();
        move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| entity.update(cx, |this, cx| f(this, window, cx))
    };
    let model_op = |entity: &Entity<Workspace>, f: fn(Entity<RepoModel>, &mut Window, &mut App)| {
        let entity = entity.clone();
        move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| {
            let model = entity.read(cx).model.clone();
            f(model, window, cx)
        }
    };
    let e = entity.clone();
    let f = focus.clone();
    let menu = menu.action_context(focus.clone()).submenu("File", window, cx, move |menu, window, cx| {
        let e2 = e.clone();
        menu.action_context(f.clone())
            .item(PopupMenuItem::new("Open…").on_click(on(&e, |this, window, cx| this.open_repository(window, cx))))
            .item(PopupMenuItem::new("Get from Version Control…").on_click(model_op(&e, clone_dialog::clone)))
            .item(PopupMenuItem::new("Create Git Repository…").on_click(model_op(&e, |model, _, cx| clone_dialog::init(model, cx))))
            .submenu("Recent Projects", window, cx, move |mut menu, _, cx| {
                let current = e2.read(cx).model.read(cx).repository().map(|r| r.root().to_path_buf());
                let recent = crate::settings::recent_projects();
                if recent.is_empty() {
                    return menu.item(PopupMenuItem::new("No recent projects").disabled(true));
                }
                for path in recent.into_iter().take(15) {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let entity = e2.clone();
                    let is_current = current.as_deref() == Some(path.as_path());
                    menu = menu.item(PopupMenuItem::new(name).checked(is_current).on_click(move |_, _, cx| {
                        let path = path.clone();
                        let model = entity.read(cx).model.clone();
                        model.update(cx, |m, cx| m.open(path, cx));
                    }));
                }
                menu
            })
            .separator()
            .menu("Settings…", Box::new(OpenSettings))
            .separator()
            .item(PopupMenuItem::new("Exit").on_click(|_, _, cx| cx.quit()))
    });
    let (e, f) = (entity.clone(), focus.clone());
    let menu = menu.submenu("View", window, cx, move |menu, window, cx| {
        let f2 = f.clone();
        let e2 = e.clone();
        let prs_title = e.read(cx).prs.read(cx).title();
        menu.action_context(f.clone())
            .submenu("Tool Windows", window, cx, move |menu, _, _| {
                let e3 = e2.clone();
                menu.action_context(f2.clone())
                    .menu("Project", Box::new(ToggleProjectWindow))
                    .menu("Commit", Box::new(ToggleCommitWindow))
                    .menu("Git", Box::new(ToggleGitWindow))
                    .menu("Find", Box::new(ToggleFindWindow))
                    .item(PopupMenuItem::new(prs_title).on_click(move |_, window, cx| {
                        e3.update(cx, |this, cx| this.toggle_tool(ToolWindow::PullRequests, window, cx))
                    }))
            })
            .menu("Hide All Tool Windows", Box::new(HideAllToolWindows))
            .separator()
            .submenu("Appearance", window, cx, |menu, _, cx| {
                let settings = Settings::get(cx).clone();
                let theme = |label: &'static str, dark: bool, system: bool, checked: bool| {
                    PopupMenuItem::new(label).checked(checked).on_click(move |_, window, cx| {
                        Settings::update(cx, |s| {
                            s.theme_follows_system = system;
                            if !system {
                                s.dark = dark;
                            }
                        });
                        theme::refresh(cx);
                        window.refresh();
                    })
                };
                menu.label("Theme")
                    .item(theme("Dark", true, false, !settings.theme_follows_system && settings.dark))
                    .item(theme("Light", false, false, !settings.theme_follows_system && !settings.dark))
                    .item(theme("Sync with OS", settings.dark, true, settings.theme_follows_system))
                    .separator()
                    .item(PopupMenuItem::new("Compact Mode").checked(settings.compact).on_click(|_, window, cx| {
                        Settings::update(cx, |s| s.compact = !s.compact);
                        window.refresh();
                    }))
            })
    });
    let f = focus.clone();
    let menu = menu.submenu("Navigate", window, cx, move |menu, _, _| {
        menu.action_context(f.clone())
            .menu("Search Everywhere", Box::new(SearchEverywhere))
            .menu("Find Action…", Box::new(FindAction))
            .separator()
            .menu("Class…", Box::new(GotoClass))
            .menu("File…", Box::new(GotoFile))
            .menu("Symbol…", Box::new(GotoSymbol))
            .menu("Line/Column…", Box::new(GotoLine))
            .separator()
            .menu("Recent Files", Box::new(RecentFiles))
            .menu("File Structure", Box::new(FileStructure))
            .separator()
            .menu("Back", Box::new(NavigateBack))
            .menu("Forward", Box::new(NavigateForward))
            .menu("Select in Project View", Box::new(SelectInProject))
            .separator()
            .menu("Find in Files…", Box::new(FindInPath))
            .menu("Replace in Files…", Box::new(ReplaceInPath))
    });
    let (e, f) = (entity.clone(), focus.clone());
    let menu = menu.submenu("Git", window, cx, move |menu, window, cx| {
        let e2 = e.clone();
        let e3 = e.clone();
        let f2 = f.clone();
        menu.action_context(f.clone())
            .menu("Commit…", Box::new(CommitChanges))
            .menu("Push…", Box::new(PushChanges))
            .menu("Update Project…", Box::new(UpdateProject))
            .item(PopupMenuItem::new("Pull…").on_click(model_op(&e, remote_dialogs::pull)))
            .item(PopupMenuItem::new("Fetch").on_click(model_op(&e, |model, _, cx| {
                model.update(cx, |model, cx| {
                    model.run_operation("Fetch", |repo| {
                        repo.run(&["fetch", "--all", "--prune"])?;
                        Ok("Fetched all remotes".to_owned())
                    }, cx)
                })
            })))
            .separator()
            .item(PopupMenuItem::new("Merge…").on_click(model_op(&e, dialogs::merge)))
            .item(PopupMenuItem::new("Rebase…").on_click(model_op(&e, dialogs::rebase)))
            .menu("Branches…", Box::new(ShowBranches))
            .item(PopupMenuItem::new("New Branch…").on_click(model_op(&e, |model, window, cx| dialogs::new_branch(model, "HEAD".into(), window, cx))))
            .item(PopupMenuItem::new("New Tag…").on_click(model_op(&e, |model, window, cx| dialogs::new_tag(model, "HEAD".into(), window, cx))))
            .item(PopupMenuItem::new("Reset HEAD…").on_click(model_op(&e, |model, window, cx| dialogs::reset_to(model, "HEAD".into(), window, cx))))
            .separator()
            // IntelliJ's Git › Current File, for the file in the editor.
            .submenu("Current File", window, cx, move |menu, _, cx| {
                let no_file = e3.read(cx).editor().is_none();
                menu.item(PopupMenuItem::new("Annotate with Git Blame").disabled(no_file).on_click(on(&e3, |this, _, cx| {
                    if let Some(editor) = this.editor().cloned() {
                        editor.update(cx, |editor, cx| editor.show_annotations(cx));
                    }
                })))
                .item(PopupMenuItem::new("Show History").disabled(no_file).on_click(on(&e3, |this, _, cx| {
                    if let Some(path) = this.editor().map(|e| e.read(cx).path().to_owned()) {
                        this.show_history(path, cx);
                    }
                })))
                .item(PopupMenuItem::new("Show Diff").disabled(no_file).on_click(on(&e3, |this, _, cx| {
                    if let Some(path) = this.editor().map(|e| e.read(cx).path().to_owned()) {
                        this.open_diff(crate::ui::diff_view::DiffSource::WorkingTree { path, unversioned: false }, cx);
                    }
                })))
            })
            .separator()
            .menu("Stash Changes…", Box::new(StashChanges))
            .item(PopupMenuItem::new("Unstash Changes…").on_click(on(&e, |this, window, cx| {
                this.show_left_tab(LeftTab::Stash, window, cx);
            })))
            .separator()
            .item(PopupMenuItem::new("Create Patch…").on_click(on(&e, |this, window, cx| {
                let commit = this.commit.clone();
                commit.update(cx, |c, cx| c.create_patch(window, cx))
            })))
            .item(PopupMenuItem::new("Apply Patch…").on_click(model_op(&e, |model, window, cx| patch_dialogs::apply_patch(model, false, window, cx))))
            .item(PopupMenuItem::new("Apply Patch from Clipboard…").on_click(model_op(&e, |model, window, cx| patch_dialogs::apply_patch(model, true, window, cx))))
            .separator()
            .item(PopupMenuItem::new("Manage Remotes…").on_click(model_op(&e, remote_dialogs::manage_remotes)))
            .item(PopupMenuItem::new("New Worktree…").on_click(model_op(&e, crate::ui::worktree_view::new_worktree)))
            .item(PopupMenuItem::new("Update Submodules").on_click(model_op(&e, |model, _, cx| {
                model.update(cx, |m, cx| {
                    m.run_operation("Update Submodules", |repo| {
                        crate::git::submodule::update(repo, &[])?;
                        Ok("Submodules updated".into())
                    }, cx)
                })
            })))
            .item(PopupMenuItem::new("Directory Mappings…").on_click(model_op(&e, crate::ui::mappings_dialog::directory_mappings)))
            .separator()
            // IntelliJ's Git › GitHub group.
            .submenu("GitHub / GitLab", window, cx, move |menu, _, cx| {
                let e3 = e2.clone();
                let (e4, e5, e6, e7) = (e2.clone(), e2.clone(), e2.clone(), e2.clone());
                let web = e2.read(cx).model.read(cx).web_repo().map(|w| w.base.clone());
                let has_file = e2.read(cx).editor().is_some();
                menu.item(PopupMenuItem::new("Share Project on GitHub…").on_click(model_op(&e2, crate::ui::github_dialogs::share_project)))
                    .item(PopupMenuItem::new("Create Pull Request…").on_click(move |_, window, cx| {
                        let prs = e4.read(cx).prs.clone();
                        prs.update(cx, |prs, cx| prs.create_pull_request(window, cx))
                    }))
                    .item(PopupMenuItem::new("View Pull Requests").on_click(move |_, window, cx| {
                        e5.update(cx, |this, cx| {
                            if !this.tools.is_open(ToolWindow::PullRequests) {
                                this.toggle_tool(ToolWindow::PullRequests, window, cx)
                            }
                        })
                    }))
                    .separator()
                    .item(PopupMenuItem::new("Open on GitHub / GitLab").disabled(web.is_none()).on_click(move |_, window, cx| {
                        // The file in the editor at its lines, else the repository's page.
                        match e6.read(cx).editor().cloned() {
                            Some(editor) => editor.update(cx, |editor, cx| editor.open_on_hosting(&crate::ui::file_editor::OpenOnHosting, window, cx)),
                            None => {
                                if let Some(web) = &web {
                                    cx.open_url(web)
                                }
                            }
                        }
                    }))
                    .item(PopupMenuItem::new("Create Gist…").disabled(!has_file).on_click(move |_, window, cx| {
                        if let Some(editor) = e7.read(cx).editor().cloned() {
                            editor.update(cx, |editor, cx| editor.create_gist(&crate::ui::file_editor::CreateGist, window, cx))
                        }
                    }))
                    .separator()
                    .item(PopupMenuItem::new("Manage Accounts…").on_click(move |_, window, cx| {
                        let prs = e3.read(cx).prs.clone();
                        crate::ui::accounts_dialog::accounts(
                            Some(Rc::new(move |cx: &mut App| prs.update(cx, |prs, cx| prs.refresh(cx)))),
                            window,
                            cx,
                        )
                    }))
            })
            .separator()
            .item(PopupMenuItem::new("VCS Operations…").action(Box::new(VcsOperations)))
            .action_context(f2)
    });
    let f = focus.clone();
    let menu = menu.submenu("Window", window, cx, move |menu, _, _| {
        menu.action_context(f.clone())
            .menu("Select Next Tab", Box::new(NextTab))
            .menu("Select Previous Tab", Box::new(PreviousTab))
            .menu("Close Tab", Box::new(CloseTab))
            .menu("Reopen Closed Tab", Box::new(ReopenClosedTab))
    });
    menu.submenu("Help", window, cx, move |menu, _, _| {
        menu.item(PopupMenuItem::new("Keyboard Shortcuts").on_click(|_, window, cx| dialogs::keymap_reference(window, cx)))
            .item(PopupMenuItem::new(format!("About {APP_NAME}")).on_click(|_, window, cx| dialogs::about(window, cx)))
    })
}
