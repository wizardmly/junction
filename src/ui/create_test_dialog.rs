//! IntelliJ's Create Test dialog (Generate › Test…): the testing library
//! (a warning when the module's build files don't declare it), the class
//! name, superclass and destination package, setUp / tearDown, and the
//! class's methods to write tests for; OK refuses a name or package that
//! isn't valid, or a class that already exists.

use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::{
    Icon, Sizable as _, WindowExt as _,
    button::Button,
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window, div, prelude::FluentBuilder as _, px,
};

use crate::domain::create_test::{TestLibrary, TestRequest, check_package};
use crate::domain::rename::check_name;
use crate::theme::ActivePalette as _;
use crate::ui::dialogs;

thread_local! {
    /// The library last chosen, as IntelliJ keeps it.
    static LAST_LIBRARY: std::cell::Cell<Option<TestLibrary>> = const { std::cell::Cell::new(None) };
}

pub struct CreateTestView {
    library: TestLibrary,
    /// The libraries the module declares.
    found: Vec<TestLibrary>,
    class_name: Entity<InputState>,
    superclass: Entity<InputState>,
    package: Entity<InputState>,
    set_up: bool,
    tear_down: bool,
    /// The methods (name, as shown) and whether checked.
    methods: Vec<(String, String, bool)>,
    /// The test source root (absolute), and the file extension.
    root: PathBuf,
    extension: &'static str,
    _subscriptions: Vec<Subscription>,
}

impl CreateTestView {
    fn file(&self, cx: &App) -> PathBuf {
        let package = self.package.read(cx).value().trim().to_owned();
        let name = self.class_name.read(cx).value().trim().to_owned();
        let mut file = self.root.clone();
        if !package.is_empty() {
            file.push(package.replace('.', "/"));
        }
        file.join(format!("{name}.{}", self.extension))
    }

    fn request(&self, cx: &App) -> Result<TestRequest, String> {
        let class_name = self.class_name.read(cx).value().trim().to_owned();
        check_name(self.extension, &class_name)?;
        let package = self.package.read(cx).value().trim().to_owned();
        check_package(self.extension, &package)?;
        if self.file(cx).exists() {
            return Err(format!("Class {class_name} already exists"));
        }
        Ok(TestRequest {
            library: self.library,
            class_name,
            superclass: self.superclass.read(cx).value().trim().to_owned(),
            package,
            set_up: self.set_up,
            tear_down: self.tear_down,
            methods: self.methods.iter().filter(|m| m.2).map(|m| m.0.clone()).collect(),
        })
    }
}

impl Render for CreateTestView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let entity = cx.entity();
        let label = |text: &'static str| div().w(px(130.)).text_sm().child(text);
        let library = Button::new("create-test-library")
            .outline()
            .small()
            .label(self.library.label())
            .child(Icon::new(crate::ui::as_icons::CHEVRON_DOWN).xsmall())
            .dropdown_menu(move |mut menu, _, _| {
                for library in TestLibrary::ALL {
                    let entity = entity.clone();
                    menu = menu.item(PopupMenuItem::new(library.label()).on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.library = library;
                            cx.notify();
                        })
                    }));
                }
                menu
            });
        let mut list = v_flex().id("create-test-methods").max_h(px(220.)).min_h(px(60.)).overflow_y_scroll().p_1().border_1().border_color(palette.border).rounded_md();
        for (ix, (_, shown, checked)) in self.methods.iter().enumerate() {
            list = list.child(Checkbox::new(SharedString::from(format!("create-test-method-{ix}"))).label(SharedString::from(shown.clone())).checked(*checked).on_change(cx.listener(
                move |this, v: &bool, _, cx| {
                    this.methods[ix].2 = *v;
                    cx.notify();
                },
            )));
        }
        let error = self.request(cx).err().filter(|_| !self.class_name.read(cx).value().is_empty());
        v_flex()
            .gap_2()
            .child(h_flex().gap_2().items_center().child(label("Testing library:")).child(library))
            .when(!self.found.contains(&self.library), |el| {
                el.child(div().text_xs().text_color(palette.status_conflict).child(format!("{} library not found in the module", self.library.label())))
            })
            .child(h_flex().gap_2().items_center().child(label("Class name:")).child(div().flex_1().child(Input::new(&self.class_name))))
            .child(h_flex().gap_2().items_center().child(label("Superclass:")).child(div().flex_1().child(Input::new(&self.superclass))))
            .child(h_flex().gap_2().items_center().child(label("Destination package:")).child(div().flex_1().child(Input::new(&self.package))))
            .child(
                h_flex()
                    .gap_3()
                    .child(label("Generate:"))
                    .child(Checkbox::new("create-test-setup").label("setUp/@Before").checked(self.set_up).on_change(cx.listener(|this, v: &bool, _, cx| {
                        this.set_up = *v;
                        cx.notify();
                    })))
                    .child(Checkbox::new("create-test-teardown").label("tearDown/@After").checked(self.tear_down).on_change(cx.listener(|this, v: &bool, _, cx| {
                        this.tear_down = *v;
                        cx.notify();
                    }))),
            )
            .child(div().text_sm().child("Generate test methods for:"))
            .child(list)
            .when_some(error, |el, e| el.child(div().text_xs().text_color(palette.status_conflict).child(e)))
    }
}

/// Opens Create Test for `class` in `package`; OK hands the request and
/// the new file (absolute) to `on_ok`.
#[allow(clippy::too_many_arguments)]
pub fn open(
    class: &str,
    package: &str,
    methods: Vec<(String, String)>,
    found: Vec<TestLibrary>,
    root: PathBuf,
    extension: &'static str,
    on_ok: impl Fn(TestRequest, PathBuf, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let library = LAST_LIBRARY.get().or_else(|| found.last().copied()).unwrap_or(TestLibrary::JUnit4);
    let view = cx.new(|cx| {
        let mut input = |value: String, cx: &mut Context<CreateTestView>| cx.new(|cx| InputState::new(window, cx).default_value(value));
        let class_name = input(format!("{class}Test"), cx);
        let superclass = input(String::new(), cx);
        let package = input(package.to_owned(), cx);
        let _subscriptions = [&class_name, &package]
            .into_iter()
            .map(|i| {
                cx.subscribe(i, |_, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                })
            })
            .collect();
        let methods = methods.into_iter().map(|(name, shown)| (name, shown, false)).collect();
        CreateTestView { library, found, class_name, superclass, package, set_up: false, tear_down: false, methods, root, extension, _subscriptions }
    });
    let focus = view.read(cx).class_name.clone();
    let on_ok = Rc::new(on_ok);
    window.open_dialog(cx, move |dialog, _, _| {
        let (view_ok, on_ok) = (view.clone(), on_ok.clone());
        dialog.title("Create Test").w(px(560.)).child(view.clone()).footer(dialogs::footer("OK")).on_ok(move |_, window, cx| {
            let v = view_ok.read(cx);
            let Ok(request) = v.request(cx) else { return false };
            let file = v.file(cx);
            LAST_LIBRARY.set(Some(request.library));
            let on_ok = on_ok.clone();
            window.defer(cx, move |window, cx| on_ok(request, file, window, cx));
            true
        })
    });
    dialogs::focus_input(&focus, window, cx);
}
