//! IntelliJ "Int UI" (new UI) colors, with translucent panels for the glass look.

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Global, Hsla, Rgba, px, rgb};

fn hex(value: u32) -> Hsla {
    rgb(value).into()
}

fn hex_alpha(value: u32, alpha: f32) -> Hsla {
    let mut color: Hsla = Rgba::from(rgb(value)).into();
    color.a = alpha;
    color
}

/// Every color the client paints. Names follow IntelliJ's UI keys where one exists.
#[derive(Clone, Debug)]
pub struct Palette {
    pub dark: bool,
    /// Painted behind everything; translucent so the OS backdrop shows through.
    pub window: Hsla,
    /// Tool window and editor surfaces, slightly more opaque than `window`.
    pub panel: Hsla,
    pub toolbar: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub text_secondary: Hsla,
    pub text_disabled: Hsla,
    pub hover: Hsla,
    pub selection: Hsla,
    pub selection_inactive: Hsla,
    pub accent: Hsla,
    pub link: Hsla,
    // VCS file status colors (FileStatus.*).
    pub status_modified: Hsla,
    pub status_added: Hsla,
    pub status_deleted: Hsla,
    pub status_unversioned: Hsla,
    pub status_conflict: Hsla,
    pub status_renamed: Hsla,
    // Log reference labels.
    pub ref_head: Hsla,
    pub ref_local: Hsla,
    pub ref_remote: Hsla,
    pub ref_tag: Hsla,
    /// Log graph lane colors, in IntelliJ's order.
    pub graph: [Hsla; 8],
    pub diff_inserted: Hsla,
    pub diff_deleted: Hsla,
    pub diff_header: Hsla,
    pub diff_modified: Hsla,
    /// Changed words inside inserted / deleted / modified lines.
    pub diff_inserted_word: Hsla,
    pub diff_deleted_word: Hsla,
    pub diff_modified_word: Hsla,
    /// Edges of change blocks in the diff divider and empty-side markers.
    pub diff_inserted_border: Hsla,
    pub diff_deleted_border: Hsla,
    pub diff_modified_border: Hsla,
    /// Merge conflicts.
    pub diff_conflict: Hsla,
    pub diff_conflict_word: Hsla,
    pub diff_conflict_border: Hsla,
    /// Editor Find highlights (IntelliJ's "Search result").
    pub search_match: Hsla,
}

impl Global for Palette {}

impl Palette {
    pub fn dark() -> Self {
        Self {
            dark: true,
            window: hex_alpha(0x1e1f22, 0.80),
            panel: hex_alpha(0x2b2d30, 0.78),
            toolbar: hex_alpha(0x2b2d30, 0.86),
            border: hex(0x1e1f22),
            text: hex(0xdfe1e5),
            text_secondary: hex(0x868a91),
            text_disabled: hex(0x5a5d63),
            hover: hex_alpha(0xdfe1e5, 0.07),
            selection: hex(0x2e436e),
            selection_inactive: hex(0x393b40),
            accent: hex(0x3574f0),
            link: hex(0x548af7),
            status_modified: hex(0x6c9fe8),
            status_added: hex(0x73bd79),
            status_deleted: hex(0x868a91),
            status_unversioned: hex(0xd5756c),
            status_conflict: hex(0xde6a66),
            status_renamed: hex(0x6c9fe8),
            ref_head: hex(0xe9b34a),
            ref_local: hex(0x73bd79),
            ref_remote: hex(0xb189f5),
            ref_tag: hex(0xa8adbd),
            graph: [
                hex(0x548af7),
                hex(0x73bd79),
                hex(0xe08855),
                hex(0xc77dbb),
                hex(0x2fc2cf),
                hex(0xf2c55c),
                hex(0xa571e6),
                hex(0xe55765),
            ],
            // IntelliJ's dark diff colors: modified blue, inserted green, deleted gray.
            diff_inserted: hex_alpha(0x2c4a33, 0.92),
            diff_deleted: hex_alpha(0x434547, 0.92),
            diff_header: hex_alpha(0x3574f0, 0.18),
            diff_modified: hex_alpha(0x2a3b56, 0.92),
            diff_inserted_word: hex(0x3d6b45),
            diff_deleted_word: hex(0x5d6063),
            diff_modified_word: hex(0x3d5a85),
            diff_inserted_border: hex(0x4f8a59),
            diff_deleted_border: hex(0x6e7174),
            diff_modified_border: hex(0x4b6ea3),
            diff_conflict: hex_alpha(0x5a3434, 0.92),
            diff_conflict_word: hex(0x7d4545),
            diff_conflict_border: hex(0x9a5555),
            search_match: hex(0x32593d),
        }
    }

    pub fn light() -> Self {
        Self {
            dark: false,
            window: hex_alpha(0xf7f8fa, 0.80),
            panel: hex_alpha(0xffffff, 0.75),
            toolbar: hex_alpha(0xf7f8fa, 0.86),
            border: hex(0xebecf0),
            text: hex(0x000000),
            text_secondary: hex(0x818594),
            text_disabled: hex(0xa8adbd),
            hover: hex_alpha(0x000000, 0.05),
            selection: hex(0xd4e2ff),
            selection_inactive: hex(0xdfe1e5),
            accent: hex(0x3574f0),
            link: hex(0x2e55a3),
            status_modified: hex(0x0a7aff),
            status_added: hex(0x208a3c),
            status_deleted: hex(0x6c707e),
            status_unversioned: hex(0xc94f4f),
            status_conflict: hex(0xd52020),
            status_renamed: hex(0x0a7aff),
            ref_head: hex(0xc27d04),
            ref_local: hex(0x208a3c),
            ref_remote: hex(0x8350d6),
            ref_tag: hex(0x6c707e),
            graph: [
                hex(0x3574f0),
                hex(0x208a3c),
                hex(0xe56d17),
                hex(0xc252b0),
                hex(0x0fa3a3),
                hex(0xc29a00),
                hex(0x8350d6),
                hex(0xd52020),
            ],
            // IntelliJ's light diff colors: modified blue, inserted green, deleted gray.
            diff_inserted: hex(0xd2f0d3),
            diff_deleted: hex(0xe7e7e7),
            diff_header: hex_alpha(0x3574f0, 0.12),
            diff_modified: hex(0xe3ecfb),
            diff_inserted_word: hex(0xade3ad),
            diff_deleted_word: hex(0xcbcbcb),
            diff_modified_word: hex(0xc0d4f7),
            diff_inserted_border: hex(0x8fd18f),
            diff_deleted_border: hex(0xb8b8b8),
            diff_modified_border: hex(0x9fbbe8),
            diff_conflict: hex(0xffdcdc),
            diff_conflict_word: hex(0xffb3b3),
            diff_conflict_border: hex(0xeb9a9a),
            search_match: hex(0xf2c94c),
        }
    }

    pub fn graph_color(&self, index: usize) -> Hsla {
        self.graph[index % self.graph.len()]
    }
}

pub trait ActivePalette {
    fn palette(&self) -> &Palette;
}

impl ActivePalette for App {
    fn palette(&self) -> &Palette {
        self.global::<Palette>()
    }
}

/// Installs the palette and aligns the component library's theme with it, so
/// inputs, menus and popovers match the IntelliJ surfaces around them.
/// Light or dark from the settings: their own choice, or the system's when
/// the theme is "Sync with OS".
pub fn effective_dark(cx: &App) -> bool {
    let settings = crate::settings::Settings::get(cx);
    if settings.theme_follows_system {
        matches!(cx.window_appearance(), gpui_kit::WindowAppearance::Dark | gpui_kit::WindowAppearance::VibrantDark)
    } else {
        settings.dark
    }
}

/// Re-applies the theme after a settings or system appearance change.
pub fn refresh(cx: &mut App) {
    let dark = effective_dark(cx);
    if cx.try_global::<Palette>().is_none_or(|p| p.dark != dark) {
        apply(dark, cx);
    }
}

pub fn apply(dark: bool, cx: &mut App) {
    let palette = if dark { Palette::dark() } else { Palette::light() };
    Theme::change(if dark { ThemeMode::Dark } else { ThemeMode::Light }, None, cx);
    let p = palette.clone();
    Theme::update(cx, move |theme| {
        theme.font_size = px(13.);
        theme.radius = px(4.);
        theme.shadow = true;
        // Component surfaces (dialogs, menus) stay opaque so text behind them
        // doesn't show through; the glass comes from our own panels.
        theme.background = if p.dark { hex(0x2b2d30) } else { hex(0xf7f8fa) };
        theme.overlay = if p.dark { hex_alpha(0x000000, 0.35) } else { hex_alpha(0x000000, 0.15) };
        theme.foreground = p.text;
        theme.border = if p.dark { hex(0x393b40) } else { hex(0xdfe1e5) };
        theme.muted_foreground = p.text_secondary;
        theme.list_active = p.selection;
        theme.list_active_border = p.selection;
        theme.list_hover = p.hover;
        theme.primary = p.accent;
        theme.button_primary = p.accent;
        theme.ring = p.accent;
        // Translucent: TextView paints the selection over its glyphs.
        theme.selection = if p.dark { hex_alpha(0x2f65ca, 0.55) } else { hex_alpha(0x2e8bff, 0.4) };
        theme.title_bar = p.toolbar;
        theme.title_bar_border = p.border;
        theme.popover = if p.dark { hex(0x2b2d30) } else { hex(0xffffff) };
        theme.input = if p.dark { hex(0x4e5157) } else { hex(0xc9ccd6) };
    });
    cx.set_global(palette);
}
