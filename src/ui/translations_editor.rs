//! The Translations Editor, as Android Studio's: a tab in the editor area
//! with the project's string resources (every module's and source set's
//! res folder, a row per key and folder) as a table (Key, Resource Folder,
//! Untranslatable, Default Value, then a column per locale), missing
//! translations in red; the toolbar's Add Key, Remove Keys, Add Locale,
//! the key and locale filters and Reload; the Key / Default Value /
//! Translation fields under the table, which write the values files on
//! Enter. The columns before the locales stay put while those scroll
//! sideways; Delete removes the selected translations (or, in the key
//! columns, the keys).

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
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, UniformListScrollHandle, Window, div, prelude::FluentBuilder as _, px,
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
#[derive(Clone, PartialEq, Eq)]
enum KeyFilter {
    All,
    Translatable,
    NeedingTranslations,
    /// Keys missing this locale's translation.
    NeedingTranslationFor(Locale),
    /// Filter by Text: in the key, default value or a translation.
    Text(String),
}

impl KeyFilter {
    fn label(&self) -> String {
        match self {
            KeyFilter::All => "Show All Keys".into(),
            KeyFilter::Translatable => "Show Translatable Keys".into(),
            KeyFilter::NeedingTranslations => "Show Keys Needing Translations".into(),
            KeyFilter::NeedingTranslationFor(l) => format!("Show Keys Needing a Translation for {}", l.label()),
            KeyFilter::Text(t) => format!("Show Keys with Values Containing \"{t}\""),
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

/// A row: its res folder (an index into the editor's) and key.
type RowId = (usize, String);

pub struct TranslationsEditor {
    /// The res folders and their labels ("app/src/main/res"), the one it
    /// was opened for first: Add Key and Add Locale write there.
    folders: Vec<(PathBuf, String)>,
    table: Table,
    filter: KeyFilter,
    /// Show All Locales (none) or one locale's column.
    locale_filter: Option<Locale>,
    /// Indexes into `table.rows` the filter lets through.
    shown: Vec<usize>,
    /// How far the locale columns are scrolled sideways.
    h_offset: f32,
    /// The locale columns' visible width, as last laid out.
    h_visible: f32,
    /// The selected rows (Remove Keys acts on them) and the current cell.
    selected: BTreeSet<RowId>,
    cell: Option<(RowId, Column)>,
    key_field: Entity<InputState>,
    default_field: Entity<InputState>,
    translation_field: Entity<InputState>,
    error: Option<String>,
    scroll: UniformListScrollHandle,
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
    pub fn new(folders: Vec<(PathBuf, String)>, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
            folders,
            table: Table::default(),
            filter: KeyFilter::All,
            locale_filter: None,
            shown: Vec::new(),
            h_offset: 0.,
            h_visible: 0.,
            selected: BTreeSet::new(),
            cell: None,
            key_field,
            default_field,
            translation_field,
            error: None,
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            _subscriptions,
        };
        this.reload(cx);
        this
    }

    /// Whether it shows this res folder.
    pub fn covers(&self, res: &std::path::Path) -> bool {
        self.folders.iter().any(|(r, _)| r == res)
    }

    pub fn title(&self) -> String {
        "Translations Editor".into()
    }

    fn files(&self, folder: usize) -> DiskRes {
        DiskRes { res: self.folders.get(folder).map(|f| f.0.clone()).unwrap_or_default() }
    }

    /// Reads the values files again (Reload, and after each edit).
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let files: Vec<DiskRes> = (0..self.folders.len()).map(|f| self.files(f)).collect();
        let files: Vec<&dyn usecase::ResFiles> = files.iter().map(|f| f as &dyn usecase::ResFiles).collect();
        // The current cell's locale may move (or go) with the columns.
        let locale = match &self.cell {
            Some((_, Column::Locale(l))) => self.table.locales.get(*l).cloned(),
            _ => None,
        };
        self.table = usecase::load_many(&files);
        if let (Some((_, column)), Some(locale)) = (&mut self.cell, locale) {
            *column = self.table.locales.iter().position(|l| *l == locale).map_or(Column::Default, Column::Locale);
        }
        self.refilter();
        cx.notify();
    }

    fn refilter(&mut self) {
        let filter = &self.filter;
        let locales = &self.table.locales;
        self.shown = (0..self.table.rows.len())
            .filter(|ix| {
                let row = &self.table.rows[*ix];
                match filter {
                    KeyFilter::All => true,
                    KeyFilter::Translatable => row.translatable,
                    KeyFilter::NeedingTranslations => row.needs_translation(),
                    KeyFilter::NeedingTranslationFor(l) => locales.iter().position(|x| x == l).is_some_and(|l| row.missing(l)),
                    KeyFilter::Text(t) => {
                        let t = t.to_lowercase();
                        let has = |v: &str| v.to_lowercase().contains(&t);
                        has(&row.key) || row.default.as_deref().is_some_and(has) || row.values.iter().flatten().any(|v| has(v))
                    }
                }
            })
            .collect();
        if self.locale_filter.as_ref().is_some_and(|l| !locales.contains(l)) {
            self.locale_filter = None;
        }
        let rows: BTreeSet<(usize, &str)> = self.table.rows.iter().map(|r| (r.folder, r.key.as_str())).collect();
        self.selected.retain(|(f, k)| rows.contains(&(*f, k.as_str())));
    }

    fn row(&self, (folder, key): &RowId) -> Option<&Row> {
        self.table.rows.iter().find(|r| r.folder == *folder && r.key == *key)
    }

    /// The files of a res folder that changed, as project paths.
    fn changed(&self, folder: usize, before: &[(String, String, Option<String>)], cx: &mut Context<Self>) {
        let files = self.files(folder);
        use crate::usecase::translations::ResFiles as _;
        let mut paths: Vec<PathBuf> = Vec::new();
        for (folder, file) in files.values_files() {
            let now = files.read(&folder, &file);
            let old = before.iter().find(|(f, n, _)| *f == folder && *n == file).and_then(|(_, _, t)| t.clone());
            if now != old {
                paths.push(files.res.join(&folder).join(&file));
            }
        }
        if !paths.is_empty() {
            cx.emit(TranslationsEvent::FilesChanged(paths));
        }
    }

    /// Runs an edit of a res folder's values files, then shows the new
    /// table.
    fn edit(&mut self, folder: usize, f: impl FnOnce(&DiskRes) -> Result<(), String>, cx: &mut Context<Self>) {
        self.edit_quietly(folder, f, cx);
        self.reload(cx);
    }

    /// An edit without reloading the table.
    fn edit_quietly(&mut self, folder: usize, f: impl FnOnce(&DiskRes) -> Result<(), String>, cx: &mut Context<Self>) {
        use crate::usecase::translations::ResFiles as _;
        let files = self.files(folder);
        let before: Vec<(String, String, Option<String>)> = files.values_files().into_iter().map(|(folder, file)| {
            let text = files.read(&folder, &file);
            (folder, file, text)
        }).collect();
        if let Err(e) = f(&files) {
            self.error = Some(e);
        }
        self.changed(folder, &before, cx);
    }

    /// Selects a cell and shows it in the fields below.
    fn select(&mut self, id: RowId, column: Column, extend: bool, window: &mut Window, cx: &mut Context<Self>) {
        if extend {
            if !self.selected.remove(&id) {
                self.selected.insert(id.clone());
            }
        } else {
            self.selected = BTreeSet::from([id.clone()]);
        }
        self.cell = Some((id, column));
        self.fill_fields(window, cx);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn fill_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (key, default, translation) = match &self.cell {
            Some((id, column)) => {
                let row = self.row(id);
                let default = row.and_then(|r| r.default.clone()).unwrap_or_default();
                let translation = match column {
                    Column::Locale(l) => row.and_then(|r| r.values.get(*l).cloned().flatten()).unwrap_or_default(),
                    _ => String::new(),
                };
                (id.1.clone(), default, translation)
            }
            None => Default::default(),
        };
        self.key_field.update(cx, |s, cx| s.set_value(key, window, cx));
        self.default_field.update(cx, |s, cx| s.set_value(default, window, cx));
        self.translation_field.update(cx, |s, cx| s.set_value(translation, window, cx));
    }

    /// Enter (or leaving) in the Default Value or Translation field.
    fn apply_field(&mut self, translation: bool, cx: &mut Context<Self>) {
        let Some(((folder, key), column)) = self.cell.clone() else { return };
        let Some(row) = self.row(&(folder, key.clone())).cloned() else { return };
        let (value, current, locale) = if translation {
            let Column::Locale(l) = column else { return };
            (self.translation_field.read(cx).value().to_string(), row.values.get(l).cloned().flatten().unwrap_or_default(), self.table.locales.get(l).cloned())
        } else {
            (self.default_field.read(cx).value().to_string(), row.default.clone().unwrap_or_default(), None)
        };
        if value == current {
            return;
        }
        self.error = None;
        self.edit(folder, |files| usecase::set_value(files, &key, locale.as_ref(), &value), cx);
    }

    fn toggle_translatable(&mut self, (folder, key): RowId, translatable: bool, cx: &mut Context<Self>) {
        self.error = None;
        self.edit(folder, |files| usecase::set_translatable(files, &key, translatable), cx);
    }

    fn add_locale(&mut self, locale: Locale, cx: &mut Context<Self>) {
        self.error = None;
        self.edit(0, |files| usecase::add_locale(files, &locale), cx);
    }

    fn set_filter(&mut self, filter: KeyFilter, cx: &mut Context<Self>) {
        self.filter = filter;
        self.refilter();
        cx.notify();
    }

    /// The locale columns shown: all, or the filter's one.
    fn shown_locales(&self) -> Vec<usize> {
        match &self.locale_filter {
            Some(l) => self.table.locales.iter().position(|x| x == l).into_iter().collect(),
            None => (0..self.table.locales.len()).collect(),
        }
    }

    fn set_locale_filter(&mut self, locale: Option<Locale>, cx: &mut Context<Self>) {
        self.locale_filter = locale;
        self.h_offset = 0.;
        cx.notify();
    }

    /// Filter by Text: a dialog asking for the text.
    fn filter_by_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = cx.new(|cx| InputState::new(window, cx));
        let this = cx.entity().downgrade();
        let focus = text.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let (text_ok, this) = (text.clone(), this.clone());
            dialog.title("Filter by Text").w(px(360.)).child(Input::new(&text)).footer(dialogs::footer("OK")).on_ok(move |_, _, cx| {
                let t = text_ok.read(cx).value().trim().to_owned();
                let filter = if t.is_empty() { KeyFilter::All } else { KeyFilter::Text(t) };
                this.update(cx, |v, cx| v.set_filter(filter, cx)).ok();
                true
            })
        });
        dialogs::focus_input(&focus, window, cx);
    }

    /// Delete: in a locale column, the selected rows' translations there;
    /// in the key columns, Remove Keys.
    fn delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, column)) = self.cell.clone() else { return };
        let Column::Locale(l) = column else { return self.remove_keys(window, cx) };
        let Some(locale) = self.table.locales.get(l).cloned() else { return };
        self.error = None;
        for (folder, key) in self.selected.clone() {
            self.edit_quietly(folder, |files| usecase::set_value(files, &key, Some(&locale), ""), cx);
        }
        self.reload(cx);
        self.fill_fields(window, cx);
    }

    /// Scrolls the locale columns sideways by `dx`.
    fn scroll_locales(&mut self, dx: f32, cx: &mut Context<Self>) {
        let total = LOCALE_W * self.shown_locales().len() as f32;
        let max = (total - self.h_visible).max(0.);
        let offset = (self.h_offset + dx).clamp(0., max);
        if offset != self.h_offset {
            self.h_offset = offset;
            cx.notify();
        }
    }

    /// Remove Keys: the selected keys, from every locale, once confirmed.
    fn remove_keys(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<RowId> = self.selected.iter().cloned().collect();
        if ids.is_empty() {
            return;
        }
        let message = match ids.as_slice() {
            [(_, key)] => format!("Delete string resource \"{key}\" from all locales?"),
            _ => format!("Delete {} string resources from all locales?", ids.len()),
        };
        let this = cx.entity().downgrade();
        dialogs::confirm("Remove Keys", message, "Delete", move |cx| {
            let ids = ids.clone();
            this.update(cx, |this, cx| {
                this.cell = None;
                this.error = None;
                let folders: BTreeSet<usize> = ids.iter().map(|(f, _)| *f).collect();
                for folder in folders {
                    let keys: Vec<String> = ids.iter().filter(|(f, _)| *f == folder).map(|(_, k)| k.clone()).collect();
                    this.edit_quietly(folder, |files| usecase::remove_keys(files, &keys), cx);
                }
                this.reload(cx);
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
        let labels: Vec<String> = self.folders.iter().map(|f| f.1.clone()).collect();
        // The Resource Folder picked: the current row's, else the first.
        let folder: Entity<usize> = cx.new(|_| self.cell.as_ref().map_or(0, |((f, _), _)| *f));
        let palette = cx.palette().clone();
        let focus_key = key.clone();
        window.open_dialog(cx, move |dialog, _, cx| {
            let label = |text: &'static str| div().w(px(110.)).text_sm().child(text);
            let (key_ok, value_ok, error_ok, this, folder_ok) = (key.clone(), value.clone(), error.clone(), this.clone(), folder.clone());
            let picked = *folder.read(cx);
            let (labels_menu, folder_menu) = (labels.clone(), folder.clone());
            let folder_picker = Button::new("tr-add-key-folder").outline().small().w_full().label(labels.get(picked).cloned().unwrap_or_default()).child(Icon::new(icons::CHEVRON_DOWN).xsmall()).dropdown_menu(move |mut menu, _, _| {
                for (ix, l) in labels_menu.iter().enumerate() {
                    let folder = folder_menu.clone();
                    menu = menu.item(PopupMenuItem::new(l.clone()).checked(ix == picked).on_click(move |_, _, cx| {
                        folder.update(cx, |f, cx| {
                            *f = ix;
                            cx.notify();
                        });
                    }));
                }
                menu
            });
            dialog
                .title("Add Key")
                .w(px(460.))
                .child(
                    v_flex()
                        .gap_2()
                        .child(h_flex().gap_2().child(label("Key:")).child(div().flex_1().child(Input::new(&key))))
                        .child(h_flex().gap_2().child(label("Default Value:")).child(div().flex_1().child(Input::new(&value))))
                        .child(h_flex().gap_2().child(label("Resource Folder:")).child(div().flex_1().child(folder_picker)))
                        .when_some(error.read(cx).clone(), |el, e| el.child(div().text_sm().text_color(palette.status_conflict).child(e))),
                )
                .footer(dialogs::footer("OK"))
                .on_ok(move |_, window, cx| {
                    let (k, v) = (key_ok.read(cx).value().trim().to_owned(), value_ok.read(cx).value().to_string());
                    let Some(this) = this.upgrade() else { return true };
                    let folder = *folder_ok.read(cx);
                    let result = this.update(cx, |this, _| usecase::add_key(&this.files(folder), &k, &v).err());
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
                                this.changed(folder, &[], cx);
                                this.reload(cx);
                                this.selected = BTreeSet::from([(folder, k.clone())]);
                                this.cell = Some(((folder, k), Column::Default));
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
            let row_id: RowId = (row.folder, row.key.clone());
            let selected = self.selected.contains(&row_id);
            let current = self.cell.as_ref().filter(|(id, _)| *id == row_id).map(|(_, c)| *c);
            let cell = |id: (&'static str, usize), width: f32, column: Column, cx: &mut Context<Self>| {
                let key = row_id.clone();
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
                .child(cell(("tr-folder", ix), FOLDER_W, Column::Key, cx).text_color(palette.text_secondary).child(self.folders.get(row.folder).map(|f| f.1.clone()).unwrap_or_default()));
            let key = row_id.clone();
            line = line.child(
                div().w(px(UNTRANSLATABLE_W)).flex_none().h_full().flex().items_center().justify_center().border_r_1().border_color(palette.border).child(
                    Checkbox::new(("tr-untranslatable", ix)).checked(!row.translatable).disabled(row.default.is_none()).on_click(cx.listener(move |this, checked: &bool, _, cx| {
                        this.toggle_translatable(key.clone(), !*checked, cx);
                    })),
                ),
            );
            line = line.child(cell(("tr-default", ix), DEFAULT_W, Column::Default, cx).when(row.default.is_none(), |el| el.bg(missing_bg)).child(row.default.clone().unwrap_or_default()));
            let mut locales = h_flex().h_full().flex_none().ml(px(-self.h_offset));
            for l in self.shown_locales() {
                let Some(value) = row.values.get(l) else { continue };
                let missing = row.missing(l);
                locales = locales.child(cell(("tr-locale", ix * 1000 + l), LOCALE_W, Column::Locale(l), cx).when(missing, |el| el.bg(missing_bg)).child(value.clone().unwrap_or_default()));
            }
            line = line.child(div().flex_1().min_w_0().h_full().overflow_hidden().child(locales));
            out.push(line.into_any_element());
        }
        out
    }
}

/// The frozen columns' width: Key, Resource Folder, Untranslatable,
/// Default Value.
const FROZEN_W: f32 = KEY_W + FOLDER_W + UNTRANSLATABLE_W + DEFAULT_W;

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
        let filter = self.filter.clone();
        let this = cx.entity().downgrade();
        let locales = self.table.locales.clone();
        let filter_button = Button::new("tr-filter").ghost().small().label(filter.label()).child(Icon::new(icons::CHEVRON_DOWN).xsmall()).dropdown_menu(move |mut menu, _, _| {
            let mut filters = vec![KeyFilter::All, KeyFilter::Translatable, KeyFilter::NeedingTranslations];
            filters.extend(locales.iter().cloned().map(KeyFilter::NeedingTranslationFor));
            for f in filters {
                let this = this.clone();
                let checked = f == filter;
                menu = menu.item(PopupMenuItem::new(f.label()).checked(checked).on_click(move |_, _, cx| {
                    let f = f.clone();
                    this.update(cx, |v, cx| v.set_filter(f, cx)).ok();
                }));
            }
            let this = this.clone();
            menu.separator().item(PopupMenuItem::new("Filter by Text").checked(matches!(filter, KeyFilter::Text(_))).on_click(move |_, window, cx| {
                this.update(cx, |v, cx| v.filter_by_text(window, cx)).ok();
            }))
        });
        let locale_filter = self.locale_filter.clone();
        let this = cx.entity().downgrade();
        let locales = self.table.locales.clone();
        let locale_label = locale_filter.as_ref().map_or("Show All Locales".to_owned(), |l| format!("Show {}", l.label()));
        let locale_button = Button::new("tr-locale-filter").ghost().small().label(locale_label).child(Icon::new(icons::CHEVRON_DOWN).xsmall()).dropdown_menu(move |mut menu, _, _| {
            let all = this.clone();
            menu = menu.item(PopupMenuItem::new("Show All Locales").checked(locale_filter.is_none()).on_click(move |_, _, cx| {
                all.update(cx, |v, cx| v.set_locale_filter(None, cx)).ok();
            }));
            for l in &locales {
                let (this, locale) = (this.clone(), l.clone());
                menu = menu.item(PopupMenuItem::new(format!("Show {}", l.label())).checked(locale_filter.as_ref() == Some(l)).on_click(move |_, _, cx| {
                    let locale = locale.clone();
                    this.update(cx, |v, cx| v.set_locale_filter(Some(locale), cx)).ok();
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
            .child(locale_button)
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
        let mut locale_headers = h_flex().h_full().flex_none().ml(px(-self.h_offset));
        for l in self.shown_locales() {
            locale_headers = locale_headers.child(Self::header_cell(self.table.locales[l].label(), LOCALE_W, &palette));
        }
        header = header.child(div().flex_1().min_w_0().h_full().overflow_hidden().child(locale_headers));
        let body: AnyElement = if self.table.rows.is_empty() {
            div().p_4().text_color(palette.text_secondary).child("No string resources").into_any_element()
        } else {
            uniform_list("translations", self.shown.len(), cx.processor(Self::render_rows)).track_scroll(&self.scroll).flex_1().into_any_element()
        };
        // The frozen columns stay; the locale columns scroll sideways with
        // the wheel (Shift+wheel, or a horizontal wheel).
        let entity = cx.entity().downgrade();
        let table = v_flex()
            .id("translations-table")
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_hidden()
            .on_scroll_wheel(cx.listener(|this, e: &gpui_kit::ScrollWheelEvent, _, cx| {
                let d = e.delta.pixel_delta(px(ROW_H));
                let dx = if e.modifiers.shift && f32::from(d.x) == 0. { -f32::from(d.y) } else { -f32::from(d.x) };
                if dx != 0. {
                    this.scroll_locales(dx, cx);
                    cx.stop_propagation();
                }
            }))
            .child(header)
            .child(body)
            .child(
                gpui_kit::canvas(
                    move |bounds, _, cx| {
                        let visible = (f32::from(bounds.size.width) - FROZEN_W).max(0.);
                        entity.update(cx, |this, cx| {
                            if (this.h_visible - visible).abs() > 0.5 {
                                this.h_visible = visible;
                                this.scroll_locales(0., cx);
                            }
                        }).ok();
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            );
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
        v_flex()
            .size_full()
            .bg(palette.panel)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &gpui_kit::KeyDownEvent, window, cx| {
                if matches!(e.keystroke.key.as_str(), "delete" | "backspace") && this.cell.is_some() && this.focus.is_focused(window) {
                    cx.stop_propagation();
                    this.delete_selected(window, cx);
                }
            }))
            .text_sm()
            .child(toolbar)
            .child(table)
            .child(fields)
    }
}

/// The project's `res` folder holding a values file, for the editor's
/// Resource Folder column: `app/src/main/res`.
pub fn res_of(path: &std::path::Path) -> Option<PathBuf> {
    let folder = path.parent()?;
    let name = folder.file_name()?.to_string_lossy();
    (name == "values" || name.starts_with("values-")).then(|| folder.parent().map(PathBuf::from)).flatten()
}
