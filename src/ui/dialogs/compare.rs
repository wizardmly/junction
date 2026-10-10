//! Compare with Branch / Revision.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::{
    WindowExt as _,
    input::{Input, InputState},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Entity, InteractiveElement as _, ParentElement as _, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, div, prelude::FluentBuilder as _, px,
};

use crate::model::RepoModel;
use crate::theme::ActivePalette as _;

use super::{branch_picker, focus_input, footer};

pub type OpenDiff = Rc<dyn Fn(crate::ui::diff_view::DiffSource, &mut Window, &mut App)>;

/// "Compare with Branch…" / "Compare with Revision…" for one file: pick a
/// branch, tag or revision, then diff that version against the working tree.
pub fn compare_file_with(model: Entity<RepoModel>, path: String, open_diff: OpenDiff, window: &mut Window, cx: &mut App) {
    let revision = cx.new(|cx| InputState::new(window, cx).placeholder("Branch, tag or revision"));
    let focus_target = revision.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let (revision_ok, path_ok, open) = (revision.clone(), path.clone(), open_diff.clone());
        let repository = model.read(cx).repository().cloned();
        dialog
            .title(format!("Compare {path} with…"))
            .w(px(460.))
            .child(branch_picker("compare-file-branches", &revision, &model, cx))
            .on_ok(move |_, window, cx| {
                let revision = revision_ok.read(cx).value().trim().to_owned();
                let valid = repository.as_ref().is_some_and(|repo| {
                    repo.run(["rev-parse", "--verify", "-q", &format!("{revision}^{{commit}}")]).is_ok()
                });
                if revision.is_empty() || !valid {
                    return false;
                }
                let source = crate::ui::diff_view::DiffSource::Between { old: revision, new: None, path: path_ok.clone(), old_path: None };
                open(source, window, cx);
                true
            })
            .footer(footer("Compare"))
    });
    focus_input(&focus_target, window, cx);
}

/// One revision of a file, for Compare with Revision.
#[derive(Clone)]
struct FileRevision {
    hash: String,
    author: String,
    time: i64,
    subject: String,
    /// The file's path in that revision (it may have been renamed since).
    path: String,
}

/// The commits that changed a file, newest first, following renames.
fn file_revisions(repository: &crate::git::Repository, path: &str) -> Vec<FileRevision> {
    let Ok(out) = repository.run(["log", "--follow", "-n", "1000", "--format=%x1e%H%x1f%an%x1f%at%x1f%s", "--name-only", "--", path]) else {
        return Vec::new();
    };
    out.split('\x1e')
        .filter_map(|record| {
            let mut lines = record.lines();
            let mut fields = lines.next()?.split('\x1f');
            let (hash, author, time, subject) = (fields.next()?, fields.next()?, fields.next()?, fields.next().unwrap_or_default());
            let at = lines.map(str::trim).find(|l| !l.is_empty()).unwrap_or(path);
            Some(FileRevision { hash: hash.to_owned(), author: author.to_owned(), time: time.parse().unwrap_or(0), subject: subject.to_owned(), path: at.to_owned() })
        })
        .collect()
}

/// Compare with Revision…: IntelliJ's list of the file's revisions (hash,
/// date, author, message), filtered as you type; the chosen one opens
/// against the local file.
pub fn compare_file_with_revision(model: Entity<RepoModel>, path: String, open_diff: OpenDiff, window: &mut Window, cx: &mut App) {
    use gpui_kit::component::h_flex;
    let Some(repository) = model.read(cx).repository().cloned() else { return };
    let revisions = Rc::new(file_revisions(&repository, &path));
    let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter by hash, author or message"));
    let focus_target = filter.clone();
    let selected = Rc::new(Cell::new(0usize));
    let last_query = Rc::new(std::cell::RefCell::new(String::new()));
    window.open_dialog(cx, move |dialog, _, cx| {
        let palette = cx.palette().clone();
        let query = filter.read(cx).value().trim().to_lowercase();
        // A new filter selects its first match.
        if *last_query.borrow() != query {
            selected.set(0);
            *last_query.borrow_mut() = query.clone();
        }
        let shown: Vec<FileRevision> = revisions
            .iter()
            .filter(|r| query.is_empty() || [&r.hash, &r.author, &r.subject].iter().any(|f| f.to_lowercase().contains(&query)))
            .cloned()
            .collect();
        let current = selected.get().min(shown.len().saturating_sub(1));
        let path = path.clone();
        let source_for = {
            let path = path.clone();
            move |r: &FileRevision| crate::ui::diff_view::DiffSource::Between {
                old: r.hash.clone(),
                new: None,
                path: path.clone(),
                old_path: (r.path != path).then(|| r.path.clone()),
            }
        };
        let mut list = v_flex().id("file-revisions").h(px(300.)).overflow_y_scroll().border_1().border_color(palette.border).rounded_md();
        for (ix, revision) in shown.iter().enumerate() {
            let (select, open, source) = (selected.clone(), open_diff.clone(), source_for(revision));
            list = list.child(
                h_flex()
                    .id(SharedString::from(format!("file-revision-{ix}")))
                    .flex_shrink_0()
                    .px_2()
                    .h(px(24.))
                    .gap_3()
                    .text_sm()
                    .when(ix == current, |el| el.bg(palette.selection))
                    .on_click(move |event, window, cx| {
                        select.set(ix);
                        if event.click_count() >= 2 {
                            window.close_dialog(cx);
                            open(source.clone(), window, cx);
                        }
                        window.refresh();
                    })
                    .child(div().w(px(70.)).flex_shrink_0().font_family("monospace").child(revision.hash[..revision.hash.len().min(8)].to_owned()))
                    .child(div().w(px(130.)).flex_shrink_0().text_color(palette.text_secondary).child(crate::ui::common::format_date(revision.time)))
                    .child(div().w(px(110.)).flex_shrink_0().truncate().text_color(palette.text_secondary).child(revision.author.clone()))
                    .child(div().flex_1().min_w_0().truncate().child(revision.subject.clone())),
            );
        }
        if shown.is_empty() {
            list = list.child(div().p_3().text_sm().text_color(palette.text_secondary).child("No revisions"));
        }
        let (select_keys, count) = (selected.clone(), shown.len());
        let chosen = shown.get(current).map(&source_for);
        let open = open_diff.clone();
        dialog
            .title(format!("Compare {path} with Revision"))
            .w(px(720.))
            .child(
                v_flex()
                    .gap_2()
                    // Up / Down move through the list while typing a filter.
                    .child(div().on_key_down(move |event, window, cx| {
                        let step: isize = match event.keystroke.key.as_str() {
                            "up" => -1,
                            "down" => 1,
                            _ => return,
                        };
                        let at = (select_keys.get().min(count.saturating_sub(1)) as isize + step).clamp(0, count.saturating_sub(1) as isize);
                        select_keys.set(at as usize);
                        cx.stop_propagation();
                        window.refresh();
                    }).child(Input::new(&filter)))
                    .child(list),
            )
            .on_ok(move |_, window, cx| {
                let Some(source) = chosen.clone() else { return false };
                open(source, window, cx);
                true
            })
            .footer(footer("Compare"))
    });
    focus_input(&focus_target, window, cx);
}
