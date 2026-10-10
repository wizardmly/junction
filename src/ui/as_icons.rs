//! Android Studio's own icons (IntelliJ's new UI set, Apache 2.0), drawn in
//! their colors with a light and a dark variant. `tools/as_icons.py` splits
//! each one into one-color layers (`as_icons_gen.rs`), since GPUI paints an
//! SVG as a mask.

use gpui_kit::component::Icon;

pub use super::as_icons_gen::*;

/// One icon: its layers, (asset path, RGBA), per theme.
#[derive(Clone, Copy)]
pub struct AsIcon {
    pub light: &'static [(&'static str, u32)],
    pub dark: &'static [(&'static str, u32)],
}

impl From<AsIcon> for Icon {
    fn from(icon: AsIcon) -> Self {
        Icon::layers(icon.light, icon.dark)
    }
}

impl From<&AsIcon> for Icon {
    fn from(icon: &AsIcon) -> Self {
        Icon::layers(icon.light, icon.dark)
    }
}

/// The asset source's lookup for "as/…" paths.
pub fn load(path: &str) -> Option<&'static [u8]> {
    FILES.iter().find(|(p, _)| *p == path).map(|(_, bytes)| *bytes)
}
