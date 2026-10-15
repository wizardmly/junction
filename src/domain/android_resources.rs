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
    pub kind: String,
    /// As `R` spells it: `Theme_App` for the style `Theme.App`.
    pub name: String,
}

impl ResourceRef {
    pub fn new(kind: &str, name: &str) -> Self {
        Self { kind: normalize_kind(kind).to_owned(), name: field_name(name) }
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
    /// Relative to the project root.
    pub path: String,
    /// The folder: `values`, `values-zh-rCN`, `drawable-hdpi`.
    pub folder: String,
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

static TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<(string-array|integer-array|string|color|dimen|bool|integer|fraction|style|plurals|array|attr|declare-styleable|item|drawable)\b([^>]*?)(/?)>"#).unwrap()
});
static NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\bname\s*=\s*"([^"]+)""#).unwrap());
static TYPE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\btype\s*=\s*"([^"]+)""#).unwrap());
static NEW_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"@\+id/([\w.]+)"#).unwrap());

/// The values a `res/values*/*.xml` declares, with the offset of each name.
pub fn parse_values(text: &str) -> Vec<(ResourceRef, usize, Option<String>)> {
    let mut out = Vec::new();
    for m in TAG.captures_iter(text) {
        let (tag, attrs) = (&m[1], m.get(2).unwrap());
        let Some(name) = NAME.captures(attrs.as_str()) else { continue };
        let name_at = attrs.start() + name.get(1).unwrap().start();
        let name = &name[1];
        let kind = match tag {
            // An item is a resource only with a type (`<item type="id" name="x"/>`);
            // inside a style or an array it is not.
            "item" => match TYPE.captures(attrs.as_str()) {
                Some(t) => t[1].to_owned(),
                None => continue,
            },
            // A framework attribute used in a styleable (`android:textColor`).
            "attr" if name.contains(':') => continue,
            other => other.to_owned(),
        };
        let self_closing = &m[3] == "/";
        let value = (!self_closing && matches!(kind.as_str(), "string" | "color" | "dimen" | "bool" | "integer" | "fraction" | "drawable")).then(|| {
            let start = m.get(0).unwrap().end();
            let end = text[start..].find("</").map_or(text.len(), |i| start + i);
            unescape(text[start..end].trim())
        });
        out.push((ResourceRef::new(&kind, name), name_at, value));
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
    let xml = match r.kind.as_str() {
        "id" => format!(r"@\+?id/{xml_name}\b"),
        "attr" => format!(r#"\?(?:attr/)?{xml_name}\b|name="{xml_name}""#),
        kind => format!(r"@{kind}/{xml_name}\b"),
    };
    // A style's parent by name (`parent="Theme.App"`, `Theme.App.Dark`).
    let style = if r.kind == "style" { format!(r#"|parent="{xml_name}""#) } else { String::new() };
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

/// The folder Android uses when no qualifier matches comes first, then the rest by name.
pub fn folder_order(folder: &str) -> (bool, String) {
    (folder.contains('-'), folder.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

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
