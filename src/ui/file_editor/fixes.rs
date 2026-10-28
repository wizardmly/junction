//! Alt+Enter (Show Context Actions) in the editor: the quick fixes of the
//! problems at the caret, as IntelliJ's popup lists them ("Change to
//! 1.13.1"). Enter or a click applies one; Escape closes it. The same
//! popup is Introduce Variable's "Expressions" chooser.

use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AppContext as _, Context, InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Pixels, Point, StatefulInteractiveElement as _, Styled as _, Window, actions, anchored, deferred, div,
    point, prelude::FluentBuilder as _, px,
};

use super::FileEditor;
use crate::domain::diagnostics::{Fix, Pos};
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;

actions!(context_actions, [ShowContextActions, CloseContextActions, SelectPrevContextAction, SelectNextContextAction, ApplyContextAction]);

const POPUP_CONTEXT: &str = "ContextActions";

pub fn init(cx: &mut gpui_kit::App) {
    cx.bind_keys([
        KeyBinding::new("alt-enter", ShowContextActions, Some(super::CONTEXT)),
        KeyBinding::new("escape", CloseContextActions, Some(POPUP_CONTEXT)),
        KeyBinding::new("up", SelectPrevContextAction, Some(POPUP_CONTEXT)),
        KeyBinding::new("down", SelectNextContextAction, Some(POPUP_CONTEXT)),
        KeyBinding::new("enter", ApplyContextAction, Some(POPUP_CONTEXT)),
    ]);
}

pub(super) struct ContextActions {
    /// Each fix and the byte range it replaces.
    items: Vec<(std::ops::Range<usize>, Choice)>,
    /// "Expressions" over the list.
    title: Option<&'static str>,
    selected: usize,
    at: Point<Pixels>,
    /// The selection to put back when the popup closes unused.
    caret: Option<std::ops::Range<usize>>,
}

#[derive(Clone)]
pub(super) enum Choice {
    Fix(Fix),
    /// An expression to introduce a variable or constant for (its text,
    /// one line).
    Expression(String, crate::ui::introduce_dialog::Introduce),
    /// Generate: what to generate.
    Generate(crate::domain::generate::Member),
    /// Surround With: the surrounder, numbered as in IntelliJ's popup;
    /// the range is the lines or the expression it wraps.
    Surround(crate::domain::surround::Surrounder, char, bool),
    /// Import class 'Date': the classes of that name to choose from.
    ImportClass(String, Vec<String>),
    /// One class of the "Class to Import" chooser.
    Import(String),
    /// Import function 'greet' / Import extension function
    /// 'String.shout' (Kotlin): its title and the functions to choose from.
    ImportFunction(String, Vec<String>),
    /// One function of the "Imports" chooser.
    ImportMember(String),
    /// An intention: its edit.
    Intention(crate::domain::intentions::Intention),
    /// An intention whose edit names classes to import.
    Typed(crate::domain::intentions::Intention, Vec<String>),
    /// In-place Introduce Variable's "Multiple occurrences found": this
    /// occurrence only (false) or all of them.
    InPlace(std::rc::Rc<super::in_place::InPlaceIntroduce>, bool),
    /// Generate › Override Methods… (false) / Implement Methods… (true).
    Override(bool),
    /// Generate › Test…: Create Test.
    Test,
    /// Use version catalog instead: the edit, and the catalog's file
    /// (absolute) with its new text.
    Catalog(crate::domain::catalog_migration::CatalogMove, std::path::PathBuf),
    /// A language server's code action.
    Server(crate::index::lsp::CodeAction),
    /// A server refactoring picked for a shortcut (Extract Function…);
    /// whether the name it makes up goes to Rename after.
    Refactor(crate::index::lsp::CodeAction, bool),
    /// Unwrap/Remove: one way to take a statement apart.
    Unwrap(crate::domain::intentions::Intention),
    /// A surround live template, numbered after the surrounders.
    LiveTemplate(crate::domain::live_templates::Template, char),
}

/// The byte offset of a line and character column.
fn offset_of(text: &str, pos: Pos) -> usize {
    let line_start = text.split_inclusive('\n').take(pos.line as usize).map(str::len).sum::<usize>();
    let line = &text[line_start.min(text.len())..];
    line_start + line.char_indices().nth(pos.col as usize).map_or(line.find('\n').unwrap_or(line.len()), |(i, _)| i)
}

impl FileEditor {
    pub(super) fn show_context_actions(&mut self, _: &ShowContextActions, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only() {
            return;
        }
        // The import hint's question answered: its class imported.
        if self.accept_import_hint(window, cx) {
            return;
        }
        let text = self.text(cx);
        let cursor = self.state.read(cx).cursor();
        let cursor_line = Pos::of(&text, cursor).line;
        let mut items: Vec<(std::ops::Range<usize>, Choice)> = Vec::new();
        // The problems under the caret first, then the rest of its line.
        let mut problems = self.problems();
        problems.sort_by_key(|p| {
            let (s, e) = (offset_of(&text, p.start), offset_of(&text, p.end));
            !(s <= cursor && cursor <= e)
        });
        for p in problems.iter().filter(|p| p.start.line == cursor_line) {
            if let Some(fix) = &p.fix {
                items.push((offset_of(&text, p.start)..offset_of(&text, p.end), Choice::Fix(fix.clone())));
            }
        }
        // Import class on a name the file doesn't see, then the intentions.
        if let (Some((lang, style)), Some(index)) = (self.jvm_style(), self.code_index.clone()) {
            let index = index.read(cx).index.clone();
            let found = index.read().ok().and_then(|index| {
                let model = crate::infra::index_code_model::IndexCodeModel::new(&index, lang, &self.path);
                crate::usecase::import_class::candidates(&model, &text, cursor)
            });
            if let Some((name, classes)) = found {
                items.push((cursor..cursor, Choice::ImportClass(name, classes)));
            }
            // Import function (Kotlin) or Import static method (Java).
            let found = index.read().ok().and_then(|index| match style {
                crate::domain::imports::ImportStyle::Kotlin => crate::index::import_function::candidates(&index, &self.path, &text, cursor),
                crate::domain::imports::ImportStyle::Java => crate::index::import_function::static_methods(&index, &self.path, &text, cursor),
            });
            if let Some(f) = found {
                items.push((cursor..cursor, Choice::ImportFunction(f.title, f.qualified)));
            }
            // Those Settings › Editor › Intentions turned off aren't offered.
            let kotlin = style == crate::domain::imports::ImportStyle::Kotlin;
            let settings = crate::settings::Settings::get(cx).clone();
            let on = |i: &crate::domain::intentions::Intention| settings.intention_on(kotlin, &i.title);
            for intention in crate::domain::intentions::intentions(style, &text, cursor).into_iter().filter(on) {
                items.push((intention.range.clone(), Choice::Intention(intention)));
            }
            if !kotlin {
                let classes = |n: &str| self.class_texts(n, false, cx);
                let typed = [
                    crate::index::intentions::indexed_for_loop(&text, cursor, &classes),
                    crate::index::intentions::enhanced_for_loop(&text, cursor, &classes),
                    crate::index::intentions::size_check(&text, cursor, false, &classes),
                    crate::index::intentions::string_comparison(&text, cursor, &classes),
                    crate::index::intentions::redundant_conversion(&text, cursor, false, &classes),
                ];
                for intention in typed.into_iter().flatten().filter(on) {
                    items.push((intention.range.clone(), Choice::Intention(intention)));
                }
            }
            if kotlin {
                for intention in crate::index::intentions::intentions(&text, cursor).into_iter().filter(on) {
                    items.push((intention.range.clone(), Choice::Intention(intention)));
                }
                if let Some((intention, imports)) = crate::index::intentions::specify_inferred_type(&text, cursor, &|n| self.class_texts(n, false, cx)).filter(|(i, _)| on(i)) {
                    items.push((intention.range.clone(), Choice::Typed(intention, imports)));
                }
                let classes = |n: &str| self.class_texts(n, false, cx);
                let typed = [
                    crate::index::intentions::iterate_over(&text, cursor, &classes),
                    crate::index::intentions::add_loop_indices(&text, cursor, &classes),
                    crate::index::intentions::size_check(&text, cursor, true, &classes),
                    crate::index::intentions::for_each_to_loop(&text, cursor, &classes),
                    crate::index::intentions::indexing_operator(&text, cursor, &classes),
                    crate::index::intentions::binary_operator(&text, cursor, &classes),
                    crate::index::intentions::or_empty(&text, cursor, &classes),
                    crate::index::intentions::negated_emptiness(&text, cursor, &classes),
                    crate::index::intentions::merge_call_chain(&text, cursor, &classes),
                    crate::index::intentions::redundant_conversion(&text, cursor, true, &classes),
                    crate::index::intentions::range_to_indices(&text, cursor, &classes),
                ];
                for intention in typed.into_iter().flatten().filter(on) {
                    items.push((intention.range.clone(), Choice::Intention(intention)));
                }
                if settings.intention_on(true, "Add names to call arguments") {
                    self.request_argument_names(&text, cursor, window, cx);
                }
            }
        }
        // A build script's dependency or plugin into the version catalog.
        if let Some((fix, file)) = self.catalog_move(&text, cursor, cx) {
            items.push((fix.range.clone(), Choice::Catalog(fix, file)));
        }
        // A language server's actions join the list when they arrive.
        self.request_server_actions(&text, cursor, window, cx);
        if items.is_empty() {
            return;
        }
        let at = self.state.read(cx).range_to_bounds(&(cursor..cursor)).map_or(point(px(80.), px(80.)), |b| b.bottom_left() + point(px(0.), px(4.)));
        self.context_actions = Some(ContextActions { items, title: None, selected: 0, at, caret: None });
        window.focus(&self.context_focus, cx);
        cx.notify();
    }

    /// "Use version catalog instead" at the caret of a build script, with
    /// the catalog of its build (`libs`, the nearest one up).
    fn catalog_move(&self, text: &str, cursor: usize, cx: &Context<Self>) -> Option<(crate::domain::catalog_migration::CatalogMove, std::path::PathBuf)> {
        if !crate::domain::version_catalog::is_build_script(&self.path) {
            return None;
        }
        let index = self.code_index.clone()?;
        let index = index.read(cx);
        let root = index.root()?.to_path_buf();
        let catalogs = index.catalogs.read().ok()?;
        let dir = self.path.rsplit_once('/').map_or("", |(d, _)| d);
        let catalog = catalogs
            .iter()
            .filter(|c| c.name == "libs")
            .filter_map(|c| {
                let build = c.path.strip_suffix("libs.versions.toml")?.trim_end_matches('/').strip_suffix("gradle")?.trim_end_matches('/');
                (build.is_empty() || dir == build || dir.starts_with(&format!("{build}/"))).then_some((build.len(), c))
            })
            .max_by_key(|(len, _)| *len)?
            .1;
        let file = root.join(&catalog.path);
        let catalog_text = std::fs::read_to_string(&file).ok()?;
        let fix = crate::domain::catalog_migration::move_to_catalog(text, cursor, catalog, &catalog_text)?;
        Some((fix, file))
    }

    /// Introduce Variable's "Expressions" popup at the caret: the one
    /// chosen is highlighted in the text as the list moves.
    pub(super) fn choose_expression(&mut self, kind: crate::ui::introduce_dialog::Introduce, text: &str, ranges: Vec<std::ops::Range<usize>>, window: &mut Window, cx: &mut Context<Self>) {
        let cursor = self.state.read(cx).cursor();
        let at = self.state.read(cx).range_to_bounds(&(cursor..cursor)).map_or(point(px(80.), px(80.)), |b| b.bottom_left() + point(px(0.), px(4.)));
        let items = ranges
            .into_iter()
            .map(|r| {
                let one_line = text[r.clone()].split_whitespace().collect::<Vec<_>>().join(" ");
                let label = if one_line.chars().count() > 60 { format!("{}…", one_line.chars().take(60).collect::<String>()) } else { one_line };
                (r, Choice::Expression(label, kind))
            })
            .collect();
        let caret = Some(self.state.read(cx).selected_range());
        self.context_actions = Some(ContextActions { items, title: Some("Expressions"), selected: 0, at, caret });
        window.focus(&self.context_focus, cx);
        cx.notify();
    }


    /// Surround With's popup at the caret: the statement surrounders for
    /// the lines, then (for an expression selected within a line) the
    /// expression ones, numbered; a number applies its item.
    pub(super) fn choose_surrounder(
        &mut self,
        style: crate::domain::imports::ImportStyle,
        lines: std::ops::Range<usize>,
        expression: Option<(std::ops::Range<usize>, Option<crate::domain::surround::ExpressionContext>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::domain::surround::{expression_surrounders, mnemonic, statement_surrounders};
        let mut items: Vec<(std::ops::Range<usize>, Choice)> = Vec::new();
        let (expression, context) = expression.map_or((None, None), |(r, c)| (Some(r), c));
        if let Some(expr) = &expression {
            for s in expression_surrounders(style, context.as_ref()) {
                items.push((expr.clone(), Choice::Surround(s, mnemonic(items.len()), true)));
            }
        }
        for s in statement_surrounders(style) {
            items.push((lines.clone(), Choice::Surround(*s, mnemonic(items.len()), false)));
        }
        // The surround live templates after them, as IntelliJ's popup.
        // A whole line that is one expression: the templates take the line.
        let text = self.text(cx);
        let range = expression.clone().filter(|e| text[lines.clone()].trim() != &text[e.clone()]).unwrap_or_else(|| lines.clone());
        for t in crate::domain::live_templates::surrounding(style == crate::domain::imports::ImportStyle::Kotlin) {
            let key = mnemonic(items.len());
            items.push((range.clone(), Choice::LiveTemplate(t, key)));
        }
        let cursor = self.state.read(cx).cursor();
        let at = self.state.read(cx).range_to_bounds(&(cursor..cursor)).map_or(point(px(80.), px(80.)), |b| b.bottom_left() + point(px(0.), px(4.)));
        self.context_actions = Some(ContextActions { items, title: Some("Surround With"), selected: 0, at, caret: None });
        window.focus(&self.context_focus, cx);
        cx.notify();
    }

    /// Surround with Live Template's popup: the templates, numbered.
    pub(super) fn choose_live_template(&mut self, templates: Vec<crate::domain::live_templates::Template>, range: std::ops::Range<usize>, window: &mut Window, cx: &mut Context<Self>) {
        use crate::domain::surround::mnemonic;
        let items = templates.into_iter().enumerate().map(|(i, t)| (range.clone(), Choice::LiveTemplate(t, mnemonic(i)))).collect();
        let cursor = self.state.read(cx).cursor();
        let at = self.state.read(cx).range_to_bounds(&(cursor..cursor)).map_or(point(px(80.), px(80.)), |b| b.bottom_left() + point(px(0.), px(4.)));
        self.context_actions = Some(ContextActions { items, title: Some("Surround with Live Template"), selected: 0, at, caret: None });
        window.focus(&self.context_focus, cx);
        cx.notify();
    }

    /// Code › Unwrap/Remove (Ctrl+Shift+Delete): the popup of the
    /// statements around the caret to take apart, innermost first.
    pub(super) fn unwrap_remove(&mut self, _: &super::editing::UnwrapRemove, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only() {
            return;
        }
        let Some(lang) = crate::index::lang::Lang::from_path(&self.path) else { return };
        let text = self.text(cx);
        let cursor = self.state.read(cx).cursor();
        let options = crate::index::unwrap::unwrap_options(lang, &text, cursor);
        if options.is_empty() {
            return Self::hint("Unwrap/Remove", "Nothing to unwrap here", window, cx);
        }
        let at = self.state.read(cx).range_to_bounds(&(cursor..cursor)).map_or(point(px(80.), px(80.)), |b| b.bottom_left() + point(px(0.), px(4.)));
        // Highlighted while chosen: what the choice changes.
        let items = options.into_iter().map(|o| (o.affected, Choice::Unwrap(o.edit))).collect();
        self.context_actions = Some(ContextActions { items, title: Some("Choose the statement to unwrap/remove"), selected: 0, at, caret: None });
        window.focus(&self.context_focus, cx);
        cx.notify();
    }

    /// In-place Introduce Variable's "Multiple occurrences found" popup.
    pub(super) fn choose_occurrences(&mut self, plan: std::rc::Rc<super::in_place::InPlaceIntroduce>, range: std::ops::Range<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let cursor = self.state.read(cx).cursor();
        let at = self.state.read(cx).range_to_bounds(&(cursor..cursor)).map_or(point(px(80.), px(80.)), |b| b.bottom_left() + point(px(0.), px(4.)));
        let items = vec![(range.clone(), Choice::InPlace(plan.clone(), false)), (range, Choice::InPlace(plan, true))];
        self.context_actions = Some(ContextActions { items, title: Some("Multiple occurrences found"), selected: 0, at, caret: None });
        window.focus(&self.context_focus, cx);
        cx.notify();
    }

    /// Generate's popup at the caret.
    pub(super) fn choose_generate(&mut self, items: Vec<crate::domain::generate::Member>, window: &mut Window, cx: &mut Context<Self>) {
        let cursor = self.state.read(cx).cursor();
        let at = self.state.read(cx).range_to_bounds(&(cursor..cursor)).map_or(point(px(80.), px(80.)), |b| b.bottom_left() + point(px(0.), px(4.)));
        let mut items: Vec<_> = items.into_iter().map(|m| (cursor..cursor, Choice::Generate(m))).collect();
        // IntelliJ's Generate lists these after the members.
        items.push((cursor..cursor, Choice::Override(false)));
        items.push((cursor..cursor, Choice::Override(true)));
        if !crate::domain::hotswap::is_test_source(std::path::Path::new(&format!("/{}", self.path))) {
            items.push((cursor..cursor, Choice::Test));
        }
        self.context_actions = Some(ContextActions { items, title: Some("Generate"), selected: 0, at, caret: None });
        window.focus(&self.context_focus, cx);
        cx.notify();
    }

    fn close_context_actions(&mut self, _: &CloseContextActions, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(caret) = self.context_actions.take().and_then(|p| p.caret) {
            self.state.update(cx, |state, cx| state.set_selected_range(caret, cx));
        }
        self.focus(window, cx);
        cx.notify();
    }

    fn step_context_action(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(popup) = &mut self.context_actions {
            popup.selected = (popup.selected as isize + delta).rem_euclid(popup.items.len() as isize) as usize;
            cx.notify();
        }
    }

    fn apply_context_action(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(popup) = self.context_actions.take() else { return };
        let Some((range, choice)) = popup.items.get(ix).cloned() else { return };
        self.focus(window, cx);
        match choice {
            Choice::Fix(fix) => self.state.update(cx, |state, cx| {
                state.set_selected_range(range, cx);
                state.replace(&fix.replacement, window, cx);
            }),
            Choice::Surround(surrounder, _, expression) => self.surround(surrounder, range, expression, window, cx),
            Choice::LiveTemplate(template, _) => self.surround_with_template(template, range, window, cx),
            Choice::Generate(member) => self.generate_member(member, window, cx),
            Choice::Expression(_, kind) => {
                if let Some(caret) = popup.caret {
                    self.state.update(cx, |state, cx| state.set_selected_range(caret, cx));
                }
                self.introduce_for(kind, range, window, cx);
            }
            Choice::ImportClass(_, classes) if classes.len() == 1 => self.import_class(&classes[0], window, cx),
            Choice::ImportClass(_, classes) => {
                // IntelliJ's "Class to Import" chooser, where the popup was.
                let items = classes.into_iter().map(|q| (range.clone(), Choice::Import(q))).collect();
                self.context_actions = Some(ContextActions { items, title: Some("Class to Import"), selected: 0, at: popup.at, caret: None });
                window.focus(&self.context_focus, cx);
            }
            Choice::Import(qualified) | Choice::ImportMember(qualified) => self.import_class(&qualified, window, cx),
            Choice::ImportFunction(_, functions) if functions.len() == 1 => self.import_class(&functions[0], window, cx),
            Choice::ImportFunction(_, functions) => {
                // Kotlin's "Imports" chooser (Java's "Method to Import"), where the popup was.
                let title = if functions[0].starts_with("static ") { "Method to Import" } else { "Imports" };
                let items = functions.into_iter().map(|q| (range.clone(), Choice::ImportMember(q))).collect();
                self.context_actions = Some(ContextActions { items, title: Some(title), selected: 0, at: popup.at, caret: None });
                window.focus(&self.context_focus, cx);
            }
            Choice::Override(implement) => self.override_methods(implement, window, cx),
            Choice::Test => self.create_test(window, cx),
            Choice::Server(action) => self.apply_server_action(action, false, window, cx),
            Choice::Refactor(action, rename) => self.apply_server_action(action, rename, window, cx),
            Choice::Catalog(fix, file) => {
                if let Err(e) = std::fs::write(&file, &fix.catalog_text) {
                    gpui_kit::component::WindowExt::push_notification(window, gpui_kit::component::notification::Notification::error(format!("Cannot write {}: {e}", file.display())), cx);
                    return;
                }
                self.state.update(cx, |state, cx| {
                    state.set_selected_range(fix.range.clone(), cx);
                    state.replace(&fix.replacement, window, cx);
                });
            }
            Choice::InPlace(plan, all) => self.introduce_in_place(plan, all, window, cx),
            Choice::Typed(intention, imports) => {
                let text = self.text(cx);
                let (replaced, replacement, shift) = super::with_imports(crate::domain::imports::ImportStyle::Kotlin, &text, intention.range.clone(), &intention.replacement, &imports);
                self.state.update(cx, |state, cx| {
                    state.set_selected_range(replaced, cx);
                    state.replace(&replacement, window, cx);
                    state.set_selected_range(intention.caret + shift..intention.caret + shift, cx);
                });
            }
            Choice::Intention(intention) | Choice::Unwrap(intention) => self.state.update(cx, |state, cx| {
                state.set_selected_range(intention.range.clone(), cx);
                state.replace(&intention.replacement, window, cx);
                state.set_selected_range(intention.caret..intention.caret, cx);
            }),
        }
        cx.notify();
    }

    /// Asks the file's language server for its code actions at the caret
    /// (the selection when there is one); they go after the editor's own,
    /// or open the popup when there were none.
    /// Add names to call arguments: the callee's declaration is found
    /// first (as Parameter Info finds it), so the intention joins the
    /// popup when it's known.
    fn request_argument_names(&mut self, text: &str, cursor: usize, window: &mut Window, cx: &mut Context<Self>) {
        use crate::domain::parameter_info as pi;
        let Some(index) = self.code_index.clone() else { return };
        let Some(call) = pi::call_at(text, cursor, true).filter(|c| !c.annotation && !matches!(c.name.as_str(), "this" | "super")) else { return };
        let root = index.read(cx).root().map(|r| r.to_path_buf()).unwrap_or_default();
        let task = index.read(cx).go_to(self.path.clone(), text.to_owned(), call.name_at, cx);
        let text = text.to_owned();
        self.argument_names_task = Some(cx.spawn_in(window, async move |this, cx| {
            let crate::index::service::Nav::Targets(targets) = task.await else { return };
            let found = cx
                .background_spawn(async move {
                    let mut overloads: Vec<Vec<String>> = Vec::new();
                    for t in targets {
                        let Ok(source) = std::fs::read_to_string(root.join(&t.path)) else { continue };
                        let Some(at) = super::parameter_info::offset_of(&source, t.line, t.col) else { continue };
                        for params in pi::declaration_parameters(&source, at) {
                            if !overloads.contains(&params) {
                                overloads.push(params);
                            }
                        }
                    }
                    // One declaration needs no choosing.
                    let best = if overloads.len() == 1 { 0 } else { pi::best_overload(&overloads, &pi::typed_arguments(&text, call.open, true))? };
                    crate::index::intentions::add_argument_names(&text, call.open, &overloads[best])
                })
                .await;
            let Some(intention) = found else { return };
            this.update_in(cx, |this, _, cx| {
                // Still open on the same text.
                if this.state.read(cx).cursor() != cursor {
                    return;
                }
                if let Some(popup) = this.context_actions.as_mut().filter(|p| p.title.is_none()) {
                    popup.items.push((intention.range.clone(), Choice::Intention(intention)));
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    fn request_server_actions(&mut self, text: &str, cursor: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.code_index.clone() else { return };
        let selection = self.state.read(cx).selected_range();
        let range = if selection.is_empty() { cursor..cursor } else { selection };
        let task = index.read(cx).code_actions(self.path.clone(), text.to_owned(), range, cx);
        let version = text.len();
        self.server_actions_task = Some(cx.spawn_in(window, async move |this, cx| {
            let actions = task.await;
            if actions.is_empty() {
                return;
            }
            this.update_in(cx, |this, window, cx| {
                let state = this.state.read(cx);
                // Moved on, or typed: they're for another place.
                if state.cursor() != cursor || state.value().len() != version {
                    return;
                }
                let mut actions = actions;
                // Its preferred fix first, then quick fixes, then refactorings.
                actions.sort_by_key(|a| (!a.preferred, !a.kind.as_deref().is_some_and(|k| k.starts_with("quickfix"))));
                let new: Vec<_> = actions.into_iter().map(|a| (cursor..cursor, Choice::Server(a))).collect();
                match this.context_actions.as_mut() {
                    Some(popup) if popup.title.is_none() => popup.items.extend(new),
                    Some(_) => {}
                    None => {
                        let at = this.state.read(cx).range_to_bounds(&(cursor..cursor)).map_or(point(px(80.), px(80.)), |b| b.bottom_left() + point(px(0.), px(4.)));
                        this.context_actions = Some(ContextActions { items: new, title: None, selected: 0, at, caret: None });
                        window.focus(&this.context_focus, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Extract Function, Introduce Variable / Constant and Inline in a
    /// file Junction doesn't refactor itself: the server's action of that
    /// kind at the selection (a chooser when it has several); the name an
    /// extraction makes up goes straight to Rename, as IntelliJ asks for it.
    pub(super) fn server_refactor(&mut self, refactor: crate::index::lsp::ServerRefactor, window: &mut Window, cx: &mut Context<Self>) {
        let title = refactor.title();
        let has_server = crate::index::lang::Lang::from_path(&self.path).and_then(crate::index::lsp::command_for).is_some();
        let Some(index) = self.code_index.clone().filter(|_| has_server) else {
            return Self::hint(title, "Cannot perform refactoring.\nNo language server for this file", window, cx);
        };
        let range = self.state.read(cx).selected_range();
        let task = index.read(cx).code_actions(self.path.clone(), self.text(cx), range, cx);
        let rename = refactor != crate::index::lsp::ServerRefactor::Inline;
        cx.spawn_in(window, async move |this, cx| {
            let actions: Vec<_> = task.await.into_iter().filter(|a| refactor.matches(a)).collect();
            this.update_in(cx, |this, window, cx| match actions.len() {
                0 => Self::hint(title, "Cannot perform refactoring.\nThe language server offers none here", window, cx),
                1 => this.apply_server_action(actions.into_iter().next().expect("one"), rename, window, cx),
                _ => {
                    let cursor = this.state.read(cx).cursor();
                    let at = this.state.read(cx).range_to_bounds(&(cursor..cursor)).map_or(point(px(80.), px(80.)), |b| b.bottom_left() + point(px(0.), px(4.)));
                    let items = actions.into_iter().map(|a| (cursor..cursor, Choice::Refactor(a, rename))).collect();
                    this.context_actions = Some(ContextActions { items, title: Some(title), selected: 0, at, caret: None });
                    window.focus(&this.context_focus, cx);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// A server action: its edits applied, in this file's text (one edit,
    /// the caret staying on its text) and on disk for other files; with
    /// `rename`, Rename on the name the edit brought in.
    fn apply_server_action(&mut self, action: crate::index::lsp::CodeAction, rename: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.code_index.clone() else { return };
        let Some(root) = index.read(cx).root().map(|r| r.to_path_buf()) else { return };
        let text = self.text(cx);
        let title = action.title.clone();
        let task = index.read(cx).code_action_edits(self.path.clone(), text, action, cx);
        let mut own = root.join(&self.path);
        let selection = self.state.read(cx).selected_range();
        cx.spawn_in(window, async move |this, cx| {
            let edits = task.await;
            this.update_in(cx, |this, window, cx| {
                let edits = match edits {
                    Ok(edits) => edits,
                    Err(message) => return Self::hint(&title, message, window, cx),
                };
                let rel = |path: &std::path::Path| path.strip_prefix(&root).ok().map(|r| r.to_string_lossy().replace('\\', "/"));
                let (mut written, mut moved) = (Vec::new(), Vec::new());
                for file in edits {
                    // A file created, renamed or deleted, in its turn.
                    if let Some(op) = &file.op {
                        if let Err(error) = crate::index::lsp::apply_file_op(&file.path, op) {
                            Self::hint(&title, error.to_string(), window, cx);
                            break;
                        }
                        match op {
                            crate::index::lsp::FileOp::Rename { to, .. } => {
                                if file.path == own {
                                    own = to.clone();
                                }
                                if let Some(pair) = rel(&file.path).zip(rel(to)) {
                                    moved.push(pair);
                                }
                            }
                            _ => written.extend(rel(&file.path)),
                        }
                        continue;
                    }
                    if file.path == own {
                        let text = this.text(cx);
                        let caret = this.state.read(cx).cursor();
                        let (new, caret) = apply_text_edits(&text, &file.edits, caret);
                        let named = introduced_name(&text, &file.edits, selection.clone()).filter(|_| rename);
                        this.replace_text(&text, &new, named.unwrap_or(caret), window, cx);
                        if named.is_some() {
                            this.rename(cx);
                        }
                    } else if let Ok(old) = std::fs::read_to_string(&file.path) {
                        let (new, _) = apply_text_edits(&old, &file.edits, 0);
                        if std::fs::write(&file.path, new).is_ok() {
                            written.extend(rel(&file.path));
                        }
                    }
                }
                if !moved.is_empty() {
                    cx.emit(super::FileEditorEvent::FilesMoved(moved));
                }
                if !written.is_empty() {
                    cx.emit(super::FileEditorEvent::FilesWritten(written));
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Optimize Imports by the file's language server (gopls, the
    /// TypeScript server…), as one undoable edit, the result in a hint.
    pub(super) fn server_optimize_imports(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.code_index.clone() else { return };
        let task = index.read(cx).organize_imports(self.path.clone(), self.text(cx), cx);
        cx.spawn_in(window, async move |this, cx| {
            let edits = task.await;
            this.update_in(cx, |this, window, cx| {
                let edits = match edits {
                    Ok(edits) => edits,
                    Err(message) => return Self::hint("Optimize Imports", message, window, cx),
                };
                let text = this.text(cx);
                let caret = this.state.read(cx).cursor();
                let (new, caret) = apply_text_edits(&text, &edits, caret);
                let notice = if new == text { "Unused imports not found" } else { "Imports optimized" };
                this.replace_text(&text, &new, caret, window, cx);
                Self::hint("Optimize Imports", notice.to_owned(), window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Applies the edits a picked server completion brings once resolved
    /// (an import above the completed word).
    pub(super) fn resolve_completion(&mut self, item: lsp_types::CompletionItem, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.code_index.clone() else { return };
        let task = index.read(cx).resolve_completion(self.path.clone(), &self.text(cx), item, cx);
        cx.spawn_in(window, async move |this, cx| {
            let edits = task.await;
            if edits.is_empty() {
                return;
            }
            this.update_in(cx, |this, window, cx| {
                let text = this.text(cx);
                let caret = this.state.read(cx).cursor();
                let (new, caret) = apply_text_edits(&text, &edits, caret);
                this.replace_text(&text, &new, caret, window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Replaces the text with `new` as one undoable edit (the changed span),
    /// the caret at `caret`.
    pub(super) fn replace_text(&mut self, old: &str, new: &str, caret: usize, window: &mut Window, cx: &mut Context<Self>) {
        if old == new {
            return;
        }
        let prefix = old.bytes().zip(new.bytes()).take_while(|(a, b)| a == b).count();
        let suffix = old.bytes().rev().zip(new.bytes().rev()).take_while(|(a, b)| a == b).count().min(old.len().min(new.len()) - prefix);
        let (mut start, mut old_end) = (prefix, old.len() - suffix);
        while !old.is_char_boundary(start) || !new.is_char_boundary(start) {
            start -= 1;
        }
        while !old.is_char_boundary(old_end) {
            old_end += 1;
        }
        let new_end = new.len() - (old.len() - old_end);
        let replacement = new[start..new_end].to_owned();
        let mut caret = caret.min(new.len());
        while !new.is_char_boundary(caret) {
            caret -= 1;
        }
        self.state.update(cx, |state, cx| {
            state.set_selected_range(start..old_end, cx);
            state.replace(replacement, window, cx);
            state.set_selected_range(caret..caret, cx);
        });
    }

    /// Adds `import qualified` where IntelliJ puts it; the caret stays on
    /// its text.
    /// Imports the one class, or opens IntelliJ's "Class to Import"
    /// chooser at `at` for several (the import hint's Alt+Enter).
    pub(super) fn import_one_of(&mut self, classes: Vec<String>, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        if let [one] = classes.as_slice() {
            return self.import_class(one, window, cx);
        }
        let cursor = self.state.read(cx).cursor();
        let items = classes.into_iter().map(|q| (cursor..cursor, Choice::Import(q))).collect();
        self.context_actions = Some(ContextActions { items, title: Some("Class to Import"), selected: 0, at, caret: None });
        window.focus(&self.context_focus, cx);
        cx.notify();
    }

    fn import_class(&mut self, qualified: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, style)) = self.jvm_style() else { return };
        let text = self.text(cx);
        let (at, insert) = crate::domain::imports::import_edit(style, &text, qualified);
        let cursor = self.state.read(cx).cursor();
        let cursor = if cursor >= at { cursor + insert.len() } else { cursor };
        self.state.update(cx, |state, cx| {
            state.set_selected_range(at..at, cx);
            state.replace(&insert, window, cx);
            state.set_selected_range(cursor..cursor, cx);
        });
    }

    /// The expression the "Expressions" popup has selected, highlighted
    /// in the text as IntelliJ does: a box per line it spans.
    pub(super) fn render_expression_highlight(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let popup = self.context_actions.as_ref()?;
        let (range, Choice::Expression(..) | Choice::Unwrap(_)) = popup.items.get(popup.selected)? else { return None };
        let state = self.state.read(cx);
        let text = state.value().to_string();
        let mut boxes = Vec::new();
        let mut line_start = range.start;
        while line_start < range.end {
            let line_end = text[line_start..range.end].find('\n').map_or(range.end, |i| line_start + i);
            if let (Some(a), Some(b)) = (state.range_to_bounds(&(line_start..line_start)), state.range_to_bounds(&(line_end..line_end))) {
                boxes.push((a.origin, b.left() - a.left(), a.size.height));
            }
            line_start = line_end + 1;
        }
        let color = cx.palette().selection.opacity(0.6);
        Some(
            deferred(div().children(boxes.into_iter().map(move |(origin, width, height)| {
                anchored().position(origin).child(div().w(width.max(px(2.))).h(height).bg(color).rounded_sm())
            })))
            .with_priority(1),
        )
    }

    pub(super) fn render_context_actions(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let popup = self.context_actions.as_ref()?;
        let palette = cx.palette().clone();
        let rows = popup.items.iter().enumerate().map(|(ix, (_, choice))| {
            let selected = ix == popup.selected;
            let (icon, label) = match choice {
                Choice::Fix(fix) => (Some(icons::INTENTION_BULB), fix.title.clone()),
                Choice::Expression(text, _) => (None, text.clone()),
                Choice::Surround(s, key, _) => (None, format!("{key}. {}", s.label(self.jvm_style().map_or(crate::domain::imports::ImportStyle::Java, |(_, style)| style)))),
                Choice::LiveTemplate(t, key) => (None, format!("{key}. {}  {}", t.name, t.description)),
                Choice::Generate(m) => (None, m.label().to_owned()),
                Choice::ImportClass(name, _) => (Some(icons::INTENTION_BULB), format!("Import class '{name}'")),
                Choice::Import(qualified) => (Some(icons::CLASS), qualified.clone()),
                Choice::ImportFunction(title, _) => (Some(icons::INTENTION_BULB), title.clone()),
                Choice::ImportMember(qualified) => (Some(icons::FUNCTION), qualified.trim_start_matches("static ").to_owned()),
                Choice::Intention(i) | Choice::Typed(i, _) => (Some(icons::INTENTION_BULB), i.title.clone()),
                Choice::InPlace(plan, false) if plan.occurrences > 1 => (None, "Replace this occurrence only".to_owned()),
                Choice::InPlace(plan, _) => (None, format!("Replace all {} occurrences", plan.occurrences)),
                Choice::Catalog(fix, _) => (Some(icons::INTENTION_BULB), fix.title.clone()),
                Choice::Unwrap(i) => (None, i.title.clone()),
                Choice::Server(action) => (Some(icons::INTENTION_BULB), action.title.clone()),
                Choice::Refactor(action, _) => (None, action.title.clone()),
                Choice::Override(false) => (None, "Override Methods…".to_owned()),
                Choice::Override(true) => (None, "Implement Methods…".to_owned()),
                Choice::Test => (None, "Test…".to_owned()),
            };
            h_flex()
                .id(("context-action", ix))
                .gap_1p5()
                .px_2()
                .py_0p5()
                .rounded_sm()
                .cursor_pointer()
                .when(selected, |el| el.bg(palette.selection))
                .when(!selected, |el| el.hover(|el| el.bg(palette.hover)))
                .when_some(icon, |el, icon| el.child(Icon::new(icon).xsmall()))
                .child(label)
                .on_click(cx.listener(move |this, _, window, cx| this.apply_context_action(ix, window, cx)))
        });
        Some(
            deferred(
                anchored().position(popup.at).snap_to_window_with_margin(px(8.)).child(
                    v_flex()
                        .id("context-actions")
                        .key_context(POPUP_CONTEXT)
                        .track_focus(&self.context_focus)
                        .on_action(cx.listener(Self::close_context_actions))
                        .on_action(cx.listener(|this, _: &SelectPrevContextAction, _, cx| this.step_context_action(-1, cx)))
                        .on_action(cx.listener(|this, _: &SelectNextContextAction, _, cx| this.step_context_action(1, cx)))
                        .on_action(cx.listener(|this, _: &ApplyContextAction, window, cx| {
                            let ix = this.context_actions.as_ref().map_or(0, |p| p.selected);
                            this.apply_context_action(ix, window, cx);
                        }))
                        .on_key_down(cx.listener(|this, e: &gpui_kit::KeyDownEvent, window, cx| {
                            // Surround With's numbers and letters choose an item.
                            let Some(key) = e.keystroke.key.chars().next().filter(|_| e.keystroke.key.chars().count() == 1) else { return };
                            let key = key.to_ascii_uppercase();
                            let ix = this.context_actions.as_ref().and_then(|p| p.items.iter().position(|(_, c)| matches!(c, Choice::Surround(_, k, _) | Choice::LiveTemplate(_, k) if *k == key)));
                            if let Some(ix) = ix {
                                cx.stop_propagation();
                                this.apply_context_action(ix, window, cx);
                            }
                        }))
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            if let Some(caret) = this.context_actions.take().and_then(|p| p.caret) {
                                this.state.update(cx, |state, cx| state.set_selected_range(caret, cx));
                            }
                            cx.notify();
                        }))
                        .min_w(px(220.))
                        .p_1()
                        .text_sm()
                        .bg(gpui_kit::component::ActiveTheme::theme(&**cx).popover)
                        .border_1()
                        .border_color(palette.border)
                        .rounded_md()
                        .shadow_lg()
                        .when_some(popup.title, |el, title| {
                            el.child(div().px_2().pb_1().text_xs().text_color(palette.text_secondary).child(title))
                        })
                        .children(rows)
                        .child(div().h(px(0.))),
                ),
            )
            .with_priority(2),
        )
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn offsets_from_positions() {
        let text = "ab\ncdé\nf";
        assert_eq!(super::offset_of(text, super::Pos::new(1, 2)), 5);
        assert_eq!(super::offset_of(text, super::Pos::new(1, 3)), 7);
        assert_eq!(super::offset_of(text, super::Pos::new(2, 1)), 9);
    }
}

/// Where the name an extraction made up lands in the edited text: in
/// the edit that replaced the selection (the extracted function's call,
/// the new variable's use), its first identifier the old text lacks, else
/// its first one (rust-analyzer names a function after the variable).
fn introduced_name(old: &str, edits: &[crate::index::lsp::TextEdit], selection: std::ops::Range<usize>) -> Option<usize> {
    let words = |t: &str| -> Vec<(usize, String)> {
        let mut out = Vec::new();
        let mut start = None;
        for (i, c) in t.char_indices().chain(std::iter::once((t.len(), ' '))) {
            match (start, c.is_alphanumeric() || c == '_') {
                (None, true) => start = Some(i),
                (Some(s), false) => {
                    if !t[s..i].starts_with(|c: char| c.is_numeric()) {
                        out.push((s, t[s..i].to_owned()));
                    }
                    start = None;
                }
                _ => {}
            }
        }
        out
    };
    let ranged = edit_offsets(old, edits);
    let (chosen, &(start, _, text)) = ranged.iter().enumerate().find(|(_, (start, end, text))| *start <= selection.end && *end >= selection.start && !words(text).is_empty())?;
    let known: std::collections::HashSet<String> = words(old).into_iter().map(|(_, w)| w).collect();
    let found = words(text);
    let (at, _) = found.iter().find(|(_, w)| !known.contains(w)).or(found.first())?;
    // Shifted by the edits before it.
    let shift: isize = ranged.iter().enumerate().filter(|(i, (_, end, _))| *i != chosen && *end <= start).map(|(_, (s, e, t))| t.len() as isize - (e - s) as isize).sum();
    Some((start as isize + shift) as usize + at)
}

/// Language server edits (0-based lines, UTF-16 columns) as byte ranges
/// of `text`, with their new text.
fn edit_offsets<'a>(text: &str, edits: &'a [crate::index::lsp::TextEdit]) -> Vec<(usize, usize, &'a str)> {
    let offset = |(line, col): (u32, u32)| -> usize {
        let start: usize = text.split_inclusive('\n').take(line as usize).map(str::len).sum();
        let rest = &text[start.min(text.len())..];
        let line_text = rest.split('\n').next().unwrap_or("");
        let mut units = 0u32;
        for (i, ch) in line_text.char_indices() {
            if units >= col {
                return start + i;
            }
            units += ch.len_utf16() as u32;
        }
        start + line_text.len()
    };
    edits.iter().map(|e| (offset(e.start), offset(e.end), e.text.as_str())).collect()
}

/// A language server's text edits applied to `text` (0-based lines,
/// UTF-16 columns), and where `caret` moves with them.
pub(crate) fn apply_text_edits(text: &str, edits: &[crate::index::lsp::TextEdit], caret: usize) -> (String, usize) {
    let mut ranged = edit_offsets(text, edits);
    // From the end, so earlier offsets stay valid.
    ranged.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    let mut out = text.to_owned();
    let mut caret = caret;
    for (start, end, new) in ranged {
        if start > end || end > out.len() {
            continue;
        }
        out.replace_range(start..end, new);
        if caret >= end {
            caret = caret + new.len() - (end - start);
        } else if caret > start {
            caret = start + new.len();
        }
    }
    (out, caret)
}

#[cfg(test)]
mod server_edit_tests {
    use super::*;
    use crate::index::lsp::TextEdit;

    #[test]
    fn finds_the_name_an_extraction_made_up() {
        let old = "fn main() {\n    let w = 3;\n    let area = w * w + 1;\n}\n";
        let selection = old.find("w * w").unwrap()..old.find(";\n}").unwrap();
        let edit = |start: (u32, u32), end: (u32, u32), text: &str| TextEdit { start, end, text: text.into() };
        let at = |edits: &[TextEdit]| introduced_name(old, edits, selection.clone()).map(|i| apply_text_edits(old, edits, 0).0[i..].split(|c: char| !c.is_alphanumeric() && c != '_').next().unwrap().to_owned());
        // rust-analyzer's Extract into function, named after the variable.
        let function = [edit((2, 15), (2, 24), "area(w)"), edit((3, 1), (3, 1), "\n\nfn area(w: i32) -> i32 {\n    w * w + 1\n}")];
        assert_eq!(at(&function).as_deref(), Some("area"));
        // A variable put before the statement shifts the call.
        let variable = [edit((2, 4), (2, 4), "let var_name = w * w + 1;\n    "), edit((2, 15), (2, 24), "var_name")];
        let edits = introduced_name(old, &variable, selection.clone()).unwrap();
        assert_eq!(&apply_text_edits(old, &variable, 0).0[edits..edits + 8], "var_name");
        assert_eq!(at(&[edit((0, 0), (0, 0), "// x\n")]), None);
    }

    #[test]
    fn applies_server_edits() {
        let text = "use a;\nfn f() { x }\n";
        let edits = vec![
            TextEdit { start: (0, 0), end: (0, 0), text: "use b;\n".into() },
            TextEdit { start: (1, 9), end: (1, 10), text: "y.z".into() },
        ];
        let caret = text.find('}').unwrap();
        let (out, caret) = apply_text_edits(text, &edits, caret);
        assert_eq!(out, "use b;\nuse a;\nfn f() { y.z }\n");
        assert_eq!(&out[caret..caret + 1], "}");
    }
}
