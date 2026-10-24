//! Code › Override Methods (Ctrl+O) and Implement Methods (Ctrl+I), as
//! Android Studio's: the members of the class's supertypes it can
//! override (or must implement) and doesn't yet, in IntelliJ's chooser
//! (each with its class; Implement starts with all checked), written in
//! IntelliJ's templates after the member at the caret, as one edit. A
//! member counts as present when the class declares one with the same
//! name and parameter types (an overload doesn't); Java's chooser has
//! Insert @Override, and a Kotlin class with several direct supertypes
//! calls an interface's member through `super<Interface>`.

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
        let candidates: Vec<(String, Member)> = members.into_iter().filter(|(_, m)| m.is_abstract == implement && !has(m)).collect();
        if candidates.is_empty() {
            let what = if kotlin { "members" } else { "methods" };
            let message = format!("No {what} to {} have been found", if implement { "implement" } else { "override" });
            return Self::hint(title, message, window, cx);
        }
        // The chooser lists each member with its class.
        let fields: Vec<Field> = candidates.iter().map(|(owner, m)| Field { name: ov::label(m, kotlin), ty: owner.clone(), is_static: false, is_final: false }).collect();
        let editor = cx.entity().downgrade();
        let run = move |chosen: Vec<Field>, options: Vec<bool>, window: &mut Window, cx: &mut gpui_kit::App| {
            let insert_override = options.first().copied().unwrap_or(true);
            let members: Vec<(Member, ov::Options)> = chosen
                .iter()
                .filter_map(|f| candidates.iter().find(|(o, m)| *o == f.ty && ov::label(m, kotlin) == f.name))
                .map(|(owner, m)| {
                    let qualifier = (kotlin && direct.len() > 1 && direct.contains(owner)).then(|| owner.clone());
                    (m.clone(), ov::Options { insert_override, qualifier })
                })
                .collect();
            editor.update(cx, |this, cx| this.insert_overrides(&members, window, cx)).ok();
        };
        // Java's "Insert @Override", on as IntelliJ's default.
        let options = if kotlin { Vec::new() } else { vec![("Insert @Override", true)] };
        crate::ui::generate_dialog::choose_with_options(title, fields, implement, options, run, window, cx);
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
        // The caret at the first member.
        let at = range.start + insert.len() - insert.trim_start().len();
        self.state.update(cx, |state, cx| {
            state.set_selected_range(range.clone(), cx);
            state.replace(insert.clone(), window, cx);
            state.set_selected_range(at..at, cx);
        });
        cx.notify();
    }
}
