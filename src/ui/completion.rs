//! The editor's completion popup over the code index (IntelliJ's basic
//! completion): autopopup while typing an identifier or after `.`, and
//! Ctrl+Space. The composition root of the completion feature: it builds
//! the index code model and runs the use case off the main thread.

use std::rc::Rc;

use gpui_kit::component::input::{CompletionProvider, Rope, RopeExt as _};
use gpui_kit::{App, AppContext as _, Entity, Task, Window};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit, CompletionTriggerKind, InsertTextFormat,
    TextEdit,
};

use crate::domain::completion::CandidateKind;
use crate::index::service::CodeIndex;
use crate::infra::index_code_model::IndexCodeModel;
use crate::usecase::complete_code;

pub struct IndexCompletion {
    pub index: Entity<CodeIndex>,
    /// The edited file, as the index keys it.
    pub path: String,
}

impl IndexCompletion {
    pub fn new(index: Entity<CodeIndex>, path: String) -> Rc<Self> {
        Rc::new(Self { index, path })
    }
}

impl CompletionProvider for IndexCompletion {
    fn completions(&self, rope: &Rope, offset: usize, trigger: CompletionContext, _: &mut Window, cx: &mut App) -> Task<anyhow::Result<CompletionResponse>> {
        let text = rope.to_string();
        if let Some(items) = catalog_completions(&self.index.read(cx).catalogs.read().unwrap(), &self.path, &text, offset, rope) {
            return Task::ready(Ok(CompletionResponse::Array(items)));
        }
        let Some(lang) = CodeIndex::lang_of(&self.path, &text) else { return Task::ready(Ok(CompletionResponse::Array(Vec::new()))) };
        let explicit = trigger.trigger_kind == CompletionTriggerKind::INVOKED;
        let index = self.index.read(cx).index.clone();
        let path = self.path.clone();
        let rope = rope.clone();
        cx.background_spawn(async move {
            let found = {
                let Ok(index) = index.read() else { return Ok(CompletionResponse::Array(Vec::new())) };
                let model = IndexCodeModel::new(&index, lang, &path);
                complete_code::complete(&model, &text, offset, explicit, IndexCodeModel::import_style(lang))
            };
            let range = lsp_types::Range { start: rope.offset_to_position(found.replace.start), end: rope.offset_to_position(found.replace.end) };
            // A call is followed by `(` already: complete the name alone.
            // The popup highlights this many leading characters of each name.
            let typed = text[found.replace.clone()].to_owned();
            let call_follows = text[offset..].trim_start().starts_with('(');
            // The index's names typed this far: templates that aren't an
            // exact match go after them.
            let starting = found.items.iter().filter(|c| c.candidate.name.starts_with(&typed)).count();
            let templates = template_items(lang, &text, found.replace.start, &typed, range);
            let mut items: Vec<CompletionItem> = found
                .items
                .into_iter()
                .map(|c| {
                    let callable = c.candidate.kind.is_callable() && !call_follows;
                    let name = c.candidate.name.clone();
                    let detail = match (&c.candidate.location, &c.candidate.owner) {
                        (Some(location), _) => Some(location.clone()),
                        (None, Some(owner)) if c.candidate.kind != CandidateKind::Keyword => Some(owner.clone()),
                        _ => None,
                    };
                    CompletionItem {
                        label: if callable { format!("{name}()") } else { name.clone() },
                        filter_text: Some(typed.clone()),
                        kind: Some(kind_of(c.candidate.kind)),
                        detail,
                        insert_text_format: callable.then_some(InsertTextFormat::SNIPPET),
                        text_edit: Some(CompletionTextEdit::Edit(TextEdit { range, new_text: if callable { format!("{name}($0)") } else { name } })),
                        additional_text_edits: c.import.map(|(at, text)| {
                            let at = rope.offset_to_position(at);
                            vec![TextEdit { range: lsp_types::Range { start: at, end: at }, new_text: text }]
                        }),
                        ..Default::default()
                    }
                })
                .collect();
            for (exact, item) in templates.into_iter().rev() {
                items.insert(if exact { 0 } else { starting.min(items.len()) }, item);
            }
            Ok(CompletionResponse::Array(items))
        })
    }

    fn is_completion_trigger(&self, _offset: usize, new_text: &str, _: &mut App) -> bool {
        // IntelliJ pops up while an identifier is typed and after a dot.
        let mut chars = new_text.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => c.is_alphanumeric() || c == '_' || c == '.' || c == '$',
            _ => false,
        }
    }
}

/// Live templates whose abbreviation starts with the typed word, where
/// they apply, as IntelliJ lists them: the abbreviation with the
/// description at the right; picking one expands it (the editor runs the
/// item's command). Whether each matches the word exactly comes with it.
fn template_items(lang: crate::index::lang::Lang, text: &str, start: usize, typed: &str, range: lsp_types::Range) -> Vec<(bool, CompletionItem)> {
    use crate::domain::live_templates as lt;
    let Some(style) = IndexCodeModel::import_style(lang) else { return Vec::new() };
    let kotlin = style == crate::domain::imports::ImportStyle::Kotlin;
    if !lt::any_starts_with(kotlin, typed) {
        return Vec::new();
    }
    let contexts = crate::index::introduce::template_contexts(lang, text, start);
    lt::completing(kotlin, typed, &contexts)
        .into_iter()
        .map(|t| {
            let item = CompletionItem {
                label: t.name.to_string(),
                filter_text: Some(typed.to_owned()),
                kind: Some(CompletionItemKind::SNIPPET),
                detail: Some(t.description.to_string()),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit { range, new_text: t.name.to_string() })),
                command: Some(lsp_types::Command { title: t.description.to_string(), command: lt::EXPAND_COMMAND.to_owned(), arguments: None }),
                ..Default::default()
            };
            (t.name == typed, item)
        })
        .collect()
}

fn kind_of(kind: CandidateKind) -> CompletionItemKind {
    match kind {
        CandidateKind::Keyword => CompletionItemKind::KEYWORD,
        CandidateKind::Local | CandidateKind::Parameter => CompletionItemKind::VARIABLE,
        CandidateKind::Class => CompletionItemKind::CLASS,
        CandidateKind::Interface => CompletionItemKind::INTERFACE,
        CandidateKind::Enum => CompletionItemKind::ENUM,
        CandidateKind::EnumMember => CompletionItemKind::ENUM_MEMBER,
        CandidateKind::TypeAlias => CompletionItemKind::TYPE_PARAMETER,
        CandidateKind::Function => CompletionItemKind::FUNCTION,
        CandidateKind::Method => CompletionItemKind::METHOD,
        CandidateKind::Constructor => CompletionItemKind::CONSTRUCTOR,
        CandidateKind::Field => CompletionItemKind::FIELD,
        CandidateKind::Property => CompletionItemKind::PROPERTY,
        CandidateKind::Constant => CompletionItemKind::CONSTANT,
        CandidateKind::Module => CompletionItemKind::MODULE,
        CandidateKind::Macro => CompletionItemKind::FUNCTION,
    }
}

/// In a build script, after `libs.`: the version catalog's next accessor
/// segments, as Gradle's generated accessors offer them.
fn catalog_completions(catalogs: &[crate::domain::version_catalog::Catalog], path: &str, text: &str, offset: usize, rope: &Rope) -> Option<Vec<CompletionItem>> {
    use crate::domain::version_catalog as vc;
    if !vc::is_build_script(path) {
        return None;
    }
    let (name, typed) = vc::completion_prefix(text, offset, catalogs.iter().map(|c| c.name.as_str()))?;
    let catalog = catalogs.iter().find(|c| c.name == name)?;
    let (done, word) = typed.rsplit_once('.').map_or(("", typed.as_str()), |(d, w)| (d, w));
    let done = if done.is_empty() { String::new() } else { format!("{done}.") };
    let start = offset - word.len();
    let range = lsp_types::Range { start: rope.offset_to_position(start), end: rope.offset_to_position(offset) };
    let mut segments: Vec<(i32, String, Option<&vc::Entry>)> = catalog
        .next_segments(&done)
        .into_iter()
        .filter_map(|(segment, entry)| Some((crate::domain::completion::match_score(word, &segment)?, segment, entry)))
        .collect();
    segments.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let items = segments
        .into_iter()
        .map(|(_, segment, entry)| CompletionItem {
            label: segment.clone(),
            filter_text: Some(word.to_owned()),
            kind: Some(if entry.is_some() { CompletionItemKind::PROPERTY } else { CompletionItemKind::MODULE }),
            detail: entry.map(|e| catalog.describe(e)).filter(|d| !d.is_empty() && !d.contains('\n')),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit { range, new_text: segment })),
            ..Default::default()
        })
        .collect();
    Some(items)
}
