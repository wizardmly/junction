//! Layout Inspector's snapshot over adb: `dumpsys activity top` or
//! `uiautomator dump`, and `screencap`.

use std::time::Duration;

use crate::android::adb::Adb;
use crate::domain::view_hierarchy::LayoutSource;

pub struct AdbLayout {
    pub adb: Adb,
}

const DUMP: &str = "/data/local/tmp/junction-window-dump.xml";

impl LayoutSource for AdbLayout {
    fn hierarchy(&self, serial: &str, uiautomator: bool) -> Result<String, String> {
        let command = if uiautomator { format!("uiautomator dump {DUMP} >/dev/null 2>&1; cat {DUMP}; rm -f {DUMP}") } else { "dumpsys activity top".to_owned() };
        let out = self.adb.run(&["-s", serial, "shell", &command], Duration::from_secs(30)).map_err(|e| e.to_string())?;
        if out.stdout.trim().is_empty() {
            return Err(if out.stderr.trim().is_empty() { "The device returned nothing".to_owned() } else { out.stderr.trim().to_owned() });
        }
        Ok(out.stdout)
    }

    fn screenshot(&self, serial: &str) -> Result<Vec<u8>, String> {
        crate::android::mirror::screenshot(&self.adb, serial).map_err(|e| e.to_string())
    }
}
