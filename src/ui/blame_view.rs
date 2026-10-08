//! Annotate with Git Blame, after IntelliJ's annotation gutter: date, author
//! and revision for every line, colored by age, with the commit's details on
//! hover and the usual gutter actions on right-click.

use std::ops::Range;
use std::rc::Rc;

use chrono::{Local, TimeZone as _};
use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, h_flex,
    menu::{ContextMenuExt as _, PopupMenuItem},
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::assets::IconName;
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, EventEmitter, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Task, UniformListScrollHandle, Window,
    div, prelude::FluentBuilder as _, px, uniform_list,
};

use crate::git::Repository;
use crate::git::blame::{self, Blame};
use crate::model::RepoModel;
use crate::theme::ActivePalette as _;
use crate::ui::common::{self, tool_button};
use crate::ui::diff_view::DiffSource;

const LINE_HEIGHT: f32 = 20.;

pub enum BlameEvent {
    SelectCommit(String),
    ShowDiff(DiffSource),
    Closed,
}

impl EventEmitter<BlameEvent> for BlameView {}

pub struct BlameView {
    model: Entity<RepoModel>,
    repository: Repository,
    path: String,
    /// `None` annotates the working tree.
    revision: Option<String>,
    blame: Rc<Blame>,
    /// Per commit: 0 for the oldest, 1 for the newest, for "Color by date".
    age: Rc<Vec<f32>>,
    error: Option<String>,
    loading: bool,
    show_date: bool,
    show_author: bool,
    show_hash: bool,
    scroll: UniformListScrollHandle,
    _task: Option<Task<()>>,
}

impl BlameView {
    pub fn new(model: Entity<RepoModel>, repository: Repository, path: String, revision: Option<String>, cx: &mut Context<Self>) -> Self {
        cx.observe(&model, |_, _, cx| cx.notify()).detach();
        let mut this = Self {
            model,
            repository,
            path,
            revision,
            blame: Rc::default(),
            age: Rc::default(),
            error: None,
            loading: false,
            show_date: true,
            show_author: true,
            show_hash: false,
            scroll: UniformListScrollHandle::new(),
            _task: None,
        };
        this.load(cx);
        this
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn revision(&self) -> Option<&str> {
        self.revision.as_deref()
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        self.error = None;
        let repository = self.repository.clone();
        let path = self.path.clone();
        let revision = self.revision.clone();
        self._task = Some(cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { blame::blame(&repository, &path, revision.as_deref()) }).await;
            this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(blame) => {
                        this.age = Rc::new(ages(&blame));
                        this.blame = Rc::new(blame);
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// "Annotate Previous Revision": blame the file as it was before `hash`.
    fn annotate_previous(&mut self, commit: usize, cx: &mut Context<Self>) {
        let Some(commit) = self.blame.commits.get(commit) else { return };
        if blame::is_uncommitted(&commit.hash) {
            return;
        }
        if !commit.path.is_empty() {
            self.path = commit.path.clone();
        }
        self.revision = Some(format!("{}^", commit.hash));
        self.blame = Rc::default();
        self.load(cx);
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let revision = match &self.revision {
            None => "Working Tree".to_owned(),
            Some(r) => r.strip_suffix('^').map_or_else(|| short(r).to_owned(), |h| format!("{}^", short(h))),
        };
        h_flex()
            .h(px(crate::ui::common::toolbar_height()))
            .px_2()
            .gap_1()
            .border_b_1()
            .border_color(palette.border)
            .bg(palette.toolbar)
            .child(common::icon(common::file_icon(&self.path)))
            .child(div().text_sm().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(self.path.clone()))
            .child(div().text_sm().text_color(palette.text_secondary).child(format!("Annotated · {revision}")))
            .child(div().flex_1())
            .child(
                tool_button("blame-date", IconName::Calendar, "Show Date")
                    .selected(self.show_date)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_date = !this.show_date;
                        cx.notify()
                    })),
            )
            .child(
                tool_button("blame-author", IconName::User, "Show Author")
                    .selected(self.show_author)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_author = !this.show_author;
                        cx.notify()
                    })),
            )
            .child(
                tool_button("blame-hash", IconName::Hash, "Show Commit Hash")
                    .selected(self.show_hash)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_hash = !this.show_hash;
                        cx.notify()
                    })),
            )
            .child(tool_button("blame-refresh", IconName::RefreshCw, "Refresh").on_click(cx.listener(|this, _, _, cx| this.load(cx))))
            .child(
                tool_button("blame-close", IconName::Close, "Close Annotations")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(BlameEvent::Closed))),
            )
    }
}

fn short(hash: &str) -> &str {
    &hash[..hash.len().min(8)]
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

impl Render for BlameView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mono = cx.theme().mono_font_family.clone();
        let blame = self.blame.clone();
        let age = self.age.clone();
        let (show_date, show_author, show_hash) = (self.show_date, self.show_author, self.show_hash);
        let selected = self.model.read(cx).selected_hash().map(str::to_owned);
        let path = self.path.clone();
        let digits = blame.lines.len().max(1).to_string().len() as f32;
        let entity = cx.entity();

        let list = uniform_list(
            "blame-lines",
            blame.lines.len(),
            cx.processor(move |_, range: Range<usize>, _, cx| {
                let palette = cx.palette().clone();
                range
                    .map(|ix| {
                        let line = &blame.lines[ix];
                        let commit = &blame.commits[line.commit];
                        let uncommitted = blame::is_uncommitted(&commit.hash);
                        // The first line of a run carries the annotation, like IntelliJ's merged gutter.
                        let first = ix == 0 || blame.lines[ix - 1].commit != line.commit;
                        let is_selected = selected.as_deref() == Some(commit.hash.as_str());
                        let tint: Hsla = if uncommitted {
                            palette.status_modified.opacity(0.18)
                        } else {
                            palette.accent.opacity(0.04 + 0.26 * age[line.commit])
                        };
                        let commit_ix = line.commit;
                        let hash = commit.hash.clone();
                        let tooltip_text = if uncommitted {
                            "Not Committed Yet".to_owned()
                        } else {
                            format!(
                                "{}\n{} <{}>\n{}\n\n{}",
                                commit.hash,
                                commit.author,
                                commit.author_email,
                                common::format_full_date(commit.author_time),
                                commit.summary
                            )
                        };
                        let (menu_entity, menu_hash, menu_path) = (entity.clone(), hash.clone(), if commit.path.is_empty() { path.clone() } else { commit.path.clone() });
                        let gutter = h_flex()
                            .id(("blame-gutter", ix))
                            .h_full()
                            .flex_shrink_0()
                            .gap_2()
                            .px_2()
                            .bg(if is_selected { palette.selection } else { tint })
                            .text_color(palette.text_secondary)
                            .cursor_pointer()
                            .hover(|s| s.text_color(palette.text))
                            .tooltip(move |window, cx| Tooltip::new(tooltip_text.clone()).build(window, cx))
                            .on_click(cx.listener({
                                let hash = hash.clone();
                                move |_, _, _, cx| {
                                    if !blame::is_uncommitted(&hash) {
                                        cx.emit(BlameEvent::SelectCommit(hash.clone()))
                                    }
                                }
                            }))
                            .context_menu(move |menu, _, _| {
                                let uncommitted = blame::is_uncommitted(&menu_hash);
                                let (e1, e2, e3, e4) = (menu_entity.clone(), menu_entity.clone(), menu_entity.clone(), menu_entity.clone());
                                let (h1, h2, h3) = (menu_hash.clone(), menu_hash.clone(), menu_hash.clone());
                                let p = menu_path.clone();
                                menu.item(PopupMenuItem::new("Show Diff").disabled(uncommitted).on_click(move |_, _, cx| {
                                    let source = DiffSource::Commit { hash: h1.clone(), path: p.clone(), old_path: None };
                                    e1.update(cx, |_, cx| cx.emit(BlameEvent::ShowDiff(source)))
                                }))
                                .item(PopupMenuItem::new("Select in Git Log").disabled(uncommitted).on_click(move |_, _, cx| {
                                    e2.update(cx, |_, cx| cx.emit(BlameEvent::SelectCommit(h2.clone())))
                                }))
                                .item(PopupMenuItem::new("Annotate Previous Revision").disabled(uncommitted).on_click(
                                    move |_, _, cx| e3.update(cx, |this, cx| this.annotate_previous(commit_ix, cx)),
                                ))
                                .separator()
                                .item(PopupMenuItem::new("Copy Revision Number").disabled(uncommitted).on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(h3.clone()))
                                }))
                                .separator()
                                .item(PopupMenuItem::new("Close Annotations").on_click(move |_, _, cx| {
                                    e4.update(cx, |_, cx| cx.emit(BlameEvent::Closed))
                                }))
                            })
                            .map(|el| {
                                if uncommitted {
                                    // One label across the columns, as IntelliJ does.
                                    let width = 76. * show_date as u8 as f32 + 120. * show_author as u8 as f32 + 64. * show_hash as u8 as f32
                                        + 8. * (show_date as u8 + show_author as u8 + show_hash as u8).saturating_sub(1) as f32;
                                    el.child(div().w(px(width)).overflow_hidden().whitespace_nowrap().when(first && width > 0., |el| el.child("Not Committed Yet")))
                                } else {
                                    el.when(show_date, |el| el.child(div().w(px(76.)).when(first, |el| el.child(short_date(commit.author_time)))))
                                        .when(show_author, |el| {
                                            el.child(
                                                div()
                                                    .w(px(120.))
                                                    .overflow_hidden()
                                                    .whitespace_nowrap()
                                                    .text_ellipsis()
                                                    .when(first, |el| el.child(commit.author.clone())),
                                            )
                                        })
                                        .when(show_hash, |el| el.child(div().w(px(64.)).when(first, |el| el.child(short(&commit.hash).to_owned()))))
                                }
                            });
                        h_flex()
                            .id(ix)
                            .h(px(LINE_HEIGHT))
                            .w_full()
                            .child(gutter)
                            .child(
                                div()
                                    .w(px(digits * 8. + 16.))
                                    .flex_shrink_0()
                                    .pr_2()
                                    .text_right()
                                    .text_color(palette.text_disabled)
                                    .child((ix + 1).to_string()),
                            )
                            .child(div().pl_2().whitespace_nowrap().child(line.text.replace('\t', "    ")))
                            .into_any_element()
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.scroll)
        .font_family(mono)
        .text_size(px(12.5))
        .flex_1()
        .w_full();

        v_flex()
            .size_full()
            .child(self.render_toolbar(cx))
            .when(self.loading, |el| el.child(div().p_3().text_color(palette.text_secondary).child("Annotating…")))
            .when_some(self.error.clone(), |el, error| el.child(div().p_3().text_color(palette.status_conflict).child(error)))
            .child(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::blame::{BlameCommit, BlameLine};

    fn commit(hash: &str, time: i64) -> BlameCommit {
        BlameCommit { hash: hash.into(), author: String::new(), author_email: String::new(), author_time: time, summary: String::new(), path: String::new() }
    }

    #[test]
    fn newest_commit_is_brightest() {
        let blame = Blame {
            commits: vec![commit(&"a".repeat(40), 300), commit(&"b".repeat(40), 100), commit(&"0".repeat(40), 0)],
            lines: vec![BlameLine { commit: 0, text: String::new(), original_line: 1 }],
        };
        assert_eq!(ages(&blame), vec![1., 0., 1.]);
    }
}
