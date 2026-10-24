//! Code › Generate (Alt+Insert) in the editor, as Android Studio's: the
//! popup of what the class can have generated, the member chooser, and
//! the members written after the one the caret is in (an existing
//! `toString()` / `equals()` replaced once confirmed), with any imports
//! they need, as one undoable edit. `equals()` / `hashCode()` call
//! `super`'s when a superclass in the project or its libraries overrides
//! them.

use std::ops::Range;

use gpui_kit::{Context, Window};

use super::FileEditor;
use crate::domain::generate::{generate, generate_with, members, Field, Member, Supers};
use crate::index::lang::Lang;
use crate::domain::imports::{import_edit, ImportStyle};
use crate::index::generate::{class_at, insertion};

impl FileEditor {
    /// Alt+Insert: the Generate popup for the class at the caret.
    pub fn generate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((lang, style)) = self.jvm_style().filter(|_| !self.read_only()) else { return };
        let text = self.text(cx);
        let caret = self.state.read(cx).cursor();
        let info = match class_at(lang, &text, caret, &crate::domain::surround::indent_unit(&text)) {
            Ok(info) => info,
            Err(message) => return Self::hint("Generate", message, window, cx),
        };
        let items: Vec<Member> = members(style)
            .iter()
            .copied()
            .filter(|m| !info.is_object || *m == Member::ToString)
            .filter(|m| *m != Member::SecondaryConstructor || !info.primary_constructor)
            .collect();
        self.choose_generate(items, window, cx);
    }

    /// One of the popup's items: its fields chosen, then generated.
    pub(super) fn generate_member(&mut self, member: Member, window: &mut Window, cx: &mut Context<Self>) {
        let Some((lang, style)) = self.jvm_style() else { return };
        let text = self.text(cx);
        let caret = self.state.read(cx).cursor();
        let Ok(info) = class_at(lang, &text, caret, &crate::domain::surround::indent_unit(&text)) else { return };
        let has = |name: &str| info.methods.iter().any(|m| m == name);
        let property = |f: &Field| {
            let mut chars = f.name.chars();
            chars.next().map(|c| c.to_uppercase().chain(chars).collect::<String>()).unwrap_or_default()
        };
        let missing_getter = |f: &Field| !has(&format!("get{}", property(f))) && !has(&format!("is{}", property(f)));
        let missing_setter = |f: &Field| !f.is_final && !has(&format!("set{}", property(f)));
        let candidates: Vec<Field> = info
            .fields
            .iter()
            .filter(|f| match (style, member) {
                // IntelliJ's: the `var`s and the properties that must be
                // initialized.
                (ImportStyle::Kotlin, Member::SecondaryConstructor) => !f.ty.is_empty() && (!f.is_final || info.uninitialized.contains(&f.name)),
                (ImportStyle::Kotlin, _) => true,
                (_, Member::Getter) => missing_getter(f),
                (_, Member::Setter) => missing_setter(f),
                (_, Member::GetterAndSetter) => missing_getter(f) || missing_setter(f),
                _ => !f.is_static,
            })
            .cloned()
            .collect();
        let none = match member {
            Member::Getter => Some("No fields without getter were found"),
            Member::Setter => Some("No fields without setter were found"),
            Member::GetterAndSetter => Some("No fields without getter and setter were found"),
            _ => None,
        };
        if let Some(message) = none.filter(|_| candidates.is_empty()) {
            return Self::hint("Generate", message, window, cx);
        }
        // What already exists and would be replaced.
        let replaced: Vec<String> = match member {
            Member::ToString => vec!["toString"],
            Member::EqualsHashCode => vec!["equals", "hashCode"],
            _ => vec![],
        }
        .into_iter()
        .filter(|n| has(n))
        .map(str::to_owned)
        .collect();
        let class = info.name.clone();
        let editor = cx.entity().downgrade();
        let window_handle = window.window_handle();
        let title = member.chooser_title(style);
        let run = move |chosen: Vec<Field>, window: &mut Window, cx: &mut gpui_kit::App| {
            let editor = editor.clone();
            let apply = move |replace: Vec<String>, window: &mut Window, cx: &mut gpui_kit::App| {
                editor.update(cx, |this, cx| this.apply_generated(member, chosen.clone(), replace, window, cx)).ok();
            };
            if replaced.is_empty() {
                return apply(Vec::new(), window, cx);
            }
            let message = match style {
                ImportStyle::Kotlin => format!("Functions {} are already defined for class {class}. Do you want to delete them and proceed?", replaced.join(", ")),
                ImportStyle::Java if member == Member::EqualsHashCode => {
                    format!("Methods 'boolean equals(Object)' and 'int hashCode()' are already defined\nfor class {class}. Do you want to delete them and proceed?")
                }
                ImportStyle::Java => format!("Method 'String toString()' is already defined\nfor class {class}. Do you want to delete it and proceed?"),
            };
            let replace = replaced.clone();
            crate::ui::dialogs::confirm(member.label(), message, "Yes", move |cx| {
                let (apply, replace) = (apply.clone(), replace.clone());
                // The question's OK runs inside its window's update: after it.
                cx.defer(move |cx| {
                    window_handle.update(cx, |_, window, cx| apply(replace, window, cx)).ok();
                });
            }, window, cx);
        };
        if member == Member::SecondaryConstructor && candidates.is_empty() {
            return self.apply_generated(member, Vec::new(), Vec::new(), window, cx);
        }
        crate::ui::generate_dialog::choose(title, candidates, run, window, cx);
    }

    /// Whether a superclass (not `Any` / `Object`) of the class with
    /// `header` overrides `equals()` / `hashCode()`.
    fn supers(&self, lang: Lang, header: &str, cx: &mut Context<Self>) -> Supers {
        let Some(index) = self.code_index.clone() else { return Supers::default() };
        let root = index.read(cx).root().map(|r| r.to_path_buf()).unwrap_or_default();
        let index = index.read(cx).index.clone();
        let members = index.read().map(|index| crate::index::overrides::super_members(&index, &root, lang, &self.path, header)).unwrap_or_default();
        let overrides = |name: &str| members.iter().any(|(owner, m)| m.name == name && !m.is_abstract && owner != "Any" && owner != "Object");
        Supers { equals: overrides("equals"), hash_code: overrides("hashCode") }
    }

    /// Writes the chosen members into the class, replacing `replace`.
    fn apply_generated(&mut self, member: Member, fields: Vec<Field>, replace: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some((lang, style)) = self.jvm_style() else { return };
        let old = self.text(cx);
        let unit = crate::domain::surround::indent_unit(&old);
        let mut caret = self.state.read(cx).cursor();
        let mut text = old.clone();
        if !replace.is_empty() {
            let Ok(info) = class_at(lang, &text, caret, &unit) else { return };
            let mut ranges: Vec<Range<usize>> = info.method_ranges.iter().filter(|(n, _)| replace.contains(n)).map(|(_, r)| whole_lines(&text, r.clone())).collect();
            ranges.sort_by_key(|r| std::cmp::Reverse(r.start));
            for r in ranges {
                if caret >= r.end {
                    caret -= r.len();
                } else if caret > r.start {
                    caret = r.start;
                }
                text.replace_range(r, "");
            }
        }
        let Ok(info) = class_at(lang, &text, caret.min(text.len()), &unit) else { return };
        // Java: a getter and a setter only where one is missing.
        let generated = if style == ImportStyle::Java && member == Member::GetterAndSetter {
            let has = |n: &str| info.methods.iter().any(|m| m == n);
            let mut out = Vec::new();
            for f in &fields {
                let both = generate(style, Member::GetterAndSetter, &info.name, info.type_params, std::slice::from_ref(f), &unit).members;
                let cap: String = f.name.chars().take(1).flat_map(char::to_uppercase).chain(f.name.chars().skip(1)).collect();
                if !has(&format!("get{cap}")) && !has(&format!("is{cap}")) {
                    out.push(both[0].clone());
                }
                if !f.is_final && !has(&format!("set{cap}")) {
                    out.push(both[1].clone());
                }
            }
            crate::domain::generate::Generated { members: out, imports: Vec::new() }
        } else if member == Member::EqualsHashCode {
            let supers = self.supers(lang, &info.header, cx);
            generate_with(style, member, &info.name, info.type_params, &fields, supers, &unit)
        } else {
            generate(style, member, &info.name, info.type_params, &fields, &unit)
        };
        let (range, insert) = insertion(&info, &generated.members);
        text.replace_range(range.clone(), &insert);
        let mut members_at = range.start + insert.len() - insert.trim_start_matches('\n').len();
        for import in generated.imports {
            let line = format!("import {import};");
            if text.lines().any(|l| l.trim() == line) || text.lines().any(|l| l.trim() == format!("import {}.*;", import.rsplit_once('.').map_or("", |p| p.0))) {
                continue;
            }
            let (at, add) = import_edit(style, &text, import);
            if at <= members_at {
                members_at += add.len();
            }
            text.insert_str(at, &add);
        }
        // One edit: the changed span between the old and new text.
        let prefix = old.bytes().zip(text.bytes()).take_while(|(a, b)| a == b).count();
        let max_suffix = old.len().min(text.len()) - prefix;
        let suffix = old.bytes().rev().zip(text.bytes().rev()).take_while(|(a, b)| a == b).count().min(max_suffix);
        let (mut start, mut old_end) = (prefix, old.len() - suffix);
        while !old.is_char_boundary(start) || !text.is_char_boundary(start) {
            start -= 1;
        }
        while !old.is_char_boundary(old_end) {
            old_end += 1;
        }
        let new_end = text.len() - (old.len() - old_end);
        let replacement = text[start..new_end].to_owned();
        let members_at = members_at + text[members_at..].len() - text[members_at..].trim_start().len();
        self.state.update(cx, |state, cx| {
            state.set_selected_range(start..old_end, cx);
            state.replace(replacement, window, cx);
            state.set_selected_range(members_at..members_at, cx);
        });
        cx.notify();
    }
}

/// A member's range widened to whole lines, and the blank line before it.
fn whole_lines(text: &str, r: Range<usize>) -> Range<usize> {
    let mut start = text[..r.start].rfind('\n').map_or(0, |i| i + 1);
    let end = text[r.end..].find('\n').map_or(text.len(), |i| r.end + i + 1);
    if start > 0 {
        let previous = text[..start - 1].rfind('\n').map_or(0, |i| i + 1);
        if text[previous..start].trim().is_empty() {
            start = previous;
        }
    }
    start..end
}
