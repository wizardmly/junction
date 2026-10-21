//! Logcat files, as Android Studio's Export / Import Logcat reads and
//! writes them (`LogcatFileIo`): its `.logcat` JSON (the device, the filter,
//! the project's application IDs and every message's header and text), and
//! plain `logcat -v threadtime` output and bugreport logs (with a uid
//! column) on import.

use serde_json::{Value, json};

use super::logcat::{Entry, Level, parse_threadtime};

/// The device a log was read from, as the file records it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceInfo {
    pub serial: String,
    pub manufacturer: String,
    pub model: String,
    /// "16"
    pub release: String,
    pub api: u32,
    /// The AVD's name, for an emulator.
    pub avd: Option<String>,
}

impl DeviceInfo {
    /// As Android Studio names it: the AVD, or "Google Pixel 9a".
    pub fn name(&self) -> String {
        if let Some(avd) = &self.avd {
            return avd.clone();
        }
        if self.model.starts_with(&self.manufacturer) || self.manufacturer.is_empty() { self.model.clone() } else { format!("{} {}", self.manufacturer, self.model) }
    }
}

/// A log read from a file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogFile {
    /// The device's name, when the file says.
    pub device: Option<String>,
    pub filter: Option<String>,
    pub application_ids: Vec<String>,
    pub entries: Vec<Entry>,
}

fn level_name(level: Level) -> &'static str {
    match level {
        Level::Verbose => "VERBOSE",
        Level::Debug => "DEBUG",
        Level::Info => "INFO",
        Level::Warn => "WARN",
        Level::Error => "ERROR",
        Level::Assert => "ASSERT",
    }
}

fn level_from_name(name: &str) -> Option<Level> {
    Some(match name {
        "VERBOSE" => Level::Verbose,
        "DEBUG" => Level::Debug,
        "INFO" => Level::Info,
        "WARN" => Level::Warn,
        "ERROR" => Level::Error,
        "ASSERT" => Level::Assert,
        _ => return None,
    })
}

/// "2026-10-15 02:05:48.123" in local time, as `logcat -v year` prints it.
fn local_time(millis: i64) -> String {
    chrono::DateTime::from_timestamp_millis(millis).map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S%.3f").to_string()).unwrap_or_default()
}

/// The `.logcat` JSON Android Studio's Export Logcat writes (Gson, pretty
/// printed). Separator lines aren't log messages and are left out.
pub fn write(entries: &[Entry], device: &DeviceInfo, filter: &str, application_ids: &[String]) -> String {
    let messages: Vec<Value> = entries
        .iter()
        .filter(|e| !e.system)
        .map(|e| {
            // The package is the app's ID once its process is known; else the process name.
            let app = if super::logcat::is_app_process(&e.package) { e.package.split(':').next().unwrap_or(&e.package) } else { "" };
            json!({
                "header": {
                    "logLevel": level_name(e.level),
                    "pid": e.pid,
                    "tid": e.tid,
                    "applicationId": app,
                    "processName": e.package,
                    "tag": e.tag,
                    "timestamp": { "seconds": e.millis.div_euclid(1000), "nanos": e.millis.rem_euclid(1000) * 1_000_000 },
                },
                "message": e.message,
            })
        })
        .collect();
    let api = json!({ "majorVersion": device.api, "minorVersion": 0 });
    let device = match &device.avd {
        Some(avd) => json!({ "emulatorDevice": {
            "deviceId": { "pluginId": "LocalEmulator", "isTemplate": false, "identifier": format!("serial={}", device.serial) },
            "serialNumber": device.serial, "isOnline": true, "release": device.release, "apiLevel": api, "featureLevel": device.api,
            "avdName": avd, "avdPath": "", "type": "HANDHELD",
        }}),
        None => json!({ "physicalDevice": {
            "deviceId": { "pluginId": "PhysicalDevice", "isTemplate": false, "identifier": format!("serial={}", device.serial) },
            "serialNumber": device.serial, "isOnline": true, "release": device.release, "apiLevel": api, "featureLevel": device.api,
            "manufacturer": device.manufacturer, "model": device.model, "type": "HANDHELD",
        }}),
    };
    let data = json!({
        "metadata": { "device": device, "filter": filter, "projectApplicationIds": application_ids },
        "logcatMessages": messages,
    });
    serde_json::to_string_pretty(&data).unwrap_or_default()
}

/// Reads a `.logcat` file, `logcat -v threadtime` output (with or without
/// the year) or a bugreport's log section.
pub fn read(text: &str) -> Result<LogFile, String> {
    let first = text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with("--------- ")).unwrap_or("");
    if first.starts_with('{') {
        return read_json(text);
    }
    let entries: Vec<Entry> = text.lines().filter_map(|l| parse_threadtime(l).or_else(|| parse_bugreport(l))).collect();
    if entries.is_empty() {
        return Err("Not a Logcat file".to_owned());
    }
    Ok(LogFile { entries, ..LogFile::default() })
}

/// A bugreport log line: threadtime with the uid before the pid
/// (`10-15 02:05:48.123  u0_a190  1234  1250 D Tag: message`).
fn parse_bugreport(line: &str) -> Option<Entry> {
    // Skip the date, time and uid fields.
    let mut rest = line;
    let mut skipped = Vec::new();
    for _ in 0..3 {
        rest = rest.trim_start();
        let end = rest.find(char::is_whitespace)?;
        skipped.push(&rest[..end]);
        rest = &rest[end..];
    }
    parse_threadtime(&format!("{} {}{rest}", skipped[0], skipped[1]))
}

fn read_json(text: &str) -> Result<LogFile, String> {
    let data: Value = serde_json::from_str(text).map_err(|e| format!("Not a Logcat file: {e}"))?;
    let metadata = data.get("metadata").filter(|m| !m.is_null());
    let device = metadata.and_then(|m| m.get("device")).and_then(|d| {
        if let Some(p) = d.get("physicalDevice") {
            let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
            Some(DeviceInfo { manufacturer: s("manufacturer"), model: s("model"), serial: s("serialNumber"), ..DeviceInfo::default() }.name())
        } else {
            d.get("emulatorDevice").and_then(|e| e.get("avdName")).and_then(Value::as_str).map(str::to_owned)
        }
    });
    let filter = metadata.and_then(|m| m.get("filter")).and_then(Value::as_str).map(str::to_owned);
    let application_ids = metadata.and_then(|m| m.get("projectApplicationIds")).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
    let messages = data.get("logcatMessages").and_then(Value::as_array).ok_or("Not a Logcat file: no logcatMessages")?;
    let entries = messages
        .iter()
        .filter_map(|m| {
            let h = m.get("header")?;
            let s = |k: &str| h.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
            let n = |k: &str| h.get(k).and_then(Value::as_u64).unwrap_or(0) as u32;
            let stamp = h.get("timestamp");
            let seconds = stamp.and_then(|t| t.get("seconds")).and_then(Value::as_i64).unwrap_or(0);
            let nanos = stamp.and_then(|t| t.get("nanos")).and_then(Value::as_i64).unwrap_or(0);
            let millis = seconds * 1000 + nanos / 1_000_000;
            let (app, process) = (s("applicationId"), s("processName"));
            Some(Entry {
                time: local_time(millis),
                millis,
                pid: n("pid"),
                tid: n("tid"),
                level: level_from_name(&s("logLevel"))?,
                tag: s("tag"),
                package: if process.is_empty() { app } else { process },
                message: m.get("message").and_then(Value::as_str).unwrap_or_default().to_owned(),
                system: false,
            })
        })
        .collect();
    Ok(LogFile { device, filter, application_ids, entries })
}

const MY_PACKAGE: &str = "package:mine";

/// The filter an imported log opens with (`safeGetFilter`): the saved one,
/// with `package:mine` spelled out as the exporting project's IDs unless
/// they are this project's. Android Studio writes several as
/// `(package:a | package:b)`; terms with the same key are ORed here, so
/// they are listed side by side.
pub fn imported_filter(file: &LogFile, application_ids: &[String]) -> Option<String> {
    let filter = file.filter.clone()?;
    if !filter.contains(MY_PACKAGE) {
        return Some(filter);
    }
    let saved = &file.application_ids;
    let same = {
        let (mut a, mut b) = (saved.clone(), application_ids.to_vec());
        a.sort();
        b.sort();
        a == b
    };
    let packages = saved.iter().map(|id| format!("package:{id}")).collect::<Vec<_>>().join(" ");
    Some(if same { filter } else { filter.replace(MY_PACKAGE, &packages).trim().to_owned() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(millis: i64, level: Level, package: &str, tag: &str, message: &str) -> Entry {
        Entry { time: local_time(millis), millis, pid: 1234, tid: 1250, level, tag: tag.into(), package: package.into(), message: message.into(), system: false }
    }

    #[test]
    fn round_trips_android_studio_json() {
        let entries = vec![
            entry(1_760_000_000_123, Level::Info, "com.example.app", "MainActivity", "onCreate"),
            super::super::logcat::separator(true, 1234, "com.example.app", 1_760_000_000_200),
            entry(1_760_000_001_000, Level::Error, "/system/bin/surfaceflinger", "SurfaceFlinger", "line 1\n\tat a.b(C.kt:3) \"quoted\""),
        ];
        let device = DeviceInfo { serial: "emulator-5554".into(), manufacturer: "Google".into(), model: "Pixel 9a".into(), release: "16".into(), api: 36, avd: None };
        let text = write(&entries, &device, "package:mine level:info", &["com.example.app".into()]);
        let json: Value = serde_json::from_str(&text).unwrap();
        let header = &json["logcatMessages"][0]["header"];
        assert_eq!((header["logLevel"].as_str(), header["timestamp"]["seconds"].as_i64(), header["timestamp"]["nanos"].as_i64()), (Some("INFO"), Some(1_760_000_000), Some(123_000_000)));
        assert_eq!(json["logcatMessages"][1]["header"]["applicationId"], "");
        assert_eq!(json["metadata"]["device"]["physicalDevice"]["model"], "Pixel 9a");
        let read = read(&text).unwrap();
        assert_eq!(read.device.as_deref(), Some("Google Pixel 9a"));
        assert_eq!(read.filter.as_deref(), Some("package:mine level:info"));
        assert_eq!(read.application_ids, ["com.example.app"]);
        // The separator isn't a message.
        assert_eq!(read.entries, vec![entries[0].clone(), entries[2].clone()]);
    }

    #[test]
    fn reads_threadtime_and_bugreport_logs() {
        let text = "--------- beginning of main\n2026-10-15 02:05:48.123  1234  1250 D Tag     : hello\n10-15 02:05:49.000  5678  5679 W Other: world\n";
        let log = read(text).unwrap();
        assert_eq!(log.entries.len(), 2);
        assert_eq!((log.entries[0].tag.as_str(), log.entries[0].message.as_str(), log.entries[1].pid), ("Tag", "hello", 5678));
        let bugreport = "========================================================\n== dumpstate: 2026-10-15 02:05:50\n10-15 02:05:48.123  u0_a190  1234  1250 I ActivityManager: Start proc\n10-15 02:05:48.200  1000  1300  1301 E Sys: boom\n";
        let log = read(bugreport).unwrap();
        let shown: Vec<(u32, u32, &str)> = log.entries.iter().map(|e| (e.pid, e.tid, e.tag.as_str())).collect();
        assert_eq!(shown, [(1234, 1250, "ActivityManager"), (1300, 1301, "Sys")]);
        assert!(read("hello\nworld").is_err());
    }

    #[test]
    fn spells_out_package_mine() {
        let file = |filter: &str, ids: &[&str]| LogFile { filter: Some(filter.into()), application_ids: ids.iter().map(|s| s.to_string()).collect(), ..LogFile::default() };
        let here = ["com.here".to_owned()];
        assert_eq!(imported_filter(&file("level:warn", &["a"]), &here).as_deref(), Some("level:warn"));
        assert_eq!(imported_filter(&file("package:mine level:warn", &["com.here"]), &here).as_deref(), Some("package:mine level:warn"));
        assert_eq!(imported_filter(&file("package:mine level:warn", &["a"]), &here).as_deref(), Some("package:a level:warn"));
        assert_eq!(imported_filter(&file("package:mine", &["a", "b"]), &here).as_deref(), Some("package:a package:b"));
        assert_eq!(imported_filter(&file("package:mine is:crash", &["a", "b"]), &here).as_deref(), Some("package:a package:b is:crash"));
        assert_eq!(imported_filter(&file("package:mine is:crash", &[]), &here).as_deref(), Some("is:crash"));
    }
}
