//! Paints one row of the commit graph.

use std::rc::Rc;

use gpui_kit::{
    App, Bounds, Hsla, InteractiveElement as _, ParentElement as _, PathBuilder, Pixels, StatefulInteractiveElement as _, Styled as _, Window,
    canvas, div, fill, point, px, size,
};

use crate::git::graph::{GraphRow, Half};
use crate::theme::Palette;

pub const LANE_WIDTH: f32 = 14.;
const NODE_RADIUS: f32 = 3.5;
const LINE_WIDTH: f32 = 1.5;

/// Width the graph needs in a row, so the subject can start right after it.
pub fn graph_width(row: &GraphRow) -> Pixels {
    px(LANE_WIDTH * row.width.max(1) as f32 + 6.)
}

/// Goes to a row: the other end of a hidden long edge whose arrow was clicked.
pub type OnArrow = Rc<dyn Fn(usize, &mut Window, &mut App)>;

pub fn graph_canvas(row: GraphRow, palette: &Palette, head: bool, row_ix: usize, on_arrow: &OnArrow) -> impl gpui_kit::IntoElement {
    let colors: Vec<Hsla> = (0..palette.graph.len()).map(|ix| palette.graph_color(ix)).collect();
    let width = graph_width(&row);
    let arrows: Vec<_> = row.arrows.iter().map(|arrow| {
        let (target, on_arrow) = (arrow.target, on_arrow.clone());
        div()
            .id(("graph-arrow", row_ix * 64 + arrow.lane.min(63)))
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(LANE_WIDTH * arrow.lane as f32 + 3.))
            .w(px(LANE_WIDTH))
            .cursor_pointer()
            .on_mouse_down(gpui_kit::MouseButton::Left, move |_, window, cx| {
                cx.stop_propagation();
                on_arrow(target, window, cx);
            })
    }).collect();
    div().relative().w(width).h_full().flex_shrink_0().child(
        canvas(
            move |_, _, _| {},
            move |bounds, _, window, _| paint_row(&row, &colors, head, bounds, window),
        )
        .size_full(),
    )
    .children(arrows)
}

fn paint_row(row: &GraphRow, colors: &[Hsla], head: bool, bounds: Bounds<Pixels>, window: &mut Window) {
    let color = |ix: usize| colors[ix % colors.len()];
    let x = |lane: usize| bounds.origin.x + px(LANE_WIDTH * lane as f32 + LANE_WIDTH / 2. + 3.);
    let top = bounds.origin.y;
    let middle = bounds.origin.y + bounds.size.height / 2.;
    let bottom = bounds.origin.y + bounds.size.height;

    for line in &row.lines {
        let (y0, y1) = match line.half {
            Half::Top => (top, middle),
            Half::Bottom => (middle, bottom),
        };
        let mut path = PathBuilder::stroke(px(LINE_WIDTH));
        path.move_to(point(x(line.from_lane), y0));
        if line.from_lane == line.to_lane {
            path.line_to(point(x(line.to_lane), y1));
        } else {
            // Gentle S-curve between lanes, like IntelliJ's angled edges.
            let mid_y = (y0 + y1) / 2.;
            path.cubic_bezier_to(
                point(x(line.to_lane), y1),
                point(x(line.from_lane), mid_y),
                point(x(line.to_lane), mid_y),
            );
        }
        if let Ok(path) = path.build() {
            window.paint_path(path, color(line.color));
        }
    }

    // A hidden long edge ends in a small arrowhead at the row's middle.
    for arrow in &row.arrows {
        let (tip, back) = if arrow.down { (middle + px(2.), middle - px(2.)) } else { (middle - px(2.), middle + px(2.)) };
        let mut path = PathBuilder::stroke(px(LINE_WIDTH));
        path.move_to(point(x(arrow.lane) - px(3.5), back));
        path.line_to(point(x(arrow.lane), tip));
        path.line_to(point(x(arrow.lane) + px(3.5), back));
        if let Ok(path) = path.build() {
            window.paint_path(path, color(arrow.color));
        }
    }

    let radius = px(if head { NODE_RADIUS + 1. } else { NODE_RADIUS });
    let center = point(x(row.node_lane), middle);
    let node = Bounds::new(point(center.x - radius, center.y - radius), size(radius * 2., radius * 2.));
    window.paint_quad(fill(node, color(row.node_color)).corner_radii(radius));
}
