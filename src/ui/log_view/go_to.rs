//! Go to Hash / Branch / Tag (Ctrl+F) with its suggestions.

use super::*;

impl LogView {
    /// Go to Hash / Branch / Tag (`Ctrl+F` in the Log): branches and tags
    /// matching what is typed are offered below the field, as IntelliJ's
    /// completion; ↑↓ pick one, Enter goes to it (or to the typed hash).
    pub(super) fn on_go_to_hash(&mut self, _: &GoToHash, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Hash, branch or tag"));
        let refs = self.model.read(cx).log_refs().clone();
        let names: Vec<(String, RefKind)> =
            refs.local_branches().chain(refs.remote_branches()).chain(refs.tags()).map(|r| (r.name.clone(), r.kind)).collect();
        let log = cx.entity();
        let suggestions = cx.new(|cx| GoToSuggestions::new(input.clone(), names, log, cx));
        let entity = cx.entity();
        window.open_dialog(cx, {
            let input = input.clone();
            move |dialog, _, _| {
            let (input_ok, suggestions_ok) = (input.clone(), suggestions.clone());
            let (up, down) = (suggestions.clone(), suggestions.clone());
            let entity = entity.clone();
            dialog
                .title("Go to Hash/Branch/Tag")
                .w(px(420.))
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .capture_action(move |_: &gpui_kit::component::input::MoveUp, _, cx| {
                                    cx.stop_propagation();
                                    up.update(cx, |s, cx| s.step(-1, cx));
                                })
                                .capture_action(move |_: &gpui_kit::component::input::MoveDown, _, cx| {
                                    cx.stop_propagation();
                                    down.update(cx, |s, cx| s.step(1, cx));
                                })
                                .child(Input::new(&input)),
                        )
                        .child(suggestions.clone()),
                )
                .footer(
                    gpui_kit::component::dialog::DialogFooter::new()
                        .gap_2()
                        .child(gpui_kit::component::dialog::DialogClose::new().child(Button::new("goto-cancel").label("Cancel").outline()))
                        .child(gpui_kit::component::dialog::DialogAction::new().child(Button::new("goto-ok").label("Go").primary())),
                )
                .on_ok(move |_, window, cx| {
                    let text = suggestions_ok.read(cx).chosen().unwrap_or_else(|| input_ok.read(cx).value().trim().to_owned());
                    entity.update(cx, |this, cx| this.go_to(&text, window, cx));
                    true
                })
            }
        });
        dialogs::focus_input(&input, window, cx);
    }

    pub(super) fn go_to(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if text.is_empty() {
            return;
        }
        let model = self.model.read(cx);
        let needle = text.to_ascii_lowercase();
        let target = model
            .log_refs()
            .find(text)
            .map(|r| r.target.clone())
            .or_else(|| {
                model.log_refs().refs.iter().find(|r| r.name == text).map(|r| r.target.clone())
            })
            .or_else(|| model.commits().iter().find(|c| c.hash.starts_with(&needle)).map(|c| c.hash.clone()))
            .or_else(|| {
                model.repository().and_then(|repo| {
                    repo.run(["rev-parse", "--verify", "-q", &format!("{text}^{{commit}}")]).ok().map(|h| h.trim().to_owned())
                })
            });
        match target.filter(|hash| model.row_of(hash).is_some()) {
            Some(hash) => {
                self.extra_selection.clear();
                self.model.update(cx, |m, cx| m.select_hash(Some(hash), cx));
                window.focus(&self.focus, cx);
            }
            None => window.push_notification(
                gpui_kit::component::notification::Notification::warning(format!("'{text}' is not in the log")),
                cx,
            ),
        }
    }
}

/// The branches and tags offered under the Go to Hash/Branch/Tag field.
pub(super) struct GoToSuggestions {
    input: Entity<InputState>,
    names: Vec<(String, RefKind)>,
    log: Entity<LogView>,
    matches: Vec<usize>,
    selected: Option<usize>,
    _subscription: Subscription,
}

impl GoToSuggestions {
    fn new(input: Entity<InputState>, names: Vec<(String, RefKind)>, log: Entity<LogView>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&input, |this, _, cx| this.refresh(cx));
        let mut this = Self { input, names, log, matches: Vec::new(), selected: None, _subscription: subscription };
        this.refresh(cx);
        this
    }

    /// Matches for the typed text: names starting with it first; the first
    /// is preselected once something is typed.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().trim().to_lowercase();
        let mut matches: Vec<usize> = (0..self.names.len()).filter(|&ix| self.names[ix].0.to_lowercase().contains(&text)).collect();
        matches.sort_by_key(|&ix| !self.names[ix].0.to_lowercase().starts_with(&text));
        matches.truncate(8);
        if matches != self.matches {
            self.selected = (!text.is_empty() && !matches.is_empty()).then_some(0);
            self.matches = matches;
            cx.notify();
        }
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.matches.is_empty() {
            return;
        }
        let last = self.matches.len() as isize - 1;
        let next = self.selected.map_or(if delta > 0 { 0 } else { last }, |s| (s as isize + delta).clamp(0, last));
        self.selected = Some(next as usize);
        cx.notify();
    }

    fn chosen(&self) -> Option<String> {
        self.selected.and_then(|s| self.matches.get(s)).map(|&ix| self.names[ix].0.clone())
    }
}

impl Render for GoToSuggestions {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mut list = v_flex().min_h(px(row_height() * 4.));
        for (row, &ix) in self.matches.iter().enumerate() {
            let (name, kind) = self.names[ix].clone();
            let (icon, color) = match kind {
                RefKind::Tag => (IconName::Tag, palette.ref_tag),
                RefKind::RemoteBranch => (IconName::GitBranch, palette.ref_remote),
                RefKind::LocalBranch => (IconName::GitBranch, palette.ref_local),
            };
            let log = self.log.clone();
            list = list.child(
                h_flex()
                    .id(("goto-suggestion", row))
                    .h(px(row_height()))
                    .px_2()
                    .gap_1()
                    .rounded_sm()
                    .text_sm()
                    .cursor_pointer()
                    .when(self.selected == Some(row), |el| el.bg(palette.selection))
                    .when(self.selected != Some(row), |el| el.hover(|s| s.bg(palette.hover)))
                    .child(Icon::new(icon).xsmall().text_color(color))
                    .child(name.clone())
                    .on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        log.update(cx, |this, cx| this.go_to(&name, window, cx));
                    }),
            );
        }
        list
    }
}
