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
            diff_inserted: hex_alpha(0x549159, 0.25),
            diff_deleted: hex_alpha(0x9c4e4e, 0.30),
            diff_header: hex_alpha(0x3574f0, 0.18),
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
            diff_inserted: hex_alpha(0x67c27a, 0.25),
            diff_deleted: hex_alpha(0xf27c7c, 0.25),
            diff_header: hex_alpha(0x3574f0, 0.12),
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
pub fn apply(dark: bool, cx: &mut App) {
    let palette = if dark { Palette::dark() } else { Palette::light() };
    Theme::change(if dark { ThemeMode::Dark } else { ThemeMode::Light }, None, cx);
    let p = palette.clone();
    Theme::update(cx, move |theme| {
        theme.font_size = px(13.);
        theme.radius = px(4.);
        theme.shadow = true;
        theme.background = p.window;
        theme.foreground = p.text;
        theme.border = if p.dark { hex(0x393b40) } else { hex(0xdfe1e5) };
        theme.muted_foreground = p.text_secondary;
        theme.list_active = p.selection;
        theme.list_active_border = p.selection;
        theme.list_hover = p.hover;
        theme.primary = p.accent;
        theme.button_primary = p.accent;
        theme.ring = p.accent;
        theme.selection = if p.dark { hex(0x214283) } else { hex(0xa6d2ff) };
        theme.title_bar = p.toolbar;
        theme.title_bar_border = p.border;
        theme.popover = if p.dark { hex(0x2b2d30) } else { hex(0xffffff) };
        theme.input = if p.dark { hex(0x4e5157) } else { hex(0xc9ccd6) };
    });
    cx.set_global(palette);
}
