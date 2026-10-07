//! Get from Version Control (Clone) and Create Git Repository.

use std::path::{Path, PathBuf};

use gpui_kit::component::{
    Sizable as _, WindowExt as _, h_flex,
    button::Button,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::{
    InteractiveElement as _, StatefulInteractiveElement as _,
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, PathPromptOptions, Render, Styled as _,
    Subscription, Task, Window, div, prelude::FluentBuilder as _, px,
};

use crate::model::RepoModel;
use crate::theme::ActivePalette as _;
use crate::ui::dialogs::{focus_input, footer};

/// The folder name git would pick for a URL: `…/repo.git` → `repo`.
pub fn directory_name(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/').trim_end_matches(".git");
    trimmed.rsplit(['/', ':', '\\']).next().unwrap_or_default().to_owned()
}

fn default_parent() -> PathBuf {
    if let Some(parent) = crate::settings::recent_projects().first().and_then(|p| p.parent().map(Path::to_path_buf)) {
        return parent;
    }
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

enum TestState {
    Idle,
    Testing,
    Ok,
    Failed(String),
}

pub struct CloneView {
    url: Entity<InputState>,
    /// Repositories of the logged-in GitHub accounts, picked to fill the URL.
    repos: Vec<crate::hosting::github::Repo>,
    repos_state: Option<String>,
    repo_filter: Entity<InputState>,
    _repos: Option<Task<()>>,
    directory: Entity<InputState>,
    parent: PathBuf,
    /// The directory follows the URL until the user edits it.
    directory_edited: bool,
    syncing: bool,
    test: TestState,
    _test: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl CloneView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let parent = default_parent();
        let url = cx.new(|cx| InputState::new(window, cx).placeholder("https://github.com/owner/repository.git"));
        let directory = cx.new(|cx| InputState::new(window, cx).default_value(parent.display().to_string()));
        let repo_filter = cx.new(|cx| InputState::new(window, cx).placeholder("Search repositories"));
        let accounts: Vec<_> = crate::hosting::account::load()
            .into_iter()
            .filter(|a| a.service == crate::hosting::account::Service::GitHub)
            .collect();
        let repos_task = (!accounts.is_empty()).then(|| {
            cx.spawn(async move |this, cx| {
                let result = cx
                    .background_spawn(async move {
                        let mut all = Vec::new();
                        for account in &accounts {
                            all.extend(crate::hosting::github::Client::new(account).repos()?);
                        }
                        anyhow::Ok(all)
                    })
                    .await;
                this.update(cx, |this, cx| {
                    match result {
                        Ok(repos) => {
                            this.repos_state = repos.is_empty().then(|| "No repositories".into());
                            this.repos = repos;
                        }
                        Err(error) => this.repos_state = Some(error.to_string()),
                    }
                    cx.notify();
                })
                .ok();
            })
        });
        let subscriptions = vec![
            cx.subscribe(&repo_filter, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(&url, window, |this, _, event: &InputEvent, window, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                this.url_changed(window, cx);
            }),
            cx.subscribe(&directory, |this, _, event: &InputEvent, _| {
                if matches!(event, InputEvent::Change) && !this.syncing {
                    this.directory_edited = true;
                }
            }),
        ];
        Self {
            url,
            repos: Vec::new(),
            repos_state: repos_task.as_ref().map(|_| "Loading repositories…".into()),
            repo_filter,
            _repos: repos_task,
            directory,
            parent,
            directory_edited: false,
            syncing: false,
            test: TestState::Idle,
            _test: None,
            _subscriptions: subscriptions,
        }
    }

    /// The directory follows the URL until edited.
    fn url_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.test = TestState::Idle;
        if !self.directory_edited {
            let name = directory_name(self.url.read(cx).value().as_ref());
            let value = self.parent.join(name).display().to_string();
            self.syncing = true;
            self.directory.update(cx, |state, cx| state.set_value(value, window, cx));
            self.syncing = false;
        }
        cx.notify();
    }

    fn test(&mut self, cx: &mut Context<Self>) {
        let url = self.url.read(cx).value().trim().to_owned();
        if url.is_empty() {
            return;
        }
        self.test = TestState::Testing;
        cx.notify();
        self._test = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let console = crate::git::GitConsole::default();
                    crate::git::run_in(&crate::git::executable(), &std::env::temp_dir(), &console, ["ls-remote", "--heads", url.as_str()], None, &[])
                })
                .await;
            this.update(cx, |this, cx| {
                this.test = match result {
                    Ok(_) => TestState::Ok,
                    Err(error) => TestState::Failed(error.to_string().lines().last().unwrap_or_default().to_owned()),
                };
                cx.notify();
            })
            .ok();
        }));
    }

    fn browse(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Directory".into()),
        });
        let directory = self.directory.clone();
        let url = self.url.clone();
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else { return };
            let Some(parent) = paths.into_iter().next() else { return };
            cx.update(|cx| {
                let name = directory_name(url.read(cx).value().as_ref());
                let value = if name.is_empty() { parent.clone() } else { parent.join(name) };
                if let Some(window) = cx.active_window() {
                    window
                        .update(cx, |_, window, cx| directory.update(cx, |s, cx| s.set_value(value.display().to_string(), window, cx)))
                        .ok();
                }
                this.update(cx, |this, _| {
                    this.parent = parent;
                    this.directory_edited = true;
                })
                .ok();
            });
        })
        .detach();
    }

    fn request(&self, cx: &App) -> Option<(String, PathBuf)> {
        let url = self.url.read(cx).value().trim().to_owned();
        let directory = PathBuf::from(self.directory.read(cx).value().trim());
        (!url.is_empty() && !directory.as_os_str().is_empty()).then_some((url, directory))
    }

    fn directory_error(&self, cx: &App) -> Option<&'static str> {
        let directory = PathBuf::from(self.directory.read(cx).value().trim());
        let not_empty = std::fs::read_dir(&directory).map(|mut d| d.next().is_some()).unwrap_or(false);
        not_empty.then_some("Directory already exists and isn't empty")
    }
}

impl Render for CloneView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let label = |text: &'static str| div().w(px(70.)).text_sm().child(text);
        let (status, color) = match &self.test {
            TestState::Idle => (None, palette.text_secondary),
            TestState::Testing => (Some("Connecting…".to_owned()), palette.text_secondary),
            TestState::Ok => (Some("Connection successful".to_owned()), palette.status_added),
            TestState::Failed(error) => (Some(error.clone()), palette.status_conflict),
        };
        let directory_error = self.directory_error(cx);
        let query = self.repo_filter.read(cx).value().trim().to_lowercase();
        let current = self.url.read(cx).value().trim().to_owned();
        let show_repos = self._repos.is_some();
        let mut list = v_flex();
        for (ix, repo) in self.repos.iter().enumerate().filter(|(_, r)| query.is_empty() || r.full_name.to_lowercase().contains(&query)) {
            let url = repo.clone_url.clone();
            let selected = url == current;
            list = list.child(
                h_flex()
                    .id(("clone-repo", ix))
                    .h(px(24.))
                    .px_2()
                    .gap_2()
                    .text_sm()
                    .cursor_pointer()
                    .when(selected, |el| el.bg(palette.selection))
                    .hover(|el| el.bg(palette.hover))
                    .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().child(repo.full_name.clone()))
                    .when(repo.private, |el| el.child(div().text_xs().text_color(palette.text_secondary).child("private")))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let url = url.clone();
                        this.url.update(cx, |s, cx| s.set_value(url, window, cx));
                        this.url_changed(window, cx);
                    })),
            );
        }
        v_flex()
            .gap_3()
            .when(show_repos, |el| {
                el.child(
                    v_flex()
                        .gap_1()
                        .child(div().text_sm().font_weight(gpui_kit::FontWeight::SEMIBOLD).child("GitHub"))
                        .child(Input::new(&self.repo_filter).small().cleanable(true))
                        .child(
                            div()
                                .id("clone-repos")
                                .h(px(160.))
                                .rounded(px(4.))
                                .border_1()
                                .border_color(palette.border)
                                .overflow_y_scroll()
                                .when_some(self.repos_state.clone(), |el, state| el.child(div().p_2().text_xs().text_color(palette.text_secondary).child(state)))
                                .child(list),
                        ),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .child(label("URL:"))
                    .child(div().flex_1().child(Input::new(&self.url).small()))
                    .child(Button::new("clone-test").small().label("Test").on_click(cx.listener(|this, _, _, cx| this.test(cx)))),
            )
            .when_some(status, |el, status| el.child(div().pl(px(78.)).text_xs().text_color(color).child(status)))
            .child(
                h_flex()
                    .gap_2()
                    .child(label("Directory:"))
                    .child(div().flex_1().child(Input::new(&self.directory).small()))
                    .child(Button::new("clone-browse").small().label("…").on_click(cx.listener(|this, _, _, cx| this.browse(cx)))),
            )
            .when_some(directory_error, |el, error| {
                el.child(div().pl(px(78.)).text_xs().text_color(palette.status_conflict).child(error))
            })
    }
}

pub fn clone(model: Entity<RepoModel>, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| CloneView::new(window, cx));
    let focus = view.read(cx).url.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let view_ok = view.clone();
        let model = model.clone();
        dialog
            .title("Get from Version Control")
            .w(px(600.))
            .child(view.clone())
            .on_ok(move |_, _, cx| {
                let view = view_ok.read(cx);
                if view.directory_error(cx).is_some() {
                    return false;
                }
                let Some((url, directory)) = view.request(cx) else { return false };
                model.update(cx, |m, cx| m.clone_repository(url, directory, cx));
                true
            })
            .footer(footer("Clone"))
    });
    focus_input(&focus, window, cx);
}

/// Create Git Repository: pick a folder and `git init` it.
pub fn init(model: Entity<RepoModel>, cx: &mut App) {
    let receiver = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Create Git Repository".into()),
    });
    cx.spawn(async move |cx| {
        let Ok(Ok(Some(paths))) = receiver.await else { return };
        let Some(dir) = paths.into_iter().next() else { return };
        model.update(cx, |m, cx| m.init_repository(dir, cx));
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::directory_name;

    #[test]
    fn names_the_clone_directory_like_git() {
        assert_eq!(directory_name("https://github.com/zed-industries/zed.git"), "zed");
        assert_eq!(directory_name("git@github.com:owner/repo.git"), "repo");
        assert_eq!(directory_name("https://example.com/a/b/"), "b");
        assert_eq!(directory_name("/srv/git/project"), "project");
    }
}
