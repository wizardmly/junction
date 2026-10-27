//! Code › Override Methods (Ctrl+O) and Implement Methods (Ctrl+I), as
//! Android Studio's: the members of the class's supertypes it can
//! override (or must implement) and doesn't yet, in IntelliJ's chooser
//! (each with its class; Implement starts with all checked), written in
//! IntelliJ's templates after the member at the caret, as one edit. A
//! member counts as present when the class declares one with the same
//! name and parameter types (an overload doesn't); Java's chooser has
//! Copy JavaDoc and Insert @Override, and a Kotlin class with several
//! direct supertypes calls an interface's member through
//! `super<Interface>` (Java a default method through `Interface.super`).
//! The chooser groups the members by class; the
//! classes the written signatures name are imported in the same edit.

use gpui_kit::{Context, KeyBinding, Window, actions};

use super::FileEditor;
use crate::domain::generate::Field;
use crate::domain::imports::ImportStyle;
use crate::domain::overrides::{self as ov, Member};
use crate::index::generate::{class_at, insertion};

actions!(overrides, [OverrideMethods, ImplementMethods]);

/// ^O / ^I on macOS too, as IntelliJ's keymaps.
pub fn init(cx: &mut gpui_kit::App) {
    cx.bind_keys([KeyBinding::new("ctrl-o", OverrideMethods, Some(super::CONTEXT)), KeyBinding::new("ctrl-i", ImplementMethods, Some(super::CONTEXT))]);
}

impl FileEditor {
    /// Ctrl+O (`implement` false) / Ctrl+I.
    pub fn override_methods(&mut self, implement: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((lang, style)) = self.jvm_style().filter(|_| !self.read_only()) else { return };
        let kotlin = style == ImportStyle::Kotlin;
        let title = match (implement, kotlin) {
            (true, true) => "Implement Members",
            (false, true) => "Override Members",
            (true, false) => "Select Methods to Implement",
            (false, false) => "Select Methods to Override",
        };
        let text = self.text(cx);
        let caret = self.state.read(cx).cursor();
        let unit = crate::domain::surround::indent_unit(&text);
        let info = match class_at(lang, &text, caret, &unit) {
            Ok(info) => info,
            Err(message) => return Self::hint(title, message, window, cx),
        };
        let Some(index) = self.code_index.clone() else { return };
        let root = index.read(cx).root().map(|r| r.to_path_buf()).unwrap_or_default();
        let index = index.read(cx).index.clone();
        let members: Vec<(String, Member)> = index.read().map(|index| crate::index::overrides::super_members(&index, &root, lang, &self.path, &info.header)).unwrap_or_default();
        // The class's own methods by signature, its properties by name.
        let declared: Vec<(String, Vec<String>, bool)> = info.method_ranges.iter().filter_map(|(_, r)| ov::declared_signature(&text[r.clone()])).collect();
        let has = |m: &Member| match m.kind {
            ov::MemberKind::Fun => declared.iter().any(|d| ov::overrides(d, m, kotlin)),
            _ => info.methods.contains(&m.name) || info.fields.iter().any(|f| f.name == m.name),
        };
        // `super<X>` for a direct supertype's member when there are several.
        let direct: Vec<String> = ov::supertypes(&info.header, kotlin).into_iter().map(|s| s.name).collect();
        let mut candidates: Vec<(String, Member)> = members.into_iter().filter(|(_, m)| m.is_abstract == implement && !has(m)).collect();
        // Grouped by class, nearest first.
        let mut owners: Vec<String> = Vec::new();
        for (owner, _) in &candidates {
            if !owners.contains(owner) {
                owners.push(owner.clone());
            }
        }
        candidates.sort_by_key(|(owner, _)| owners.iter().position(|o| o == owner));
        if candidates.is_empty() {
            let what = if kotlin { "members" } else { "methods" };
            let message = format!("No {what} to {} have been found", if implement { "implement" } else { "override" });
            return Self::hint(title, message, window, cx);
        }
        // The chooser lists each member with its class.
        let fields: Vec<Field> = candidates.iter().map(|(owner, m)| Field { name: ov::label(m, kotlin), ty: owner.clone(), is_static: false, is_final: false }).collect();
        let editor = cx.entity().downgrade();
        let run = move |chosen: Vec<Field>, options: Vec<bool>, window: &mut Window, cx: &mut gpui_kit::App| {
            let copy_doc = options.first().copied().unwrap_or(false);
            let insert_override = options.get(1).copied().unwrap_or(true);
            let members: Vec<(Member, ov::Options)> = chosen
                .iter()
                .filter_map(|f| candidates.iter().find(|(o, m)| *o == f.ty && ov::label(m, kotlin) == f.name))
                .map(|(owner, m)| {
                    let qualifier = (kotlin && direct.len() > 1 && direct.contains(owner)).then(|| owner.clone());
                    (m.clone(), ov::Options { insert_override, copy_doc, qualifier })
                })
                .collect();
            editor.update(cx, |this, cx| this.insert_overrides(&members, window, cx)).ok();
        };
        // Copy JavaDoc off and (Java's) Insert @Override on, as IntelliJ's defaults.
        let mut options = vec![("Copy JavaDoc", false)];
        if !kotlin {
            options.push(("Insert @Override", true));
        }
        crate::ui::generate_dialog::choose_grouped(title, fields, implement, options, run, window, cx);
    }

    /// Writes the members after the one at the caret, as one edit.
    fn insert_overrides(&mut self, members: &[(Member, ov::Options)], window: &mut Window, cx: &mut Context<Self>) {
        let Some((lang, style)) = self.jvm_style() else { return };
        if members.is_empty() {
            return;
        }
        let kotlin = style == ImportStyle::Kotlin;
        let text = self.text(cx);
        let unit = crate::domain::surround::indent_unit(&text);
        let caret = self.state.read(cx).cursor();
        let Ok(info) = class_at(lang, &text, caret, &unit) else { return };
        let generated: Vec<String> = members.iter().map(|(m, options)| ov::generate_with(m, kotlin, &unit, options)).collect();
        let (range, insert) = insertion(&info, &generated);
        let mut new_text = text.clone();
        new_text.replace_range(range.clone(), &insert);
        // The caret at the first member.
        let mut at = range.start + insert.len() - insert.trim_start().len();
        // The classes the signatures name that the file doesn't see yet.
        let implicit = crate::domain::imports::implicit_packages(style);
        for qualified in members.iter().flat_map(|(m, _)| m.imports.iter()) {
            let header = crate::domain::imports::header(&new_text);
            let imported: Vec<String> = header.imports.iter().map(|(_, q)| q.clone()).chain(header.wildcards.iter().map(|w| format!("{w}.*"))).collect();
            if crate::domain::imports::is_visible(qualified, Some(&header.package), &imported, &implicit) {
                continue;
            }
            let (offset, line) = crate::domain::imports::import_edit(style, &new_text, qualified);
            new_text.insert_str(offset, &line);
            if offset <= at {
                at += line.len();
            }
        }
        // One edit from the first change to the last.
        let mut prefix = text.bytes().zip(new_text.bytes()).take_while(|(a, b)| a == b).count().min(range.start);
        let mut suffix = text.bytes().rev().zip(new_text.bytes().rev()).take_while(|(a, b)| a == b).count().min(text.len() - range.end);
        while !text.is_char_boundary(prefix) {
            prefix -= 1;
        }
        while !text.is_char_boundary(text.len() - suffix) || !new_text.is_char_boundary(new_text.len() - suffix) {
            suffix -= 1;
        }
        let replaced = prefix..text.len() - suffix;
        let replacement = new_text[prefix..new_text.len() - suffix].to_owned();
        self.state.update(cx, |state, cx| {
            state.set_selected_range(replaced, cx);
            state.replace(replacement, window, cx);
            state.set_selected_range(at..at, cx);
        });
        cx.notify();
    }
}
