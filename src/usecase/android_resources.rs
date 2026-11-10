//! Android resources for navigation and previews: the project's resource
//! table, where a reference leads, and the value a color or string has.

use std::collections::HashMap;

use crate::domain::android_resources::{self as res, Declaration, ResourceRef, ResourceSource};

/// All declarations in one block, grouped by resource (one allocation, so
/// a big app's few hundred thousand don't fragment the heap), and where
/// each resource's group is.
#[derive(Default)]
pub struct ResourceTable {
    all: Vec<Declaration>,
    by_ref: HashMap<ResourceRef, (u32, u32)>,
}

impl ResourceTable {
    pub fn build(source: &dyn ResourceSource) -> Self {
        let mut all = source.scan();
        // By resource, then the default folder's first.
        all.sort_unstable_by(|a, b| {
            (&a.resource, res::folder_order(&a.folder), &a.path, a.line).cmp(&(&b.resource, res::folder_order(&b.folder), &b.path, b.line))
        });
        // An id declared in several places of one file counts once per file.
        all.dedup_by(|a, b| a.resource == b.resource && a.path == b.path && &*a.resource.kind == "id");
        all.shrink_to_fit();
        let mut by_ref: HashMap<ResourceRef, (u32, u32)> = HashMap::new();
        let mut start = 0;
        for i in 1..=all.len() {
            if i == all.len() || all[i].resource != all[start].resource {
                // One copy of each name for all its declarations.
                let key = all[start].resource.clone();
                for d in &mut all[start + 1..i] {
                    d.resource = key.clone();
                }
                by_ref.insert(key, (start as u32, i as u32));
                start = i;
            }
        }
        Self { all, by_ref }
    }

    /// (resources, declarations, values) for benchmarks.
    #[cfg(test)]
    pub fn stats(&self) -> (usize, usize, usize) {
        (self.by_ref.len(), self.all.len(), self.all.iter().filter(|d| d.value.is_some()).count())
    }

    /// Go to Declaration: every declaration, the default folder's first.
    pub fn declarations(&self, r: &ResourceRef) -> &[Declaration] {
        self.by_ref.get(r).map_or(&[], |&(a, b)| &self.all[a as usize..b as usize])
    }

    /// The default value (the unqualified folder's), following references
    /// (`@color/primary` → `#FF6200EE`).
    pub fn value(&self, r: &ResourceRef) -> Option<String> {
        let mut r = r.clone();
        for _ in 0..8 {
            let value = self.declarations(&r).iter().find_map(|d| d.value.clone())?;
            match value.strip_prefix('@').and_then(|v| v.split_once('/')) {
                Some((kind, name)) if !kind.starts_with("android") => r = ResourceRef::new(kind, name),
                _ => return Some(value),
            }
        }
        None
    }

    /// A color resource's RGBA.
    pub fn color(&self, r: &ResourceRef) -> Option<u32> {
        self.value(r).and_then(|v| res::parse_color(&v))
    }

    /// Every color resource with a color value, by name, for the color
    /// picker's Resources tab (AS's `CompactResourcePicker`).
    pub fn colors(&self) -> Vec<(String, u32)> {
        let mut out: Vec<(String, u32)> = self.by_ref.keys().filter(|r| &*r.kind == "color").filter_map(|r| Some((r.name.to_string(), self.color(r)?))).collect();
        out.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// A drawable or mipmap declared as XML (a vector, a shape…), the
    /// unqualified folder's first.
    pub fn drawable_xml(&self, r: &ResourceRef) -> Option<&Declaration> {
        if !matches!(&*r.kind, "drawable" | "mipmap") {
            return None;
        }
        self.declarations(r).iter().filter(|d| d.path.ends_with(".xml")).min_by_key(|d| res::folder_order(&d.folder))
    }

    /// A drawable or mipmap's bitmap to preview (the densest PNG / WebP / JPG).
    pub fn image(&self, r: &ResourceRef) -> Option<&Declaration> {
        if !matches!(&*r.kind, "drawable" | "mipmap") {
            return None;
        }
        let density = |folder: &str| ["xxxhdpi", "xxhdpi", "xhdpi", "hdpi", "mdpi"].iter().position(|d| folder.contains(d)).unwrap_or(5);
        self.declarations(r).iter().filter(|d| [".png", ".webp", ".jpg", ".jpeg", ".gif"].iter().any(|e| d.path.ends_with(e)) && !d.path.ends_with(".9.png")).min_by_key(|d| density(&d.folder))
    }

    /// The image file a drawable reference names for drawing it in a
    /// drawable preview: the densest, a `.9.png` too.
    pub fn bitmap(&self, r: &ResourceRef) -> Option<&Declaration> {
        if !matches!(&*r.kind, "drawable" | "mipmap") {
            return None;
        }
        let density = |folder: &str| ["xxxhdpi", "xxhdpi", "xhdpi", "hdpi", "mdpi"].iter().position(|d| folder.contains(d)).unwrap_or(5);
        self.declarations(r).iter().filter(|d| [".png", ".webp", ".jpg", ".jpeg", ".gif"].iter().any(|e| d.path.ends_with(e))).min_by_key(|d| density(&d.folder))
    }

    /// That image, read: its bytes, density and whether it's a nine-patch.
    pub fn load_bitmap(&self, root: &std::path::Path, r: &ResourceRef) -> Option<crate::domain::drawable::Bitmap> {
        let d = self.bitmap(r)?;
        let bytes = std::fs::read(root.join(d.path.to_string())).ok()?;
        Some(crate::domain::drawable::Bitmap { bytes, density: crate::domain::drawable::Bitmap::density_of(&d.folder), nine_patch: d.path.ends_with(".9.png") })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(Vec<Declaration>);
    impl ResourceSource for Fake {
        fn scan(&self) -> Vec<Declaration> {
            self.0.clone()
        }
    }

    fn decl(kind: &str, name: &str, folder: &str, value: Option<&str>) -> Declaration {
        Declaration { resource: ResourceRef::new(kind, name), path: format!("app/src/main/res/{folder}/x.xml").into(), folder: folder.into(), line: 1, col: 1, value: value.map(Into::into) }
    }

    #[test]
    fn resolves_values() {
        let t = ResourceTable::build(&Fake(vec![
            decl("string", "app_name", "values-zh", Some("应用")),
            decl("string", "app_name", "values", Some("App")),
            decl("color", "primary", "values", Some("@color/purple")),
            decl("color", "purple", "values", Some("#6200EE")),
        ]));
        let name = ResourceRef::new("string", "app_name");
        assert_eq!(&*t.declarations(&name)[0].folder, "values");
        assert_eq!(t.value(&name).as_deref(), Some("App"));
        assert_eq!(t.color(&ResourceRef::new("color", "primary")), Some(0x6200EEFF));
        let marks = previews(&t, None, "a/Main.kt", "x\nval c = R.color.primary\nR.color.none");
        assert_eq!(marks, vec![(1, Preview::Color(0x6200EEFF))]);
        let marks = previews(&t, None, "res/layout/a.xml", "<View android:background=\"#F00\"\n android:tint=\"@color/purple\"/>");
        assert_eq!(marks, vec![(0, Preview::Color(0xFF0000FF)), (1, Preview::Color(0x6200EEFF))]);
        assert_eq!(color_literal("a.xml", "<View android:background=\"#F00\""), Some(26..30));
        assert_eq!(color_literal("a.xml", "    <color name=\"a\">#FF6200EE</color>"), Some(20..29));
        assert_eq!(color_literal("a.xml", " android:tint=\"@color/purple\"/>"), None);
        // Compose literals.
        let code = "val Purple = Color(0xFF6200EE)\nval Clear = Color(color = 0x00000000L)\nColor(0xFF000000).copy(alpha = 0.5f)\nColor(red = 1f, green = 0f, blue = 0f)";
        assert_eq!(previews(&t, None, "ui/Theme.kt", code), vec![(0, Preview::Color(0x6200EEFF)), (1, Preview::Color(0)), (2, Preview::Color(0x000000FF)), (3, Preview::Color(0xFF0000FF))]);
        assert!(previews(&t, None, "ui/Theme.java", code).is_empty());
        // The framework's colors, from its own table.
        let framework = ResourceTable::build(&Fake(vec![decl("color", "white", "values", Some("#ffffffff"))]));
        let code = "a(android.R.color.white)\nb(R.color.white)";
        assert_eq!(previews(&t, Some(&framework), "a/Main.kt", code), vec![(0, Preview::Color(0xFFFFFFFF))]);
        assert!(previews(&t, None, "a/Main.kt", code).is_empty());
        let xml = "<View android:background=\"@android:color/white\"\n android:tint=\"@color/purple\"/>";
        assert_eq!(previews(&t, Some(&framework), "a/res/layout/a.xml", xml), vec![(0, Preview::Color(0xFFFFFFFF)), (1, Preview::Color(0x6200EEFF))]);
        assert_eq!(color_literal("ui/Theme.kt", "val Purple = Color(0xFF6200EE)"), Some(19..29));
        assert_eq!(color_literal("ui/Theme.kt", "val c = R.color.primary"), None);
        assert_eq!(format_color_literal(0x112233FF, "0xff6200ee"), "0xff112233");
        assert_eq!(format_color_literal(0x112233FF, "#6200EE"), "#112233");
        let line = "val Red = Color(red = 255, green = 0, blue = 0)";
        let args = color_literal("ui/Theme.kt", line).unwrap();
        assert_eq!(&line[args.clone()], "red = 255, green = 0, blue = 0");
        assert_eq!(format_color_literal(0x112233FF, &line[args]), "red = 17, green = 34, blue = 51, alpha = 255");
        assert_eq!(color_literal("ui/Theme.kt", "Color(a, b, c)"), None);
        assert_eq!(preview_reference("a.xml", " android:tint=\"@color/purple\"/>"), Some(22));
        assert_eq!(preview_reference("a.kt", "val c = R.color.primary"), Some(16));
    }

    #[test]
    fn color_popup_targets() {
        let t = ResourceTable::build(&Fake(vec![
            decl("color", "purple", "values", Some("#6200EE")),
            decl("color", "accent", "values", Some("@color/purple")),
            decl("color", "broken", "values", Some("@color/none")),
            decl("string", "app_name", "values", Some("App")),
        ]));
        assert_eq!(t.colors(), vec![("accent".to_owned(), 0x6200EEFF), ("purple".to_owned(), 0x6200EEFF)]);
        // XML: a literal or a reference, both tabs.
        let line = "<View android:background=\"#F00\"";
        assert_eq!(color_target("a.xml", line), Some(ColorTarget { range: 26..30, custom: true, resources: true, reference: false }));
        let line = " android:tint=\"@android:color/white\"/>";
        let target = color_target("a.xml", line).unwrap();
        assert_eq!((&line[target.range.clone()], target.custom, target.reference), ("@android:color/white", true, true));
        assert_eq!(color_target("a.xml", " android:src=\"@drawable/ic\"/>"), None);
        // Code: R.color.x takes resources only, Compose literals custom only.
        let line = "setColor(android.R.color.white)";
        let target = color_target("a/Main.java", line).unwrap();
        assert_eq!((&line[target.range.clone()], target.custom, target.resources), ("android.R.color.white", false, true));
        let line = "val c = R.color.primary";
        assert_eq!(&line[color_target("a/Main.kt", line).unwrap().range], "R.color.primary");
        let target = color_target("ui/Theme.kt", "val Purple = Color(0xFF6200EE)").unwrap();
        assert_eq!((target.custom, target.resources), (true, false));
        assert_eq!(color_reference("a.xml", "colorPrimary", false), "@color/colorPrimary");
        assert_eq!(color_reference("a.xml", "white", true), "@android:color/white");
        assert_eq!(color_reference("a/Main.kt", "brand.dark", false), "R.color.brand_dark");
        assert_eq!(color_reference("a/Main.java", "white", true), "android.R.color.white");
        // A custom color over a reference: #RRGGBB, #AARRGGBB when not opaque.
        assert_eq!(format_color_literal(0x112233FF, "@color/purple"), "#112233");
        assert_eq!(format_color_literal(0x11223380, "@color/purple"), "#80112233");
    }
}

/// What the editor gutter previews on a line, as Android Studio does: a
/// color's swatch or a bitmap drawable's thumbnail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Preview {
    /// RGBA.
    Color(u32),
    /// The image, relative to the project.
    Image(String),
    /// A drawable XML (a vector, once read), relative to the project.
    Vector(String),
}

/// The byte range of the color literal whose swatch a line shows, the
/// text a color picker rewrites: `#FF6200EE` in XML, a Compose
/// `Color(0xFF6200EE)` in Kotlin.
pub fn color_literal(path: &str, line: &str) -> Option<std::ops::Range<usize>> {
    if path.ends_with(".kt") {
        // A resource reference comes first on the line, as its preview does.
        if CODE.is_match(line) {
            return None;
        }
        if let Some(hex) = COMPOSE.captures(line).and_then(|c| c.get(1)) {
            return res::parse_compose_color(hex.as_str()).map(|_| hex.range());
        }
        // `Color(red, green, blue, alpha)`: the picker rewrites them all.
        let args = COMPONENTS.captures(line)?.get(1)?;
        return res::parse_compose_components(args.as_str()).map(|_| args.range());
    }
    if !path.ends_with(".xml") {
        return None;
    }
    let hex = XML.captures(line)?.get(3)?;
    res::parse_color(hex.as_str()).map(|_| hex.range())
}

/// What a color swatch's picker rewrites on its line, as Android Studio's
/// `ColorRenderer`: the color literal, or the `@color/x` / `R.color.x`
/// reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColorTarget {
    /// The bytes of the line it rewrites.
    pub range: std::ops::Range<usize>,
    /// The Custom tab: a color literal can be written there (not in code
    /// over `R.color.x`).
    pub custom: bool,
    /// The Resources tab: a resource can be written there (not over a
    /// Compose `Color(…)`).
    pub resources: bool,
    /// It is a resource reference now: the picker opens on Resources.
    pub reference: bool,
}

/// The color swatch's target on a Kotlin, Java or XML line.
pub fn color_target(path: &str, line: &str) -> Option<ColorTarget> {
    let code = path.ends_with(".kt") || path.ends_with(".java");
    if code {
        if let Some(c) = CODE.captures(line)
            && c.get(1).is_some_and(|k| k.as_str() == "color")
        {
            let whole = c.get(0)?;
            // The match may start with the character before `R.`.
            let start = whole.as_str().find("android.R.").or_else(|| whole.as_str().find("R.")).map_or(whole.start(), |i| whole.start() + i);
            return Some(ColorTarget { range: start..whole.end(), custom: false, resources: true, reference: true });
        }
        return color_literal(path, line).map(|range| ColorTarget { range, custom: true, resources: false, reference: false });
    }
    if !path.ends_with(".xml") {
        return None;
    }
    let c = XML.captures(line)?;
    if let Some(hex) = c.get(3) {
        res::parse_color(hex.as_str())?;
        return Some(ColorTarget { range: hex.range(), custom: true, resources: true, reference: false });
    }
    (c.get(1)?.as_str() == "color").then(|| ColorTarget { range: c.get(0).unwrap().range(), custom: true, resources: true, reference: true })
}

/// A picked color resource as the file refers to it: `@color/x` /
/// `@android:color/x` in XML, `R.color.x` / `android.R.color.x` in code.
pub fn color_reference(path: &str, name: &str, framework: bool) -> String {
    let field = res::field_name(name);
    match (path.ends_with(".xml"), framework) {
        (true, false) => format!("@color/{name}"),
        (true, true) => format!("@android:color/{name}"),
        (false, false) => format!("R.color.{field}"),
        (false, true) => format!("android.R.color.{field}"),
    }
}

/// A picked RGBA color as the literal at `like` takes it.
pub fn format_color_literal(rgba: u32, like: &str) -> String {
    if like.starts_with('@') {
        // Over a reference: Android Studio's `colorToString`, `#AARRGGBB`
        // unless opaque.
        res::format_color(rgba, "#000000")
    } else if like.starts_with('#') {
        res::format_color(rgba, like)
    } else if like.contains(',') {
        res::format_compose_components(rgba, like)
    } else {
        res::format_compose_color(rgba, like)
    }
}

/// Where on a Kotlin, Java or XML line the previewed resource's name
/// starts, for Go to Declaration from its gutter icon.
pub fn preview_reference(path: &str, line: &str) -> Option<usize> {
    let code = path.ends_with(".kt") || path.ends_with(".java");
    let caps = if code { CODE.captures(line) } else { XML.captures(line) }?;
    caps.get(2).map(|m| m.start())
}

static CODE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"(?:^|[^\w.]|\bandroid\.)R\.(color|drawable|mipmap)\.(\w+)").unwrap());
static COMPOSE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"\bColor\(\s*(?:color\s*=\s*)?(0[xX][0-9a-fA-F]{1,8}L?)\s*\)").unwrap());
static COMPONENTS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"\bColor\(([^()]*,[^()]*)\)").unwrap());
static XML: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r#"@(?:android:)?(color|drawable|mipmap)/([\w.]+)|[">](#[0-9a-fA-F]{3,8})["<]"#).unwrap());

/// The first previewable resource of each line of a Kotlin, Java or XML
/// file; `framework` resolves `android.R.color.x` and `@android:color/x`.
pub fn previews(table: &ResourceTable, framework: Option<&ResourceTable>, path: &str, text: &str) -> Vec<(u32, Preview)> {
    let code = path.ends_with(".kt") || path.ends_with(".java");
    if !code && !path.ends_with(".xml") {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (line, content) in text.lines().enumerate() {
        let caps = if code { CODE.captures(content) } else { XML.captures(content) };
        let Some(c) = caps else {
            // A Compose `Color(0xFF6200EE)`.
            let compose = || {
                COMPOSE
                    .captures(content)
                    .and_then(|c| res::parse_compose_color(c.get(1)?.as_str()))
                    .or_else(|| COMPONENTS.captures(content).and_then(|c| res::parse_compose_components(c.get(1)?.as_str())))
            };
            if let Some(rgba) = path.ends_with(".kt").then(compose).flatten() {
                out.push((line as u32, Preview::Color(rgba)));
            }
            continue;
        };
        let preview = match (c.get(1), c.get(2), c.get(3)) {
            (_, _, Some(hex)) => res::parse_color(hex.as_str()).map(Preview::Color),
            (Some(kind), Some(name), _) => {
                let r = ResourceRef::new(kind.as_str(), name.as_str());
                let framework_ref = c.get(0).is_some_and(|m| m.as_str().contains("android"));
                let Some(table) = (if framework_ref { framework } else { Some(table) }) else { continue };
                match kind.as_str() {
                    "color" => table.color(&r).map(Preview::Color),
                    _ => table.image(&r).map(|d| Preview::Image(d.path.to_string())).or_else(|| table.drawable_xml(&r).map(|d| Preview::Vector(d.path.to_string()))),
                }
            }
            _ => None,
        };
        if let Some(p) = preview {
            out.push((line as u32, p));
        }
    }
    out
}
