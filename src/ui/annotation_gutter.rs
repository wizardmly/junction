//! Annotate with Git Blame in the editor's gutter, as IntelliJ shows it:
//! date and author beside every line, tinted by age, the commit's details
//! on hover, and the annotation actions on right-click.

use std::cell::Cell;
use std::rc::Rc;

use chrono::{Local, TimeZone as _};
use gpui_kit::component::{
    ActiveTheme as _,
    input::RopeExt as _,
    menu::{ContextMenuExt as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::{
    App, Bounds, ClipboardItem, DispatchPhase, Entity, InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, ParentElement as _, Pixels, Point, SharedString, Styled as _, TextAlign, TextRun, WeakEntity, Window, canvas,
    div, fill, font, point, prelude::FluentBuilder as _, px, size,
};

use crate::git::blame::{self, Blame, BlameCommit};
use crate::theme::ActivePalette as _;
use crate::ui::common;
use crate::ui::file_editor::FileEditor;

const FONT_SIZE: f32 = 12.;
const PADDING: f32 = 6.;
/// Author names are cut to this many characters, as IntelliJ's column.
const AUTHOR_CHARS: usize = 14;

/// The annotation's View options (IntelliJ's View submenu).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnnotationView {
    pub date: bool,
    pub author: bool,
    pub hash: bool,
}

impl Default for AnnotationView {
    fn default() -> Self {
        Self { date: true, author: true, hash: false }
    }
}

/// Blame of one editor's text and how it is shown.
#[derive(Default)]
pub struct Annotation {
    pub blame: Rc<Blame>,
    /// Per commit: 0 for the oldest, 1 for the newest.
    age: Rc<Vec<f32>>,
    pub loading: bool,
    pub error: Option<String>,
    pub view: AnnotationView,
    /// The line under the mouse and where the mouse is, from the last paint.
    pub hovered: Rc<Cell<Option<(usize, Point<Pixels>)>>>,
}

/// What the gutter asks its editor to do.
pub enum AnnotationAction {
    /// Annotate the file as of a commit (Annotate Revision / Previous Revision).
    Annotate { hash: String, path: String },
    SelectInLog(String),
    ShowDiff { hash: String, path: String },
    SetView(AnnotationView),
    Close,
    /// The hovered line changed: restart the hover card's delay.
    Hover,
    /// A click: the hover card makes way.
    HideCard,
}

impl Annotation {
    /// Shown while the first blame runs.
    pub fn loading() -> Self {
        Self { loading: true, ..Default::default() }
    }

    pub fn set_blame(&mut self, blame: Blame) {
        self.age = Rc::new(ages(&blame));
        self.blame = Rc::new(blame);
        self.loading = false;
        self.error = None;
    }

    /// The commit annotating a 0-based line.
    pub fn commit_at(&self, line: usize) -> Option<&BlameCommit> {
        self.blame.lines.get(line).and_then(|l| self.blame.commits.get(l.commit))
    }

    fn label(&self, commit: &BlameCommit) -> String {
        if blame::is_uncommitted(&commit.hash) {
            return "Not Committed Yet".to_owned();
        }
        let mut parts = Vec::new();
        if self.view.date {
            parts.push(short_date(commit.author_time));
        }
        if self.view.author {
            let name: String = commit.author.chars().take(AUTHOR_CHARS).collect();
            parts.push(format!("{name:<AUTHOR_CHARS$}"));
        }
        if self.view.hash {
            parts.push(commit.hash[..commit.hash.len().min(8)].to_owned());
        }
        parts.join(" ").trim_end().to_owned()
    }

    /// Characters the widest label takes, for the column's width.
    fn chars(&self) -> usize {
        let columns = [(self.view.date, 10), (self.view.author, AUTHOR_CHARS), (self.view.hash, 8)];
        let shown: Vec<usize> = columns.iter().filter(|(on, _)| *on).map(|(_, n)| *n).collect();
        let width = shown.iter().sum::<usize>() + shown.len().saturating_sub(1);
        // "Not Committed Yet" when every column is hidden but the line is new.
        if self.blame.commits.iter().any(|c| blame::is_uncommitted(&c.hash)) { width.max(17) } else { width.max(1) }
    }

    /// The gutter column, painted beside the editor from its layout so it
    /// scrolls with the text.
    pub fn gutter(&self, state: Entity<gpui_kit::component::input::EditorState>, editor: WeakEntity<FileEditor>, window: &mut Window, cx: &mut App) -> impl IntoElement + use<> {
        let palette = cx.palette().clone();
        let mono = cx.theme().mono_font_family.clone();
        let run = {
            let (mono, color) = (mono.clone(), palette.text_secondary);
            move |len: usize| TextRun { len, font: font(mono.clone()), color, background_color: None, underline: None, strikethrough: None }
        };
        let char_width = window.text_system().shape_line("0".into(), px(FONT_SIZE), &[run(1)], None).width;
        let width = char_width * self.chars() as f32 + px(PADDING * 2.);
        let labels: Rc<Vec<String>> = Rc::new(self.blame.commits.iter().map(|c| self.label(c)).collect());
        let blame_lines = self.blame.clone();
        let age = self.age.clone();
        let hovered = self.hovered.clone();
        let paint_editor = editor.clone();
        let paint = canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                // Where each visible line sits, from the editor's layout.
                let (line_height, placed) = {
                    let state = state.read(cx);
                    let Some(line_height) = state.line_height() else { return };
                    let Some(visible) = state.visible_row_range() else { return };
                    let rope = state.text();
                    let lines = rope.lines_len().max(1);
                    let placed: Vec<(usize, Pixels)> = (visible.start..visible.end.min(lines))
                        .filter_map(|line| {
                            let at = rope.line_start_offset(line);
                            Some((line, state.range_to_bounds(&(at..at))?.top()))
                        })
                        .collect();
                    (line_height, placed)
                };
                let hovered_line = hovered.get().map(|(line, _)| line);
                let hovered_commit = hovered_line.and_then(|l| blame_lines.lines.get(l)).map(|l| l.commit);
                let mut rows: Vec<(Bounds<Pixels>, usize)> = Vec::new();
                for (line, top) in placed {
                    let Some(blamed) = blame_lines.lines.get(line) else { continue };
                    let row = Bounds::new(point(bounds.left(), top), size(bounds.size.width, line_height)).intersect(&bounds);
                    if row.size.height <= px(0.) {
                        continue;
                    }
                    let commit = &blame_lines.commits[blamed.commit];
                    let uncommitted = blame::is_uncommitted(&commit.hash);
                    // Newer commits are tinted stronger, as IntelliJ's "Color by age".
                    let tint = if hovered_commit == Some(blamed.commit) {
                        palette.selection.opacity(0.6)
                    } else if uncommitted {
                        palette.status_modified.opacity(0.18)
                    } else {
                        palette.accent.opacity(0.04 + 0.22 * age[blamed.commit])
                    };
                    window.paint_quad(fill(row, tint));
                    let label = &labels[blamed.commit];
                    let mut text_run = run(label.len());
                    if hovered_commit == Some(blamed.commit) {
                        text_run.color = palette.text;
                    }
                    let shaped = window.text_system().shape_line(SharedString::from(label.clone()), px(FONT_SIZE), &[text_run], None);
                    let _ = shaped.paint(point(bounds.left() + px(PADDING), top), line_height, TextAlign::Left, None, window, cx);
                    rows.push((row, line));
                }
                let (move_rows, move_editor, move_hovered) = (rows.clone(), paint_editor.clone(), hovered.clone());
                window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    let line = move_rows.iter().find(|(r, _)| r.contains(&e.position)).map(|(_, l)| *l);
                    let was = move_hovered.get().map(|(l, _)| l);
                    move_hovered.set(line.map(|l| (l, e.position)));
                    if was != line {
                        move_editor.update(cx, |editor, cx| editor.annotation_action(AnnotationAction::Hover, cx)).ok();
                    }
                });
                let click_editor = paint_editor.clone();
                window.on_mouse_event(move |e: &MouseDownEvent, phase, _, cx| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    let Some(&(_, line)) = rows.iter().find(|(r, _)| r.contains(&e.position)) else { return };
                    click_editor.update(cx, |editor, cx| editor.annotation_action(AnnotationAction::HideCard, cx)).ok();
                    if e.button != MouseButton::Left {
                        return;
                    }
                    cx.stop_propagation();
                    click_editor
                        .update(cx, |editor, cx| {
                            let hash = editor.annotation().and_then(|a| a.commit_at(line)).map(|c| c.hash.clone());
                            if let Some(hash) = hash.filter(|h| !blame::is_uncommitted(h)) {
                                editor.annotation_action(AnnotationAction::SelectInLog(hash), cx);
                            }
                        })
                        .ok();
                });
            },
        )
        .absolute()
        .inset_0();

        let menu_hovered = self.hovered.clone();
        let commit_of = {
            let blame = self.blame.clone();
            move |line: usize| blame.lines.get(line).and_then(|l| blame.commits.get(l.commit)).cloned()
        };
        let view = self.view;
        div()
            .id("annotation-gutter")
            .relative()
            .h_full()
            .w(width)
            .flex_shrink_0()
            .border_r_1()
            .border_color(palette.border)
            .child(paint)
            .context_menu(move |menu, window, cx| {
                let commit = menu_hovered.get().and_then(|(line, _)| commit_of(line));
                let committed = commit.clone().filter(|c| !blame::is_uncommitted(&c.hash));
                let hash = committed.as_ref().map(|c| c.hash.clone()).unwrap_or_default();
                let path = committed.as_ref().map(|c| c.path.clone()).unwrap_or_default();
                let previous = committed.as_ref().and_then(|c| c.previous.clone());
                let act_editor = editor.clone();
                let act = move |action: AnnotationAction| {
                    let editor = act_editor.clone();
                    let action = Rc::new(Cell::new(Some(action)));
                    move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
                        if let Some(action) = action.take() {
                            editor.update(cx, |editor, cx| editor.annotation_action(action, cx)).ok();
                        }
                    }
                };
                let toggle_act = act.clone();
                let toggle = move |label: &'static str, on: bool, edit: fn(&mut AnnotationView)| {
                    let act = toggle_act.clone();
                    let mut next = view;
                    edit(&mut next);
                    PopupMenuItem::new(label).checked(on).on_click(act(AnnotationAction::SetView(next)))
                };
                let copy = hash.clone();
                menu.item(
                    PopupMenuItem::new("Annotate Revision")
                        .disabled(committed.is_none())
                        .on_click(act(AnnotationAction::Annotate { hash: hash.clone(), path: path.clone() })),
                )
                // The commit that added the file has no previous revision.
                .item(PopupMenuItem::new("Annotate Previous Revision").disabled(previous.is_none()).on_click({
                    let (hash, path) = previous.clone().unwrap_or_default();
                    act(AnnotationAction::Annotate { hash, path })
                }))
                .separator()
                .item(
                    PopupMenuItem::new("Show Diff")
                        .disabled(committed.is_none())
                        .on_click(act(AnnotationAction::ShowDiff { hash: hash.clone(), path: path.clone() })),
                )
                .item(PopupMenuItem::new("Select in Git Log").disabled(committed.is_none()).on_click(act(AnnotationAction::SelectInLog(hash.clone()))))
                .item(
                    PopupMenuItem::new("Copy Revision Number")
                        .disabled(committed.is_none())
                        .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))),
                )
                .separator()
                .submenu("View", window, cx, move |m, _, _| {
                    m.item(toggle("Date", view.date, |v| v.date = !v.date))
                        .item(toggle("Author", view.author, |v| v.author = !v.author))
                        .item(toggle("Commit Hash", view.hash, |v| v.hash = !v.hash))
                })
                .separator()
                .item(PopupMenuItem::new("Close Annotations").on_click(act(AnnotationAction::Close)))
            })
    }

    /// The hover card for a line: the commit's hash, author, date and message.
    pub fn hover_card(&self, line: usize, cx: &App) -> Option<impl IntoElement + use<>> {
        let commit = self.commit_at(line)?.clone();
        let palette = cx.palette().clone();
        let card = v_flex()
            .max_w(px(460.))
            .p_2()
            .gap_1()
            .rounded(px(4.))
            .border_1()
            .border_color(palette.border)
            .bg(cx.theme().popover)
            .shadow_md()
            .text_sm();
        Some(if blame::is_uncommitted(&commit.hash) {
            card.child("Not Committed Yet")
        } else {
            card.child(div().text_color(palette.link).child(commit.hash.clone()))
                .child(div().text_color(palette.text_secondary).child(format!(
                    "{} <{}>  {}",
                    commit.author,
                    commit.author_email,
                    common::format_full_date(commit.author_time)
                )))
                .when(!commit.summary.is_empty(), |el| el.child(div().pt_1().child(commit.summary.clone())))
        })
    }
}

/// Ranks commits by author time so the gutter can fade older lines.
fn ages(blame: &Blame) -> Vec<f32> {
    let mut times: Vec<i64> = blame.commits.iter().filter(|c| !blame::is_uncommitted(&c.hash)).map(|c| c.author_time).collect();
    times.sort_unstable();
    times.dedup();
    let span = times.len().saturating_sub(1).max(1) as f32;
    blame
        .commits
        .iter()
        .map(|c| {
            if blame::is_uncommitted(&c.hash) {
                1.
            } else {
                times.binary_search(&c.author_time).unwrap_or(0) as f32 / span
            }
        })
        .collect()
}

fn short_date(unix_seconds: i64) -> String {
    Local.timestamp_opt(unix_seconds, 0).single().map(|t| t.format("%Y/%m/%d").to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::blame::BlameLine;

    fn commit(hash: &str, time: i64, author: &str) -> BlameCommit {
        BlameCommit {
            hash: hash.into(),
            author: author.into(),
            author_email: String::new(),
            author_time: time,
            summary: String::new(),
            path: String::new(),
            previous: None,
        }
    }

    #[test]
    fn newest_commit_is_brightest_and_labels_follow_the_view() {
        let blame = Blame {
            commits: vec![commit(&"a".repeat(40), 300, "Alice"), commit(&"b".repeat(40), 100, "Bob"), commit(&"0".repeat(40), 0, "")],
            lines: vec![BlameLine { commit: 0, text: String::new(), original_line: 1 }],
        };
        assert_eq!(ages(&blame), vec![1., 0., 1.]);
        let mut annotation = Annotation::default();
        annotation.set_blame(blame);
        annotation.view = AnnotationView { date: false, author: true, hash: true };
        assert_eq!(annotation.label(&annotation.blame.commits[0].clone()), format!("Alice{} aaaaaaaa", " ".repeat(AUTHOR_CHARS - 5)));
        assert_eq!(annotation.label(&annotation.blame.commits[2].clone()), "Not Committed Yet");
        assert_eq!(annotation.chars(), AUTHOR_CHARS + 1 + 8);
    }
}
