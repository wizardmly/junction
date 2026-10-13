//! Show Diff in Separate Window: a diff in a window of its own, as
//! IntelliJ's diff frame. Its navigation (Jump to Source, Select in Git
//! Log, Annotate Revision) goes to the project window it came from.

use gpui_kit::component::TitleBar;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, Subscription,
    WeakEntity, Window, WindowBounds, WindowOptions, div, px, size,
};

use crate::git::Repository;
use crate::theme::ActivePalette as _;
use crate::ui::diff_view::{DiffEvent, DiffSource, DiffView, FilesChanged};
use crate::ui::navigate::OpenTarget;
use crate::ui::workspace::{APP_NAME, Workspace};

pub struct DiffWindow {
    diff: Entity<DiffView>,
    title: String,
    _subscriptions: Vec<Subscription>,
}

/// Opens `source` in a new window; its events reach `workspace` in `home`.
pub fn open(repository: Repository, source: DiffSource, workspace: WeakEntity<Workspace>, home: AnyWindowHandle, cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(1200.), px(800.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(500.), px(300.))),
        app_id: Some("junction".into()),
        ..TitleBar::window_options()
    };
    let opened = gpui_kit::open_window(options, cx, move |_, cx| {
        cx.new(|cx| {
            let diff = cx.new(DiffView::new);
            diff.update(cx, |view, cx| {
                view.set_in_window();
                view.show(repository, source, cx);
            });
            // Runs a workspace method in the project window.
            let forward = move |workspace: WeakEntity<Workspace>, cx: &mut App, f: Box<dyn FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>)>| {
                home.update(cx, |_, window, cx| {
                    workspace.update(cx, |ws, cx| f(ws, window, cx)).ok();
                })
                .ok();
            };
            let (ws1, ws2, ws3) = (workspace.clone(), workspace.clone(), workspace);
            let subscriptions = vec![
                cx.subscribe(&diff, move |_, diff, event: &DiffEvent, cx| {
                    let event = event.clone();
                    forward(ws1.clone(), cx, Box::new(move |ws, window, cx| ws.on_diff_event(&diff, &event, window, cx)));
                }),
                cx.subscribe(&diff, move |_, diff, event: &OpenTarget, cx| {
                    let target = event.0.clone();
                    let root = diff.read(cx).repository().map(|r| r.root().to_path_buf());
                    forward(
                        ws2.clone(),
                        cx,
                        Box::new(move |ws, window, cx| {
                            window.activate_window();
                            ws.go_to_diff_target(root, target, window, cx)
                        }),
                    );
                }),
                cx.subscribe(&diff, move |_, _, _: &FilesChanged, cx| {
                    forward(ws3.clone(), cx, Box::new(|ws, _, cx| ws.reload_model(cx)));
                }),
            ];
            DiffWindow { diff, title: String::new(), _subscriptions: subscriptions }
        })
    });
    if let Err(error) = opened {
        eprintln!("cannot open the diff window: {error}");
    }
}

impl Render for DiffWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let title = self.diff.read(cx).title().map_or_else(|| APP_NAME.to_owned(), |t| format!("{t} – {APP_NAME}"));
        if self.title != title {
            window.set_window_title(&title);
            self.title = title.clone();
        }
        gpui_kit::component::v_flex()
            .size_full()
            .bg(palette.window)
            .text_color(palette.text)
            .child(TitleBar::new().child(div().text_sm().child(title)))
            .child(div().flex_1().min_h_0().bg(palette.panel).child(self.diff.clone()))
    }
}
