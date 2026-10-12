//! The AVDs in the AVD folder: `<name>.ini` pointing at `<name>.avd/` with
//! its `config.ini`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::FormFactor;

/// `key=value` lines, as the emulator writes them.
pub fn parse_ini(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Avd {
    /// The AVD's id: what `emulator -avd` takes.
    pub name: String,
    pub display_name: String,
    /// The `.ini` beside the folder.
    pub ini: PathBuf,
    /// The `.avd` folder.
    pub dir: PathBuf,
    pub api: Option<u32>,
    /// A preview's codename (`android-Baklava`).
    pub codename: Option<String>,
    pub abi: Option<String>,
    pub form: FormFactor,
    /// config.ini, for View Details.
    pub config: BTreeMap<String, String>,
}

impl Avd {
    pub fn size_px(&self) -> Option<(u32, u32)> {
        Some((self.config.get("hw.lcd.width")?.parse().ok()?, self.config.get("hw.lcd.height")?.parse().ok()?))
    }

    pub fn density(&self) -> Option<u32> {
        self.config.get("hw.lcd.density")?.parse().ok()
    }
}

/// `android-34`, `system-images/android-34/google_apis/x86_64/`,
/// `android-34-ext10` → 34; `android-Baklava` → the codename.
fn api_of(text: &str) -> (Option<u32>, Option<String>) {
    let Some(start) = text.find("android-") else { return (None, None) };
    let rest = &text[start + "android-".len()..];
    let token: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
    match token.parse() {
        Ok(api) => (Some(api), None),
        Err(_) if !token.is_empty() => (None, Some(token)),
        Err(_) => (None, None),
    }
}

/// The device kind from the system image's tags.
pub fn form_factor(tags: &str, device: &str) -> FormFactor {
    let tags = tags.to_ascii_lowercase();
    if tags.contains("wear") {
        FormFactor::Wear
    } else if tags.contains("android-tv") || tags.contains("google-tv") {
        FormFactor::Tv
    } else if tags.contains("automotive") {
        FormFactor::Automotive
    } else if tags.contains("android-desktop") {
        FormFactor::Desktop
    } else if tags.contains("xr") {
        FormFactor::Xr
    } else if device.to_ascii_lowercase().contains("tablet") {
        FormFactor::Tablet
    } else {
        FormFactor::Phone
    }
}

pub fn load(ini: &Path, avd_home: &Path) -> Option<Avd> {
    let name = ini.file_stem()?.to_string_lossy().into_owned();
    let outer = parse_ini(&std::fs::read_to_string(ini).ok()?);
    let dir = outer
        .get("path")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .or_else(|| outer.get("path.rel").map(|rel| avd_home.parent().unwrap_or(avd_home).join(rel)).filter(|p| p.is_dir()))
        .unwrap_or_else(|| avd_home.join(format!("{name}.avd")));
    let config = std::fs::read_to_string(dir.join("config.ini")).map(|t| parse_ini(&t)).unwrap_or_default();
    let (mut api, mut codename) = outer.get("target").map(|t| api_of(t)).unwrap_or((None, None));
    if api.is_none() && codename.is_none() {
        (api, codename) = config.get("image.sysdir.1").map(|t| api_of(t)).unwrap_or((None, None));
    }
    let tags = config.get("tag.ids").or_else(|| config.get("tag.id")).cloned().unwrap_or_default();
    let form = form_factor(&tags, config.get("hw.device.name").map_or("", String::as_str));
    let display_name = config.get("avd.ini.displayname").cloned().filter(|n| !n.is_empty()).unwrap_or_else(|| name.replace('_', " "));
    let abi = config.get("abi.type").cloned();
    Some(Avd { name, display_name, ini: ini.to_path_buf(), dir, api, codename, abi, form, config })
}

/// Every AVD in the folder, by name.
pub fn list(avd_home: &Path) -> Vec<Avd> {
    let Ok(entries) = std::fs::read_dir(avd_home) else { return Vec::new() };
    let mut avds: Vec<Avd> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "ini"))
        .filter_map(|p| load(&p, avd_home))
        .collect();
    avds.sort_by(|a, b| a.name.cmp(&b.name));
    avds
}

/// Wipe Data: the user data image and the snapshots go; the emulator
/// makes a fresh user data from the system image on the next boot.
pub fn wipe_data(avd: &Avd) -> std::io::Result<()> {
    for file in ["userdata-qemu.img", "userdata-qemu.img.qcow2"] {
        let path = avd.dir.join(file);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    let snapshots = avd.dir.join("snapshots");
    if snapshots.exists() {
        std::fs::remove_dir_all(snapshots)?;
    }
    Ok(())
}

/// Delete: the `.avd` folder and its `.ini`.
pub fn delete(avd: &Avd) -> std::io::Result<()> {
    if avd.dir.exists() {
        std::fs::remove_dir_all(&avd.dir)?;
    }
    if avd.ini.exists() {
        std::fs::remove_file(&avd.ini)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_an_avd() {
        let home = std::env::temp_dir().join(format!("junction-avd-test-{}", std::process::id()));
        let dir = home.join("Pixel_7_API_34.avd");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(home.join("Pixel_7_API_34.ini"), format!("avd.ini.encoding=UTF-8\npath={}\npath.rel=avd/Pixel_7_API_34.avd\ntarget=android-34\n", dir.display())).unwrap();
        std::fs::write(
            dir.join("config.ini"),
            "avd.ini.displayname=Pixel 7 API 34\nabi.type=x86_64\nhw.device.name=pixel_7\nhw.lcd.width=1080\nhw.lcd.height=2400\nhw.lcd.density=420\nimage.sysdir.1=system-images/android-34/google_apis/x86_64/\ntag.id=google_apis\n",
        )
        .unwrap();
        std::fs::write(home.join("Wear.ini"), "path=/nowhere\ntarget=android-33\n").unwrap();
        let avds = list(&home);
        std::fs::remove_dir_all(&home).ok();
        assert_eq!(avds.len(), 2);
        let pixel = &avds[0];
        assert_eq!((pixel.display_name.as_str(), pixel.api, pixel.abi.as_deref(), pixel.form), ("Pixel 7 API 34", Some(34), Some("x86_64"), FormFactor::Phone));
        assert_eq!((pixel.size_px(), pixel.density()), (Some((1080, 2400)), Some(420)));
        assert_eq!((avds[1].display_name.as_str(), avds[1].api), ("Wear", Some(33)));
    }

    #[test]
    fn api_levels_and_kinds() {
        assert_eq!(api_of("system-images/android-35/google_apis_playstore/arm64-v8a/"), (Some(35), None));
        assert_eq!(api_of("android-34-ext10"), (Some(34), None));
        assert_eq!(api_of("android-Baklava"), (None, Some("Baklava".into())));
        assert_eq!(form_factor("android-wear", ""), FormFactor::Wear);
        assert_eq!(form_factor("google_apis,google-tv", ""), FormFactor::Tv);
        assert_eq!(form_factor("android-automotive-playstore", ""), FormFactor::Automotive);
        assert_eq!(form_factor("google_apis", "pixel_tablet"), FormFactor::Tablet);
    }
}
