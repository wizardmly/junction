//! Settings › Editor › Live Templates kept across sessions: the person's
//! templates and the built-in ones turned off, in `live-templates.json` in
//! the settings folder.

use std::path::{Path, PathBuf};

use crate::domain::live_templates::CustomTemplates;

fn store_path() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("live-templates.json"))
}

/// Reads the saved templates and puts them into effect.
pub fn load() {
    if let Some(path) = store_path() {
        crate::domain::live_templates::set_custom(load_from(&path));
    }
}

/// Saves the templates and puts them into effect.
pub fn save(custom: CustomTemplates) {
    if let Some(path) = store_path() {
        save_to(&path, &custom);
    }
    crate::domain::live_templates::set_custom(custom);
}

fn load_from(path: &Path) -> CustomTemplates {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_to(path: &Path, custom: &CustomTemplates) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_vec_pretty(custom) {
        let _ = std::fs::write(path, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::live_templates::{Context, UserTemplate};

    #[test]
    fn round_trips() {
        let path = std::env::temp_dir().join(format!("junction-live-templates-{}/live-templates.json", std::process::id()));
        assert_eq!(load_from(&path), CustomTemplates::default());
        let custom = CustomTemplates {
            user: vec![UserTemplate {
                abbreviation: "logd".into(),
                description: "Log.d".into(),
                text: "Log.d(TAG, \"$MSG$\")$END$".into(),
                defaults: vec![("MSG".into(), "\"here\"".into())],
                expressions: vec![("MSG".into(), "methodName()".into())],
                skip: vec!["MSG".into()],
                kotlin: vec![Context::Statement],
                java: vec![],
                enabled: true,
            }],
            disabled: vec![CustomTemplates::key(true, "sout")],
        };
        save_to(&path, &custom);
        assert_eq!(load_from(&path), custom);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
