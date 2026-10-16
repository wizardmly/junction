//! The debugger port over JDWP, as IntelliJ's debugger talks to a VM
//! (through the JDK's JDI): a socket to a VM's JDWP agent (a JVM's
//! `-agentlib:jdwp`, or an Android app's through adb), a reader thread
//! handing each reply to the command waiting for it and queueing the VM's
//! events, and the commands a debug session needs.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use crate::domain::debugger::{
    java_name, line_of, ClassRef, DebugEvent, DebugVm, EventSet, Frame, Location, MethodInfo, StepDepth, Suspend, ThreadInfo, ThreadStatus, Value, Variable,
};

const HANDSHAKE: &[u8] = b"JDWP-Handshake";
/// How long a command waits for its reply.
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

// Command sets and commands (JDWP spec).
const VM: u8 = 1;
const REFERENCE_TYPE: u8 = 2;
const CLASS_TYPE: u8 = 3;
const METHOD: u8 = 6;
const OBJECT_REFERENCE: u8 = 9;
const STRING_REFERENCE: u8 = 10;
const THREAD_REFERENCE: u8 = 11;
const ARRAY_REFERENCE: u8 = 13;
const EVENT_REQUEST: u8 = 15;
const STACK_FRAME: u8 = 16;
const EVENT: u8 = 64;

// Event kinds.
const SINGLE_STEP: u8 = 1;
const BREAKPOINT: u8 = 2;
const THREAD_START: u8 = 6;
const THREAD_DEATH: u8 = 7;
const CLASS_PREPARE: u8 = 8;
const VM_START: u8 = 90;
const VM_DEATH: u8 = 99;

// Suspend policies.
const SUSPEND_ALL: u8 = 2;

/// `ABSENT_INFORMATION`: no source file, line or variable table.
const ABSENT_INFORMATION: u16 = 101;
/// `NATIVE_METHOD`.
const NATIVE_METHOD: u16 = 511;

#[derive(Clone, Copy, Debug)]
struct IdSizes {
    field: usize,
    method: usize,
    object: usize,
    reference_type: usize,
    frame: usize,
}

/// A reply: its error code and data.
type Reply = (u16, Vec<u8>);

#[derive(Default)]
struct ClassCache {
    name: Option<String>,
    source: Option<Option<String>>,
    methods: Option<Vec<MethodInfo>>,
    superclass: Option<u64>,
    /// Instance fields: (id, name).
    fields: Option<Vec<(u64, String)>>,
    /// Every own field: (id, name, static).
    all_fields: Option<Vec<(u64, String, bool)>>,
}

struct Inner {
    writer: Mutex<TcpStream>,
    next_id: AtomicU32,
    pending: Mutex<HashMap<u32, Sender<Reply>>>,
    /// Raw `Event.Composite` packets, parsed once the ID sizes are known.
    events: Mutex<Receiver<Option<Vec<u8>>>>,
    sizes: OnceLock<IdSizes>,
    classes: Mutex<HashMap<u64, ClassCache>>,
}

/// A connection to a VM's JDWP agent.
pub struct JdwpVm {
    inner: Arc<Inner>,
}

impl JdwpVm {
    /// Connects to a JDWP agent listening at `address` (`127.0.0.1:5005`).
    pub fn connect(address: &str) -> io::Result<Self> {
        let stream = TcpStream::connect(address)?;
        Self::over(stream)
    }

    /// Debugs over an open socket: handshakes, then asks the VM its sizes.
    pub fn over(mut stream: TcpStream) -> io::Result<Self> {
        stream.set_nodelay(true).ok();
        stream.set_read_timeout(Some(REPLY_TIMEOUT))?;
        stream.write_all(HANDSHAKE)?;
        let mut reply = [0u8; 14];
        stream.read_exact(&mut reply)?;
        if reply != HANDSHAKE {
            return Err(io::Error::other("JDWP handshake failed"));
        }
        stream.set_read_timeout(None)?;
        let reader = stream.try_clone()?;
        let (event_tx, event_rx) = mpsc::channel();
        let inner = Arc::new(Inner {
            writer: Mutex::new(stream),
            next_id: AtomicU32::new(1),
            pending: Mutex::new(HashMap::new()),
            events: Mutex::new(event_rx),
            sizes: OnceLock::new(),
            classes: Mutex::new(HashMap::new()),
        });
        let weak = Arc::downgrade(&inner);
        std::thread::Builder::new().name("jdwp-reader".into()).spawn(move || read_loop(reader, weak, event_tx))?;
        let vm = Self { inner };
        let data = vm.command(VM, 7, &[]).map_err(io::Error::other)?;
        let mut r = Bytes::new(&data);
        let mut size = || r.int().map(|n| n as usize).ok_or_else(|| io::Error::other("bad IDSizes reply"));
        let sizes = IdSizes { field: size()?, method: size()?, object: size()?, reference_type: size()?, frame: size()? };
        vm.inner.sizes.set(sizes).ok();
        Ok(vm)
    }

    fn sizes(&self) -> IdSizes {
        *self.inner.sizes.get().expect("ID sizes are known once connected")
    }

    /// Sends a command and waits for its reply's data.
    fn command(&self, set: u8, command: u8, data: &[u8]) -> Result<Vec<u8>, String> {
        let (code, data) = self.command_raw(set, command, data)?;
        if code != 0 {
            return Err(error_text(code));
        }
        Ok(data)
    }

    /// A command whose error code the caller looks at.
    fn command_raw(&self, set: u8, command: u8, data: &[u8]) -> Result<Reply, String> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.inner.pending.lock().unwrap().insert(id, tx);
        let mut packet = Vec::with_capacity(11 + data.len());
        packet.extend_from_slice(&(11 + data.len() as u32).to_be_bytes());
        packet.extend_from_slice(&id.to_be_bytes());
        packet.extend_from_slice(&[0, set, command]);
        packet.extend_from_slice(data);
        if let Err(e) = self.inner.writer.lock().unwrap().write_all(&packet) {
            self.inner.pending.lock().unwrap().remove(&id);
            return Err(format!("The VM is gone: {e}"));
        }
        match rx.recv_timeout(REPLY_TIMEOUT) {
            Ok(reply) => Ok(reply),
            Err(RecvTimeoutError::Timeout) => {
                self.inner.pending.lock().unwrap().remove(&id);
                Err("The VM did not reply".into())
            }
            Err(RecvTimeoutError::Disconnected) => Err("The VM is gone".into()),
        }
    }

    fn out(&self) -> Out {
        Out { data: Vec::new(), sizes: self.sizes() }
    }

    fn reader<'a>(&self, data: &'a [u8]) -> In<'a> {
        In { bytes: Bytes::new(data), sizes: self.sizes() }
    }

    // --- Classes ------------------------------------------------------------

    fn name_of_class(&self, class: u64) -> Result<String, String> {
        if let Some(name) = self.inner.classes.lock().unwrap().get(&class).and_then(|c| c.name.clone()) {
            return Ok(name);
        }
        let data = self.command(REFERENCE_TYPE, 1, &self.out().reference_type(class).data)?;
        let name = java_name(&self.reader(&data).string().ok_or("bad Signature reply")?);
        self.inner.classes.lock().unwrap().entry(class).or_default().name = Some(name.clone());
        Ok(name)
    }

    fn superclass_of(&self, class: u64) -> Result<u64, String> {
        if let Some(sup) = self.inner.classes.lock().unwrap().get(&class).and_then(|c| c.superclass) {
            return Ok(sup);
        }
        let sup = match self.command(CLASS_TYPE, 1, &self.out().reference_type(class).data) {
            Ok(data) => self.reader(&data).reference_type().ok_or("bad Superclass reply")?,
            // An interface or array type.
            Err(_) => 0,
        };
        self.inner.classes.lock().unwrap().entry(class).or_default().superclass = Some(sup);
        Ok(sup)
    }

    /// A class's instance fields, then its superclasses', as IntelliJ
    /// lists them (statics hidden).
    fn instance_fields(&self, class: u64) -> Result<Vec<(u64, String)>, String> {
        let mut all = Vec::new();
        let mut at = class;
        let mut seen = 0;
        while at != 0 && seen < 32 {
            seen += 1;
            let cached = self.inner.classes.lock().unwrap().get(&at).and_then(|c| c.fields.clone());
            let own = match cached {
                Some(fields) => fields,
                None => {
                    let data = self.command(REFERENCE_TYPE, 4, &self.out().reference_type(at).data)?;
                    let mut r = self.reader(&data);
                    let count = r.bytes.int().ok_or("bad Fields reply")?;
                    let mut fields = Vec::new();
                    for _ in 0..count {
                        let id = r.field().ok_or("bad Fields reply")?;
                        let name = r.string().ok_or("bad Fields reply")?;
                        let _signature = r.string();
                        let modifiers = r.bytes.int().ok_or("bad Fields reply")?;
                        // Static (0x8) and synthetic (0xf0000000) fields aren't shown.
                        if modifiers & 0x8 == 0 && (modifiers as u32) & 0xf000_0000 == 0 {
                            fields.push((id, name));
                        }
                    }
                    self.inner.classes.lock().unwrap().entry(at).or_default().fields = Some(fields.clone());
                    fields
                }
            };
            // The class's own fields after its superclass's, as declared.
            all.splice(0..0, own);
            at = self.superclass_of(at)?;
        }
        Ok(all)
    }

    /// A class's own fields, statics included.
    fn own_fields(&self, class: u64) -> Result<Vec<(u64, String, bool)>, String> {
        if let Some(fields) = self.inner.classes.lock().unwrap().get(&class).and_then(|c| c.all_fields.clone()) {
            return Ok(fields);
        }
        let data = self.command(REFERENCE_TYPE, 4, &self.out().reference_type(class).data)?;
        let mut r = self.reader(&data);
        let count = r.bytes.int().ok_or("bad Fields reply")?;
        let mut fields = Vec::new();
        for _ in 0..count {
            let id = r.field().ok_or("bad Fields reply")?;
            let name = r.string().ok_or("bad Fields reply")?;
            let _signature = r.string();
            let modifiers = r.bytes.int().ok_or("bad Fields reply")?;
            fields.push((id, name, modifiers & 0x8 != 0));
        }
        self.inner.classes.lock().unwrap().entry(class).or_default().all_fields = Some(fields.clone());
        Ok(fields)
    }

    /// The reply of an invoke: the returned value, or the thrown exception.
    fn invoke_result(&self, data: &[u8]) -> Result<Value, String> {
        let mut r = self.reader(data);
        let tag = r.bytes.u8().ok_or("bad InvokeMethod reply")?;
        let value = self.resolve(tag, &mut r)?;
        let _exception_tag = r.bytes.u8();
        let exception = r.object().unwrap_or(0);
        if exception != 0 {
            let class = self.class_of(exception).map(|c| c.name).unwrap_or_else(|_| "java.lang.Throwable".into());
            return Err(format!("Method threw '{class}' exception."));
        }
        Ok(value)
    }

    fn method_info(&self, class: u64, method: u64) -> Result<Option<MethodInfo>, String> {
        Ok(self.methods(class)?.into_iter().find(|m| m.id == method))
    }

    // --- Values -------------------------------------------------------------

    /// A tagged value read from a reply, resolved: a string's text, an
    /// object's class, an array's length.
    fn resolve(&self, tag: u8, r: &mut In) -> Result<Value, String> {
        let bad = || "bad value".to_string();
        Ok(match tag {
            b'V' => Value::Void,
            b'Z' => Value::Boolean(r.bytes.u8().ok_or_else(bad)? != 0),
            b'B' => Value::Byte(r.bytes.u8().ok_or_else(bad)? as i8),
            b'C' => Value::Char(r.bytes.u16().ok_or_else(bad)?),
            b'S' => Value::Short(r.bytes.u16().ok_or_else(bad)? as i16),
            b'I' => Value::Int(r.bytes.int().ok_or_else(bad)?),
            b'J' => Value::Long(r.bytes.long().ok_or_else(bad)?),
            b'F' => Value::Float(f32::from_bits(r.bytes.int().ok_or_else(bad)? as u32)),
            b'D' => Value::Double(f64::from_bits(r.bytes.long().ok_or_else(bad)? as u64)),
            _ => {
                let id = r.object().ok_or_else(bad)?;
                self.object_value(tag, id)?
            }
        })
    }

    fn object_value(&self, tag: u8, id: u64) -> Result<Value, String> {
        if id == 0 {
            return Ok(Value::Null);
        }
        if tag == b's' {
            let data = self.command(STRING_REFERENCE, 1, &self.out().object(id).data)?;
            return Ok(Value::String { id, text: self.reader(&data).string().unwrap_or_default() });
        }
        let data = self.command(OBJECT_REFERENCE, 1, &self.out().object(id).data)?;
        let mut r = self.reader(&data);
        let _kind = r.bytes.u8();
        let class = self.name_of_class(r.reference_type().ok_or("bad ReferenceType reply")?)?;
        if tag == b'[' {
            let data = self.command(ARRAY_REFERENCE, 1, &self.out().object(id).data)?;
            let length = self.reader(&data).bytes.int().unwrap_or(0).max(0) as usize;
            return Ok(Value::Array { id, class, length });
        }
        Ok(Value::Object { id, class })
    }

    fn array_elements(&self, id: u64, length: usize, limit: usize) -> Result<Vec<Variable>, String> {
        let count = length.min(limit);
        if count == 0 {
            return Ok(Vec::new());
        }
        let mut out = self.out().object(id);
        out.int(0).int(count as i32);
        let data = self.command(ARRAY_REFERENCE, 2, &out.data)?;
        let mut r = self.reader(&data);
        let tag = r.bytes.u8().ok_or("bad GetValues reply")?;
        let n = r.bytes.int().ok_or("bad GetValues reply")?;
        // Primitive elements come untagged, objects tagged.
        let primitive = matches!(tag, b'Z' | b'B' | b'C' | b'S' | b'I' | b'J' | b'F' | b'D');
        let mut vars = Vec::new();
        for i in 0..n {
            let tag = if primitive { tag } else { r.bytes.u8().ok_or("bad GetValues reply")? };
            vars.push(Variable { name: format!("[{i}]"), value: self.resolve(tag, &mut r)? });
        }
        Ok(vars)
    }
}

impl Drop for JdwpVm {
    fn drop(&mut self) {
        let _ = self.inner.writer.lock().unwrap().shutdown(std::net::Shutdown::Both);
    }
}

fn read_loop(mut stream: TcpStream, inner: std::sync::Weak<Inner>, events: Sender<Option<Vec<u8>>>) {
    loop {
        let mut header = [0u8; 11];
        if stream.read_exact(&mut header).is_err() {
            break;
        }
        let len = u32::from_be_bytes(header[0..4].try_into().unwrap()) as usize;
        let id = u32::from_be_bytes(header[4..8].try_into().unwrap());
        let mut data = vec![0u8; len.saturating_sub(11)];
        if stream.read_exact(&mut data).is_err() {
            break;
        }
        if header[8] & 0x80 != 0 {
            let code = u16::from_be_bytes([header[9], header[10]]);
            let Some(inner) = inner.upgrade() else { break };
            if let Some(tx) = inner.pending.lock().unwrap().remove(&id) {
                let _ = tx.send((code, data));
            }
        } else if header[9] == EVENT && header[10] == 100 {
            let _ = events.send(Some(data));
        }
    }
    // The socket closed: the VM died or was disconnected.
    let _ = events.send(None);
    if let Some(inner) = inner.upgrade() {
        inner.pending.lock().unwrap().clear();
    }
}

impl DebugVm for JdwpVm {
    fn all_classes(&self) -> Result<Vec<ClassRef>, String> {
        let data = self.command(VM, 3, &[])?;
        let mut r = self.reader(&data);
        let count = r.bytes.int().ok_or("bad AllClasses reply")?;
        let mut classes = Vec::with_capacity(count.max(0) as usize);
        for _ in 0..count {
            let _kind = r.bytes.u8();
            let id = r.reference_type().ok_or("bad AllClasses reply")?;
            let name = java_name(&r.string().ok_or("bad AllClasses reply")?);
            let _status = r.bytes.int();
            self.inner.classes.lock().unwrap().entry(id).or_default().name.get_or_insert_with(|| name.clone());
            classes.push(ClassRef { id, name });
        }
        Ok(classes)
    }

    fn source_file(&self, class: u64) -> Result<Option<String>, String> {
        if let Some(source) = self.inner.classes.lock().unwrap().get(&class).and_then(|c| c.source.clone()) {
            return Ok(source);
        }
        let (code, data) = self.command_raw(REFERENCE_TYPE, 7, &self.out().reference_type(class).data)?;
        let source = match code {
            0 => self.reader(&data).string(),
            ABSENT_INFORMATION => None,
            code => return Err(error_text(code)),
        };
        self.inner.classes.lock().unwrap().entry(class).or_default().source = Some(source.clone());
        Ok(source)
    }

    fn methods(&self, class: u64) -> Result<Vec<MethodInfo>, String> {
        if let Some(methods) = self.inner.classes.lock().unwrap().get(&class).and_then(|c| c.methods.clone()) {
            return Ok(methods);
        }
        let data = self.command(REFERENCE_TYPE, 5, &self.out().reference_type(class).data)?;
        let mut r = self.reader(&data);
        let count = r.bytes.int().ok_or("bad Methods reply")?;
        let mut methods = Vec::new();
        for _ in 0..count {
            let id = r.method().ok_or("bad Methods reply")?;
            let name = r.string().ok_or("bad Methods reply")?;
            let signature = r.string().ok_or("bad Methods reply")?;
            let modifiers = r.bytes.int().unwrap_or(0);
            methods.push(MethodInfo { id, name, signature, is_static: modifiers & 0x8 != 0, lines: Vec::new() });
        }
        for method in &mut methods {
            let mut out = self.out().reference_type(class);
            out.method(method.id);
            let (code, data) = self.command_raw(METHOD, 1, &out.data)?;
            match code {
                0 => {
                    let mut r = self.reader(&data);
                    let (_start, _end) = (r.bytes.long(), r.bytes.long());
                    let lines = r.bytes.int().unwrap_or(0);
                    for _ in 0..lines {
                        let (Some(index), Some(line)) = (r.bytes.long(), r.bytes.int()) else { break };
                        method.lines.push((index as u64, line as u32));
                    }
                }
                ABSENT_INFORMATION | NATIVE_METHOD => {}
                code => return Err(error_text(code)),
            }
        }
        self.inner.classes.lock().unwrap().entry(class).or_default().methods = Some(methods.clone());
        Ok(methods)
    }

    fn watch_class_prepare(&self, pattern: &str) -> Result<u32, String> {
        let mut out = self.out();
        // Suspend all until the session has set its breakpoints in the class.
        // ClassMatch, which every VM has (SourceNameMatch came with 1.6).
        out.u8(CLASS_PREPARE).u8(SUSPEND_ALL).int(1);
        out.u8(5).string(pattern);
        let data = self.command(EVENT_REQUEST, 1, &out.data)?;
        self.reader(&data).bytes.int().map(|n| n as u32).ok_or_else(|| "bad Set reply".into())
    }

    fn set_breakpoint(&self, location: Location) -> Result<u32, String> {
        let mut out = self.out();
        out.u8(BREAKPOINT).u8(SUSPEND_ALL).int(1);
        out.u8(7).location(location);
        let data = self.command(EVENT_REQUEST, 1, &out.data)?;
        self.reader(&data).bytes.int().map(|n| n as u32).ok_or_else(|| "bad Set reply".into())
    }

    fn clear_breakpoint(&self, request: u32) -> Result<(), String> {
        let mut out = self.out();
        out.u8(BREAKPOINT).int(request as i32);
        self.command(EVENT_REQUEST, 2, &out.data).map(drop)
    }

    fn step(&self, thread: u64, depth: StepDepth) -> Result<u32, String> {
        let mut out = self.out();
        out.u8(SINGLE_STEP).u8(SUSPEND_ALL).int(2);
        let depth = match depth {
            StepDepth::Into => 0,
            StepDepth::Over => 1,
            StepDepth::Out => 2,
        };
        // Step modifier: the thread, by line; then once.
        out.u8(10).object_id(thread).int(1).int(depth);
        out.u8(1).int(1);
        let data = self.command(EVENT_REQUEST, 1, &out.data)?;
        self.reader(&data).bytes.int().map(|n| n as u32).ok_or_else(|| "bad Set reply".into())
    }

    fn clear_step(&self, request: u32) -> Result<(), String> {
        let mut out = self.out();
        out.u8(SINGLE_STEP).int(request as i32);
        self.command(EVENT_REQUEST, 2, &out.data).map(drop)
    }

    fn suspend(&self) -> Result<(), String> {
        self.command(VM, 8, &[]).map(drop)
    }

    fn resume(&self) -> Result<(), String> {
        self.command(VM, 9, &[]).map(drop)
    }

    fn threads(&self) -> Result<Vec<ThreadInfo>, String> {
        let data = self.command(VM, 4, &[])?;
        let mut r = self.reader(&data);
        let count = r.bytes.int().ok_or("bad AllThreads reply")?;
        let ids: Vec<u64> = (0..count).filter_map(|_| r.object()).collect();
        let mut threads = Vec::new();
        for id in ids {
            // A thread may end while it's listed.
            let Ok(name) = self.command(THREAD_REFERENCE, 1, &self.out().object(id).data) else { continue };
            let name = self.reader(&name).string().unwrap_or_default();
            let Ok(status) = self.command(THREAD_REFERENCE, 4, &self.out().object(id).data) else { continue };
            let mut r = self.reader(&status);
            let status = match r.bytes.int().unwrap_or(1) {
                0 => ThreadStatus::Zombie,
                2 => ThreadStatus::Sleeping,
                3 => ThreadStatus::Monitor,
                4 => ThreadStatus::Waiting,
                5 => ThreadStatus::NotStarted,
                _ => ThreadStatus::Running,
            };
            let suspended = r.bytes.int().unwrap_or(0) & 1 != 0;
            threads.push(ThreadInfo { id, name, status, suspended });
        }
        Ok(threads)
    }

    fn frames(&self, thread: u64) -> Result<Vec<Frame>, String> {
        let mut out = self.out().object(thread);
        out.int(0).int(-1);
        let data = self.command(THREAD_REFERENCE, 6, &out.data)?;
        let mut r = self.reader(&data);
        let count = r.bytes.int().ok_or("bad Frames reply")?;
        let mut raw = Vec::new();
        for _ in 0..count {
            let id = r.frame().ok_or("bad Frames reply")?;
            let location = r.location().ok_or("bad Frames reply")?;
            raw.push((id, location));
        }
        raw.into_iter()
            .map(|(id, location)| {
                let class = self.name_of_class(location.class)?;
                let method = self.method_info(location.class, location.method)?;
                let line = method.as_ref().and_then(|m| line_of(m, location.index));
                let source = self.source_file(location.class)?;
                Ok(Frame { id, location, class, method: method.map(|m| m.name).unwrap_or_default(), line, source })
            })
            .collect()
    }

    fn locals(&self, thread: u64, frame: &Frame) -> Result<Vec<Variable>, String> {
        let mut vars = Vec::new();
        let mut out = self.out().object(thread);
        out.frame_id(frame.id);
        if let Ok(data) = self.command(STACK_FRAME, 3, &out.data) {
            let mut r = self.reader(&data);
            let tag = r.bytes.u8().unwrap_or(b'L');
            let id = r.object().unwrap_or(0);
            if id != 0 {
                vars.push(Variable { name: "this".into(), value: self.object_value(tag, id)? });
            }
        }
        // The variables in scope at the frame's index.
        let mut out = self.out().reference_type(frame.location.class);
        out.method(frame.location.method);
        let (code, data) = self.command_raw(METHOD, 2, &out.data)?;
        if code == ABSENT_INFORMATION || code == NATIVE_METHOD {
            return Ok(vars);
        }
        if code != 0 {
            return Err(error_text(code));
        }
        let mut r = self.reader(&data);
        let _args = r.bytes.int();
        let count = r.bytes.int().ok_or("bad VariableTable reply")?;
        let mut visible: Vec<(i32, String, String)> = Vec::new();
        for _ in 0..count {
            let start = r.bytes.long().ok_or("bad VariableTable reply")? as u64;
            let name = r.string().ok_or("bad VariableTable reply")?;
            let signature = r.string().ok_or("bad VariableTable reply")?;
            let length = r.bytes.int().ok_or("bad VariableTable reply")? as u64;
            let slot = r.bytes.int().ok_or("bad VariableTable reply")?;
            if name != "this" && start <= frame.location.index && frame.location.index < start + length {
                visible.push((slot, name, signature));
            }
        }
        visible.sort_by_key(|(slot, _, _)| *slot);
        if visible.is_empty() {
            return Ok(vars);
        }
        let mut out = self.out().object(thread);
        out.frame_id(frame.id).int(visible.len() as i32);
        for (slot, _, signature) in &visible {
            out.int(*slot).u8(signature.as_bytes()[0]);
        }
        let data = self.command(STACK_FRAME, 1, &out.data)?;
        let mut r = self.reader(&data);
        let n = r.bytes.int().ok_or("bad GetValues reply")?;
        for (_, name, _) in visible.into_iter().take(n.max(0) as usize) {
            let tag = r.bytes.u8().ok_or("bad GetValues reply")?;
            vars.push(Variable { name, value: self.resolve(tag, &mut r)? });
        }
        Ok(vars)
    }

    fn children(&self, object: u64, limit: usize) -> Result<Vec<Variable>, String> {
        let data = self.command(OBJECT_REFERENCE, 1, &self.out().object(object).data)?;
        let mut r = self.reader(&data);
        let kind = r.bytes.u8().ok_or("bad ReferenceType reply")?;
        let class = r.reference_type().ok_or("bad ReferenceType reply")?;
        // Kind 3: an array type.
        if kind == 3 {
            let data = self.command(ARRAY_REFERENCE, 1, &self.out().object(object).data)?;
            let length = self.reader(&data).bytes.int().unwrap_or(0).max(0) as usize;
            return self.array_elements(object, length, limit);
        }
        let fields = self.instance_fields(class)?;
        if fields.is_empty() {
            return Ok(Vec::new());
        }
        let mut out = self.out().object(object);
        out.int(fields.len() as i32);
        for (id, _) in &fields {
            out.field_id(*id);
        }
        let data = self.command(OBJECT_REFERENCE, 2, &out.data)?;
        let mut r = self.reader(&data);
        let n = r.bytes.int().ok_or("bad GetValues reply")?;
        let mut vars = Vec::new();
        for (_, name) in fields.into_iter().take(n.max(0) as usize) {
            let tag = r.bytes.u8().ok_or("bad GetValues reply")?;
            vars.push(Variable { name, value: self.resolve(tag, &mut r)? });
        }
        Ok(vars)
    }

    fn find_class(&self, name: &str) -> Result<Option<ClassRef>, String> {
        let mut out = self.out();
        out.string(&crate::domain::debugger::signature_of(name));
        let data = self.command(VM, 2, &out.data)?;
        let mut r = self.reader(&data);
        let count = r.bytes.int().ok_or("bad ClassesBySignature reply")?;
        if count < 1 {
            return Ok(None);
        }
        let _kind = r.bytes.u8();
        let id = r.reference_type().ok_or("bad ClassesBySignature reply")?;
        Ok(Some(ClassRef { id, name: name.to_owned() }))
    }

    fn class_of(&self, object: u64) -> Result<ClassRef, String> {
        let data = self.command(OBJECT_REFERENCE, 1, &self.out().object(object).data)?;
        let mut r = self.reader(&data);
        let _kind = r.bytes.u8();
        let id = r.reference_type().ok_or("bad ReferenceType reply")?;
        Ok(ClassRef { id, name: self.name_of_class(id)? })
    }

    fn superclass(&self, class: u64) -> Result<u64, String> {
        self.superclass_of(class)
    }

    fn class_name(&self, class: u64) -> Result<String, String> {
        self.name_of_class(class)
    }

    fn field(&self, class: u64, object: Option<u64>, name: &str) -> Result<Option<Value>, String> {
        let mut at = class;
        let mut depth = 0;
        while at != 0 && depth < 32 {
            depth += 1;
            if let Some((id, _, is_static)) = self.own_fields(at)?.into_iter().find(|(_, n, _)| n == name) {
                let data = match (is_static, object) {
                    (false, Some(object)) => {
                        let mut out = self.out().object(object);
                        out.int(1).field_id(id);
                        self.command(OBJECT_REFERENCE, 2, &out.data)?
                    }
                    (true, _) => {
                        let mut out = self.out().reference_type(at);
                        out.int(1).field_id(id);
                        self.command(REFERENCE_TYPE, 6, &out.data)?
                    }
                    // An instance field without an instance.
                    (false, None) => return Ok(None),
                };
                let mut r = self.reader(&data);
                let _count = r.bytes.int();
                let tag = r.bytes.u8().ok_or("bad GetValues reply")?;
                return self.resolve(tag, &mut r).map(Some);
            }
            at = self.superclass_of(at)?;
        }
        Ok(None)
    }

    fn invoke(&self, thread: u64, class: u64, object: Option<u64>, method: u64, args: &[Value]) -> Result<Value, String> {
        // INVOKE_SINGLE_THREADED: the other threads stay suspended.
        const SINGLE_THREADED: i32 = 1;
        let data = match object {
            Some(object) => {
                let mut out = self.out().object(object);
                out.object_id(thread);
                let size = out.sizes.reference_type;
                out.id(class, size).method(method).int(args.len() as i32);
                for arg in args {
                    out.value(arg);
                }
                out.int(SINGLE_THREADED);
                self.command(OBJECT_REFERENCE, 6, &out.data)?
            }
            None => {
                let mut out = self.out().reference_type(class);
                out.object_id(thread).method(method).int(args.len() as i32);
                for arg in args {
                    out.value(arg);
                }
                out.int(SINGLE_THREADED);
                self.command(CLASS_TYPE, 3, &out.data)?
            }
        };
        self.invoke_result(&data)
    }

    fn create_string(&self, text: &str) -> Result<Value, String> {
        let mut out = self.out();
        out.string(text);
        let data = self.command(VM, 11, &out.data)?;
        let id = self.reader(&data).object().ok_or("bad CreateString reply")?;
        Ok(Value::String { id, text: text.to_owned() })
    }

    fn array_element(&self, array: u64, index: usize) -> Result<Value, String> {
        let data = self.command(ARRAY_REFERENCE, 1, &self.out().object(array).data)?;
        let length = self.reader(&data).bytes.int().unwrap_or(0).max(0) as usize;
        if index >= length {
            return Err(format!("Array index {index} out of bounds for length {length}"));
        }
        let mut out = self.out().object(array);
        out.int(index as i32).int(1);
        let data = self.command(ARRAY_REFERENCE, 2, &out.data)?;
        let mut r = self.reader(&data);
        let tag = r.bytes.u8().ok_or("bad GetValues reply")?;
        let _n = r.bytes.int();
        let primitive = matches!(tag, b'Z' | b'B' | b'C' | b'S' | b'I' | b'J' | b'F' | b'D');
        let tag = if primitive { tag } else { r.bytes.u8().ok_or("bad GetValues reply")? };
        self.resolve(tag, &mut r)
    }

    fn next_events(&self, timeout: Duration) -> Result<Option<EventSet>, String> {
        let raw = match self.inner.events.lock().unwrap().recv_timeout(timeout) {
            Ok(Some(raw)) => raw,
            Ok(None) | Err(RecvTimeoutError::Disconnected) => return Ok(Some(EventSet { suspended: Suspend::None, events: vec![DebugEvent::VmDeath] })),
            Err(RecvTimeoutError::Timeout) => return Ok(None),
        };
        parse_events(&raw, self.sizes()).map(Some).ok_or_else(|| "bad event packet".into())
    }

    fn exit(&self, code: i32) {
        let _ = self.command(VM, 10, &code.to_be_bytes());
    }

    fn dispose(&self) {
        let _ = self.command(VM, 6, &[]);
        // The VM forgets the debugger; the connection ends with it.
        let _ = self.inner.writer.lock().unwrap().shutdown(std::net::Shutdown::Both);
    }

}

/// An `Event.Composite` packet. Events of kinds never requested end the
/// set (their layout isn't read).
fn parse_events(data: &[u8], sizes: IdSizes) -> Option<EventSet> {
    let mut r = In { bytes: Bytes::new(data), sizes };
    let suspended = match r.bytes.u8()? {
        0 => Suspend::None,
        1 => Suspend::Thread,
        _ => Suspend::All,
    };
    let count = r.bytes.int()?;
    let mut events = Vec::new();
    for _ in 0..count {
        let kind = r.bytes.u8()?;
        let request = r.bytes.int()? as u32;
        let event = match kind {
            VM_START => DebugEvent::VmStart { thread: r.object()? },
            VM_DEATH => DebugEvent::VmDeath,
            THREAD_START => DebugEvent::ThreadStart { thread: r.object()? },
            THREAD_DEATH => DebugEvent::ThreadDeath { thread: r.object()? },
            BREAKPOINT => DebugEvent::Breakpoint { thread: r.object()?, location: r.location()?, request },
            SINGLE_STEP => DebugEvent::Step { thread: r.object()?, location: r.location()?, request },
            CLASS_PREPARE => {
                let thread = r.object()?;
                let _kind = r.bytes.u8()?;
                let id = r.reference_type()?;
                let name = java_name(&r.string()?);
                let _status = r.bytes.int()?;
                DebugEvent::ClassPrepare { thread, class: ClassRef { id, name }, request }
            }
            _ => break,
        };
        events.push(event);
    }
    Some(EventSet { suspended, events })
}

fn error_text(code: u16) -> String {
    let name = match code {
        10 => "INVALID_THREAD",
        13 => "THREAD_NOT_SUSPENDED",
        20 => "INVALID_OBJECT",
        21 => "INVALID_CLASS",
        23 => "INVALID_METHODID",
        30 => "INVALID_FRAMEID",
        35 => "INVALID_SLOT",
        99 => "NOT_IMPLEMENTED",
        101 => "ABSENT_INFORMATION",
        112 => "VM_DEAD",
        _ => "",
    };
    if name.is_empty() { format!("JDWP error {code}") } else { format!("JDWP error {code} ({name})") }
}

// --- Encoding ---------------------------------------------------------------

/// Big-endian reads of a reply.
struct Bytes<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Bytes<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0 }
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let bytes = self.data.get(self.at..self.at + n)?;
        self.at += n;
        Some(bytes)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }

    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_be_bytes([b[0], b[1]]))
    }

    fn int(&mut self) -> Option<i32> {
        self.take(4).map(|b| i32::from_be_bytes(b.try_into().unwrap()))
    }

    fn long(&mut self) -> Option<i64> {
        self.take(8).map(|b| i64::from_be_bytes(b.try_into().unwrap()))
    }

    /// An ID of `size` bytes.
    fn id(&mut self, size: usize) -> Option<u64> {
        self.take(size).map(|b| b.iter().fold(0u64, |acc, x| (acc << 8) | *x as u64))
    }
}

/// Reads with the VM's ID sizes.
struct In<'a> {
    bytes: Bytes<'a>,
    sizes: IdSizes,
}

impl In<'_> {
    fn string(&mut self) -> Option<String> {
        let len = self.bytes.int()?.max(0) as usize;
        self.bytes.take(len).map(|b| String::from_utf8_lossy(b).into_owned())
    }

    fn object(&mut self) -> Option<u64> {
        self.bytes.id(self.sizes.object)
    }

    fn reference_type(&mut self) -> Option<u64> {
        self.bytes.id(self.sizes.reference_type)
    }

    fn method(&mut self) -> Option<u64> {
        self.bytes.id(self.sizes.method)
    }

    fn field(&mut self) -> Option<u64> {
        self.bytes.id(self.sizes.field)
    }

    fn frame(&mut self) -> Option<u64> {
        self.bytes.id(self.sizes.frame)
    }

    fn location(&mut self) -> Option<Location> {
        let _kind = self.bytes.u8()?;
        let class = self.reference_type()?;
        let method = self.method()?;
        let index = self.bytes.long()? as u64;
        Some(Location { class, method, index })
    }
}

/// A command's data.
struct Out {
    data: Vec<u8>,
    sizes: IdSizes,
}

impl Out {
    fn u8(&mut self, b: u8) -> &mut Self {
        self.data.push(b);
        self
    }

    fn int(&mut self, n: i32) -> &mut Self {
        self.data.extend_from_slice(&n.to_be_bytes());
        self
    }

    fn string(&mut self, s: &str) -> &mut Self {
        self.int(s.len() as i32);
        self.data.extend_from_slice(s.as_bytes());
        self
    }

    fn id(&mut self, id: u64, size: usize) -> &mut Self {
        self.data.extend_from_slice(&id.to_be_bytes()[8 - size.min(8)..]);
        self
    }

    fn object_id(&mut self, id: u64) -> &mut Self {
        let size = self.sizes.object;
        self.id(id, size)
    }

    fn field_id(&mut self, id: u64) -> &mut Self {
        let size = self.sizes.field;
        self.id(id, size)
    }

    fn frame_id(&mut self, id: u64) -> &mut Self {
        let size = self.sizes.frame;
        self.id(id, size)
    }

    fn method(&mut self, id: u64) -> &mut Self {
        let size = self.sizes.method;
        self.id(id, size)
    }

    /// A command starting with an object ID.
    fn object(mut self, id: u64) -> Self {
        self.object_id(id);
        self
    }

    /// A command starting with a reference type ID.
    fn reference_type(mut self, id: u64) -> Self {
        let size = self.sizes.reference_type;
        self.id(id, size);
        self
    }

    /// A tagged value, as a method's argument.
    fn value(&mut self, value: &Value) -> &mut Self {
        match value {
            Value::Void | Value::Null => self.u8(b'L').object_id(0),
            Value::Boolean(b) => self.u8(b'Z').u8(*b as u8),
            Value::Byte(n) => self.u8(b'B').u8(*n as u8),
            Value::Char(c) => {
                self.u8(b'C');
                self.data.extend_from_slice(&c.to_be_bytes());
                self
            }
            Value::Short(n) => {
                self.u8(b'S');
                self.data.extend_from_slice(&n.to_be_bytes());
                self
            }
            Value::Int(n) => self.u8(b'I').int(*n),
            Value::Long(n) => {
                self.u8(b'J');
                self.data.extend_from_slice(&n.to_be_bytes());
                self
            }
            Value::Float(f) => self.u8(b'F').int(f.to_bits() as i32),
            Value::Double(f) => {
                self.u8(b'D');
                self.data.extend_from_slice(&f.to_bits().to_be_bytes());
                self
            }
            Value::String { id, .. } => self.u8(b's').object_id(*id),
            Value::Array { id, .. } => self.u8(b'[').object_id(*id),
            Value::Object { id, .. } => self.u8(b'L').object_id(*id),
        }
    }

    fn location(&mut self, location: Location) -> &mut Self {
        // Type tag 1: a class.
        self.u8(1);
        let size = self.sizes.reference_type;
        self.id(location.class, size).method(location.method);
        self.data.extend_from_slice(&location.index.to_be_bytes());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::debugger::{line_locations, value_text};
    use std::io::BufRead as _;
    use std::process::{Child, Command, Stdio};

    const SIZES: IdSizes = IdSizes { field: 8, method: 8, object: 8, reference_type: 8, frame: 8 };

    #[test]
    fn parses_event_packets() {
        let mut out = Out { data: Vec::new(), sizes: SIZES };
        out.u8(2).int(2);
        out.u8(VM_START).int(0).object_id(7);
        out.u8(BREAKPOINT).int(3).object_id(7).location(Location { class: 11, method: 12, index: 4 });
        let set = parse_events(&out.data, SIZES).unwrap();
        assert_eq!(set.suspended, Suspend::All);
        assert_eq!(set.events, vec![DebugEvent::VmStart { thread: 7 }, DebugEvent::Breakpoint { thread: 7, location: Location { class: 11, method: 12, index: 4 }, request: 3 }]);
        // Narrow IDs (Android's 4-byte ones) round-trip.
        let narrow = IdSizes { object: 4, reference_type: 4, method: 4, ..SIZES };
        let mut out = Out { data: Vec::new(), sizes: narrow };
        out.u8(0).int(1).u8(CLASS_PREPARE).int(9).object_id(1).u8(1).id(0x22, 4).string("Lcom/example/Calc;").int(7);
        let set = parse_events(&out.data, narrow).unwrap();
        assert_eq!(set.events, vec![DebugEvent::ClassPrepare { thread: 1, class: ClassRef { id: 0x22, name: "com.example.Calc".into() }, request: 9 }]);
    }

    /// A JVM waiting for a debugger, running `Main` compiled from `source`.
    struct Debuggee {
        child: Child,
        port: u16,
        _dir: TempDir,
    }

    impl Drop for Debuggee {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    struct TempDir(std::path::PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// `None` without a JDK on the PATH.
    fn debuggee(name: &str, source: &str) -> Option<Debuggee> {
        let dir = std::env::temp_dir().join(format!("junction-jdwp-{name}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("demo")).ok()?;
        std::fs::write(dir.join("demo/Main.java"), source).ok()?;
        let compiled = Command::new("javac").arg("-g").arg("demo/Main.java").current_dir(&dir).stderr(Stdio::null()).status().ok()?;
        if !compiled.success() {
            return None;
        }
        let mut child = Command::new("java")
            .arg("-agentlib:jdwp=transport=dt_socket,server=y,suspend=y,address=127.0.0.1:0")
            .arg("-cp")
            .arg(&dir)
            .arg("demo.Main")
            .env_remove("JAVA_TOOL_OPTIONS")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        // "Listening for transport dt_socket at address: 40123"
        let mut stdout = std::io::BufReader::new(child.stdout.take()?);
        let mut line = String::new();
        stdout.read_line(&mut line).ok()?;
        let port = line.trim().rsplit(':').next()?.trim().parse().ok()?;
        std::thread::spawn(move || std::io::copy(&mut stdout, &mut std::io::sink()));
        Some(Debuggee { child, port, _dir: TempDir(dir) })
    }

    const MAIN: &str = r#"package demo;

public class Main {
    static class Point {
        int x = 3;
        String label = "p";
    }

    static int twice(int n) {
        int result = n * 2;
        return result;
    }

    public static void main(String[] args) {
        int a = 21;
        String s = "hello";
        int[] numbers = {1, 2, 3};
        Point p = new Point();
        int b = twice(a);
        System.out.println(s + b + numbers.length + p.x);
    }
}
"#;

    fn wait_for(vm: &JdwpVm, mut want: impl FnMut(&DebugEvent) -> bool) -> DebugEvent {
        for _ in 0..50 {
            if let Some(set) = vm.next_events(Duration::from_millis(200)).unwrap() {
                if let Some(e) = set.events.iter().find(|e| want(e)) {
                    return e.clone();
                }
                if set.suspended != Suspend::None {
                    vm.resume().unwrap();
                }
            }
        }
        panic!("no such event");
    }

    /// An adb server for one debugger: `host:transport:<serial>`, then
    /// `jdwp:<pid>`, relayed to a JVM's JDWP port.
    fn fake_adb(jvm_port: u16) -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut client, _) = listener.accept().unwrap();
            let read_request = |client: &mut TcpStream| {
                let mut len = [0u8; 4];
                client.read_exact(&mut len).unwrap();
                let mut body = vec![0u8; usize::from_str_radix(std::str::from_utf8(&len).unwrap(), 16).unwrap()];
                client.read_exact(&mut body).unwrap();
                String::from_utf8(body).unwrap()
            };
            assert_eq!(read_request(&mut client), "host:transport:emulator-5554");
            client.write_all(b"OKAY").unwrap();
            assert_eq!(read_request(&mut client), "jdwp:4242");
            client.write_all(b"OKAY").unwrap();
            let jvm = TcpStream::connect(("127.0.0.1", jvm_port)).unwrap();
            let (mut a, mut b) = (client.try_clone().unwrap(), jvm.try_clone().unwrap());
            std::thread::spawn(move || std::io::copy(&mut a, &mut b));
            let (mut jvm, mut client) = (jvm, client);
            let _ = std::io::copy(&mut jvm, &mut client);
        });
        port
    }

    #[test]
    fn debugs_through_adb() {
        let Some(java) = debuggee("adb", MAIN) else {
            eprintln!("no JDK: skipped");
            return;
        };
        let adb = fake_adb(java.port);
        let stream = crate::android::jdwp::connect_via(adb, "emulator-5554", 4242).unwrap();
        let vm = JdwpVm::over(stream).unwrap();
        wait_for(&vm, |e| matches!(e, DebugEvent::VmStart { .. }));
        assert!(vm.all_classes().unwrap().iter().any(|c| c.name == "java.lang.String"));
        vm.resume().unwrap();
        wait_for(&vm, |e| matches!(e, DebugEvent::VmDeath));
    }

    const EVAL: &str = r#"package demo;

import java.util.*;

public class Main {
    static int COUNT = 7;

    static class Point {
        int x = 3;
        private String label = "p";
        public String getLabel() { return label; }
        @Override public String toString() { return "Point(" + x + ")"; }
    }

    static class Plain {
        int y = 1;
    }

    static int twice(int n) { return n * 2; }

    public static void main(String[] args) {
        int a = 21;
        long big = 3000000000L;
        String s = "hello";
        int[] numbers = {1, 2, 3};
        Point p = new Point();
        Plain plain = new Plain();
        List<String> list = new ArrayList<>(List.of("x", "y"));
        Map<String, Integer> map = new HashMap<>(Map.of("k", 5));
        Integer boxed = 42;
        Object nothing = null;
        System.out.println(a + s + numbers.length + p + plain + list + map + boxed + nothing + big);
    }
}
"#;

    #[test]
    fn evaluates_expressions() {
        use crate::domain::debugger::value_text;
        use crate::usecase::evaluate::{children, condition_holds, describe, evaluate, Scope};
        let Some(java) = debuggee("eval", EVAL) else {
            eprintln!("no JDK: skipped");
            return;
        };
        let vm = JdwpVm::connect(&format!("127.0.0.1:{}", java.port)).unwrap();
        wait_for(&vm, |e| matches!(e, DebugEvent::VmStart { .. }));
        vm.watch_class_prepare("demo.*").unwrap();
        vm.resume().unwrap();
        let DebugEvent::ClassPrepare { class, .. } = wait_for(&vm, |e| matches!(e, DebugEvent::ClassPrepare { class, .. } if class.name == "demo.Main")) else { unreachable!() };
        // The println line.
        let location = line_locations(class.id, &vm.methods(class.id).unwrap(), 32)[0];
        vm.set_breakpoint(location).unwrap();
        vm.resume().unwrap();
        let DebugEvent::Breakpoint { thread, .. } = wait_for(&vm, |e| matches!(e, DebugEvent::Breakpoint { .. })) else { unreachable!() };
        let frames = vm.frames(thread).unwrap();
        let locals = vm.locals(thread, &frames[0]).unwrap();
        let scope = Scope { vm: &vm, thread, frame: &frames[0], locals: &locals };
        let eval = |text: &str| evaluate(&scope, text).map(|v| value_text(&v));
        assert_eq!(eval("a * 2 + 1"), Ok("43".into()));
        assert_eq!(eval("a / 2.0"), Ok("10.5".into()));
        assert_eq!(eval("big + 1"), Ok("3000000001".into()));
        assert_eq!(eval("s + \" \" + a"), Ok("\"hello 21\"".into()));
        assert_eq!(eval("s.length() + numbers.length"), Ok("8".into()));
        assert_eq!(eval("numbers[1] * 10"), Ok("20".into()));
        assert_eq!(eval("p.x"), Ok("3".into()));
        // Kotlin-style property: the getter.
        assert_eq!(eval("p.label"), Ok("\"p\"".into()));
        assert_eq!(eval("\"at \" + p"), Ok("\"at Point(3)\"".into()));
        assert_eq!(eval("list.size()"), Ok("2".into()));
        assert_eq!(eval("list.get(1)"), Ok("\"y\"".into()));
        assert_eq!(eval("list.contains(\"x\")"), Ok("true".into()));
        assert_eq!(eval("map.get(\"k\") + 1"), Ok("6".into()));
        assert_eq!(eval("boxed == 42"), Ok("true".into()));
        assert_eq!(eval("twice(a)"), Ok("42".into()));
        assert_eq!(eval("Main.twice(2)"), Ok("4".into()));
        assert_eq!(eval("COUNT"), Ok("7".into()));
        assert_eq!(eval("Math.max(a, 100)"), Ok("100".into()));
        assert_eq!(eval("Integer.parseInt(\"12\") + 1"), Ok("13".into()));
        assert_eq!(eval("nothing == null && a > 20"), Ok("true".into()));
        assert_eq!(eval("y"), Err("Cannot find local variable 'y'".into()));
        assert_eq!(eval("a / 0"), Err("java.lang.ArithmeticException: / by zero".into()));
        assert_eq!(eval("Integer.parseInt(\"z\")"), Err("Method threw 'java.lang.NumberFormatException' exception.".into()));
        assert_eq!(eval("nothing.toString()"), Err("java.lang.NullPointerException".into()));
        assert_eq!(condition_holds(&scope, "a == 21"), Ok(true));
        assert_eq!(condition_holds(&scope, "s.isEmpty()"), Ok(false));
        assert!(condition_holds(&scope, "a + 1").is_err());

        // The Variables view's descriptions and collection children.
        let value = |name: &str| locals.iter().find(|v| v.name == name).unwrap().value.clone();
        assert_eq!(describe(&vm, thread, &value("p")).as_deref(), Some("\"Point(3)\""));
        assert_eq!(describe(&vm, thread, &value("plain")), None);
        assert_eq!(describe(&vm, thread, &value("list")).as_deref(), Some("size = 2"));
        assert_eq!(describe(&vm, thread, &value("map")).as_deref(), Some("size = 1"));
        assert_eq!(describe(&vm, thread, &value("boxed")).as_deref(), Some("42"));
        let items: Vec<String> = children(&vm, thread, &value("list"), 100).unwrap().iter().map(|v| format!("{} = {}", v.name, value_text(&v.value))).collect();
        assert_eq!(items, ["[0] = \"x\"", "[1] = \"y\""]);
        let entries = children(&vm, thread, &value("map"), 100).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(describe(&vm, thread, &entries[0].value).as_deref(), Some("\"k=5\""));
        vm.resume().unwrap();
        wait_for(&vm, |e| matches!(e, DebugEvent::VmDeath));
    }

    #[test]
    fn debugs_a_jvm() {
        let Some(java) = debuggee("main", MAIN) else {
            eprintln!("no JDK: skipped");
            return;
        };
        let vm = JdwpVm::connect(&format!("127.0.0.1:{}", java.port)).unwrap();
        let DebugEvent::VmStart { .. } = wait_for(&vm, |e| matches!(e, DebugEvent::VmStart { .. })) else { unreachable!() };
        // Main isn't loaded yet: a breakpoint waits for it.
        let request = vm.watch_class_prepare("demo.*").unwrap();
        vm.resume().unwrap();
        let DebugEvent::ClassPrepare { class, thread, .. } = wait_for(&vm, |e| matches!(e, DebugEvent::ClassPrepare { request: r, .. } if *r == request)) else { unreachable!() };
        assert_eq!(class.name, "demo.Main");
        assert_eq!(vm.source_file(class.id).unwrap().as_deref(), Some("Main.java"));
        let methods = vm.methods(class.id).unwrap();
        // Line 19: `int b = twice(a);`.
        let locations = line_locations(class.id, &methods, 19);
        assert_eq!(locations.len(), 1);
        let breakpoint = vm.set_breakpoint(locations[0]).unwrap();
        let _ = thread;
        vm.resume().unwrap();
        let DebugEvent::Breakpoint { thread, request, .. } = wait_for(&vm, |e| matches!(e, DebugEvent::Breakpoint { .. })) else { unreachable!() };
        assert_eq!(request, breakpoint);

        let frames = vm.frames(thread).unwrap();
        assert_eq!((frames[0].method.as_str(), frames[0].line, frames[0].class.as_str()), ("main", Some(19), "demo.Main"));
        let locals = vm.locals(thread, &frames[0]).unwrap();
        let shown: Vec<(String, String)> = locals.iter().map(|v| (v.name.clone(), value_text(&v.value))).collect();
        assert_eq!(shown[0], ("args".into(), shown[0].1.clone()));
        assert!(shown[0].1.starts_with("{String[0]@"), "{shown:?}");
        assert_eq!(shown[1], ("a".into(), "21".into()));
        assert_eq!(shown[2], ("s".into(), "\"hello\"".into()));
        assert!(shown[3].1.starts_with("{int[3]@"), "{shown:?}");
        assert!(shown[4].1.starts_with("{Main$Point@"), "{shown:?}");
        // Expanding the array and the object.
        let numbers = vm.children(locals[3].value.object_id().unwrap(), 100).unwrap();
        assert_eq!(numbers.iter().map(|v| format!("{} = {}", v.name, value_text(&v.value))).collect::<Vec<_>>(), ["[0] = 1", "[1] = 2", "[2] = 3"]);
        let point = vm.children(locals[4].value.object_id().unwrap(), 100).unwrap();
        assert_eq!(point.iter().map(|v| format!("{} = {}", v.name, value_text(&v.value))).collect::<Vec<_>>(), ["x = 3", "label = \"p\""]);
        let threads = vm.threads().unwrap();
        assert!(threads.iter().any(|t| t.id == thread && t.name == "main" && t.suspended));

        // Step Into `twice`, Step Over its first line, Step Out.
        vm.clear_breakpoint(breakpoint).unwrap();
        let step = |depth| {
            let request = vm.step(thread, depth).unwrap();
            vm.resume().unwrap();
            let DebugEvent::Step { thread: t, .. } = wait_for(&vm, |e| matches!(e, DebugEvent::Step { .. })) else { unreachable!() };
            vm.clear_step(request).unwrap();
            let top = &vm.frames(t).unwrap()[0];
            (top.method.clone(), top.line)
        };
        assert_eq!(step(StepDepth::Into), ("twice".into(), Some(10)));
        assert_eq!(step(StepDepth::Over), ("twice".into(), Some(11)));
        assert_eq!(step(StepDepth::Out).0, "main");
        vm.resume().unwrap();
        wait_for(&vm, |e| matches!(e, DebugEvent::VmDeath));
    }
}
