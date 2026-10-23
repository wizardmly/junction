//! Android's VectorDrawable as Android Studio previews it: the `<vector>`
//! XML turned into SVG, its paths (`pathData` is SVG path syntax) with
//! their fill and stroke, groups with their pivot / rotate / scale /
//! translate, clip paths, gradients (`<aapt:attr>`), the vector's tint and
//! alpha, a stroke's `trimPathStart` / `End` / `Offset`; `@color/…`
//! resolved by the caller.

use std::fmt::Write as _;

use crate::domain::android_resources::parse_color;

const ANDROID: &str = "http://schemas.android.com/apk/res/android";

/// The preview: SVG text, and the drawable's size in dp.
#[derive(Clone, Debug, PartialEq)]
pub struct VectorImage {
    pub svg: String,
    pub width: f32,
    pub height: f32,
}

/// Whether an XML file is a vector drawable (its root is `<vector>`).
#[cfg(test)]
pub fn is_vector(text: &str) -> bool {
    roxmltree::Document::parse(text).is_ok_and(|d| d.root_element().tag_name().name() == "vector")
}

/// The vector as SVG, drawn `scale` times its dp size. `color` resolves a
/// reference (`@color/accent`) to RGBA.
pub fn to_svg(text: &str, scale: f32, color: &dyn Fn(&str) -> Option<u32>) -> Result<VectorImage, String> {
    let doc = roxmltree::Document::parse(text).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    if root.tag_name().name() != "vector" {
        return Err("Not a vector drawable".into());
    }
    let attr = |name: &str| root.attribute((ANDROID, name));
    let dp = |v: Option<&str>| v.and_then(|v| v.trim_end_matches("dp").trim_end_matches("dip").trim_end_matches("px").parse::<f32>().ok());
    let (Some(width), Some(height)) = (dp(attr("width")), dp(attr("height"))) else { return Err("The vector has no android:width / android:height".into()) };
    let vw = attr("viewportWidth").and_then(|v| v.parse::<f32>().ok()).unwrap_or(width);
    let vh = attr("viewportHeight").and_then(|v| v.parse::<f32>().ok()).unwrap_or(height);
    let mut cx = Writer { out: String::new(), defs: String::new(), ids: 0, color, tint: attr("tint").and_then(|t| resolve(t, color)) };
    for child in root.children().filter(|n| n.is_element()) {
        cx.node(child);
    }
    let alpha = attr("alpha").and_then(|a| a.parse::<f32>().ok()).unwrap_or(1.);
    let body = std::mem::take(&mut cx.out);
    let mut svg = String::new();
    let _ = write!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {vw} {vh}\" preserveAspectRatio=\"none\">",
        width * scale,
        height * scale
    );
    if !cx.defs.is_empty() {
        let _ = write!(svg, "<defs>{}</defs>", cx.defs);
    }
    let _ = write!(svg, "<g opacity=\"{alpha}\">{body}</g></svg>");
    Ok(VectorImage { svg, width, height })
}

struct Writer<'a> {
    out: String,
    defs: String,
    ids: usize,
    color: &'a dyn Fn(&str) -> Option<u32>,
    tint: Option<u32>,
}

impl Writer<'_> {
    fn id(&mut self, kind: &str) -> String {
        self.ids += 1;
        format!("{kind}{}", self.ids)
    }

    fn node(&mut self, node: roxmltree::Node) {
        match node.tag_name().name() {
            "group" => self.group(node),
            "path" => self.path(node),
            _ => {}
        }
    }

    /// A group: Android's transform is translate(pivot + translate),
    /// rotate, scale, translate(-pivot). A `<clip-path>` clips the
    /// group's later children.
    fn group(&mut self, node: roxmltree::Node) {
        let f = |name: &str, default: f32| node.attribute((ANDROID, name)).and_then(|v| v.parse::<f32>().ok()).unwrap_or(default);
        let (px, py) = (f("pivotX", 0.), f("pivotY", 0.));
        let transform = format!(
            "translate({} {}) rotate({}) scale({} {}) translate({} {})",
            px + f("translateX", 0.),
            py + f("translateY", 0.),
            f("rotation", 0.),
            f("scaleX", 1.),
            f("scaleY", 1.),
            -px,
            -py
        );
        let _ = write!(self.out, "<g transform=\"{transform}\">");
        let mut open = 0;
        for child in node.children().filter(|n| n.is_element()) {
            if child.tag_name().name() == "clip-path" {
                let Some(data) = child.attribute((ANDROID, "pathData")) else { continue };
                let id = self.id("clip");
                let _ = write!(self.defs, "<clipPath id=\"{id}\"><path transform=\"{transform}\" d=\"{}\"/></clipPath>", escape(data));
                // The clip is in the group's space: applied outside it.
                let _ = write!(self.out, "</g><g clip-path=\"url(#{id})\"><g transform=\"{transform}\">");
                open += 1;
                continue;
            }
            self.node(child);
        }
        // The group, then each clip around it (clips add up, as Android's).
        self.out.push_str("</g>");
        for _ in 0..open {
            self.out.push_str("</g>");
        }
    }

    fn path(&mut self, node: roxmltree::Node) {
        let Some(data) = node.attribute((ANDROID, "pathData")) else { return };
        let attr = |name: &str| node.attribute((ANDROID, name));
        let num = |name: &str, default: f32| attr(name).and_then(|v| v.parse::<f32>().ok()).unwrap_or(default);
        let fill = self.paint(node, "fillColor");
        let stroke = self.paint(node, "strokeColor");
        let mut out = format!("<path d=\"{}\"", escape(data));
        match fill {
            Some(paint) => {
                let _ = write!(out, " fill=\"{}\" fill-opacity=\"{}\"", paint.0, paint.1 * num("fillAlpha", 1.));
            }
            None => out.push_str(" fill=\"none\""),
        }
        if attr("fillType") == Some("evenOdd") {
            out.push_str(" fill-rule=\"evenodd\"");
        }
        let width = num("strokeWidth", 0.);
        // Trimmed: only that part of the stroke, as dashes; none left,
        // no stroke.
        let (start, end, offset) = (num("trimPathStart", 0.), num("trimPathEnd", 1.), num("trimPathOffset", 0.));
        let trim = if stroke.is_some() && width > 0. { crate::domain::path_length::trim_dashes(crate::domain::path_length::length(data), start, end, offset) } else { None };
        if let Some(paint) = stroke.filter(|_| width > 0. && trim.as_ref().is_none_or(|(d, _)| !d.is_empty())) {
            let _ = write!(out, " stroke=\"{}\" stroke-opacity=\"{}\" stroke-width=\"{width}\"", paint.0, paint.1 * num("strokeAlpha", 1.));
            let cap = match attr("strokeLineCap") {
                Some("round") => "round",
                Some("square") => "square",
                _ => "butt",
            };
            let join = match attr("strokeLineJoin") {
                Some("round") => "round",
                Some("bevel") => "bevel",
                _ => "miter",
            };
            let _ = write!(out, " stroke-linecap=\"{cap}\" stroke-linejoin=\"{join}\" stroke-miterlimit=\"{}\"", num("strokeMiterLimit", 4.));
            if let Some((dashes, dash_offset)) = &trim {
                let _ = write!(out, " stroke-dasharray=\"{dashes}\" stroke-dashoffset=\"{dash_offset}\"");
            }
        }
        out.push_str("/>");
        self.out.push_str(&out);
    }

    /// A fill or stroke: a color (with its alpha), or a gradient from an
    /// `<aapt:attr name="android:fillColor">`; the vector's tint wins.
    fn paint(&mut self, node: roxmltree::Node, name: &str) -> Option<(String, f32)> {
        let gradient = node
            .children()
            .find(|c| c.tag_name().name() == "attr" && c.attribute("name") == Some(&format!("android:{name}")))
            .and_then(|a| a.children().find(|c| c.tag_name().name() == "gradient"));
        if let Some(tint) = self.tint {
            let has = node.attribute((ANDROID, name)).is_some() || gradient.is_some();
            return has.then(|| rgb(tint));
        }
        if let Some(g) = gradient {
            return self.gradient(g);
        }
        let value = node.attribute((ANDROID, name))?;
        let rgba = resolve(value, self.color)?;
        Some(rgb(rgba))
    }

    fn gradient(&mut self, g: roxmltree::Node) -> Option<(String, f32)> {
        let attr = |name: &str| g.attribute((ANDROID, name));
        let num = |name: &str| attr(name).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.);
        let color = |name: &str| attr(name).and_then(|v| resolve(v, self.color));
        let mut stops: Vec<(f32, u32)> = g
            .children()
            .filter(|c| c.tag_name().name() == "item")
            .filter_map(|c| Some((c.attribute((ANDROID, "offset"))?.parse().ok()?, resolve(c.attribute((ANDROID, "color"))?, self.color)?)))
            .collect();
        if stops.is_empty() {
            let (start, end) = (color("startColor")?, color("endColor")?);
            stops.push((0., start));
            if let Some(center) = color("centerColor") {
                stops.push((0.5, center));
            }
            stops.push((1., end));
        }
        let stop_text: String = stops
            .iter()
            .map(|(offset, rgba)| {
                let (c, a) = rgb(*rgba);
                format!("<stop offset=\"{offset}\" stop-color=\"{c}\" stop-opacity=\"{a}\"/>")
            })
            .collect();
        let id = self.id("gradient");
        match attr("type").unwrap_or("linear") {
            "radial" => {
                let _ = write!(
                    self.defs,
                    "<radialGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" cx=\"{}\" cy=\"{}\" r=\"{}\">{stop_text}</radialGradient>",
                    num("centerX"),
                    num("centerY"),
                    num("gradientRadius")
                );
            }
            "sweep" => return Some(rgb(stops[0].1)),
            _ => {
                let _ = write!(
                    self.defs,
                    "<linearGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\">{stop_text}</linearGradient>",
                    num("startX"),
                    num("startY"),
                    num("endX"),
                    num("endY")
                );
            }
        }
        Some((format!("url(#{id})"), 1.))
    }
}

/// A color value: `#ARGB` forms, a reference the caller knows, or the
/// framework's plain colors.
pub(crate) fn resolve(value: &str, color: &dyn Fn(&str) -> Option<u32>) -> Option<u32> {
    let value = value.trim();
    if value.starts_with('#') {
        return parse_color(value);
    }
    if let Some(name) = value.strip_prefix("@android:color/") {
        return Some(match name {
            "white" => 0xffffffff,
            "black" => 0x000000ff,
            "transparent" => 0,
            "darker_gray" => 0xaaaaaaff,
            _ => return None,
        });
    }
    // A theme attribute (`?attr/colorControlNormal`): the caller's theme,
    // else near black as Android Studio's default theme draws it.
    if value.starts_with('?') {
        return color(value).or(Some(0x000000_8a));
    }
    color(value)
}

/// RGBA as an SVG color and opacity.
fn rgb(rgba: u32) -> (String, f32) {
    (format!("#{:06x}", rgba >> 8), (rgba & 0xff) as f32 / 255.)
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ICON: &str = r##"<vector xmlns:android="http://schemas.android.com/apk/res/android"
        xmlns:aapt="http://schemas.android.com/aapt"
        android:width="24dp" android:height="24dp"
        android:viewportWidth="24" android:viewportHeight="24">
        <path android:fillColor="#FF000000" android:pathData="M12,2L2,22h20z"/>
        <group android:pivotX="12" android:pivotY="12" android:rotation="45" android:scaleX="0.5">
            <path android:strokeColor="@color/accent" android:strokeWidth="2" android:strokeLineCap="round" android:pathData="M0,0L24,24" android:fillType="evenOdd"/>
        </group>
        <path android:pathData="M0,0h24v24h-24z">
            <aapt:attr name="android:fillColor">
                <gradient android:type="linear" android:startX="0" android:startY="0" android:endX="24" android:endY="0" android:startColor="#80FF0000" android:endColor="#0000FF"/>
            </aapt:attr>
        </path>
    </vector>"##;

    fn colors(name: &str) -> Option<u32> {
        (name == "@color/accent").then_some(0x3574f0ff)
    }

    #[test]
    fn converts_to_svg() {
        let image = to_svg(ICON, 4., &colors).unwrap();
        assert_eq!((image.width, image.height), (24., 24.));
        let svg = &image.svg;
        assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"96\" height=\"96\" viewBox=\"0 0 24 24\""), "{svg}");
        assert!(svg.contains("<path d=\"M12,2L2,22h20z\" fill=\"#000000\" fill-opacity=\"1\"/>"), "{svg}");
        assert!(svg.contains("transform=\"translate(12 12) rotate(45) scale(0.5 1) translate(-12 -12)\""), "{svg}");
        assert!(svg.contains("fill=\"none\" fill-rule=\"evenodd\" stroke=\"#3574f0\" stroke-opacity=\"1\" stroke-width=\"2\" stroke-linecap=\"round\""), "{svg}");
        assert!(svg.contains("<linearGradient id=\"gradient1\" gradientUnits=\"userSpaceOnUse\" x1=\"0\" y1=\"0\" x2=\"24\" y2=\"0\"><stop offset=\"0\" stop-color=\"#ff0000\" stop-opacity=\"0.5019608\"/>"), "{svg}");
        assert!(svg.contains("fill=\"url(#gradient1)\""), "{svg}");
        // It parses as SVG.
        assert!(roxmltree::Document::parse(svg).is_ok());
        assert!(is_vector(ICON) && !is_vector("<selector/>"));
    }

    #[test]
    fn tints_and_clips() {
        let tinted = ICON.replace("android:viewportHeight=\"24\">", "android:viewportHeight=\"24\" android:tint=\"#FFFFFF\" android:alpha=\"0.5\">");
        let svg = to_svg(&tinted, 1., &colors).unwrap().svg;
        assert!(svg.contains("fill=\"#ffffff\"") && !svg.contains("#000000") && svg.contains("<g opacity=\"0.5\">"), "{svg}");
        let clipped = r##"<vector xmlns:android="http://schemas.android.com/apk/res/android" android:width="10dp" android:height="10dp" android:viewportWidth="10" android:viewportHeight="10">
            <group><clip-path android:pathData="M0,0h5v5h-5z"/><path android:fillColor="#f00" android:pathData="M0,0h10v10h-10z"/></group></vector>"##;
        let svg = to_svg(clipped, 1., &colors).unwrap().svg;
        assert!(svg.contains("<clipPath id=\"clip1\">") && svg.contains("clip-path=\"url(#clip1)\""), "{svg}");
        assert!(roxmltree::Document::parse(&svg).is_ok(), "{svg}");
        assert_eq!(to_svg("<selector/>", 1., &colors), Err("Not a vector drawable".into()));
    }

    #[test]
    fn trims_strokes() {
        let v = |trim: &str| format!(r##"<vector xmlns:android="http://schemas.android.com/apk/res/android" android:width="10dp" android:height="10dp" android:viewportWidth="10" android:viewportHeight="10">
            <path android:strokeColor="#f00" android:strokeWidth="1" android:pathData="M0,5 L10,5" {trim}/></vector>"##);
        let svg = to_svg(&v("android:trimPathStart=\"0.2\" android:trimPathEnd=\"0.7\""), 1., &colors).unwrap().svg;
        assert!(svg.contains("stroke-dasharray=\"5 20\" stroke-dashoffset=\"-2\""), "{svg}");
        let svg = to_svg(&v("android:trimPathEnd=\"0\""), 1., &colors).unwrap().svg;
        assert!(!svg.contains("stroke="), "{svg}");
        let svg = to_svg(&v(""), 1., &colors).unwrap().svg;
        assert!(!svg.contains("dasharray"), "{svg}");
    }
}