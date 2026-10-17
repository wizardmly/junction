//! The Translations Editor's rules, as Android Studio's: the `<string>`
//! resources of `res/values*/` folders, the locale a folder is for
//! (`values-fr`, `values-zh-rCN`, `values-b+sr+Latn`) and its label
//! ("Chinese (zh) in China (CN)"), Android's string escaping, and the
//! edits that set, add or remove a string in a values file.

use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

/// A `<string>` in a values file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StringEntry {
    pub name: String,
    /// What the editor shows: unescaped, or the markup as written.
    pub value: String,
    pub translatable: bool,
    /// The element, `<string …>…</string>` or `<string …/>`.
    pub range: Range<usize>,
    /// The text between the tags (empty at the end of a self-closed one).
    pub content: Range<usize>,
}

static STRING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?s)<string\b([^>]*?)(/>|>(.*?)</string\s*>)"#).unwrap());
static ATTR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"([\w:]+)\s*=\s*"([^"]*)""#).unwrap());
static COMMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<!--.*?-->").unwrap());

/// The `<string>` resources of a values file, in order.
pub fn parse_strings(text: &str) -> Vec<StringEntry> {
    // Commented out ones don't count.
    let comments: Vec<Range<usize>> = COMMENT.find_iter(text).map(|m| m.range()).collect();
    let mut out = Vec::new();
    for c in STRING.captures_iter(text) {
        let whole = c.get(0).unwrap();
        if comments.iter().any(|r| r.contains(&whole.start())) {
            continue;
        }
        let attrs = c.get(1).map_or("", |m| m.as_str());
        let attr = |name: &str| ATTR.captures_iter(attrs).find(|a| &a[1] == name).map(|a| a[2].to_owned());
        let Some(name) = attr("name") else { continue };
        let content = match c.get(3) {
            Some(m) => m.range(),
            None => whole.end()..whole.end(),
        };
        out.push(StringEntry {
            name,
            value: unescape(&text[content.clone()]),
            translatable: attr("translatable").as_deref() != Some("false"),
            range: whole.range(),
            content,
        });
    }
    out
}

/// Android's escaping undone for display: entities, `\'`, `\"`, `\\`, and
/// the quotes that keep spaces. Markup (`<b>`, `<xliff:g>`) stays as written.
pub fn unescape(raw: &str) -> String {
    let has_markup = raw.contains('<');
    let mut text = if has_markup { raw.to_owned() } else { raw.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&") };
    if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') && !text.ends_with("\\\"") {
        text = text[1..text.len() - 1].to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some(e @ ('\'' | '"' | '\\' | '@' | '?')) => out.push(e),
                Some(other) => {
                    // `\n`, `\t`, `…` are shown as written.
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Text typed in the editor as a string resource: `'` and `"` escaped,
/// `&` and a `<` that starts no tag as entities, a leading `@` or `?`
/// escaped, and spaces at either end kept by quoting.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '\'' | '"' => {
                out.push('\\');
                out.push(c);
            }
            '\\' => {
                // `\n`, `\t`, `\u…` stay; a lone backslash is doubled.
                match chars.get(i + 1) {
                    Some('n' | 't' | 'u' | '\\' | '\'' | '"' | '@' | '?') => out.push('\\'),
                    _ => out.push_str("\\\\"),
                }
            }
            '&' => {
                let rest: String = chars[i..].iter().take(10).collect();
                let entity = rest.find(';').is_some_and(|end| end > 1 && rest[1..end].chars().all(|c| c.is_ascii_alphanumeric() || c == '#'));
                out.push_str(if entity { "&" } else { "&amp;" });
            }
            '<' => {
                let tag = chars.get(i + 1).is_some_and(|n| n.is_ascii_alphabetic() || *n == '/');
                out.push_str(if tag { "<" } else { "&lt;" });
            }
            '@' | '?' if i == 0 => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    if text.starts_with(' ') || text.ends_with(' ') { format!("\"{out}\"") } else { out }
}

/// What a values folder's qualifiers say about its locale.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Locale {
    pub language: String,
    pub region: Option<String>,
    pub script: Option<String>,
}

impl Locale {
    /// The locale of `values-fr`, `values-zh-rCN`, `values-b+sr+Latn`,
    /// `values-es-rUS-v21`; none for `values`, `values-night`, `values-v21`.
    pub fn of_folder(folder: &str) -> Option<Self> {
        let qualifiers = folder.strip_prefix("values-")?;
        let mut parts = qualifiers.split('-');
        let first = parts.next()?;
        if let Some(bcp) = first.strip_prefix("b+") {
            let mut tags = bcp.split('+');
            let language = tags.next()?.to_ascii_lowercase();
            let (mut region, mut script) = (None, None);
            for tag in tags {
                match tag.len() {
                    4 => script = Some(tag.to_owned()),
                    2 | 3 => region = Some(tag.to_ascii_uppercase()),
                    _ => {}
                }
            }
            return Some(Self { language, region, script });
        }
        if !(2..=3).contains(&first.len()) || !first.chars().all(|c| c.is_ascii_lowercase()) || language_name(first).is_none() {
            return None;
        }
        let region = parts.next().and_then(|r| r.strip_prefix('r')).filter(|r| r.len() == 2 && r.chars().all(|c| c.is_ascii_uppercase()) || r.len() == 3 && r.chars().all(|c| c.is_ascii_digit())).map(str::to_owned);
        Some(Self { language: first.to_owned(), region, script: None })
    }

    pub fn new(language: &str, region: Option<&str>) -> Self {
        Self { language: language.to_owned(), region: region.map(str::to_owned), script: None }
    }

    /// The folder Android Studio creates for it: `values-fr`, `values-zh-rCN`.
    pub fn folder(&self) -> String {
        match (&self.script, &self.region) {
            (Some(script), region) => format!("values-b+{}+{script}{}", self.language, region.as_ref().map(|r| format!("+{r}")).unwrap_or_default()),
            (None, Some(region)) => format!("values-{}-r{region}", self.language),
            (None, None) => format!("values-{}", self.language),
        }
    }

    /// Android Studio's column header: "French (fr)", "Chinese (zh) in
    /// China (CN)".
    pub fn label(&self) -> String {
        let language = match language_name(&self.language) {
            Some(name) => format!("{name} ({})", self.language),
            None => self.language.clone(),
        };
        let language = match &self.script {
            Some(script) => format!("{language} {script}"),
            None => language,
        };
        match &self.region {
            Some(region) => match region_name(region) {
                Some(name) => format!("{language} in {name} ({region})"),
                None => format!("{language} ({region})"),
            },
            None => language,
        }
    }
}

/// The languages Android Studio's Add Locale offers, by code.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("af", "Afrikaans"), ("am", "Amharic"), ("ar", "Arabic"), ("as", "Assamese"), ("az", "Azerbaijani"), ("be", "Belarusian"), ("bg", "Bulgarian"),
    ("bn", "Bengali"), ("bs", "Bosnian"), ("ca", "Catalan"), ("cs", "Czech"), ("cy", "Welsh"), ("da", "Danish"), ("de", "German"), ("el", "Greek"),
    ("en", "English"), ("es", "Spanish"), ("et", "Estonian"), ("eu", "Basque"), ("fa", "Persian"), ("fi", "Finnish"), ("fil", "Filipino"), ("fr", "French"),
    ("ga", "Irish"), ("gl", "Galician"), ("gu", "Gujarati"), ("ha", "Hausa"), ("he", "Hebrew"), ("hi", "Hindi"), ("hr", "Croatian"), ("hu", "Hungarian"),
    ("hy", "Armenian"), ("id", "Indonesian"), ("in", "Indonesian"), ("is", "Icelandic"), ("it", "Italian"), ("iw", "Hebrew"), ("ja", "Japanese"),
    ("ka", "Georgian"), ("kk", "Kazakh"), ("km", "Khmer"), ("kn", "Kannada"), ("ko", "Korean"), ("ky", "Kyrgyz"), ("lo", "Lao"), ("lt", "Lithuanian"),
    ("lv", "Latvian"), ("mk", "Macedonian"), ("ml", "Malayalam"), ("mn", "Mongolian"), ("mr", "Marathi"), ("ms", "Malay"), ("my", "Burmese"),
    ("nb", "Norwegian Bokmål"), ("ne", "Nepali"), ("nl", "Dutch"), ("no", "Norwegian"), ("or", "Odia"), ("pa", "Punjabi"), ("pl", "Polish"),
    ("pt", "Portuguese"), ("ro", "Romanian"), ("ru", "Russian"), ("si", "Sinhala"), ("sk", "Slovak"), ("sl", "Slovenian"), ("sq", "Albanian"),
    ("sr", "Serbian"), ("sv", "Swedish"), ("sw", "Swahili"), ("ta", "Tamil"), ("te", "Telugu"), ("th", "Thai"), ("tl", "Tagalog"), ("tr", "Turkish"),
    ("uk", "Ukrainian"), ("ur", "Urdu"), ("uz", "Uzbek"), ("vi", "Vietnamese"), ("yo", "Yoruba"), ("zh", "Chinese"), ("zu", "Zulu"),
];

const REGIONS: &[(&str, &str)] = &[
    ("AR", "Argentina"), ("AT", "Austria"), ("AU", "Australia"), ("BE", "Belgium"), ("BR", "Brazil"), ("CA", "Canada"), ("CH", "Switzerland"),
    ("CL", "Chile"), ("CN", "China"), ("CO", "Colombia"), ("CZ", "Czechia"), ("DE", "Germany"), ("DK", "Denmark"), ("EG", "Egypt"), ("ES", "Spain"),
    ("FI", "Finland"), ("FR", "France"), ("GB", "United Kingdom"), ("GR", "Greece"), ("HK", "Hong Kong"), ("ID", "Indonesia"), ("IE", "Ireland"),
    ("IL", "Israel"), ("IN", "India"), ("IT", "Italy"), ("JP", "Japan"), ("KR", "South Korea"), ("MX", "Mexico"), ("MY", "Malaysia"),
    ("NL", "Netherlands"), ("NO", "Norway"), ("NZ", "New Zealand"), ("PE", "Peru"), ("PH", "Philippines"), ("PL", "Poland"), ("PT", "Portugal"),
    ("RO", "Romania"), ("RU", "Russia"), ("SA", "Saudi Arabia"), ("SE", "Sweden"), ("SG", "Singapore"), ("TH", "Thailand"), ("TR", "Turkey"),
    ("TW", "Taiwan"), ("UA", "Ukraine"), ("US", "United States"), ("VE", "Venezuela"), ("VN", "Vietnam"), ("ZA", "South Africa"), ("419", "Latin America"),
];

pub fn language_name(code: &str) -> Option<&'static str> {
    LANGUAGES.iter().find(|(c, _)| *c == code).map(|(_, n)| *n)
}

fn region_name(code: &str) -> Option<&'static str> {
    REGIONS.iter().find(|(c, _)| *c == code).map(|(_, n)| *n)
}

/// The values file with a string set to `value` (shown text): its content
/// replaced, or a new element before `</resources>`.
pub fn set_string(text: &str, name: &str, value: &str) -> String {
    let escaped = escape(value);
    if let Some(e) = parse_strings(text).into_iter().find(|e| e.name == name) {
        let mut out = text.to_owned();
        if e.content.is_empty() && text[e.range.clone()].ends_with("/>") {
            // `<string name="x"/>` becomes `<string name="x">value</string>`.
            let open = text[e.range.start..e.range.end - 2].trim_end().to_owned();
            out.replace_range(e.range, &format!("{open}>{escaped}</string>"));
        } else {
            out.replace_range(e.content, &escaped);
        }
        return out;
    }
    insert_element(text, &format!("<string name=\"{name}\">{escaped}</string>"))
}

/// Adds an element as the last child of `<resources>`, indented as the
/// others.
fn insert_element(text: &str, element: &str) -> String {
    let Some(close) = text.rfind("</resources>") else { return new_values_file(element) };
    let indent = parse_strings(text)
        .last()
        .map(|e| text[..e.range.start].rsplit('\n').next().unwrap_or("").chars().take_while(|c| c.is_whitespace()).collect::<String>())
        .unwrap_or_else(|| "    ".to_owned());
    let line_start = text[..close].rfind('\n').map_or(0, |i| i + 1);
    let mut out = text.to_owned();
    if text[line_start..close].trim().is_empty() {
        out.insert_str(line_start, &format!("{indent}{element}\n"));
    } else {
        out.insert_str(close, &format!("\n{indent}{element}\n"));
    }
    out
}

/// A new `strings.xml` holding one element.
pub fn new_values_file(element: &str) -> String {
    format!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<resources>\n    {element}\n</resources>\n")
}

/// The values file without a string: its element and its line.
pub fn remove_string(text: &str, name: &str) -> String {
    let Some(e) = parse_strings(text).into_iter().find(|e| e.name == name) else { return text.to_owned() };
    let line_start = text[..e.range.start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[e.range.end..].find('\n').map_or(text.len(), |i| e.range.end + i + 1);
    let alone = text[line_start..e.range.start].trim().is_empty() && text[e.range.end..line_end].trim().is_empty();
    let mut out = text.to_owned();
    if alone {
        out.replace_range(line_start..line_end, "");
    } else {
        out.replace_range(e.range, "");
    }
    out
}

/// The Untranslatable checkbox: `translatable="false"` on the default
/// string, or not.
pub fn set_translatable(text: &str, name: &str, translatable: bool) -> String {
    let Some(e) = parse_strings(text).into_iter().find(|e| e.name == name) else { return text.to_owned() };
    let element = &text[e.range.clone()];
    let open_end = element.find('>').unwrap_or(element.len());
    let open = &element[..open_end];
    static TRANSLATABLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\s+translatable\s*=\s*"[^"]*""#).unwrap());
    let bare = TRANSLATABLE.replace(open, "").into_owned();
    let new_open = if translatable {
        bare
    } else {
        let at = bare.trim_end_matches('/').trim_end().len();
        format!("{} translatable=\"false\"{}", &bare[..at], &bare[at..])
    };
    let mut out = text.to_owned();
    out.replace_range(e.range.start..e.range.start + open_end, &new_open);
    out
}

/// Whether a key can be added: Android Studio's checks.
pub fn check_key(key: &str, existing: &[&str]) -> Result<(), String> {
    if key.trim().is_empty() {
        return Err("Key cannot be empty".into());
    }
    if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.') || key.starts_with(|c: char| c.is_ascii_digit()) {
        return Err(format!("'{key}' is not a valid resource name character"));
    }
    if existing.contains(&key) {
        return Err(format!("Key '{key}' already exists"));
    }
    Ok(())
}

/// The values files of one `res` folder.
pub trait ResFiles {
    /// Each `values*` folder's XML files: (folder, file name).
    fn values_files(&self) -> Vec<(String, String)>;
    fn read(&self, folder: &str, file: &str) -> Option<String>;
    /// Writes a file, creating its folder.
    fn write(&self, folder: &str, file: &str, text: &str) -> Result<(), String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    const STRINGS: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<resources>\n    <string name=\"app_name\" translatable=\"false\">Calc</string>\n    <string name=\"hello\">Hello, it\\'s &amp; me</string>\n    <!-- <string name=\"old\">Old</string> -->\n    <string name=\"empty\"/>\n    <string name=\"styled\">Hi <b>you</b></string>\n</resources>\n";

    #[test]
    fn parses_strings() {
        let entries = parse_strings(STRINGS);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["app_name", "hello", "empty", "styled"]);
        assert!(!entries[0].translatable && entries[1].translatable);
        assert_eq!(entries[1].value, "Hello, it's & me");
        assert_eq!(entries[2].value, "");
        assert_eq!(entries[3].value, "Hi <b>you</b>");
    }

    #[test]
    fn escapes_like_android() {
        assert_eq!(escape("it's \"so\" & <3"), "it\\'s \\\"so\\\" &amp; &lt;3");
        assert_eq!(escape("@home"), "\\@home");
        assert_eq!(escape(" padded "), "\" padded \"");
        assert_eq!(escape("line\\nnext"), "line\\nnext");
        assert_eq!(escape("Hi <b>you</b> &amp;"), "Hi <b>you</b> &amp;");
        for text in ["it's", "say \"hi\"", "a & b", "@x", "?y", "back\\slash"] {
            assert_eq!(unescape(&escape(text)), text);
        }
        assert_eq!(unescape("\" keep \""), " keep ");
    }

    #[test]
    fn locales_of_folders() {
        assert_eq!(Locale::of_folder("values-fr"), Some(Locale::new("fr", None)));
        assert_eq!(Locale::of_folder("values-zh-rCN"), Some(Locale::new("zh", Some("CN"))));
        assert_eq!(Locale::of_folder("values-es-rUS-v21"), Some(Locale::new("es", Some("US"))));
        assert_eq!(Locale::of_folder("values-b+sr+Latn"), Some(Locale { language: "sr".into(), region: None, script: Some("Latn".into()) }));
        for not in ["values", "values-night", "values-v21", "values-land", "values-sw600dp", "values-ldrtl"] {
            assert_eq!(Locale::of_folder(not), None, "{not}");
        }
        assert_eq!(Locale::new("fr", None).label(), "French (fr)");
        assert_eq!(Locale::new("zh", Some("CN")).label(), "Chinese (zh) in China (CN)");
        assert_eq!(Locale::new("es", Some("419")).label(), "Spanish (es) in Latin America (419)");
        assert_eq!(Locale::new("zh", Some("TW")).folder(), "values-zh-rTW");
        assert_eq!(Locale::of_folder("values-b+sr+Latn").unwrap().folder(), "values-b+sr+Latn");
    }

    #[test]
    fn edits_values_files() {
        let set = set_string(STRINGS, "hello", "Bonjour l'ami");
        assert!(set.contains("<string name=\"hello\">Bonjour l\\'ami</string>"));
        let filled = set_string(STRINGS, "empty", "Now");
        assert!(filled.contains("<string name=\"empty\">Now</string>"), "{filled}");
        let added = set_string(STRINGS, "new_key", "New");
        assert!(added.ends_with("    <string name=\"styled\">Hi <b>you</b></string>\n    <string name=\"new_key\">New</string>\n</resources>\n"), "{added}");
        // A commented out string isn't the one set.
        assert!(set_string(STRINGS, "old", "x").contains("    <string name=\"old\">x</string>\n</resources>"));
        let removed = remove_string(STRINGS, "hello");
        assert!(!removed.contains("hello") && removed.contains("<resources>\n    <string name=\"app_name\""));
        let translatable = set_translatable(STRINGS, "app_name", true);
        assert!(translatable.contains("<string name=\"app_name\">Calc</string>"));
        let untranslatable = set_translatable(STRINGS, "hello", false);
        assert!(untranslatable.contains("<string name=\"hello\" translatable=\"false\">Hello"));
        assert!(set_translatable(STRINGS, "empty", false).contains("<string name=\"empty\" translatable=\"false\"/>"));
        assert_eq!(new_values_file("<string name=\"a\">A</string>"), "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<resources>\n    <string name=\"a\">A</string>\n</resources>\n");
    }

    #[test]
    fn checks_keys() {
        assert!(check_key("", &[]).is_err());
        assert_eq!(check_key("hello", &["hello"]), Err("Key 'hello' already exists".into()));
        assert!(check_key("bad key", &[]).is_err());
        assert_eq!(check_key("good_key.2", &[]), Ok(()));
    }
}
