//! The Network Inspector: the app's sockets, read from the device and
//! added to the session's connection log.

use crate::domain::background_tasks::parse_uid;
use crate::domain::network_connections::{NetworkSource, Socket, parse_sockets};

/// The app's uid, which owns its sockets.
pub fn uid(source: &dyn NetworkSource, serial: &str, package: &str) -> Result<u32, String> {
    parse_uid(&source.packages(serial, package)?, package).ok_or_else(|| format!("{package} isn't installed"))
}

/// The app's sockets now.
pub fn sockets(source: &dyn NetworkSource, serial: &str, uid: u32) -> Result<Vec<Socket>, String> {
    let text = source.sockets(serial)?;
    if !text.contains("local_address") {
        return Err(format!("The device doesn't show its sockets: {}", text.trim()));
    }
    Ok(parse_sockets(&text, uid))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(&'static str);

    impl NetworkSource for Fake {
        fn sockets(&self, _: &str) -> Result<String, String> {
            Ok(self.0.to_owned())
        }

        fn packages(&self, _: &str, _: &str) -> Result<String, String> {
            Ok("package:com.example uid:10190\n".into())
        }
    }

    #[test]
    fn reads_the_apps_sockets() {
        let fake = Fake("== tcp\n  sl  local_address rem_address st\n   0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000 10190 0 1 1\n");
        assert_eq!(uid(&fake, "s", "com.example"), Ok(10190));
        assert!(uid(&fake, "s", "com.other").is_err());
        assert_eq!(sockets(&fake, "s", 10190).unwrap().len(), 1);
        assert!(sockets(&Fake("cat: /proc/net/tcp: Permission denied"), "s", 10190).is_err());
    }
}
