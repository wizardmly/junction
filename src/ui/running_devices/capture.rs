//! Take Screenshot (the preview dialog) and Record Screen.

use super::*;
use std::rc::Rc;

impl MirrorView {
    pub(super) fn take_screenshot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.live().is_none() {
            return;
        }
        let (adb, serial, title) = (self.adb.clone(), self.serial.clone(), self.title.clone());
        let weak = cx.entity().downgrade();
        let window_handle = window.window_handle();
        cx.spawn(async move |_, cx| {
            let result = cx.background_spawn({
                let (adb, serial) = (adb.clone(), serial.clone());
                async move { mirror::screenshot(&adb, &serial) }
            });
            let result = result.await;
            cx.update_window(window_handle, |_, window, cx| match result {
                Ok(png) => {
                    let again = weak.clone();
                    let recapture: Rc<dyn Fn(&mut Window, &mut App)> = Rc::new(move |window, cx| {
                        again.update(cx, |this, cx| this.take_screenshot(window, cx)).ok();
                    });
                    screenshot_dialog(png, recapture, window, cx)
                }
                Err(e) => {
                    weak.update(cx, |_, cx| cx.emit(MirrorEvent::Notify { title: format!("Screenshot of {title} failed"), message: e.to_string(), error: true }))
                        .ok();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn toggle_recording(&mut self, cx: &mut Context<Self>) {
        if let Some(recording) = self.recording.take() {
            let (adb, serial, title) = (self.adb.clone(), self.serial.clone(), self.title.clone());
            let name = format!("Screen_recording_{}.mp4", chrono::Local::now().format("%Y%m%d_%H%M%S"));
            let target = cx.prompt_for_new_path(&default_save_dir(), Some(&name));
            cx.notify();
            cx.spawn(async move |this, cx| {
                let target = chosen_path(target.await, &name);
                let result = cx
                    .background_spawn({
                        let target = target.clone();
                        async move {
                            match target {
                                Some(target) => recording.finish(&adb, &serial, &target).map(|_| Some(target)),
                                None => {
                                    recording.cancel(&adb, &serial);
                                    Ok(None)
                                }
                            }
                        }
                    })
                    .await;
                this.update(cx, |_, cx| match result {
                    Ok(Some(path)) => {
                        cx.emit(MirrorEvent::Notify { title: "Screen recording saved".into(), message: path.display().to_string(), error: false })
                    }
                    Ok(None) => {}
                    Err(e) => cx.emit(MirrorEvent::Notify { title: format!("Screen recording of {title} failed"), message: e.to_string(), error: true }),
                })
                .ok();
            })
            .detach();
            return;
        }
        if self.live().is_none() {
            return;
        }
        match Recording::start(&self.adb, &self.serial) {
            Ok(recording) => {
                self.recording = Some(recording);
                // The elapsed time on the button follows the clock.
                cx.spawn(async move |this, cx| {
                    loop {
                        cx.background_executor().timer(Duration::from_secs(1)).await;
                        let Ok(recording) = this.update(cx, |this, cx| {
                            cx.notify();
                            this.recording.is_some()
                        }) else {
                            break;
                        };
                        if !recording {
                            break;
                        }
                    }
                })
                .detach();
            }
            Err(e) => cx.emit(MirrorEvent::Notify { title: "Cannot record the screen".into(), message: e.to_string(), error: true }),
        }
        cx.notify();
    }
}

/// Take Screenshot: the image, Recapture, Copy to Clipboard, and Save
/// (Running Devices and Logcat).
pub(crate) fn screenshot_dialog(png: Vec<u8>, recapture: Rc<dyn Fn(&mut Window, &mut App)>, window: &mut Window, cx: &mut App) {
    use gpui_kit::component::dialog::DialogFooter;
    let image = Arc::new(Image::from_bytes(ImageFormat::Png, png.clone()));
    let png = Arc::new(png);
    window.open_dialog(cx, move |dialog, _, _| {
        let (copy_image, save_png, recapture) = (image.clone(), png.clone(), recapture.clone());
        dialog
            .title("Screenshot")
            .w(px(560.))
            .child(
                div()
                    .h(px(480.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(img(image.clone()).object_fit(ObjectFit::Contain).max_w_full().max_h_full()),
            )
            .footer(
                DialogFooter::new()
                    .w_full()
                    .gap_2()
                    .child(Button::new("rd-shot-recapture").label("Recapture").outline().on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        recapture(window, cx);
                    }))
                    .child(Button::new("rd-shot-copy").label("Copy to Clipboard").outline().on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_image(&copy_image));
                    }))
                    .child(div().flex_1())
                    .child(Button::new("rd-shot-cancel").label("Cancel").outline().on_click(|_, window, cx| window.close_dialog(cx)))
                    .child(Button::new("rd-shot-save").label("Save").primary().on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        let name = format!("Screenshot_{}.png", chrono::Local::now().format("%Y%m%d_%H%M%S"));
                        let target = cx.prompt_for_new_path(&default_save_dir(), Some(&name));
                        let png = save_png.clone();
                        cx.spawn(async move |_| {
                            if let Some(path) = chosen_path(target.await, &name) {
                                let _ = std::fs::write(path, png.as_slice());
                            }
                        })
                        .detach();
                    })),
            )
    });
}
