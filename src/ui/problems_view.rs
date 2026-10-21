//! The Problems tool window, as IntelliJ's: the File tab lists the open
//! editor's problems (syntax errors and the compiler's), Project Errors
//! every file the last builds and compiles reported, Inspection the
//! results of Analyze › Inspect Code… on its scope. Options: show
//! warnings, sort by severity; Previous / Next Problem, Expand / Collapse
//! All, Autoscroll to Source.

use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::{
    Selectable as _, Sizable as _, h_flex,
    button::ButtonVariants as _,
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription,
    Window, div, prelude::FluentBuilder as _, px,
};

use crate::domain::diagnostics::{Problem, Severity};
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::common::tool_button;
use crate::ui::compile_service::{CompileEvent, CompileService};
use crate::ui::file_editor::FileEditor;
use crate::ui::item_tree::{self, ItemTree, ItemTreeEvent, TreeFile, TreeItem};

pub enum ProblemsEvent {
    OpenFile { path: PathBuf, line: u32, col: u32 },
    Hide,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    File,
    Project,
    Inspection,
}

pub struct ProblemsView {
    project: Rc<dyn Fn(&App) -> Option<PathBuf>>,
    editor: Option<Entity<FileEditor>>,
    tab: Tab,
    file_tree: Entity<ItemTree>,
    project_tree: Entity<ItemTree>,
    inspection_tree: Entity<ItemTree>,
    /// Inspect Code's scope ("Project 'psd'") and findings by file.
    inspection: Option<(String, Vec<(PathBuf, Vec<Problem>)>)>,
    show_warnings: bool,
    by_severity: bool,
    _editor_subscription: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ProblemsEvent> for ProblemsView {}

impl ProblemsView {
    pub fn new(project: Rc<dyn Fn(&App) -> Option<PathBuf>>, cx: &mut Context<Self>) -> Self {
        let file_tree = cx.new(|cx| ItemTree::new(("problem", "problems"), cx));
        let project_tree = cx.new(|cx| ItemTree::new(("problem", "problems"), cx));
        let inspection_tree = cx.new(|cx| ItemTree::new(("problem", "problems"), cx));
        let compile = CompileService::global(cx);
        let forward = |_: &mut Self, _: Entity<ItemTree>, event: &ItemTreeEvent, cx: &mut Context<Self>| match event {
            ItemTreeEvent::Open { path, line, col } => cx.emit(ProblemsEvent::OpenFile { path: path.clone(), line: *line, col: *col }),
        };
        let subscriptions = vec![
            cx.subscribe(&file_tree, forward),
            cx.subscribe(&project_tree, forward),
            cx.subscribe(&inspection_tree, forward),
            cx.subscribe(&compile, |this, _, event: &CompileEvent, cx| {
                if matches!(event, CompileEvent::ProblemsChanged) {
                    this.refresh_project(cx);
                }
            }),
        ];
        let mut this = Self { project, editor: None, tab: Tab::File, file_tree, project_tree, inspection_tree, inspection: None, show_warnings: true, by_severity: false, _editor_subscription: None, _subscriptions: subscriptions };
        this.refresh_project(cx);
        this
    }

    /// Follows the active editor (None while no file is in front).
    pub fn set_editor(&mut self, editor: Option<Entity<FileEditor>>, cx: &mut Context<Self>) {
        if self.editor.as_ref().map(|e| e.entity_id()) == editor.as_ref().map(|e| e.entity_id()) {
            return;
        }
        self._editor_subscription = editor.as_ref().map(|e| cx.observe(e, |this, _, cx| this.refresh_file(cx)));
        self.editor = editor;
        self.refresh_file(cx);
    }

    /// The problem count of the open file (the File tab's title).
    fn file_count(&self, cx: &App) -> usize {
        self.file_tree.read(cx).item_count()
    }

    fn items(&self, problems: &[Problem]) -> Vec<TreeItem> {
        let mut problems: Vec<&Problem> = problems.iter().filter(|p| self.show_warnings || p.severity == Severity::Error).collect();
        if self.by_severity {
            problems.sort_by_key(|p| (p.severity, p.start));
        }
        problems
            .into_iter()
            .map(|p| TreeItem {
                icon: Some(match p.severity {
                    Severity::Error => icons::STATUS_ERROR,
                    Severity::Warning => icons::STATUS_WARNING,
                }),
                prefix: None,
                text: p.message.lines().next().unwrap_or_default().to_owned().into(),
                highlight: None,
                line: p.start.line,
                col: p.start.col,
            })
            .collect()
    }

    fn tree_file(&self, path: PathBuf, problems: &[Problem], cx: &App) -> Option<TreeFile> {
        let items = self.items(problems);
        if items.is_empty() {
            return None;
        }
        let root = (self.project)(cx);
        let (name, dir) = item_tree::name_and_dir(&path, root.as_deref());
        Some(TreeFile { path, name, dir, items })
    }

    fn refresh_file(&mut self, cx: &mut Context<Self>) {
        let files = match &self.editor {
            Some(editor) => {
                let editor = editor.read(cx);
                self.tree_file(editor.absolute_path(), &editor.problems(), cx).into_iter().collect()
            }
            None => Vec::new(),
        };
        self.file_tree.update(cx, |t, cx| t.set_files(files, cx));
        cx.notify();
    }

    fn refresh_project(&mut self, cx: &mut Context<Self>) {
        let all = CompileService::global(cx).read(cx).all_problems();
        let files: Vec<TreeFile> = all.into_iter().filter_map(|(path, problems)| self.tree_file(path, &problems, cx)).collect();
        self.project_tree.update(cx, |t, cx| t.set_files(files, cx));
        cx.notify();
    }

    /// Inspect Code's results: shown in their own tab, which comes to front.
    pub fn show_inspection(&mut self, scope: String, findings: Vec<(PathBuf, Vec<Problem>)>, cx: &mut Context<Self>) {
        self.inspection = Some((scope, findings));
        self.tab = Tab::Inspection;
        self.refresh_inspection(cx);
    }

    fn refresh_inspection(&mut self, cx: &mut Context<Self>) {
        let findings = self.inspection.as_ref().map(|(_, f)| f.clone()).unwrap_or_default();
        let files: Vec<TreeFile> = findings.into_iter().filter_map(|(path, problems)| self.tree_file(path, &problems, cx)).collect();
        self.inspection_tree.update(cx, |t, cx| t.set_files(files, cx));
        cx.notify();
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.refresh_file(cx);
        self.refresh_project(cx);
        self.refresh_inspection(cx);
    }

    fn tree(&self) -> &Entity<ItemTree> {
        match self.tab {
            Tab::File => &self.file_tree,
            Tab::Project => &self.project_tree,
            Tab::Inspection => &self.inspection_tree,
        }
    }
}

impl Render for ProblemsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let file_name = self.editor.as_ref().map(|e| e.read(cx).absolute_path().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        let file_count = self.file_count(cx);
        let project_count = self.project_tree.read(cx).item_count();
        let inspection = self.inspection.as_ref().map(|(scope, _)| (format!("Inspection of {scope}"), self.inspection_tree.read(cx).item_count()));
        let tab = |id: &'static str, label: String, count: usize, which: Tab, active: bool| {
            h_flex()
                .id(id)
                .h(px(24.))
                .px_2()
                .gap_1()
                .rounded(px(4.))
                .text_sm()
                .cursor_pointer()
                .when(active, |el| el.bg(palette.selection))
                .when(!active, |el| el.hover(|el| el.bg(palette.hover)))
                .child(label)
                .when(count > 0, |el| el.child(div().text_color(palette.text_secondary).child(count.to_string())))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.tab = which;
                    cx.notify();
                }))
        };
        let file_label = match &file_name {
            Some(name) => format!("File {name}"),
            None => "File".to_owned(),
        };
        let header = h_flex()
            .h(px(crate::ui::common::header_height()))
            .px_2()
            .gap_2()
            .flex_shrink_0()
            .border_b_1()
            .border_color(palette.border)
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Problems:"))
            .child(tab("problems-tab-file", file_label, file_count, Tab::File, self.tab == Tab::File))
            .child(tab("problems-tab-project", "Project Errors".into(), project_count, Tab::Project, self.tab == Tab::Project))
            .when_some(inspection, |el, (label, count)| el.child(tab("problems-tab-inspection", label, count, Tab::Inspection, self.tab == Tab::Inspection)))
            .child(div().flex_1())
            .child(tool_button("problems-hide", icons::HIDE, "Hide").on_click(cx.listener(|_, _, _, cx| cx.emit(ProblemsEvent::Hide))));
        let (show_warnings, by_severity) = (self.show_warnings, self.by_severity);
        let this = cx.entity().downgrade();
        let options = gpui_kit::component::button::Button::new("problems-options")
            .ghost()
            .xsmall()
            .icon(gpui_kit::component::Icon::new(icons::SHOW))
            .tooltip("View Options")
            .dropdown_menu(move |menu, _, _| {
                let (a, b) = (this.clone(), this.clone());
                menu.item(PopupMenuItem::new("Show Warnings").checked(show_warnings).on_click(move |_, _, cx| {
                    a.update(cx, |v, cx| {
                        v.show_warnings = !v.show_warnings;
                        v.refresh(cx);
                    })
                    .ok();
                }))
                .item(PopupMenuItem::new("Sort by Severity").checked(by_severity).on_click(move |_, _, cx| {
                    b.update(cx, |v, cx| {
                        v.by_severity = !v.by_severity;
                        v.refresh(cx);
                    })
                    .ok();
                }))
            });
        let tree = self.tree().clone();
        let autoscroll = tree.read(cx).autoscroll;
        let (t1, t2, t3, t4, t5) = (tree.clone(), tree.clone(), tree.clone(), tree.clone(), tree.clone());
        let toolbar = v_flex()
            .w(px(30.))
            .flex_none()
            .items_center()
            .py_1()
            .gap_0p5()
            .border_r_1()
            .border_color(palette.border)
            .child(tool_button("problems-prev", icons::UP, "Previous Problem").on_click(move |_, _, cx| t1.update(cx, |t, cx| t.step(-1, cx))))
            .child(tool_button("problems-next", icons::DOWN, "Next Problem").on_click(move |_, _, cx| t2.update(cx, |t, cx| t.step(1, cx))))
            .child(tool_button("problems-expand", icons::EXPAND_ALL, "Expand All").on_click(move |_, _, cx| t3.update(cx, |t, cx| t.set_all_collapsed(false, cx))))
            .child(tool_button("problems-collapse", icons::COLLAPSE_ALL, "Collapse All").on_click(move |_, _, cx| t4.update(cx, |t, cx| t.set_all_collapsed(true, cx))))
            .child(tool_button("problems-autoscroll", icons::AUTOSCROLL_FROM_SOURCE, "Autoscroll to Source").selected(autoscroll).on_click(move |_, _, cx| {
                t5.update(cx, |t, cx| {
                    t.autoscroll = !t.autoscroll;
                    cx.notify();
                })
            }))
            .child(options);
        let empty = tree.read(cx).is_empty();
        let empty_text = match (self.tab, &file_name) {
            (Tab::File, Some(name)) => format!("No problems found in {name}"),
            (Tab::File, None) => "No file is open in the editor".to_owned(),
            (Tab::Project, _) => "No errors found by the latest build".to_owned(),
            (Tab::Inspection, _) => "No suspicious code found".to_owned(),
        };
        v_flex().size_full().bg(palette.panel).child(header).child(
            h_flex().flex_1().min_h_0().items_stretch().child(toolbar).child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .when(empty, |el| el.child(div().p_4().text_sm().text_color(palette.text_secondary).child(empty_text)))
                    .when(!empty, |el| el.child(tree)),
            ),
        )
    }
}
