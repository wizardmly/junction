//! GitHub actions that open a dialog: Share Project on GitHub, Create Gist
//! and Create Pull Request, as the GitHub plugin of Android Studio has them.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Sizable as _, WindowExt as _, h_flex,
    button::Button,
    checkbox::Checkbox,
    input::{Input, InputState, Textarea, TextareaState},
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::{App, AppContext as _, ClipboardItem, Entity, ParentElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px};

use crate::git::Repository;
use crate::hosting::account::{self, Account, Service};
use crate::hosting::github::Client;
use crate::model::{RepoEvent, RepoModel};
use crate::theme::ActivePalette as _;
use crate::ui::dialogs::{focus_input, footer};
use crate::ui::pull_requests::PrTarget;

/// The GitHub accounts, or the Accounts dialog (then `None`) when there are none.
fn github_accounts(window: &mut Window, cx: &mut App) -> Option<Vec<Account>> {
    let accounts: Vec<Account> = account::load().into_iter().filter(|a| a.service == Service::GitHub).collect();
    if accounts.is_empty() {
        crate::ui::accounts_dialog::accounts(None, window, cx);
        return None;
    }
    Some(accounts)
}

fn notify(model: &Entity<RepoModel>, title: &str, message: String, error: bool, cx: &mut App) {
    let title = title.to_owned();
    model.update(cx, |_, cx| cx.emit(RepoEvent::Notify { title, message, error }));
}

/// "Share by:" — the account picker shown when there is more than one.
fn account_picker(id: &'static str, accounts: &[Account], chosen: &Rc<Cell<usize>>) -> impl gpui_kit::IntoElement {
    let label = accounts.get(chosen.get()).map(|a| format!("{} ({})", a.login, a.server)).unwrap_or_default();
    let (accounts, chosen) = (accounts.to_vec(), chosen.clone());
    Button::new(id).small().outline().label(label).icon(IconName::ChevronDown).dropdown_menu(move |mut menu, _, _| {
        for (ix, a) in accounts.iter().enumerate() {
            let chosen = chosen.clone();
            menu = menu.item(PopupMenuItem::new(format!("{} ({})", a.login, a.server)).checked(chosen.get() == ix).on_click(move |_, window, _| {
                chosen.set(ix);
                window.refresh();
            }));
        }
        menu
    })
}

/// Git › GitHub › Share Project on GitHub: creates the repository, adds it as
/// a remote, makes an initial commit when there is none, and pushes.
pub fn share_project(model: Entity<RepoModel>, window: &mut Window, cx: &mut App) {
    let Some(accounts) = github_accounts(window, cx) else { return };
    let Some(repo) = model.read(cx).repository().cloned() else { return };
    let folder = repo.root().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let has_origin = repo.run(["remote"]).map(|r| r.lines().any(|l| l == "origin")).unwrap_or(false);
    let name = cx.new(|cx| InputState::new(window, cx).default_value(folder));
    let remote = cx.new(|cx| InputState::new(window, cx).default_value(if has_origin { "github" } else { "origin" }));
    let description = cx.new(|cx| InputState::new(window, cx));
    let private = Rc::new(Cell::new(false));
    let chosen = Rc::new(Cell::new(0usize));
    let focus = name.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let secondary = cx.palette().text_secondary;
        let label = |text: &'static str| div().w(px(120.)).flex_none().text_sm().text_color(secondary).child(text);
        let (private_cell, private_ok, chosen_ok) = (private.clone(), private.clone(), chosen.clone());
        let (name_ok, remote_ok, description_ok, accounts_ok, model) = (name.clone(), remote.clone(), description.clone(), accounts.clone(), model.clone());
        dialog
            .title("Share Project on GitHub")
            .w(px(480.))
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        h_flex().gap_2().child(label("Repository name:")).child(div().flex_1().child(Input::new(&name).small())).child(
                            Checkbox::new("share-private").label("Private").checked(private.get()).on_change(move |v, window, _| {
                                private_cell.set(*v);
                                window.refresh();
                            }),
                        ),
                    )
                    .child(h_flex().gap_2().child(label("Remote:")).child(div().flex_1().child(Input::new(&remote).small())))
                    .child(h_flex().gap_2().child(label("Description:")).child(div().flex_1().child(Input::new(&description).small())))
                    .when(accounts.len() > 1, |el| el.child(h_flex().gap_2().child(label("Share by:")).child(account_picker("share-account", &accounts, &chosen)))),
            )
            .on_ok(move |_, _, cx| {
                let name = name_ok.read(cx).value().trim().to_owned();
                let remote = remote_ok.read(cx).value().trim().to_owned();
                if name.is_empty() || remote.is_empty() {
                    return false;
                }
                let description = description_ok.read(cx).value().trim().to_owned();
                let account = accounts_ok[chosen_ok.get().min(accounts_ok.len() - 1)].clone();
                let private = private_ok.get();
                model.update(cx, |m, cx| {
                    m.run_operation("Share Project on GitHub", move |repo| share(repo, &account, &name, private, &description, &remote), cx)
                });
                true
            })
            .footer(footer("Share"))
    });
    focus_input(&focus, window, cx);
}

fn share(repo: &Repository, account: &Account, name: &str, private: bool, description: &str, remote: &str) -> anyhow::Result<String> {
    let created = Client::new(account).create_repo(name, private, description)?;
    repo.run(["remote", "add", remote, &created.clone_url])?;
    if repo.run(["rev-parse", "--verify", "--quiet", "HEAD"]).is_err() {
        repo.run(["add", "--all"])?;
        repo.run(["commit", "-m", "Initial commit"])?;
    }
    let branch = repo.run(["symbolic-ref", "--short", "HEAD"])?.trim().to_owned();
    repo.run(["push", "--set-upstream", remote, &branch])?;
    Ok(format!("Successfully shared project on GitHub: {}", created.html_url))
}

/// Create Gist: one or more files, a description, secret by default; the
/// gist's URL is opened in the browser or copied.
pub fn create_gist(model: Entity<RepoModel>, files: Vec<(String, String)>, window: &mut Window, cx: &mut App) {
    if files.is_empty() {
        return;
    }
    let Some(accounts) = github_accounts(window, cx) else { return };
    let single = files.len() == 1;
    let filename = cx.new(|cx| InputState::new(window, cx).default_value(files[0].0.clone()));
    let description = cx.new(|cx| TextareaState::new(window, cx).rows(3));
    let secret = Rc::new(Cell::new(true));
    let open = Rc::new(Cell::new(true));
    let copy = Rc::new(Cell::new(false));
    let chosen = Rc::new(Cell::new(0usize));
    let files = Rc::new(RefCell::new(files));
    let focus = filename.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let secondary = cx.palette().text_secondary;
        let label = |text: &'static str| div().w(px(90.)).text_sm().text_color(secondary).child(text);
        let check = |id: &'static str, text: &'static str, cell: &Rc<Cell<bool>>| {
            let cell = cell.clone();
            Checkbox::new(id).label(text).checked(cell.get()).on_change(move |v, window, _| {
                cell.set(*v);
                window.refresh();
            })
        };
        let (filename_ok, description_ok, files_ok, accounts_ok, model) = (filename.clone(), description.clone(), files.clone(), accounts.clone(), model.clone());
        let (secret_ok, open_ok, copy_ok, chosen_ok) = (secret.clone(), open.clone(), copy.clone(), chosen.clone());
        dialog
            .title("Create Gist")
            .w(px(480.))
            .child(
                v_flex()
                    .gap_2()
                    .when(single, |el| el.child(h_flex().gap_2().child(label("Filename:")).child(div().flex_1().child(Input::new(&filename).small()))))
                    .when(!single, |el| el.child(div().text_sm().text_color(secondary).child(format!("{} files", files.borrow().len()))))
                    .child(v_flex().gap_1().child(label("Description:")).child(Textarea::new(&description)))
                    .child(h_flex().gap_4().child(check("gist-secret", "Secret", &secret)).child(check("gist-open", "Open in browser", &open)).child(check("gist-copy", "Copy URL", &copy)))
                    .when(accounts.len() > 1, |el| el.child(h_flex().gap_2().child(label("Create for:")).child(account_picker("gist-account", &accounts, &chosen)))),
            )
            .on_ok(move |_, _, cx| {
                let mut files = files_ok.borrow().clone();
                if single {
                    let name = filename_ok.read(cx).value().trim().to_owned();
                    if !name.is_empty() {
                        files[0].0 = name;
                    }
                }
                let description = description_ok.read(cx).value().trim().to_owned();
                let account = accounts_ok[chosen_ok.get().min(accounts_ok.len() - 1)].clone();
                let (public, open, copy) = (!secret_ok.get(), open_ok.get(), copy_ok.get());
                let model = model.clone();
                cx.spawn(async move |cx| {
                    let result = cx.background_spawn(async move { Client::new(&account).create_gist(&description, public, &files) }).await;
                    cx.update(|cx| match result {
                        Ok(url) => {
                            if open {
                                cx.open_url(&url);
                            }
                            if copy {
                                cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
                            }
                            notify(&model, "Create Gist", format!("Gist created: {url}"), false, cx);
                        }
                        Err(error) => notify(&model, "Create Gist failed", error.to_string(), true, cx),
                    });
                })
                .detach();
                true
            })
            .footer(footer("Create"))
    });
    focus_input(&focus, window, cx);
}

/// The base branch a new pull request targets: the remote's default branch.
fn default_base(repo: &Repository, remote: &str) -> String {
    if let Ok(head) = repo.run(["symbolic-ref", "--short", &format!("refs/remotes/{remote}/HEAD")]) {
        if let Some((_, branch)) = head.trim().split_once('/') {
            return branch.to_owned();
        }
    }
    for candidate in ["main", "master", "develop"] {
        if repo.run(["rev-parse", "--verify", "--quiet", &format!("refs/remotes/{remote}/{candidate}")]).is_ok() {
            return candidate.to_owned();
        }
    }
    "main".into()
}

/// Title and description from the commits the PR would bring: a single
/// commit gives both; several give the branch's first commit subject.
fn default_message(repo: &Repository, remote: &str, base: &str) -> (String, String) {
    let range = format!("{remote}/{base}..HEAD");
    let subjects = repo.run(["log", "--reverse", "--format=%s", &range]).unwrap_or_default();
    let subjects: Vec<&str> = subjects.lines().collect();
    match subjects.as_slice() {
        [] => (String::new(), String::new()),
        [only] => (only.to_string(), repo.run(["log", "-1", "--format=%b"]).map(|b| b.trim().to_owned()).unwrap_or_default()),
        [first, ..] => (first.to_string(), subjects.iter().map(|s| format!("- {s}")).collect::<Vec<_>>().join("\n")),
    }
}

/// Create Pull Request: base and head branches, title, description, draft.
/// The head branch is pushed first.
pub fn create_pull_request(model: Entity<RepoModel>, target: PrTarget, on_created: Rc<dyn Fn(&mut App)>, window: &mut Window, cx: &mut App) {
    let Some(repo) = model.read(cx).repository().cloned() else { return };
    let Some(head) = model.read(cx).refs().current_branch.clone() else {
        notify(&model, "Create Pull Request", "Check out a branch to create a pull request from".into(), true, cx);
        return;
    };
    let base_name = default_base(&repo, &target.remote);
    let (title_text, body_text) = default_message(&repo, &target.remote, &base_name);
    let remote_branches: Vec<String> = model
        .read(cx)
        .refs()
        .remote_branches()
        .filter_map(|r| r.name.strip_prefix(&format!("{}/", target.remote)).map(str::to_owned))
        .filter(|n| n != "HEAD")
        .collect();
    let base = cx.new(|cx| InputState::new(window, cx).default_value(base_name));
    let title = cx.new(|cx| InputState::new(window, cx).default_value(title_text).placeholder("Title"));
    let body = cx.new(|cx| TextareaState::new(window, cx).rows(6).default_value(body_text).placeholder("Description"));
    let draft = Rc::new(Cell::new(false));
    let focus = title.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let secondary = cx.palette().text_secondary;
        let label = |text: &'static str| div().w(px(60.)).text_sm().text_color(secondary).child(text);
        let draft_cell = draft.clone();
        let (base_ok, title_ok, body_ok, draft_ok, menu_base) = (base.clone(), title.clone(), body.clone(), draft.clone(), base.clone());
        let (model, target, head, on_created, branches) = (model.clone(), target.clone(), head.clone(), on_created.clone(), remote_branches.clone());
        dialog
            .title("Create Pull Request")
            .w(px(560.))
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(label("Base:"))
                            .child(div().flex_1().child(Input::new(&base).small()))
                            .child(Button::new("cpr-bases").small().outline().icon(IconName::ChevronDown).dropdown_menu(move |mut menu, _, _| {
                                for name in &branches {
                                    let (input, name) = (menu_base.clone(), name.clone());
                                    menu = menu.item(PopupMenuItem::new(name.clone()).on_click(move |_, window, cx| input.update(cx, |s, cx| s.set_value(name.clone(), window, cx))));
                                }
                                menu
                            }))
                            .child(div().text_sm().text_color(secondary).child(format!("← {}:{head}", target.repo.split('/').next().unwrap_or_default()))),
                    )
                    .child(h_flex().gap_2().child(label("Title:")).child(div().flex_1().child(Input::new(&title).small())))
                    .child(Textarea::new(&body))
                    .child(Checkbox::new("cpr-draft").label("Create draft pull request").checked(draft.get()).on_change(move |v, window, _| {
                        draft_cell.set(*v);
                        window.refresh();
                    })),
            )
            .on_ok(move |_, _, cx| {
                let title = title_ok.read(cx).value().trim().to_owned();
                let base = base_ok.read(cx).value().trim().to_owned();
                if title.is_empty() || base.is_empty() {
                    return false;
                }
                let body = body_ok.read(cx).value().to_string();
                let draft = draft_ok.get();
                let (target, head, on_created, model) = (target.clone(), head.clone(), on_created.clone(), model.clone());
                let Some(repo) = model.read(cx).repository().cloned() else { return true };
                cx.spawn(async move |cx| {
                    let result = cx
                        .background_spawn(async move {
                            repo.run(["push", "--set-upstream", &target.remote, &head])?;
                            Ok::<_, anyhow::Error>(Client::new(&target.account).create_pull(&target.repo, &title, &body, &head, &base, draft)?)
                        })
                        .await;
                    cx.update(|cx| match result {
                        Ok(pr) => {
                            notify(&model, "Create Pull Request", format!("Pull request #{} created: {}", pr.number, pr.html_url), false, cx);
                            model.update(cx, |m, cx| m.reload(cx));
                            on_created(cx);
                        }
                        Err(error) => notify(&model, "Create Pull Request failed", error.to_string(), true, cx),
                    });
                })
                .detach();
                true
            })
            .footer(footer("Create Pull Request"))
    });
    focus_input(&focus, window, cx);
}
