//! Settings › Version Control › GitHub / GitLab: accounts logged in with a token.
//! "Log In" checks the token against the API before saving it.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable as _, Icon, Sizable as _, WindowExt as _, h_flex,
    button::{Button, ButtonVariants as _},
    input::{Input, InputState},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};

use crate::hosting::account::{self, Account, Service};
use crate::hosting::github;
use crate::theme::ActivePalette as _;

pub struct AccountsView {
    accounts: Vec<Account>,
    /// The kind of account being added.
    service: Service,
    server: Entity<InputState>,
    token: Entity<InputState>,
    adding: bool,
    checking: bool,
    error: Option<String>,
    /// Called after an account is added or removed (refreshes PR lists).
    on_change: Option<std::rc::Rc<dyn Fn(&mut App)>>,
}

impl AccountsView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let server = cx.new(|cx| InputState::new(window, cx).default_value("github.com"));
        let token = cx.new(|cx| InputState::new(window, cx).placeholder("Personal access token").masked(true));
        let accounts = account::load();
        let adding = accounts.is_empty();
        Self { accounts, service: Service::GitHub, server, token, adding, checking: false, error: None, on_change: None }
    }

    /// GitHub or GitLab: the server field follows unless the user typed one.
    fn set_service(&mut self, service: Service, window: &mut Window, cx: &mut Context<Self>) {
        let default = |s: Service| if s == Service::GitHub { "github.com" } else { "gitlab.com" };
        let current = self.server.read(cx).value().trim().to_owned();
        if current.is_empty() || current == default(self.service) {
            self.server.update(cx, |state, cx| state.set_value(default(service), window, cx));
        }
        self.service = service;
        cx.notify();
    }

    fn log_in(&mut self, cx: &mut Context<Self>) {
        let server = self.server.read(cx).value().trim().trim_start_matches("https://").trim_end_matches('/').to_owned();
        let token = self.token.read(cx).value().trim().to_owned();
        if server.is_empty() || token.is_empty() {
            self.error = Some("Enter the server and a token".into());
            cx.notify();
            return;
        }
        self.checking = true;
        self.error = None;
        cx.notify();
        let mut candidate = Account { service: self.service, server, login: String::new(), token };
        cx.spawn(async move |this, cx| {
            let probe = candidate.clone();
            let result = cx.background_spawn(async move { github::Client::new(&probe).user() }).await;
            this.update(cx, |this, cx| {
                this.checking = false;
                match result {
                    Ok(user) => {
                        candidate.login = user.login;
                        this.accounts.retain(|a| !(a.server == candidate.server && a.login == candidate.login));
                        this.accounts.push(candidate);
                        this.error = account::save(&this.accounts).err().map(|e| e.to_string());
                        this.adding = false;
                        if let Some(on_change) = this.on_change.clone() {
                            on_change(cx);
                        }
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for AccountsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mut list = v_flex().gap_px();
        for (ix, acc) in self.accounts.iter().enumerate() {
            list = list.child(
                h_flex()
                    .h(px(30.))
                    .px_2()
                    .gap_2()
                    .text_sm()
                    .child(Icon::new(IconName::GitPullRequest).small().text_color(palette.text_secondary))
                    .child(div().w(px(48.)).text_xs().text_color(palette.text_secondary).child(if acc.service == Service::GitLab { "GitLab" } else { "GitHub" }))
                    .child(div().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(acc.login.clone()))
                    .child(div().flex_1().text_color(palette.text_secondary).child(acc.server.clone()))
                    .child(Button::new(("account-remove", ix)).xsmall().outline().label("Remove").on_click(cx.listener(move |this, _, _, cx| {
                        this.accounts.remove(ix);
                        this.error = account::save(&this.accounts).err().map(|e| e.to_string());
                        if let Some(on_change) = this.on_change.clone() {
                            on_change(cx);
                        }
                        cx.notify();
                    }))),
            );
        }
        if self.accounts.is_empty() {
            list = list.child(div().p_2().text_sm().text_color(palette.text_secondary).child("No accounts"));
        }
        v_flex()
            .gap_3()
            .child(div().p_1().rounded(px(4.)).border_1().border_color(palette.border).child(list))
            .when(!self.adding, |el| {
                el.child(h_flex().child(Button::new("account-add").small().outline().icon(IconName::Plus).label("Add Account…").on_click(cx.listener(|this, _, _, cx| {
                    this.adding = true;
                    cx.notify();
                }))))
            })
            .when(self.adding, |el| {
                let server = self.server.read(cx).value().trim().to_owned();
                let gitlab = self.service == Service::GitLab;
                let host = if !server.is_empty() { server } else if gitlab { "gitlab.com".to_owned() } else { "github.com".to_owned() };
                let host = host.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/').to_owned();
                let (token_url, scopes) = if gitlab {
                    (
                        format!("https://{host}/-/user_settings/personal_access_tokens?name=Junction%20Studio&scopes=api,read_user,write_repository"),
                        "Generate token (scopes: api, read_user, write_repository)",
                    )
                } else {
                    (
                        format!("https://{host}/settings/tokens/new?scopes=repo,gist,read:org,workflow&description=Junction%20Studio"),
                        "Generate token (scopes: repo, gist, read:org, workflow)",
                    )
                };
                let service = self.service;
                el.child(
                    v_flex()
                        .gap_2()
                        .p_2()
                        .rounded(px(4.))
                        .border_1()
                        .border_color(palette.border)
                        .child(div().text_sm().font_weight(gpui_kit::FontWeight::SEMIBOLD).child("Log In with Token"))
                        .child(
                            h_flex()
                                .gap_2()
                                .child(div().w(px(60.)).text_sm().text_color(palette.text_secondary).child("Service:"))
                                .child(Button::new("account-github").xsmall().label("GitHub").map(|b| if service == Service::GitHub { b.primary() } else { b.outline() }).on_click(
                                    cx.listener(|this, _, window, cx| this.set_service(Service::GitHub, window, cx)),
                                ))
                                .child(Button::new("account-gitlab").xsmall().label("GitLab").map(|b| if service == Service::GitLab { b.primary() } else { b.outline() }).on_click(
                                    cx.listener(|this, _, window, cx| this.set_service(Service::GitLab, window, cx)),
                                )),
                        )
                        .child(h_flex().gap_2().child(div().w(px(60.)).text_sm().text_color(palette.text_secondary).child("Server:")).child(div().flex_1().child(Input::new(&self.server).small())))
                        .child(h_flex().gap_2().child(div().w(px(60.)).text_sm().text_color(palette.text_secondary).child("Token:")).child(div().flex_1().child(Input::new(&self.token).small())))
                        .child(
                            div()
                                .id("account-generate")
                                .text_sm()
                                .text_color(palette.link)
                                .cursor_pointer()
                                .child(scopes)
                                .on_click(move |_, _, cx| cx.open_url(&token_url)),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Button::new("account-login")
                                        .small()
                                        .primary()
                                        .label(if self.checking { "Logging In…" } else { "Log In" })
                                        .disabled(self.checking)
                                        .on_click(cx.listener(|this, _, _, cx| this.log_in(cx))),
                                )
                                .when(!self.accounts.is_empty(), |el| {
                                    el.child(Button::new("account-cancel").small().outline().label("Cancel").on_click(cx.listener(|this, _, _, cx| {
                                        this.adding = false;
                                        cx.notify();
                                    })))
                                }),
                        ),
                )
            })
            .when_some(self.error.clone(), |el, e| el.child(div().text_sm().text_color(palette.status_conflict).child(SharedString::from(e))))
    }
}

pub fn accounts(on_change: Option<std::rc::Rc<dyn Fn(&mut App)>>, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| {
        let mut view = AccountsView::new(window, cx);
        view.on_change = on_change;
        view
    });
    window.open_dialog(cx, move |dialog, _, _| {
        dialog.title("GitHub / GitLab Accounts").w(px(560.)).child(view.clone()).footer(
            gpui_kit::component::dialog::DialogFooter::new()
                .child(gpui_kit::component::dialog::DialogClose::new().child(Button::new("accounts-close").label("Close").primary())),
        )
    });
}
