//! Theme attributes (`?attr/colorPrimary`, `?android:attr/textColorPrimary`)
//! as the app's theme gives them: the theme from the manifest, its items,
//! then its parents' (an explicit `parent`, or the name before the last
//! dot), as Android's `Resources.Theme` looks them up.

/// One `<style>`: its parent and items, in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    /// The parent's name (`Theme.MaterialComponents.DayNight`); a framework
    /// parent keeps its `android:` prefix.
    pub parent: Option<String>,
    /// `(name, value)`: `colorPrimary`, `android:colorAccent`.
    pub items: Vec<(String, String)>,
}

/// The styles a values XML declares.
pub fn parse_styles(text: &str) -> Vec<(String, Style)> {
    let Ok(doc) = roxmltree::Document::parse(text) else { return Vec::new() };
    doc.root_element()
        .children()
        .filter(|n| n.has_tag_name("style"))
        .filter_map(|n| {
            let name = n.attribute("name")?.to_owned();
            let parent = match n.attribute("parent") {
                // `parent=""` ends the chain, even for a dotted name.
                Some(p) => {
                    let p = p.trim();
                    let p = p.strip_prefix("@style/").or_else(|| p.strip_prefix("style/")).unwrap_or(p);
                    let p = p.strip_prefix("@android:style/").map(|s| format!("android:{s}")).unwrap_or_else(|| p.to_owned());
                    (!p.is_empty()).then_some(p)
                }
                None => name.rsplit_once('.').map(|(prefix, _)| prefix.to_owned()),
            };
            let items = n
                .children()
                .filter(|c| c.has_tag_name("item"))
                .filter_map(|c| Some((c.attribute("name")?.trim().to_owned(), c.text().unwrap_or("").trim().to_owned())))
                .collect();
            Some((name, Style { parent, items }))
        })
        .collect()
}

/// `?attr/colorPrimary` → `colorPrimary`, `?android:attr/colorAccent` →
/// `android:colorAccent`; None for anything else.
pub fn attr_name(value: &str) -> Option<String> {
    let rest = value.trim().strip_prefix('?')?;
    let (framework, rest) = match rest.strip_prefix("android:") {
        Some(r) => (true, r),
        None => (false, rest),
    };
    let name = rest.strip_prefix("attr/").unwrap_or(rest);
    if name.is_empty() || name.contains('/') {
        return None;
    }
    Some(if framework { format!("android:{name}") } else { name.to_owned() })
}

/// The application's theme in a manifest (`@style/Theme.App` → `Theme.App`).
pub fn manifest_theme(manifest: &str) -> Option<String> {
    let doc = roxmltree::Document::parse(manifest).ok()?;
    let app = doc.descendants().find(|n| n.has_tag_name("application"))?;
    let theme = app.attribute(("http://schemas.android.com/apk/res/android", "theme"))?;
    theme.strip_prefix("@style/").map(str::to_owned)
}

/// The value `theme` gives the attribute `name` (`colorPrimary`), walking
/// its parents; `style` finds a style by name (None for one not in the
/// project, such as a library's or the framework's).
pub fn attr_value(theme: &str, name: &str, style: &dyn Fn(&str) -> Option<Style>) -> Option<String> {
    let mut current = Some(theme.to_owned());
    // Against parent loops.
    for _ in 0..32 {
        let s = style(&current?)?;
        if let Some((_, value)) = s.items.iter().rev().find(|(n, _)| n == name) {
            return Some(value.clone());
        }
        current = s.parent;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_theme_attributes() {
        let xml = r#"<resources>
            <style name="Theme.App" parent="Theme.Base"><item name="colorPrimary">@color/brand</item></style>
            <style name="Theme.Base" parent="@style/Theme.MaterialComponents"><item name="colorPrimary">#111111</item><item name="colorAccent">#222222</item><item name="android:colorAccent">#333333</item></style>
            <style name="Theme.App.Dark"><item name="colorSurface">#000000</item></style>
            <style name="Loop" parent="Loop"/>
        </resources>"#;
        let styles = parse_styles(xml);
        assert_eq!(styles[1].1.parent.as_deref(), Some("Theme.MaterialComponents"));
        assert_eq!(styles[2].1.parent.as_deref(), Some("Theme.App"));
        let find = |n: &str| styles.iter().find(|(s, _)| s == n).map(|(_, s)| s.clone());
        assert_eq!(attr_value("Theme.App", "colorPrimary", &find).as_deref(), Some("@color/brand"));
        assert_eq!(attr_value("Theme.App.Dark", "colorAccent", &find).as_deref(), Some("#222222"));
        assert_eq!(attr_value("Theme.App", "android:colorAccent", &find).as_deref(), Some("#333333"));
        assert_eq!(attr_value("Theme.App", "colorError", &find), None);
        assert_eq!(attr_value("Loop", "x", &find), None);
        assert_eq!(attr_name("?attr/colorPrimary").as_deref(), Some("colorPrimary"));
        assert_eq!(attr_name("?colorPrimary").as_deref(), Some("colorPrimary"));
        assert_eq!(attr_name("?android:attr/colorAccent").as_deref(), Some("android:colorAccent"));
        assert_eq!(attr_name("@color/x"), None);
        let manifest = r#"<manifest xmlns:android="http://schemas.android.com/apk/res/android"><application android:theme="@style/Theme.App"/></manifest>"#;
        assert_eq!(manifest_theme(manifest).as_deref(), Some("Theme.App"));
    }
}
