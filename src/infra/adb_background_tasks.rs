//! The device's view of an app's background work over adb: `dumpsys
//! jobscheduler`, `alarm` and `power`, and its uid from `pm`.

use std::time::Duration;

use crate::android::adb::Adb;
use crate::domain::background_tasks::BackgroundTaskSource;

pub struct AdbBackgroundTasks {
    pub adb: Adb,
}

impl AdbBackgroundTasks {
    fn shell(&self, serial: &str, command: &str) -> Result<String, String> {
        let out = self.adb.run(&["-s", serial, "shell", command], Duration::from_secs(20)).map_err(|e| e.to_string())?;
        if out.stdout.trim().is_empty() && !out.stderr.trim().is_empty() {
            return Err(out.stderr.trim().to_owned());
        }
        Ok(out.stdout)
    }
}

/// A package name is letters, digits, `_` and `.`; anything else isn't passed to the shell.
fn package_arg(package: &str) -> Result<&str, String> {
    if !package.is_empty() && package.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.') { Ok(package) } else { Err(format!("Not a package name: {package}")) }
}

impl BackgroundTaskSource for AdbBackgroundTasks {
    fn jobs(&self, serial: &str, package: &str) -> Result<String, String> {
        self.shell(serial, &format!("dumpsys jobscheduler {}", package_arg(package)?))
    }

    fn alarms(&self, serial: &str) -> Result<String, String> {
        self.shell(serial, "dumpsys alarm")
    }

    fn power(&self, serial: &str) -> Result<String, String> {
        self.shell(serial, "dumpsys power")
    }

    fn packages(&self, serial: &str, package: &str) -> Result<String, String> {
        self.shell(serial, &format!("pm list packages -U {}", package_arg(package)?))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn package_names_only() {
        assert!(super::package_arg("com.example_app.debug").is_ok());
        assert!(super::package_arg("com.x; reboot").is_err());
    }
}
