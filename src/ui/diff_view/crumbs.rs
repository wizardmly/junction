//! The diff's breadcrumbs (gutter menu › Breadcrumbs): for each pane, the
//! declarations around its caret line (XML tags in markup), top or bottom;
//! a click puts the caret on that declaration.

use std::rc::Rc;

use gpui_kit::{
    AnyElement, Context, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, StatefulInteractiveElement as _, Styled as _, div,
    prelude::FluentBuilder as _, px,
};

use super::{DiffView, ViewerMode};
use crate::index::lang::Lang;
use crate::index::symbols::{self, SymbolKind};
use crate::theme::Palette;
use crate::ui::text_buffer::Selection;

/// A declaration a line can be inside: its label and line span (0-based).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Scope {
    pub label: String,
    pub first: usize,
    pub last: usize,
}

/// The scopes of a text, outermost first among those that nest.
pub(super) fn scopes(path: &str, text: &str) -> Vec<Scope> {
    let lower = path.to_ascii_lowercase();
    if [".xml", ".html", ".htm", ".xhtml", ".svg", ".plist", ".iml"].iter().any(|ext| lower.ends_with(ext)) {
        return xml_scopes(text);
    }
    let Some(lang) = Lang::from_path(path) else { return Vec::new() };
    let mut scopes: Vec<Scope> = symbols::extract(lang, text)
        .into_iter()
        .filter(|s| s.kind.is_type() || s.kind.is_callable() || matches!(s.kind, SymbolKind::Module | SymbolKind::Namespace | SymbolKind::Extension))
        .map(|s| Scope {
            label: if s.kind.is_callable() { format!("{}()", s.name) } else { s.name },
            first: s.line as usize,
            last: (s.end_line as usize).max(s.line as usize),
        })
        .collect();
    scopes.sort_by(|a, b| a.first.cmp(&b.first).then(b.last.cmp(&a.last)));
    scopes
}

/// The scopes a line is in, outermost first.
pub(super) fn crumbs_at(scopes: &[Scope], line: usize) -> Vec<&Scope> {
    scopes.iter().filter(|s| s.first <= line && line <= s.last).collect()
}

/// Elements of a markup text with their line spans (unclosed ones run to
/// the end); comments, declarations and processing instructions skipped.
fn xml_scopes(text: &str) -> Vec<Scope> {
    let mut out: Vec<Scope> = Vec::new();
    let mut open: Vec<usize> = Vec::new();
    let bytes = text.as_bytes();
    let mut line = 0;
    let mut i = 0;
    let line_of = |from: usize, to: usize, line: usize| line + bytes[from..to].iter().filter(|b| **b == b'\n').count();
    let mut last = 0;
    while let Some(at) = text[i..].find('<').map(|k| i + k) {
        line = line_of(last, at, line);
        last = at;
        let rest = &text[at..];
        let skip_to = |end: &str| rest.find(end).map_or(text.len(), |k| at + k + end.len());
        if rest.starts_with("<!--") {
            i = skip_to("-->");
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            i = skip_to("]]>");
            continue;
        }
        if rest.starts_with("<?") || rest.starts_with("<!") {
            i = skip_to(">");
            continue;
        }
        let end = skip_to(">");
        let tag = &text[at..end];
        let closing = tag.starts_with("</");
        let name: String = tag.trim_start_matches(['<', '/']).chars().take_while(|c| !c.is_whitespace() && *c != '>' && *c != '/').collect();
        let end_line = line_of(at, end.min(text.len()), line);
        if closing {
            if let Some(k) = open.iter().rposition(|&o| out[o].label == name) {
                for o in open.drain(k..) {
                    out[o].last = line;
                }
            }
        } else if !name.is_empty() {
            out.push(Scope { label: name, first: line, last: end_line });
            if !tag.ends_with("/>") {
                open.push(out.len() - 1);
            }
        }
        i = end;
    }
    let lines = text.lines().count().saturating_sub(1);
    for o in open {
        out[o].last = lines;
    }
    out
}

impl DiffView {
    /// A pane's scopes, parsed once per text.
    fn pane_scopes(&self, pane: usize) -> Rc<Vec<Scope>> {
        if let Some(scopes) = self.scopes.borrow()[pane].clone() {
            return scopes;
        }
        let path = self.source.as_ref().map(|s| s.path().to_owned()).unwrap_or_default();
        let scopes = Rc::new(scopes(&path, self.panes.buffers[pane].text()));
        self.scopes.borrow_mut()[pane] = Some(scopes.clone());
        scopes
    }

    /// The texts changed: parse again when next shown.
    pub(super) fn forget_scopes(&self) {
        *self.scopes.borrow_mut() = Default::default();
    }

    /// The line a pane's breadcrumbs follow: the caret's, or the line
    /// matching it on the other side.
    fn crumb_line(&self, pane: usize) -> Option<usize> {
        let (caret_pane, sel) = self.panes.caret?;
        let line = self.panes.buffers[caret_pane].line_of(sel.head);
        Some(if caret_pane == pane { line } else { self.transfer_line(line, caret_pane == 0) })
    }

    /// One pane's breadcrumbs bar content.
    fn crumbs(&self, pane: usize, palette: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let scopes = self.pane_scopes(pane);
        let line = self.crumb_line(pane);
        let chain: Vec<Scope> = line.map(|l| crumbs_at(&scopes, l).into_iter().cloned().collect()).unwrap_or_default();
        let mut row = gpui_kit::component::h_flex().min_w_0().overflow_hidden().whitespace_nowrap().px(px(6.)).gap_1();
        let count = chain.len();
        for (ix, scope) in chain.into_iter().enumerate() {
            let first = scope.first;
            row = row
                .child(
                    div()
                        .id(ElementId::NamedInteger(format!("diff-crumb-{pane}").into(), ix as u64))
                        .px_1()
                        .rounded(px(3.))
                        .cursor_pointer()
                        .text_color(if ix + 1 == count { palette.text } else { palette.text_secondary })
                        .hover(|st| st.bg(palette.hover))
                        .child(scope.label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let buffer = &this.panes.buffers[pane];
                            let offset = buffer.line_range(first.min(buffer.line_count().saturating_sub(1))).start;
                            this.panes.caret = Some((pane, Selection::caret(offset)));
                            let row = this.panes.row_of(pane, first);
                            this.panes.show_rows(vec![(pane, row)]);
                            cx.notify();
                        })),
                )
                .when(ix + 1 < count, |el| el.child(div().text_color(palette.text_disabled).child("›")));
        }
        row.into_any_element()
    }

    /// The breadcrumbs bar: one cell per pane, sized as the panes are.
    pub(super) fn render_breadcrumbs(&self, top: bool, palette: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let cell = |el: gpui_kit::Div, weight: f32| {
            el.flex_basis(px(0.)).min_w_0().map(|mut el| {
                el.style().flex_grow = Some(weight);
                el
            })
        };
        let bar = gpui_kit::component::h_flex()
            .h(px(24.))
            .flex_shrink_0()
            .text_xs()
            .bg(palette.panel)
            .border_color(palette.border)
            .map(|el| if top { el.border_b_1() } else { el.border_t_1() })
            .child(div().w(px(crate::ui::text_panes::STRIPE_WIDTH)).flex_shrink_0());
        let bar = if self.mode == ViewerMode::Unified {
            bar.child(cell(div(), 1.).child(self.crumbs(1, palette, cx)))
        } else {
            bar.child(cell(div(), self.panes.weight(0)).child(self.crumbs(0, palette, cx)))
                .child(div().w(px(crate::ui::diff_panes::DIVIDER_WIDTH)).flex_shrink_0())
                .child(cell(div(), self.panes.weight(1)).child(self.crumbs(1, palette, cx)))
        };
        bar.child(div().w(px(crate::ui::text_panes::STRIPE_WIDTH)).flex_shrink_0()).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_tags_nest_by_line() {
        let text = "<?xml version=\"1.0\"?>\n<project version=\"4\">\n  <component name=\"A\">\n    <option name=\"x\" />\n  </component>\n</project>\n";
        let scopes = scopes("gradle.xml", text);
        let labels: Vec<&str> = crumbs_at(&scopes, 3).iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["project", "component", "option"]);
        let labels: Vec<&str> = crumbs_at(&scopes, 4).iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["project", "component"]);
    }

    #[test]
    fn code_scopes_are_declarations() {
        let text = "class A {\n  fun b() {\n    val c = 1\n  }\n}\n";
        let scopes = scopes("A.kt", text);
        let labels: Vec<&str> = crumbs_at(&scopes, 2).iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["A", "b()"]);
    }
}
