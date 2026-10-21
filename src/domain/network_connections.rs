//! The Network Inspector's connections: the app's sockets in the kernel's
//! `/proc/net/tcp`, `tcp6`, `udp` and `udp6` tables (each row carries the
//! owning uid), and a log of them across reads, so connections that have
//! closed stay listed with when they were open. Android Studio's inspector
//! sees HTTP requests through an agent in the app; without one, this is
//! what the device shows.

// `core::net`: plain address types, no I/O.
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    Tcp,
    Udp,
}

impl Protocol {
    pub fn label(self) -> &'static str {
        match self {
            Self::Tcp => "TCP",
            Self::Udp => "UDP",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Socket {
    pub protocol: Protocol,
    pub local: (IpAddr, u16),
    pub remote: (IpAddr, u16),
    /// `ESTABLISHED`, `LISTEN`, …
    pub state: &'static str,
    pub uid: u32,
    /// The kernel's socket inode: the same socket across reads.
    pub inode: u64,
}

impl Socket {
    /// A listening or unconnected socket has no remote end.
    pub fn connected(&self) -> bool {
        self.remote.1 != 0
    }
}

/// `tcp_states.h`.
fn tcp_state(code: u8) -> &'static str {
    match code {
        0x01 => "ESTABLISHED",
        0x02 => "SYN_SENT",
        0x03 => "SYN_RECV",
        0x04 => "FIN_WAIT1",
        0x05 => "FIN_WAIT2",
        0x06 => "TIME_WAIT",
        0x07 => "CLOSE",
        0x08 => "CLOSE_WAIT",
        0x09 => "LAST_ACK",
        0x0a => "LISTEN",
        0x0b => "CLOSING",
        _ => "UNKNOWN",
    }
}

/// `0100007F:1F90` (IPv4) or 32 hex digits (IPv6), each 32-bit word in the
/// host's (little-endian) byte order. IPv4-mapped IPv6 addresses are IPv4.
fn address(text: &str) -> Option<(IpAddr, u16)> {
    let (ip, port) = text.split_once(':')?;
    let port = u16::from_str_radix(port, 16).ok()?;
    let words: Vec<u32> = (0..ip.len() / 8).map(|i| u32::from_str_radix(&ip[i * 8..i * 8 + 8], 16).map(u32::swap_bytes)).collect::<Result<_, _>>().ok()?;
    let ip = match words.as_slice() {
        [v4] => IpAddr::V4(Ipv4Addr::from(*v4)),
        [a, b, c, d] => {
            let v6 = Ipv6Addr::from((*a as u128) << 96 | (*b as u128) << 64 | (*c as u128) << 32 | *d as u128);
            v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6))
        }
        _ => return None,
    };
    Some((ip, port))
}

/// `/proc/net/<table>` rows, as printed by
/// `for f in tcp tcp6 udp udp6; do echo "== $f"; cat /proc/net/$f; done`,
/// owned by `uid`.
pub fn parse_sockets(text: &str, uid: u32) -> Vec<Socket> {
    let mut protocol = Protocol::Tcp;
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some(table) = line.strip_prefix("== ") {
            protocol = if table.trim().starts_with("udp") { Protocol::Udp } else { Protocol::Tcp };
            continue;
        }
        // `sl local rem st tx:rx tr:when retrnsmt uid timeout inode …`
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 10 || !f[0].ends_with(':') {
            continue;
        }
        let (Some(local), Some(remote), Ok(st), Ok(owner), Ok(inode)) = (address(f[1]), address(f[2]), u8::from_str_radix(f[3], 16), f[7].parse::<u32>(), f[9].parse::<u64>()) else { continue };
        if owner != uid {
            continue;
        }
        // UDP sockets reuse the codes: 07 is unconnected, 01 connected.
        let state = match (protocol, st) {
            (Protocol::Udp, 0x01) => "CONNECTED",
            (Protocol::Udp, _) => "OPEN",
            (Protocol::Tcp, code) => tcp_state(code),
        };
        out.push(Socket { protocol, local, remote, state, uid: owner, inode });
    }
    out
}

/// A socket the log has seen, with when (milliseconds, the caller's clock).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connection {
    pub socket: Socket,
    pub first_seen: u64,
    pub last_seen: u64,
    /// Not in the latest read.
    pub closed: bool,
}

/// The app's sockets across reads.
#[derive(Clone, Debug, Default)]
pub struct ConnectionLog {
    pub connections: Vec<Connection>,
}

/// The same socket in two reads: by inode, or by its ends once the kernel
/// has let go of it (inode 0, as in TIME_WAIT).
fn same(a: &Socket, b: &Socket) -> bool {
    if a.inode != 0 && b.inode != 0 { a.inode == b.inode } else { a.protocol == b.protocol && a.local == b.local && a.remote == b.remote }
}

/// The longest log kept; the oldest closed connections go first.
const MAX_CONNECTIONS: usize = 2000;

impl ConnectionLog {
    /// Adds a read taken at `now`: new sockets are added, ones still open
    /// updated, and ones gone marked closed.
    pub fn update(&mut self, sockets: Vec<Socket>, now: u64) {
        for c in &mut self.connections {
            if !c.closed && !sockets.iter().any(|s| same(&c.socket, s)) {
                c.closed = true;
            }
        }
        for socket in sockets {
            match self.connections.iter_mut().find(|c| !c.closed && same(&c.socket, &socket)) {
                Some(c) => {
                    c.last_seen = now;
                    c.socket = socket;
                }
                None => self.connections.push(Connection { socket, first_seen: now, last_seen: now, closed: false }),
            }
        }
        if self.connections.len() > MAX_CONNECTIONS {
            let mut excess = self.connections.len() - MAX_CONNECTIONS;
            self.connections.retain(|c| {
                if excess > 0 && c.closed {
                    excess -= 1;
                    false
                } else {
                    true
                }
            });
        }
    }
}

/// Where the Network Inspector reads the device's sockets.
pub trait NetworkSource {
    /// The `/proc/net` tables, in [`parse_sockets`]'s format.
    fn sockets(&self, serial: &str) -> Result<String, String>;
    /// `pm list packages -U <package>`.
    fn packages(&self, serial: &str, package: &str) -> Result<String, String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLES: &str = "\
== tcp
  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000 10190        0 1111 1 0000000000000000 100 0 0 10 0
   1: 0F02000A:A1B2 2E52D98E:01BB 01 00000000:00000000 02:000A7D8C 00000000 10190        0 2222 1 0000000000000000 20 4 30 10 -1
   2: 0F02000A:A1B3 2E52D98E:01BB 01 00000000:00000000 02:000A7D8C 00000000 10191        0 3333 1 0000000000000000 20 4 30 10 -1
== tcp6
  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0000000000000000FFFF00000F02000A:B000 0000000000000000FFFF00001F0D5A8E:01BB 01 00000000:00000000 00:00000000 00000000 10190        0 4444 1 0000000000000000 20 4 0 10 -1
   1: B80D0120000000000000000001000000:B001 B80D0120000000000000000002000000:0050 06 00000000:00000000 03:00001234 00000000 10190        0 0 3 0000000000000000
== udp
  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode ref pointer drops
  10: 00000000:14E9 00000000:0000 07 00000000:00000000 00:00000000 00000000 10190        0 5555 2 0000000000000000 0
== udp6
";

    #[test]
    fn parses_proc_net() {
        let sockets = parse_sockets(TABLES, 10190);
        let shown: Vec<String> = sockets.iter().map(|s| format!("{} {}:{} {}:{} {} {}", s.protocol.label(), s.local.0, s.local.1, s.remote.0, s.remote.1, s.state, s.inode)).collect();
        assert_eq!(
            shown,
            [
                "TCP 127.0.0.1:8080 0.0.0.0:0 LISTEN 1111",
                "TCP 10.0.2.15:41394 142.217.82.46:443 ESTABLISHED 2222",
                "TCP 10.0.2.15:45056 142.90.13.31:443 ESTABLISHED 4444",
                "TCP 2001:db8::1:45057 2001:db8::2:80 TIME_WAIT 0",
                "UDP 0.0.0.0:5353 0.0.0.0:0 OPEN 5555",
            ]
        );
        assert!(!sockets[0].connected() && sockets[1].connected());
    }

    #[test]
    fn logs_connections_across_reads() {
        let all = parse_sockets(TABLES, 10190);
        let mut log = ConnectionLog::default();
        log.update(all.clone(), 1000);
        assert_eq!((log.connections.len(), log.connections.iter().filter(|c| !c.closed).count()), (5, 5));
        // The HTTPS connection closes; the TIME_WAIT one (inode 0) stays.
        let later: Vec<Socket> = all.iter().filter(|s| s.inode != 2222).cloned().collect();
        log.update(later.clone(), 3000);
        assert_eq!((log.connections.len(), log.connections.iter().filter(|c| !c.closed).count()), (5, 4));
        let https = log.connections.iter().find(|c| c.socket.inode == 2222).unwrap();
        assert_eq!((https.closed, https.first_seen, https.last_seen), (true, 1000, 1000));
        let waiting = log.connections.iter().find(|c| c.socket.inode == 0).unwrap();
        assert_eq!((waiting.closed, waiting.last_seen), (false, 3000));
        // A new connection on the same ends is a new entry.
        log.update(all, 5000);
        assert_eq!((log.connections.len(), log.connections.iter().filter(|c| !c.closed).count()), (6, 5));
    }
}
