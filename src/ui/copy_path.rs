//! Copy Path/Reference… (IntelliJ's "Copy" popup): the file's absolute
//! path, name, and its path from the content root, source root and
//! repository root, and its GitHub / GitLab URL, each with what it copies
//! shown beside it.

use std::path::Path;

use gpui_kit::component::{Icon, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, ClipboardItem, Context, FocusHandle, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Point,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};

use crate::model::RepoModel;
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::common;

/// One row of the popup.
#[derive(Clone)]
pub struct CopyEntry {
    pub label: SharedString,
    pub shortcut: &'static str,
    pub value: String,
    pub icon: Option<Icon>,
    /// A separator above the row.
    pub group_start: bool,
}

/// Source folders inside a Gradle source set (`src/main/java`), as Android
/// Studio marks them source or resource roots.
const SOURCE_SET_ROOTS: [&str; 13] = ["java", "kotlin", "res", "assets", "cpp", "aidl", "jni", "jniLibs", "resources", "rs", "shaders", "groovy", "proto"];

/// The content root and source root of a project-relative file, as
/// IntelliJ's modules have them: a Gradle source set (`app/src/main`) and
/// its source folder (`app/src/main/java`); a Cargo crate and its `src`;
/// a Dart package and `lib`; a Swift package's `Sources/<target>`; else the
/// module folder (the one with a build file) or the project.
pub fn roots_of(root: &Path, path: &str) -> (String, Option<String>) {
    let module = common::module_of(root, path, &mut Default::default());
    let rest = if module.is_empty() { path } else { path.strip_prefix(&format!("{module}/")).unwrap_or(path) };
    let parts: Vec<&str> = rest.split('/').collect();
    let join = |extra: &[&str]| {
        let mut all: Vec<&str> = if module.is_empty() { Vec::new() } else { vec![module.as_str()] };
        all.extend_from_slice(extra);
        all.join("/")
    };
    let dir = root.join(&module);
    let gradle = ["build.gradle", "build.gradle.kts"].iter().any(|f| dir.join(f).is_file());
    if gradle && parts.len() >= 3 && parts[0] == "src" {
        let content = join(&parts[..2]);
        let source = (parts.len() >= 4 && SOURCE_SET_ROOTS.contains(&parts[2])).then(|| join(&parts[..3]));
        return (content, source);
    }
    let source = if parts.len() >= 2 {
        match parts[0] {
            "src" | "lib" | "test" | "tests" | "bin" | "benches" | "examples" => Some(join(&parts[..1])),
            "Sources" | "Tests" if parts.len() >= 3 => Some(join(&parts[..2])),
            _ => None,
        }
    } else if dir.join("go.mod").is_file() {
        Some(module.clone())
    } else {
        None
    };
    (module.clone(), source)
}

fn relative(path: &str, root: &str) -> String {
    if root.is_empty() { path.to_owned() } else { path.strip_prefix(&format!("{root}/")).unwrap_or(path).to_owned() }
}

/// An absolute path as the platform writes it.
fn native(path: &Path) -> String {
    let text = path.to_string_lossy().into_owned();
    if cfg!(windows) { text.replace('/', "\\") } else { text }
}

/// The popup's rows for `paths` (project-relative, or absolute for a
/// library file); several paths give one line each. `lines` (1-based) adds
/// the selection to the web URL, as the editor's Copy does.
pub fn entries(model: &RepoModel, paths: &[String], lines: Option<(usize, usize)>) -> Vec<CopyEntry> {
    let root = model.project_root().map(Path::to_path_buf).or_else(|| model.project_repository().map(|r| r.root().to_path_buf()));
    let full = |p: &str| match (&root, Path::new(p).is_absolute()) {
        (Some(root), false) if !p.is_empty() => root.join(p),
        (Some(root), false) => root.clone(),
        _ => Path::new(p).to_path_buf(),
    };
    let lines_of = |f: &dyn Fn(&str) -> Option<String>| -> Option<String> {
        let all: Vec<String> = paths.iter().filter_map(|p| f(p)).filter(|v| !v.is_empty()).collect();
        (!all.is_empty()).then(|| all.join("\n"))
    };
    let mut out = vec![
        CopyEntry { label: "Absolute Path".into(), shortcut: "Ctrl+Shift+C", value: lines_of(&|p| Some(native(&full(p)))).unwrap_or_default(), icon: None, group_start: false },
        CopyEntry {
            label: "File Name".into(),
            shortcut: "",
            value: lines_of(&|p| full(p).file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default(),
            icon: None,
            group_start: false,
        },
    ];
    let in_project = |p: &str| !Path::new(p).is_absolute();
    let mut group = Vec::new();
    if let Some(root) = &root {
        if let Some(v) = lines_of(&|p| in_project(p).then(|| relative(p, &roots_of(root, p).0))) {
            group.push(("Path from Content Root", v));
        }
        if let Some(v) = lines_of(&|p| in_project(p).then(|| roots_of(root, p).1.map(|s| relative(p, &s))).flatten()) {
            group.push(("Path from Source Root", v));
        }
    }
    if let Some(v) = lines_of(&|p| in_project(p).then(|| model.route(p).map(|(_, own)| own).unwrap_or_else(|| p.to_owned()))) {
        group.push(("Path From Repository Root", v));
    }
    let first = out.len();
    out.extend(group.into_iter().map(|(label, value)| CopyEntry { label: label.into(), shortcut: "", value, icon: None, group_start: false }));
    // The web URL of the repository holding the file, at the branch it
    // tracks (else the commit), as IntelliJ's "GitHub Repository URL".
    let web: Vec<(crate::git::hosting::Host, String)> = paths
        .iter()
        .filter(|p| in_project(p))
        .filter_map(|p| {
            let (repo, own) = model.route(p)?;
            let web = crate::git::hosting::web_repo(&repo)?;
            let revision = repo
                .run(["rev-parse", "--abbrev-ref", "@{upstream}"])
                .ok()
                .and_then(|u| u.trim().split_once('/').map(|(_, b)| b.to_owned()))
                .or_else(|| repo.run(["rev-parse", "HEAD"]).ok().map(|h| h.trim().to_owned()))?;
            Some((web.host, web.file_url(&revision, &own, if paths.len() == 1 { lines } else { None })))
        })
        .collect();
    if let Some((host, _)) = web.first().filter(|(h, _)| *h != crate::git::hosting::Host::Other) {
        let icon = (*host == crate::git::hosting::Host::GitHub).then(|| Icon::from(icons::GITHUB));
        let value = web.iter().map(|(_, u)| u.clone()).collect::<Vec<_>>().join("\n");
        out.push(CopyEntry { label: format!("{} Repository URL", host.name()).into(), shortcut: "", value, icon, group_start: false });
    }
    if let Some(e) = out.get_mut(first) {
        e.group_start = true;
    }
    out
}

struct CopyPopup {
    entries: Vec<CopyEntry>,
    selected: usize,
    focus: FocusHandle,
}

impl CopyPopup {
    fn pick(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.entries.get(ix) else { return };
        cx.write_to_clipboard(ClipboardItem::new_string(entry.value.clone()));
        window.close_dialog(cx);
    }
}

impl Render for CopyPopup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mut rows = Vec::new();
        for (ix, entry) in self.entries.iter().enumerate() {
            if entry.group_start {
                rows.push(div().my_1().h(px(1.)).bg(palette.border).into_any_element());
            }
            // The first line of what it copies, after the label.
            let preview = entry.value.lines().next().unwrap_or_default().to_owned();
            rows.push(
                h_flex()
                    .id(("copy-path", ix))
                    .h(px(26.))
                    .px_2()
                    .gap_2()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .text_sm()
                    .when(ix == self.selected, |el| el.bg(palette.selection))
                    .when(ix != self.selected, |el| el.hover(|el| el.bg(palette.hover)))
                    .child(div().w(px(16.)).flex_none().when_some(entry.icon.clone(), |el, icon| el.child(common::icon(icon))))
                    .child(div().flex_none().whitespace_nowrap().child(entry.label.clone()))
                    .when(!entry.shortcut.is_empty(), |el| {
                        el.child(div().flex_none().whitespace_nowrap().child(crate::ui::file_menus::shortcut_label(entry.shortcut)))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .pl_4()
                            .flex()
                            .justify_center()
                            .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(palette.text_secondary).child(preview)),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| this.pick(ix, window, cx)))
                    .into_any_element(),
            );
        }
        v_flex()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, cx| {
                let n = this.entries.len().max(1);
                match event.keystroke.key.as_str() {
                    "down" => this.selected = (this.selected + 1) % n,
                    "up" => this.selected = (this.selected + n - 1) % n,
                    "enter" => {
                        cx.stop_propagation();
                        this.pick(this.selected, window, cx);
                        return;
                    }
                    _ => return,
                }
                cx.notify();
            }))
            .children(rows)
    }
}

/// Opens the popup at `at` (the mouse), or centered.
pub fn open(entries: Vec<CopyEntry>, at: Option<Point<Pixels>>, window: &mut Window, cx: &mut App) {
    if entries.is_empty() {
        return;
    }
    let view = cx.new(|cx| CopyPopup { entries, selected: 0, focus: cx.focus_handle() });
    let shown = view.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let confirm = shown.clone();
        let dialog = dialog
            .close_button(false)
            .title(h_flex().w_full().justify_center().child(div().text_sm().font_weight(gpui_kit::FontWeight::BOLD).child("Copy")))
            .w(px(740.))
            .child(shown.clone())
            .on_ok(move |_, window, cx| {
                confirm.update(cx, |this, cx| this.pick(this.selected, window, cx));
                false
            });
        match at {
            Some(at) => dialog.position(at).dim(false),
            None => dialog,
        }
    });
    let focus = view.read(cx).focus.clone();
    window.defer(cx, move |window, cx| window.focus(&focus, cx));
}

/// Opens the popup for `paths` in the project of `model`.
pub fn open_for(model: &gpui_kit::Entity<RepoModel>, paths: Vec<String>, lines: Option<(usize, usize)>, at: Option<Point<Pixels>>, window: &mut Window, cx: &mut App) {
    let entries = entries(model.read(cx), &paths, lines);
    open(entries, at, window, cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_content_and_source_roots() {
        let dir = std::env::temp_dir().join(format!("junction-copy-path-{}", std::process::id()));
        let _ = std::fs::create_dir_all(dir.join("app/src/main/java/top"));
        let _ = std::fs::create_dir_all(dir.join("core/src"));
        std::fs::write(dir.join("app/build.gradle.kts"), "").unwrap();
        std::fs::write(dir.join("core/Cargo.toml"), "").unwrap();
        let (content, source) = roots_of(&dir, "app/src/main/java/top/A.kt");
        assert_eq!(content, "app/src/main");
        assert_eq!(source.as_deref(), Some("app/src/main/java"));
        assert_eq!(roots_of(&dir, "app/src/main/AndroidManifest.xml"), ("app/src/main".into(), None));
        assert_eq!(roots_of(&dir, "core/src/lib.rs"), ("core".into(), Some("core/src".into())));
        assert_eq!(roots_of(&dir, "README.md"), (String::new(), None));
        let _ = std::fs::remove_dir_all(dir);
    }
}
