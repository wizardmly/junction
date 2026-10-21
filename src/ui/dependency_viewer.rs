//! Analyze Dependencies…: IntelliJ's Dependency Viewer for a scope. The
//! scope's Java and Kotlin files on the left, what the selected one uses
//! on the right: project files (a click opens one) and library packages.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::{Icon, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{App, InteractiveElement as _, IntoElement as _, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px};

use crate::index::dependencies::FileDeps;
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::common::file_icon;

pub type OpenFile = Rc<dyn Fn(String, &mut Window, &mut App)>;

fn split(path: &str) -> (String, String) {
    match path.rsplit_once('/') {
        Some((dir, name)) => (name.to_owned(), dir.to_owned()),
        None => (path.to_owned(), String::new()),
    }
}

pub fn open(scope: String, deps: Vec<FileDeps>, open_file: OpenFile, window: &mut Window, cx: &mut App) {
    let selected = Rc::new(Cell::new(0usize));
    let deps = Rc::new(deps);
    window.open_dialog(cx, move |dialog, _, cx| {
        let palette = cx.palette().clone();
        let current = selected.get().min(deps.len().saturating_sub(1));
        let row = |id: SharedString, icon: Icon, name: String, detail: String, active: bool| {
            h_flex()
                .id(id)
                .px_2()
                .h(px(24.))
                .gap_1p5()
                .flex_none()
                .cursor_pointer()
                .when(active, |el| el.bg(palette.selection))
                .when(!active, |el| el.hover(|el| el.bg(palette.hover)))
                .child(icon.xsmall())
                .child(div().flex_shrink(1.).min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(name))
                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(palette.text_secondary).child(detail))
        };
        let mut files = v_flex().id("deps-files").size_full().overflow_y_scroll().text_sm();
        for (ix, d) in deps.iter().enumerate() {
            let (name, dir) = split(&d.file);
            let count = d.project.len() + d.libraries.len();
            let select = selected.clone();
            files = files.child(row(format!("deps-file-{ix}").into(), file_icon(&d.file), name, format!("{dir}  {count}"), ix == current).on_click(move |_, window, _| {
                select.set(ix);
                window.refresh();
            }));
        }
        let mut used = v_flex().id("deps-used").size_full().overflow_y_scroll().text_sm();
        if let Some(d) = deps.get(current) {
            let heading = |label: String| div().px_2().pt_1().pb_0p5().text_xs().font_weight(gpui_kit::FontWeight::SEMIBOLD).text_color(palette.text_secondary).child(label);
            used = used.child(heading(format!("Project files ({})", d.project.len())));
            for (ix, path) in d.project.iter().enumerate() {
                let (name, dir) = split(path);
                let (open_file, path) = (open_file.clone(), path.clone());
                used = used.child(row(format!("deps-project-{ix}").into(), file_icon(&path), name, dir, false).on_click(move |_, window, cx| {
                    window.close_dialog(cx);
                    open_file(path.clone(), window, cx);
                }));
            }
            used = used.child(heading(format!("Libraries ({})", d.libraries.len())));
            for (ix, package) in d.libraries.iter().enumerate() {
                used = used.child(row(format!("deps-library-{ix}").into(), Icon::new(icons::PACKAGE), package.clone(), String::new(), false));
            }
        }
        let pane = || v_flex().h_full().min_w_0().border_1().border_color(palette.border).rounded(px(4.));
        let body = if deps.is_empty() {
            div().p_3().text_sm().text_color(palette.text_secondary).child("No Java or Kotlin files in the scope").into_any_element()
        } else {
            h_flex()
                .h(px(460.))
                .gap_2()
                .child(pane().w(px(400.)).flex_none().child(files))
                .child(pane().flex_1().child(used))
                .into_any_element()
        };
        dialog.title(format!("Dependencies of {scope}")).w(px(960.)).child(body)
    });
}
