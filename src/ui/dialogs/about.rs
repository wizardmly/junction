//! About, Keymap Reference and GPG key setup.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::{
    WindowExt as _,
    checkbox::Checkbox,
    radio::RadioGroup,
    v_flex,
};
use gpui_kit::{
    App, Entity, IntoElement as _, ParentElement as _,
    Styled as _, Window, div, px,
};

use crate::model::RepoModel;
use crate::theme::ActivePalette as _;

use super::footer;

/// Configure GPG Key: sign commits with one of the user's secret keys (repository config).
pub fn configure_gpg(model: Entity<RepoModel>, window: &mut Window, cx: &mut App) {
    let Some(repository) = model.read(cx).repository().cloned() else { return };
    let keys = crate::git::gpg::secret_keys(&repository);
    let (sign, current) = crate::git::gpg::signing_config(&repository);
    let sign = Rc::new(Cell::new(sign));
    let selected = Rc::new(Cell::new(keys.iter().position(|k| current.ends_with(&k.id) || k.id.ends_with(&current)).unwrap_or(0)));
    window.open_dialog(cx, move |dialog, _, cx| {
        let secondary = cx.palette().text_secondary;
        let (sign_set, sign_ok) = (sign.clone(), sign.clone());
        let (key_set, key_ok) = (selected.clone(), selected.clone());
        let keys_ok = keys.clone();
        let repository = repository.clone();
        let model = model.clone();
        let body = v_flex()
            .gap_3()
            .child(Checkbox::new("gpg-sign").label("Sign commits with GPG key").checked(sign.get()).on_change(
                move |value, window, _| {
                    sign_set.set(*value);
                    window.refresh();
                },
            ))
            .child(if keys.is_empty() {
                div()
                    .text_sm()
                    .text_color(secondary)
                    .child("No secret keys found. Install GnuPG and create a key with gpg --full-generate-key.")
                    .into_any_element()
            } else {
                RadioGroup::new("gpg-key")
                    .children(keys.iter().map(|k| format!("{}  {}", k.id, k.user)))
                    .selected_index(Some(selected.get()))
                    .disabled(!sign.get())
                    .on_change(move |ix, window, _| {
                        key_set.set(*ix);
                        window.refresh();
                    })
                    .into_any_element()
            })
            .child(div().text_xs().text_color(secondary).child("Saved to this repository's config (commit.gpgSign, user.signingKey)."));
        dialog
            .title("Configure GPG Key")
            .w(px(520.))
            .child(body)
            .on_ok(move |_, _, cx| {
                let sign = sign_ok.get();
                let key = keys_ok.get(key_ok.get()).map(|k| k.id.clone()).unwrap_or_default();
                let result = crate::git::gpg::set_signing_config(&repository, sign, &key);
                let (title, message, error) = match result {
                    Ok(()) if sign => ("GPG", format!("Commits will be signed with {key}"), false),
                    Ok(()) => ("GPG", "Commit signing turned off".to_owned(), false),
                    Err(error) => ("GPG", error.to_string(), true),
                };
                model.update(cx, |m, cx| m.notify(title, message, error, cx));
                true
            })
            .footer(footer("OK"))
    });
}

/// Help › About.
pub fn about(window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, |dialog, _, cx| {
        let palette = cx.palette().clone();
        dialog.title(format!("About {}", crate::ui::workspace::APP_NAME)).w(px(380.)).child(
            v_flex()
                .gap_1()
                .text_sm()
                .child(div().text_lg().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(crate::ui::workspace::APP_NAME))
                .child(div().text_color(palette.text_secondary).child("Git & Code Navigator"))
                .child(format!("Version {}", env!("CARGO_PKG_VERSION")))
                .child(div().text_color(palette.text_secondary).child("Spell checking uses the SCOWL word list.")),
        )
    });
}

/// IntelliJ's default keymap as this client implements it:
/// (group, action, Windows / Linux, macOS).
pub const KEYMAP: &[(&str, &str, &str, &str)] = &[
    ("Git", "Commit…", "Ctrl+K", "⌘K"),
    ("Git", "Push…", "Ctrl+Shift+K", "⇧⌘K"),
    ("Git", "Update Project…", "Ctrl+T", "⌘T"),
    ("Git", "Branches…", "Ctrl+Shift+`", "⇧⌘`"),
    ("Git", "VCS Operations Popup", "Alt+`", "⌃V"),
    ("Git", "Rollback", "Ctrl+Alt+Z", "⌥⌘Z"),
    ("Git", "Show Diff", "Ctrl+D", "⌘D"),
    ("Git", "Add to VCS", "Ctrl+Alt+A", "⌥⌘A"),
    ("Git", "Move to Another Changelist", "Alt+Shift+M", "⇧⌘M"),
    ("Git", "Commit Message History", "Ctrl+M", "⌃M"),
    ("Git", "Refresh", "Ctrl+Alt+Y", "⌥⌘Y"),
    ("Diff", "Next / Previous Difference", "F7 / Shift+F7", "F7 / ⇧F7"),
    ("Diff", "Compare Next / Previous File", "Alt+Right / Alt+Left", "⌥→ / ⌥←"),
    ("Diff", "Jump to Source", "F4", "⌘↓"),
    ("Editor", "Next / Previous Change", "Ctrl+Alt+Shift+Down / Up", "⌃⌥⇧↓ / ↑"),
    ("Editor", "Go to Declaration", "Ctrl+B", "⌘B"),
    ("Editor", "Find Usages", "Alt+F7", "⌥F7"),
    ("Editor", "Find / Replace", "Ctrl+F / Ctrl+R", "⌘F / ⌘R"),
    ("Editor", "Find Next / Previous", "F3 / Shift+F3", "⌘G / ⇧⌘G"),
    ("Editor", "Show Context Actions (spelling)", "Alt+Enter", "⌥↩"),
    ("Navigate", "Search Everywhere", "Double Shift", "Double ⇧"),
    ("Navigate", "Find Action", "Ctrl+Shift+A", "⇧⌘A"),
    ("Navigate", "Class / File / Symbol", "Ctrl+N / Ctrl+Shift+N / Ctrl+Alt+Shift+N", "⌘O / ⇧⌘O / ⌥⌘O"),
    ("Navigate", "Line/Column", "Ctrl+G", "⌘L"),
    ("Navigate", "Recent Files", "Ctrl+E", "⌘E"),
    ("Navigate", "File Structure", "Ctrl+F12", "⌘F12"),
    ("Navigate", "Back / Forward", "Ctrl+Alt+Left / Right", "⌘[ / ⌘]"),
    ("Navigate", "Select In Project View", "Alt+F1", "⌥F1"),
    ("Navigate", "Find / Replace in Files", "Ctrl+Shift+F / Ctrl+Shift+R", "⇧⌘F / ⇧⌘R"),
    ("Window", "Project / Find / Git / Commit", "Alt+1 / Alt+3 / Alt+9 / Alt+0", "⌘1 / ⌘3 / ⌘9 / ⌘0"),
    ("Window", "Hide Active Tool Window", "Shift+Escape", "⇧⎋"),
    ("Window", "Hide All Tool Windows", "Ctrl+Shift+F12", "⇧⌘F12"),
    ("Window", "Close Tab", "Ctrl+F4", "⌘W"),
    ("Window", "Select Next / Previous Tab", "Alt+Right / Alt+Left", "⇧⌘] / ⇧⌘["),
    ("Window", "Settings", "Ctrl+Alt+S", "⌘,"),
];

/// Help › Keyboard Shortcuts: the table above for this platform.
pub fn keymap_reference(window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, |dialog, _, cx| {
        let palette = cx.palette().clone();
        use gpui_kit::{InteractiveElement as _, StatefulInteractiveElement as _};
        let mut list = v_flex().id("keymap-list").max_h(px(520.)).overflow_y_scroll().gap_px().text_sm();
        let mut group = "";
        for (g, action, pc, mac) in KEYMAP {
            if *g != group {
                group = g;
                list = list.child(div().pt_2().pb_0p5().text_xs().font_weight(gpui_kit::FontWeight::SEMIBOLD).text_color(palette.text_secondary).child(*g));
            }
            let keys = if cfg!(target_os = "macos") { *mac } else { *pc };
            list = list.child(
                gpui_kit::component::h_flex()
                    .gap_4()
                    .child(div().flex_1().child(*action))
                    .child(div().text_color(palette.text_secondary).child(keys)),
            );
        }
        dialog.title("Keyboard Shortcuts").w(px(560.)).child(list)
    });
}
