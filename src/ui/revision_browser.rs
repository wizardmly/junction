//! Show Repository at Revision: the files of a commit's tree; picking one
//! opens it read-only in the file editor.

use std::rc::Rc;

use gpui_kit::component::{
    Icon, Sizable as _, WindowExt as _, h_flex,
    list::ListItem,
    tree::{TreeState, tree},
    v_flex,
};
use gpui_kit::assets::IconName;
use gpui_kit::{App, AppContext as _, Entity, ParentElement as _, SharedString, Styled as _, Window, div, px};

use crate::model::RepoModel;
use crate::theme::ActivePalette as _;
use crate::ui::common::{self, FILE_PREFIX, row_height};

pub type OpenFile = Rc<dyn Fn(String, String, &mut Window, &mut App)>;

pub fn open(model: Entity<RepoModel>, revision: String, open_file: OpenFile, window: &mut Window, cx: &mut App) {
    let Some(repository) = model.read(cx).repository().cloned() else { return };
    let files: Vec<String> = repository
        .run(["ls-tree", "-r", "--name-only", "-z", &revision])
        .map(|out| out.split('\0').filter(|p| !p.is_empty()).map(str::to_owned).collect())
        .unwrap_or_default();
    let count = files.len();
    let state = cx.new(|cx| TreeState::new(cx));
    state.update(cx, |tree, cx| tree.set_items(common::file_tree_with(files, "", false), cx));
    let short = revision[..revision.len().min(8)].to_owned();
    window.open_dialog(cx, move |dialog, _, cx| {
        let palette = cx.palette().clone();
        let open_file = open_file.clone();
        let revision = revision.clone();
        let rows = tree(&state, move |ix, entry, _, _, _| {
            let item = entry.item();
            let path = item.id.strip_prefix(FILE_PREFIX).map(str::to_owned);
            let (open_file, revision) = (open_file.clone(), revision.clone());
            let row = h_flex()
                .gap_1()
                .pl(px(entry.depth() as f32 * 14.))
                .text_sm()
                .child(if entry.is_folder() {
                    Icon::new(if entry.is_expanded() { IconName::ChevronDown } else { IconName::ChevronRight }).xsmall()
                } else {
                    Icon::new(IconName::Circle).xsmall().text_color(gpui_kit::transparent_black())
                })
                .child(Icon::new(path.as_deref().map_or(IconName::Folder, common::file_icon)).small())
                .child(div().child(item.label.clone()));
            ListItem::new(ix).py_0().px_1().h(px(row_height())).child(row).on_click(move |event, window, cx| {
                // Double-click (or Enter-like single click on a file) opens it, as IntelliJ's F4.
                if let (Some(path), true) = (path.clone(), event.click_count() >= 2) {
                    window.close_dialog(cx);
                    open_file(revision.clone(), path, window, cx);
                }
            })
        });
        dialog
            .title(SharedString::from(format!("Repository at {short}")))
            .w(px(640.))
            .child(
                v_flex()
                    .gap_2()
                    .child(div().text_xs().text_color(palette.text_secondary).child(format!(
                        "{count} files. Double-click a file to open it read-only."
                    )))
                    .child(div().h(px(460.)).border_1().border_color(palette.border).rounded(px(4.)).child(rows.size_full())),
            )
    });
}
