//! The adb commands the Device Manager and Wi-Fi pairing use, and parsers
//! for their output.

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

use super::process::{self, Output};

#[derive(Clone, Debug, PartialEq)]
pub struct AdbDevice {
    pub serial: String,
    /// `device`, `offline`, `unauthorized`, `no permissions`, …
    pub state: String,
    pub model: Option<String>,
}

impl AdbDevice {
    pub fn is_emulator(&self) -> bool {
        self.serial.starts_with("emulator-")
    }
}

/// `adb devices -l`.
pub fn parse_devices(text: &str) -> Vec<AdbDevice> {
    text.lines()
        .filter(|l| !l.starts_with('*') && !l.starts_with("List of devices") && !l.trim().is_empty())
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let serial = words.next()?.to_owned();
            let rest: Vec<&str> = words.collect();
            // The state is every word before the first key:value ("no permissions (…)").
            let state_words: Vec<&str> = rest.iter().take_while(|w| !w.contains(':')).copied().collect();
            let state = state_words.first().map(|_| state_words.join(" ")).unwrap_or_default();
            let state = if state.starts_with("no permissions") { "no permissions".to_owned() } else { state };
            let model = rest.iter().find_map(|w| w.strip_prefix("model:")).map(|m| m.replace('_', " "));
            Some(AdbDevice { serial, state, model })
        })
        .collect()
}

/// `adb shell getprop`: `[key]: [value]` lines.
pub fn parse_getprop(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let (key, value) = line.split_once("]: [")?;
            Some((key.trim().trim_start_matches('[').to_owned(), value.trim().trim_end_matches(']').to_owned()))
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceKind {
    Pairing,
    Connect,
}

/// A device advertising itself over mDNS (`adb mdns services`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MdnsService {
    pub name: String,
    pub kind: ServiceKind,
    pub host: String,
    pub port: u16,
}

impl MdnsService {
    pub fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// ```text
/// List of discovered mdns services
/// adb-939AX05XBZ-vWgJpq	_adb-tls-connect._tcp.	192.168.1.86:39149
/// studio-k3Jd9-xyzA	_adb-tls-pairing._tcp	192.168.1.86:37313
/// ```
pub fn parse_mdns_services(text: &str) -> Vec<MdnsService> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split(|c: char| c == '\t' || c == ' ').filter(|p| !p.is_empty());
            let name = parts.next()?.to_owned();
            let kind = match parts.next()?.trim_end_matches('.') {
                "_adb-tls-pairing._tcp" => ServiceKind::Pairing,
                "_adb-tls-connect._tcp" => ServiceKind::Connect,
                _ => return None,
            };
            let (host, port) = parts.next()?.rsplit_once(':')?;
            Some(MdnsService { name, kind, host: host.to_owned(), port: port.parse().ok()? })
        })
        .collect()
}

/// `Successfully paired to 192.168.1.86:41915 [guid=adb-939AX05XBZ-vWgJpq]`
/// → (host, guid).
pub fn parse_pair(text: &str) -> Option<(String, String)> {
    let rest = &text[text.find("Successfully paired to ")? + "Successfully paired to ".len()..];
    let (address, rest) = rest.split_once(' ')?;
    let host = address.rsplit_once(':').map_or(address, |(h, _)| h).to_owned();
    let guid = rest.strip_prefix("[guid=")?.split(']').next()?.to_owned();
    Some((host, guid))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MdnsSupport {
    Supported,
    /// `adb mdns` unknown: platform-tools too old.
    AdbTooOld,
    /// The ADB_MDNS environment turned discovery off.
    Disabled,
    NotSupported,
    Error,
}

#[derive(Clone, Debug)]
pub struct Adb {
    pub path: PathBuf,
}

const QUICK: Duration = Duration::from_secs(10);

impl Adb {
    pub fn run(&self, args: &[&str], timeout: Duration) -> io::Result<Output> {
        process::run(&self.path, args, timeout)
    }

    pub fn devices(&self) -> io::Result<Vec<AdbDevice>> {
        // The first call starts the adb server, which takes a few seconds.
        let out = self.run(&["devices", "-l"], Duration::from_secs(20))?;
        if !out.success {
            return Err(io::Error::other(out.message()));
        }
        Ok(parse_devices(&out.stdout))
    }

    pub fn shell(&self, serial: &str, command: &str) -> io::Result<String> {
        Ok(self.run(&["-s", serial, "shell", command], QUICK)?.stdout)
    }

    pub fn getprop(&self, serial: &str) -> io::Result<HashMap<String, String>> {
        Ok(parse_getprop(&self.shell(serial, "getprop")?))
    }

    /// The AVD an emulator runs: the console's `avd name`, else the boot
    /// property (API 30+) or the older kernel one.
    pub fn avd_name(&self, serial: &str, props: &HashMap<String, String>) -> Option<String> {
        if let Ok(out) = self.run(&["-s", serial, "emu", "avd", "name"], QUICK) {
            let name = out.stdout.lines().next().unwrap_or("").trim().to_owned();
            if out.success && !name.is_empty() && name != "OK" && !name.starts_with("KO") {
                return Some(name);
            }
        }
        ["ro.boot.qemu.avd_name", "ro.kernel.qemu.avd_name"].iter().find_map(|k| props.get(*k).filter(|v| !v.is_empty()).cloned())
    }

    /// Stop: the emulator console's `kill`.
    pub fn emu_kill(&self, serial: &str) -> io::Result<()> {
        self.run(&["-s", serial, "emu", "kill"], QUICK).map(|_| ())
    }

    pub fn mdns_check(&self) -> MdnsSupport {
        match self.run(&["mdns", "check"], Duration::from_secs(20)) {
            Err(_) => MdnsSupport::Error,
            Ok(out) if !out.success => {
                let text = format!("{}{}", out.stdout, out.stderr).to_ascii_lowercase();
                if text.contains("unknown") && text.contains("command") { MdnsSupport::AdbTooOld } else { MdnsSupport::Error }
            }
            Ok(out) if out.stdout.contains("mdns discovery disabled") => MdnsSupport::Disabled,
            Ok(out) if out.stdout.contains("mdns daemon version") || out.stdout.contains("mdns daemon") => MdnsSupport::Supported,
            Ok(out) if out.stdout.trim().is_empty() => MdnsSupport::Error,
            Ok(_) => MdnsSupport::NotSupported,
        }
    }

    pub fn mdns_services(&self) -> io::Result<Vec<MdnsService>> {
        let out = self.run(&["mdns", "services"], QUICK)?;
        if !out.success {
            return Err(io::Error::other(out.message()));
        }
        Ok(parse_mdns_services(&out.stdout))
    }

    /// `adb pair host:port code`; the paired device's (host, guid).
    pub fn pair(&self, address: &str, code: &str) -> Result<(String, String), String> {
        let out = self.run(&["pair", address, code], Duration::from_secs(60)).map_err(|e| e.to_string())?;
        let text = format!("{}\n{}", out.stdout, out.stderr);
        match parse_pair(&text) {
            Some(result) if out.success => Ok(result),
            _ => Err(out.message()),
        }
    }

    pub fn connect(&self, address: &str) -> io::Result<Output> {
        self.run(&["connect", address], Duration::from_secs(30))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices() {
        let text = "* daemon not running; starting now at tcp:5037\n* daemon started successfully\nList of devices attached\n\
            emulator-5554          device product:sdk_gphone64_x86_64 model:sdk_gphone64_x86_64 device:emu64xa transport_id:1\n\
            R58M12345              unauthorized usb:1-1 transport_id:2\n\
            0123456789ABCDEF       no permissions (missing udev rules? user is in the plugdev group); see [http://developer.android.com/tools/device.html] usb:3-2 transport_id:3\n\
            adb-939AX05XBZ-vWgJpq._adb-tls-connect._tcp device product:panther model:Pixel_7 device:panther transport_id:4\n";
        let devices = parse_devices(text);
        assert_eq!(devices.len(), 4);
        assert_eq!((devices[0].serial.as_str(), devices[0].state.as_str(), devices[0].model.as_deref()), ("emulator-5554", "device", Some("sdk gphone64 x86 64")));
        assert!(devices[0].is_emulator());
        assert_eq!(devices[1].state, "unauthorized");
        assert_eq!(devices[2].state, "no permissions");
        assert_eq!(devices[3].model.as_deref(), Some("Pixel 7"));
    }

    #[test]
    fn getprop() {
        let props = parse_getprop("[ro.build.version.sdk]: [34]\n[ro.product.model]: [Pixel 7]\n[empty]: []\n");
        assert_eq!(props.get("ro.build.version.sdk").map(String::as_str), Some("34"));
        assert_eq!(props.get("ro.product.model").map(String::as_str), Some("Pixel 7"));
        assert_eq!(props.get("empty").map(String::as_str), Some(""));
    }

    #[test]
    fn mdns_and_pair() {
        let services = parse_mdns_services(
            "List of discovered mdns services\nadb-939AX05XBZ-vWgJpq\t_adb-tls-connect._tcp.\t192.168.1.86:39149\nstudio-k3Jd9\t_adb-tls-pairing._tcp\t192.168.1.86:37313\n",
        );
        assert_eq!(services.len(), 2);
        assert_eq!(services[0].kind, ServiceKind::Connect);
        assert_eq!((services[1].name.as_str(), services[1].kind, services[1].address()), ("studio-k3Jd9", ServiceKind::Pairing, "192.168.1.86:37313".into()));
        assert_eq!(
            parse_pair("Enter pairing code: Successfully paired to 192.168.1.86:41915 [guid=adb-939AX05XBZ-vWgJpq]"),
            Some(("192.168.1.86".into(), "adb-939AX05XBZ-vWgJpq".into()))
        );
        assert_eq!(parse_pair("Failed: Wrong password or connection was dropped."), None);
    }
}
