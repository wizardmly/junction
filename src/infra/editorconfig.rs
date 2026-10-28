//! The `.editorconfig` files that apply to a file: its folder's and its
//! parents' up to the project root, or up to one marked `root = true`.

use std::path::Path;

use crate::domain::editorconfig::{self, EditorConfig};

/// What `.editorconfig` gives `file` (absolute): its indent unit and its
/// `max_line_length`.
pub fn style_for(root: &Path, file: &Path) -> (Option<String>, Option<usize>) {
    let configs = configs_for(root, file);
    (editorconfig::indent(&configs), editorconfig::max_line_length(&configs))
}

/// The configs that apply, from the farthest to the nearest, each with
/// the file's path relative to its folder.
fn configs_for(root: &Path, file: &Path) -> Vec<(EditorConfig, String)> {
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
    configs
}
