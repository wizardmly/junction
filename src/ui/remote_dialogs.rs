//! Git › Pull… and Git › Manage Remotes…, after IntelliJ's dialogs.

use gpui_kit::component::{
    Disableable as _, Sizable as _, WindowExt as _, h_flex,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    input::{Input, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::assets::IconName;
use gpui_kit::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};

use crate::git::remotes::{self, Remote};
use crate::model::RepoModel;
use crate::theme::ActivePalette as _;
use crate::ui::dialogs::{focus_input, footer};

/// The Pull dialog's "Modify options", in IntelliJ's order.
const PULL_OPTIONS: [(&str, &str); 6] = [
    ("--rebase", "Rebase incoming changes on top of the current branch"),
    ("--ff-only", "Fast-forward only"),
    ("--no-ff", "Create a merge commit even when fast-forward is possible"),
    ("--squash", "Squash commits into a single change"),
    ("--no-commit", "Merge without committing"),
    ("--no-verify", "Skip pre-merge and commit-msg hooks"),
];

/// Options that can't be combined with option `ix` (IntelliJ greys them out).
fn pull_conflicts(ix: usize) -> &'static [usize] {
    match ix {
        0 => &[1, 2, 3, 4],
        1 => &[0, 2, 3],
        2 => &[0, 1, 3],
        3 => &[0, 1, 2],
        4 => &[0],
        _ => &[],
    }
}

pub struct PullView {
    model: Entity<RepoModel>,
    remotes: Vec<Remote>,
    remote: Option<String>,
    branch: Entity<InputState>,
    options: [bool; 6],
}

impl PullView {
    fn new(model: Entity<RepoModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let m = model.read(cx);
        let remotes = m.repository().and_then(|r| remotes::list(r).ok()).unwrap_or_default();
        // Preselect the current branch's upstream, as IntelliJ does.
        let refs = m.refs();
        let upstream = refs
            .current_branch
            .as_ref()
            .and_then(|name| refs.find(&format!("refs/heads/{name}")))
            .and_then(|r| r.upstream.clone());
        let (remote, branch) = match upstream.as_deref().and_then(|u| u.split_once('/')) {
            Some((remote, branch)) => (Some(remote.to_owned()), branch.to_owned()),
            None => (remotes.first().map(|r| r.name.clone()), String::new()),
        };
        let options = [crate::settings::Settings::get(cx).update_method == crate::settings::UpdateMethod::Rebase, false, false, false, false, false];
        let branch = cx.new(|cx| InputState::new(window, cx).placeholder("Branch").default_value(branch));
        Self { model, remotes, remote, branch, options }
    }

    fn args(&self, cx: &App) -> Option<Vec<String>> {
        let remote = self.remote.clone()?;
        let mut args = vec!["pull".to_owned()];
        if !self.options[0] {
            // Without --rebase, merge explicitly so pull.rebase in the config can't surprise.
            args.push("--no-rebase".into());
        }
        args.extend(PULL_OPTIONS.iter().zip(self.options).filter(|(_, on)| *on).map(|((o, _), _)| o.to_string()));
        args.push("--autostash".into());
        args.push(remote);
        let branch = self.branch.read(cx).value().trim().to_owned();
        if !branch.is_empty() {
            args.push(branch);
        }
        Some(args)
    }
}

impl Render for PullView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let entity = cx.entity();
        let remote_names: Vec<String> = self.remotes.iter().map(|r| r.name.clone()).collect();
        let remote = self.remote.clone();
        let prefix = remote.clone().map(|r| format!("{r}/")).unwrap_or_default();
        let branches: Vec<String> = self
            .model
            .read(cx)
            .refs()
            .remote_branches()
            .filter_map(|r| r.name.strip_prefix(&prefix).map(str::to_owned))
            .collect();
        let branch_input = self.branch.clone();
        let mut options = v_flex().gap_1();
        for (ix, (option, hint)) in PULL_OPTIONS.iter().enumerate() {
            let disabled = pull_conflicts(ix).iter().any(|&other| self.options[other]);
            options = options.child(
                h_flex()
                    .gap_2()
                    .child(
                        Checkbox::new(SharedString::from(format!("pull-opt-{ix}")))
                            .label(*option)
                            .checked(self.options[ix])
                            .disabled(disabled)
                            .on_change(cx.listener(move |this, v: &bool, _, cx| {
                                this.options[ix] = *v;
                                cx.notify();
                            })),
                    )
                    .child(div().text_xs().text_color(palette.text_secondary).child(*hint)),
            );
        }
        let command = self.args(cx).map(|a| format!("git {}", a.join(" "))).unwrap_or_default();
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("pull-remote")
                            .outline()
                            .small()
                            .label(remote.clone().unwrap_or_else(|| "No remotes".into()))
                            .disabled(remote_names.is_empty())
                            .dropdown_menu(move |mut menu, _, _| {
                                for name in &remote_names {
                                    let entity = entity.clone();
                                    let name = name.clone();
                                    menu = menu.item(PopupMenuItem::new(name.clone()).on_click(move |_, _, cx| {
                                        entity.update(cx, |this, cx| {
                                            this.remote = Some(name.clone());
                                            cx.notify();
                                        })
                                    }));
                                }
                                menu
                            }),
                    )
                    .child(div().flex_1().child(Input::new(&self.branch).small()))
                    .child(Button::new("pull-branches").outline().small().icon(IconName::ChevronDown).dropdown_menu(move |mut menu, _, _| {
                        for branch in &branches {
                            let input = branch_input.clone();
                            let branch = branch.clone();
                            menu = menu.item(PopupMenuItem::new(branch.clone()).on_click(move |_, window, cx| {
                                input.update(cx, |s, cx| s.set_value(branch.clone(), window, cx))
                            }));
                        }
                        menu
                    })),
            )
            .child(div().text_sm().text_color(palette.text_secondary).child("Options"))
            .child(options)
            .child(div().text_xs().font_family(cx.theme_mono()).text_color(palette.text_secondary).child(command))
    }
}

trait MonoFont {
    fn theme_mono(&self) -> SharedString;
}

impl MonoFont for App {
    fn theme_mono(&self) -> SharedString {
        use gpui_kit::component::ActiveTheme as _;
        self.theme().mono_font_family.clone()
    }
}

/// Git › Pull…: remote, branch and options; runs `git pull`.
pub fn pull(model: Entity<RepoModel>, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| PullView::new(model.clone(), window, cx));
    let current = model.read(cx).refs().current_branch.clone().unwrap_or_else(|| "HEAD".into());
    let focus = view.read(cx).branch.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let (view_ok, model) = (view.clone(), model.clone());
        dialog
            .title(format!("Pull to {current}"))
            .w(px(560.))
            .child(view.clone())
            .footer(footer("Pull"))
            .on_ok(move |_, _, cx| {
                let Some(args) = view_ok.read(cx).args(cx) else { return false };
                model.update(cx, |model, cx| {
                    model.run_operation("Pull", move |repo| {
                        let before = repo.run(["rev-parse", "HEAD"]).unwrap_or_default();
                        repo.run(&args)?;
                        let after = repo.run(["rev-parse", "HEAD"]).unwrap_or_default();
                        if before == after {
                            return Ok("Already up to date".into());
                        }
                        let (before, after) = (before.trim(), after.trim());
                        // The fetched branch the pull merged or rebased onto.
                        let incoming = repo.run(["rev-parse", "FETCH_HEAD"]).map(|o| o.trim().to_owned()).unwrap_or_else(|_| after.to_owned());
                        Ok(format!(
                            "{}\u{1f}{}",
                            crate::git::ops::updated_summary(repo, before, after, &incoming),
                            crate::git::ops::updated_ranges(before, after, &incoming)
                        ))
                    }, cx)
                });
                true
            })
    });
    focus_input(&focus, window, cx);
}

/// Manage Remotes: the remotes table with Add / Edit / Remove, and an
/// inline form in place of IntelliJ's nested Define Remote dialog.
pub struct RemotesView {
    model: Entity<RepoModel>,
    remotes: Vec<Remote>,
    selected: Option<usize>,
    /// `Some(None)` while adding, `Some(Some(name))` while editing `name`.
    editing: Option<Option<String>>,
    name: Entity<InputState>,
    url: Entity<InputState>,
    error: Option<String>,
    /// The URL is being checked with `git ls-remote` before saving.
    checking: bool,
    /// Remove asks first: the remote awaiting confirmation.
    confirm_remove: Option<String>,
    _task: Option<gpui_kit::Task<()>>,
}

impl RemotesView {
    fn new(model: Entity<RepoModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("origin"));
        let url = cx.new(|cx| InputState::new(window, cx).placeholder("https://github.com/owner/repository.git"));
        let mut this = Self {
            model,
            remotes: Vec::new(),
            selected: None,
            editing: None,
            name,
            url,
            error: None,
            checking: false,
            confirm_remove: None,
            _task: None,
        };
        this.reload(cx);
        this
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.remotes = self.model.read(cx).repository().and_then(|r| remotes::list(r).ok()).unwrap_or_default();
        self.selected = self.selected.filter(|&ix| ix < self.remotes.len());
        cx.notify();
    }

    fn start_edit(&mut self, remote: Option<Remote>, window: &mut Window, cx: &mut Context<Self>) {
        let (name, url) = match &remote {
            Some(r) => (r.name.clone(), r.fetch_url.clone()),
            // The first remote is usually "origin", as IntelliJ suggests.
            None => (if self.remotes.is_empty() { "origin".into() } else { String::new() }, String::new()),
        };
        let focus = if name.is_empty() { &self.name } else { &self.url };
        focus_input(focus, window, cx);
        self.name.update(cx, |s, cx| s.set_value(name, window, cx));
        self.url.update(cx, |s, cx| s.set_value(url, window, cx));
        self.editing = Some(remote.map(|r| r.name));
        self.confirm_remove = None;
        self.error = None;
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(editing) = self.editing.clone() else { return };
        let name = self.name.read(cx).value().trim().to_owned();
        let url = self.url.read(cx).value().trim().to_owned();
        if name.is_empty() || url.is_empty() {
            self.error = Some("Name and URL are required".into());
            cx.notify();
            return;
        }
        let Some(repository) = self.model.read(cx).repository().cloned() else { return };
        // A new URL is checked first (IntelliJ's "Checking URL…"); a rename
        // that keeps the URL isn't.
        let unchanged = editing.as_ref().is_some_and(|old| self.remotes.iter().any(|r| &r.name == old && r.fetch_url == url));
        self.checking = true;
        self.error = None;
        cx.notify();
        self._task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if !unchanged {
                        remotes::check_url(&repository, &url)?;
                    }
                    match &editing {
                        None => remotes::add(&repository, &name, &url),
                        Some(old) => remotes::edit(&repository, old, &name, &url),
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                this.checking = false;
                match result {
                    Ok(()) => {
                        this.editing = None;
                        this.error = None;
                        this.model.update(cx, |m, cx| m.reload(cx));
                    }
                    Err(error) => this.error = Some(error.to_string().lines().last().unwrap_or_default().to_owned()),
                }
                this.reload(cx);
            })
            .ok();
        }));
    }

    fn remove(&mut self, cx: &mut Context<Self>) {
        let Some(name) = self.confirm_remove.take() else { return };
        let Some(repository) = self.model.read(cx).repository().cloned() else { return };
        match remotes::remove(&repository, &name) {
            Ok(()) => {
                self.error = None;
                self.model.update(cx, |m, cx| m.reload(cx));
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        self.reload(cx);
    }
}

impl Render for RemotesView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let selected = self.selected;
        let has_selection = selected.is_some();
        let mut table = v_flex().min_h(px(140.)).border_1().border_color(palette.border).rounded_md().overflow_hidden();
        if self.remotes.is_empty() {
            table = table.child(div().p_3().text_sm().text_color(palette.text_secondary).child("No remotes"));
        }
        for (ix, remote) in self.remotes.iter().enumerate() {
            let url = match &remote.push_url {
                Some(push) => format!("{}  (push: {push})", remote.fetch_url),
                None => remote.fetch_url.clone(),
            };
            let edit_remote = remote.clone();
            table = table.child(
                h_flex()
                    .id(SharedString::from(format!("remote-{ix}")))
                    .px_2()
                    .py_1()
                    .gap_3()
                    .text_sm()
                    .when(selected == Some(ix), |el| el.bg(palette.selection))
                    .on_click(cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                        this.selected = Some(ix);
                        if event.click_count() == 2 {
                            this.start_edit(Some(edit_remote.clone()), window, cx);
                        }
                        cx.notify();
                    }))
                    .child(div().w(px(110.)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child(remote.name.clone()))
                    .child(div().flex_1().text_color(palette.text_secondary).child(url)),
            );
        }
        let editing = self.editing.clone();
        let checking = self.checking;
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_1()
                    .child(Button::new("remote-add").ghost().small().icon(IconName::Plus).tooltip("Add").on_click(cx.listener(
                        |this, _, window, cx| this.start_edit(None, window, cx),
                    )))
                    .child(Button::new("remote-remove").ghost().small().icon(IconName::Minus).tooltip("Remove").disabled(!has_selection).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.confirm_remove = this.selected.and_then(|ix| this.remotes.get(ix)).map(|r| r.name.clone());
                            this.editing = None;
                            this.error = None;
                            cx.notify();
                        }),
                    ))
                    .child(Button::new("remote-edit").ghost().small().icon(IconName::Settings2).tooltip("Edit").disabled(!has_selection).on_click(
                        cx.listener(|this, _, window, cx| {
                            let remote = this.selected.and_then(|ix| this.remotes.get(ix)).cloned();
                            if remote.is_some() {
                                this.start_edit(remote, window, cx);
                            }
                        }),
                    )),
            )
            .child(table)
            .when_some(self.confirm_remove.clone(), |el, name| {
                el.child(
                    h_flex()
                        .gap_2()
                        .p_2()
                        .border_1()
                        .border_color(palette.border)
                        .rounded_md()
                        .text_sm()
                        .child(div().flex_1().child(format!("Remove remote '{name}'?")))
                        .child(Button::new("remote-remove-cancel").small().label("Cancel").on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_remove = None;
                            cx.notify();
                        })))
                        .child(Button::new("remote-remove-ok").primary().small().label("Remove").on_click(cx.listener(|this, _, _, cx| this.remove(cx)))),
                )
            })
            .when_some(editing, |el, editing| {
                el.child(
                    v_flex()
                        .gap_2()
                        .p_2()
                        .border_1()
                        .border_color(palette.border)
                        .rounded_md()
                        .child(div().text_sm().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(if editing.is_some() { "Edit Remote" } else { "Define Remote" }))
                        .child(h_flex().gap_2().child(div().w(px(50.)).text_sm().child("Name:")).child(div().flex_1().child(Input::new(&self.name).small())))
                        .child(h_flex().gap_2().child(div().w(px(50.)).text_sm().child("URL:")).child(div().flex_1().child(Input::new(&self.url).small())))
                        .child(
                            h_flex()
                                .gap_2()
                                .justify_end()
                                .child(Button::new("remote-cancel").small().label("Cancel").on_click(cx.listener(|this, _, _, cx| {
                                    this.editing = None;
                                    this.error = None;
                                    cx.notify();
                                })))
                                .when(checking, |el| el.child(div().text_xs().text_color(palette.text_secondary).child("Checking URL…")))
                                .child(
                                    Button::new("remote-save")
                                        .primary()
                                        .small()
                                        .label("OK")
                                        .disabled(checking)
                                        .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                                ),
                        ),
                )
            })
            .when_some(self.error.clone(), |el, error| el.child(div().text_xs().text_color(palette.status_conflict).child(error)))
    }
}

pub fn manage_remotes(model: Entity<RepoModel>, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| RemotesView::new(model, window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog.title("Git Remotes").w(px(620.)).child(view.clone()).footer(
            gpui_kit::component::dialog::DialogFooter::new()
                .child(gpui_kit::component::dialog::DialogClose::new().child(Button::new("remotes-close").label("Close").primary())),
        )
    });
}

/// Branch › Edit Tracking Branch…: pick the remote branch a local branch tracks.
/// An empty value removes the tracking information.
pub fn edit_tracking_branch(model: Entity<RepoModel>, branch: String, upstream: Option<String>, window: &mut Window, cx: &mut App) {
    let remote_branches: Vec<String> = model.read(cx).refs().remote_branches().map(|r| r.name.clone()).collect();
    let input = cx.new(|cx| {
        InputState::new(window, cx).placeholder("No tracking branch").default_value(upstream.clone().unwrap_or_default())
    });
    let focus_target = input.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let secondary = cx.palette().text_secondary;
        let (ok_input, menu_input, model, branch_ok) = (input.clone(), input.clone(), model.clone(), branch.clone());
        let branches = remote_branches.clone();
        let previous = upstream.clone();
        dialog
            .title(format!("Edit Tracking Branch of '{branch}'"))
            .w(px(440.))
            .child(
                v_flex()
                    .gap_2()
                    .child(div().text_sm().text_color(secondary).child("Remote branch"))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().flex_1().child(Input::new(&input)))
                            .child(Button::new("tracking-branches").outline().icon(IconName::ChevronDown).dropdown_menu(move |mut menu, _, _| {
                                for name in &branches {
                                    let input = menu_input.clone();
                                    let name = name.clone();
                                    menu = menu.item(PopupMenuItem::new(name.clone()).on_click(move |_, window, cx| {
                                        input.update(cx, |s, cx| s.set_value(name.clone(), window, cx))
                                    }));
                                }
                                menu
                            })),
                    ),
            )
            .on_ok(move |_, _, cx| {
                let target = ok_input.read(cx).value().trim().to_owned();
                if Some(target.as_str()) == previous.as_deref() || (target.is_empty() && previous.is_none()) {
                    return true;
                }
                let branch = branch_ok.clone();
                model.update(cx, |model, cx| {
                    model.run_operation("Edit Tracking Branch", move |repo| {
                        if target.is_empty() {
                            repo.run(["branch", "--unset-upstream", &branch])?;
                            Ok(format!("{branch} no longer tracks a remote branch"))
                        } else {
                            repo.run(["branch", &format!("--set-upstream-to={target}"), &branch])?;
                            Ok(format!("{branch} now tracks {target}"))
                        }
                    }, cx)
                });
                true
            })
            .footer(footer("OK"))
    });
    focus_input(&focus_target, window, cx);
}
