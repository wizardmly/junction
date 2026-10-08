//! Settings › Version Control › GitHub: accounts logged in with a token.
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
        Self { accounts, server, token, adding, checking: false, error: None, on_change: None }
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
        let mut candidate = Account { service: Service::GitHub, server, login: String::new(), token };
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
                let host = if server.is_empty() { "github.com".to_owned() } else { server };
                let token_url = format!("https://{host}/settings/tokens/new?scopes=repo,gist,read:org,workflow&description=Junction");
                el.child(
                    v_flex()
                        .gap_2()
                        .p_2()
                        .rounded(px(4.))
                        .border_1()
                        .border_color(palette.border)
                        .child(div().text_sm().font_weight(gpui_kit::FontWeight::SEMIBOLD).child("Log In with Token"))
                        .child(h_flex().gap_2().child(div().w(px(60.)).text_sm().text_color(palette.text_secondary).child("Server:")).child(div().flex_1().child(Input::new(&self.server).small())))
                        .child(h_flex().gap_2().child(div().w(px(60.)).text_sm().text_color(palette.text_secondary).child("Token:")).child(div().flex_1().child(Input::new(&self.token).small())))
                        .child(
                            div()
                                .id("account-generate")
                                .text_sm()
                                .text_color(palette.link)
                                .cursor_pointer()
                                .child("Generate token (scopes: repo, gist, read:org, workflow)")
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
            .when_some(self.error.clone(), |el, e| el.child(div().text_sm().text_color(palette.status_deleted).child(SharedString::from(e))))
    }
}

pub fn accounts(on_change: Option<std::rc::Rc<dyn Fn(&mut App)>>, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| {
        let mut view = AccountsView::new(window, cx);
        view.on_change = on_change;
        view
    });
    window.open_dialog(cx, move |dialog, _, _| {
        dialog.title("GitHub Accounts").w(px(560.)).child(view.clone()).footer(
            gpui_kit::component::dialog::DialogFooter::new()
                .child(gpui_kit::component::dialog::DialogClose::new().child(Button::new("accounts-close").label("Close").primary())),
        )
    });
}
