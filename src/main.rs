//! GitGlass: a desktop Git client recreating the Android Studio / IntelliJ
//! Git experience with a translucent "glass" window.

// A GUI app on Windows: no console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod askpass;
mod assets;
mod git;
mod model;
mod settings;
mod theme;
mod ui;

use std::path::PathBuf;

use gpui_kit::component::TitleBar;
use gpui_kit::{AppContext as _, Bounds, WindowBackgroundAppearance, WindowBounds, WindowOptions, px, size};

use crate::model::RepoModel;
use crate::ui::workspace::Workspace;

fn main() {
    if let Some(prompt) = askpass::requested_prompt() {
        askpass::run(prompt);
        return;
    }
    let path = std::env::args().nth(1).map(PathBuf::from).or_else(|| std::env::current_dir().ok());

    gpui_kit::application().with_assets(assets::AppAssets).run(move |cx| {
        gpui_kit::init(cx);
        ui::log_view::init(cx);
        ui::workspace::init(cx);
        let settings = settings::Settings::load();
        let dark = std::env::var("GITGLASS_THEME").map(|t| t != "light").unwrap_or(settings.dark);
        cx.set_global(settings);
        theme::apply(dark, cx);

        let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(800.), px(500.))),
            // Glass: Mica on Windows 11, vibrancy-style blur elsewhere.
            window_background: if cfg!(target_os = "windows") {
                WindowBackgroundAppearance::MicaBackdrop
            } else {
                WindowBackgroundAppearance::Blurred
            },
            app_id: Some("gitglass".into()),
            ..TitleBar::window_options()
        };

        gpui_kit::open_window(options, cx, move |window, cx| {
            let model = cx.new(|cx| RepoModel::new(path.clone(), cx));
            cx.new(|cx| Workspace::new(model, window, cx))
        })
        .expect("failed to open window");
        cx.activate(true);
    });
}
