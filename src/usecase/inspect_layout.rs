//! Layout Inspector's Refresh: the screen's view tree and its image.

use crate::domain::view_hierarchy::{self, LayoutSource, LayoutWindow};

pub struct Capture {
    pub windows: Vec<LayoutWindow>,
    pub png: Vec<u8>,
    /// The image's size in device pixels.
    pub size: (u32, u32),
}

pub fn capture(source: &dyn LayoutSource, serial: &str, uiautomator: bool) -> Result<Capture, String> {
    let text = source.hierarchy(serial, uiautomator)?;
    let windows = if uiautomator {
        view_hierarchy::parse_uiautomator(&text).into_iter().collect()
    } else {
        view_hierarchy::parse_dumpsys(&text)
    };
    if windows.is_empty() {
        return Err(if uiautomator {
            // Only one client can hold UI Automator (Maestro, Appium, a test run).
            format!("UI Automator gave no layout ({}); another tool may be using UI Automator, so stop it or turn UI Automator off", text.lines().next().unwrap_or_default().trim())
        } else {
            "No activity with a view hierarchy is on the screen".to_owned()
        });
    }
    // A layout without its image is still worth showing.
    let png = source.screenshot(serial).unwrap_or_default();
    let size = view_hierarchy::png_size(&png).unwrap_or_else(|| {
        let root = windows[0].nodes[0].bounds;
        (root.right.max(1) as u32, root.bottom.max(1) as u32)
    });
    Ok(Capture { windows, png, size })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(&'static str);

    impl LayoutSource for Fake {
        fn hierarchy(&self, _: &str, _: bool) -> Result<String, String> {
            Ok(self.0.to_owned())
        }

        fn screenshot(&self, _: &str) -> Result<Vec<u8>, String> {
            Err("no screen".into())
        }
    }

    #[test]
    fn layout_without_image_takes_the_root_size() {
        let text = "  ACTIVITY a/.Main 1 pid=2\n    View Hierarchy:\n      DecorView@1[Main]\n        android.widget.FrameLayout{1 V.E...... ........ 0,0-720,1280}\n";
        let got = capture(&Fake(text), "s", false).unwrap();
        assert_eq!(got.size, (720, 1280));
        assert!(got.png.is_empty());
        assert!(capture(&Fake("nothing"), "s", false).is_err());
    }
}
