//! Build › Generate Signed Bundle / APK…, Android Studio's wizard in one
//! dialog: the package (App Bundle or APK), the application module and its
//! variant, the destination folder, the keystore and the key. The keystore
//! path, the key alias and the destination come back next time for the
//! module; the passwords with Remember passwords, from the system's
//! password safe. Create new… opens New Key Store, which makes one with
//! the JDK's keytool.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::{
    Sizable as _, WindowExt as _, h_flex,
    button::Button,
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    radio::RadioGroup,
    v_flex,
};
use gpui_kit::{App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, PathPromptOptions, Render, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px};

use crate::domain::password_safe::PasswordSafe;
use crate::domain::run::{ModuleInfo, ModuleType};
use crate::domain::signing::{self, KeyTool, NewKeystore, PackageFormat, RememberedKey, SigningRequest, SigningStore};
use crate::theme::ActivePalette as _;
use crate::ui::dialogs::{focus_input, footer_enabled};
use crate::ui::run_service::RunService;

pub struct SignedApkDialog {
    /// The application modules, with their build's root.
    modules: Vec<(PathBuf, ModuleInfo)>,
    module: usize,
    variant: String,
    format: PackageFormat,
    destination: Entity<InputState>,
    store_file: Entity<InputState>,
    store_password: Entity<InputState>,
    key_alias: Entity<InputState>,
    key_password: Entity<InputState>,
    remember_passwords: bool,
    /// What the module remembers, the destinations of both formats among it.
    remembered: RememberedKey,
    error: Option<&'static str>,
    store: Rc<dyn SigningStore>,
    passwords: Arc<dyn PasswordSafe>,
    run: Entity<RunService>,
    /// The keytool for a build root.
    keytool: KeyToolFor,
    _subscriptions: Vec<Subscription>,
}

/// Makes the keytool of a build's JDK.
pub type KeyToolFor = Rc<dyn Fn(PathBuf) -> Box<dyn KeyTool + Send>>;

impl SignedApkDialog {
    fn new(modules: Vec<(PathBuf, ModuleInfo)>, store: Rc<dyn SigningStore>, passwords: Arc<dyn PasswordSafe>, run: Entity<RunService>, keytool: KeyToolFor, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let destination = cx.new(|cx| InputState::new(window, cx));
        let store_file = cx.new(|cx| InputState::new(window, cx).placeholder("Path to the .jks or .keystore file"));
        let store_password = cx.new(|cx| InputState::new(window, cx).masked(true));
        let key_alias = cx.new(|cx| InputState::new(window, cx));
        let key_password = cx.new(|cx| InputState::new(window, cx).masked(true));
        // An edit clears the last check's message.
        let subscriptions = [&destination, &store_file, &store_password, &key_alias, &key_password]
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
            destination,
            store_file,
            store_password,
            key_alias,
            key_password,
            remember_passwords: false,
            remembered: RememberedKey::default(),
            error: None,
            store,
            passwords,
            run,
            keytool,
            _subscriptions: subscriptions,
        };
        this.select_module(0, window, cx);
        this
    }

    /// A module: its release variant, and the package, destination and key
    /// it was signed with last.
    fn select_module(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, module)) = self.modules.get(ix) else { return };
        self.module = ix;
        self.variant = signing::default_variant(module);
        let dir = module.dir.clone();
        match self.store.load(&dir) {
            Some(key) => {
                self.store_file.update(cx, |s, cx| s.set_value(key.store_file.display().to_string(), window, cx));
                self.key_alias.update(cx, |s, cx| s.set_value(key.key_alias.clone(), window, cx));
                self.format = key.format();
                self.remember_passwords = key.remember_passwords;
                self.remembered = key;
            }
            None => self.remembered = RememberedKey::default(),
        }
        let destination = self.remembered.destination(self.format, &dir);
        self.destination.update(cx, |s, cx| s.set_value(destination.display().to_string(), window, cx));
        self.error = None;
        self.load_passwords(window, cx);
        cx.notify();
    }

    fn set_format(&mut self, format: PackageFormat, window: &mut Window, cx: &mut Context<Self>) {
        if format == self.format {
            return;
        }
        self.format = format;
        if let Some((_, module)) = self.modules.get(self.module) {
            let destination = self.remembered.destination(format, &module.dir);
            self.destination.update(cx, |s, cx| s.set_value(destination.display().to_string(), window, cx));
        }
        cx.notify();
    }

    /// The keystore's saved passwords, read in the background (the safe may
    /// ask to be unlocked) into the password fields still empty, as Android
    /// Studio's `KeystoreStep.tryLoadSavedPasswords`.
    fn load_passwords(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let store_file = PathBuf::from(self.store_file.read(cx).value().trim());
        let alias = self.key_alias.read(cx).value().to_string();
        if store_file.as_os_str().is_empty() {
            return;
        }
        let (store_key, key_key) = signing::password_keys(&store_file, &alias);
        let safe = self.passwords.clone();
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let (store, key) = cx.background_spawn(async move { (safe.get(&store_key), safe.get(&key_key)) }).await;
            let _ = handle.update(cx, |_, window, cx| {
                this.update(cx, |this, cx| {
                    for (input, password) in [(this.store_password.clone(), store), (this.key_password.clone(), key)] {
                        if let Some(password) = password
                            && input.read(cx).value().is_empty()
                        {
                            input.update(cx, |s, cx| s.set_value(password, window, cx));
                        }
                    }
                })
                .ok();
            });
        })
        .detach();
    }

    fn request(&self, cx: &App) -> SigningRequest {
        SigningRequest {
            format: self.format,
            store_file: PathBuf::from(self.store_file.read(cx).value().trim()),
            store_password: self.store_password.read(cx).value().to_string(),
            key_alias: self.key_alias.read(cx).value().trim().to_owned(),
            key_password: self.key_password.read(cx).value().to_string(),
            destination: PathBuf::from(self.destination.read(cx).value().trim()),
        }
    }

    fn browse(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Choose Key Store File".into()) });
        let input = self.store_file.clone();
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            cx.update(|cx| {
                if let Some(window) = cx.active_window() {
                    window
                        .update(cx, |_, window, cx| {
                            input.update(cx, |s, cx| s.set_value(path.display().to_string(), window, cx));
                            this.update(cx, |this, cx| this.load_passwords(window, cx)).ok();
                        })
                        .ok();
                }
            });
        })
        .detach();
    }

    /// Destination Folder's browse button: Select APK Destination Folder.
    fn browse_destination(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Select APK Destination Folder".into()) });
        let input = self.destination.clone();
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

    /// Create new…: New Key Store over this dialog; the keystore made fills
    /// the fields.
    fn create_keystore(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((root, _)) = self.modules.get(self.module) else { return };
        let keytool = (self.keytool)(root.clone());
        let parent = cx.entity();
        NewKeystoreDialog::open(keytool, parent, window, cx);
    }

    fn fill(&mut self, new: &NewKeystore, window: &mut Window, cx: &mut Context<Self>) {
        self.store_file.update(cx, |s, cx| s.set_value(new.store_file.display().to_string(), window, cx));
        self.store_password.update(cx, |s, cx| s.set_value(new.store_password.clone(), window, cx));
        self.key_alias.update(cx, |s, cx| s.set_value(new.key_alias.trim().to_owned(), window, cx));
        self.key_password.update(cx, |s, cx| s.set_value(new.key_password.clone(), window, cx));
        self.error = None;
        cx.notify();
    }

    /// Create: checks the fields, remembers the keystore, alias and
    /// destination (and the passwords, or forgets them), and starts the
    /// build; false keeps the dialog open on an error.
    fn create(&mut self, cx: &mut Context<Self>) -> bool {
        let request = self.request(cx);
        let writable = std::fs::metadata(&request.destination).is_ok_and(|m| m.is_dir() && !m.permissions().readonly());
        if let Err(error) = signing::check(&request, request.store_file.is_file()).and_then(|()| signing::check_destination(&request, writable)) {
            self.error = Some(error);
            cx.notify();
            return false;
        }
        let Some((root, module)) = self.modules.get(self.module).cloned() else { return false };
        self.remembered.record(&request, self.remember_passwords);
        self.store.save(&module.dir, &self.remembered);
        let (store_key, key_key) = signing::password_keys(&request.store_file, &request.key_alias);
        let safe = self.passwords.clone();
        let passwords = self.remember_passwords.then(|| (request.store_password.clone(), request.key_password.clone()));
        cx.background_spawn(async move {
            safe.set(&store_key, passwords.as_ref().map(|p| p.0.as_str()));
            safe.set(&key_key, passwords.as_ref().map(|p| p.1.as_str()));
        })
        .detach();
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
                    .on_change(move |ix, window, cx| e.update(cx, |this, cx| this.set_format(if *ix == 1 { PackageFormat::Apk } else { PackageFormat::Bundle }, window, cx))),
            )
            .child(row("Module:", h_flex().child(module_button).into_any_element()))
            .child(row("Build variant:", h_flex().child(variant_button).into_any_element()))
            .child(
                h_flex()
                    .gap_2()
                    .child(label("Destination folder:"))
                    .child(div().flex_1().child(Input::new(&self.destination).small()))
                    .child(Button::new("signed-destination").small().label("…").on_click(cx.listener(|this, _, _, cx| this.browse_destination(cx)))),
            )
            .child(div().h(px(1.)).bg(palette.border))
            .child(
                h_flex()
                    .gap_2()
                    .child(label("Key store path:"))
                    .child(div().flex_1().child(Input::new(&self.store_file).small()))
                    .child(Button::new("signed-create").small().label("Create new…").on_click(cx.listener(|this, _, window, cx| this.create_keystore(window, cx))))
                    .child(Button::new("signed-browse").small().label("Choose existing…").on_click(cx.listener(|this, _, _, cx| this.browse(cx)))),
            )
            .child(row("Key store password:", Input::new(&self.store_password).small().mask_toggle().into_any_element()))
            .child(row("Key alias:", Input::new(&self.key_alias).small().into_any_element()))
            .child(row("Key password:", Input::new(&self.key_password).small().mask_toggle().into_any_element()))
            .child(
                div().pl(px(138.)).child(Checkbox::new("signed-remember").label("Remember passwords").checked(self.remember_passwords).on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.remember_passwords = *checked;
                    cx.notify();
                }))),
            )
            .when_some(self.error, |el, error| el.child(div().pl(px(138.)).text_xs().text_color(palette.status_conflict).child(error)))
    }
}

/// The application modules of the project's builds, as Build Variants lists
/// them.
pub fn application_modules(builds: &[(PathBuf, Vec<ModuleInfo>)]) -> Vec<(PathBuf, ModuleInfo)> {
    builds.iter().flat_map(|(root, modules)| modules.iter().filter(|m| m.kind == ModuleType::Application).map(move |m| (root.clone(), m.clone()))).collect()
}

pub fn open(modules: Vec<(PathBuf, ModuleInfo)>, store: Rc<dyn SigningStore>, passwords: Arc<dyn PasswordSafe>, run: Entity<RunService>, keytool: KeyToolFor, window: &mut Window, cx: &mut App) {
    if modules.is_empty() {
        window.push_notification(
            gpui_kit::component::notification::Notification::error("The project has no Android application module. Sync the project with its Gradle files first.").title("Generate Signed Bundle / APK"),
            cx,
        );
        return;
    }
    let view = cx.new(|cx| SignedApkDialog::new(modules, store, passwords, run.clone(), keytool, window, cx));
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

/// New Key Store: the keystore, its key and the key's certificate.
pub struct NewKeystoreDialog {
    store_file: Entity<InputState>,
    store_password: Entity<InputState>,
    store_confirm: Entity<InputState>,
    key_alias: Entity<InputState>,
    key_password: Entity<InputState>,
    key_confirm: Entity<InputState>,
    validity: Entity<InputState>,
    certificate: Vec<Entity<InputState>>,
    error: Option<String>,
    creating: bool,
    keytool: Option<Box<dyn KeyTool + Send>>,
    parent: Entity<SignedApkDialog>,
    _subscriptions: Vec<Subscription>,
}

impl NewKeystoreDialog {
    fn open(keytool: Box<dyn KeyTool + Send>, parent: Entity<SignedApkDialog>, window: &mut Window, cx: &mut App) {
        let view = cx.new(|cx| {
            let defaults = NewKeystore::default();
            let input = |value: &str, masked: bool, window: &mut Window, cx: &mut Context<Self>| cx.new(|cx| InputState::new(window, cx).masked(masked).default_value(value.to_owned()));
            let store_file = input("", false, window, cx);
            let store_password = input("", true, window, cx);
            let store_confirm = input("", true, window, cx);
            let key_alias = input(&defaults.key_alias, false, window, cx);
            let key_password = input("", true, window, cx);
            let key_confirm = input("", true, window, cx);
            let validity = input(&defaults.validity_years, false, window, cx);
            let certificate: Vec<_> = (0..6).map(|_| input("", false, window, cx)).collect();
            let subscriptions = [&store_file, &store_password, &store_confirm, &key_alias, &key_password, &key_confirm, &validity]
                .into_iter()
                .chain(certificate.iter())
                .map(|input| {
                    cx.subscribe(input, |this: &mut Self, _, event: &InputEvent, cx| {
                        if matches!(event, InputEvent::Change) && this.error.take().is_some() {
                            cx.notify();
                        }
                    })
                })
                .collect();
            Self { store_file, store_password, store_confirm, key_alias, key_password, key_confirm, validity, certificate, error: None, creating: false, keytool: Some(keytool), parent, _subscriptions: subscriptions }
        });
        let focus = view.read(cx).store_file.clone();
        window.open_dialog(cx, move |dialog, _, cx| {
            let view_ok = view.clone();
            let creating = view.read(cx).creating;
            dialog
                .title("New Key Store")
                .w(px(520.))
                .child(view.clone())
                .on_ok(move |_, window, cx| {
                    view_ok.update(cx, |this, cx| this.create(window, cx));
                    false
                })
                .footer(footer_enabled("OK", !creating))
        });
        focus_input(&focus, window, cx);
    }

    fn request(&self, cx: &App) -> NewKeystore {
        let text = |input: &Entity<InputState>| input.read(cx).value().to_string();
        NewKeystore {
            store_file: PathBuf::from(text(&self.store_file).trim()),
            store_password: text(&self.store_password),
            store_confirm: text(&self.store_confirm),
            key_alias: text(&self.key_alias),
            key_password: text(&self.key_password),
            key_confirm: text(&self.key_confirm),
            validity_years: text(&self.validity),
            certificate: std::array::from_fn(|i| text(&self.certificate[i])),
        }
    }

    fn browse(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Key Store Folder".into()) });
        let input = self.store_file.clone();
        cx.spawn(async move |_, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else { return };
            let Some(dir) = paths.into_iter().next() else { return };
            cx.update(|cx| {
                if let Some(window) = cx.active_window() {
                    let path = dir.join("upload-keystore.jks");
                    window.update(cx, |_, window, cx| input.update(cx, |s, cx| s.set_value(path.display().to_string(), window, cx))).ok();
                }
            });
        })
        .detach();
    }

    /// OK: checks, then keytool in the background; done, the fields of
    /// Generate Signed Bundle or APK take the new key and this closes.
    fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.creating {
            return;
        }
        let new = self.request(cx);
        let years = match signing::check_new(&new) {
            Ok(years) => years,
            Err(error) => {
                self.error = Some(error.into());
                cx.notify();
                return;
            }
        };
        if new.store_file.exists() {
            self.error = Some(format!("{} already exists.", new.store_file.display()));
            cx.notify();
            return;
        }
        let Some(keytool) = self.keytool.take() else { return };
        self.creating = true;
        cx.notify();
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let (keytool, result) = cx
                .background_spawn({
                    let new = new.clone();
                    async move {
                        let result = keytool.create(&new, years * 365);
                        (keytool, result)
                    }
                })
                .await;
            let _ = handle.update(cx, |_, window, cx| {
                this.update(cx, |this, cx| {
                    this.creating = false;
                    this.keytool = Some(keytool);
                    match result {
                        Ok(()) => {
                            this.parent.update(cx, |p, cx| p.fill(&new, window, cx));
                            window.close_dialog(cx);
                        }
                        Err(error) => this.error = Some(error),
                    }
                    cx.notify();
                })
                .ok();
            });
        })
        .detach();
    }
}

impl Render for NewKeystoreDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let label = |text: &'static str| div().w(px(150.)).flex_none().text_sm().child(text);
        let row = |name: &'static str, input: &Entity<InputState>| h_flex().gap_2().child(label(name)).child(div().flex_1().child(Input::new(input).small()));
        let heading = |text: &'static str| div().pt_1().text_sm().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(text);
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .child(label("Key store path:"))
                    .child(div().flex_1().child(Input::new(&self.store_file).small()))
                    .child(Button::new("keystore-browse").small().label("…").on_click(cx.listener(|this, _, _, cx| this.browse(cx)))),
            )
            .child(row("Password:", &self.store_password))
            .child(row("Confirm:", &self.store_confirm))
            .child(heading("Key"))
            .child(row("Alias:", &self.key_alias))
            .child(row("Password:", &self.key_password))
            .child(row("Confirm:", &self.key_confirm))
            .child(row("Validity (years):", &self.validity))
            .child(heading("Certificate"))
            .children(signing::CERTIFICATE_FIELDS.iter().zip(&self.certificate).map(|(name, input)| row(name, input)))
            .when(self.creating, |el| el.child(div().text_xs().text_color(palette.text_secondary).child("Creating the key store…")))
            .when_some(self.error.clone(), |el, error| el.child(div().text_xs().text_color(palette.status_conflict).child(error)))
    }
}
