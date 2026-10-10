fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // Windows gives the main thread a 1 MB stack, against 8 MB on Linux
        // and macOS; GPUI's layout and paint recurse through the whole element
        // tree on that thread, so give it the same 8 MB everywhere.
        let stack = 8 * 1024 * 1024;
        match std::env::var("CARGO_CFG_TARGET_ENV").as_deref() {
            Ok("msvc") => println!("cargo:rustc-link-arg-bins=/STACK:{stack}"),
            _ => println!("cargo:rustc-link-arg-bins=-Wl,--stack,{stack}"),
        }
        embed_icon();
    }
}

/// Embeds assets/junction.ico as icon resource 1, which Explorer shows for
/// junction.exe and GPUI loads for the window class (taskbar, Alt+Tab).
fn embed_icon() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let icon = std::path::Path::new(&manifest_dir).join("assets").join("junction.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    let rc = std::path::Path::new(&out_dir).join("junction.rc");
    let icon = icon.display().to_string().replace('\\', "\\\\");
    std::fs::write(&rc, format!("1 ICON \"{icon}\"\n")).unwrap();
    embed_resource::compile(&rc, embed_resource::NONE).manifest_optional().unwrap();
}
