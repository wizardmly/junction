//! Small pieces shared by the Git client's views.

use std::collections::BTreeMap;
use crate::ui::as_icons as icons;

use chrono::{Local, TimeZone as _};
use gpui_kit::component::{
    Icon, Sizable as _,
    button::{Button, ButtonVariants as _},
    tree::TreeItem,
};
use gpui_kit::assets::IconName;
use gpui_kit::Styled as _;
use gpui_kit::{ElementId, Hsla, SharedString};

use crate::git::{FileChangeKind, StatusKind};
use crate::theme::Palette;

static COMPACT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Appearance › Compact mode, set from the settings.
pub fn set_compact(compact: bool) {
    COMPACT.store(compact, std::sync::atomic::Ordering::Relaxed);
}

pub fn compact() -> bool {
    COMPACT.load(std::sync::atomic::Ordering::Relaxed)
}

/// Tree and list rows: 24px, 20px in compact mode.
pub fn row_height() -> f32 {
    if compact() { 20. } else { 24. }
}

/// Tool window toolbars.
pub fn toolbar_height() -> f32 {
    if compact() { 28. } else { 32. }
}

/// Tab rows and viewer headers.
pub fn header_height() -> f32 {
    if compact() { 26. } else { 30. }
}

pub fn icon(name: impl Into<Icon>) -> Icon {
    Icon::new(name).small()
}

/// A borderless icon button, as used on IntelliJ tool window toolbars.
pub fn tool_button(id: impl Into<ElementId>, name: impl Into<Icon>, tooltip: impl Into<SharedString>) -> Button {
    Button::new(id).ghost().xsmall().icon(Icon::new(name)).tooltip(tooltip)
}

/// IntelliJ's default log date format: "Today 14:32", "Yesterday 09:15", or the date.
/// IntelliJ's three-state checkbox: a dash when only part of what it
/// stands for is included; clicking it then includes everything.
pub fn tri_checkbox(
    id: impl Into<ElementId>,
    checked: bool,
    partial: bool,
    tooltip: &'static str,
    cx: &gpui_kit::App,
    on_change: impl Fn(bool, &mut gpui_kit::Window, &mut gpui_kit::App) + 'static,
) -> gpui_kit::Div {
    use gpui_kit::{ParentElement as _, Styled as _, prelude::FluentBuilder as _, px};
    let theme = gpui_kit::component::ActiveTheme::theme(cx);
    let (mark_bg, mark_fg) = (theme.primary, theme.primary_foreground);
    gpui_kit::div()
        .relative()
        .child(
            gpui_kit::component::checkbox::Checkbox::new(id)
                .checked(checked || partial)
                .tooltip(tooltip)
                .on_change(move |value, window, cx| on_change(partial || *value, window, cx)),
        )
        .when(partial, |el| {
            el.child(
                gpui_kit::div()
                    .absolute()
                    .inset_0()
                    .rounded(px(3.))
                    .bg(mark_bg)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(gpui_kit::div().w(px(8.)).h(px(2.)).bg(mark_fg)),
            )
        })
}

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

/// "Just now", "5 minutes ago", "3 hours ago", "2 days ago", then the date
/// (View Options › relative dates).
pub fn format_relative_date(unix_seconds: i64) -> String {
    let elapsed = Local::now().timestamp() - unix_seconds;
    let plural = |n: i64, unit: &str| format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
    match elapsed {
        ..60 => "Just now".into(),
        60..3600 => plural(elapsed / 60, "minute"),
        3600..86_400 => plural(elapsed / 3600, "hour"),
        86_400..604_800 => plural(elapsed / 86_400, "day"),
        _ => format_date(unix_seconds),
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

/// A file's icon by its type, as Android Studio draws it.
pub fn file_icon(path: &str) -> Icon {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path).to_lowercase();
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    let icon = match ext {
        _ if name.ends_with(".gradle.kts") => icons::GRADLE_KOTLIN,
        "gradle" => icons::GRADLE,
        "kt" => icons::KOTLIN,
        "kts" => icons::KOTLIN_SCRIPT,
        "java" => icons::CLASS,
        // Markdown's "M↓" comes from a plugin whose icon isn't published.
        "md" | "markdown" => return Icon::empty().path("junction/markdown.svg").text_color(gpui_kit::rgb(0x3574f0)),
        "properties" => icons::FILE_PROPERTIES,
        "conf" | "cfg" | "ini" | "toml" => icons::FILE_CONFIG,
        "editorconfig" => icons::FILE_EDITOR_CONFIG,
        _ if name.starts_with(".gitignore") || name == ".gitattributes" || name.ends_with("ignore") => icons::FILE_IGNORED,
        "sh" | "bash" | "zsh" | "command" => icons::FILE_SHELL,
        _ if name == "gradlew" => icons::FILE_SHELL,
        "xml" | "svg" => icons::FILE_XML,
        "html" | "htm" => icons::FILE_HTML,
        "css" | "scss" => icons::FILE_CSS,
        "js" | "mjs" | "ts" | "tsx" | "jsx" => icons::FILE_JAVASCRIPT,
        "json" => icons::FILE_JSON,
        "yaml" | "yml" => icons::FILE_YAML,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" => icons::FILE_IMAGE,
        "zip" | "jar" | "aar" | "tar" | "gz" | "7z" => icons::FILE_ARCHIVE,
        "patch" | "diff" => icons::FILE_PATCH,
        "mf" => icons::FILE_MANIFEST,
        "so" | "dll" | "exe" | "bin" | "class" | "dex" => icons::FILE_BINARY,
        // Languages Android Studio draws with plugin icons we don't have.
        "rs" | "c" | "h" | "cc" | "cpp" | "hpp" | "swift" | "dart" | "go" | "m" | "mm" | "v" | "py" => {
            return Icon::new(IconName::FileCode).text_color(gpui_kit::rgb(0x6c707e));
        }
        _ => icons::FILE_TEXT,
    };
    icon.into()
}


/// The Log's Root column: a color per repository of a multi-root
/// project, as IntelliJ tells the roots apart.
pub fn root_color(ix: usize) -> gpui_kit::Hsla {
    const COLORS: [u32; 8] = [0x4c8cdd, 0xe0954a, 0x6fb35f, 0xc06bd1, 0xd9c24a, 0x4bb5b0, 0xd7615d, 0x8a8fd6];
    gpui_kit::rgb(COLORS[ix % COLORS.len()]).into()
}

pub const FILE_PREFIX: &str = "f:";
pub const DIR_PREFIX: &str = "d:";
/// Group By › Module nodes: `m:<module folder>` ("" is the root module).
pub const MODULE_PREFIX: &str = "m:";
/// Group By › Repository nodes.
pub const REPO_PREFIX: &str = "r:";

/// Build files that make a folder a module, as IntelliJ's modules: Gradle,
/// Cargo, Go, Dart, Swift, CMake, npm, Maven, V.
pub const BUILD_FILES: [&str; 12] = [
    "build.gradle", "build.gradle.kts", "Cargo.toml", "go.mod", "pubspec.yaml", "Package.swift",
    "CMakeLists.txt", "package.json", "pom.xml", "v.mod", "settings.gradle", "settings.gradle.kts",
];

/// The module a file belongs to: its nearest folder (inside `root`) holding
/// a build file, "" for the root module. Folders are looked up once per call site.
pub fn module_of(root: &std::path::Path, path: &str, cache: &mut std::collections::HashMap<String, bool>) -> String {
    let mut dir = path.rsplit_once('/').map_or("", |(d, _)| d);
    while !dir.is_empty() {
        let is_module = *cache
            .entry(dir.to_owned())
            .or_insert_with(|| BUILD_FILES.iter().any(|f| root.join(dir).join(f).is_file()));
        if is_module {
            return dir.to_owned();
        }
        dir = dir.rsplit_once('/').map_or("", |(d, _)| d);
    }
    String::new()
}

/// The Commit / Changes tree under one group: optionally by repository, then
/// module, then directory (Group By), as IntelliJ nests them.
pub fn grouped_file_tree(
    paths: Vec<String>,
    scope: &str,
    expanded: bool,
    by_directory: bool,
    modules: Option<(&str, &dyn Fn(&str) -> String)>,
    repositories: Option<&[(String, String)]>,
) -> Vec<TreeItem> {
    let leaves = |paths: Vec<String>, prefix: &str| -> Vec<TreeItem> {
        if by_directory {
            // Directories below a module start at the module folder.
            let mut root = DirNode::default();
            for path in paths {
                let rel = path.strip_prefix(prefix).map(|r| r.trim_start_matches('/')).unwrap_or(&path).to_owned();
                let mut node = &mut root;
                let mut parts: Vec<&str> = rel.split('/').collect();
                parts.pop();
                for part in parts {
                    node = node.dirs.entry(part.to_owned()).or_default();
                }
                node.files.push(path);
            }
            build_items(&root, prefix, scope, expanded)
        } else {
            flat_file_list(paths, scope)
        }
    };
    let by_module = |paths: Vec<String>| -> Vec<TreeItem> {
        let Some((root_name, module_of)) = modules else { return leaves(paths, "") };
        let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for path in paths {
            groups.entry(module_of(&path)).or_default().push(path);
        }
        groups
            .into_iter()
            .map(|(module, files)| {
                let label = if module.is_empty() { root_name.to_owned() } else { module.rsplit('/').next().unwrap_or(&module).to_owned() };
                TreeItem::new(format!("{scope}{MODULE_PREFIX}{module}"), label).expanded(expanded).children(leaves(files, &module))
            })
            .collect()
    };
    match repositories {
        // One node per repository (`r:<its folder in the project>`), each
        // file under the deepest root holding it.
        Some(roots) if roots.len() > 1 => {
            let mut files: Vec<Vec<String>> = vec![Vec::new(); roots.len()];
            for path in paths {
                let ix = roots
                    .iter()
                    .enumerate()
                    .filter(|(_, (prefix, _))| prefix.is_empty() || path.strip_prefix(prefix.as_str()).is_some_and(|r| r.starts_with('/')))
                    .max_by_key(|(_, (prefix, _))| prefix.len())
                    .map_or(0, |(ix, _)| ix);
                files[ix].push(path);
            }
            roots
                .iter()
                .zip(files)
                .filter(|(_, files)| !files.is_empty())
                .map(|((prefix, name), files)| {
                    let children = if modules.is_some() { by_module(files) } else { leaves(files, prefix) };
                    TreeItem::new(format!("{scope}{REPO_PREFIX}{prefix}"), name.clone()).expanded(expanded).children(children)
                })
                .collect()
        }
        Some(roots) => {
            let name = roots.first().map(|(_, name)| name.as_str()).unwrap_or_default();
            vec![TreeItem::new(format!("{scope}{REPO_PREFIX}"), name.to_owned()).expanded(expanded).children(by_module(paths))]
        }
        None => by_module(paths),
    }
}

#[derive(Default)]
struct DirNode {
    dirs: BTreeMap<String, DirNode>,
    files: Vec<String>,
}

/// Builds a directory-grouped tree of `paths`, compacting chains of single-
/// child directories into one node ("src/main/kotlin") like IntelliJ does.
/// File items get the id `f:<path>`; directories `d:<path>`.
pub fn file_tree(paths: impl IntoIterator<Item = String>, scope: &str) -> Vec<TreeItem> {
    file_tree_with(paths, scope, true)
}

/// Files without directory nodes (Group By › Directory off).
pub fn flat_file_list(paths: impl IntoIterator<Item = String>, scope: &str) -> Vec<TreeItem> {
    let mut paths: Vec<String> = paths.into_iter().collect();
    paths.sort_by(|a, b| a.rsplit('/').next().cmp(&b.rsplit('/').next()).then(a.cmp(b)));
    paths
        .into_iter()
        .map(|path| {
            let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
            TreeItem::new(format!("{scope}{FILE_PREFIX}{path}"), name)
        })
        .collect()
}

/// `file_tree` with directories collapsed (whole-repository trees).
pub fn file_tree_with(paths: impl IntoIterator<Item = String>, scope: &str, expanded: bool) -> Vec<TreeItem> {
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
    build_items(&root, "", scope, expanded)
}

fn build_items(node: &DirNode, prefix: &str, scope: &str, expanded: bool) -> Vec<TreeItem> {
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
                .expanded(expanded)
                .children(build_items(child, &path, scope, expanded)),
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

/// A clickable part of a commit message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    Url(String),
    /// A hash-like word; IntelliJ links it to the commit when one matches.
    Commit(String),
}

/// URLs and hash-like words in a commit message, with their byte ranges.
pub fn find_links(text: &str) -> Vec<(std::ops::Range<usize>, Link)> {
    let mut links = Vec::new();
    let bytes = text.as_bytes();
    let mut ix = 0;
    while ix < text.len() {
        let rest = &text[ix..];
        if rest.starts_with("https://") || rest.starts_with("http://") {
            let len = rest.find(|c: char| c.is_whitespace() || "<>\"'`".contains(c)).unwrap_or(rest.len());
            // Trailing punctuation belongs to the sentence, not the URL.
            let url = rest[..len].trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']']);
            links.push((ix..ix + url.len(), Link::Url(url.to_owned())));
            ix += len.max(1);
            continue;
        }
        let at_word_start = ix == 0 || !bytes[ix - 1].is_ascii_alphanumeric();
        if at_word_start && bytes[ix].is_ascii_hexdigit() {
            let len = rest.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(rest.len());
            let word = &rest[..len];
            if (7..=40).contains(&len) && word.bytes().all(|b| b.is_ascii_hexdigit()) && word.bytes().any(|b| b.is_ascii_digit()) {
                links.push((ix..ix + len, Link::Commit(word.to_ascii_lowercase())));
            }
            ix += len.max(1);
            continue;
        }
        ix += rest.chars().next().map_or(1, char::len_utf8);
    }
    links
}

#[cfg(test)]
mod link_tests {
    use super::*;

    #[test]
    fn finds_urls_and_hashes() {
        let text = "Revert 1a2b3c4d (see https://example.com/issues/12). Café deadbeef";
        let links = find_links(text);
        assert_eq!(links.len(), 2);
        assert_eq!(&text[links[0].0.clone()], "1a2b3c4d");
        assert_eq!(links[1].1, Link::Url("https://example.com/issues/12".into()));
        assert_eq!(&text[links[1].0.clone()], "https://example.com/issues/12");
    }
}

/// A panel view drawn from its last frame unless it changed: hovering a
/// menu or typing in one panel then doesn't lay out and paint the others.
/// (Its render must only read entities and globals that notify.)
pub fn cached<V: gpui_kit::Render>(view: &gpui_kit::Entity<V>) -> gpui_kit::AnyElement {
    use gpui_kit::{IntoElement as _, Styled as _};
    view.clone().cached(gpui_kit::StyleRefinement::default().size_full()).into_any_element()
}

/// Esc with no modifiers.
pub fn is_plain_escape(keystroke: &gpui_kit::Keystroke) -> bool {
    let m = &keystroke.modifiers;
    keystroke.key == "escape" && !(m.control || m.alt || m.shift || m.platform)
}

/// A press on a popup's title bar starts moving it (IntelliJ's popups and
/// dialogs all move with the mouse); `drag.tracker()` follows the mouse.
pub fn drag_start(drag: &gpui_kit::component::dialog::DialogDrag) -> impl Fn(&gpui_kit::MouseDownEvent, &mut gpui_kit::Window, &mut gpui_kit::App) + 'static {
    let drag = drag.clone();
    move |e, window, _| {
        drag.start(e.position);
        window.refresh();
    }
}
