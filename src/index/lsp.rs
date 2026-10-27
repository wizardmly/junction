//! A minimal Language Server Protocol client: one server process per
//! command and project, JSON-RPC over stdio, used for Go to Declaration,
//! Find Usages and Quick Documentation when the server is installed, and
//! the problems it publishes for the editor.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
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
    /// Edits the server asked to apply (`workspace/applyEdit`) while one
    /// of its commands ran.
    applied: Arc<Mutex<Vec<FileEdit>>>,
    /// The diagnostics it published, by URI, for code actions' context
    /// and the editor.
    diagnostics: Arc<Mutex<HashMap<String, Value>>>,
    /// Counts the diagnostics it has published, so a reader can tell
    /// whether anything changed.
    diagnostics_rev: Arc<AtomicU64>,
    /// The code action kinds it says it offers, when it says.
    code_action_kinds: Mutex<Option<Vec<String>>>,
}

/// A problem a server published: 0-based lines and UTF-16 columns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerDiagnostic {
    pub start: (u32, u32),
    pub end: (u32, u32),
    /// 1 error, 2 warning, 3 information, 4 hint (LSP's DiagnosticSeverity;
    /// a missing one is an error).
    pub severity: u8,
    pub message: String,
}

/// The diagnostics of a `publishDiagnostics` notification.
pub fn parse_diagnostics(list: &Value) -> Vec<ServerDiagnostic> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| {
            let pos = |p: &str| Some((d.pointer(&format!("/range/{p}/line"))?.as_u64()? as u32, d.pointer(&format!("/range/{p}/character"))?.as_u64()? as u32));
            let severity = d.get("severity").and_then(Value::as_u64).unwrap_or(1).clamp(1, 4) as u8;
            Some(ServerDiagnostic { start: pos("start")?, end: pos("end")?, severity, message: d.get("message")?.as_str()?.to_owned() })
        })
        .collect()
}

/// A completion answer's items: a list, or `{ isIncomplete, items }`;
/// items that don't parse are left out.
pub fn parse_completions(result: &Value) -> Vec<lsp_types::CompletionItem> {
    let items = match result {
        Value::Array(items) => items,
        Value::Object(list) => match list.get("items") {
            Some(Value::Array(items)) => items,
            _ => return Vec::new(),
        },
        _ => return Vec::new(),
    };
    items.iter().filter_map(|i| serde_json::from_value(i.clone()).ok()).collect()
}

/// A text edit a server answered with: 0-based line and UTF-16 column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub start: (u32, u32),
    pub end: (u32, u32),
    pub text: String,
}

/// A file's edits from a workspace edit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEdit {
    pub path: PathBuf,
    pub edits: Vec<TextEdit>,
}

/// The code action kind of a server's Organize Imports.
pub const ORGANIZE_IMPORTS: &str = "source.organizeImports";

/// A code action a server offers: a quick fix, a refactoring.
#[derive(Clone, Debug)]
pub struct CodeAction {
    pub title: String,
    pub kind: Option<String>,
    pub edit: Option<Vec<FileEdit>>,
    pub command: Option<Value>,
    /// The action as sent, for `codeAction/resolve`.
    pub raw: Value,
    pub preferred: bool,
}

fn parse_text_edits(value: &Value) -> Vec<TextEdit> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let range = e.get("range")?;
            let pos = |p: &Value| Some((p.get("line")?.as_u64()? as u32, p.get("character")?.as_u64()? as u32));
            Some(TextEdit { start: pos(range.get("start")?)?, end: pos(range.get("end")?)?, text: e.get("newText")?.as_str()?.to_owned() })
        })
        .collect()
}

/// A WorkspaceEdit's text edits (`changes` or `documentChanges`; file
/// creations and renames aren't applied).
pub fn parse_workspace_edit(value: &Value) -> Vec<FileEdit> {
    let mut out = Vec::new();
    if let Some(changes) = value.get("changes").and_then(Value::as_object) {
        for (uri, edits) in changes {
            if let Some(path) = uri_to_path(uri) {
                out.push(FileEdit { path, edits: parse_text_edits(edits) });
            }
        }
    }
    for change in value.get("documentChanges").and_then(Value::as_array).into_iter().flatten() {
        let Some(uri) = change.pointer("/textDocument/uri").and_then(Value::as_str) else { continue };
        if let Some(path) = uri_to_path(uri) {
            out.push(FileEdit { path, edits: parse_text_edits(change.get("edits").unwrap_or(&Value::Null)) });
        }
    }
    out
}

fn parse_code_action(value: &Value) -> Option<CodeAction> {
    // A bare Command, or a CodeAction.
    if value.get("command").is_some_and(Value::is_string) {
        return Some(CodeAction { title: value.get("title")?.as_str()?.to_owned(), kind: None, edit: None, command: Some(value.clone()), raw: value.clone(), preferred: false });
    }
    if value.get("disabled").is_some() {
        return None;
    }
    Some(CodeAction {
        title: value.get("title")?.as_str()?.to_owned(),
        kind: value.get("kind").and_then(Value::as_str).map(str::to_owned),
        edit: value.get("edit").map(parse_workspace_edit),
        command: value.get("command").cloned(),
        raw: value.clone(),
        preferred: value.get("isPreferred").and_then(Value::as_bool).unwrap_or(false),
    })
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
        let applied: Arc<Mutex<Vec<FileEdit>>> = Arc::default();
        let diagnostics: Arc<Mutex<HashMap<String, Value>>> = Arc::default();
        let diagnostics_rev: Arc<AtomicU64> = Arc::default();
        let (applied_rx, diagnostics_rx, rev_rx) = (applied.clone(), diagnostics.clone(), diagnostics_rev.clone());
        let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let client = Arc::new(Self {
            stdin: Mutex::new(stdin),
            child: Mutex::new(child),
            pending: pending.clone(),
            next_id: AtomicI64::new(1),
            docs: Mutex::default(),
            progress: progress.clone(),
            alive: alive.clone(),
            applied,
            diagnostics,
            diagnostics_rev,
            code_action_kinds: Mutex::default(),
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
                            // A command's edits: applied by whoever ran it.
                            "workspace/applyEdit" => {
                                let edit = message.pointer("/params/edit").cloned().unwrap_or(Value::Null);
                                applied_rx.lock().unwrap().extend(parse_workspace_edit(&edit));
                                json!({ "applied": true })
                            }
                            _ => Value::Null,
                        };
                        if let Some(client) = weak.upgrade() {
                            if let Ok(mut stdin) = client.stdin.lock() {
                                write_message(&mut stdin, &json!({ "jsonrpc": "2.0", "id": id, "result": result })).ok();
                            }
                        }
                    }
                    (Some("textDocument/publishDiagnostics"), None) => {
                        if let (Some(uri), Some(list)) = (message.pointer("/params/uri").and_then(Value::as_str), message.pointer("/params/diagnostics")) {
                            diagnostics_rx.lock().unwrap().insert(uri.to_owned(), list.clone());
                            rev_rx.fetch_add(1, Ordering::SeqCst);
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
        let initialized = client.request(
            "initialize",
            json!({
                "processId": std::process::id(),
                "rootUri": root_uri,
                "rootPath": root.to_string_lossy(),
                "workspaceFolders": [{ "uri": root_uri, "name": name }],
                "clientInfo": { "name": "Junction Studio" },
                "capabilities": {
                    "general": { "positionEncodings": ["utf-16"] },
                    "window": { "workDoneProgress": true },
                    "workspace": {
                        "workspaceFolders": true,
                        "configuration": true,
                        "applyEdit": true,
                        "workspaceEdit": { "documentChanges": true }
                    },
                    "textDocument": {
                        "synchronization": { "didSave": true, "dynamicRegistration": false },
                        "definition": { "linkSupport": true },
                        "declaration": { "linkSupport": true },
                        "implementation": { "linkSupport": true },
                        "references": {},
                        "hover": { "contentFormat": ["markdown", "plaintext"] },
                        "completion": {
                            "contextSupport": true,
                            "completionItem": { "snippetSupport": false, "labelDetailsSupport": true, "deprecatedSupport": true, "resolveSupport": { "properties": ["additionalTextEdits"] } }
                        },
                        "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
                        "rename": { "prepareSupport": true },
                        "publishDiagnostics": {},
                        "codeAction": {
                            "codeActionLiteralSupport": { "codeActionKind": { "valueSet": ["", "quickfix", "refactor", "refactor.extract", "refactor.inline", "refactor.rewrite", "source", "source.organizeImports"] } },
                            "resolveSupport": { "properties": ["edit"] },
                            "dataSupport": true,
                            "isPreferredSupport": true
                        }
                    }
                }
            }),
            Duration::from_secs(90),
        )?;
        if let (Ok(mut kinds), Some(list)) = (client.code_action_kinds.lock(), initialized.pointer("/capabilities/codeActionProvider/codeActionKinds").and_then(Value::as_array)) {
            *kinds = Some(list.iter().filter_map(Value::as_str).map(str::to_owned).collect());
        }
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

    /// How many times the server has published diagnostics.
    pub fn diagnostics_revision(&self) -> u64 {
        self.diagnostics_rev.load(Ordering::SeqCst)
    }

    /// What the server last published for a file; none before it has.
    pub fn published_diagnostics(&self, path: &Path) -> Option<Vec<ServerDiagnostic>> {
        let uri = path_to_uri(path);
        let all = self.diagnostics.lock().ok()?;
        // Servers spell URIs their own way (`c%3A`, `C:`): compare paths too.
        let list = all.get(&uri).or_else(|| all.iter().find(|(u, _)| uri_to_path(u).as_deref() == Some(path)).map(|(_, l)| l))?;
        Some(parse_diagnostics(list))
    }

    /// Everything the server has published, by file: open or not (a
    /// project-wide check after indexing or a save).
    pub fn all_published(&self) -> Vec<(PathBuf, Vec<ServerDiagnostic>)> {
        let Ok(all) = self.diagnostics.lock() else { return Vec::new() };
        all.iter().filter_map(|(uri, list)| Some((uri_to_path(uri)?, parse_diagnostics(list)))).collect()
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

    /// Completions at a position (0-based line, UTF-16 column): `after_dot`
    /// says the caret follows a `.`, the trigger character.
    pub fn completion(&self, path: &Path, lang: Lang, text: &str, line: u32, col: u32, after_dot: bool, timeout: Duration) -> Result<Vec<lsp_types::CompletionItem>> {
        let mut params = self.position_params(path, lang, text, line, col)?;
        params["context"] = if after_dot { json!({ "triggerKind": 2, "triggerCharacter": "." }) } else { json!({ "triggerKind": 1 }) };
        let result = self.request("textDocument/completion", params, timeout)?;
        Ok(parse_completions(&result))
    }

    /// The edits a completion brings once resolved (an auto-import that the
    /// server computes only for the picked item), 0-based lines and UTF-16
    /// columns.
    pub fn resolve_completion(&self, item: &lsp_types::CompletionItem, timeout: Duration) -> Result<Vec<TextEdit>> {
        let result = self.request("completionItem/resolve", serde_json::to_value(item)?, timeout)?;
        Ok(result.get("additionalTextEdits").map(parse_text_edits).unwrap_or_default())
    }

    /// The code actions for a range (0-based lines, UTF-16 columns), with
    /// the server's own diagnostics there as their context.
    pub fn code_actions(&self, path: &Path, lang: Lang, text: &str, start: (u32, u32), end: (u32, u32), timeout: Duration) -> Result<Vec<CodeAction>> {
        let uri = self.sync(path, lang, text)?;
        let range = json!({ "start": { "line": start.0, "character": start.1 }, "end": { "line": end.0, "character": end.1 } });
        let diagnostics: Vec<Value> = self
            .diagnostics
            .lock()
            .ok()
            .and_then(|d| d.get(&uri).cloned())
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .filter(|d| {
                let line = |p: &str| d.pointer(p).and_then(Value::as_u64).unwrap_or(0) as u32;
                line("/range/start/line") <= end.0 && line("/range/end/line") >= start.0
            })
            .collect();
        let params = json!({ "textDocument": { "uri": uri }, "range": range, "context": { "diagnostics": diagnostics, "triggerKind": 1 } });
        let result = self.request("textDocument/codeAction", params, timeout)?;
        Ok(result.as_array().into_iter().flatten().filter_map(parse_code_action).collect())
    }

    /// Whether the server offers code actions of this kind; `None` when
    /// it didn't list its kinds.
    pub fn offers(&self, kind: &str) -> Option<bool> {
        let kinds = self.code_action_kinds.lock().ok()?;
        Some(kinds.as_ref()?.iter().any(|k| k == kind || k.starts_with(&format!("{kind}."))))
    }

    /// The server's Organize Imports for the whole file (a code action of
    /// kind `source.organizeImports`); none when it has no such action.
    pub fn organize_imports(&self, path: &Path, lang: Lang, text: &str, timeout: Duration) -> Result<Option<CodeAction>> {
        let uri = self.sync(path, lang, text)?;
        let lines = text.split('\n').count() as u32;
        let range = json!({ "start": { "line": 0, "character": 0 }, "end": { "line": lines, "character": 0 } });
        let params = json!({ "textDocument": { "uri": uri }, "range": range, "context": { "diagnostics": [], "only": [ORGANIZE_IMPORTS], "triggerKind": 1 } });
        let result = self.request("textDocument/codeAction", params, timeout)?;
        Ok(result.as_array().into_iter().flatten().filter_map(parse_code_action).find(|a| a.kind.as_deref().is_some_and(|k| k == ORGANIZE_IMPORTS || k.starts_with("source.organizeImports."))))
    }

    /// The edits of a code action: its own, resolved, or what its command
    /// asked to apply.
    pub fn code_action_edits(&self, action: &CodeAction) -> Result<Vec<FileEdit>> {
        let mut edits = action.edit.clone().unwrap_or_default();
        let mut command = action.command.clone();
        if action.edit.is_none() && action.raw.get("data").is_some() {
            let resolved = self.request("codeAction/resolve", action.raw.clone(), Duration::from_secs(10))?;
            if let Some(edit) = resolved.get("edit") {
                edits = parse_workspace_edit(edit);
            }
            command = resolved.get("command").cloned().or(command);
        }
        if let Some(command) = command {
            // A Command, or a CodeAction's command field.
            let (name, arguments) = match command.get("command") {
                Some(Value::String(name)) => (name.clone(), command.get("arguments").cloned().unwrap_or(json!([]))),
                _ => return Ok(edits),
            };
            self.applied.lock().map_err(|_| anyhow!("poisoned"))?.clear();
            self.request("workspace/executeCommand", json!({ "command": name, "arguments": arguments }), Duration::from_secs(10))?;
            edits.extend(std::mem::take(&mut *self.applied.lock().map_err(|_| anyhow!("poisoned"))?));
        }
        Ok(edits)
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
    /// Launched; the initialize handshake hasn't finished.
    Starting,
    Running(Arc<LspClient>),
    Failed(String),
}

/// The servers of one project, started on first use.
#[derive(Default)]
pub struct LspManager {
    root: PathBuf,
    slots: Arc<Mutex<HashMap<String, Slot>>>,
}

impl LspManager {
    pub fn new(root: PathBuf) -> Self {
        Self { root, slots: Arc::default() }
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

    /// The running server for a language and file. One that isn't running
    /// yet starts in the background and this returns `None` meanwhile, so
    /// callers answer from the built-in index instead of waiting for it: a
    /// big Rust or Gradle project keeps rust-analyzer or a JVM server in its
    /// handshake for a long time.
    pub fn client(&self, lang: Lang, file: &Path) -> Option<Arc<LspClient>> {
        let command = command_for(lang)?;
        let root = self.workspace_for(lang, file);
        let key = format!("{command}\0{}", root.display());
        {
            let mut slots = self.slots.lock().ok()?;
            match slots.get(&key) {
                Some(Slot::Running(client)) if client.is_alive() => return Some(client.clone()),
                Some(Slot::Failed(_) | Slot::Starting) => return None,
                _ => {}
            }
            slots.insert(key.clone(), Slot::Starting);
        }
        let slots = self.slots.clone();
        std::thread::Builder::new()
            .name(format!("start {}", command.split_whitespace().next().unwrap_or("server")))
            .spawn(move || {
                let slot = match LspClient::start(&command, &root) {
                    Ok(client) => Slot::Running(client),
                    Err(error) => Slot::Failed(error.to_string()),
                };
                if let Ok(mut slots) = slots.lock() {
                    slots.insert(key, slot);
                }
            })
            .ok()?;
        None
    }

    /// The server once it has started, waiting up to `timeout` for it.
    pub fn client_within(&self, lang: Lang, file: &Path, timeout: Duration) -> Option<Arc<LspClient>> {
        let until = std::time::Instant::now() + timeout;
        loop {
            if let Some(client) = self.client(lang, file) {
                return Some(client);
            }
            let starting = {
                let key = format!("{}\0{}", command_for(lang)?, self.workspace_for(lang, file).display());
                matches!(self.slots.lock().ok()?.get(&key), Some(Slot::Starting))
            };
            if !starting || std::time::Instant::now() >= until {
                return None;
            }
            std::thread::sleep(Duration::from_millis(50));
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
                    Slot::Starting => "starting".to_owned(),
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

    /// Counts what the running servers have published, so a reader can
    /// tell whether anything changed.
    pub fn diagnostics_revision(&self) -> u64 {
        let Ok(slots) = self.slots.lock() else { return 0 };
        slots.values().map(|s| if let Slot::Running(c) = s { c.diagnostics_revision() } else { 0 }).sum()
    }

    /// What every running server has published, by file.
    pub fn all_diagnostics(&self) -> Vec<(PathBuf, Vec<ServerDiagnostic>)> {
        let clients: Vec<Arc<LspClient>> = match self.slots.lock() {
            Ok(slots) => slots.values().filter_map(|s| if let Slot::Running(c) = s { Some(c.clone()) } else { None }).collect(),
            Err(_) => return Vec::new(),
        };
        let mut out: Vec<(PathBuf, Vec<ServerDiagnostic>)> = Vec::new();
        for (path, list) in clients.iter().flat_map(|c| c.all_published()) {
            match out.iter_mut().find(|(p, _)| *p == path) {
                Some((_, existing)) => existing.extend(list),
                None => out.push((path, list)),
            }
        }
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
    fn parses_diagnostics() {
        let list = json!([
            { "range": { "start": { "line": 2, "character": 4 }, "end": { "line": 2, "character": 9 } }, "severity": 2, "message": "Variable 'x' is never used", "source": "kotlin" },
            { "range": { "start": { "line": 5, "character": 0 }, "end": { "line": 5, "character": 3 } }, "message": "Unresolved reference: foo" },
            { "message": "no range" }
        ]);
        assert_eq!(
            parse_diagnostics(&list),
            [
                ServerDiagnostic { start: (2, 4), end: (2, 9), severity: 2, message: "Variable 'x' is never used".into() },
                ServerDiagnostic { start: (5, 0), end: (5, 3), severity: 1, message: "Unresolved reference: foo".into() },
            ]
        );
    }

    #[test]
    fn parses_completions() {
        let list = json!({ "isIncomplete": false, "items": [
            { "label": "greet", "kind": 3, "sortText": "09.0001", "textEdit": { "range": { "start": { "line": 1, "character": 0 }, "end": { "line": 1, "character": 2 } }, "newText": "greet" } },
            { "label": "Path", "kind": 7, "additionalTextEdits": [{ "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } }, "newText": "from pathlib import Path\n" }] },
            { "kind": 3 }
        ] });
        let items = parse_completions(&list);
        assert_eq!(items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), ["greet", "Path"]);
        assert_eq!(items[0].kind, Some(lsp_types::CompletionItemKind::FUNCTION));
        assert_eq!(items[1].additional_text_edits.as_ref().map(Vec::len), Some(1));
        assert_eq!(parse_completions(&json!([{ "label": "x" }])).len(), 1);
        assert!(parse_completions(&Value::Null).is_empty());
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
        let dir = std::env::temp_dir().join(format!("junction-lsp-{}", std::process::id()));
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
