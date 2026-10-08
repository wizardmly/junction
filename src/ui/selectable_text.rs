//! Plain text that takes part in the window's text selection, with optional
//! clickable link ranges. Unlike gpui-base's `SelectableText`, links work, and
//! unlike `TextView` the reading order (`document_order`) is ours, so a drag
//! across several of these copies them top to bottom, as in IntelliJ's
//! read-only text panes.

use std::ops::Range;
use std::rc::Rc;

use gpui_kit::base::{TextSelection, TextSelectionHandle, TextSelectionRegistration, TextSelectionRun};
use gpui_kit::{
    App, BorderStyle, Bounds, Corners, CursorStyle, Edges, Element, ElementId, GlobalElementId, HighlightStyle, Hitbox,
    HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseUpEvent, PaintQuad, Pixels, Point,
    SharedString, StyledText, TextStyleRefinement, Window, transparent_black,
};

type LinkHandler = Rc<dyn Fn(usize, &mut Window, &mut App)>;

pub struct SelectableText {
    id: ElementId,
    text: SharedString,
    styled_text: StyledText,
    document_order: u64,
    style: TextStyleRefinement,
    links: Vec<Range<usize>>,
    on_link: Option<LinkHandler>,
}

pub fn selectable_text(id: impl Into<ElementId>, text: impl Into<SharedString>) -> SelectableText {
    let text = text.into();
    SelectableText {
        id: id.into(),
        styled_text: StyledText::new(text.clone()),
        text,
        document_order: 0,
        style: TextStyleRefinement::default(),
        links: Vec::new(),
        on_link: None,
    }
}

impl SelectableText {
    /// Reading order among the texts a drag can span; copy joins them with
    /// newlines in this order.
    pub fn order(mut self, order: u64) -> Self {
        self.document_order = order;
        self
    }

    pub fn color(mut self, color: Hsla) -> Self {
        self.style.color = Some(color);
        self
    }

    pub fn font_family(mut self, family: Option<SharedString>) -> Self {
        self.style.font_family = family;
        self
    }

    /// Paints `links` in `color`; a click (not a drag) on one calls
    /// `on_click` with its index.
    pub fn links(
        mut self,
        links: Vec<Range<usize>>,
        color: Hsla,
        on_click: impl Fn(usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        let highlight = HighlightStyle { color: Some(color), ..Default::default() };
        self.styled_text =
            StyledText::new(self.text.clone()).with_highlights(links.iter().map(|range| (range.clone(), highlight)));
        self.links = links;
        self.on_link = Some(Rc::new(on_click));
        self
    }

    fn link_at(links: &[Range<usize>], layout: &gpui_kit::TextLayout, position: Point<Pixels>) -> Option<usize> {
        let index = layout.index_for_position(position).ok()?;
        links.iter().position(|range| range.contains(&index))
    }
}

/// The selection highlight of `range`, one quad per visual line.
fn selection_quads(layout: &gpui_kit::TextLayout, range: Range<usize>) -> Vec<Bounds<Pixels>> {
    let (Some(start), Some(end)) = (layout.position_for_index(range.start), layout.position_for_index(range.end)) else {
        return Vec::new();
    };
    let (bounds, line_height) = (layout.bounds(), layout.line_height());
    if start.y == end.y {
        return vec![Bounds::from_corners(start, Point::new(end.x, end.y + line_height))];
    }
    let mut quads = vec![Bounds::from_corners(start, Point::new(bounds.right(), start.y + line_height))];
    if end.y > start.y + line_height {
        quads.push(Bounds::from_corners(
            Point::new(bounds.left(), start.y + line_height),
            Point::new(bounds.right(), end.y),
        ));
    }
    quads.push(Bounds::from_corners(Point::new(bounds.left(), end.y), Point::new(end.x, end.y + line_height)));
    quads
}

impl IntoElement for SelectableText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SelectableText {
    type RequestLayoutState = TextSelectionHandle;
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let handle = window.with_element_state(
            global_id.expect("SelectableText has an id"),
            |retained: Option<TextSelectionHandle>, _| {
                let handle = retained.unwrap_or_else(|| TextSelectionHandle::new(self.text.to_string(), cx));
                (handle.clone(), handle)
            },
        );
        let (layout_id, ()) = window.with_text_style(Some(self.style.clone()), |window| {
            self.styled_text.request_layout(global_id, inspector_id, window, cx)
        });
        (layout_id, handle)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        handle: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        window.with_text_style(Some(self.style.clone()), |window| {
            self.styled_text.prepaint(global_id, inspector_id, bounds, &mut (), window, cx)
        });
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let registration = TextSelectionRegistration::new(hitbox.clone(), bounds)
            .with_document_order(self.document_order)
            .with_text_bounds(vec![bounds])
            .with_rendered_element(handle, window, cx);
        handle.register(registration, window, cx);
        hitbox
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        handle: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let layout = self.styled_text.layout().clone();
        let selected_before = TextSelection::selected_text(window, cx);
        let projection = handle.update_runs(
            &[TextSelectionRun::new(self.text.clone(), layout.clone(), bounds).with_document_order(self.document_order)],
            cx,
        );
        if selected_before != TextSelection::selected_text(window, cx) {
            window.refresh();
        }
        let color = gpui_kit::base::Theme::global(cx).tokens.colors.selection;
        for range in projection.ranges().iter().flatten().cloned() {
            for quad in selection_quads(&layout, range) {
                window.paint_quad(PaintQuad {
                    bounds: quad,
                    background: color.into(),
                    corner_radii: Corners::default(),
                    border_widths: Edges::default(),
                    border_color: transparent_black(),
                    border_style: BorderStyle::default(),
                });
            }
        }
        window.with_text_style(Some(self.style.clone()), |window| {
            self.styled_text.paint(global_id, inspector_id, bounds, &mut (), &mut (), window, cx)
        });

        if let Some(on_link) = self.on_link.clone() {
            let links = std::mem::take(&mut self.links);
            if hitbox.is_hovered(window) && Self::link_at(&links, &layout, window.mouse_position()).is_some() {
                window.set_cursor_style(CursorStyle::PointingHand, hitbox);
            }
            let hitbox = hitbox.clone();
            window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                if !phase.bubble() || event.button != MouseButton::Left || !hitbox.is_hovered(window) {
                    return;
                }
                // A drag or double-click selects text instead of following the link.
                if TextSelection::has_selection(window, cx) {
                    return;
                }
                if let Some(ix) = Self::link_at(&links, &layout, event.position) {
                    on_link(ix, window, cx);
                }
            });
        }
    }
}
