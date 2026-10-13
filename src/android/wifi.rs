//! Wireless debugging in the device list: devices advertising it over mDNS
//! (`adb mdns services`) next to the connections adb has. Devices seen
//! before are connected on their own, a device that came back on a new port
//! replaces its old offline connection, and the rest are offered for Wi-Fi
//! pairing. The devices seen before are kept on disk to name those rows.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::adb::{Adb, AdbDevice, MdnsService, ServiceKind};

/// How often one address is connected on its own.
const RETRY: Duration = Duration::from_secs(30);

/// What a scan keeps about Wi-Fi devices.
#[derive(Default)]
pub struct WifiState {
    pub known: KnownDevices,
    /// Addresses connected on their own, and when.
    auto_connected: HashMap<String, Instant>,
    /// adb serial → hardware serial (`ro.serialno`), for Wi-Fi connections
    /// that went offline and no longer answer getprop.
    pub hardware: HashMap<String, String>,
}

/// The adb devices after a pass, and the Wi-Fi rows to add.
#[derive(Default)]
pub struct Reconciled {
    pub devices: Vec<AdbDevice>,
    /// Offline Wi-Fi connections whose device advertises a new address it
    /// could not be connected on: adb serial → that address.
    pub moved: HashMap<String, String>,
    /// Devices available for Wi-Fi pairing: (hardware serial, address).
    pub available: Vec<(String, String)>,
}

pub fn reconcile(adb: &Adb, mut devices: Vec<AdbDevice>, state: &mut WifiState) -> Reconciled {
    let services = adb.mdns_services().unwrap_or_default();
    let services: Vec<&MdnsService> = services.iter().filter(|s| s.kind == ServiceKind::Connect).collect();

    // Turning wireless debugging off and on again moves the device to a new
    // port, and adb keeps the old connection as offline. Connect the new
    // port and drop the old one in the same pass, so the device's row goes
    // from offline to online in place.
    let mut moved = HashMap::new();
    let mut reconnected = false;
    for device in devices.iter().filter(|d| d.state != "device") {
        let Some((hardware, host)) = wifi_identity(&device.serial) else { continue };
        let hardware = hardware.map(str::to_owned).or_else(|| state.hardware.get(&device.serial).cloned());
        let Some(service) = services.iter().find(|s| {
            s.address() != device.serial
                && !device.serial.starts_with(&s.name)
                && (hardware.as_deref().is_some_and(|h| service_serial(&s.name) == Some(h)) || host.is_some_and(|h| s.host == h))
        }) else {
            continue;
        };
        let address = service.address();
        if due(&address, state) && connect(adb, &address) {
            let _ = adb.run(&["disconnect", &device.serial], Duration::from_secs(5));
            reconnected = true;
        } else {
            moved.insert(device.serial.clone(), address);
        }
    }

    // Advertised devices adb has no connection to; its connections are
    // named after the service or the address. A device seen before was
    // probably paired with this computer: connect it as adb would on its
    // own, instead of waiting for a click.
    let mut available: Vec<(String, String)> = Vec::new();
    for service in &services {
        let address = service.address();
        let connected = devices.iter().any(|d| d.serial.starts_with(&service.name) || d.serial == address);
        let Some(serial) = service_serial(&service.name).filter(|_| !connected && !moved.values().any(|a| a == &address)) else { continue };
        if state.known.get(serial).is_some() && due(&address, state) && connect(adb, &address) {
            reconnected = true;
            continue;
        }
        if !available.iter().any(|(s, _)| s == serial) {
            available.push((serial.to_owned(), address));
        }
    }
    if reconnected {
        if let Ok(now) = adb.devices() {
            devices = now;
        }
    }
    Reconciled { devices, moved, available }
}

/// Whether `address` may be connected on its own now (then not again
/// for a while).
fn due(address: &str, state: &mut WifiState) -> bool {
    let due = state.auto_connected.get(address).is_none_or(|t| t.elapsed() > RETRY);
    if due {
        state.auto_connected.insert(address.to_owned(), Instant::now());
    }
    due
}

/// `adb connect`; whether it connected.
pub fn connect(adb: &Adb, address: &str) -> bool {
    adb.connect(address).is_ok_and(|out| {
        let text = format!("{}{}", out.stdout, out.stderr).to_lowercase();
        out.success && text.contains("connected to") && !text.contains("failed") && !text.contains("unable")
    })
}

/// Which device an adb Wi-Fi connection is: the hardware serial of an
/// `adb-<serial>-<suffix>._adb-tls-connect._tcp` connection, the host of a
/// `host:port` one; None for USB and emulators.
pub fn wifi_identity(serial: &str) -> Option<(Option<&str>, Option<&str>)> {
    if let Some(name) = serial.split_once("._adb-tls-connect").map(|(n, _)| n) {
        return Some((service_serial(name), None));
    }
    let (host, port) = serial.rsplit_once(':')?;
    port.parse::<u16>().ok()?;
    Some((None, Some(host)))
}

/// The hardware serial in an `adb-<serial>-<suffix>` service name.
fn service_serial(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("adb-")?;
    Some(rest.rsplit_once('-').map_or(rest, |(serial, _)| serial)).filter(|s| !s.is_empty())
}

/// The properties a Device Manager row is made of.
const KNOWN_PROPS: &[&str] = &[
    "ro.product.model",
    "ro.product.manufacturer",
    "ro.build.version.sdk",
    "ro.build.version.sdk_minor",
    "ro.build.version.sdk_full",
    "ro.product.cpu.abi",
    "ro.build.characteristics",
];

/// Physical devices seen before, by hardware serial (`ro.serialno`), with
/// their row properties; read from disk on first use.
#[derive(Default)]
pub struct KnownDevices(Option<HashMap<String, HashMap<String, String>>>);

impl KnownDevices {
    fn file() -> Option<PathBuf> {
        crate::index::libraries::cache_dir().map(|d| d.join("known-devices.json"))
    }

    fn all(&mut self) -> &mut HashMap<String, HashMap<String, String>> {
        self.0.get_or_insert_with(|| Self::file().and_then(|f| std::fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default())
    }

    pub fn get(&mut self, serial: &str) -> Option<&HashMap<String, String>> {
        self.all().get(serial)
    }

    /// Remembers a physical device's row properties.
    pub fn remember(&mut self, props: &HashMap<String, String>) {
        let Some(serial) = props.get("ro.serialno").filter(|s| !s.is_empty()).cloned() else { return };
        let kept: HashMap<String, String> = KNOWN_PROPS.iter().filter_map(|k| props.get(*k).map(|v| (k.to_string(), v.clone()))).collect();
        if self.all().get(&serial) == Some(&kept) {
            return;
        }
        self.all().insert(serial, kept);
        if let (Some(file), Some(known)) = (Self::file(), &self.0) {
            let _ = std::fs::create_dir_all(file.parent().unwrap_or(Path::new(".")));
            if let Ok(json) = serde_json::to_vec_pretty(known) {
                let _ = std::fs::write(file, json);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities() {
        assert_eq!(service_serial("adb-4C091FDAQ000AB-vWgJpq"), Some("4C091FDAQ000AB"));
        assert_eq!(service_serial("studio-abc"), None);
        assert_eq!(wifi_identity("adb-4C09-vWg._adb-tls-connect._tcp"), Some((Some("4C09"), None)));
        assert_eq!(wifi_identity("192.168.1.5:41234"), Some((None, Some("192.168.1.5"))));
        assert_eq!(wifi_identity("R58M12345"), None);
        assert_eq!(wifi_identity("emulator-5554"), None);
    }
}
