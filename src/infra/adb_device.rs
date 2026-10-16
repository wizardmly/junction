//! The device port over adb: install, stop, start and find the app's process.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::android::adb::Adb;
use crate::domain::run::DeviceBridge;

pub struct AdbDevice {
    pub adb: Adb,
}

const INSTALL_TIMEOUT: Duration = Duration::from_secs(300);
const QUICK: Duration = Duration::from_secs(15);

impl AdbDevice {
    fn shell(&self, serial: &str, command: &str) -> String {
        self.adb.run(&["-s", serial, "shell", command], QUICK).map(|o| format!("{}{}", o.stdout, o.stderr)).unwrap_or_default()
    }
}

impl DeviceBridge for AdbDevice {
    fn install(&self, serial: &str, apks: &[PathBuf]) -> Result<Duration, String> {
        let started = Instant::now();
        let paths: Vec<String> = apks.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        // -r replace, -t test-only APKs (debug builds from Android Studio's injected flag).
        let mut args = vec!["-s", serial, if apks.len() > 1 { "install-multiple" } else { "install" }, "-r", "-t"];
        args.extend(paths.iter().map(String::as_str));
        let out = self.adb.run(&args, INSTALL_TIMEOUT).map_err(|e| e.to_string())?;
        let text = format!("{}\n{}", out.stdout, out.stderr);
        if out.success && text.lines().any(|l| l.trim() == "Success") {
            return Ok(started.elapsed());
        }
        Err(install_failure(&text))
    }

    fn force_stop(&self, serial: &str, application_id: &str) {
        self.shell(serial, &format!("am force-stop {application_id}"));
    }

    fn launcher_activity(&self, serial: &str, application_id: &str) -> Option<String> {
        ["android.intent.category.LAUNCHER", "android.intent.category.LEANBACK_LAUNCHER"].iter().find_map(|category| {
            let out = self.shell(serial, &format!("cmd package resolve-activity --brief -a android.intent.action.MAIN -c {category} {application_id}"));
            parse_component(&out, application_id)
        })
    }

    fn start(&self, serial: &str, component: &str, debug: bool, flags: &str) -> Result<(), String> {
        let out = self.shell(serial, &crate::domain::run::start_command(component, debug, flags));
        match out.lines().find(|l| l.starts_with("Error") || l.contains("Exception")) {
            Some(error) => Err(error.trim().to_owned()),
            None => Ok(()),
        }
    }

    fn pid(&self, serial: &str, application_id: &str) -> Option<u32> {
        self.shell(serial, &format!("pidof {application_id}")).split_whitespace().next()?.parse().ok()
    }
}

/// `Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE: …]` → the bracketed part.
fn install_failure(text: &str) -> String {
    let line = text.lines().find(|l| l.contains("Failure [") || l.contains("INSTALL_")).or_else(|| text.lines().rev().find(|l| !l.trim().is_empty())).unwrap_or("Unknown failure");
    match (line.find('['), line.rfind(']')) {
        (Some(a), Some(b)) if b > a => line[a + 1..b].to_owned(),
        _ => line.trim().trim_start_matches("adb: ").to_owned(),
    }
}

/// The last `pkg/activity` line of `resolve-activity --brief`.
fn parse_component(out: &str, application_id: &str) -> Option<String> {
    out.lines().map(str::trim).rev().find(|l| l.starts_with(&format!("{application_id}/")) && !l.contains(' ')).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_adb_answers() {
        assert_eq!(install_failure("Performing Streamed Install\nadb: failed to install a.apk: Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE: Existing package signatures do not match]\n"), "INSTALL_FAILED_UPDATE_INCOMPATIBLE: Existing package signatures do not match");
        assert_eq!(parse_component("priority=0 preferredOrder=0\ncom.x/.MainActivity\n", "com.x"), Some("com.x/.MainActivity".into()));
        assert_eq!(parse_component("No activity found\n", "com.x"), None);
    }
}
