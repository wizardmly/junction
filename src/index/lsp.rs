//! A minimal Language Server Protocol client: one server process per
//! command and project, JSON-RPC over stdio, used for Go to Declaration,
//! Find Usages and Quick Documentation when the server is installed.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use super::lang::Lang;

struct Config {
    enabled: bool,
    overrides: BTreeMap<String, String>,
}

static CONFIG: RwLock<Config> = RwLock::new(Config { enabled: true, overrides: BTreeMap::new() });

/// Called when settings change.
pub fn configure(enabled: bool, overrides: BTreeMap<String, String>) {
    if let Ok(mut config) = CONFIG.write() {
        config.enabled = enabled;
        config.overrides = overrides;
    }
}

/// Whether a command's program is on PATH (or an existing path).
pub fn find_program(program: &str) -> Option<PathBuf> {
    let candidate = Path::new(program);
    if candidate.components().count() > 1 {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    let path = std::env::var_os("PATH")?;
    let exts: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ".bat", ""] } else { &[""] };
    for dir in std::env::split_paths(&path) {
        for ext in exts {
            let full = dir.join(format!("{program}{ext}"));
            if full.is_file() {
                return Some(full);
            }
        }
    }
    None
}

/// The server command for a language: the user's override, else the first
/// default whose program is installed. `None` when off or not installed.
pub fn command_for(lang: Lang) -> Option<String> {
    let config = CONFIG.read().ok()?;
    if !config.enabled {
        return None;
    }
    // TSX is served by the TypeScript server.
    let lang = if lang == Lang::Tsx { Lang::TypeScript } else { lang };
    if let Some(command) = config.overrides.get(lang.key()) {
        let command = command.trim();
        return (!command.is_empty() && command != "off").then(|| command.to_owned());
    }
    detected(lang)
}

/// The first default server for a language that is installed.
pub fn detected(lang: Lang) -> Option<String> {
    lang.default_servers().iter().find(|c| c.split_whitespace().next().and_then(find_program).is_some()).map(|c| c.to_string())
}

pub fn path_to_uri(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file://");
    if !s.starts_with('/') {
        out.push('/');
    }
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&rest[i + 1..i + 3], 16) {
                decoded.push(b);
                i += 3;
                continue;
            }
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    let mut s = String::from_utf8(decoded).ok()?;
    // file:///C:/x → C:/x on Windows.
    if s.len() > 2 && s.as_bytes()[0] == b'/' && s.as_bytes()[2] == b':' {
        s.remove(0);
    }
    Some(PathBuf::from(s))
}

/// A location a server answered with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub path: PathBuf,
    pub line: u32,
    pub col: u32,
}

fn parse_locations(value: &Value) -> Vec<Location> {
    let items: Vec<&Value> = match value {
        Value::Array(items) => items.iter().collect(),
        Value::Object(_) => vec![value],
        _ => Vec::new(),
    };
    items
        .into_iter()
        .filter_map(|item| {
            // Location { uri, range } or LocationLink { targetUri, targetSelectionRange }.
            let uri = item.get("uri").or_else(|| item.get("targetUri"))?.as_str()?;
            let range = item.get("range").or_else(|| item.get("targetSelectionRange"))?;
            let start = range.get("start")?;
            Some(Location {
                path: uri_to_path(uri)?,
                line: start.get("line")?.as_u64()? as u32,
                col: start.get("character")?.as_u64()? as u32,
            })
        })
        .collect()
}

type Pending = Arc<Mutex<HashMap<i64, Sender<Result<Value, String>>>>>;

pub struct LspClient {
    stdin: Mutex<ChildStdin>,
    child: Mutex<Child>,
    pending: Pending,
    next_id: AtomicI64,
    /// Open documents: version and the text last sent.
    docs: Mutex<HashMap<String, (i32, u64)>>,
    /// The server's work-done progress ("Indexing…"), if any.
    pub progress: Arc<Mutex<Option<String>>>,
    alive: Arc<std::sync::atomic::AtomicBool>,
}

fn write_message(stdin: &mut ChildStdin, value: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(value)?;
    write!(stdin, "Content-Length: {}\r\n\r\n", body.len())?;
    stdin.write_all(&body)?;
    stdin.flush()
}

fn read_message(reader: &mut impl BufRead) -> Option<Value> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(v) = line.strip_prefix("Content-Length:") {
            length = v.trim().parse::<usize>().ok();
        }
    }
    let mut body = vec![0; length?];
    reader.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

fn hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

impl LspClient {
    /// Starts a server in `root` and completes the initialize handshake.
    pub fn start(command: &str, root: &Path) -> Result<Arc<Self>> {
        let mut parts = command.split_whitespace();
        let program = parts.next().ok_or_else(|| anyhow!("empty server command"))?;
        let program_path = find_program(program).ok_or_else(|| anyhow!("{program} is not installed"))?;
        let mut cmd = Command::new(program_path);
        cmd.args(parts).current_dir(root).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        let pending: Pending = Arc::default();
        let progress: Arc<Mutex<Option<String>>> = Arc::default();
        let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let client = Arc::new(Self {
            stdin: Mutex::new(stdin),
            child: Mutex::new(child),
            pending: pending.clone(),
            next_id: AtomicI64::new(1),
            docs: Mutex::default(),
            progress: progress.clone(),
            alive: alive.clone(),
        });
        let weak = Arc::downgrade(&client);
        std::thread::Builder::new().name(format!("lsp {program}")).spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut titles: HashMap<String, String> = HashMap::new();
            while let Some(message) = read_message(&mut reader) {
                let method = message.get("method").and_then(Value::as_str);
                match (method, message.get("id")) {
                    // A response to one of our requests.
                    (None, Some(id)) => {
                        let Some(id) = id.as_i64() else { continue };
                        if let Some(tx) = pending.lock().ok().and_then(|mut p| p.remove(&id)) {
                            let result = match message.get("error") {
                                Some(error) => Err(error.get("message").and_then(Value::as_str).unwrap_or("error").to_owned()),
                                None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                            };
                            tx.send(result).ok();
                        }
                    }
                    // A request from the server: answer what a passive client can.
                    (Some(method), Some(id)) => {
                        let result = match method {
                            "workspace/configuration" => {
                                let n = message.pointer("/params/items").and_then(Value::as_array).map_or(0, Vec::len);
                                Value::Array(vec![Value::Null; n])
                            }
                            "workspace/workspaceFolders" => Value::Null,
                            _ => Value::Null,
                        };
                        if let Some(client) = weak.upgrade() {
                            if let Ok(mut stdin) = client.stdin.lock() {
                                write_message(&mut stdin, &json!({ "jsonrpc": "2.0", "id": id, "result": result })).ok();
                            }
                        }
                    }
                    (Some("$/progress"), None) => {
                        let token = message.pointer("/params/token").map(|t| t.to_string()).unwrap_or_default();
                        let value = message.pointer("/params/value");
                        let kind = value.and_then(|v| v.get("kind")).and_then(Value::as_str);
                        match kind {
                            Some("begin") => {
                                let title = value.and_then(|v| v.get("title")).and_then(Value::as_str).unwrap_or("Working").to_owned();
                                titles.insert(token, title.clone());
                                *progress.lock().unwrap() = Some(title);
                            }
                            Some("report") => {
                                let title = titles.get(&token).cloned().unwrap_or_default();
                                let detail = value.and_then(|v| v.get("message")).and_then(Value::as_str).unwrap_or_default();
                                *progress.lock().unwrap() = Some(format!("{title} {detail}").trim().to_owned());
                            }
                            Some("end") => {
                                titles.remove(&token);
                                *progress.lock().unwrap() = titles.values().next().cloned();
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            alive.store(false, Ordering::Relaxed);
            if let Ok(mut pending) = pending.lock() {
                for (_, tx) in pending.drain() {
                    tx.send(Err("language server exited".into())).ok();
                }
            }
        })?;

        let root_uri = path_to_uri(root);
        let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        client.request(
            "initialize",
            json!({
                "processId": std::process::id(),
                "rootUri": root_uri,
                "rootPath": root.to_string_lossy(),
                "workspaceFolders": [{ "uri": root_uri, "name": name }],
                "clientInfo": { "name": "GitGlass" },
                "capabilities": {
                    "general": { "positionEncodings": ["utf-16"] },
                    "window": { "workDoneProgress": true },
                    "workspace": { "workspaceFolders": true, "configuration": true },
                    "textDocument": {
                        "synchronization": { "didSave": true, "dynamicRegistration": false },
                        "definition": { "linkSupport": true },
                        "declaration": { "linkSupport": true },
                        "implementation": { "linkSupport": true },
                        "references": {},
                        "hover": { "contentFormat": ["markdown", "plaintext"] },
                        "documentSymbol": { "hierarchicalDocumentSymbolSupport": true }
                    }
                }
            }),
            Duration::from_secs(90),
        )?;
        client.notify("initialized", json!({}))?;
        Ok(client)
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    pub fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        if !self.is_alive() {
            bail!("language server exited");
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().map_err(|_| anyhow!("poisoned"))?.insert(id, tx);
        {
            let mut stdin = self.stdin.lock().map_err(|_| anyhow!("poisoned"))?;
            write_message(&mut stdin, &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        }
        match rx.recv_timeout(timeout) {
            Ok(result) => result.map_err(|e| anyhow!("{e}")),
            Err(_) => {
                self.pending.lock().ok().map(|mut p| p.remove(&id));
                // Let the server stop working on it.
                self.notify("$/cancelRequest", json!({ "id": id })).ok();
                bail!("{method} timed out")
            }
        }
    }

    pub fn notify(&self, method: &str, params: Value) -> Result<()> {
        let mut stdin = self.stdin.lock().map_err(|_| anyhow!("poisoned"))?;
        write_message(&mut stdin, &json!({ "jsonrpc": "2.0", "method": method, "params": params }))?;
        Ok(())
    }

    /// Sends the editor's text: didOpen the first time, didChange after.
    pub fn sync(&self, path: &Path, lang: Lang, text: &str) -> Result<String> {
        let uri = path_to_uri(path);
        let h = hash(text);
        let mut docs = self.docs.lock().map_err(|_| anyhow!("poisoned"))?;
        match docs.get_mut(&uri) {
            None => {
                self.notify("textDocument/didOpen", json!({ "textDocument": { "uri": uri, "languageId": lang.language_id(), "version": 1, "text": text } }))?;
                docs.insert(uri.clone(), (1, h));
            }
            Some((version, last)) if *last != h => {
                *version += 1;
                *last = h;
                self.notify("textDocument/didChange", json!({ "textDocument": { "uri": uri, "version": *version }, "contentChanges": [{ "text": text }] }))?;
            }
            Some(_) => {}
        }
        Ok(uri)
    }

    fn position_params(&self, path: &Path, lang: Lang, text: &str, line: u32, col: u32) -> Result<Value> {
        let uri = self.sync(path, lang, text)?;
        Ok(json!({ "textDocument": { "uri": uri }, "position": { "line": line, "character": col } }))
    }

    pub fn definition(&self, path: &Path, lang: Lang, text: &str, line: u32, col: u32, timeout: Duration) -> Result<Vec<Location>> {
        let params = self.position_params(path, lang, text, line, col)?;
        let result = self.request("textDocument/definition", params, timeout)?;
        Ok(parse_locations(&result))
    }

    pub fn references(&self, path: &Path, lang: Lang, text: &str, line: u32, col: u32) -> Result<Vec<Location>> {
        let mut params = self.position_params(path, lang, text, line, col)?;
        params["context"] = json!({ "includeDeclaration": true });
        let result = self.request("textDocument/references", params, Duration::from_secs(30))?;
        Ok(parse_locations(&result))
    }

    /// Quick Documentation as markdown.
    pub fn hover(&self, path: &Path, lang: Lang, text: &str, line: u32, col: u32, timeout: Duration) -> Result<Option<String>> {
        let params = self.position_params(path, lang, text, line, col)?;
        let result = self.request("textDocument/hover", params, timeout)?;
        let contents = match result.get("contents") {
            Some(c) => c,
            None => return Ok(None),
        };
        let piece = |v: &Value| -> String {
            match v {
                Value::String(s) => s.clone(),
                Value::Object(o) => {
                    let value = o.get("value").and_then(Value::as_str).unwrap_or_default();
                    match o.get("language").and_then(Value::as_str) {
                        Some(language) => format!("```{language}\n{value}\n```"),
                        None => value.to_owned(),
                    }
                }
                _ => String::new(),
            }
        };
        let text = match contents {
            Value::Array(items) => items.iter().map(piece).collect::<Vec<_>>().join("\n\n"),
            other => piece(other),
        };
        Ok((!text.trim().is_empty()).then_some(text))
    }

    pub fn shutdown(&self) {
        if self.is_alive() {
            self.request("shutdown", Value::Null, Duration::from_secs(2)).ok();
            self.notify("exit", Value::Null).ok();
        }
        if let Ok(mut child) = self.child.lock() {
            std::thread::sleep(Duration::from_millis(50));
            child.kill().ok();
            child.wait().ok();
        }
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            child.kill().ok();
            child.wait().ok();
        }
    }
}

enum Slot {
    Running(Arc<LspClient>),
    Failed(String),
}

/// The servers of one project, started on first use.
#[derive(Default)]
pub struct LspManager {
    root: PathBuf,
    slots: Mutex<HashMap<String, Slot>>,
}

impl LspManager {
    pub fn new(root: PathBuf) -> Self {
        Self { root, slots: Mutex::default() }
    }

    /// The directory a server for `lang` should treat as its workspace when
    /// working on `file`: the nearest enclosing project (Cargo.toml, go.mod,
    /// pubspec.yaml, …) inside the repository, else the repository itself.
    /// Gradle and Maven builds use the outermost one, so a module opens its
    /// whole build.
    pub fn workspace_for(&self, lang: Lang, file: &Path) -> PathBuf {
        let markers = lang.project_markers();
        let outermost = matches!(lang, Lang::Java | Lang::Kotlin);
        let mut found = None;
        let mut dir = file.parent();
        while let Some(d) = dir {
            if !d.starts_with(&self.root) {
                break;
            }
            if markers.iter().any(|m| d.join(m).exists()) {
                found = Some(d.to_path_buf());
                if !outermost {
                    break;
                }
            }
            if d == self.root {
                break;
            }
            dir = d.parent();
        }
        found.unwrap_or_else(|| self.root.clone())
    }

    /// The running server for a language and file, starting it if needed.
    /// Blocks while it starts; call off the main thread.
    pub fn client(&self, lang: Lang, file: &Path) -> Option<Arc<LspClient>> {
        let command = command_for(lang)?;
        let root = self.workspace_for(lang, file);
        let key = format!("{command}\0{}", root.display());
        // Start outside the lock so one slow server doesn't block others.
        {
            let slots = self.slots.lock().ok()?;
            match slots.get(&key) {
                Some(Slot::Running(client)) if client.is_alive() => return Some(client.clone()),
                Some(Slot::Failed(_)) => return None,
                _ => {}
            }
        }
        let started = LspClient::start(&command, &root);
        let mut slots = self.slots.lock().ok()?;
        match started {
            Ok(client) => {
                slots.insert(key, Slot::Running(client.clone()));
                Some(client)
            }
            Err(error) => {
                slots.insert(key, Slot::Failed(error.to_string()));
                None
            }
        }
    }

    /// For the status bar: each server and its state.
    pub fn status(&self) -> Vec<(String, String)> {
        let Ok(slots) = self.slots.lock() else { return Vec::new() };
        let mut out: Vec<(String, String)> = slots
            .iter()
            .map(|(key, slot)| {
                let (command, root) = key.split_once('\0').unwrap_or((key, ""));
                let mut name = command.split_whitespace().next().unwrap_or(command).to_owned();
                // Several workspaces of one server: say which.
                if Path::new(root) != self.root {
                    if let Ok(rel) = Path::new(root).strip_prefix(&self.root) {
                        name = format!("{name} ({})", rel.to_string_lossy().replace('\\', "/"));
                    }
                }
                let state = match slot {
                    Slot::Running(c) if !c.is_alive() => "stopped".to_owned(),
                    Slot::Running(c) => c.progress.lock().ok().and_then(|p| p.clone()).unwrap_or_else(|| "ready".into()),
                    Slot::Failed(e) => format!("failed: {e}"),
                };
                (name, state)
            })
            .collect();
        out.sort();
        out
    }

    pub fn shutdown(&self) {
        if let Ok(mut slots) = self.slots.lock() {
            for (_, slot) in slots.drain() {
                if let Slot::Running(client) = slot {
                    client.shutdown();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_round_trip() {
        let p = Path::new("/home/me/my project/src/a#b.rs");
        let uri = path_to_uri(p);
        assert_eq!(uri, "file:///home/me/my%20project/src/a%23b.rs");
        assert_eq!(uri_to_path(&uri).unwrap(), p);
        assert_eq!(uri_to_path("file:///C:/x/y.rs").unwrap(), PathBuf::from("C:/x/y.rs"));
    }

    #[test]
    fn parses_location_shapes() {
        let link = json!([{ "targetUri": "file:///a.rs", "targetRange": {}, "targetSelectionRange": { "start": { "line": 3, "character": 4 }, "end": { "line": 3, "character": 8 } } }]);
        assert_eq!(parse_locations(&link), [Location { path: "/a.rs".into(), line: 3, col: 4 }]);
        let single = json!({ "uri": "file:///b.rs", "range": { "start": { "line": 1, "character": 0 }, "end": { "line": 1, "character": 2 } } });
        assert_eq!(parse_locations(&single), [Location { path: "/b.rs".into(), line: 1, col: 0 }]);
        assert!(parse_locations(&Value::Null).is_empty());
    }

    /// Against a real server when one is installed (skipped otherwise).
    #[test]
    fn talks_to_pyright() {
        if find_program("pyright-langserver").is_none() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("gitglass-lsp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let text = "def greet(name):\n    return name\n\ngreet('x')\n";
        let file = dir.join("main.py");
        std::fs::write(&file, text).unwrap();
        let client = LspClient::start("pyright-langserver --stdio", &dir).unwrap();
        let mut found = Vec::new();
        // The first answers can come before the server has analyzed the file.
        for _ in 0..20 {
            found = client.definition(&file, Lang::Python, text, 3, 1, Duration::from_secs(15)).unwrap_or_default();
            if !found.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        assert_eq!(found.first().map(|l| (l.line, l.col)), Some((0, 4)));
        client.shutdown();
        std::fs::remove_dir_all(&dir).ok();
    }
}
