//! Drawable XML as Android Studio previews it, beyond vectors: a
//! `<shape>` (rectangle with its corners, oval, line, ring; solid fill or
//! gradient; stroke with dashes; `<size>`), a `<selector>` in its default
//! state (and each item's state for the state list), `<layer-list>` and
//! `<ripple>` layers with their insets, `<inset>`, `<color>`; at level 0
//! and not running, as Android first draws them: an `<animated-vector>`'s
//! vector, an `<animated-selector>` as a selector, an `<animation-list>`'s
//! first frame, a `<rotate>` at its `fromDegrees`, and nothing for
//! `<clip>` and `<scale>`. Drawables they reference are loaded and drawn
//! in place, vectors through `vector_drawable`. Everything as SVG in dp.

use std::fmt::Write as _;

use roxmltree::Node;

use crate::domain::vector_drawable::{self, VectorImage};

const ANDROID: &str = "http://schemas.android.com/apk/res/android";

/// A shape without `<size>` (and nothing else sized) is drawn this big.
pub const DEFAULT_SIZE: f32 = 100.;

/// The project's resources, as the preview needs them: references
/// (`@color/accent`, `@dimen/radius`, `@drawable/bg`) to a color, a value,
/// a drawable's XML.
pub struct Lookup<'a> {
    pub color: &'a dyn Fn(&str) -> Option<u32>,
    pub value: &'a dyn Fn(&str) -> Option<String>,
    pub drawable: &'a dyn Fn(&str) -> Option<String>,
}

/// The roots this module draws.
const ROOTS: &[&str] = &[
    "vector",
    "shape",
    "selector",
    "layer-list",
    "ripple",
    "inset",
    "color",
    "level-list",
    "transition",
    "animated-vector",
    "animated-selector",
    "animation-list",
    "rotate",
    "scale",
    "clip",
];

/// Whether an XML file is a drawable the preview can draw.
pub fn is_previewable(text: &str) -> bool {
    roxmltree::Document::parse(text).is_ok_and(|d| ROOTS.contains(&d.root_element().tag_name().name()))
}

/// The drawable as SVG, `scale` times its dp size.
pub fn render(text: &str, scale: f32, lookup: &Lookup) -> Result<VectorImage, String> {
    let doc = roxmltree::Document::parse(text).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    let name = root.tag_name().name();
    if !ROOTS.contains(&name) {
        return Err(format!("<{name}> drawables are not previewed"));
    }
    let mut w = Writer { lookup, ids: 0, depth: 0 };
    let (width, height) = w.intrinsic(root, text).unwrap_or((DEFAULT_SIZE, DEFAULT_SIZE));
    let mut body = String::new();
    w.draw(root, text, (0., 0., width, height), &mut body)?;
    Ok(VectorImage { svg: wrap(&body, width, height, scale), width, height })
}

/// A selector's items as its state list shows them: each item's states
/// ("pressed", "not enabled", "default") and its drawable, `size` dp.
pub fn states(text: &str, size: f32, lookup: &Lookup) -> Vec<(String, String)> {
    let Ok(doc) = roxmltree::Document::parse(text) else { return Vec::new() };
    let root = doc.root_element();
    if !matches!(root.tag_name().name(), "selector" | "animated-selector") {
        return Vec::new();
    }
    let mut out = Vec::new();
    for item in root.children().filter(|n| n.has_tag_name("item")) {
        let mut w = Writer { lookup, ids: 0, depth: 0 };
        let (iw, ih) = w.item_intrinsic(item, text).unwrap_or((DEFAULT_SIZE, DEFAULT_SIZE));
        let scale = size / iw.max(ih).max(1.);
        let mut body = String::new();
        if w.draw_item(item, text, (0., 0., iw, ih), &mut body).is_ok() {
            out.push((state_label(item), wrap(&body, iw, ih, scale)));
        }
    }
    out
}

fn wrap(body: &str, width: f32, height: f32, scale: f32) -> String {
    format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {width} {height}\">{body}</svg>", width * scale, height * scale)
}

/// "pressed, not enabled", or "default" for an item without states.
fn state_label(item: Node) -> String {
    let states: Vec<String> = item
        .attributes()
        .filter_map(|a| {
            let state = a.name().strip_prefix("state_")?;
            Some(if a.value() == "true" { state.replace('_', " ") } else { format!("not {}", state.replace('_', " ")) })
        })
        .collect();
    if states.is_empty() { "default".into() } else { states.join(", ") }
}

/// Whether a selector item matches a View's default state: enabled and
/// window-focused, nothing else.
fn matches_default(item: Node) -> bool {
    item.attributes().all(|a| match a.name().strip_prefix("state_") {
        Some("enabled" | "window_focused") => a.value() == "true",
        Some(_) => a.value() != "true",
        None => true,
    })
}

type Rect = (f32, f32, f32, f32);

struct Writer<'a, 'b> {
    lookup: &'a Lookup<'b>,
    ids: usize,
    /// Nested references, against loops.
    depth: usize,
}

fn attr<'n>(node: Node<'n, 'n>, name: &str) -> Option<&'n str> {
    node.attribute((ANDROID, name))
}

impl Writer<'_, '_> {
    fn dimen(&self, value: Option<&str>) -> Option<f32> {
        let value = value?.trim();
        let value = if value.starts_with('@') { (self.lookup.value)(value)? } else { value.to_owned() };
        let number = value.trim_end_matches(|c: char| c.is_ascii_alphabetic());
        number.parse::<f32>().ok()
    }

    fn color(&self, value: Option<&str>) -> Option<u32> {
        vector_drawable::resolve(value?, self.lookup.color)
    }

    /// A referenced drawable's XML, or a color, drawn in `rect`.
    fn draw_reference(&mut self, reference: &str, rect: Rect, out: &mut String) -> Result<(), String> {
        if let Some(rgba) = self.color(Some(reference)).filter(|_| !reference.starts_with("@drawable") && !reference.starts_with("@mipmap")) {
            fill_rect(rect, rgba, out);
            return Ok(());
        }
        if self.depth > 8 {
            return Err("Drawables reference each other in a loop".into());
        }
        let Some(text) = (self.lookup.drawable)(reference) else { return Ok(()) };
        let doc = roxmltree::Document::parse(&text).map_err(|e| e.to_string())?;
        self.depth += 1;
        let result = self.draw(doc.root_element(), &text, rect, out);
        self.depth -= 1;
        result
    }

    fn reference_intrinsic(&mut self, reference: &str) -> Option<(f32, f32)> {
        if self.depth > 8 {
            return None;
        }
        let text = (self.lookup.drawable)(reference)?;
        let doc = roxmltree::Document::parse(&text).ok()?;
        self.depth += 1;
        let size = self.intrinsic(doc.root_element(), &text);
        self.depth -= 1;
        size
    }

    /// A drawable's own size in dp, if it has one.
    fn intrinsic(&mut self, node: Node, text: &str) -> Option<(f32, f32)> {
        match node.tag_name().name() {
            "vector" => {
                let image = vector_drawable::to_svg(&standalone(node, text), 1., self.lookup.color).ok()?;
                Some((image.width, image.height))
            }
            "shape" => {
                let size = node.children().find(|n| n.has_tag_name("size"))?;
                let (w, h) = (self.dimen(attr(size, "width")), self.dimen(attr(size, "height")));
                Some((w.or(h)?, h.or(w)?))
            }
            "selector" | "level-list" | "animated-selector" | "animation-list" => {
                let item = node.children().filter(|n| n.has_tag_name("item")).find(|n| matches_default(*n)).or_else(|| node.children().find(|n| n.has_tag_name("item")))?;
                self.item_intrinsic(item, text)
            }
            "layer-list" | "ripple" | "transition" => {
                let mut size: Option<(f32, f32)> = None;
                for item in layers(node) {
                    let inset = |name| self.dimen(attr(item, name)).unwrap_or(0.);
                    let (l, t, r, b) = (inset("left"), inset("top"), inset("right"), inset("bottom"));
                    let own = match (self.dimen(attr(item, "width")), self.dimen(attr(item, "height"))) {
                        (Some(w), Some(h)) => Some((w, h)),
                        _ => self.item_intrinsic(item, text),
                    };
                    if let Some((w, h)) = own {
                        let (w, h) = (w + l + r, h + t + b);
                        size = Some(size.map_or((w, h), |(sw, sh)| (sw.max(w), sh.max(h))));
                    }
                }
                size
            }
            "animated-vector" => match attr(node, "drawable") {
                Some(r) => self.reference_intrinsic(r),
                None => self.intrinsic(inline_drawable(node)?, text),
            },
            "rotate" | "scale" | "clip" => self.item_intrinsic(node, text),
            "inset" => {
                let (w, h) = match attr(node, "drawable") {
                    Some(r) => self.reference_intrinsic(r)?,
                    None => {
                        let child = node.children().find(|n| n.is_element())?;
                        self.intrinsic(child, text)?
                    }
                };
                let all = self.dimen(attr(node, "inset")).unwrap_or(0.);
                let side = |name| self.dimen(attr(node, name)).unwrap_or(all);
                Some((w + side("insetLeft") + side("insetRight"), h + side("insetTop") + side("insetBottom")))
            }
            _ => None,
        }
    }

    fn item_intrinsic(&mut self, item: Node, text: &str) -> Option<(f32, f32)> {
        match attr(item, "drawable") {
            Some(r) => self.reference_intrinsic(r),
            None => {
                let child = item.children().find(|n| n.is_element())?;
                self.intrinsic(child, text)
            }
        }
    }

    /// An item's drawable (its `android:drawable` or its child) in `rect`.
    fn draw_item(&mut self, item: Node, text: &str, rect: Rect, out: &mut String) -> Result<(), String> {
        match attr(item, "drawable") {
            Some(r) => self.draw_reference(r, rect, out),
            None => match item.children().find(|n| n.is_element()) {
                Some(child) => self.draw(child, text, rect, out),
                None => Ok(()),
            },
        }
    }

    fn draw(&mut self, node: Node, text: &str, rect: Rect, out: &mut String) -> Result<(), String> {
        match node.tag_name().name() {
            "vector" => {
                let image = vector_drawable::to_svg(&standalone(node, text), 1., self.lookup.color)?;
                self.ids += 1;
                // Its own viewBox, placed in the rect; its ids made unique.
                let svg = image.svg.replace("id=\"", &format!("id=\"v{}", self.ids)).replace("url(#", &format!("url(#v{}", self.ids));
                let at = svg.find("viewBox=").ok_or("Bad vector")?;
                let (x, y, w, h) = rect;
                let _ = write!(out, "<svg x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" {}", &svg[at..]);
                Ok(())
            }
            "shape" => {
                self.shape(node, rect, out);
                Ok(())
            }
            "color" => {
                if let Some(rgba) = self.color(attr(node, "color")) {
                    fill_rect(rect, rgba, out);
                }
                Ok(())
            }
            "selector" | "level-list" | "animated-selector" | "animation-list" => {
                let items: Vec<Node> = node.children().filter(|n| n.has_tag_name("item")).collect();
                match items.iter().find(|n| matches_default(**n)).or(items.first()) {
                    Some(item) => self.draw_item(*item, text, rect, out),
                    None => Ok(()),
                }
            }
            "layer-list" | "ripple" | "transition" => {
                for item in layers(node) {
                    let inset = |name| self.dimen(attr(item, name)).unwrap_or(0.);
                    let (l, t, r, b) = (inset("left"), inset("top"), inset("right"), inset("bottom"));
                    let (x, y, w, h) = (rect.0 + l, rect.1 + t, (rect.2 - l - r).max(0.), (rect.3 - t - b).max(0.));
                    // An item with a size sits by its gravity, else centered.
                    let gravity = attr(item, "gravity").unwrap_or("center");
                    let (iw, ih) = (self.dimen(attr(item, "width")).unwrap_or(w).min(w), self.dimen(attr(item, "height")).unwrap_or(h).min(h));
                    let fill = gravity.contains("fill");
                    let ix = if fill || iw == w {
                        x
                    } else if gravity.contains("left") || gravity.contains("start") {
                        x
                    } else if gravity.contains("right") || gravity.contains("end") {
                        x + w - iw
                    } else {
                        x + (w - iw) / 2.
                    };
                    let iy = if fill || ih == h {
                        y
                    } else if gravity.contains("top") {
                        y
                    } else if gravity.contains("bottom") {
                        y + h - ih
                    } else {
                        y + (h - ih) / 2.
                    };
                    let (iw, ih) = if fill { (w, h) } else { (iw, ih) };
                    self.draw_item(item, text, (ix, iy, iw, ih), out)?;
                }
                Ok(())
            }
            "inset" => {
                let all = self.dimen(attr(node, "inset")).unwrap_or(0.);
                let side = |name| self.dimen(attr(node, name)).unwrap_or(all);
                let (l, t, r, b) = (side("insetLeft"), side("insetTop"), side("insetRight"), side("insetBottom"));
                let inner = (rect.0 + l, rect.1 + t, (rect.2 - l - r).max(0.), (rect.3 - t - b).max(0.));
                match attr(node, "drawable") {
                    Some(r) => self.draw_reference(r, inner, out),
                    None => match node.children().find(|n| n.is_element()) {
                        Some(child) => self.draw(child, text, inner, out),
                        None => Ok(()),
                    },
                }
            }
            "animated-vector" => match attr(node, "drawable") {
                Some(r) => self.draw_reference(r, rect, out),
                None => match inline_drawable(node) {
                    Some(child) => self.draw(child, text, rect, out),
                    None => Ok(()),
                },
            },
            "rotate" => {
                // RotateDrawable at level 0: fromDegrees about the pivot
                // (a fraction or `50%` of the bounds, else px; default the
                // centre).
                let degrees = attr(node, "fromDegrees").and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.);
                let pivot = |name: &str, size: f32| match attr(node, name) {
                    Some(v) if v.ends_with('%') => v.trim_end_matches('%').parse::<f32>().ok().map(|p| p / 100. * size),
                    Some(v) => self.dimen(Some(v)),
                    None => Some(size / 2.),
                };
                let (x, y, w, h) = rect;
                let (px, py) = (x + pivot("pivotX", w).unwrap_or(w / 2.), y + pivot("pivotY", h).unwrap_or(h / 2.));
                let _ = write!(out, "<g transform=\"rotate({degrees} {px} {py})\">");
                let result = self.draw_item(node, text, rect, out);
                out.push_str("</g>");
                result
            }
            // ClipDrawable and ScaleDrawable draw nothing at level 0; a
            // bitmap or nine-patch isn't drawn here.
            _ => Ok(()),
        }
    }

    /// GradientDrawable: the shape inset by half the stroke, filled with
    /// its solid color or gradient, then stroked.
    fn shape(&mut self, node: Node, rect: Rect, out: &mut String) {
        let child = |name: &str| node.children().find(|n| n.has_tag_name(name));
        let kind = attr(node, "shape").unwrap_or("rectangle");
        let stroke = child("stroke");
        let stroke_width = stroke.and_then(|s| self.dimen(attr(s, "width"))).unwrap_or(0.);
        let stroke_color = stroke.and_then(|s| self.color(attr(s, "color")));
        let half = if stroke_color.is_some() { stroke_width / 2. } else { 0. };
        let (x, y, w, h) = (rect.0 + half, rect.1 + half, (rect.2 - 2. * half).max(0.), (rect.3 - 2. * half).max(0.));
        // The fill: a gradient over the solid color.
        let mut fill = "none".to_owned();
        if let Some(rgba) = child("solid").and_then(|s| self.color(attr(s, "color"))) {
            let (c, a) = rgb(rgba);
            fill = format!("{c}\" fill-opacity=\"{a}");
        }
        if let Some(g) = child("gradient") {
            if let Some(id) = self.gradient(g, (x, y, w, h), out) {
                fill = format!("url(#{id})");
            }
        }
        let mut paint = format!("fill=\"{fill}\"");
        if let Some(rgba) = stroke_color.filter(|_| stroke_width > 0.) {
            let (c, a) = rgb(rgba);
            let _ = write!(paint, " stroke=\"{c}\" stroke-opacity=\"{a}\" stroke-width=\"{stroke_width}\"");
            let dash = stroke.and_then(|s| self.dimen(attr(s, "dashWidth"))).unwrap_or(0.);
            if dash > 0. {
                let gap = stroke.and_then(|s| self.dimen(attr(s, "dashGap"))).unwrap_or(0.);
                let _ = write!(paint, " stroke-dasharray=\"{dash} {gap}\"");
            }
        }
        match kind {
            "oval" => {
                let _ = write!(out, "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\" {paint}/>", x + w / 2., y + h / 2., w / 2., h / 2.);
            }
            "line" => {
                // A line is only its stroke, across the middle.
                if let Some(rgba) = stroke_color {
                    let (c, a) = rgb(rgba);
                    let cy = rect.1 + rect.3 / 2.;
                    let _ = write!(out, "<line x1=\"{}\" y1=\"{cy}\" x2=\"{}\" y2=\"{cy}\" stroke=\"{c}\" stroke-opacity=\"{a}\" stroke-width=\"{}\"/>", rect.0, rect.0 + rect.2, stroke_width.max(1.));
                }
            }
            "ring" => {
                let inner = self.dimen(attr(node, "innerRadius")).unwrap_or_else(|| rect.2 / attr(node, "innerRadiusRatio").and_then(|v| v.parse::<f32>().ok()).unwrap_or(9.));
                let thickness = self.dimen(attr(node, "thickness")).unwrap_or_else(|| rect.2 / attr(node, "thicknessRatio").and_then(|v| v.parse::<f32>().ok()).unwrap_or(3.));
                let (cx, cy, outer) = (rect.0 + rect.2 / 2., rect.1 + rect.3 / 2., inner + thickness);
                let circle = |r: f32| format!("M{},{cy}a{r},{r} 0 1,0 {},0a{r},{r} 0 1,0 {},0z", cx - r, 2. * r, -2. * r);
                let _ = write!(out, "<path d=\"{}{}\" fill-rule=\"evenodd\" {paint}/>", circle(outer), circle(inner));
            }
            _ => {
                let corners = child("corners");
                let all = corners.and_then(|c| self.dimen(attr(c, "radius"))).unwrap_or(0.);
                let corner = |name: &str| corners.and_then(|c| self.dimen(attr(c, name))).unwrap_or(all).min(w / 2.).min(h / 2.).max(0.);
                let (tl, tr, br, bl) = (corner("topLeftRadius"), corner("topRightRadius"), corner("bottomRightRadius"), corner("bottomLeftRadius"));
                if tl == tr && tr == br && br == bl {
                    let _ = write!(out, "<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" rx=\"{tl}\" ry=\"{tl}\" {paint}/>");
                } else {
                    let _ = write!(
                        out,
                        "<path d=\"M{},{y}H{}A{tr},{tr} 0 0 1 {},{}V{}A{br},{br} 0 0 1 {},{}H{}A{bl},{bl} 0 0 1 {x},{}V{}A{tl},{tl} 0 0 1 {},{y}z\" {paint}/>",
                        x + tl,
                        x + w - tr,
                        x + w,
                        y + tr,
                        y + h - br,
                        x + w - br,
                        y + h,
                        x + bl,
                        y + h - bl,
                        y + tl,
                        x + tl,
                    );
                }
            }
        }
    }

    /// A `<gradient>` as an SVG gradient over the shape's bounds; its id.
    fn gradient(&mut self, g: Node, (x, y, w, h): Rect, out: &mut String) -> Option<String> {
        let start = self.color(attr(g, "startColor"))?;
        let end = self.color(attr(g, "endColor")).unwrap_or(start);
        let center = self.color(attr(g, "centerColor"));
        let fraction = |name: &str| attr(g, name).and_then(|v| v.trim_end_matches('%').parse::<f32>().ok().map(|f| if v.ends_with('%') { f / 100. } else { f }));
        let mut stops = String::new();
        let mut stop = |offset: f32, rgba: u32| {
            let (c, a) = rgb(rgba);
            let _ = write!(stops, "<stop offset=\"{offset}\" stop-color=\"{c}\" stop-opacity=\"{a}\"/>");
        };
        stop(0., start);
        if let Some(c) = center {
            stop(if attr(g, "type") == Some("radial") { 0.5 } else { fraction("centerX").unwrap_or(0.5) }, c);
        }
        stop(1., end);
        self.ids += 1;
        let id = format!("shape{}", self.ids);
        match attr(g, "type").unwrap_or("linear") {
            "radial" => {
                let (cx, cy) = (x + w * fraction("centerX").unwrap_or(0.5), y + h * fraction("centerY").unwrap_or(0.5));
                // A radius in dp, or a fraction of the shape (`50%`).
                let radius = attr(g, "gradientRadius").map_or(w.min(h) / 2., |r| {
                    if r.ends_with('%') || r.ends_with("%p") {
                        r.trim_end_matches('p').trim_end_matches('%').parse::<f32>().unwrap_or(50.) / 100. * w.min(h)
                    } else {
                        self.dimen(Some(r)).unwrap_or(w.min(h) / 2.)
                    }
                });
                let _ = write!(out, "<defs><radialGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" cx=\"{cx}\" cy=\"{cy}\" r=\"{radius}\">{stops}</radialGradient></defs>");
            }
            // Linear, and sweep drawn as linear (SVG has no conic gradient).
            _ => {
                // Android's angles, multiples of 45: 0 left to right, 90 bottom to top.
                let angle = attr(g, "angle").and_then(|a| a.parse::<i32>().ok()).unwrap_or(0).rem_euclid(360);
                let (x1, y1, x2, y2) = match angle / 45 {
                    1 => (0., 1., 1., 0.),
                    2 => (0., 1., 0., 0.),
                    3 => (1., 1., 0., 0.),
                    4 => (1., 0., 0., 0.),
                    5 => (1., 0., 0., 1.),
                    6 => (0., 0., 0., 1.),
                    7 => (0., 0., 1., 1.),
                    _ => (0., 0., 1., 0.),
                };
                let (y1, y2) = if angle / 45 == 4 || angle / 45 == 0 { (0.5, 0.5) } else { (y1, y2) };
                let (x1, x2) = if angle / 45 == 2 || angle / 45 == 6 { (0.5, 0.5) } else { (x1, x2) };
                let _ = write!(
                    out,
                    "<defs><linearGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\">{stops}</linearGradient></defs>",
                    x + w * x1,
                    y + h * y1,
                    x + w * x2,
                    y + h * y2
                );
            }
        }
        Some(id)
    }
}

/// An `<animated-vector>`'s inline drawable: `<aapt:attr name="android:drawable">`.
fn inline_drawable<'n>(node: Node<'n, 'n>) -> Option<Node<'n, 'n>> {
    node.children().find(|c| c.tag_name().name() == "attr" && c.attribute("name") == Some("android:drawable"))?.children().find(|n| n.is_element())
}

/// A layer-list's or ripple's items; a ripple's mask isn't drawn.
fn layers<'n>(node: Node<'n, 'n>) -> impl Iterator<Item = Node<'n, 'n>> {
    node.children().filter(|n| n.has_tag_name("item") && attr(*n, "id") != Some("@android:id/mask"))
}

/// An inline `<vector>` as a document of its own, with the namespaces the
/// file's root declared.
fn standalone(node: Node, text: &str) -> String {
    let own = &text[node.range()];
    if node.parent_element().is_none() {
        return own.to_owned();
    }
    let mut namespaces = String::new();
    for ns in node.namespaces() {
        if let Some(prefix) = ns.name() {
            if !own.contains(&format!("xmlns:{prefix}=")) {
                let _ = write!(namespaces, " xmlns:{prefix}=\"{}\"", ns.uri());
            }
        }
    }
    own.replacen("<vector", &format!("<vector{namespaces}"), 1)
}

fn fill_rect((x, y, w, h): Rect, rgba: u32, out: &mut String) {
    let (c, a) = rgb(rgba);
    let _ = write!(out, "<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" fill=\"{c}\" fill-opacity=\"{a}\"/>");
}

fn rgb(rgba: u32) -> (String, f32) {
    (format!("#{:06x}", rgba >> 8), (rgba & 0xff) as f32 / 255.)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NS: &str = "xmlns:android=\"http://schemas.android.com/apk/res/android\"";

    fn render_with(text: &str, files: &[(&str, String)]) -> Result<VectorImage, String> {
        let color = |r: &str| (r == "@color/accent").then_some(0x3574f0ff);
        let value = |r: &str| (r == "@dimen/radius").then(|| "6dp".to_owned());
        let drawable = |r: &str| files.iter().find(|(name, _)| *name == r).map(|(_, t)| t.clone());
        render(text, 1., &Lookup { color: &color, value: &value, drawable: &drawable })
    }

    #[test]
    fn draws_shapes() {
        let rounded = format!(
            "<shape {NS} android:shape=\"rectangle\"><solid android:color=\"@color/accent\"/><corners android:radius=\"@dimen/radius\"/>\
             <stroke android:width=\"2dp\" android:color=\"#FF0000\" android:dashWidth=\"4dp\" android:dashGap=\"2dp\"/><size android:width=\"48dp\" android:height=\"24dp\"/></shape>"
        );
        let image = render_with(&rounded, &[]).unwrap();
        assert_eq!((image.width, image.height), (48., 24.));
        assert!(image.svg.contains("<rect x=\"1\" y=\"1\" width=\"46\" height=\"22\" rx=\"6\" ry=\"6\" fill=\"#3574f0\" fill-opacity=\"1\" stroke=\"#ff0000\""), "{}", image.svg);
        assert!(image.svg.contains("stroke-dasharray=\"4 2\""), "{}", image.svg);
        // Without a size, the default; an oval with a gradient.
        let oval = format!("<shape {NS} android:shape=\"oval\"><gradient android:startColor=\"#000\" android:endColor=\"#fff\" android:angle=\"90\"/></shape>");
        let image = render_with(&oval, &[]).unwrap();
        assert_eq!(image.width, DEFAULT_SIZE);
        assert!(image.svg.contains("<linearGradient id=\"shape1\" gradientUnits=\"userSpaceOnUse\" x1=\"50\" y1=\"100\" x2=\"50\" y2=\"0\">"), "{}", image.svg);
        assert!(image.svg.contains("<ellipse cx=\"50\" cy=\"50\" rx=\"50\" ry=\"50\" fill=\"url(#shape1)\"/>"), "{}", image.svg);
        // Corners that differ: a path.
        let tab = format!("<shape {NS}><solid android:color=\"#fff\"/><corners android:topLeftRadius=\"8dp\" android:topRightRadius=\"8dp\"/><size android:width=\"40dp\" android:height=\"20dp\"/></shape>");
        assert!(render_with(&tab, &[]).unwrap().svg.contains("<path d=\"M8,0H32A8,8 0 0 1 40,8V20A0,0"));
        assert!(roxmltree::Document::parse(&render_with(&tab, &[]).unwrap().svg).is_ok());
    }

    #[test]
    fn draws_selectors_and_layers() {
        let pressed = format!("<shape {NS}><solid android:color=\"#FF0000\"/><size android:width=\"10dp\" android:height=\"10dp\"/></shape>");
        let normal = format!("<shape {NS}><solid android:color=\"#00FF00\"/><size android:width=\"10dp\" android:height=\"10dp\"/></shape>");
        let files = [("@drawable/pressed", pressed), ("@drawable/normal", normal)];
        let selector = format!(
            "<selector {NS}><item android:state_pressed=\"true\" android:drawable=\"@drawable/pressed\"/><item android:state_enabled=\"false\" android:drawable=\"@drawable/pressed\"/><item android:drawable=\"@drawable/normal\"/></selector>"
        );
        let image = render_with(&selector, &files).unwrap();
        assert!(image.svg.contains("#00ff00") && !image.svg.contains("#ff0000"), "{}", image.svg);
        assert_eq!((image.width, image.height), (10., 10.));
        let color = |_: &str| None;
        let value = |_: &str| None;
        let drawable = |r: &str| files.iter().find(|(n, _)| *n == r).map(|(_, t)| t.clone());
        let list = states(&selector, 16., &Lookup { color: &color, value: &value, drawable: &drawable });
        assert_eq!(list.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>(), ["pressed", "not enabled", "default"]);
        // Layers with insets over a color; a ripple's mask left out.
        let layers = format!(
            "<layer-list {NS}><item><color android:color=\"#000000\"/></item><item android:left=\"2dp\" android:top=\"2dp\" android:drawable=\"@drawable/normal\"/></layer-list>"
        );
        let image = render_with(&layers, &files).unwrap();
        assert_eq!((image.width, image.height), (12., 12.));
        assert!(image.svg.contains("<rect x=\"0\" y=\"0\" width=\"12\" height=\"12\" fill=\"#000000\"") && image.svg.contains("<rect x=\"2\" y=\"2\" width=\"10\" height=\"10\" rx=\"0\""), "{}", image.svg);
        let ripple = format!("<ripple {NS} android:color=\"#20000000\"><item android:id=\"@android:id/mask\"><color android:color=\"#FFFFFF\"/></item><item android:drawable=\"@drawable/normal\"/></ripple>");
        let svg = render_with(&ripple, &files).unwrap().svg;
        assert!(svg.contains("#00ff00") && !svg.contains("#ffffff"), "{svg}");
        // An inline vector keeps the file's namespace; a loop stops.
        let inline = format!("<selector {NS}><item><vector android:width=\"8dp\" android:height=\"8dp\" android:viewportWidth=\"8\" android:viewportHeight=\"8\"><path android:fillColor=\"#f00\" android:pathData=\"M0,0h8v8z\"/></vector></item></selector>");
        let image = render_with(&inline, &[]).unwrap();
        assert!(image.width == 8. && image.svg.contains("<svg x=\"0\" y=\"0\" width=\"8\" height=\"8\""), "{}", image.svg);
        // Every picture parses as SVG.
        for text in [&selector, &layers, &ripple, &inline] {
            let svg = render_with(text, &files).unwrap().svg;
            assert!(roxmltree::Document::parse(&svg).is_ok(), "{svg}");
        }
        let looped = [("@drawable/me", format!("<inset {NS} android:drawable=\"@drawable/me\" android:inset=\"1dp\"/>"))];
        assert!(render_with(&looped[0].1, &looped).is_err());
        assert!(is_previewable(&selector) && !is_previewable("<bitmap/>"));
    }

    #[test]
    fn draws_animated_and_level_drawables() {
        let vector = format!("<vector {NS} android:width=\"24dp\" android:height=\"24dp\" android:viewportWidth=\"24\" android:viewportHeight=\"24\"><path android:fillColor=\"#f00\" android:pathData=\"M0,0h24v24h-24z\"/></vector>");
        // An animated vector: its vector, by reference or inline.
        let by_ref = format!("<animated-vector {NS} android:drawable=\"@drawable/v\"><target android:name=\"a\" android:animation=\"@animator/x\"/></animated-vector>");
        let image = render_with(&by_ref, &[("@drawable/v", vector.clone())]).unwrap();
        assert_eq!((image.width, image.height), (24., 24.));
        assert!(image.svg.contains("fill=\"#ff0000\""), "{}", image.svg);
        let inline = format!(
            "<animated-vector {NS} xmlns:aapt=\"http://schemas.android.com/aapt\"><aapt:attr name=\"android:drawable\">{}</aapt:attr></animated-vector>",
            vector.replace(&format!(" {NS}"), "")
        );
        let image = render_with(&inline, &[]).unwrap();
        assert!(image.svg.contains("fill=\"#ff0000\"") && roxmltree::Document::parse(&image.svg).is_ok(), "{}", image.svg);
        // A rotate at fromDegrees about its centre; clip draws nothing.
        let rotate = format!("<rotate {NS} android:fromDegrees=\"45\" android:drawable=\"@drawable/v\"/>");
        let image = render_with(&rotate, &[("@drawable/v", vector.clone())]).unwrap();
        assert!(image.svg.contains("<g transform=\"rotate(45 12 12)\">"), "{}", image.svg);
        let clip = format!("<clip {NS} android:drawable=\"@drawable/v\"/>");
        let image = render_with(&clip, &[("@drawable/v", vector.clone())]).unwrap();
        assert_eq!(image.width, 24.);
        assert!(!image.svg.contains("path"), "{}", image.svg);
        // An animation list: its first frame.
        let frames = format!("<animation-list {NS}><item android:drawable=\"@color/accent\" android:duration=\"100\"/><item android:drawable=\"#000\" android:duration=\"100\"/></animation-list>");
        assert!(render_with(&frames, &[]).unwrap().svg.contains("#3574f0"));
        assert!(is_previewable(&rotate) && is_previewable(&inline));
    }
}