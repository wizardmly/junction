//! The Shelf tab of the Commit tool window: shelved changelists, their files,
//! Unshelve, Rename, Delete, Recently Deleted, and Import Patches.

use std::collections::HashMap;

use gpui_kit::component::{
    Disableable as _, Icon, Sizable as _, WindowExt as _, h_flex,
    input::{Input, InputState},
    list::ListItem,
    menu::{ContextMenuExt as _, PopupMenuItem},
    scroll::ScrollableElement as _,
    tree::{TreeState, tree},
    v_flex, v_resizable, resizable_panel,
};
use gpui_kit::assets::IconName;
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement, ParentElement as _,
    PathPromptOptions, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window,
    div, prelude::FluentBuilder as _, px,
};

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui_kit::component::checkbox::Checkbox;

use crate::git::FileChangeKind;
use crate::git::changelists::Changelists;
use crate::git::patch::{self, ApplyOutcome, Shelf};
use crate::model::{RepoEvent, RepoModel};
use crate::theme::ActivePalette as _;
use crate::ui::common::{self, FILE_PREFIX, row_height, tool_button};
use crate::ui::dialogs::{focus_input, footer};
use crate::ui::diff_view::DiffSource;

pub enum ShelfEvent {
    OpenDiff(DiffSource),
}

impl EventEmitter<ShelfEvent> for ShelfView {}

pub struct ShelfView {
    model: Entity<RepoModel>,
    shelves: Vec<Shelf>,
    selected: Option<String>,
    show_deleted: bool,
    files: Entity<TreeState>,
    kinds: HashMap<String, FileChangeKind>,
    old_paths: HashMap<String, String>,
    last_file: Option<SharedString>,
    _load: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl ShelfView {
    pub fn new(model: Entity<RepoModel>, cx: &mut Context<Self>) -> Self {
        let files = cx.new(|cx| TreeState::new(cx));
        let subscriptions = vec![
            cx.subscribe(&model, |this, _, event, cx| {
                if matches!(event, RepoEvent::Reloaded) {
                    this.reload(cx);
                }
            }),
            cx.observe(&files, |this, files, cx| {
                let selected = files.read(cx).selected_item().map(|i| i.id.clone());
                if selected != this.last_file {
                    this.last_file = selected.clone();
                    let path = selected.as_deref().and_then(|id| id.strip_prefix(FILE_PREFIX)).map(str::to_owned);
                    if let (Some(path), Some(shelf)) = (path, this.current()) {
                        if !shelf.commit.is_empty() {
                            let old_path = this.old_paths.get(&path).cloned();
                            cx.emit(ShelfEvent::OpenDiff(DiffSource::Between {
                                old: shelf.base.clone(),
                                new: Some(shelf.commit.clone()),
                                path,
                                old_path,
                            }));
                        }
                    }
                }
            }),
        ];
        let mut this = Self {
            model,
            shelves: Vec::new(),
            selected: None,
            show_deleted: false,
            files,
            kinds: HashMap::new(),
            old_paths: HashMap::new(),
            last_file: None,
            _load: None,
            _subscriptions: subscriptions,
        };
        this.reload(cx);
        this
    }

    fn current(&self) -> Option<&Shelf> {
        let id = self.selected.as_deref()?;
        self.shelves.iter().find(|s| s.id == id)
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(repository) = self.model.read(cx).repository().cloned() else { return };
        self._load = Some(cx.spawn(async move |this, cx| {
            let shelves = cx.background_spawn(async move { patch::shelves(&repository) }).await;
            this.update(cx, |this, cx| {
                this.shelves = shelves;
                let show_deleted = this.show_deleted;
                let keep = this.selected.clone().filter(|id| this.shelves.iter().any(|s| &s.id == id && (!s.deleted || show_deleted)));
                let first = this.shelves.iter().find(|s| !s.deleted).map(|s| s.id.clone());
                this.select(keep.or(first), cx);
            })
            .ok();
        }));
    }

    fn select(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        self.selected = id;
        self.kinds.clear();
        self.old_paths.clear();
        self.last_file = None;
        let Some(shelf) = self.current().cloned() else {
            self.files.update(cx, |tree, cx| tree.set_items(Vec::new(), cx));
            cx.notify();
            return;
        };
        let Some(repository) = self.model.read(cx).repository().cloned() else { return };
        self._load = Some(cx.spawn(async move |this, cx| {
            let changes = cx
                .background_spawn(async move { patch::shelf_files(&repository, &shelf).unwrap_or_default() })
                .await;
            this.update(cx, |this, cx| {
                this.kinds = changes.iter().map(|c| (c.path.clone(), c.kind)).collect();
                this.old_paths =
                    changes.iter().filter_map(|c| c.old_path.clone().map(|o| (c.path.clone(), o))).collect();
                let items = common::file_tree(changes.into_iter().map(|c| c.path), "");
                this.files.update(cx, |tree, cx| tree.set_items(items, cx));
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// IntelliJ's Unshelve Changes dialog: the changelist to put the changes
    /// in (an existing one or a new name) and whether to remove them from the shelf.
    fn unshelve_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(shelf) = self.current().cloned() else { return };
        let Some(repository) = self.model.read(cx).repository().cloned() else { return };
        let lists = Changelists::load(&repository);
        let names: Vec<String> = lists.lists.iter().map(|l| l.name.clone()).collect();
        let selected = Rc::new(RefCell::new(lists.active.clone()));
        let remove = Rc::new(Cell::new(!shelf.deleted));
        let new_name = cx.new(|cx| InputState::new(window, cx).placeholder("New changelist name"));
        let entity = cx.entity();
        window.open_dialog(cx, move |dialog, _, cx| {
            let palette = cx.palette().clone();
            let mut list = v_flex().gap_0p5();
            for (ix, name) in names.iter().enumerate() {
                let chosen = *selected.borrow() == *name;
                let (select, name_owned) = (selected.clone(), name.clone());
                list = list.child(
                    h_flex()
                        .id(SharedString::from(format!("unshelve-target-{ix}")))
                        .px_2()
                        .h(px(26.))
                        .rounded_sm()
                        .text_sm()
                        .cursor_pointer()
                        .when(chosen, |el| el.bg(palette.selection))
                        .child(name.clone())
                        .on_click(move |_, window, _| {
                            *select.borrow_mut() = name_owned.clone();
                            window.refresh();
                        }),
                );
            }
            let (remove_set, remove_ok) = (remove.clone(), remove.clone());
            let (selected, new_name_ok, entity, shelf) = (selected.clone(), new_name.clone(), entity.clone(), shelf.clone());
            dialog
                .title("Unshelve Changes")
                .w(px(420.))
                .child(
                    v_flex()
                        .gap_2()
                        .child(div().text_sm().text_color(palette.text_secondary).child("Changelist:"))
                        .child(list)
                        .child(div().text_sm().text_color(palette.text_secondary).child("Or a new changelist:"))
                        .child(Input::new(&new_name))
                        .child(
                            Checkbox::new("unshelve-remove")
                                .label("Remove successfully applied files from the shelf")
                                .checked(remove.get())
                                .on_change(move |value, window, _| {
                                    remove_set.set(*value);
                                    window.refresh();
                                }),
                        ),
                )
                .footer(footer("Unshelve"))
                .on_ok(move |_, _, cx| {
                    let new = new_name_ok.read(cx).value().trim().to_owned();
                    let target = if new.is_empty() { selected.borrow().clone() } else { new };
                    let keep = !remove_ok.get();
                    let shelf = shelf.clone();
                    entity.update(cx, |this, cx| this.unshelve_to(shelf, keep, Some(target), cx));
                    true
                })
        });
    }

    fn unshelve(&mut self, keep: bool, cx: &mut Context<Self>) {
        let Some(shelf) = self.current().cloned() else { return };
        self.unshelve_to(shelf, keep, None, cx);
    }

    /// Unshelves `shelf`, filing its files into `changelist` when given.
    fn unshelve_to(&mut self, shelf: Shelf, keep: bool, changelist: Option<String>, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| {
            model.run_operation("Unshelve", move |repo| {
                let keep = keep || shelf.deleted;
                let outcome = patch::unshelve(repo, &shelf, None, keep)?;
                if let Some(target) = changelist {
                    let files: Vec<String> = patch::shelf_files(repo, &shelf)?.into_iter().map(|f| f.path).collect();
                    let mut lists = Changelists::load(repo);
                    lists.add(&target, "", false);
                    lists.move_files(&files, &target);
                    lists.save(repo);
                }
                Ok(match outcome {
                    ApplyOutcome::Clean => format!("Unshelved \u{201c}{}\u{201d}", shelf.name),
                    ApplyOutcome::Merged => format!("Unshelved \u{201c}{}\u{201d} with a three-way merge", shelf.name),
                    ApplyOutcome::Conflicts => format!("Unshelved \u{201c}{}\u{201d} with conflicts", shelf.name),
                })
            }, cx)
        });
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        let Some(shelf) = self.current().cloned() else { return };
        self.model.update(cx, |model, cx| {
            model.run_operation("Delete Shelf", move |repo| {
                patch::delete_shelf(repo, &shelf)?;
                Ok(if shelf.deleted {
                    format!("Deleted \u{201c}{}\u{201d} permanently", shelf.name)
                } else {
                    format!("Moved \u{201c}{}\u{201d} to Recently Deleted", shelf.name)
                })
            }, cx)
        });
    }

    fn restore(&mut self, cx: &mut Context<Self>) {
        let Some(shelf) = self.current().cloned() else { return };
        self.model.update(cx, |model, cx| {
            model.run_operation("Restore Shelf", move |repo| {
                patch::set_deleted(repo, &shelf, false)?;
                Ok(format!("Restored \u{201c}{}\u{201d}", shelf.name))
            }, cx)
        });
    }

    fn rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(shelf) = self.current().cloned() else { return };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(shelf.name.clone()));
        let model = self.model.clone();
        let focus = input.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let input = input.clone();
            let model = model.clone();
            let shelf = shelf.clone();
            dialog
                .title("Rename Shelved Changelist")
                .w(px(420.))
                .child(Input::new(&input))
                .on_ok(move |_, _, cx| {
                    let name = input.read(cx).value().trim().to_owned();
                    if name.is_empty() {
                        return false;
                    }
                    let shelf = shelf.clone();
                    model.update(cx, |model, cx| {
                        model.run_operation("Rename Shelf", move |repo| {
                            patch::rename_shelf(repo, &shelf, &name)?;
                            Ok(format!("Renamed to \u{201c}{name}\u{201d}"))
                        }, cx)
                    });
                    true
                })
                .footer(footer("Rename"))
        });
        focus_input(&focus, window, cx);
    }

    fn import(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Import Patches".into()),
        });
        let model = self.model.clone();
        cx.spawn(async move |_, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else { return };
            model.update(cx, |model, cx| {
                model.run_operation("Import Patches", move |repo| {
                    for path in &paths {
                        let text = std::fs::read_to_string(path)?;
                        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                        patch::import_patch(repo, &name, &text)?;
                    }
                    Ok(format!("Imported {} patch{}", paths.len(), if paths.len() == 1 { "" } else { "es" }))
                }, cx)
            });
        })
        .detach();
    }

    fn render_row(&self, shelf: &Shelf, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let selected = self.selected.as_deref() == Some(shelf.id.as_str());
        let id = shelf.id.clone();
        let deleted = shelf.deleted;
        let menu_entity = cx.entity();
        let menu_id = id.clone();
        h_flex()
            .id(SharedString::from(format!("shelf-{id}")))
            .h(px(row_height() + 4.))
            .px_2()
            .gap_2()
            .text_sm()
            .when(selected, |el| el.bg(palette.selection))
            .when(!selected, |el| el.hover(|s| s.bg(palette.hover)))
            .on_click(cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                this.select(Some(id.clone()), cx);
                if event.click_count() == 2 && !deleted {
                    this.unshelve_dialog(window, cx);
                }
            }))
            .context_menu(move |menu, _, _| {
                let entity = menu_entity.clone();
                let id = menu_id.clone();
                let act = move |f: fn(&mut ShelfView, &mut Window, &mut Context<ShelfView>)| {
                    let entity = entity.clone();
                    let id = id.clone();
                    move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut gpui_kit::App| {
                        entity.update(cx, |this, cx| {
                            if this.selected.as_deref() != Some(id.as_str()) {
                                this.select(Some(id.clone()), cx);
                            }
                            f(this, window, cx)
                        })
                    }
                };
                menu.item(PopupMenuItem::new("Unshelve…").on_click(act(|this, window, cx| this.unshelve_dialog(window, cx))))
                    .item(PopupMenuItem::new("Unshelve and Keep in Shelf").on_click(act(|this, _, cx| this.unshelve(true, cx))))
                    .separator()
                    .when(deleted, |menu| menu.item(PopupMenuItem::new("Restore").on_click(act(|this, _, cx| this.restore(cx)))))
                    .item(PopupMenuItem::new("Rename…").on_click(act(|this, window, cx| this.rename(window, cx))))
                    .item(PopupMenuItem::new(if deleted { "Delete Permanently" } else { "Delete" }).on_click(act(|this, _, cx| this.delete(cx))))
            })
            .child(Icon::new(IconName::Layers).small().text_color(palette.text_secondary))
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .when(deleted, |el| el.text_color(palette.text_secondary))
                    .child(shelf.name.clone()),
            )
            .child(div().text_xs().text_color(palette.text_secondary).child(common::format_date(shelf.time)))
    }
}

impl Render for ShelfView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let current = self.current().cloned();
        let has_selection = current.is_some();
        let deleted_count = self.shelves.iter().filter(|s| s.deleted).count();

        let mut list = v_flex();
        let active: Vec<Shelf> = self.shelves.iter().filter(|s| !s.deleted).cloned().collect();
        for shelf in &active {
            list = list.child(self.render_row(shelf, cx));
        }
        if active.is_empty() {
            list = list.child(
                div()
                    .p_3()
                    .text_sm()
                    .text_color(palette.text_secondary)
                    .child("No shelved changes. Use Shelve Changes in the Commit tab to put changes aside."),
            );
        }
        if deleted_count > 0 {
            list = list.child(
                h_flex()
                    .id("shelf-deleted")
                    .h(px(row_height() + 4.))
                    .px_2()
                    .gap_1()
                    .text_sm()
                    .text_color(palette.text_secondary)
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_deleted = !this.show_deleted;
                        cx.notify();
                    }))
                    .child(Icon::new(if self.show_deleted { IconName::ChevronDown } else { IconName::ChevronRight }).xsmall())
                    .child(format!("Recently Deleted ({deleted_count})")),
            );
            if self.show_deleted {
                let deleted: Vec<Shelf> = self.shelves.iter().filter(|s| s.deleted).cloned().collect();
                for shelf in &deleted {
                    list = list.child(div().pl_3().child(self.render_row(shelf, cx)));
                }
            }
        }

        let kinds = self.kinds.clone();
        let tree_palette = palette.clone();
        let files = tree(&self.files, move |ix, entry, _, _, _| {
            let palette = &tree_palette;
            let item = entry.item();
            let path = item.id.strip_prefix(FILE_PREFIX).map(str::to_owned);
            let color = path.as_ref().and_then(|p| kinds.get(p)).map_or(palette.text, |k| common::change_color(*k, palette));
            ListItem::new(ix).py_0().px_1().h(px(row_height())).child(
                h_flex()
                    .gap_1()
                    .pl(px(entry.depth() as f32 * 14.))
                    .text_sm()
                    .child(
                        Icon::new(match &path {
                            Some(p) => common::file_icon(p),
                            None => IconName::Folder,
                        })
                        .small()
                        .text_color(palette.text_secondary),
                    )
                    .child(div().text_color(color).child(item.label.clone())),
            )
        });

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(px(crate::ui::common::toolbar_height()))
                    .px_1()
                    .gap_0p5()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(
                        tool_button("shelf-unshelve", IconName::ArrowUpFromLine, "Unshelve")
                            .disabled(!has_selection)
                            .on_click(cx.listener(|this, _, window, cx| this.unshelve_dialog(window, cx))),
                    )
                    .child(
                        tool_button("shelf-delete", IconName::X, "Delete")
                            .disabled(!has_selection)
                            .on_click(cx.listener(|this, _, _, cx| this.delete(cx))),
                    )
                    .child(tool_button("shelf-import", IconName::ArrowDownToLine, "Import Patches…").on_click(
                        cx.listener(|this, _, _, cx| this.import(cx)),
                    ))
                    .child(tool_button("shelf-refresh", IconName::RefreshCw, "Refresh").on_click(cx.listener(
                        |this, _, _, cx| this.reload(cx),
                    ))),
            )
            .child(
                div().flex_1().min_h_0().child(
                    v_resizable("shelf-split")
                        .child(resizable_panel().child(div().id("shelf-list").size_full().overflow_y_scrollbar().child(list)))
                        .child(resizable_panel().size(px(220.)).child(files.size_full())),
                ),
            )
    }
}
