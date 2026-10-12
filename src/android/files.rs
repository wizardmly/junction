//! A device's file system for Device Explorer, as Android Studio reaches
//! it: `ls -al` listings, `mkdir` / `touch` / `rm` through `adb shell`,
//! `adb pull` / `adb push` for transfers. Without root, the folders under
//! `/data/data/<package>` are reached through `run-as <package>`; with
//! root (`adb root`, or a working `su 0`), everything is reached directly.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;

use super::adb::Adb;
use super::process;

/// What `ls -l` says an entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
    Block,
    Character,
    Socket,
    Fifo,
    Other,
}

/// One line of a listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub name: String,
    pub kind: EntryKind,
    pub permissions: String,
    /// `2024-05-30 18:40`
    pub date: String,
    /// In bytes; `None` for device nodes ("1, 3") and the entries Device
    /// Explorer makes up (`/data/data/<package>`).
    pub size: Option<u64>,
    pub link_target: Option<String>,
    /// The package whose `run-as` reaches this entry.
    pub run_as: Option<String>,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.kind == EntryKind::Directory
    }

    /// An entry Device Explorer adds itself, where `ls` can't look.
    fn synthetic(parent: &str, name: &str, kind: EntryKind, run_as: Option<String>) -> Self {
        Self {
            path: resolve(parent, name),
            name: name.to_owned(),
            kind,
            permissions: String::new(),
            date: String::new(),
            size: None,
            link_target: None,
            run_as,
        }
    }
}

/// `parent` + `/` + `name`.
pub fn resolve(parent: &str, name: &str) -> String {
    if parent.ends_with('/') { format!("{parent}{name}") } else { format!("{parent}/{name}") }
}

/// The folder a device path is in.
pub fn parent(path: &str) -> &str {
    match path.trim_end_matches('/').rfind('/') {
        Some(0) | None => "/",
        Some(i) => &path[..i],
    }
}

/// A path for the device's shell.
pub fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// ddmlib's `LS_L_PATTERN`, plus the ACL / SELinux mark some `ls` builds append.
static LS_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([bcdlsp-][-r][-w][-xsS][-r][-w][-xsS][-r][-w][-xstST])[.+@]?\s+(?:\d+\s+)?(\S+)\s+(\S+)\s+([\d\s,]*?)\s*(\d{4}-\d\d-\d\d)\s+(\d\d:\d\d)(?::\d\d(?:\.\d+)?)?(?:\s+[+-]\d{4})?\s+(.*)$").unwrap()
});

fn kind_of(permissions: &str) -> EntryKind {
    match permissions.as_bytes().first() {
        Some(b'-') => EntryKind::File,
        Some(b'd') => EntryKind::Directory,
        Some(b'l') => EntryKind::Symlink,
        Some(b'b') => EntryKind::Block,
        Some(b'c') => EntryKind::Character,
        Some(b's') => EntryKind::Socket,
        Some(b'p') => EntryKind::Fifo,
        _ => EntryKind::Other,
    }
}

/// The entries of `ls -al <parent>/`.
pub fn parse_ls(text: &str, parent: &str, run_as: Option<&str>) -> Vec<Entry> {
    text.lines()
        .filter_map(|line| {
            let caps = LS_LINE.captures(line.trim_end_matches('\r'))?;
            let permissions = caps[1].to_owned();
            let kind = kind_of(&permissions);
            let mut name = caps[7].to_owned();
            let mut link_target = None;
            if kind == EntryKind::Symlink {
                if let Some((link, target)) = name.split_once(" -> ") {
                    link_target = Some(target.to_owned());
                    name = link.to_owned();
                }
            }
            if name == "." || name == ".." || name.is_empty() {
                return None;
            }
            let size_text = caps[4].trim();
            let size = if size_text.contains(',') { None } else { size_text.parse().ok() };
            Some(Entry {
                path: resolve(parent, &name),
                name,
                kind,
                permissions,
                date: format!("{} {}", &caps[5], &caps[6]),
                size,
                link_target,
                run_as: run_as.map(str::to_owned),
            })
        })
        .collect()
}

/// `package:com.example.app` lines.
pub fn parse_packages(text: &str) -> Vec<String> {
    let mut packages: Vec<String> = text.lines().filter_map(|l| l.trim().strip_prefix("package:")).map(str::to_owned).filter(|p| !p.is_empty()).collect();
    packages.sort();
    packages
}

/// `pm list packages -f`: `package:/data/app/~~x==/com.example-y==/base.apk=com.example`.
pub fn parse_package_paths(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let (path, package) = l.trim().strip_prefix("package:")?.rsplit_once('=')?;
            Some((package.to_owned(), path.to_owned()))
        })
        .collect()
}

/// IntelliJ's `StringUtil.formatDuration`: `1 m 5 s 432 ms`.
pub fn format_duration(millis: u128) -> String {
    let (m, s, ms) = (millis / 60_000, millis / 1000 % 60, millis % 1000);
    let mut parts = Vec::new();
    if m > 0 {
        parts.push(format!("{m} m"));
    }
    if s > 0 {
        parts.push(format!("{s} s"));
    }
    if ms > 0 || parts.is_empty() {
        parts.push(format!("{ms} ms"));
    }
    parts.join(" ")
}

/// Android Studio's `getHumanizedSize`: `890 B`, `67 KB`, `223.7 MB`.
pub fn humanized_size(bytes: u64) -> String {
    const K: f64 = 1024.;
    let b = bytes as f64;
    let one = |v: f64| {
        let text = format!("{v:.1}");
        text.strip_suffix(".0").map(str::to_owned).unwrap_or(text)
    };
    if b > K * K * K {
        format!("{} GB", one(b / (K * K * K)))
    } else if b > K * K {
        format!("{} MB", one(b / (K * K)))
    } else if b > K {
        format!("{} KB", one(b / K))
    } else {
        format!("{bytes} B")
    }
}

/// Files, folders and bytes a transfer moved (counted on the computer's side).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub files: usize,
    pub dirs: usize,
    pub bytes: u64,
}

impl Totals {
    pub fn of(path: &Path) -> Self {
        let mut totals = Totals::default();
        let Ok(meta) = std::fs::symlink_metadata(path) else { return totals };
        if meta.is_dir() {
            totals.dirs += 1;
            for entry in std::fs::read_dir(path).into_iter().flatten().flatten() {
                let inner = Totals::of(&entry.path());
                totals.files += inner.files;
                totals.dirs += inner.dirs;
                totals.bytes += inner.bytes;
            }
        } else {
            totals.files += 1;
            totals.bytes += meta.len();
        }
        totals
    }

    pub fn add(&mut self, other: Totals) {
        self.files += other.files;
        self.dirs += other.dirs;
        self.bytes += other.bytes;
    }
}

/// `There were errors …` / `Successfully downloaded …`, as Android Studio
/// reports a transfer.
pub fn transfer_summary(totals: Totals, millis: u128, problems: &[String], download: bool) -> (bool, String) {
    let (past, present) = if download { ("downloaded", "downloading") } else { ("uploaded", "uploading") };
    let plural = |n: u64, word: &str, many: &str| if n == 1 { word.to_owned() } else { many.to_owned() };
    let files = plural(totals.files as u64, "file", "files");
    let dirs = plural(totals.dirs as u64, "directory", "directories");
    let bytes = plural(totals.bytes, "byte", "bytes");
    let n = |v: u64| {
        let digits = v.to_string();
        let mut out = String::new();
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i) % 3 == 0 {
                out.push(',');
            }
            out.push(c);
        }
        out
    };
    let duration = format_duration(millis);
    if problems.is_empty() {
        let text = if totals.dirs > 0 {
            format!(
                "Successfully {past} {} {files} and {} {dirs} for a total size of {} {bytes} in {duration}.",
                n(totals.files as u64),
                n(totals.dirs as u64),
                n(totals.bytes)
            )
        } else {
            format!("Successfully {past} {} {files} for a total of size of {} {bytes} in {duration}.", n(totals.files as u64), n(totals.bytes))
        };
        return (true, text);
    }
    let mut text = format!("There were errors {present} files and/or directories");
    if totals.files > 0 {
        let verb = if totals.files <= 1 { "was" } else { "were" };
        text += &format!(", although {} {files} {verb} successfully {past} in {duration} for a total of size of {} {bytes}", n(totals.files as u64), n(totals.bytes));
    }
    let shown: Vec<&str> = problems.iter().take(10).map(String::as_str).collect();
    text += &format!(":\n  {}{}", shown.join(",\n  "), if problems.len() > 10 { "\n  ..." } else { "" });
    (false, text)
}

const TRANSFER: Duration = Duration::from_secs(60 * 60);
const SHELL: Duration = Duration::from_secs(30);
/// Where files pass through on their way to and from `run-as` folders.
const DEVICE_TEMP: &str = "/data/local/tmp";

/// One device's file system.
#[derive(Clone, Debug)]
pub struct FileSystem {
    pub adb: Adb,
    pub serial: String,
    /// adbd runs as root (`adb root`).
    pub root: bool,
    /// `su 0` works.
    pub su: bool,
}

impl FileSystem {
    /// Probes how root the device is, as Android Studio's capabilities check.
    pub fn open(adb: Adb, serial: &str) -> Self {
        let mut fs = Self { adb, serial: serial.to_owned(), root: false, su: false };
        fs.root = fs.shell_raw("id").is_ok_and(|o| o.stdout.contains("uid=0("));
        if !fs.root {
            fs.su = fs.shell_raw("su 0 id").is_ok_and(|o| o.stdout.contains("uid=0("));
        }
        fs
    }

    fn shell_raw(&self, command: &str) -> std::io::Result<process::Output> {
        self.adb.run(&["-s", &self.serial, "shell", command], SHELL)
    }

    /// `command` as the package (`run-as`), as root (`su 0`), or as is.
    fn wrap(&self, run_as: Option<&str>, command: &str) -> String {
        match run_as {
            Some(package) => format!("run-as {package} {command}"),
            None if self.su => format!("su 0 sh -c {}", quote(command)),
            None => command.to_owned(),
        }
    }

    /// Runs a shell command; its error text when it fails.
    fn shell(&self, run_as: Option<&str>, command: &str) -> Result<String, String> {
        let out = self.shell_raw(&self.wrap(run_as, command)).map_err(|e| e.to_string())?;
        let error = out.stderr.trim();
        // Old adb shells don't pass the exit code through: an error printed is a failure.
        if !out.success || !error.is_empty() {
            let message = if error.is_empty() { out.stdout.trim() } else { error };
            return Err(clean_error(message));
        }
        Ok(out.stdout)
    }

    pub fn has_root(&self) -> bool {
        self.root || self.su
    }

    /// Without root, `/data` can't be listed: Device Explorer shows its
    /// `app`, `data` and `local` folders, the packages under the first two
    /// and `tmp` under the last, as Android Studio does.
    fn synthetic_children(&self, path: &str) -> Option<Result<Vec<Entry>, String>> {
        if self.has_root() {
            return None;
        }
        let dir = |parent: &str, name: &str, run_as: Option<String>| Entry::synthetic(parent, name, EntryKind::Directory, run_as);
        match path.trim_end_matches('/') {
            "/data" => Some(Ok(vec![dir("/data", "app", None), dir("/data", "data", None), dir("/data", "local", None)])),
            "/data/local" => Some(Ok(vec![dir("/data/local", "tmp", None)])),
            "/data/data" => Some(self.packages().map(|packages| packages.iter().map(|p| dir("/data/data", p, Some(p.clone()))).collect())),
            "/data/app" => Some(self.shell(None, "pm list packages -f").map(|text| {
                parse_package_paths(&text)
                    .into_iter()
                    .filter_map(|(package, path)| {
                        let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
                        match segments.as_slice() {
                            ["data", "app", name] => Some(Entry::synthetic("/data/app", name, EntryKind::File, Some(package))),
                            ["data", "app", name, ..] => Some(dir("/data/app", name, Some(package))),
                            _ => None,
                        }
                    })
                    .collect::<Vec<_>>()
            })),
            _ => None,
        }
    }

    /// The entries of a folder, sorted by name as Android Studio's tree.
    pub fn list(&self, path: &str, run_as: Option<&str>) -> Result<Vec<Entry>, String> {
        if run_as.is_none() {
            if let Some(children) = self.synthetic_children(path) {
                return children;
            }
        }
        let dir = if path.ends_with('/') { path.to_owned() } else { format!("{path}/") };
        let out = self.shell_raw(&self.wrap(run_as, &format!("ls -al {}", quote(&dir)))).map_err(|e| e.to_string())?;
        let mut entries = parse_ls(&out.stdout, path, run_as);
        if entries.is_empty() {
            // An error, or an empty folder.
            let error = out.stderr.trim();
            let error = if error.is_empty() && !out.success { out.stdout.trim() } else { error };
            if !error.is_empty() {
                return Err(clean_error(error));
            }
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    /// Whether a symbolic link leads to a folder (`ls -l -d <link>/` lists it).
    pub fn is_dir_link(&self, path: &str, run_as: Option<&str>) -> bool {
        let command = self.wrap(run_as, &format!("ls -l -d {}", quote(&format!("{path}/"))));
        self.shell_raw(&command).is_ok_and(|o| o.stdout.lines().any(|l| LS_LINE.is_match(l.trim_end())))
    }

    /// Which of a folder's symbolic links lead to folders, in one shell call.
    pub fn dir_links(&self, paths: &[String], run_as: Option<&str>) -> std::collections::HashSet<String> {
        if paths.is_empty() {
            return Default::default();
        }
        let list: Vec<String> = paths.iter().map(|p| quote(p)).collect();
        let script = format!("for p in {}; do [ -d \"$p\" ] && echo \"$p\"; done", list.join(" "));
        let command = match run_as {
            Some(package) => format!("run-as {package} sh -c {}", quote(&script)),
            None => self.wrap(None, &script),
        };
        let Ok(out) = self.shell_raw(&command) else { return Default::default() };
        out.stdout.lines().map(|l| l.trim_end_matches('\r').to_owned()).filter(|l| paths.contains(l)).collect()
    }

    pub fn packages(&self) -> Result<Vec<String>, String> {
        self.shell(None, "pm list packages").map(|text| parse_packages(&text))
    }

    fn check_name(name: &str, what: &str) -> Result<(), String> {
        if name.contains('/') {
            return Err(format!("{what} name \"{name}\" contains invalid characters"));
        }
        Ok(())
    }

    pub fn new_file(&self, parent: &str, name: &str, run_as: Option<&str>) -> Result<(), String> {
        Self::check_name(name, "File")?;
        let path = resolve(parent, name);
        if self.shell(run_as, &format!("test -e {}", quote(&path))).is_ok() {
            return Err(format!("File {path} already exists on device"));
        }
        self.shell(run_as, &format!("touch {}", quote(&path))).map(|_| ())
    }

    pub fn new_directory(&self, parent: &str, name: &str, run_as: Option<&str>) -> Result<(), String> {
        Self::check_name(name, "Directory")?;
        self.shell(run_as, &format!("mkdir {}", quote(&resolve(parent, name)))).map(|_| ())
    }

    pub fn delete(&self, entry: &Entry) -> Result<(), String> {
        let recursive = if entry.is_dir() { "-r " } else { "" };
        self.shell(entry.run_as.as_deref(), &format!("rm {recursive}-f {}", quote(&entry.path))).map(|_| ())
    }

    fn adb_transfer(&self, args: &[&str]) -> Result<(), String> {
        let mut all = vec!["-s", self.serial.as_str()];
        all.extend_from_slice(args);
        let out = self.adb.run(&all, TRANSFER).map_err(|e| e.to_string())?;
        if !out.success {
            return Err(clean_error(&out.message()));
        }
        Ok(())
    }

    /// Copies a device file or folder to `local` (the folder or file to create).
    pub fn download(&self, entry: &Entry, local: &Path) -> Result<(), String> {
        let is_dir = entry.is_dir() || (entry.kind == EntryKind::Symlink && self.is_dir_link(&entry.path, entry.run_as.as_deref()));
        match entry.run_as.as_deref() {
            Some(package) if !self.has_root() => self.download_run_as(&entry.path, is_dir, local, package),
            _ if is_dir => {
                std::fs::create_dir_all(local).map_err(|e| e.to_string())?;
                // `adb pull dir/. local` copies the folder's contents into `local`.
                let source = format!("{}/.", entry.path.trim_end_matches('/'));
                self.adb_transfer(&["pull", &source, &local.to_string_lossy()])
            }
            _ => {
                if let Some(parent) = local.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                if self.su && !self.root {
                    // adbd can't read what only root may: copy it out through su first.
                    let temp = format!("{DEVICE_TEMP}/junction-{}", unique());
                    self.shell(None, &format!("cat {} > {}; chmod 644 {}", quote(&entry.path), quote(&temp), quote(&temp)))?;
                    let result = self.adb_transfer(&["pull", &temp, &local.to_string_lossy()]);
                    let _ = self.shell_raw(&format!("rm -f {}", quote(&temp)));
                    return result;
                }
                self.adb_transfer(&["pull", &entry.path, &local.to_string_lossy()])
            }
        }
    }

    /// An app's private files: `run-as <package> cat` streams each one out.
    fn download_run_as(&self, path: &str, is_dir: bool, local: &Path, package: &str) -> Result<(), String> {
        if is_dir {
            std::fs::create_dir_all(local).map_err(|e| e.to_string())?;
            for child in self.list(path, Some(package))? {
                let child_dir = child.is_dir() || (child.kind == EntryKind::Symlink && self.is_dir_link(&child.path, Some(package)));
                self.download_run_as(&child.path, child_dir, &local.join(&child.name), package)?;
            }
            return Ok(());
        }
        let command = format!("run-as {package} cat {}", quote(path));
        let (ok, bytes, stderr) =
            process::run_bytes(&self.adb.path, &["-s", &self.serial, "exec-out", &command], TRANSFER).map_err(|e| e.to_string())?;
        if !ok || (bytes.is_empty() && !stderr.trim().is_empty()) {
            return Err(clean_error(if stderr.trim().is_empty() { "Error reading file" } else { stderr.trim() }));
        }
        if let Some(parent) = local.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(local, bytes).map_err(|e| e.to_string())
    }

    /// Copies a local file or folder into a device folder.
    pub fn upload(&self, local: &Path, parent: &str, run_as: Option<&str>) -> Result<(), String> {
        let name = local.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let target = resolve(parent, &name);
        let local_text = local.to_string_lossy().into_owned();
        match run_as {
            Some(package) if !self.has_root() => {
                // Push to a temporary place, then copy in as the app.
                let temp = format!("{DEVICE_TEMP}/junction-{}", unique());
                self.adb_transfer(&["push", &local_text, &temp])?;
                let result = self.shell(Some(package), &format!("cp -r {} {}", quote(&temp), quote(&target))).map(|_| ());
                let _ = self.shell_raw(&format!("rm -r -f {}", quote(&temp)));
                result
            }
            _ if self.su && !self.root => {
                let temp = format!("{DEVICE_TEMP}/junction-{}", unique());
                self.adb_transfer(&["push", &local_text, &temp])?;
                let result = self.shell(None, &format!("cp -r {} {}", quote(&temp), quote(&target))).map(|_| ());
                let _ = self.shell_raw(&format!("rm -r -f {}", quote(&temp)));
                result
            }
            _ => self.adb_transfer(&["push", &local_text, &target]),
        }
    }
}

/// `ls: /data: Permission denied` → `/data: Permission denied`.
fn clean_error(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or(text).trim();
    let line = line.strip_prefix("adb: error: ").unwrap_or(line);
    for tool in ["ls: ", "rm: ", "mkdir: ", "touch: ", "cp: ", "cat: ", "run-as: "] {
        if let Some(rest) = line.strip_prefix(tool) {
            return rest.to_owned();
        }
    }
    line.to_owned()
}

fn unique() -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{:x}{:x}", std::process::id(), nanos)
}

/// Where Open copies a device file: one folder per device, mirroring its paths.
pub fn local_copy(serial: &str, device_path: &str) -> PathBuf {
    let safe: String = serial.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' }).collect();
    let mut path = super::sdk::home()
        .map(|h| h.join(".cache"))
        .unwrap_or_else(std::env::temp_dir)
        .join("Junction")
        .join("device-explorer")
        .join(safe);
    for segment in device_path.split('/').filter(|s| !s.is_empty() && *s != "." && *s != "..") {
        // Characters Windows doesn't allow in names.
        let segment: String = segment.chars().map(|c| if "<>:\"|?*\\".contains(c) { '_' } else { c }).collect();
        path.push(segment);
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ls() {
        let text = "total 52\n\
            drwxr-xr-x  20 root   root      4096 2024-05-30 18:40 .\n\
            drwxr-xr-x  20 root   root      4096 2024-05-30 18:40 ..\n\
            dr-xr-xr-x  41 root   root         0 2024-06-01 09:12 acct\n\
            lrw-r--r--   1 root   root        11 2024-05-30 18:40 bin -> /system/bin\n\
            crw-rw-rw-   1 root   root      1,   3 2024-06-01 09:12 null\n\
            -rw-r--r--   1 root   root      1234 2024-05-30 18:40 init.rc\n\
            drwxrwx--x  49 system system    4096 2024-06-01 09:12 my dir\n\
            ls: ./cache: Permission denied\n";
        let entries = parse_ls(text, "/", None);
        assert_eq!(entries.len(), 5);
        assert_eq!((entries[0].path.as_str(), entries[0].kind, entries[0].date.as_str(), entries[0].size), ("/acct", EntryKind::Directory, "2024-06-01 09:12", Some(0)));
        assert_eq!((entries[1].name.as_str(), entries[1].link_target.as_deref(), entries[1].kind), ("bin", Some("/system/bin"), EntryKind::Symlink));
        assert_eq!((entries[2].kind, entries[2].size), (EntryKind::Character, None));
        assert_eq!((entries[3].size, entries[3].permissions.as_str()), (Some(1234), "-rw-r--r--"));
        assert_eq!(entries[4].path, "/my dir");
        // Old `ls -l` (no link count), a folder below the root.
        let old = parse_ls("drwxr-xr-x root     root              2017-01-01 00:00 obb\n-rw-rw---- root sdcard_rw 5 2017-01-01 00:00 a.txt", "/sdcard", None);
        assert_eq!((old[0].path.as_str(), old[0].size), ("/sdcard/obb", None));
        assert_eq!(old[1].size, Some(5));
    }

    #[test]
    fn helpers() {
        assert_eq!(parent("/sdcard/Download/a.txt"), "/sdcard/Download");
        assert_eq!(parent("/sdcard"), "/");
        assert_eq!(quote("it's"), "'it'\\''s'");
        assert_eq!(humanized_size(890), "890 B");
        assert_eq!(humanized_size(68_608), "67 KB");
        assert_eq!(humanized_size(234_567_890), "223.7 MB");
        assert_eq!(humanized_size(1_234_567_890), "1.1 GB");
        assert_eq!(format_duration(65_432), "1 m 5 s 432 ms");
        assert_eq!(format_duration(0), "0 ms");
        assert_eq!(parse_packages("package:b.app\npackage:a.app\n"), vec!["a.app", "b.app"]);
        assert_eq!(
            parse_package_paths("package:/data/app/~~x==/com.example-y==/base.apk=com.example\n"),
            vec![("com.example".into(), "/data/app/~~x==/com.example-y==/base.apk".into())]
        );
        assert_eq!(clean_error("ls: /data: Permission denied\n"), "/data: Permission denied");
    }

    #[test]
    fn summaries() {
        let (ok, text) = transfer_summary(Totals { files: 2, dirs: 1, bytes: 12_345 }, 1_500, &[], true);
        assert!(ok);
        assert_eq!(text, "Successfully downloaded 2 files and 1 directory for a total size of 12,345 bytes in 1 s 500 ms.");
        let (ok, text) = transfer_summary(Totals { files: 1, dirs: 0, bytes: 5 }, 20, &[], false);
        assert!(ok);
        assert_eq!(text, "Successfully uploaded 1 file for a total of size of 5 bytes in 20 ms.");
        let (ok, text) = transfer_summary(Totals::default(), 20, &["/a: Permission denied".into()], true);
        assert!(!ok);
        assert_eq!(text, "There were errors downloading files and/or directories:\n  /a: Permission denied");
    }
}
