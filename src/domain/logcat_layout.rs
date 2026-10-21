//! Logcat's tabs and panels kept across restarts, as Android Studio keeps
//! them in the workspace (`LogcatPanelConfig` per panel, the splitters'
//! orientation per tab).

use serde::{Deserialize, Serialize};

/// One panel: what it followed and how it showed it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelState {
    #[serde(default)]
    pub filter: String,
    /// The device picked in the combo; else it follows the run target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub compact: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub wrap: bool,
    /// Load Proguard Mappings…' file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mapping: Option<String>,
    /// An imported log file the panel shows instead of a device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaneLayout {
    Log(PanelState),
    /// Two panels side by side (`down`: one above the other).
    Split { down: bool, first: Box<PaneLayout>, second: Box<PaneLayout> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabLayout {
    pub name: String,
    pub pane: PaneLayout,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogcatLayout {
    #[serde(default)]
    pub tabs: Vec<TabLayout>,
    #[serde(default)]
    pub active: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let layout = LogcatLayout {
            tabs: vec![
                TabLayout { name: "Logcat".into(), pane: PaneLayout::Log(PanelState { filter: "package:mine".into(), ..Default::default() }) },
                TabLayout {
                    name: "Errors".into(),
                    pane: PaneLayout::Split {
                        down: true,
                        first: Box::new(PaneLayout::Log(PanelState { filter: "level:error".into(), device: Some("emulator-5554".into()), compact: true, ..Default::default() })),
                        second: Box::new(PaneLayout::Log(PanelState { file: Some("/tmp/crash.logcat".into()), mapping: Some("/p/mapping.txt".into()), wrap: true, ..Default::default() })),
                    },
                },
            ],
            active: 1,
        };
        let json = serde_json::to_string(&layout).unwrap();
        assert_eq!(serde_json::from_str::<LogcatLayout>(&json).unwrap(), layout);
        assert!(json.contains(r#"{"log":{"filter":"package:mine"}}"#), "{json}");
    }
}
