//! Build › Generate Signed Bundle / APK…, Android Studio's wizard in one
//! dialog: the package (App Bundle or APK), the application module and its
//! variant, the keystore and the key. The keystore path and the key alias
//! come back next time for the module; the passwords don't.

use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::{
    Sizable as _, WindowExt as _, h_flex,
    button::Button,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    radio::RadioGroup,
    v_flex,
};
use gpui_kit::{App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, PathPromptOptions, Render, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px};

use crate::domain::run::{ModuleInfo, ModuleType};
use crate::domain::signing::{self, PackageFormat, RememberedKey, SigningRequest, SigningStore};
use crate::theme::ActivePalette as _;
use crate::ui::dialogs::{focus_input, footer_enabled};
use crate::ui::run_service::RunService;

pub struct SignedApkDialog {
    /// The application modules, with their build's root.
    modules: Vec<(PathBuf, ModuleInfo)>,
    module: usize,
    variant: String,
    format: PackageFormat,
    store_file: Entity<InputState>,
    store_password: Entity<InputState>,
    key_alias: Entity<InputState>,
    key_password: Entity<InputState>,
    error: Option<&'static str>,
    store: Rc<dyn SigningStore>,
    run: Entity<RunService>,
    _subscriptions: Vec<Subscription>,
}

impl SignedApkDialog {
    fn new(modules: Vec<(PathBuf, ModuleInfo)>, store: Rc<dyn SigningStore>, run: Entity<RunService>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store_file = cx.new(|cx| InputState::new(window, cx).placeholder("Path to the .jks or .keystore file"));
        let store_password = cx.new(|cx| InputState::new(window, cx).masked(true));
        let key_alias = cx.new(|cx| InputState::new(window, cx));
        let key_password = cx.new(|cx| InputState::new(window, cx).masked(true));
        // An edit clears the last check's message.
        let subscriptions = [&store_file, &store_password, &key_alias, &key_password]
            .into_iter()
            .map(|input| {
                cx.subscribe(input, |this: &mut Self, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) && this.error.take().is_some() {
                        cx.notify();
                    }
                })
            })
            .collect();
        let mut this = Self {
            modules,
            module: 0,
            variant: String::new(),
            format: PackageFormat::default(),
            store_file,
            store_password,
            key_alias,
            key_password,
            error: None,
            store,
            run,
            _subscriptions: subscriptions,
        };
        this.select_module(0, window, cx);
        this
    }

    /// A module: its release variant and the key it was signed with last.
    fn select_module(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, module)) = self.modules.get(ix) else { return };
        self.module = ix;
        self.variant = signing::default_variant(module);
        if let Some(key) = self.store.load(&module.dir) {
            self.store_file.update(cx, |s, cx| s.set_value(key.store_file.display().to_string(), window, cx));
            self.key_alias.update(cx, |s, cx| s.set_value(key.key_alias, window, cx));
        }
        self.error = None;
        cx.notify();
    }

    fn request(&self, cx: &App) -> SigningRequest {
        SigningRequest {
            format: self.format,
            store_file: PathBuf::from(self.store_file.read(cx).value().trim()),
            store_password: self.store_password.read(cx).value().to_string(),
            key_alias: self.key_alias.read(cx).value().trim().to_owned(),
            key_password: self.key_password.read(cx).value().to_string(),
        }
    }

    fn browse(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Choose Key Store File".into()) });
        let input = self.store_file.clone();
        cx.spawn(async move |_, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            cx.update(|cx| {
                if let Some(window) = cx.active_window() {
                    window.update(cx, |_, window, cx| input.update(cx, |s, cx| s.set_value(path.display().to_string(), window, cx))).ok();
                }
            });
        })
        .detach();
    }

    /// Create: checks the fields, remembers the keystore and alias, and
    /// starts the build; false keeps the dialog open on an error.
    fn create(&mut self, cx: &mut Context<Self>) -> bool {
        let request = self.request(cx);
        if let Err(error) = signing::check(&request, request.store_file.is_file()) {
            self.error = Some(error);
            cx.notify();
            return false;
        }
        let Some((root, module)) = self.modules.get(self.module).cloned() else { return false };
        self.store.save(&module.dir, &RememberedKey { store_file: request.store_file.clone(), key_alias: request.key_alias.clone() });
        let variant = self.variant.clone();
        self.run.update(cx, |run, cx| run.generate_signed(root, module, variant, request, cx));
        true
    }
}

impl Render for SignedApkDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let label = |text: &'static str| div().w(px(130.)).flex_none().text_sm().child(text);
        let row = |name: &'static str, field: gpui_kit::AnyElement| h_flex().gap_2().child(label(name)).child(div().flex_1().child(field));
        let entity = cx.entity();
        let module_name = self.modules.get(self.module).map(|(_, m)| m.name()).unwrap_or_default();
        let module_names: Vec<String> = self.modules.iter().map(|(_, m)| m.name()).collect();
        let variants = self.modules.get(self.module).map(|(_, m)| m.variants.clone()).unwrap_or_default();
        let format_ix = (self.format == PackageFormat::Apk) as usize;
        let e = entity.clone();
        let module_button = Button::new("signed-module").small().outline().label(module_name).dropdown_menu(move |mut menu, _, _| {
            for (ix, name) in module_names.iter().enumerate() {
                let e = e.clone();
                menu = menu.item(PopupMenuItem::new(name.clone()).on_click(move |_, window, cx| e.update(cx, |this, cx| this.select_module(ix, window, cx))));
            }
            menu
        });
        let e = entity.clone();
        let variant_button = Button::new("signed-variant").small().outline().label(self.variant.clone()).dropdown_menu(move |mut menu, _, _| {
            for variant in &variants {
                let (e, variant) = (e.clone(), variant.clone());
                menu = menu.item(PopupMenuItem::new(variant.clone()).on_click(move |_, _, cx| {
                    e.update(cx, |this, cx| {
                        this.variant = variant.clone();
                        cx.notify();
                    })
                }));
            }
            menu
        });
        let e = entity.clone();
        v_flex()
            .gap_3()
            .child(
                RadioGroup::new("signed-format")
                    .children([PackageFormat::Bundle.label(), PackageFormat::Apk.label()])
                    .selected_index(Some(format_ix))
                    .on_change(move |ix, _, cx| {
                        e.update(cx, |this, cx| {
                            this.format = if *ix == 1 { PackageFormat::Apk } else { PackageFormat::Bundle };
                            cx.notify();
                        })
                    }),
            )
            .child(row("Module:", h_flex().child(module_button).into_any_element()))
            .child(row("Build variant:", h_flex().child(variant_button).into_any_element()))
            .child(div().h(px(1.)).bg(palette.border))
            .child(
                h_flex()
                    .gap_2()
                    .child(label("Key store path:"))
                    .child(div().flex_1().child(Input::new(&self.store_file).small()))
                    .child(Button::new("signed-browse").small().label("Choose existing…").on_click(cx.listener(|this, _, _, cx| this.browse(cx)))),
            )
            .child(row("Key store password:", Input::new(&self.store_password).small().mask_toggle().into_any_element()))
            .child(row("Key alias:", Input::new(&self.key_alias).small().into_any_element()))
            .child(row("Key password:", Input::new(&self.key_password).small().mask_toggle().into_any_element()))
            .when_some(self.error, |el, error| el.child(div().pl(px(138.)).text_xs().text_color(palette.status_conflict).child(error)))
    }
}

/// The application modules of the project's builds, as Build Variants lists
/// them.
pub fn application_modules(builds: &[(PathBuf, Vec<ModuleInfo>)]) -> Vec<(PathBuf, ModuleInfo)> {
    builds.iter().flat_map(|(root, modules)| modules.iter().filter(|m| m.kind == ModuleType::Application).map(move |m| (root.clone(), m.clone()))).collect()
}

pub fn open(modules: Vec<(PathBuf, ModuleInfo)>, store: Rc<dyn SigningStore>, run: Entity<RunService>, window: &mut Window, cx: &mut App) {
    if modules.is_empty() {
        window.push_notification(
            gpui_kit::component::notification::Notification::error("The project has no Android application module. Sync the project with its Gradle files first.").title("Generate Signed Bundle / APK"),
            cx,
        );
        return;
    }
    let view = cx.new(|cx| SignedApkDialog::new(modules, store, run.clone(), window, cx));
    let focus = view.read(cx).store_password.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let busy = run.read(cx).is_busy();
        let view_ok = view.clone();
        dialog
            .title("Generate Signed Bundle or APK")
            .w(px(560.))
            .child(view.clone())
            .on_ok(move |_, _, cx| view_ok.update(cx, |this, cx| this.create(cx)))
            .footer(footer_enabled("Create", !busy))
    });
    focus_input(&focus, window, cx);
}
