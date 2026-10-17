//! The Translations Editor's table, as Android Studio builds it: every
//! `<string>` key of a res folder with its default value and each locale's
//! translation, and the edits it makes to the values files (a translation
//! set or cleared, a key added or removed, Untranslatable, a locale added).

use std::collections::BTreeMap;

use crate::domain::translations::{self as tr, Locale};

pub use crate::domain::translations::ResFiles;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub key: String,
    pub translatable: bool,
    pub default: Option<String>,
    /// The translation in each of the table's locales.
    pub values: Vec<Option<String>>,
}

impl Row {
    /// A translatable key without this locale's translation: red.
    pub fn missing(&self, locale: usize) -> bool {
        self.translatable && self.values.get(locale).is_some_and(Option::is_none)
    }

    pub fn needs_translation(&self) -> bool {
        (0..self.values.len()).any(|l| self.missing(l))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Table {
    /// Sorted by label, as the columns are.
    pub locales: Vec<Locale>,
    /// Sorted by key.
    pub rows: Vec<Row>,
}

/// Reads the folder's strings into the table.
pub fn load(files: &dyn ResFiles) -> Table {
    let mut defaults: BTreeMap<String, (String, bool)> = BTreeMap::new();
    let mut translations: BTreeMap<Locale, BTreeMap<String, String>> = BTreeMap::new();
    for (folder, file) in files.values_files() {
        let locale = Locale::of_folder(&folder);
        if locale.is_none() && folder != "values" {
            continue;
        }
        let Some(text) = files.read(&folder, &file) else { continue };
        for e in tr::parse_strings(&text) {
            match &locale {
                None => {
                    defaults.entry(e.name).or_insert((e.value, e.translatable));
                }
                Some(locale) => {
                    translations.entry(locale.clone()).or_default().entry(e.name).or_insert(e.value);
                }
            }
        }
    }
    let mut locales: Vec<Locale> = translations.keys().cloned().collect();
    locales.sort_by_key(|l| l.label());
    let mut keys: Vec<String> = defaults.keys().cloned().collect();
    for map in translations.values() {
        keys.extend(map.keys().filter(|k| !defaults.contains_key(*k)).cloned());
    }
    keys.sort();
    keys.dedup();
    let rows = keys
        .into_iter()
        .map(|key| {
            let (default, translatable) = match defaults.get(&key) {
                Some((value, translatable)) => (Some(value.clone()), *translatable),
                None => (None, true),
            };
            let values = locales.iter().map(|l| translations.get(l).and_then(|m| m.get(&key)).cloned()).collect();
            Row { key, translatable, default, values }
        })
        .collect();
    Table { locales, rows }
}

/// The file of a folder that declares a key.
fn file_with(files: &dyn ResFiles, folder: &str, key: &str) -> Option<String> {
    files.values_files().into_iter().filter(|(f, _)| f == folder).map(|(_, file)| file).find(|file| files.read(folder, file).is_some_and(|text| tr::parse_strings(&text).iter().any(|e| e.name == key)))
}

/// The folder a column (none: Default Value) writes to: the one it was
/// read from when there is one.
fn folder_of(files: &dyn ResFiles, locale: Option<&Locale>) -> String {
    let Some(locale) = locale else { return "values".into() };
    let mut existing: Vec<String> = files.values_files().into_iter().map(|(f, _)| f).filter(|f| Locale::of_folder(f).as_ref() == Some(locale)).collect();
    existing.sort_by_key(|f| f.len());
    existing.into_iter().next().unwrap_or_else(|| locale.folder())
}

/// A cell edited: the value written where the key is (else into the
/// folder's `strings.xml`); an empty translation is removed, as Android
/// Studio does.
pub fn set_value(files: &dyn ResFiles, key: &str, locale: Option<&Locale>, value: &str) -> Result<(), String> {
    let folder = folder_of(files, locale);
    let found = file_with(files, &folder, key);
    if value.is_empty() && locale.is_some() {
        let Some(file) = found else { return Ok(()) };
        let text = files.read(&folder, &file).unwrap_or_default();
        return files.write(&folder, &file, &tr::remove_string(&text, key));
    }
    let file = found.unwrap_or_else(|| "strings.xml".to_owned());
    let text = files.read(&folder, &file);
    let new = match text {
        Some(text) => tr::set_string(&text, key, value),
        None => tr::set_string(&tr::new_values_file("").replace("    \n", ""), key, value),
    };
    files.write(&folder, &file, &new)
}

/// Add Key: the default value in the folder's `strings.xml`.
pub fn add_key(files: &dyn ResFiles, key: &str, default: &str) -> Result<(), String> {
    let existing = load(files);
    let keys: Vec<&str> = existing.rows.iter().map(|r| r.key.as_str()).collect();
    tr::check_key(key, &keys)?;
    set_value(files, key, None, default)
}

/// Remove Keys: from every values folder.
pub fn remove_keys(files: &dyn ResFiles, keys: &[String]) -> Result<(), String> {
    for (folder, file) in files.values_files() {
        let Some(text) = files.read(&folder, &file) else { continue };
        let new = keys.iter().fold(text.clone(), |t, k| tr::remove_string(&t, k));
        if new != text {
            files.write(&folder, &file, &new)?;
        }
    }
    Ok(())
}

/// The Untranslatable checkbox, on the default string.
pub fn set_translatable(files: &dyn ResFiles, key: &str, translatable: bool) -> Result<(), String> {
    let Some(file) = file_with(files, "values", key) else { return Err(format!("'{key}' has no default value")) };
    let text = files.read("values", &file).unwrap_or_default();
    files.write("values", &file, &tr::set_translatable(&text, key, translatable))
}

/// Add Locale: the folder with one string so it isn't empty, as Android
/// Studio adds `app_name` (else the first translatable key) with its
/// default value; an untranslatable `app_name` isn't copied.
pub fn add_locale(files: &dyn ResFiles, locale: &Locale) -> Result<(), String> {
    let table = load(files);
    if table.locales.contains(locale) {
        return Ok(());
    }
    let seed = table.rows.iter().find(|r| r.key == "app_name" && r.translatable && r.default.is_some()).or_else(|| table.rows.iter().find(|r| r.translatable && r.default.is_some()));
    let Some(row) = seed else { return Err("There are no strings to translate".into()) };
    let element = format!("<string name=\"{}\">{}</string>", row.key, tr::escape(row.default.as_deref().unwrap_or("")));
    files.write(&locale.folder(), "strings.xml", &tr::new_values_file(&element))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Fake(RefCell<BTreeMap<(String, String), String>>);

    impl Fake {
        fn with(files: &[(&str, &str, &str)]) -> Self {
            let fake = Fake::default();
            for (folder, file, text) in files {
                fake.0.borrow_mut().insert((folder.to_string(), file.to_string()), text.to_string());
            }
            fake
        }
        fn text(&self, folder: &str, file: &str) -> String {
            self.0.borrow().get(&(folder.into(), file.into())).cloned().unwrap_or_default()
        }
    }

    impl ResFiles for Fake {
        fn values_files(&self) -> Vec<(String, String)> {
            self.0.borrow().keys().cloned().collect()
        }
        fn read(&self, folder: &str, file: &str) -> Option<String> {
            self.0.borrow().get(&(folder.into(), file.into())).cloned()
        }
        fn write(&self, folder: &str, file: &str, text: &str) -> Result<(), String> {
            self.0.borrow_mut().insert((folder.into(), file.into()), text.into());
            Ok(())
        }
    }

    fn file(body: &str) -> String {
        tr::new_values_file(body)
    }

    fn project() -> Fake {
        Fake::with(&[
            ("values", "strings.xml", &file("<string name=\"app_name\" translatable=\"false\">Calc</string>\n    <string name=\"hello\">Hello</string>\n    <string name=\"bye\">Bye</string>")),
            ("values", "colors.xml", &file("<color name=\"red\">#f00</color>")),
            ("values-fr", "strings.xml", &file("<string name=\"hello\">Bonjour</string>")),
            ("values-zh-rCN", "strings.xml", &file("<string name=\"hello\">你好</string>\n    <string name=\"bye\">再见</string>\n    <string name=\"extra\">多</string>")),
            ("values-night", "strings.xml", &file("<string name=\"night\">x</string>")),
        ])
    }

    #[test]
    fn loads_the_table() {
        let table = load(&project());
        let labels: Vec<String> = table.locales.iter().map(Locale::label).collect();
        assert_eq!(labels, ["Chinese (zh) in China (CN)", "French (fr)"]);
        let keys: Vec<&str> = table.rows.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, ["app_name", "bye", "extra", "hello"]);
        let bye = &table.rows[1];
        assert_eq!((bye.default.as_deref(), bye.values.clone()), (Some("Bye"), vec![Some("再见".to_owned()), None]));
        assert!(bye.missing(1) && !bye.missing(0) && bye.needs_translation());
        // Untranslatable keys need nothing.
        assert!(!table.rows[0].needs_translation());
        // A translation without a default.
        assert_eq!(table.rows[2].default, None);
    }

    #[test]
    fn edits_the_files() {
        let files = project();
        let fr = Locale::new("fr", None);
        set_value(&files, "bye", Some(&fr), "Au revoir").unwrap();
        assert!(files.text("values-fr", "strings.xml").contains("<string name=\"bye\">Au revoir</string>"));
        // Cleared: removed.
        set_value(&files, "hello", Some(&fr), "").unwrap();
        assert!(!files.text("values-fr", "strings.xml").contains("hello"));
        // A locale without a folder yet gets one.
        set_value(&files, "hello", Some(&Locale::new("de", None)), "Hallo").unwrap();
        assert_eq!(files.text("values-de", "strings.xml"), file("<string name=\"hello\">Hallo</string>"));
        set_value(&files, "hello", None, "Hi there").unwrap();
        assert!(files.text("values", "strings.xml").contains("<string name=\"hello\">Hi there</string>"));
        assert_eq!(add_key(&files, "hello", "x"), Err("Key 'hello' already exists".into()));
        add_key(&files, "welcome", "Welcome").unwrap();
        assert!(load(&files).rows.iter().any(|r| r.key == "welcome" && r.default.as_deref() == Some("Welcome")));
        set_translatable(&files, "app_name", true).unwrap();
        assert!(load(&files).rows[0].translatable);
        remove_keys(&files, &["hello".into()]).unwrap();
        assert!(!files.text("values-zh-rCN", "strings.xml").contains("hello") && !files.text("values", "strings.xml").contains("hello"));
    }

    #[test]
    fn adds_a_locale_with_app_name() {
        let files = project();
        add_locale(&files, &Locale::new("es", Some("419"))).unwrap();
        // app_name is untranslatable here: the first translatable key.
        assert_eq!(files.text("values-es-r419", "strings.xml"), file("<string name=\"bye\">Bye</string>"));
        set_translatable(&files, "app_name", true).unwrap();
        add_locale(&files, &Locale::new("de", None)).unwrap();
        assert_eq!(files.text("values-de", "strings.xml"), file("<string name=\"app_name\">Calc</string>"));
        assert_eq!(load(&files).locales.len(), 4);
    }
}
