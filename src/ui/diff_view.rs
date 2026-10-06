//! The editor area: shows a unified diff of the file picked in the Log's
//! changes tree or the Commit tool window. (Side-by-side viewer is M2.)

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    AppContext as _, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Task, UniformListScrollHandle, Window, div,
    prelude::FluentBuilder as _, px, uniform_list,
};

use crate::git::Repository;
use crate::theme::ActivePalette as _;
use crate::ui::common;

/// What to compare.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffSource {
    /// A file as changed by a commit (against its first parent).
    Commit { hash: String, path: String, old_path: Option<String> },
    /// A file in the working tree against HEAD.
    WorkingTree { path: String, unversioned: bool },
}

impl DiffSource {
    fn title(&self) -> String {
        match self {
            DiffSource::Commit { hash, path, .. } => format!("{path}  ({})", &hash[..hash.len().min(8)]),
            DiffSource::WorkingTree { path, .. } => format!("{path}  (Local Changes)"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LineKind {
    Context,
    Added,
    Removed,
    Hunk,
    Meta,
}

#[derive(Clone)]
struct DiffLine {
    kind: LineKind,
    old_line: Option<u32>,
    new_line: Option<u32>,
    text: SharedString,
}

pub struct DiffView {
    source: Option<DiffSource>,
    lines: Vec<DiffLine>,
    error: Option<String>,
    scroll: UniformListScrollHandle,
    _task: Option<Task<()>>,
}

impl DiffView {
    pub fn new() -> Self {
        Self { source: None, lines: Vec::new(), error: None, scroll: UniformListScrollHandle::new(), _task: None }
    }

    pub fn show(&mut self, repository: Repository, source: DiffSource, cx: &mut Context<Self>) {
        if self.source.as_ref() == Some(&source) {
            return;
        }
        self.source = Some(source.clone());
        self.lines.clear();
        self.error = None;
        cx.notify();
        self._task = Some(cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { load_diff(&repository, &source) }).await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(lines) => this.lines = lines,
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            })
            .ok();
        }));
    }
}

fn load_diff(repository: &Repository, source: &DiffSource) -> anyhow::Result<Vec<DiffLine>> {
    let output = match source {
        DiffSource::Commit { hash, path, old_path } => {
            let mut args = vec!["show".to_owned(), "--format=".into(), "-M".into(), "--first-parent".into(), hash.clone(), "--".into()];
            args.extend(old_path.iter().cloned());
            args.push(path.clone());
            repository.run(&args)?
        }
        DiffSource::WorkingTree { path, unversioned: true } => {
            // A new file has no diff in git's eyes: show it as all added.
            let content = std::fs::read_to_string(repository.root().join(path)).unwrap_or_default();
            let mut text = format!("@@ -0,0 +1,{} @@\n", content.lines().count());
            for line in content.lines() {
                text.push('+');
                text.push_str(line);
                text.push('\n');
            }
            text
        }
        DiffSource::WorkingTree { path, .. } => {
            let has_head = repository.run(["rev-parse", "--verify", "-q", "HEAD"]).is_ok();
            if has_head {
                repository.run(["diff", "HEAD", "-M", "--", path])?
            } else {
                repository.run(["diff", "--cached", "--", path])?
            }
        }
    };
    Ok(parse_unified(&output))
}

fn parse_unified(output: &str) -> Vec<DiffLine> {
    let mut lines = Vec::new();
    let (mut old_line, mut new_line) = (0u32, 0u32);
    let mut in_hunk = false;
    for raw in output.lines() {
        let line = if let Some(header) = raw.strip_prefix("@@") {
            in_hunk = true;
            // @@ -a,b +c,d @@
            let mut parts = header.split_whitespace();
            old_line = parts.next().and_then(|p| p.trim_start_matches('-').split(',').next()?.parse().ok()).unwrap_or(1);
            new_line = parts.next().and_then(|p| p.trim_start_matches('+').split(',').next()?.parse().ok()).unwrap_or(1);
            DiffLine { kind: LineKind::Hunk, old_line: None, new_line: None, text: raw.to_owned().into() }
        } else if !in_hunk {
            DiffLine { kind: LineKind::Meta, old_line: None, new_line: None, text: raw.to_owned().into() }
        } else if let Some(text) = raw.strip_prefix('+') {
            new_line += 1;
            DiffLine { kind: LineKind::Added, old_line: None, new_line: Some(new_line - 1), text: text.to_owned().into() }
        } else if let Some(text) = raw.strip_prefix('-') {
            old_line += 1;
            DiffLine { kind: LineKind::Removed, old_line: Some(old_line - 1), new_line: None, text: text.to_owned().into() }
        } else if raw.starts_with("diff --git") {
            in_hunk = false;
            DiffLine { kind: LineKind::Meta, old_line: None, new_line: None, text: raw.to_owned().into() }
        } else {
            old_line += 1;
            new_line += 1;
            let text = raw.strip_prefix(' ').unwrap_or(raw);
            DiffLine { kind: LineKind::Context, old_line: Some(old_line - 1), new_line: Some(new_line - 1), text: text.to_owned().into() }
        };
        lines.push(line);
    }
    lines
}

impl Render for DiffView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let Some(source) = self.source.clone() else {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .text_color(palette.text_secondary)
                .child(div().text_lg().child("Select a file to view changes"))
                .child(div().text_sm().child("Double-click a file in the Log or the Commit tool window"))
                .into_any_element();
        };
        let mono = cx.theme().mono_font_family.clone();
        let lines = self.lines.clone();
        let count = lines.len();
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(px(30.))
                    .px_3()
                    .gap_2()
                    .border_b_1()
                    .border_color(palette.border)
                    .bg(palette.toolbar)
                    .child(common::icon(common::file_icon(match &source {
                        DiffSource::Commit { path, .. } | DiffSource::WorkingTree { path, .. } => path,
                    })))
                    .child(div().text_sm().child(source.title())),
            )
            .when_some(self.error.clone(), |el, error| el.child(div().p_3().text_color(palette.status_conflict).child(error)))
            .child(
                uniform_list("diff-lines", count, move |range, _, cx| {
                    let palette = cx.palette().clone();
                    range
                        .map(|ix| {
                            let line = &lines[ix];
                            let (bg, fg) = match line.kind {
                                LineKind::Added => (Some(palette.diff_inserted), palette.text),
                                LineKind::Removed => (Some(palette.diff_deleted), palette.text),
                                LineKind::Hunk => (Some(palette.diff_header), palette.text_secondary),
                                LineKind::Meta => (None, palette.text_secondary),
                                LineKind::Context => (None, palette.text),
                            };
                            let number = |n: Option<u32>| {
                                div()
                                    .w(px(44.))
                                    .flex_shrink_0()
                                    .text_right()
                                    .pr_2()
                                    .text_color(palette.text_disabled)
                                    .child(n.map(|n| n.to_string()).unwrap_or_default())
                            };
                            h_flex()
                                .id(ix)
                                .h(px(20.))
                                .w_full()
                                .when_some(bg, |el, bg| el.bg(bg))
                                .child(number(line.old_line))
                                .child(number(line.new_line))
                                .child(
                                    div()
                                        .pl_2()
                                        .text_color(fg)
                                        .when(line.kind == LineKind::Hunk, |el| el.font_weight(FontWeight::MEDIUM))
                                        .whitespace_nowrap()
                                        .child(line.text.clone()),
                                )
                        })
                        .collect()
                })
                .track_scroll(&self.scroll)
                .font_family(mono)
                .text_size(px(12.5))
                .flex_1()
                .w_full(),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_lines_from_hunk_headers() {
        let lines = parse_unified("diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -3,2 +3,3 @@ fn\n ctx\n-old\n+new\n+more\n");
        let body: Vec<_> = lines.iter().filter(|l| l.kind != LineKind::Meta).collect();
        assert!(body[0].kind == LineKind::Hunk);
        assert_eq!((body[1].old_line, body[1].new_line), (Some(3), Some(3)));
        assert_eq!(body[2].old_line, Some(4));
        assert_eq!(body[3].new_line, Some(4));
        assert_eq!(body[4].new_line, Some(5));
    }
}
