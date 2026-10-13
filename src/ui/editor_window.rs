//! Open Tab in New Window (Shift+F4): a file's editor in a window of its
//! own, as IntelliJ's detached editor. Its navigation (Go to Declaration,
//! Show History, Annotate…) goes to the project window it came from.

use gpui_kit::component::TitleBar;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, Subscription,
    WeakEntity, Window, WindowBounds, WindowOptions, div, px, size,
};

use crate::git::Repository;
use crate::index::service::CodeIndex;
use crate::theme::ActivePalette as _;
use crate::ui::file_editor::{FileEditor, FileEditorEvent};
use crate::ui::workspace::{APP_NAME, Workspace};

pub struct EditorWindow {
    editor: Entity<FileEditor>,
    title: String,
    _subscriptions: Vec<Subscription>,
}

/// What the new window opens.
pub struct EditorSource {
    pub repository: Repository,
    pub own: String,
    pub path: String,
    pub revision: Option<String>,
    /// The caret's line to keep (0-based).
    pub line: usize,
}

pub fn open(source: EditorSource, index: Entity<CodeIndex>, workspace: WeakEntity<Workspace>, home: AnyWindowHandle, cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(1000.), px(760.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(400.), px(240.))),
        app_id: Some("junction".into()),
        ..TitleBar::window_options()
    };
    let opened = gpui_kit::open_window(options, cx, move |window, cx| {
        cx.new(|cx| {
            let EditorSource { repository, own, path, revision, line } = source;
            let editor = cx.new(|cx| FileEditor::new(repository, own, path, revision, window, cx));
            editor.update(cx, |editor, cx| {
                editor.attach_index(index, cx);
                editor.go_to(line as u32, 0, window, cx);
            });
            let subscription = cx.subscribe(&editor, move |_, editor, event: &FileEditorEvent, cx| {
                home.update(cx, |_, window, cx| {
                    workspace.update(cx, |ws, cx| ws.on_file_editor_event(&editor, event, window, cx)).ok();
                })
                .ok();
            });
            EditorWindow { editor, title: String::new(), _subscriptions: vec![subscription] }
        })
    });
    if let Err(error) = opened {
        eprintln!("cannot open the editor window: {error}");
    }
}

impl Render for EditorWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let name = {
            let path = self.editor.read(cx).path();
            path.rsplit(['/', '\\']).next().unwrap_or(path).to_owned()
        };
        let title = format!("{name} – {APP_NAME}");
        if self.title != title {
            window.set_window_title(&title);
            self.title = title.clone();
        }
        gpui_kit::component::v_flex()
            .size_full()
            .bg(palette.window)
            .text_color(palette.text)
            .child(TitleBar::new().child(div().text_sm().child(name)))
            .child(div().flex_1().min_h_0().bg(palette.panel).child(self.editor.clone()))
    }
}
