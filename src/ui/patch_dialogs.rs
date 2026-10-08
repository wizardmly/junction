//! Create Patch, Apply Patch (from a file or the clipboard), and Shelve Changes.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::{
    Sizable as _, WindowExt as _, h_flex,
    button::Button,
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    radio::RadioGroup,
    scroll::ScrollableElement as _,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, PathPromptOptions, Render,
    Styled as _, Subscription, Task, Window, div, prelude::FluentBuilder as _, px,
};

use crate::git::patch::{self, ApplyOutcome, PatchFile};
use crate::model::RepoModel;
use crate::theme::ActivePalette as _;
use crate::ui::dialogs::{focus_input, footer};

/// What "Create Patch…" diffs.
#[derive(Clone, Debug)]
pub enum PatchSource {
    /// From the Log: `old..new` (the parent of the oldest selected commit to the newest).
    Commits { old: String, new: String, label: String },
    /// From the Commit tool window: local changes in these files.
    Local { paths: Vec<String> },
    /// From the Changes tool window: these files between `old` and `new` (the working tree when `None`).
    Between { old: String, new: Option<String>, paths: Vec<String> },
}

impl PatchSource {
    fn label(&self) -> String {
        match self {
            PatchSource::Commits { label, .. } => label.clone(),
            PatchSource::Local { .. } => "Local_Changes".into(),
            PatchSource::Between { old, new, .. } => {
                let short = |r: &str| r[..r.len().min(8)].to_owned();
                format!("{}_{}", short(old), new.as_deref().map_or("local".to_owned(), short))
            }
        }
    }
}

fn build(repository: &crate::git::Repository, source: &PatchSource, reverse: bool) -> anyhow::Result<String> {
    match source {
        PatchSource::Commits { old, new, .. } => patch::between(repository, old, new, reverse),
        PatchSource::Local { paths } => patch::local_changes(repository, paths, reverse),
        PatchSource::Between { old, new, paths } => patch::files_between(repository, old, new.as_deref(), paths, reverse),
    }
}

/// Copy as Patch to Clipboard: no dialog.
pub fn copy_patch(model: Entity<RepoModel>, source: PatchSource, cx: &mut App) {
    let Some(repository) = model.read(cx).repository().cloned() else { return };
    let task = cx.background_spawn(async move { build(&repository, &source, false) });
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| match result {
            Ok(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                model.update(cx, |m, cx| m.notify("Patch copied to clipboard", "", false, cx));
            }
            Err(error) => model.update(cx, |m, cx| m.notify("Create Patch failed", error.to_string(), true, cx)),
        });
    })
    .detach();
}

/// IntelliJ's Create Patch dialog: save to a file or copy to the clipboard, optionally reversed.
pub fn create_patch(model: Entity<RepoModel>, source: PatchSource, window: &mut Window, cx: &mut App) {
    let Some(root) = model.read(cx).repository().map(|r| r.root().to_path_buf()) else { return };
    let default = root.join(format!("{}.patch", source.label()));
    let path = cx.new(|cx| InputState::new(window, cx).default_value(default.display().to_string()));
    let to_clipboard = Rc::new(Cell::new(false));
    let reverse = Rc::new(Cell::new(false));
    let focus = path.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let secondary = cx.palette().text_secondary;
        let (clip_set, clip_ok) = (to_clipboard.clone(), to_clipboard.clone());
        let (rev_set, rev_ok) = (reverse.clone(), reverse.clone());
        let path_ok = path.clone();
        let browse_path = path.clone();
        let model = model.clone();
        let source = source.clone();
        let root = root.clone();
        dialog
            .title("Create Patch")
            .w(px(560.))
            .child(
                v_flex()
                    .gap_3()
                    .child(
                        RadioGroup::new("patch-target")
                            .children(["Save to file", "Copy to clipboard"])
                            .selected_index(Some(to_clipboard.get() as usize))
                            .on_change(move |ix, window, _| {
                                clip_set.set(*ix == 1);
                                window.refresh();
                            }),
                    )
                    .when(!to_clipboard.get(), |el| {
                        el.child(
                            h_flex()
                                .gap_2()
                                .child(div().text_sm().child("File:"))
                                .child(div().flex_1().child(Input::new(&path).small()))
                                .child(Button::new("patch-browse").small().label("…").on_click(move |_, _, cx| {
                                    save_prompt(&browse_path, &root, cx)
                                })),
                        )
                    })
                    .child(
                        Checkbox::new("patch-reverse").label("Reverse patch").checked(reverse.get()).on_change(
                            move |value, window, _| {
                                rev_set.set(*value);
                                window.refresh();
                            },
                        ),
                    )
                    .child(div().text_xs().text_color(secondary).child(match &source {
                        PatchSource::Commits { .. } => "Unified diff of the selected commits, binary files included.".to_owned(),
                        PatchSource::Between { paths, .. } => format!(
                            "Unified diff of {} compared file{}, binary files included.",
                            paths.len(),
                            if paths.len() == 1 { "" } else { "s" }
                        ),
                        PatchSource::Local { paths } => format!(
                            "Unified diff of local changes in {} file{}, unversioned files included.",
                            paths.len(),
                            if paths.len() == 1 { "" } else { "s" }
                        ),
                    })),
            )
            .on_ok(move |_, _, cx| {
                let source = source.clone();
                let reverse = rev_ok.get();
                if clip_ok.get() {
                    let Some(repository) = model.read(cx).repository().cloned() else { return true };
                    let task = cx.background_spawn(async move { build(&repository, &source, reverse) });
                    let model = model.clone();
                    cx.spawn(async move |cx| {
                        let result = task.await;
                        cx.update(|cx| match result {
                            Ok(text) => {
                                cx.write_to_clipboard(ClipboardItem::new_string(text));
                                model.update(cx, |m, cx| m.notify("Patch copied to clipboard", "", false, cx));
                            }
                            Err(error) => {
                                model.update(cx, |m, cx| m.notify("Create Patch failed", error.to_string(), true, cx))
                            }
                        });
                    })
                    .detach();
                    return true;
                }
                let target = PathBuf::from(path_ok.read(cx).value().trim());
                if target.as_os_str().is_empty() {
                    return false;
                }
                model.update(cx, |model, cx| {
                    model.run_operation("Create Patch", move |repo| {
                        let text = build(repo, &source, reverse)?;
                        if let Some(parent) = target.parent() {
                            std::fs::create_dir_all(parent).ok();
                        }
                        std::fs::write(&target, text)?;
                        Ok(format!("Patch saved to {}", target.display()))
                    }, cx)
                });
                true
            })
            .footer(footer("Create"))
    });
    focus_input(&focus, window, cx);
}

fn save_prompt(input: &Entity<InputState>, root: &std::path::Path, cx: &mut App) {
    let receiver = cx.prompt_for_new_path(root, Some("changes.patch"));
    let input = input.clone();
    cx.spawn(async move |cx| {
        if let Ok(Ok(Some(path))) = receiver.await {
            cx.update(|cx| {
                if let Some(window) = cx.active_window() {
                    window
                        .update(cx, |_, window, cx| {
                            input.update(cx, |state, cx| state.set_value(path.display().to_string(), window, cx))
                        })
                        .ok();
                }
            });
        }
    })
    .detach();
}

/// Apply Patch: a patch file (or clipboard text), the files it changes, and Apply.
pub struct ApplyPatchView {
    model: Entity<RepoModel>,
    path: Option<Entity<InputState>>,
    text: Option<String>,
    files: Vec<PatchFile>,
    error: Option<String>,
    _load: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl ApplyPatchView {
    fn new(model: Entity<RepoModel>, clipboard: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = Vec::new();
        let path = clipboard.is_none().then(|| {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Path to a .patch or .diff file"));
            subscriptions.push(cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.reload(cx);
                }
            }));
            input
        });
        let mut this = Self {
            model,
            path,
            text: clipboard,
            files: Vec::new(),
            error: None,
            _load: None,
            _subscriptions: subscriptions,
        };
        this.reload(cx);
        this
    }

    fn patch_text(&self, cx: &App) -> anyhow::Result<String> {
        if let Some(text) = &self.text {
            return Ok(text.clone());
        }
        let path = self.path.as_ref().map(|p| p.read(cx).value().trim().to_owned()).unwrap_or_default();
        if path.is_empty() {
            anyhow::bail!("Choose a patch file");
        }
        Ok(std::fs::read_to_string(&path)?)
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(repository) = self.model.read(cx).repository().cloned() else { return };
        let text = self.patch_text(cx);
        self._load = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { text.and_then(|text| patch::files(&repository, &text)) })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(files) if files.is_empty() => {
                        this.files.clear();
                        this.error = Some("The patch doesn't change any files".into());
                    }
                    Ok(files) => {
                        this.files = files;
                        this.error = None;
                    }
                    Err(error) => {
                        this.files.clear();
                        this.error = Some(error.to_string().lines().last().unwrap_or_default().to_owned());
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn browse(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self.path.clone() else { return };
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Apply Patch".into()),
        });
        cx.spawn(async move |_, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await {
                let Some(path) = paths.into_iter().next() else { return };
                cx.update(|cx| {
                    if let Some(window) = cx.active_window() {
                        window
                            .update(cx, |_, window, cx| {
                                input.update(cx, |state, cx| state.set_value(path.display().to_string(), window, cx))
                            })
                            .ok();
                    }
                });
            }
        })
        .detach();
    }
}

impl Render for ApplyPatchView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mut list = v_flex();
        for file in &self.files {
            list = list.child(
                h_flex()
                    .h(px(22.))
                    .px_2()
                    .gap_2()
                    .text_sm()
                    .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().child(file.path.clone()))
                    .child(div().text_xs().text_color(palette.status_added).child(file.added.map_or("bin".into(), |n| format!("+{n}"))))
                    .child(div().text_xs().text_color(palette.status_conflict).child(file.removed.map_or(String::new(), |n| format!("-{n}")))),
            );
        }
        v_flex()
            .gap_2()
            .when_some(self.path.clone(), |el, path| {
                el.child(
                    h_flex()
                        .gap_2()
                        .child(div().text_sm().child("Patch file:"))
                        .child(div().flex_1().child(Input::new(&path).small()))
                        .child(
                            Button::new("apply-browse")
                                .small()
                                .label("…")
                                .on_click(cx.listener(|this, _, _, cx| this.browse(cx))),
                        ),
                )
            })
            .child(div().text_xs().text_color(palette.text_secondary).child(format!(
                "{} file{} will be changed",
                self.files.len(),
                if self.files.len() == 1 { "" } else { "s" }
            )))
            .child(
                div()
                    .id("apply-files")
                    .h(px(200.))
                    .border_1()
                    .border_color(palette.border)
                    .rounded(px(4.))
                    .overflow_y_scrollbar()
                    .child(list),
            )
            .when_some(self.error.clone(), |el, error| el.child(div().text_xs().text_color(palette.status_conflict).child(error)))
    }
}

/// Apply Patch… (`clipboard` false) or Apply Patch from Clipboard….
pub fn apply_patch(model: Entity<RepoModel>, clipboard: bool, window: &mut Window, cx: &mut App) {
    let text = if clipboard {
        match cx.read_from_clipboard().and_then(|item| item.text()) {
            Some(text) => Some(text),
            None => {
                model.update(cx, |m, cx| m.notify("Apply Patch", "The clipboard doesn't contain a patch", true, cx));
                return;
            }
        }
    } else {
        None
    };
    let view = cx.new(|cx| ApplyPatchView::new(model.clone(), text, window, cx));
    let focus = view.read(cx).path.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let view_ok = view.clone();
        let model = model.clone();
        dialog
            .title(if clipboard { "Apply Patch from Clipboard" } else { "Apply Patch" })
            .w(px(560.))
            .child(view.clone())
            .on_ok(move |_, _, cx| {
                let view = view_ok.read(cx);
                if view.files.is_empty() {
                    return false;
                }
                let Ok(text) = view.patch_text(cx) else { return false };
                let count = view.files.len();
                model.update(cx, |model, cx| {
                    model.run_operation("Apply Patch", move |repo| {
                        let files = if count == 1 { "1 file".to_owned() } else { format!("{count} files") };
                        Ok(match patch::apply(repo, &text, false)? {
                            ApplyOutcome::Clean => format!("Patch applied to {files}"),
                            ApplyOutcome::Merged => format!("Patch applied to {files} with a three-way merge"),
                            ApplyOutcome::Conflicts => "Patch applied with conflicts; resolve them in the Commit tool window".into(),
                        })
                    }, cx)
                });
                true
            })
            .footer(footer("Apply"))
    });
    if let Some(input) = focus {
        focus_input(&input, window, cx);
    }
}

/// Shelve Changes: a name for the shelf, then the files are saved and rolled back.
pub fn shelve(model: Entity<RepoModel>, paths: Vec<String>, default_name: String, window: &mut Window, cx: &mut App) {
    if paths.is_empty() {
        model.update(cx, |m, cx| m.notify("Shelve Changes", "Select the files to shelve", true, cx));
        return;
    }
    let name = cx.new(|cx| InputState::new(window, cx).default_value(default_name));
    let keep = Rc::new(Cell::new(false));
    let focus = name.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let secondary = cx.palette().text_secondary;
        let (keep_set, keep_ok) = (keep.clone(), keep.clone());
        let name_ok = name.clone();
        let model = model.clone();
        let paths = paths.clone();
        let count = paths.len();
        dialog
            .title("Shelve Changes")
            .w(px(460.))
            .child(
                v_flex()
                    .gap_3()
                    .child(div().text_sm().text_color(secondary).child(format!(
                        "{count} file{} will be saved to the Shelf",
                        if count == 1 { "" } else { "s" }
                    )))
                    .child(Input::new(&name))
                    .child(
                        Checkbox::new("shelve-keep")
                            .label("Keep changes in the working tree")
                            .checked(keep.get())
                            .on_change(move |value, window, _| {
                                keep_set.set(*value);
                                window.refresh();
                            }),
                    ),
            )
            .on_ok(move |_, _, cx| {
                let name = name_ok.read(cx).value().trim().to_owned();
                if name.is_empty() {
                    return false;
                }
                let paths = paths.clone();
                let keep = keep_ok.get();
                model.update(cx, |model, cx| {
                    model.run_operation("Shelve Changes", move |repo| {
                        patch::shelve(repo, &paths, &name, keep)?;
                        Ok(format!("Shelved {} file{} as \u{201c}{name}\u{201d}", paths.len(), if paths.len() == 1 { "" } else { "s" }))
                    }, cx)
                });
                true
            })
            .footer(footer("Shelve Changes"))
    });
    focus_input(&focus, window, cx);
}

/// Shelve Silently: no dialog, named after the files.
pub fn shelve_silently(model: Entity<RepoModel>, paths: Vec<String>, cx: &mut App) {
    if paths.is_empty() {
        return;
    }
    let name = default_shelf_name(&paths);
    model.update(cx, |model, cx| {
        model.run_operation("Shelve Changes", move |repo| {
            patch::shelve(repo, &paths, &name, false)?;
            Ok(format!("Shelved \u{201c}{name}\u{201d}"))
        }, cx)
    });
}

pub fn default_shelf_name(paths: &[String]) -> String {
    let first = paths.first().map(|p| p.rsplit('/').next().unwrap_or(p).to_owned()).unwrap_or_default();
    match paths.len() {
        0 => "Changes".into(),
        1 => format!("Changes in {first}"),
        n => format!("Changes in {first} and {} more", n - 1),
    }
}
