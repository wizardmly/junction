//! Junction: a desktop Git client recreating the Android Studio / IntelliJ
//! Git experience with a translucent "glass" window.

// A GUI app on Windows: no console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod askpass;
mod assets;
mod crash;
mod git;
mod hosting;
mod index;
mod model;
mod settings;
mod spell;
mod theme;
mod ui;

use std::path::PathBuf;

use gpui_kit::component::TitleBar;
use gpui_kit::{AppContext as _, Bounds, WindowBackgroundAppearance, WindowBounds, WindowOptions, px, size};

use crate::model::RepoModel;
use crate::ui::workspace::Workspace;

fn main() {
    crash::install();
    // Started by git as the interactive-rebase sequence editor.
    if git::rebase::handle_sequence_editor() {
        return;
    }
    if let Some(prompt) = askpass::requested_prompt() {
        askpass::run(prompt);
        return;
    }
    let path = std::env::args().nth(1).map(PathBuf::from).or_else(|| std::env::current_dir().ok());

    gpui_kit::application().with_assets(assets::AppAssets).run(move |cx| {
        gpui_kit::init(cx);
        ui::log_view::init(cx);
        ui::workspace::init(cx);
        ui::commit_view::init(cx);
        ui::spell_overlay::init(cx);
        ui::file_editor::init(cx);
        ui::find_bar::init(cx);
        let settings = settings::Settings::load();
        settings.apply_git();
        ui::common::set_compact(settings.compact);
        cx.set_global(settings);
        let dark = std::env::var("JUNCTION_THEME").map(|t| t != "light").unwrap_or_else(|_| theme::effective_dark(cx));
        cx.set_global(model::ExcludedHunks::default());
        theme::apply(dark, cx);

        open_project_window(path.clone(), cx);
        cx.activate(true);
    });
}

/// Opens a project window, as File › Open does in IntelliJ (also used for
/// worktrees and submodules opened in a new window).
pub fn open_project_window(path: Option<PathBuf>, cx: &mut gpui_kit::App) {
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
        app_id: Some("junction".into()),
        ..TitleBar::window_options()
    };
    gpui_kit::open_window(options, cx, move |window, cx| {
        let model = cx.new(|cx| RepoModel::new(path.clone(), cx));
        cx.new(|cx| Workspace::new(model, window, cx))
    })
    .expect("failed to open window");
}
