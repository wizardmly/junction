// Windows gives the main thread a 1 MB stack, against 8 MB on Linux and
// macOS; GPUI's layout and paint recurse through the whole element tree on
// that thread, so give it the same 8 MB everywhere.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let stack = 8 * 1024 * 1024;
        match std::env::var("CARGO_CFG_TARGET_ENV").as_deref() {
            Ok("msvc") => println!("cargo:rustc-link-arg-bins=/STACK:{stack}"),
            _ => println!("cargo:rustc-link-arg-bins=-Wl,--stack,{stack}"),
        }
    }
}
