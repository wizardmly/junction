//! Junction: a desktop Git client recreating the Android Studio / IntelliJ
//! Git experience with a translucent "glass" window.

// A GUI app on Windows: no console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(test)]
mod bench;
mod bookmarks;
mod domain;
mod infra;
mod usecase;
mod askpass;
mod assets;
mod android;
mod crash;
mod git;
mod hosting;
mod index;
mod model;
mod settings;
mod spell;
mod terminal;
mod theme;
mod ui;
#[cfg(test)]
mod arch_tests;

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
    let arg = std::env::args().nth(1).map(PathBuf::from);

    gpui_kit::application().with_assets(assets::AppAssets).run(move |cx| {
        gpui_kit::init(cx);
        ui::log_view::init(cx);
        ui::workspace::init(cx);
        ui::commit_view::init(cx);
        ui::branches_popup::init(cx);
        ui::spell_overlay::init(cx);
        ui::file_editor::init(cx);
        ui::find_bar::init(cx);
        // After every module bound its keys: Settings › Keymap's changes go over them.
        ui::keymap::init(cx);
        let settings = settings::Settings::load();
        settings.apply_git();
        ui::common::set_compact(settings.compact);
        cx.set_global(settings);
        let dark = std::env::var("JUNCTION_THEME").map(|t| t != "light").unwrap_or_else(|_| theme::effective_dark(cx));
        cx.set_global(model::ExcludedHunks::default());
        bookmarks::Bookmarks::init(cx);
        theme::apply(dark, cx);

        let path = arg.clone().or_else(|| startup_project(settings::Settings::get(cx).reopen_projects));
        open_project_window(path, cx);
        cx.activate(true);
    });
}

/// Without a folder to open: the folder it was started in when that's a
/// repository (run from a terminal), else the last project, as IntelliJ
/// reopens it on startup (unless turned off); else the Welcome screen.
fn startup_project(reopen: bool) -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok();
    if cwd.as_ref().is_some_and(|d| d.ancestors().any(|a| a.join(".git").exists())) {
        return cwd;
    }
    settings::recent_projects().into_iter().find(|p| reopen && p.is_dir())
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
