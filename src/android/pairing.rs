//! Wi-Fi pairing's QR code: the device scans `WIFI:T:ADB;S:<service>;P:<password>;;`
//! and advertises a pairing service by that name, which adb then pairs
//! with using the password.

use rand::Rng as _;

/// Android Studio's prefix, so a scanning device is told apart from one
/// waiting for a pairing code.
pub const QR_SERVICE_PREFIX: &str = "studio-";

#[derive(Clone, Debug)]
pub struct QrPairing {
    pub service: String,
    pub password: String,
    /// The QR code's modules, row by row (true = dark).
    pub modules: Vec<Vec<bool>>,
}

fn random_string(len: usize, chars: &[u8]) -> String {
    let mut rng = rand::thread_rng();
    (0..len).map(|_| chars[rng.gen_range(0..chars.len())] as char).collect()
}

pub fn pairing_string(service: &str, password: &str) -> String {
    format!("WIFI:T:ADB;S:{service};P:{password};;")
}

/// A new service name and password, and their QR code.
pub fn new_qr() -> QrPairing {
    let service = format!("{QR_SERVICE_PREFIX}{}", random_string(10, b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-"));
    let password = random_string(12, b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@#$%^&*()-+*/<>{}");
    let code = qrcode::QrCode::new(pairing_string(&service, &password).as_bytes()).expect("a pairing string fits a QR code");
    let width = code.width();
    let colors = code.to_colors();
    let modules = colors.chunks(width).map(|row| row.iter().map(|c| *c == qrcode::Color::Dark).collect()).collect();
    QrPairing { service, password, modules }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_code() {
        let qr = new_qr();
        assert!(qr.service.starts_with("studio-") && qr.service.len() == 17);
        assert_eq!(qr.password.len(), 12);
        assert!(qr.modules.len() >= 21 && qr.modules.iter().all(|r| r.len() == qr.modules.len()));
        assert_eq!(pairing_string("studio-a", "p"), "WIFI:T:ADB;S:studio-a;P:p;;");
    }
}
