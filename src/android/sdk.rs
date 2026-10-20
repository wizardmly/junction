//! Finding the Android SDK and the AVD folder as Android Studio does.

use std::path::{Path, PathBuf};

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

pub fn home() -> Option<PathBuf> {
    env_path(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).or_else(|| env_path("HOME"))
}

fn exe(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_owned() }
}

/// A folder that looks like an SDK: it has platform-tools or the emulator.
fn is_sdk(dir: &Path) -> bool {
    dir.join("platform-tools").is_dir() || dir.join("emulator").is_dir()
}

/// `sdk.dir` from a Gradle project's local.properties (`C\:\\Users\\…`).
pub fn sdk_dir_from_local_properties(text: &str) -> Option<PathBuf> {
    crate::domain::project_structure::sdk_dir(text).map(PathBuf::from)
}

/// The default SDK folder Android Studio installs to.
fn default_sdk() -> Option<PathBuf> {
    if cfg!(windows) {
        env_path("LOCALAPPDATA").map(|d| d.join("Android").join("Sdk"))
    } else if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library/Android/sdk"))
    } else {
        home().map(|h| h.join("Android/Sdk"))
    }
}

/// adb found on PATH: its SDK is the folder above platform-tools.
fn adb_on_path() -> Option<PathBuf> {
    let name = exe("adb");
    std::env::split_paths(&std::env::var_os("PATH")?).map(|dir| dir.join(&name)).find(|p| p.is_file())
}

/// The SDK: the project's local.properties, ANDROID_HOME, ANDROID_SDK_ROOT,
/// the default install folder, then the one holding the adb on PATH.
pub fn locate(project: Option<&Path>) -> Option<PathBuf> {
    let from_project = project.and_then(|p| std::fs::read_to_string(p.join("local.properties")).ok()).and_then(|t| sdk_dir_from_local_properties(&t));
    let candidates = [from_project, env_path("ANDROID_HOME"), env_path("ANDROID_SDK_ROOT"), default_sdk()];
    candidates
        .into_iter()
        .flatten()
        .find(|dir| is_sdk(dir))
        .or_else(|| adb_on_path()?.parent()?.parent().map(Path::to_path_buf).filter(|d| is_sdk(d)))
}

/// The SDK's adb, else the one on PATH.
pub fn adb(sdk: Option<&Path>) -> Option<PathBuf> {
    sdk.map(|s| s.join("platform-tools").join(exe("adb"))).filter(|p| p.is_file()).or_else(adb_on_path)
}

pub fn emulator(sdk: Option<&Path>) -> Option<PathBuf> {
    sdk.map(|s| s.join("emulator").join(exe("emulator"))).filter(|p| p.is_file())
}

/// Where AVDs live: ANDROID_AVD_HOME, else `avd` under ANDROID_EMULATOR_HOME,
/// ANDROID_USER_HOME or ANDROID_SDK_HOME/.android, else ~/.android/avd.
pub fn avd_home() -> Option<PathBuf> {
    env_path("ANDROID_AVD_HOME")
        .or_else(|| env_path("ANDROID_EMULATOR_HOME").map(|d| d.join("avd")))
        .or_else(|| env_path("ANDROID_USER_HOME").map(|d| d.join("avd")))
        .or_else(|| env_path("ANDROID_SDK_HOME").map(|d| d.join(".android").join("avd")))
        .or_else(|| home().map(|h| h.join(".android").join("avd")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_properties_sdk_dir() {
        let text = "## comment\nsdk.dir=C\\:\\\\Users\\\\Jake\\\\AppData\\\\Local\\\\Android\\\\Sdk\n";
        assert_eq!(sdk_dir_from_local_properties(text), Some(PathBuf::from(r"C:\Users\Jake\AppData\Local\Android\Sdk")));
        assert_eq!(sdk_dir_from_local_properties("sdk.dir=/home/me/Android/Sdk"), Some(PathBuf::from("/home/me/Android/Sdk")));
        assert_eq!(sdk_dir_from_local_properties("ndk.dir=/x"), None);
    }
}
