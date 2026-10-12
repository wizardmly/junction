//! The debuggable processes of a device, as Device Explorer's Processes
//! tab lists them: adb's `track-jdwp` names them, a DDM `HELO` over each
//! one's JDWP connection describes it (name, package, user, VM, ABI,
//! waiting for a debugger), and a JDWP `VirtualMachine.Exit` kills it.
//! All through the adb server's socket, as Android Studio's adblib does.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use super::adb::Adb;

const TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Process {
    pub pid: u32,
    pub name: Option<String>,
    pub package: Option<String>,
    pub user_id: Option<u32>,
    pub vm: Option<String>,
    pub abi: Option<String>,
    pub waiting: bool,
}

impl Process {
    /// Only the PID is known: the process didn't answer.
    pub fn pid_only(&self) -> bool {
        self.name.is_none()
    }

    pub fn display_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| format!("<unknown-{}>", self.pid))
    }

    /// The package `am force-stop` / `pm clear` act on.
    pub fn package_name(&self) -> Option<&str> {
        self.package.as_deref().or(self.name.as_deref()).map(|n| n.split(':').next().unwrap_or(n)).filter(|n| !n.starts_with('<'))
    }
}

fn server_port() -> u16 {
    std::env::var("ANDROID_ADB_SERVER_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(5037)
}

/// Sends one adb server request and reads its `OKAY` / `FAIL`.
fn request(stream: &mut TcpStream, text: &str) -> io::Result<()> {
    stream.write_all(format!("{:04x}{text}", text.len()).as_bytes())?;
    let mut status = [0u8; 4];
    stream.read_exact(&mut status)?;
    match &status {
        b"OKAY" => Ok(()),
        b"FAIL" => {
            let message = read_hex_block(stream).unwrap_or_default();
            Err(io::Error::other(message))
        }
        _ => Err(io::Error::other("unexpected reply from adb")),
    }
}

fn read_hex_block(stream: &mut TcpStream) -> io::Result<String> {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len)?;
    let len = usize::from_str_radix(std::str::from_utf8(&len).unwrap_or("0"), 16).map_err(io::Error::other)?;
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body)?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// A connection to the device's adbd through the adb server.
fn transport(adb: &Adb, serial: &str) -> io::Result<TcpStream> {
    let connect = || -> io::Result<TcpStream> {
        let stream = TcpStream::connect_timeout(&([127, 0, 0, 1], server_port()).into(), TIMEOUT)?;
        stream.set_read_timeout(Some(TIMEOUT))?;
        stream.set_write_timeout(Some(TIMEOUT))?;
        Ok(stream)
    };
    let mut stream = match connect() {
        Ok(stream) => stream,
        Err(_) => {
            // The server isn't running yet.
            let _ = adb.run(&["start-server"], Duration::from_secs(20));
            connect()?
        }
    };
    request(&mut stream, &format!("host:transport:{serial}"))?;
    Ok(stream)
}

/// The PIDs of the device's debuggable processes.
pub fn pids(adb: &Adb, serial: &str) -> io::Result<Vec<u32>> {
    let mut stream = transport(adb, serial)?;
    request(&mut stream, "track-jdwp")?;
    let text = read_hex_block(&mut stream)?;
    Ok(parse_pids(&text))
}

pub fn parse_pids(text: &str) -> Vec<u32> {
    text.lines().filter_map(|l| l.trim().parse().ok()).collect()
}

const HANDSHAKE: &[u8] = b"JDWP-Handshake";
const DDM_COMMAND_SET: u8 = 0xc7;

fn open(adb: &Adb, serial: &str, pid: u32) -> io::Result<TcpStream> {
    let mut stream = transport(adb, serial)?;
    request(&mut stream, &format!("jdwp:{pid}"))?;
    stream.write_all(HANDSHAKE)?;
    let mut reply = [0u8; 14];
    stream.read_exact(&mut reply)?;
    if reply != HANDSHAKE {
        return Err(io::Error::other("JDWP handshake failed"));
    }
    Ok(stream)
}

fn command_packet(id: u32, set: u8, command: u8, data: &[u8]) -> Vec<u8> {
    let mut packet = Vec::with_capacity(11 + data.len());
    packet.extend_from_slice(&(11 + data.len() as u32).to_be_bytes());
    packet.extend_from_slice(&id.to_be_bytes());
    packet.push(0);
    packet.push(set);
    packet.push(command);
    packet.extend_from_slice(data);
    packet
}

/// A JDWP packet: (id, is reply, command set, data).
fn read_packet(stream: &mut TcpStream) -> io::Result<(u32, bool, u8, Vec<u8>)> {
    let mut header = [0u8; 11];
    stream.read_exact(&mut header)?;
    let len = u32::from_be_bytes(header[0..4].try_into().unwrap()) as usize;
    let id = u32::from_be_bytes(header[4..8].try_into().unwrap());
    let reply = header[8] & 0x80 != 0;
    let mut data = vec![0u8; len.saturating_sub(11)];
    stream.read_exact(&mut data)?;
    Ok((id, reply, header[9], data))
}

/// Asks a process to describe itself.
pub fn hello(adb: &Adb, serial: &str, pid: u32) -> io::Result<Process> {
    let mut stream = open(adb, serial, pid)?;
    let mut chunk = b"HELO".to_vec();
    chunk.extend_from_slice(&4u32.to_be_bytes());
    chunk.extend_from_slice(&1u32.to_be_bytes());
    stream.write_all(&command_packet(1, DDM_COMMAND_SET, 1, &chunk))?;
    let mut waiting = false;
    for _ in 0..16 {
        let (id, reply, set, data) = read_packet(&mut stream)?;
        if reply && id == 1 {
            let mut process = parse_helo(&data).ok_or_else(|| io::Error::other("bad HELO reply"))?;
            process.pid = pid;
            process.waiting |= waiting;
            return Ok(process);
        }
        // The VM sends WAIT (and APNM) on its own.
        if !reply && set == DDM_COMMAND_SET && data.starts_with(b"WAIT") {
            waiting = true;
        }
    }
    Err(io::Error::other("no HELO reply"))
}

/// VirtualMachine.Exit(1), as Android Studio's Kill process.
pub fn kill(adb: &Adb, serial: &str, pid: u32) -> io::Result<()> {
    let mut stream = open(adb, serial, pid)?;
    stream.write_all(&command_packet(1, 1, 10, &1i32.to_be_bytes()))?;
    // The reply may or may not arrive before the VM goes away.
    let _ = read_packet(&mut stream);
    Ok(())
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.at)
    }

    fn u32(&mut self) -> Option<u32> {
        let bytes = self.data.get(self.at..self.at + 4)?;
        self.at += 4;
        Some(u32::from_be_bytes(bytes.try_into().ok()?))
    }

    fn u8(&mut self) -> Option<u8> {
        let byte = *self.data.get(self.at)?;
        self.at += 1;
        Some(byte)
    }

    /// `chars` UTF-16BE code units.
    fn utf16(&mut self, chars: usize) -> Option<String> {
        let bytes = self.data.get(self.at..self.at + chars * 2)?;
        self.at += chars * 2;
        let units: Vec<u16> = bytes.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        Some(String::from_utf16_lossy(&units))
    }
}

/// The `HELO` chunk of a reply, as ddmlib's `HandleHello` reads it.
pub fn parse_helo(data: &[u8]) -> Option<Process> {
    if data.get(0..4)? != b"HELO" {
        return None;
    }
    let len = u32::from_be_bytes(data.get(4..8)?.try_into().ok()?) as usize;
    let mut r = Reader { data: data.get(8..8 + len.min(data.len() - 8))?, at: 0 };
    let _version = r.u32()?;
    let pid = r.u32()?;
    let vm_len = r.u32()? as usize;
    let name_len = r.u32()? as usize;
    let vm = r.utf16(vm_len)?;
    let name = r.utf16(name_len)?;
    let mut process = Process { pid, vm: Some(vm).filter(|v| !v.is_empty()), name: Some(name).filter(|n| !n.is_empty()), ..Default::default() };
    if r.remaining() >= 4 {
        process.user_id = r.u32();
    }
    if r.remaining() >= 4 {
        let n = r.u32()? as usize;
        process.abi = r.utf16(n).filter(|a| !a.is_empty());
    }
    if r.remaining() >= 4 {
        let n = r.u32()? as usize;
        let _jvm_flags = r.utf16(n);
    }
    if r.remaining() >= 1 {
        let _native_debuggable = r.u8();
    }
    if r.remaining() >= 4 {
        let n = r.u32()? as usize;
        process.package = r.utf16(n).filter(|p| !p.is_empty());
    }
    Some(process)
}

/// `ps -A -o PID,UID,NAME` (or the old `ps`): name and Android user by PID,
/// for processes that didn't answer `HELO`.
pub fn parse_ps(text: &str) -> HashMap<u32, (String, Option<u32>)> {
    let mut lines = text.lines();
    let Some(header) = lines.next() else { return HashMap::new() };
    let columns: Vec<&str> = header.split_whitespace().collect();
    let pid_ix = columns.iter().position(|c| *c == "PID");
    let uid_ix = columns.iter().position(|c| *c == "UID" || *c == "USER");
    lines
        .filter_map(|line| {
            let words: Vec<&str> = line.split_whitespace().collect();
            let pid = words.get(pid_ix?)?.parse().ok()?;
            let name = words.last()?.to_string();
            // `u0_a123` / `10123`: the Android user is uid / 100000.
            let user = uid_ix.and_then(|i| words.get(i)).and_then(|u| {
                u.parse::<u32>().ok().map(|uid| uid / 100_000).or_else(|| u.strip_prefix('u').and_then(|r| r.split('_').next()).and_then(|n| n.parse().ok()))
            });
            Some((pid, (name, user)))
        })
        .collect()
}

/// The debuggable processes, sorted by name; `known` keeps what earlier
/// calls learned so each process is asked only once.
pub fn list(adb: &Adb, serial: &str, known: &mut HashMap<u32, Process>) -> io::Result<Vec<Process>> {
    let pids = pids(adb, serial)?;
    known.retain(|pid, _| pids.contains(pid));
    let ask: Vec<u32> = pids
        .iter()
        .copied()
        .filter(|pid| known.get(pid).is_none_or(|p| p.pid_only() || p.name.as_deref() == Some("<pre-initialized>")))
        .collect();
    let answers: Vec<(u32, Option<Process>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = ask.iter().map(|&pid| (pid, scope.spawn(move || hello(adb, serial, pid).ok()))).collect();
        handles.into_iter().map(|(pid, h)| (pid, h.join().ok().flatten())).collect()
    });
    let mut missing = Vec::new();
    for (pid, answer) in answers {
        match answer {
            Some(process) => {
                known.insert(pid, process);
            }
            None => missing.push(pid),
        }
    }
    if !missing.is_empty() {
        let ps = adb
            .shell(serial, "ps -A -o PID,UID,NAME 2>/dev/null || ps")
            .map(|text| parse_ps(&text))
            .unwrap_or_default();
        for pid in missing {
            let mut process = Process { pid, ..Default::default() };
            if let Some((name, user)) = ps.get(&pid) {
                process.name = Some(name.clone());
                process.user_id = *user;
            }
            known.insert(pid, process);
        }
    }
    let mut processes: Vec<Process> = pids.iter().filter_map(|pid| known.get(pid).cloned()).collect();
    processes.sort_by(|a, b| a.display_name().cmp(&b.display_name()).then(a.pid.cmp(&b.pid)));
    Ok(processes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(|u| u.to_be_bytes()).collect()
    }

    #[test]
    fn helo() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&1u32.to_be_bytes());
        payload.extend_from_slice(&4321u32.to_be_bytes());
        payload.extend_from_slice(&13u32.to_be_bytes());
        payload.extend_from_slice(&16u32.to_be_bytes());
        payload.extend(utf16("Dalvik v2.1.0"));
        payload.extend(utf16("com.example.demo"));
        payload.extend_from_slice(&0u32.to_be_bytes());
        payload.extend_from_slice(&5u32.to_be_bytes());
        payload.extend(utf16("arm64"));
        payload.extend_from_slice(&0u32.to_be_bytes());
        payload.push(0);
        payload.extend_from_slice(&16u32.to_be_bytes());
        payload.extend(utf16("com.example.demo"));
        let mut chunk = b"HELO".to_vec();
        chunk.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        chunk.extend(payload);
        let p = parse_helo(&chunk).unwrap();
        assert_eq!(p.pid, 4321);
        assert_eq!(p.vm.as_deref(), Some("Dalvik v2.1.0"));
        assert_eq!(p.name.as_deref(), Some("com.example.demo"));
        assert_eq!((p.user_id, p.abi.as_deref(), p.package.as_deref()), (Some(0), Some("arm64"), Some("com.example.demo")));
    }

    #[test]
    fn ps_and_pids() {
        assert_eq!(parse_pids("123\n4567\n"), vec![123, 4567]);
        let ps = parse_ps("  PID   UID NAME\n  812 10123 com.android.systemui\n 900 1010145 com.example:remote\n");
        assert_eq!(ps.get(&812), Some(&("com.android.systemui".to_owned(), Some(0))));
        assert_eq!(ps.get(&900), Some(&("com.example:remote".to_owned(), Some(10))));
        let old = parse_ps("USER     PID   PPID  VSIZE  RSS     WCHAN    PC         NAME\nu10_a12 700 1 0 0 0 0 S com.old\n");
        assert_eq!(old.get(&700), Some(&("com.old".to_owned(), Some(10))));
        let p = Process { pid: 1, name: Some("com.example:remote".into()), ..Default::default() };
        assert_eq!(p.package_name(), Some("com.example"));
    }
}
