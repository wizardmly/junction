//! File › Project Structure… (Android Studio's PSD) over one Gradle build:
//! reads what its SDK Location, Project and Modules pages show, and writes
//! the edited values back into the files they came from.

use crate::domain::project_files::ProjectFiles;
use crate::domain::project_structure::{self as ps, Place, Property, Site};
use crate::domain::version_catalog::Catalog;

pub const LOCAL_PROPERTIES: &str = "local.properties";
pub const WRAPPER_PROPERTIES: &str = "gradle/wrapper/gradle-wrapper.properties";

/// A module's page.
#[derive(Clone, Debug)]
pub struct ModulePage {
    /// ":app".
    pub path: String,
    /// Its build script, relative to the build root; none without one.
    pub script: Option<String>,
    pub properties: Vec<Property>,
    pub default_config: Vec<Property>,
}

/// What the dialog shows, and where each value is written.
#[derive(Clone, Debug)]
pub struct Structure {
    pub sdk_dir: String,
    /// The Android Gradle Plugin version, and each place it's written.
    pub agp: Option<String>,
    agp_sites: Vec<(Site, String)>,
    pub gradle: Option<String>,
    pub modules: Vec<ModulePage>,
    root_script: Option<String>,
    settings: Option<String>,
    /// The catalogs' paths, by index.
    catalogs: Vec<String>,
}

/// What the person changed: a value per page, `None` when untouched.
#[derive(Clone, Debug, Default)]
pub struct Edits {
    pub sdk_dir: Option<String>,
    pub agp: Option<String>,
    pub gradle: Option<String>,
    /// (module index, label, value).
    pub modules: Vec<(usize, &'static str, String)>,
}

impl Edits {
    pub fn is_empty(&self) -> bool {
        self.sdk_dir.is_none() && self.agp.is_none() && self.gradle.is_none() && self.modules.is_empty()
    }
}

/// `build.gradle.kts`, else `build.gradle`, in a directory.
fn script_in(files: &dyn ProjectFiles, dir: &str, base: &str) -> Option<(String, String)> {
    ["kts", ""].iter().find_map(|ext| {
        let name = if ext.is_empty() { format!("{base}.gradle") } else { format!("{base}.gradle.{ext}") };
        let path = if dir.is_empty() { name } else { format!("{dir}/{name}") };
        files.read(&path).ok().map(|text| (path, text))
    })
}

/// `modules`: each module's path and directory relative to the build root.
pub fn read(files: &dyn ProjectFiles, modules: &[(String, String)], catalogs: Vec<Catalog>) -> Structure {
    let catalogs: Vec<(Catalog, String)> = catalogs.into_iter().filter_map(|c| files.read(&c.path).ok().map(|text| (c, text))).collect();
    let sdk_dir = files.read(LOCAL_PROPERTIES).ok().and_then(|text| ps::sdk_dir(&text)).unwrap_or_default();
    let root = script_in(files, "", "build");
    let settings = script_in(files, "", "settings");
    let agp_sites = ps::agp_sites(root.as_ref().map_or("", |(_, t)| t), settings.as_ref().map_or("", |(_, t)| t), &catalogs);
    let gradle = files.read(WRAPPER_PROPERTIES).ok().and_then(|text| crate::domain::build::wrapper_version(&text));
    let modules = modules
        .iter()
        .map(|(path, dir)| {
            let script = script_in(files, dir, "build");
            let (properties, default_config) = ps::module_properties(script.as_ref().map_or("", |(_, t)| t), &catalogs);
            ModulePage { path: path.clone(), script: script.map(|(p, _)| p), properties, default_config }
        })
        .collect();
    Structure {
        sdk_dir,
        agp: agp_sites.first().map(|(_, v)| v.clone()),
        agp_sites,
        gradle,
        modules,
        root_script: root.map(|(p, _)| p),
        settings: settings.map(|(p, _)| p),
        catalogs: catalogs.into_iter().map(|(c, _)| c.path).collect(),
    }
}

impl Structure {
    fn file_of(&self, place: Place, script: Option<&String>) -> Option<String> {
        match place {
            Place::Script => script.cloned(),
            Place::Settings => self.settings.clone(),
            Place::Catalog(ix) => self.catalogs.get(ix).cloned(),
        }
    }

    /// A row of a module's page by its label.
    pub fn property(&self, module: usize, label: &str) -> Option<&Property> {
        let page = self.modules.get(module)?;
        page.properties.iter().chain(&page.default_config).find(|p| p.label == label)
    }
}

/// Why a value can't be written; the first problem found.
pub fn check(structure: &Structure, edits: &Edits) -> Result<(), String> {
    for (module, label, value) in &edits.modules {
        let Some(site) = structure.property(*module, label).and_then(|p| p.site.as_ref()) else { continue };
        ps::check(site, value).map_err(|e| format!("{label}: {e}"))?;
    }
    for (label, value) in [("Android Gradle Plugin Version", &edits.agp), ("Gradle Version", &edits.gradle)] {
        if value.as_deref().is_some_and(|v| v.trim().is_empty() || v.trim().contains(char::is_whitespace)) {
            return Err(format!("{label}: enter a version."));
        }
    }
    Ok(())
}

/// Writes the edits; the files it changed, relative to the build root.
pub fn apply(files: &dyn ProjectFiles, structure: &Structure, edits: &Edits) -> Result<Vec<String>, String> {
    check(structure, edits)?;
    // Each file's sites and values, written from the end so earlier ranges
    // stay put.
    let mut writes: Vec<(String, Vec<(Site, String)>)> = Vec::new();
    let mut add = |file: String, site: Site, value: String| {
        let sites = match writes.iter_mut().find(|(f, _)| *f == file) {
            Some((_, sites)) => sites,
            None => {
                writes.push((file, Vec::new()));
                &mut writes.last_mut().unwrap().1
            }
        };
        // Two modules naming one catalog version write it once.
        if !sites.iter().any(|(s, _)| s.range == site.range) {
            sites.push((site, value));
        }
    };
    for (module, label, value) in &edits.modules {
        let Some(page) = structure.modules.get(*module) else { continue };
        let Some(site) = structure.property(*module, label).and_then(|p| p.site.clone()) else { continue };
        if let Some(file) = structure.file_of(site.place, page.script.as_ref()) {
            add(file, site, value.trim().to_owned());
        }
    }
    if let Some(agp) = &edits.agp {
        for (site, _) in &structure.agp_sites {
            if let Some(file) = structure.file_of(site.place, structure.root_script.as_ref()) {
                add(file, site.clone(), agp.trim().to_owned());
            }
        }
    }
    let mut changed = Vec::new();
    for (file, mut sites) in writes {
        let mut text = files.read(&file)?;
        sites.sort_by_key(|(s, _)| std::cmp::Reverse(s.range.start));
        for (site, value) in &sites {
            text = ps::write(&text, site, value);
        }
        files.write(&file, &text)?;
        changed.push(file);
    }
    if let Some(version) = &edits.gradle {
        let text = files.read(WRAPPER_PROPERTIES)?;
        let text = ps::set_wrapper_version(&text, version).ok_or_else(|| format!("{WRAPPER_PROPERTIES} has no distributionUrl."))?;
        files.write(WRAPPER_PROPERTIES, &text)?;
        changed.push(WRAPPER_PROPERTIES.into());
    }
    if let Some(dir) = &edits.sdk_dir {
        let text = files.read(LOCAL_PROPERTIES).unwrap_or_default();
        files.write(LOCAL_PROPERTIES, &ps::set_sdk_dir(&text, dir))?;
        changed.push(LOCAL_PROPERTIES.into());
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::*;

    struct Files(RefCell<HashMap<String, String>>);

    impl ProjectFiles for Files {
        fn read(&self, path: &str) -> Result<String, String> {
            self.0.borrow().get(path).cloned().ok_or_else(|| format!("{path}: not found"))
        }
        fn write(&self, path: &str, text: &str) -> Result<(), String> {
            self.0.borrow_mut().insert(path.into(), text.into());
            Ok(())
        }
        fn rename(&self, _: &str, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn list(&self) -> Vec<String> {
            self.0.borrow().keys().cloned().collect()
        }
    }

    const CATALOG: &str = "[versions]\nagp = \"8.5.0\"\nminSdk = \"24\"\n[plugins]\nandroid-application = { id = \"com.android.application\", version.ref = \"agp\" }\n";

    fn project() -> Files {
        let files = [
            ("settings.gradle.kts", "include(\":app\", \":lib\")\n"),
            ("build.gradle.kts", "plugins { alias(libs.plugins.android.application) apply false }\n"),
            ("app/build.gradle.kts", "android {\n    compileSdk = 34\n    defaultConfig {\n        minSdk = libs.versions.minSdk.get().toInt()\n        versionName = \"1.0\"\n    }\n}\n"),
            ("lib/build.gradle", "android {\n    compileSdkVersion 34\n    defaultConfig {\n        minSdkVersion libs.versions.minSdk.get().toInt()\n    }\n}\n"),
            ("gradle/libs.versions.toml", CATALOG),
            (WRAPPER_PROPERTIES, "distributionUrl=https\\://services.gradle.org/distributions/gradle-8.7-bin.zip\n"),
            (LOCAL_PROPERTIES, "sdk.dir=/opt/android-sdk\n"),
        ];
        Files(RefCell::new(files.iter().map(|(p, t)| (p.to_string(), t.to_string())).collect()))
    }

    fn structure(files: &Files) -> Structure {
        let catalogs = vec![Catalog::parse("libs", "gradle/libs.versions.toml", CATALOG)];
        read(files, &[(":app".into(), "app".into()), (":lib".into(), "lib".into())], catalogs)
    }

    #[test]
    fn reads_every_page() {
        let files = project();
        let s = structure(&files);
        assert_eq!(s.sdk_dir, "/opt/android-sdk");
        assert_eq!((s.agp.as_deref(), s.gradle.as_deref()), (Some("8.5.0"), Some("8.7")));
        assert_eq!(s.modules[1].script.as_deref(), Some("lib/build.gradle"));
        assert_eq!(s.property(0, "Min SDK Version").unwrap().value, "24");
        assert_eq!(s.property(1, "Compile Sdk Version").unwrap().value, "34");
    }

    #[test]
    fn writes_back_where_values_came_from() {
        let files = project();
        let s = structure(&files);
        let edits = Edits {
            sdk_dir: Some("/sdk".into()),
            agp: Some("8.7.2".into()),
            gradle: Some("8.11.1".into()),
            modules: vec![(0, "Compile Sdk Version", "35".into()), (0, "Min SDK Version", "26".into()), (1, "Min SDK Version", "26".into()), (0, "Version Name", "1.1".into())],
        };
        let mut changed = apply(&files, &s, &edits).unwrap();
        changed.sort();
        assert_eq!(changed, ["app/build.gradle.kts", "gradle/libs.versions.toml", WRAPPER_PROPERTIES, LOCAL_PROPERTIES]);
        let read = |p: &str| files.read(p).unwrap();
        assert!(read("app/build.gradle.kts").contains("compileSdk = 35\n"));
        assert!(read("app/build.gradle.kts").contains("versionName = \"1.1\""));
        assert_eq!(read("gradle/libs.versions.toml"), CATALOG.replace("8.5.0", "8.7.2").replace("\"24\"", "\"26\""));
        assert!(read(WRAPPER_PROPERTIES).contains("gradle-8.11.1-bin.zip"));
        assert_eq!(read(LOCAL_PROPERTIES), "sdk.dir=/sdk\n");
        let bad = Edits { modules: vec![(0, "Compile Sdk Version", "x".into())], ..Default::default() };
        assert!(apply(&files, &s, &bad).unwrap_err().starts_with("Compile Sdk Version"));
    }
}
