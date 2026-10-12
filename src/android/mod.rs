//! Android devices for the Device Manager tool window: the AVDs on disk,
//! the devices adb sees, starting and stopping emulators, and Wi-Fi
//! pairing, all through the SDK's own command-line tools.

pub mod adb;
pub mod avd;
pub mod pairing;
pub mod process;
pub mod sdk;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use adb::{Adb, AdbDevice};
use avd::Avd;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FormFactor {
    Phone,
    Tablet,
    Wear,
    Tv,
    Automotive,
    Desktop,
    Xr,
}

impl FormFactor {
    /// Group By › Form Factor.
    pub fn label(self) -> &'static str {
        match self {
            FormFactor::Phone => "Phone",
            FormFactor::Tablet => "Tablet",
            FormFactor::Wear => "Wear OS",
            FormFactor::Tv => "TV",
            FormFactor::Automotive => "Automotive",
            FormFactor::Desktop => "Desktop",
            FormFactor::Xr => "XR",
        }
    }

    /// `ro.build.characteristics` of a physical device.
    fn from_characteristics(text: &str) -> Self {
        let text = text.to_ascii_lowercase();
        if text.contains("watch") {
            FormFactor::Wear
        } else if text.contains("tv") {
            FormFactor::Tv
        } else if text.contains("automotive") {
            FormFactor::Automotive
        } else if text.contains("tablet") {
            FormFactor::Tablet
        } else {
            FormFactor::Phone
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Virtual,
    Physical,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Virtual => "Virtual",
            Kind::Physical => "Physical",
        }
    }
}

/// One row of the Device Manager.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    /// `avd:<name>` or `adb:<serial>`; stays the same while it starts and stops.
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub form: FormFactor,
    pub api: Option<u32>,
    pub codename: Option<String>,
    pub abi: Option<String>,
    /// The adb serial while adb sees it.
    pub serial: Option<String>,
    pub avd: Option<Avd>,
    /// adb sees it and it finished booting.
    pub online: bool,
    /// adb sees it but it is still booting.
    pub booting: bool,
    /// Why it can't be used (unauthorized, no permissions, offline).
    pub error: Option<String>,
    pub props: HashMap<String, String>,
}

impl Device {
    /// The API column: `34`, or a preview's codename.
    pub fn api_label(&self) -> String {
        self.api.map(|a| a.to_string()).or_else(|| self.codename.clone()).unwrap_or_default()
    }

    /// The second line under the name: `Android 14.0 ("UpsideDownCake") | x86_64`.
    pub fn version_label(&self) -> String {
        let version = match (self.api, &self.codename) {
            (Some(api), _) => release_name(api),
            (None, Some(codename)) => format!("Android {codename} Preview"),
            (None, None) => return String::new(),
        };
        match self.abi.as_deref().map(cpu_arch) {
            Some(arch) => format!("{version} | {arch}"),
            None => version,
        }
    }
}

/// Android's release name for an API level, as Android Studio labels it.
pub fn release_name(api: u32) -> String {
    let (version, codename) = match api {
        16 => ("4.1", "Jelly Bean"),
        17 => ("4.2", "Jelly Bean"),
        18 => ("4.3", "Jelly Bean"),
        19 => ("4.4", "KitKat"),
        20 => ("4.4W", "KitKat Wear"),
        21 => ("5.0", "Lollipop"),
        22 => ("5.1", "Lollipop"),
        23 => ("6.0", "Marshmallow"),
        24 => ("7.0", "Nougat"),
        25 => ("7.1.1", "Nougat"),
        26 => ("8.0", "Oreo"),
        27 => ("8.1", "Oreo"),
        28 => ("9.0", "Pie"),
        29 => ("10.0", "Q"),
        30 => ("11.0", "R"),
        31 => ("12.0", "S"),
        32 => ("12L", "Sv2"),
        33 => ("13.0", "Tiramisu"),
        34 => ("14.0", "UpsideDownCake"),
        35 => ("15.0", "VanillaIceCream"),
        36 => ("16.0", "Baklava"),
        _ => return format!("API {api}"),
    };
    format!("Android {version} (\"{codename}\")")
}

/// The CPU an ABI runs on: arm64-v8a → arm64.
pub fn cpu_arch(abi: &str) -> &str {
    match abi {
        "arm64-v8a" => "arm64",
        "armeabi-v7a" | "armeabi" => "arm",
        other => other,
    }
}

/// "Google Pixel 7": the manufacturer, capitalized, unless the model
/// already starts with it.
pub fn physical_name(props: &HashMap<String, String>, fallback: &str) -> String {
    let model = props.get("ro.product.model").map(|s| s.trim()).filter(|s| !s.is_empty());
    let maker = props.get("ro.product.manufacturer").map(|s| s.trim()).filter(|s| !s.is_empty());
    match (maker, model) {
        (Some(maker), Some(model)) if model.to_lowercase().starts_with(&maker.to_lowercase()) => model.to_owned(),
        (Some(maker), Some(model)) => {
            let mut chars = maker.chars();
            let maker: String = chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default();
            format!("{maker} {model}")
        }
        (None, Some(model)) => model.to_owned(),
        _ => fallback.to_owned(),
    }
}

/// What adb learned about each serial, kept between scans: properties are
/// read once a device finished booting.
#[derive(Default)]
pub struct ScanCache {
    props: HashMap<String, HashMap<String, String>>,
    avd_names: HashMap<String, String>,
}

/// The SDK tools a scan uses.
#[derive(Clone, Debug, Default)]
pub struct Tools {
    pub sdk: Option<PathBuf>,
    pub adb: Option<Adb>,
    pub emulator: Option<PathBuf>,
}

impl Tools {
    pub fn locate(project: Option<&Path>) -> Self {
        let sdk = sdk::locate(project);
        let adb = sdk::adb(sdk.as_deref()).map(|path| Adb { path });
        let emulator = sdk::emulator(sdk.as_deref());
        Self { sdk, adb, emulator }
    }
}

/// Every AVD (running or not) and every device adb sees.
pub fn scan(tools: &Tools, cache: &mut ScanCache) -> (Vec<Device>, Option<String>) {
    let avds = sdk::avd_home().map(|home| avd::list(&home)).unwrap_or_default();
    let (adb_devices, adb_error) = match &tools.adb {
        Some(adb) => match adb.devices() {
            Ok(devices) => (devices, None),
            Err(e) => (Vec::new(), Some(format!("adb: {e}"))),
        },
        None => (Vec::new(), None),
    };
    cache.props.retain(|serial, _| adb_devices.iter().any(|d| &d.serial == serial));
    cache.avd_names.retain(|serial, _| adb_devices.iter().any(|d| &d.serial == serial));
    let mut by_avd: HashMap<String, (AdbDevice, HashMap<String, String>)> = HashMap::new();
    let mut physical = Vec::new();
    for device in adb_devices {
        let props = match (&tools.adb, device.state.as_str()) {
            (Some(adb), "device") => match cache.props.get(&device.serial) {
                Some(props) => props.clone(),
                None => {
                    let props = adb.getprop(&device.serial).unwrap_or_default();
                    // Kept once booted; a booting device is asked again.
                    if props.get("sys.boot_completed").is_some_and(|v| v == "1") {
                        cache.props.insert(device.serial.clone(), props.clone());
                    }
                    props
                }
            },
            _ => HashMap::new(),
        };
        if device.is_emulator() {
            let name = match cache.avd_names.get(&device.serial) {
                Some(name) => Some(name.clone()),
                None => {
                    let name = tools.adb.as_ref().and_then(|adb| adb.avd_name(&device.serial, &props));
                    if let Some(name) = &name {
                        cache.avd_names.insert(device.serial.clone(), name.clone());
                    }
                    name
                }
            };
            if let Some(name) = name.filter(|n| avds.iter().any(|a| &a.name == n)) {
                by_avd.insert(name, (device, props));
                continue;
            }
        }
        physical.push((device, props));
    }
    let mut devices: Vec<Device> = avds
        .into_iter()
        .map(|avd| {
            let running = by_avd.remove(&avd.name);
            let (serial, online, booting, props) = match running {
                Some((device, props)) => {
                    let booted = props.get("sys.boot_completed").is_some_and(|v| v == "1");
                    let usable = device.state == "device";
                    (Some(device.serial), usable && booted, !(usable && booted), props)
                }
                None => (None, false, false, HashMap::new()),
            };
            Device {
                id: format!("avd:{}", avd.name),
                name: avd.display_name.clone(),
                kind: Kind::Virtual,
                form: avd.form,
                api: avd.api,
                codename: avd.codename.clone(),
                abi: avd.abi.clone(),
                serial,
                online,
                booting,
                error: None,
                props,
                avd: Some(avd),
            }
        })
        .collect();
    for (device, props) in physical {
        let error = match device.state.as_str() {
            "device" => None,
            "unauthorized" => Some("Unauthorized: allow USB debugging on the device".to_owned()),
            "no permissions" => Some("Missing permissions: check the udev rules".to_owned()),
            "offline" => Some("Offline".to_owned()),
            other => Some(other.to_owned()),
        };
        let fallback = device.model.clone().unwrap_or_else(|| device.serial.clone());
        devices.push(Device {
            id: format!("adb:{}", device.serial),
            name: physical_name(&props, &fallback),
            kind: Kind::Physical,
            form: FormFactor::from_characteristics(props.get("ro.build.characteristics").map_or("", String::as_str)),
            api: props.get("ro.build.version.sdk").and_then(|v| v.parse().ok()),
            codename: None,
            abi: props.get("ro.product.cpu.abi").cloned(),
            online: error.is_none(),
            booting: false,
            error,
            serial: Some(device.serial),
            props,
            avd: None,
        });
    }
    (devices, adb_error)
}

/// Starts an AVD in its own emulator window; Cold Boot skips the
/// Quick Boot snapshot. The emulator's output goes to a log file whose
/// tail explains a failed start.
pub fn launch(tools: &Tools, avd: &str, cold_boot: bool) -> std::io::Result<(std::process::Child, PathBuf)> {
    let emulator = tools
        .emulator
        .clone()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "The Android Emulator is not installed (SDK › emulator)"))?;
    let log = std::env::temp_dir().join(format!("junction-emulator-{avd}.log"));
    let file = std::fs::File::create(&log)?;
    let mut command = process::command(&emulator);
    command.args(["-netdelay", "none", "-netspeed", "full", "-avd", avd]);
    if cold_boot {
        command.arg("-no-snapshot-load");
    }
    if let Some(sdk) = &tools.sdk {
        if std::env::var_os("ANDROID_HOME").is_none() && std::env::var_os("ANDROID_SDK_ROOT").is_none() {
            command.env("ANDROID_HOME", sdk);
        }
    }
    if let Some(dir) = emulator.parent() {
        command.current_dir(dir);
    }
    let child = command.stdin(std::process::Stdio::null()).stdout(file.try_clone()?).stderr(file).spawn()?;
    Ok((child, log))
}

/// The last meaningful lines of an emulator log: why it didn't start.
pub fn launch_error(log: &Path) -> String {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let important: Vec<&str> = lines.iter().copied().filter(|l| l.contains("ERROR") || l.contains("FATAL") || l.contains("PANIC")).collect();
    let pick = if important.is_empty() { &lines[lines.len().saturating_sub(3)..] } else { &important[important.len().saturating_sub(3)..] };
    // "ERROR   | x86_64 emulation …": the message without its level tag.
    pick.iter().map(|l| l.split_once("| ").filter(|(tag, _)| tag.trim().chars().all(|c| c.is_ascii_uppercase())).map_or(*l, |(_, m)| m)).collect::<Vec<_>>().join("\n")
}

/// View Details: (label, value) rows of the Summary section.
pub fn summary(device: &Device, adb: Option<&Adb>) -> Vec<(&'static str, String)> {
    let unknown = || "Unknown".to_owned();
    let shell = |command: &str| -> Option<String> {
        let (adb, serial) = (adb?, device.serial.as_deref()?);
        if !device.online {
            return None;
        }
        adb.shell(serial, command).ok()
    };
    let wm_value = |text: Option<String>, key: &str| -> Option<String> {
        let text = text?;
        // An override wins over the physical value.
        let line = text.lines().find(|l| l.starts_with("Override")).or_else(|| text.lines().find(|l| l.contains(key)))?;
        Some(line.split_once(':')?.1.trim().to_owned())
    };
    let (size, density) = match &device.avd {
        Some(avd) => (avd.size_px(), avd.density()),
        None => {
            let size = wm_value(shell("wm size"), "size").and_then(|s| {
                let (w, h) = s.split_once('x')?;
                Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
            });
            let density = wm_value(shell("wm density"), "density").and_then(|d| d.parse().ok());
            (size, density)
        }
    };
    let abis = device
        .props
        .get("ro.product.cpu.abilist")
        .cloned()
        .filter(|s| !s.is_empty())
        .or_else(|| device.abi.clone())
        .unwrap_or_else(unknown);
    let mut rows = vec![
        ("API level", Some(device.api_label()).filter(|a| !a.is_empty()).unwrap_or_else(unknown)),
        ("Resolution (px)", size.map(|(w, h)| format!("{w} × {h}")).unwrap_or_else(unknown)),
        (
            "Resolution (dp)",
            match (size, density) {
                (Some((w, h)), Some(d)) if d > 0 => format!("{} × {}", w * 160 / d, h * 160 / d),
                _ => unknown(),
            },
        ),
        ("Density", density.map(|d| format!("{d} dpi")).unwrap_or_else(unknown)),
        ("Preferred ABI", device.abi.clone().unwrap_or_else(unknown)),
        ("Supported ABIs", abis),
    ];
    if device.online {
        let storage = shell("df /data").and_then(|out| {
            let line = out.lines().nth(1)?;
            let kb: u64 = line.split_whitespace().nth(3)?.trim_end_matches('K').parse().ok()?;
            Some(format!("{} MB", kb / 1024))
        });
        rows.push(("Available storage", storage.unwrap_or_else(unknown)));
        let battery = shell("dumpsys battery").map(|out| {
            if out.contains("Wireless powered: true") {
                "Wireless".to_owned()
            } else if out.contains("USB powered: true") {
                "USB".to_owned()
            } else if out.contains("AC powered: true") {
                "AC".to_owned()
            } else {
                out.lines().find_map(|l| l.trim().strip_prefix("level: ")).map(|l| format!("Battery: {l}")).unwrap_or_else(unknown)
            }
        });
        rows.push(("Power", battery.unwrap_or_else(unknown)));
    }
    if let Some(avd) = &device.avd {
        rows.push(("Size on disk", dir_size(&avd.dir).map(format_size).unwrap_or_else(unknown)));
    }
    rows
}

fn dir_size(dir: &Path) -> Option<u64> {
    let mut total = 0;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let meta = entry.metadata().ok()?;
        total += if meta.is_dir() { dir_size(&entry.path()).unwrap_or(0) } else { meta.len() };
    }
    Some(total)
}

fn format_size(bytes: u64) -> String {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    if bytes as f64 >= GB { format!("{:.1} GB", bytes as f64 / GB) } else { format!("{} MB", bytes / (1024 * 1024)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_versions() {
        let props: HashMap<String, String> = [("ro.product.manufacturer", "samsung"), ("ro.product.model", "SM-G991B")].map(|(k, v)| (k.to_owned(), v.to_owned())).into();
        assert_eq!(physical_name(&props, "x"), "Samsung SM-G991B");
        let props: HashMap<String, String> = [("ro.product.manufacturer", "Google"), ("ro.product.model", "Pixel 7")].map(|(k, v)| (k.to_owned(), v.to_owned())).into();
        assert_eq!(physical_name(&props, "x"), "Google Pixel 7");
        assert_eq!(release_name(34), "Android 14.0 (\"UpsideDownCake\")");
        assert_eq!(cpu_arch("arm64-v8a"), "arm64");
        assert_eq!(FormFactor::from_characteristics("nosdcard,watch"), FormFactor::Wear);
    }
}
