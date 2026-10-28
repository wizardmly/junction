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

/// The framework's resource at `offset`: `android.R.string.ok` in Kotlin or
/// Java, `@android:color/white`, `?android:attr/textColorPrimary` or
/// `?android:textColorPrimary` in XML.
pub fn framework_reference_at(path: &str, text: &str, offset: usize) -> Option<ResourceRef> {
    if path.ends_with(".xml") {
        let offset = offset.min(text.len());
        let stop = |c: char| c == '"' || c == '\'' || c == '>' || c == '<' || c.is_whitespace();
        let start = text[..offset].rfind(stop).map_or(0, |i| i + 1);
        let end = text[offset..].find(stop).map_or(text.len(), |i| offset + i);
        static REF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(@|\?)android:(?:(\w+)/)?([\w.]+)$").unwrap());
        let c = REF.captures(&text[start..end])?;
        let kind = match (c.get(2), &c[1]) {
            (Some(k), _) => k.as_str(),
            (None, "?") => "attr",
            (None, _) => return None,
        };
        return Some(ResourceRef::new(kind, &c[3]));
    }
    let (start, end) = word_around(text, offset)?;
    let rest = text[..start].strip_suffix('.')?;
    let type_start = rest.rfind(|c: char| !(c.is_alphanumeric() || c == '_')).map_or(0, |i| i + 1);
    let kind = &rest[type_start..];
    let qualifier = rest[..type_start].strip_suffix("android.R.")?;
    let whole = !qualifier.ends_with(|c: char| c.is_alphanumeric() || c == '_' || c == '.');
    (whole && !kind.is_empty()).then(|| ResourceRef::new(kind, &text[start..end]))
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

/// A Compose `Color(red, green, blue[, alpha])`'s arguments as RGBA,
/// as Android Studio's ComposeColorLineMarkerProviderDescriptor reads
/// them: three or four Ints (0–255, decimal or `0x`) or Floats (0–1,
/// `0.5f`), positional or named `red =` / `green =` / `blue =` / `alpha =`.
pub fn parse_compose_components(args: &str) -> Option<u32> {
    const NAMES: [&str; 4] = ["red", "green", "blue", "alpha"];
    let parts: Vec<&str> = args.split(',').map(str::trim).collect();
    if !matches!(parts.len(), 3 | 4) {
        return None;
    }
    let float = parts[0].rsplit('=').next()?.trim().ends_with(['f', 'F']) && !parts[0].rsplit('=').next()?.trim().starts_with("0x");
    let mut named: Vec<(&str, u32)> = Vec::new();
    let mut positional = Vec::new();
    for part in &parts {
        let (name, value) = match part.split_once('=') {
            Some((n, v)) => (Some(n.trim()), v.trim()),
            None => (None, *part),
        };
        let channel = if float {
            let v: f32 = value.strip_suffix(['f', 'F'])?.parse().ok()?;
            if !(0.0..=1.0).contains(&v) {
                return None;
            }
            // java.awt.Color's float constructor.
            (v * 255. + 0.5) as u32
        } else {
            let v = match value.strip_prefix("0x").or_else(|| value.strip_prefix("0X")) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => value.parse().ok()?,
            };
            (v <= 255).then_some(v)?
        };
        match name {
            Some(n) if NAMES.contains(&n) && !named.iter().any(|(m, _)| *m == n) => named.push((n, channel)),
            Some(_) => return None,
            None => positional.push(channel),
        }
    }
    // Positional values fill the names not given, in order.
    if named.iter().any(|(n, _)| !NAMES[..parts.len()].contains(n)) {
        return None;
    }
    let unnamed: Vec<&str> = NAMES[..parts.len()].iter().copied().filter(|n| !named.iter().any(|(m, _)| m == n)).collect();
    if unnamed.len() != positional.len() {
        return None;
    }
    named.extend(unnamed.into_iter().zip(positional));
    let get = |n: &str| named.iter().find(|(m, _)| *m == n).map(|(_, v)| *v);
    let alpha = if parts.len() == 4 { get("alpha")? } else { 0xFF };
    Some((get("red")? << 24) | (get("green")? << 16) | (get("blue")? << 8) | alpha)
}

/// A picked RGBA color as Android Studio writes it over `Color(…)`'s
/// components: always with alpha, named when any argument was, `0x` Ints
/// when any argument was hex, Floats rounded to 3 decimals.
pub fn format_compose_components(rgba: u32, like: &str) -> String {
    let parts: Vec<&str> = like.split(',').map(str::trim).collect();
    let named = parts.iter().any(|p| p.contains('='));
    let values: Vec<&str> = parts.iter().map(|p| p.rsplit('=').next().unwrap_or(p).trim()).collect();
    let float = values.first().is_some_and(|v| v.ends_with(['f', 'F']) && !v.starts_with("0x"));
    let hex = values.iter().any(|v| v.starts_with("0x"));
    let channels = [rgba >> 24, (rgba >> 16) & 0xFF, (rgba >> 8) & 0xFF, rgba & 0xFF];
    let text = channels.map(|c| {
        if float {
            // Kotlin's Float.toString of the rounded value: `0.5`, `1.0`.
            let rounded: f32 = format!("{:.3}", c as f32 / 255.).parse().unwrap_or(0.);
            let s = rounded.to_string();
            format!("{}f", if s.contains('.') { s } else { format!("{s}.0") })
        } else if hex {
            format!("0x{c:X}")
        } else {
            c.to_string()
        }
    });
    if named {
        format!("red = {}, green = {}, blue = {}, alpha = {}", text[0], text[1], text[2], text[3])
    } else {
        text.join(", ")
    }
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
        // Components.
        assert_eq!(parse_compose_components("255, 0, 0"), Some(0xFF0000FF));
        assert_eq!(parse_compose_components("0xFF, 0x80, 0, 0x40"), Some(0xFF800040));
        assert_eq!(parse_compose_components("red = 1f, green = 0.5f, blue = 0f"), Some(0xFF8000FF));
        assert_eq!(parse_compose_components("blue = 0f, red = 1f, 0f, alpha = .5f"), Some(0xFF000080));
        assert_eq!(parse_compose_components("256, 0, 0"), None);
        assert_eq!(parse_compose_components("1.5f, 0f, 0f"), None);
        assert_eq!(parse_compose_components("red, 0, 0"), None);
        assert_eq!(parse_compose_components("1, 2"), None);
        assert_eq!(parse_compose_components("1, 2, alpha = 3"), None);
        assert_eq!(parse_compose_components("red = 1, red = 2, 3"), None);
        assert_eq!(format_compose_components(0x11223380, "255, 0, 0"), "17, 34, 51, 128");
        assert_eq!(format_compose_components(0x112233FF, "red = 0xFF, green = 0, blue = 0"), "red = 0x11, green = 0x22, blue = 0x33, alpha = 0xFF");
        assert_eq!(format_compose_components(0xFF8000FF, "1f, 0f, 0f"), "1.0f, 0.502f, 0.0f, 1.0f");
        assert_eq!(parse_compose_components(&format_compose_components(0x12345678, "0f, 0f, 0f")), Some(0x12345678));
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

    #[test]
    fn framework_references() {
        let kt = "val a = android.R.string.ok\nval b = R.string.ok\nval c = my.android.R.string.ok";
        assert_eq!(framework_reference_at("a.kt", kt, kt.find("ok").unwrap() + 1), Some(ResourceRef::new("string", "ok")));
        assert_eq!(framework_reference_at("a.kt", kt, kt.rfind("R.string.ok\nval").map_or(0, |i| i + 9)), None);
        assert_eq!(framework_reference_at("a.kt", kt, kt.len() - 1), None);
        let xml = r#"<View android:background="@android:color/white" android:textColor="?android:attr/textColorPrimary" android:tint="?android:colorAccent" style="@style/Mine"/>"#;
        assert_eq!(framework_reference_at("a.xml", xml, xml.find("white").unwrap()), Some(ResourceRef::new("color", "white")));
        assert_eq!(framework_reference_at("a.xml", xml, xml.find("textColorPrimary").unwrap()), Some(ResourceRef::new("attr", "textColorPrimary")));
        assert_eq!(framework_reference_at("a.xml", xml, xml.find("colorAccent").unwrap()), Some(ResourceRef::new("attr", "colorAccent")));
        assert_eq!(framework_reference_at("a.xml", xml, xml.find("Mine").unwrap()), None);
        // The attribute's own name is not a reference.
        assert_eq!(framework_reference_at("a.xml", xml, xml.find("background").unwrap()), None);
    }
}

/// Where the project's resources come from.
pub trait ResourceSource {
    /// Every declaration of every module's `res` folders.
    fn scan(&self) -> Vec<Declaration>;
}
