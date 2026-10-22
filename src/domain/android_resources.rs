//! Android resources as Android Studio knows them: the values declared in
//! `res/values*/*.xml`, the file resources (`res/drawable*/ic.png`,
//! `res/layout/main.xml`…), the ids layouts declare with `@+id/`, and the
//! references to them from code (`R.string.app_name`) and XML
//! (`@string/app_name`, `?attr/colorPrimary`).

use std::sync::LazyLock;

use regex::Regex;

/// A resource's `R` class: `string`, `color`, `drawable`, `id`…
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ResourceRef {
    pub kind: std::sync::Arc<str>,
    /// As `R` spells it: `Theme_App` for the style `Theme.App`. Shared:
    /// a name is declared once per translation.
    pub name: std::sync::Arc<str>,
}

impl ResourceRef {
    pub fn new(kind: &str, name: &str) -> Self {
        Self { kind: normalize_kind(kind).into(), name: field_name(name).into() }
    }

    /// `@string/app_name`.
    pub fn xml(&self) -> String {
        format!("@{}/{}", self.kind, self.name)
    }
}

/// Where a resource is declared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declaration {
    pub resource: ResourceRef,
    /// Relative to the project root (shared by the file's declarations).
    pub path: std::sync::Arc<str>,
    /// The folder: `values`, `values-zh-rCN`, `drawable-hdpi`.
    pub folder: std::sync::Arc<str>,
    /// 0-based line and character column of the name (0, 0 for a file).
    pub line: u32,
    pub col: u32,
    /// A value's text (`My App`, `#FF6200EE`, `@color/purple`); None for files and ids.
    pub value: Option<String>,
}

/// R field names: dots, dashes and colons become underscores.
pub fn field_name(name: &str) -> String {
    name.replace(['.', '-', ':'], "_")
}

/// The `R` class of a values tag or folder type.
fn normalize_kind(kind: &str) -> &str {
    match kind {
        "string-array" | "integer-array" => "array",
        "declare-styleable" => "styleable",
        other => other,
    }
}

/// Folders whose files are resources themselves.
const FILE_KINDS: [&str; 13] = ["drawable", "mipmap", "layout", "menu", "raw", "xml", "anim", "animator", "color", "font", "navigation", "transition", "interpolator"];

/// `res/drawable-hdpi/ic_launcher.png` → (drawable, ic_launcher, drawable-hdpi).
pub fn file_resource(path: &str) -> Option<(ResourceRef, String)> {
    let mut parts = path.rsplit('/');
    let file = parts.next()?;
    let folder = parts.next()?;
    if parts.next()? != "res" {
        return None;
    }
    let kind = folder.split('-').next()?;
    if !FILE_KINDS.contains(&kind) || file.starts_with('.') {
        return None;
    }
    let stem = file.split('.').next()?;
    Some((ResourceRef::new(kind, stem), folder.to_owned()))
}

/// `res/values-zh/strings.xml` → `values-zh`.
pub fn values_folder(path: &str) -> Option<String> {
    let mut parts = path.rsplit('/');
    let file = parts.next()?;
    let folder = parts.next()?;
    (parts.next()? == "res" && (folder == "values" || folder.starts_with("values-")) && file.ends_with(".xml")).then(|| folder.to_owned())
}

const VALUE_TAGS: [&str; 15] = ["string-array", "integer-array", "string", "color", "dimen", "bool", "integer", "fraction", "style", "plurals", "array", "attr", "declare-styleable", "item", "drawable"];
static NEW_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"@\+id/([\w.]+)"#).unwrap());

/// `attr="value"` in a tag's attributes: the value and its offset there.
fn attribute<'a>(attrs: &'a str, attr: &str) -> Option<(&'a str, usize)> {
    let bytes = attrs.as_bytes();
    let mut from = 0;
    while let Some(i) = attrs[from..].find(attr).map(|i| from + i) {
        from = i + attr.len();
        if i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b':' || bytes[i - 1] == b'_') {
            continue;
        }
        let rest = attrs[from..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else { continue };
        let rest = rest.trim_start();
        let Some(value) = rest.strip_prefix('"') else { continue };
        let at = attrs.len() - value.len();
        return value.find('"').map(|end| (&value[..end], at));
    }
    None
}

/// The values a `res/values*/*.xml` declares, with the offset of each name.
/// A plain scan over the tags: a translated strings.xml runs to 700 KB and
/// a project has hundreds of them.
pub fn parse_values(text: &str) -> Vec<(ResourceRef, usize, Option<String>)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(lt) = text[from..].find('<').map(|i| from + i) {
        from = lt + 1;
        let rest = &text[from..];
        // Comments may hold markup; skip them whole.
        if rest.starts_with("!--") {
            from = rest.find("-->").map_or(text.len(), |i| from + i + 3);
            continue;
        }
        let Some(tag) = VALUE_TAGS.iter().find(|t| rest.starts_with(**t) && rest.as_bytes().get(t.len()).is_some_and(|b| !(b.is_ascii_alphanumeric() || *b == b'-' || *b == b'_'))) else { continue };
        let attrs_start = from + tag.len();
        let Some(gt) = text[attrs_start..].find('>').map(|i| attrs_start + i) else { break };
        let self_closing = text.as_bytes()[gt - 1] == b'/';
        let attrs = &text[attrs_start..if self_closing { gt - 1 } else { gt }];
        from = gt + 1;
        let Some((name, name_at)) = attribute(attrs, "name") else { continue };
        let name_at = attrs_start + name_at;
        let kind = match *tag {
            // An item is a resource only with a type (`<item type="id" name="x"/>`);
            // inside a style or an array it is not.
            "item" => match attribute(attrs, "type") {
                Some((t, _)) => t,
                None => continue,
            },
            // A framework attribute used in a styleable (`android:textColor`).
            "attr" if name.contains(':') => continue,
            other => other,
        };
        let value = (!self_closing && matches!(kind, "string" | "color" | "dimen" | "bool" | "integer" | "fraction" | "drawable")).then(|| {
            let end = text[from..].find("</").map_or(text.len(), |i| from + i);
            unescape(text[from..end].trim())
        });
        out.push((ResourceRef::new(kind, name), name_at, value));
    }
    out
}

/// The ids a layout (or menu, navigation graph…) declares with `@+id/`.
pub fn parse_new_ids(text: &str) -> Vec<(ResourceRef, usize)> {
    NEW_ID.captures_iter(text).map(|c| (ResourceRef::new("id", &c[1]), c.get(1).unwrap().start())).collect()
}

fn unescape(value: &str) -> String {
    let value = value.strip_prefix("<![CDATA[").and_then(|v| v.strip_suffix("]]>")).unwrap_or(value);
    let value = if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') { &value[1..value.len() - 1] } else { value };
    value.replace("\\'", "'").replace("\\\"", "\"").replace("\\n", "\n").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

/// The identifier around `offset`: its byte range.
fn word_around(text: &str, offset: usize) -> Option<(usize, usize)> {
    let offset = offset.min(text.len());
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let start = text[..offset].char_indices().rev().take_while(|(_, c)| is_word(*c)).last().map_or(offset, |(i, _)| i);
    let end = text[offset..].char_indices().find(|(_, c)| !is_word(*c)).map_or(text.len(), |(i, _)| offset + i);
    (start < end).then_some((start, end))
}

/// `R.string.app_name` (or `R.string` when on the type) at `offset` in
/// Kotlin or Java. The framework's `android.R.…` is not the project's.
pub fn code_reference_at(text: &str, offset: usize) -> Option<ResourceRef> {
    let (start, end) = word_around(text, offset)?;
    let word = &text[start..end];
    let before = &text[..start];
    // On the name: `R.<type>.` before it.
    if let Some(rest) = before.strip_suffix('.') {
        let type_start = rest.rfind(|c: char| !(c.is_alphanumeric() || c == '_')).map_or(0, |i| i + 1);
        let kind = &rest[type_start..];
        let r = rest[..type_start].strip_suffix('.')?;
        if r.ends_with('R') && !r[..r.len() - 1].ends_with(|c: char| c.is_alphanumeric() || c == '_') && !r[..r.len() - 1].ends_with("android.") && !kind.is_empty() {
            return Some(ResourceRef::new(kind, word));
        }
    }
    None
}

/// `@string/app_name`, `@+id/title`, `?attr/colorPrimary` or
/// `?colorPrimary` at `offset` in XML; `@android:…` is the framework's.
/// The bool tells a `@+id/` (a declaration).
pub fn xml_reference_at(text: &str, offset: usize) -> Option<(ResourceRef, bool)> {
    let offset = offset.min(text.len());
    let stop = |c: char| c == '"' || c == '\'' || c == '>' || c == '<' || c.is_whitespace();
    let start = text[..offset].rfind(stop).map_or(0, |i| i + 1);
    let end = text[offset..].find(stop).map_or(text.len(), |i| offset + i);
    let token = &text[start..end];
    static REF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(@|\?)(\+)?(?:([\w.]+):)?(?:(\w+)/)?([\w.]+)$").unwrap());
    let c = REF.captures(token)?;
    if c.get(3).is_some_and(|p| p.as_str() == "android") {
        return None;
    }
    let kind = match (c.get(4), &c[1]) {
        (Some(k), _) => k.as_str(),
        (None, "?") => "attr",
        (None, _) => return None,
    };
    Some((ResourceRef::new(kind, &c[5]), c.get(2).is_some()))
}

/// On a values declaration's name (`<string name="app_name">`): the resource.
pub fn declaration_at(path: &str, text: &str, offset: usize) -> Option<ResourceRef> {
    values_folder(path)?;
    parse_values(text).into_iter().find(|(r, at, _)| {
        let raw_len = text[*at..].find('"').unwrap_or(0);
        (*at..=*at + raw_len).contains(&offset) && !r.name.is_empty()
    }).map(|(r, _, _)| r)
}

/// The patterns that find a resource's uses: `R.type.name` in code,
/// `@type/name` (and `@+id/`, `?attr/`) in XML. A regex.
pub fn usage_pattern(r: &ResourceRef) -> String {
    // A style `Theme.App` is `Theme_App` in R but written with dots in XML.
    let xml_name = regex::escape(&r.name).replace('_', "[._]");
    let code = format!(r"\bR\.{}\.{}\b", regex::escape(&r.kind), regex::escape(&r.name));
    let xml = match &*r.kind {
        "id" => format!(r"@\+?id/{xml_name}\b"),
        "attr" => format!(r#"\?(?:attr/)?{xml_name}\b|name="{xml_name}""#),
        kind => format!(r"@{kind}/{xml_name}\b"),
    };
    // A style's parent by name (`parent="Theme.App"`, `Theme.App.Dark`).
    let style = if &*r.kind == "style" { format!(r#"|parent="{xml_name}""#) } else { String::new() };
    format!("{code}|{xml}{style}")
}

/// A color value as RGBA, from `#RGB`, `#ARGB`, `#RRGGBB` or `#AARRGGBB`.
pub fn parse_color(value: &str) -> Option<u32> {
    let hex = value.trim().strip_prefix('#')?;
    let digits: Vec<u32> = hex.chars().map(|c| c.to_digit(16)).collect::<Option<_>>()?;
    let (a, r, g, b) = match digits.as_slice() {
        [r, g, b] => (15 * 17, r * 17, g * 17, b * 17),
        [a, r, g, b] => (a * 17, r * 17, g * 17, b * 17),
        [r1, r2, g1, g2, b1, b2] => (255, r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2),
        [a1, a2, r1, r2, g1, g2, b1, b2] => (a1 * 16 + a2, r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2),
        _ => return None,
    };
    Some((r << 24) | (g << 16) | (b << 8) | a)
}

/// An RGBA color written back over the literal `like` the way Android
/// Studio's picker writes it: `#RRGGBB` when opaque and the literal had no
/// alpha, else `#AARRGGBB`, in the literal's letter case.
pub fn format_color(rgba: u32, like: &str) -> String {
    let (r, g, b, a) = (rgba >> 24, (rgba >> 16) & 0xFF, (rgba >> 8) & 0xFF, rgba & 0xFF);
    let digits = like.trim().trim_start_matches('#').len();
    let hex = if a == 0xFF && matches!(digits, 3 | 6) { format!("#{r:02X}{g:02X}{b:02X}") } else { format!("#{a:02X}{r:02X}{g:02X}{b:02X}") };
    if like.chars().any(|c| c.is_ascii_lowercase()) { hex.to_ascii_lowercase() } else { hex }
}

/// A Compose `Color(0xFF6200EE)` literal's RGBA: the number is ARGB, so
/// `0x6200EE` is fully transparent, as Compose reads it.
pub fn parse_compose_color(literal: &str) -> Option<u32> {
    let hex = literal.trim().strip_prefix("0x").or_else(|| literal.trim().strip_prefix("0X"))?;
    let hex = hex.trim_end_matches(['L', 'u', 'U']);
    if hex.is_empty() || hex.len() > 8 {
        return None;
    }
    let argb = u32::from_str_radix(hex, 16).ok()?;
    Some(argb.rotate_left(8))
}

/// An RGBA color written back over a Compose literal the way Android
/// Studio's picker writes it: `0xAARRGGBB`, in the literal's letter case.
pub fn format_compose_color(rgba: u32, like: &str) -> String {
    let hex = format!("{:08X}", rgba.rotate_right(8));
    let digits = like.trim().get(2..).unwrap_or("");
    let hex = if digits.chars().any(|c| c.is_ascii_lowercase()) { hex.to_ascii_lowercase() } else { hex };
    format!("{}{hex}", like.trim().get(..2).unwrap_or("0x"))
}

/// The folder Android uses when no qualifier matches comes first, then the rest by name.
pub fn folder_order(folder: &str) -> (bool, &str) {
    (folder.contains('-'), folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_colors_like_the_literal() {
        assert_eq!(format_color(0x112233FF, "#FF6200EE"), "#FF112233");
        assert_eq!(format_color(0x112233FF, "#6200ee"), "#112233");
        assert_eq!(format_color(0x11223380, "#6200EE"), "#80112233");
        assert_eq!(format_color(0xAABBCCFF, "#abc"), "#aabbcc");
        assert_eq!(parse_color(&format_color(0x12345678, "#000")), Some(0x12345678));
    }

    #[test]
    fn compose_colors() {
        assert_eq!(parse_compose_color("0xFF6200EE"), Some(0x6200EEFF));
        assert_eq!(parse_compose_color("0x806200EEL"), Some(0x6200EE80));
        // No alpha digits: transparent, as Compose reads the number.
        assert_eq!(parse_compose_color("0x6200EE"), Some(0x6200EE00));
        assert_eq!(parse_compose_color("0x1FF6200EE"), None);
        assert_eq!(format_compose_color(0x112233FF, "0xFF6200EE"), "0xFF112233");
        assert_eq!(format_compose_color(0x11223380, "0xff6200ee"), "0x80112233");
        assert_eq!(format_compose_color(0xAABBCCFF, "0Xff6200ee"), "0Xffaabbcc");
    }

    #[test]
    fn reads_values() {
        let xml = r#"<resources>
    <string name="app_name">My \'App\'</string>
    <color name="purple_500">#FF6200EE</color>
    <style name="Theme.App" parent="Theme.Material3.DayNight">
        <item name="colorPrimary">@color/purple_500</item>
    </style>
    <item type="id" name="extra_id"/>
    <declare-styleable name="Chip"><attr name="chipColor" format="color"/><attr name="android:textColor"/></declare-styleable>
    <string-array name="planets"><item>Earth</item></string-array>
</resources>"#;
        let v = parse_values(xml);
        let names: Vec<String> = v.iter().map(|(r, _, _)| format!("{}/{}", r.kind, r.name)).collect();
        assert_eq!(names, ["string/app_name", "color/purple_500", "style/Theme_App", "id/extra_id", "styleable/Chip", "attr/chipColor", "array/planets"]);
        assert_eq!(v[0].2.as_deref(), Some("My 'App'"));
        assert_eq!(v[1].2.as_deref(), Some("#FF6200EE"));
        assert_eq!(&xml[v[0].1..v[0].1 + 8], "app_name");
    }

    #[test]
    fn finds_references() {
        let kt = "setTitle(R.string.app_name); android.R.string.ok; Rx.string.no";
        assert_eq!(code_reference_at(kt, kt.find("app_name").unwrap() + 2), Some(ResourceRef::new("string", "app_name")));
        assert_eq!(code_reference_at(kt, kt.find(".ok").unwrap() + 2), None);
        assert_eq!(code_reference_at(kt, kt.find("no").unwrap()), None);
        let xml = r#"<TextView android:id="@+id/title" android:text="@string/app_name" android:textColor="?attr/colorPrimary" style="@style/Theme.App" android:icon="@android:drawable/ic"/>"#;
        assert_eq!(xml_reference_at(xml, xml.find("app_name").unwrap()), Some((ResourceRef::new("string", "app_name"), false)));
        assert_eq!(xml_reference_at(xml, xml.find("title").unwrap()), Some((ResourceRef::new("id", "title"), true)));
        assert_eq!(xml_reference_at(xml, xml.find("colorPrimary").unwrap()), Some((ResourceRef::new("attr", "colorPrimary"), false)));
        assert_eq!(xml_reference_at(xml, xml.find("Theme.App").unwrap() + 7), Some((ResourceRef::new("style", "Theme_App"), false)));
        assert_eq!(xml_reference_at(xml, xml.find("ic\"").unwrap()), None);
    }

    #[test]
    fn file_resources_and_patterns() {
        assert_eq!(file_resource("app/src/main/res/drawable-hdpi/ic_launcher.9.png"), Some((ResourceRef::new("drawable", "ic_launcher"), "drawable-hdpi".into())));
        assert_eq!(file_resource("app/src/main/res/values/strings.xml"), None);
        assert_eq!(values_folder("app/src/main/res/values-zh-rCN/strings.xml").as_deref(), Some("values-zh-rCN"));
        let re = Regex::new(&usage_pattern(&ResourceRef::new("style", "Theme.App"))).unwrap();
        assert!(re.is_match("R.style.Theme_App") && re.is_match("@style/Theme.App") && re.is_match(r#"parent="Theme.App""#));
        assert_eq!(parse_color("#F00"), Some(0xFF0000FF));
        assert_eq!(parse_color("#806200EE"), Some(0x6200EE80));
        let xml = r#"<string name="app_name">x</string>"#;
        assert_eq!(declaration_at("a/res/values/strings.xml", xml, xml.find("app").unwrap() + 1), Some(ResourceRef::new("string", "app_name")));
    }
}

/// Where the project's resources come from.
pub trait ResourceSource {
    /// Every declaration of every module's `res` folders.
    fn scan(&self) -> Vec<Declaration>;
}
