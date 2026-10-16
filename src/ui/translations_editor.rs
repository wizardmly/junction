//! The Translations Editor, as Android Studio's: a tab in the editor area
//! with a module's string resources as a table (Key, Resource Folder,
//! Untranslatable, Default Value, then a column per locale), missing
//! translations in red; the toolbar's Add Key, Remove Keys, Add Locale,
//! the key filter and Reload; and the Key / Default Value / Translation
//! fields under the table, which write the values files on Enter.

use std::collections::BTreeSet;
use std::path::PathBuf;

use gpui_kit::component::{
    Disableable as _, Icon, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    Pixels, Render, ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, UniformListScrollHandle, Window, div, prelude::FluentBuilder as _, px,
    uniform_list,
};

use crate::domain::translations::{self as tr, Locale};
use crate::infra::res_files::DiskRes;
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::common::tool_button;
use crate::ui::dialogs;
use crate::usecase::translations::{self as usecase, Row, Table};

pub enum TranslationsEvent {
    /// Values files written: the project's paths, for open editors.
    FilesChanged(Vec<PathBuf>),
}

/// Android Studio's key filter.
#[derive(Clone, Copy, PartialEq, Eq)]
enum KeyFilter {
    All,
    Translatable,
    NeedingTranslations,
}

impl KeyFilter {
    fn label(self) -> &'static str {
        match self {
            KeyFilter::All => "Show All Keys",
            KeyFilter::Translatable => "Show Translatable Keys",
            KeyFilter::NeedingTranslations => "Show Keys Needing Translations",
        }
    }
}

/// A cell: the Default Value or a locale's column.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Column {
    Key,
    Default,
    Locale(usize),
}

const KEY_W: f32 = 200.;
const FOLDER_W: f32 = 170.;
const UNTRANSLATABLE_W: f32 = 110.;
const DEFAULT_W: f32 = 220.;
const LOCALE_W: f32 = 200.;
const ROW_H: f32 = 24.;

pub struct TranslationsEditor {
    res: PathBuf,
    /// "app/src/main/res", for the Resource Folder column.
    folder_label: String,
    table: Table,
    filter: KeyFilter,
    /// Indexes into `table.rows` the filter lets through.
    shown: Vec<usize>,
    /// The selected keys (Remove Keys acts on them) and the current cell.
    selected: BTreeSet<String>,
    cell: Option<(String, Column)>,
    key_field: Entity<InputState>,
    default_field: Entity<InputState>,
    translation_field: Entity<InputState>,
    error: Option<String>,
    scroll: UniformListScrollHandle,
    h_scroll: ScrollHandle,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TranslationsEvent> for TranslationsEditor {}

impl Focusable for TranslationsEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TranslationsEditor {
    pub fn new(res: PathBuf, folder_label: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let key_field = cx.new(|cx| InputState::new(window, cx));
        let default_field = cx.new(|cx| InputState::new(window, cx));
        let translation_field = cx.new(|cx| InputState::new(window, cx));
        let _subscriptions = vec![
            cx.subscribe_in(&default_field, window, |this, _, e: &InputEvent, _, cx| {
                if matches!(e, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.apply_field(false, cx);
                }
            }),
            cx.subscribe_in(&translation_field, window, |this, _, e: &InputEvent, _, cx| {
                if matches!(e, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.apply_field(true, cx);
                }
            }),
        ];
        let mut this = Self {
            res,
            folder_label,
            table: Table::default(),
            filter: KeyFilter::All,
            shown: Vec::new(),
            selected: BTreeSet::new(),
            cell: None,
            key_field,
            default_field,
            translation_field,
            error: None,
            scroll: UniformListScrollHandle::new(),
            h_scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
            _subscriptions,
        };
        this.reload(cx);
        this
    }

    pub fn res(&self) -> &PathBuf {
        &self.res
    }

    pub fn title(&self) -> String {
        "Translations Editor".into()
    }

    fn files(&self) -> DiskRes {
        DiskRes { res: self.res.clone() }
    }

    /// Reads the values files again (Reload, and after each edit).
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.table = usecase::load(&self.files());
        self.refilter();
        cx.notify();
    }

    fn refilter(&mut self) {
        let filter = self.filter;
        self.shown = (0..self.table.rows.len())
            .filter(|ix| {
                let row = &self.table.rows[*ix];
                match filter {
                    KeyFilter::All => true,
                    KeyFilter::Translatable => row.translatable,
                    KeyFilter::NeedingTranslations => row.needs_translation(),
                }
            })
            .collect();
        let keys: BTreeSet<&str> = self.table.rows.iter().map(|r| r.key.as_str()).collect();
        self.selected.retain(|k| keys.contains(k.as_str()));
    }

    fn row(&self, key: &str) -> Option<&Row> {
        self.table.rows.iter().find(|r| r.key == key)
    }

    /// The files of a column's folder that changed, as project paths.
    fn changed(&self, before: &[(String, String, Option<String>)], cx: &mut Context<Self>) {
        let files = self.files();
        use crate::usecase::translations::ResFiles as _;
        let mut paths: Vec<PathBuf> = Vec::new();
        for (folder, file) in files.values_files() {
            let now = files.read(&folder, &file);
            let old = before.iter().find(|(f, n, _)| *f == folder && *n == file).and_then(|(_, _, t)| t.clone());
            if now != old {
                paths.push(self.res.join(&folder).join(&file));
            }
        }
        if !paths.is_empty() {
            cx.emit(TranslationsEvent::FilesChanged(paths));
        }
    }

    /// Runs an edit of the values files, then shows the new table.
    fn edit(&mut self, f: impl FnOnce(&DiskRes) -> Result<(), String>, cx: &mut Context<Self>) {
        use crate::usecase::translations::ResFiles as _;
        let files = self.files();
        let before: Vec<(String, String, Option<String>)> = files.values_files().into_iter().map(|(folder, file)| {
            let text = files.read(&folder, &file);
            (folder, file, text)
        }).collect();
        self.error = f(&files).err();
        self.changed(&before, cx);
        self.reload(cx);
    }

    /// Selects a cell and shows it in the fields below.
    fn select(&mut self, key: String, column: Column, extend: bool, window: &mut Window, cx: &mut Context<Self>) {
        if extend {
            if !self.selected.remove(&key) {
                self.selected.insert(key.clone());
            }
        } else {
            self.selected = BTreeSet::from([key.clone()]);
        }
        self.cell = Some((key, column));
        self.fill_fields(window, cx);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn fill_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (key, default, translation) = match &self.cell {
            Some((key, column)) => {
                let row = self.row(key);
                let default = row.and_then(|r| r.default.clone()).unwrap_or_default();
                let translation = match column {
                    Column::Locale(l) => row.and_then(|r| r.values.get(*l).cloned().flatten()).unwrap_or_default(),
                    _ => String::new(),
                };
                (key.clone(), default, translation)
            }
            None => Default::default(),
        };
        self.key_field.update(cx, |s, cx| s.set_value(key, window, cx));
        self.default_field.update(cx, |s, cx| s.set_value(default, window, cx));
        self.translation_field.update(cx, |s, cx| s.set_value(translation, window, cx));
    }

    /// Enter (or leaving) in the Default Value or Translation field.
    fn apply_field(&mut self, translation: bool, cx: &mut Context<Self>) {
        let Some((key, column)) = self.cell.clone() else { return };
        let Some(row) = self.row(&key).cloned() else { return };
        let (value, current, locale) = if translation {
            let Column::Locale(l) = column else { return };
            (self.translation_field.read(cx).value().to_string(), row.values.get(l).cloned().flatten().unwrap_or_default(), self.table.locales.get(l).cloned())
        } else {
            (self.default_field.read(cx).value().to_string(), row.default.clone().unwrap_or_default(), None)
        };
        if value == current {
            return;
        }
        self.edit(|files| usecase::set_value(files, &key, locale.as_ref(), &value), cx);
    }

    fn toggle_translatable(&mut self, key: String, translatable: bool, cx: &mut Context<Self>) {
        self.edit(|files| usecase::set_translatable(files, &key, translatable), cx);
    }

    fn add_locale(&mut self, locale: Locale, cx: &mut Context<Self>) {
        self.edit(|files| usecase::add_locale(files, &locale), cx);
    }

    fn set_filter(&mut self, filter: KeyFilter, cx: &mut Context<Self>) {
        self.filter = filter;
        self.refilter();
        cx.notify();
    }

    /// Remove Keys: the selected keys, from every locale, once confirmed.
    fn remove_keys(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let keys: Vec<String> = self.selected.iter().cloned().collect();
        if keys.is_empty() {
            return;
        }
        let message = match keys.as_slice() {
            [key] => format!("Delete string resource \"{key}\" from all locales?"),
            _ => format!("Delete {} string resources from all locales?", keys.len()),
        };
        let this = cx.entity().downgrade();
        dialogs::confirm("Remove Keys", message, "Delete", move |cx| {
            let keys = keys.clone();
            this.update(cx, |this, cx| {
                this.cell = None;
                this.edit(|files| usecase::remove_keys(files, &keys), cx);
            })
            .ok();
        }, window, cx);
    }

    /// Add Key: Key, Default Value; OK checks the key as Android Studio does.
    fn add_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = cx.new(|cx| InputState::new(window, cx));
        let value = cx.new(|cx| InputState::new(window, cx));
        let error: Entity<Option<String>> = cx.new(|_| None);
        let this = cx.entity().downgrade();
        let folder = self.folder_label.clone();
        let palette = cx.palette().clone();
        let focus_key = key.clone();
        window.open_dialog(cx, move |dialog, _, cx| {
            let label = |text: &'static str| div().w(px(110.)).text_sm().child(text);
            let (key_ok, value_ok, error_ok, this) = (key.clone(), value.clone(), error.clone(), this.clone());
            dialog
                .title("Add Key")
                .w(px(460.))
                .child(
                    v_flex()
                        .gap_2()
                        .child(h_flex().gap_2().child(label("Key:")).child(div().flex_1().child(Input::new(&key))))
                        .child(h_flex().gap_2().child(label("Default Value:")).child(div().flex_1().child(Input::new(&value))))
                        .child(h_flex().gap_2().child(label("Resource Folder:")).child(div().flex_1().text_sm().child(folder.clone())))
                        .when_some(error.read(cx).clone(), |el, e| el.child(div().text_sm().text_color(palette.status_conflict).child(e))),
                )
                .footer(dialogs::footer("OK"))
                .on_ok(move |_, window, cx| {
                    let (k, v) = (key_ok.read(cx).value().trim().to_owned(), value_ok.read(cx).value().to_string());
                    let Some(this) = this.upgrade() else { return true };
                    let result = this.update(cx, |this, _| usecase::add_key(&this.files(), &k, &v).err());
                    match result {
                        Some(e) => {
                            error_ok.update(cx, |slot, cx| {
                                *slot = Some(e);
                                cx.notify();
                            });
                            false
                        }
                        None => {
                            this.update(cx, |this, cx| {
                                // Written already: show it, selected.
                                this.changed(&[], cx);
                                this.reload(cx);
                                this.selected = BTreeSet::from([k.clone()]);
                                this.cell = Some((k, Column::Default));
                                this.fill_fields(window, cx);
                            });
                            true
                        }
                    }
                })
        });
        dialogs::focus_input(&focus_key, window, cx);
    }

    fn header_cell(text: impl Into<SharedString>, width: f32, palette: &crate::theme::Palette) -> gpui_kit::Div {
        div()
            .w(px(width))
            .flex_none()
            .h_full()
            .px_2()
            .flex()
            .items_center()
            .border_r_1()
            .border_color(palette.border)
            .overflow_hidden()
            .whitespace_nowrap()
            .font_weight(FontWeight::SEMIBOLD)
            .child(text.into())
    }

    fn render_rows(&mut self, range: std::ops::Range<usize>, _: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let palette = cx.palette().clone();
        let missing_bg: Hsla = gpui_kit::rgba(if palette.dark { 0xdb5c5c26 } else { 0xe557651f }).into();
        let missing_fg: Hsla = gpui_kit::rgb(if palette.dark { 0xff6b68 } else { 0xcd0000 }).into();
        let mut out = Vec::new();
        for ix in range {
            let Some(row) = self.shown.get(ix).and_then(|r| self.table.rows.get(*r)).cloned() else { continue };
            let selected = self.selected.contains(&row.key);
            let current = self.cell.as_ref().filter(|(k, _)| *k == row.key).map(|(_, c)| *c);
            let cell = |id: (&'static str, usize), width: f32, column: Column, cx: &mut Context<Self>| {
                let key = row.key.clone();
                div()
                    .id(id)
                    .w(px(width))
                    .flex_none()
                    .h_full()
                    .px_2()
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .border_r_1()
                    .border_color(palette.border)
                    .when(current == Some(column), |el| el.border_1().border_color(palette.accent))
                    .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                        let extend = e.modifiers().control || e.modifiers().platform || e.modifiers().shift;
                        this.select(key.clone(), column, extend, window, cx);
                        if e.click_count() == 2 {
                            // Double-click edits in the field below.
                            let field = if matches!(column, Column::Locale(_)) { this.translation_field.clone() } else { this.default_field.clone() };
                            field.update(cx, |s, cx| s.focus(window, cx));
                        }
                    }))
            };
            let key_color = if row.needs_translation() { missing_fg } else { palette.text };
            let mut line = h_flex()
                .id(("translation-row", ix))
                .h(px(ROW_H))
                .border_b_1()
                .border_color(palette.border)
                .when(selected, |el| el.bg(palette.selection))
                .child(cell(("tr-key", ix), KEY_W, Column::Key, cx).text_color(key_color).child(row.key.clone()))
                .child(cell(("tr-folder", ix), FOLDER_W, Column::Key, cx).text_color(palette.text_secondary).child(self.folder_label.clone()));
            let key = row.key.clone();
            line = line.child(
                div().w(px(UNTRANSLATABLE_W)).flex_none().h_full().flex().items_center().justify_center().border_r_1().border_color(palette.border).child(
                    Checkbox::new(("tr-untranslatable", ix)).checked(!row.translatable).disabled(row.default.is_none()).on_click(cx.listener(move |this, checked: &bool, _, cx| {
                        this.toggle_translatable(key.clone(), !*checked, cx);
                    })),
                ),
            );
            line = line.child(cell(("tr-default", ix), DEFAULT_W, Column::Default, cx).when(row.default.is_none(), |el| el.bg(missing_bg)).child(row.default.clone().unwrap_or_default()));
            for (l, value) in row.values.iter().enumerate() {
                let missing = row.missing(l);
                line = line.child(cell(("tr-locale", ix * 1000 + l), LOCALE_W, Column::Locale(l), cx).when(missing, |el| el.bg(missing_bg)).child(value.clone().unwrap_or_default()));
            }
            out.push(line.into_any_element());
        }
        out
    }
}

/// The table's width: the fixed columns and one per locale.
fn table_width(locales: usize) -> Pixels {
    px(KEY_W + FOLDER_W + UNTRANSLATABLE_W + DEFAULT_W + LOCALE_W * locales as f32)
}

impl Render for TranslationsEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let this = cx.entity().downgrade();
        let present: Vec<Locale> = self.table.locales.clone();
        let add_locale = Button::new("tr-add-locale").ghost().xsmall().icon(Icon::new(icons::LANGUAGE)).child(Icon::new(icons::CHEVRON_DOWN).xsmall()).tooltip("Add Locale").dropdown_menu(move |mut menu, _, _| {
            menu = menu.scrollable(true).max_h(px(420.));
            for (code, name) in tr::LANGUAGES.iter().filter(|(c, _)| !matches!(*c, "in" | "iw" | "no")) {
                let locale = Locale::new(code, None);
                if present.contains(&locale) {
                    continue;
                }
                let this = this.clone();
                menu = menu.item(PopupMenuItem::new(format!("{name} ({code})")).on_click(move |_, _, cx| {
                    let locale = locale.clone();
                    this.update(cx, |v, cx| v.add_locale(locale, cx)).ok();
                }));
            }
            menu
        });
        let filter = self.filter;
        let this = cx.entity().downgrade();
        let filter_button = Button::new("tr-filter").ghost().small().label(filter.label()).child(Icon::new(icons::CHEVRON_DOWN).xsmall()).dropdown_menu(move |mut menu, _, _| {
            for f in [KeyFilter::All, KeyFilter::Translatable, KeyFilter::NeedingTranslations] {
                let this = this.clone();
                menu = menu.item(PopupMenuItem::new(f.label()).checked(f == filter).on_click(move |_, _, cx| {
                    this.update(cx, |v, cx| v.set_filter(f, cx)).ok();
                }));
            }
            menu
        });
        let toolbar = h_flex()
            .h(px(32.))
            .px_1()
            .gap_1()
            .flex_none()
            .border_b_1()
            .border_color(palette.border)
            .child(tool_button("tr-add-key", icons::ADD, "Add Key").on_click(cx.listener(|this, _, window, cx| this.add_key(window, cx))))
            .child(tool_button("tr-remove-keys", icons::REMOVE, "Remove Keys").on_click(cx.listener(|this, _, window, cx| this.remove_keys(window, cx))))
            .child(add_locale)
            .child(div().w(px(1.)).h(px(16.)).mx_1().bg(palette.border))
            .child(filter_button)
            .child(tool_button("tr-reload", icons::REFRESH, "Reload").on_click(cx.listener(|this, _, _, cx| this.reload(cx))))
            .child(div().flex_1())
            .when_some(self.error.clone(), |el, e| el.child(div().text_sm().text_color(palette.status_conflict).child(e)));
        let mut header = h_flex()
            .h(px(ROW_H + 4.))
            .flex_none()
            .bg(palette.toolbar)
            .border_b_1()
            .border_color(palette.border)
            .child(Self::header_cell("Key", KEY_W, &palette))
            .child(Self::header_cell("Resource Folder", FOLDER_W, &palette))
            .child(Self::header_cell("Untranslatable", UNTRANSLATABLE_W, &palette))
            .child(Self::header_cell("Default Value", DEFAULT_W, &palette));
        for locale in &self.table.locales {
            header = header.child(Self::header_cell(locale.label(), LOCALE_W, &palette));
        }
        let width = table_width(self.table.locales.len());
        let body: AnyElement = if self.table.rows.is_empty() {
            div().p_4().text_color(palette.text_secondary).child("No string resources").into_any_element()
        } else {
            uniform_list("translations", self.shown.len(), cx.processor(Self::render_rows)).track_scroll(&self.scroll).flex_1().into_any_element()
        };
        let table = div()
            .id("translations-h-scroll")
            .flex_1()
            .min_h_0()
            .overflow_x_scroll()
            .track_scroll(&self.h_scroll)
            .child(v_flex().w(width).h_full().child(header).child(body));
        let label = |text: &'static str| div().w(px(100.)).flex_none().text_sm().child(text);
        let translation_label = match &self.cell {
            Some((_, Column::Locale(l))) => self.table.locales.get(*l).map(|l| l.label()).unwrap_or_default(),
            _ => String::new(),
        };
        let editing_translation = matches!(self.cell, Some((_, Column::Locale(_))));
        let fields = v_flex()
            .flex_none()
            .p_2()
            .gap_1()
            .border_t_1()
            .border_color(palette.border)
            .child(h_flex().gap_2().child(label("Key:")).child(div().flex_1().child(Input::new(&self.key_field).small().disabled(true))))
            .child(h_flex().gap_2().child(label("Default Value:")).child(div().flex_1().child(Input::new(&self.default_field).small().disabled(self.cell.is_none()))))
            .child(
                h_flex()
                    .gap_2()
                    .child(label("Translation:"))
                    .child(div().flex_1().child(Input::new(&self.translation_field).small().disabled(!editing_translation)))
                    .child(div().w(px(200.)).text_xs().text_color(palette.text_secondary).child(translation_label)),
            );
        v_flex().size_full().bg(palette.panel).track_focus(&self.focus).text_sm().child(toolbar).child(table).child(fields)
    }
}

/// The project's `res` folder holding a values file, for the editor's
/// Resource Folder column: `app/src/main/res`.
pub fn res_of(path: &std::path::Path) -> Option<PathBuf> {
    let folder = path.parent()?;
    let name = folder.file_name()?.to_string_lossy();
    (name == "values" || name.starts_with("values-")).then(|| folder.parent().map(PathBuf::from)).flatten()
}
