//! The device's sockets over adb: the kernel's `/proc/net` tables, which
//! list every app's sockets with their uid.

use std::time::Duration;

use crate::android::adb::Adb;
use crate::domain::network_connections::NetworkSource;
use crate::infra::adb_background_tasks::package_arg;

pub struct AdbNetwork {
    pub adb: Adb,
}

impl AdbNetwork {
    fn shell(&self, serial: &str, command: &str) -> Result<String, String> {
        let out = self.adb.run(&["-s", serial, "shell", command], Duration::from_secs(10)).map_err(|e| e.to_string())?;
        if out.stdout.trim().is_empty() && !out.stderr.trim().is_empty() {
            return Err(out.stderr.trim().to_owned());
        }
        Ok(out.stdout)
    }
}

impl NetworkSource for AdbNetwork {
    fn sockets(&self, serial: &str) -> Result<String, String> {
        self.shell(serial, "for f in tcp tcp6 udp udp6; do echo \"== $f\"; cat /proc/net/$f; done")
    }

    fn packages(&self, serial: &str, package: &str) -> Result<String, String> {
        self.shell(serial, &format!("pm list packages -U {}", package_arg(package)?))
    }
}
