//! Small pieces shared by the Git client's views.

use std::collections::BTreeMap;

use chrono::{Local, TimeZone as _};
use gpui_kit::component::{
    Icon, Sizable as _,
    button::{Button, ButtonVariants as _},
    tree::TreeItem,
};
use gpui_kit::assets::IconName;
use gpui_kit::{ElementId, Hsla, SharedString};

use crate::git::{FileChangeKind, StatusKind};
use crate::theme::Palette;

pub const ROW_HEIGHT: f32 = 24.;

pub fn icon(name: IconName) -> Icon {
    Icon::new(name).small()
}

/// A borderless icon button, as used on IntelliJ tool window toolbars.
pub fn tool_button(id: impl Into<ElementId>, name: IconName, tooltip: impl Into<SharedString>) -> Button {
    Button::new(id).ghost().xsmall().icon(Icon::new(name)).tooltip(tooltip)
}

/// IntelliJ's default log date format: "Today 14:32", "Yesterday 09:15", or the date.
pub fn format_date(unix_seconds: i64) -> String {
    let Some(time) = Local.timestamp_opt(unix_seconds, 0).single() else { return String::new() };
    let today = Local::now().date_naive();
    let date = time.date_naive();
    if date == today {
        format!("Today {}", time.format("%H:%M"))
    } else if Some(date) == today.pred_opt() {
        format!("Yesterday {}", time.format("%H:%M"))
    } else {
        time.format("%Y/%m/%d, %H:%M").to_string()
    }
}

pub fn format_full_date(unix_seconds: i64) -> String {
    Local
        .timestamp_opt(unix_seconds, 0)
        .single()
        .map(|t| t.format("%Y/%m/%d at %H:%M").to_string())
        .unwrap_or_default()
}

pub fn change_color(kind: FileChangeKind, palette: &Palette) -> Hsla {
    match kind {
        FileChangeKind::Added | FileChangeKind::Copied => palette.status_added,
        FileChangeKind::Deleted => palette.status_deleted,
        FileChangeKind::Renamed => palette.status_renamed,
        FileChangeKind::Modified | FileChangeKind::TypeChanged => palette.status_modified,
    }
}

pub fn status_color(kind: StatusKind, palette: &Palette) -> Hsla {
    match kind {
        StatusKind::Added => palette.status_added,
        StatusKind::Modified | StatusKind::Renamed => palette.status_modified,
        StatusKind::Deleted => palette.status_deleted,
        StatusKind::Unversioned => palette.status_unversioned,
        StatusKind::Conflicted => palette.status_conflict,
    }
}

/// Icon for a file, by extension.
pub fn file_icon(path: &str) -> IconName {
    match path.rsplit('.').next().unwrap_or_default() {
        "rs" | "kt" | "kts" | "java" | "c" | "h" | "cc" | "cpp" | "hpp" | "swift" | "dart" | "go" | "m" | "mm"
        | "v" | "js" | "ts" | "tsx" | "py" => IconName::FileCode,
        _ => IconName::File,
    }
}

pub const FILE_PREFIX: &str = "f:";
pub const DIR_PREFIX: &str = "d:";

#[derive(Default)]
struct DirNode {
    dirs: BTreeMap<String, DirNode>,
    files: Vec<String>,
}

/// Builds a directory-grouped tree of `paths`, compacting chains of single-
/// child directories into one node ("src/main/kotlin") like IntelliJ does.
/// File items get the id `f:<path>`; directories `d:<path>`.
pub fn file_tree(paths: impl IntoIterator<Item = String>, scope: &str) -> Vec<TreeItem> {
    let mut root = DirNode::default();
    for path in paths {
        let mut node = &mut root;
        let mut parts: Vec<&str> = path.split('/').collect();
        parts.pop();
        for part in parts {
            node = node.dirs.entry(part.to_owned()).or_default();
        }
        node.files.push(path);
    }
    build_items(&root, "", scope)
}

fn build_items(node: &DirNode, prefix: &str, scope: &str) -> Vec<TreeItem> {
    let mut items = Vec::new();
    for (name, child) in &node.dirs {
        let mut label = name.clone();
        let mut path = join(prefix, name);
        let mut child = child;
        while child.files.is_empty() && child.dirs.len() == 1 {
            let (next_name, next) = child.dirs.iter().next().unwrap();
            label = format!("{label}/{next_name}");
            path = join(&path, next_name);
            child = next;
        }
        items.push(
            TreeItem::new(format!("{scope}{DIR_PREFIX}{path}"), label)
                .expanded(true)
                .children(build_items(child, &path, scope)),
        );
    }
    for file in &node.files {
        let name = file.rsplit('/').next().unwrap_or(file);
        items.push(TreeItem::new(format!("{scope}{FILE_PREFIX}{file}"), name.to_owned()));
    }
    items
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() { name.to_owned() } else { format!("{prefix}/{name}") }
}

/// Counts files under every directory item, for the gray "3 files" hint.
pub fn count_files(items: &[TreeItem], counts: &mut std::collections::HashMap<SharedString, usize>) -> usize {
    let mut total = 0;
    for item in items {
        if item.children.is_empty() && item.id.contains(FILE_PREFIX) {
            total += 1;
        } else {
            let n = count_files(&item.children, counts);
            counts.insert(item.id.clone(), n);
            total += n;
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compacts_single_child_directories() {
        let items = file_tree(
            ["src/main/kotlin/A.kt".to_owned(), "src/main/kotlin/B.kt".to_owned(), "README.md".to_owned()],
            "",
        );
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].label.as_ref(), "src/main/kotlin");
        assert_eq!(items[0].children.len(), 2);
        assert_eq!(items[1].id.as_ref(), "f:README.md");
    }
}
