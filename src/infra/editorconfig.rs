//! The `.editorconfig` files that apply to a file: its folder's and its
//! parents' up to the project root, or up to one marked `root = true`.

use std::path::Path;

use crate::domain::editorconfig::{self, EditorConfig};

/// The indent unit `.editorconfig` gives `file` (absolute), if any.
pub fn indent_for(root: &Path, file: &Path) -> Option<String> {
    let mut configs: Vec<(EditorConfig, String)> = Vec::new();
    for dir in file.ancestors().skip(1) {
        if let Ok(text) = std::fs::read_to_string(dir.join(".editorconfig")) {
            let config = editorconfig::parse(&text);
            let relative = file.strip_prefix(dir).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            let stop = config.root;
            configs.push((config, relative));
            if stop {
                break;
            }
        }
        if dir == root {
            break;
        }
    }
    configs.reverse();
    editorconfig::indent(&configs)
}
