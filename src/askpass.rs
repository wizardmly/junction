//! Credentials prompt. Git runs Junction itself as `GIT_ASKPASS` /
//! `SSH_ASKPASS` with the prompt as the only argument ("Username for
//! 'https://…':", "Password for …", "Enter passphrase for key …"); we show a
//! small login window, like IntelliJ's, and print the answer on stdout.

use gpui_kit::component::{
    Sizable as _, TitleBar, h_flex,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::{
    AppContext as _, Bounds, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, Subscription,
    Window, WindowBounds, WindowOptions, div, px, size,
};

use crate::theme::ActivePalette as _;

/// Set in git's environment so a Junction started as askpass knows it.
pub const ENV: &str = "JUNCTION_ASKPASS";

/// Environment for git commands so credential prompts come to us.
pub fn git_env() -> Vec<(&'static str, String)> {
    let Ok(exe) = std::env::current_exe() else { return Vec::new() };
    let exe = exe.to_string_lossy().into_owned();
    vec![
        (ENV, "1".into()),
        ("GIT_ASKPASS", exe.clone()),
        ("SSH_ASKPASS", exe),
        // OpenSSH 8.4+: use SSH_ASKPASS even with a terminal attached.
        ("SSH_ASKPASS_REQUIRE", "force".into()),
    ]
}

/// When started by git as askpass, returns the prompt to show.
pub fn requested_prompt() -> Option<String> {
    std::env::var_os(ENV)?;
    std::env::args().nth(1)
}

struct Prompt {
    prompt: String,
    input: Entity<InputState>,
    _subscription: Subscription,
}

fn finish(answer: Option<String>) -> ! {
    match answer {
        Some(answer) => {
            println!("{answer}");
            std::process::exit(0)
        }
        None => std::process::exit(1),
    }
}

impl Render for Prompt {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let secret = is_secret(&self.prompt);
        v_flex()
            .size_full()
            .bg(palette.panel)
            .text_color(palette.text)
            .child(TitleBar::new().child(div().text_sm().child(if secret { "Enter Password" } else { "Log In to Git" })))
            .child(
                v_flex()
                    .p_4()
                    .gap_3()
                    .child(div().text_sm().child(self.prompt.trim().to_owned()))
                    .child(if secret { Input::new(&self.input).mask_toggle() } else { Input::new(&self.input) })
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .child(Button::new("askpass-cancel").label("Cancel").outline().small().on_click(|_, _, _| finish(None)))
                            .child(Button::new("askpass-ok").label("OK").primary().small().on_click(cx.listener(
                                |this, _, _, cx| finish(Some(this.input.read(cx).value().to_string())),
                            ))),
                    ),
            )
    }
}

fn is_secret(prompt: &str) -> bool {
    let lower = prompt.to_lowercase();
    lower.contains("password") || lower.contains("passphrase") || lower.contains("token")
}

pub fn run(prompt: String) {
    gpui_kit::application().with_assets(crate::assets::AppAssets).run(move |cx| {
        gpui_kit::init(cx);
        let settings = crate::settings::Settings::load();
        crate::theme::apply(settings.dark, cx);
        cx.set_global(settings);
        let bounds = Bounds::centered(None, size(px(460.), px(165.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            is_resizable: false,
            app_id: Some("junction-askpass".into()),
            ..TitleBar::window_options()
        };
        gpui_kit::open_window(options, cx, move |window, cx| {
            let input = cx.new(|cx| {
                let mut state = InputState::new(window, cx);
                if is_secret(&prompt) {
                    state.set_masked(true, window, cx);
                }
                state
            });
            input.update(cx, |state, cx| state.focus(window, cx));
            let subscription = cx.subscribe(&input, |input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    finish(Some(input.read(cx).value().to_string()));
                }
            });
            cx.new(|_| Prompt { prompt: prompt.clone(), input, _subscription: subscription })
        })
        .expect("failed to open window");
        cx.on_window_closed(|_, _| finish(None)).detach();
        cx.activate(true);
    });
}
