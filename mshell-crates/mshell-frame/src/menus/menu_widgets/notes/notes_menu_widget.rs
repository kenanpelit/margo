//! Notes Hub menu widget — three tabs (Scratchpad / Notes /
//! Todos) over the persistent `notes.json` document.
//!
//! Layout:
//!   * **Header** — the §12 [`PanelHeader`]: title, a counters line
//!     (notes · open todos) and a reload-from-disk action.
//!   * **Tabs** — the shared segmented capsule driving a `gtk::Stack`.
//!     - Scratchpad: a multi-line `gtk::TextView`. Edits debounce-
//!       save every ~600 ms after the user stops typing.
//!     - Notes: scrollable card list. Each card: title + body
//!       + delete. "+ Add note" appends a new card.
//!     - Todos: named pages, a show-completed filter, copy/export,
//!       and reorderable rows with expandable details
//!       ([`super::todos_tab`]).
//!
//! Every menu (one per monitor) works on the one shared
//! [`notes_store`]: an edit is applied to the shared document, which
//! notifies the other menus and the bar pills and is written by the
//! store's single writer (atomic write, last edit wins). A notes.json
//! that fails to parse is never saved over.

use super::model::{
    ExportFormat, GENERAL_PAGE_ID, Note, NotesState, Todo, export_file_name, export_todos,
};
use super::todos_tab::{TodosTab, clear_listbox, placeholder_row};
use crate::bars::bar_widgets::notes::{new_id, notes_store};
use crate::common_widgets::segmented::{Segmented, SegmentedStyle};
use crate::menus::menu_widgets::panel_header::PanelHeader;
use mshell_config::toast::{ToastEvent, ToastSeverity, push_toast};
use relm4::gtk::glib;
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, EditableExt, ListBoxRowExt, OrientableExt, TextBufferExt, TextViewExt,
    WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

pub(super) const SAVE_DEBOUNCE: Duration = Duration::from_millis(600);

/// The tabs, in segmented-control order (stack child names).
const TABS: [(&str, &str); 3] = [
    ("scratchpad", "Scratchpad"),
    ("notes", "Notes"),
    ("todos", "Todos"),
];

/// What the inline page-name entry is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PageNameMode {
    New,
    Rename,
}

pub(crate) struct NotesMenuWidgetModel {
    state: NotesState,
    header: PanelHeader,
    tabs: Segmented,
    stack: gtk::Stack,
    scratchpad_view: gtk::TextView,
    notes_list: gtk::ListBox,
    todos: TodosTab,
    /// Suppress the next save trigger from `scratchpad_view` so a
    /// programmatic `set_text` (after an external reload) doesn't
    /// pump back through the debounce path.
    scratchpad_quiet: std::rc::Rc<std::cell::Cell<bool>>,
}

impl std::fmt::Debug for NotesMenuWidgetModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotesMenuWidgetModel")
            .field("state", &self.state)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum NotesMenuWidgetInput {
    /// Re-read the JSON file from disk (the shared store notifies us).
    ReloadFromDisk,
    /// Switch tab (index into [`TABS`]).
    SelectTab(usize),
    /// Persist current scratchpad/notes/todos to disk.
    PersistNow,
    /// Append an empty note card and persist.
    AddNote,
    /// Replace an entire note (title/body) and persist.
    UpdateNote {
        id: String,
        title: String,
        body: String,
    },
    /// Delete a note by id and persist.
    DeleteNote(String),
    /// Append a todo to the current page and persist.
    AddTodo(String),
    /// Flip a todo's completed state and persist.
    ToggleTodo(String),
    /// Edit a todo's text and persist.
    UpdateTodo {
        id: String,
        text: String,
    },
    /// Edit a todo's details and persist.
    UpdateTodoDetails {
        id: String,
        details: String,
    },
    /// Move a todo by `delta` visible positions on its page.
    MoveTodo {
        id: String,
        delta: i32,
    },
    /// Delete a todo and persist.
    DeleteTodo(String),
    /// Show a todo page.
    SelectPage(String),
    /// Open the inline page-name entry.
    BeginPageName(PageNameMode),
    /// Confirm the page-name entry.
    CommitPageName(PageNameMode, String),
    CancelPageName,
    /// Delete the shown page (its todos move to General).
    DeleteCurrentPage,
    SetShowCompleted(bool),
    /// Copy the shown page to the clipboard.
    CopyTodos(ExportFormat),
    /// Save every page to `~/Documents/todo_<stamp>.<ext>`.
    ExportTodos(ExportFormat),
}

#[derive(Debug)]
pub(crate) enum NotesMenuWidgetOutput {}

pub(crate) struct NotesMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum NotesMenuWidgetCommandOutput {
    /// The shared document changed (any menu's edit, or a reload).
    Loaded(NotesState),
    /// An export finished: the written path, or the error.
    Exported(Result<std::path::PathBuf, String>),
}

#[relm4::component(pub(crate))]
impl Component for NotesMenuWidgetModel {
    type CommandOutput = NotesMenuWidgetCommandOutput;
    type Input = NotesMenuWidgetInput;
    type Output = NotesMenuWidgetOutput;
    type Init = NotesMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "notes-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 12,

            #[local_ref]
            header_widget -> gtk::Box {},

            #[local_ref]
            tabs_widget -> gtk::Box {
                set_halign: gtk::Align::Fill,
            },

            #[local_ref]
            stack_widget -> gtk::Stack {
                set_transition_type: gtk::StackTransitionType::Crossfade,
                set_transition_duration: 120,
                set_vhomogeneous: false,
                set_interpolate_size: true,

                // ── Scratchpad tab ──────────────────────────────
                #[name = "fill_list"]
                add_named[Some("scratchpad")] = &gtk::ScrolledWindow {
                    set_min_content_height: 240,
                    set_hscrollbar_policy: gtk::PolicyType::Never,
                    set_propagate_natural_height: true,

                    #[local_ref]
                    scratchpad_view_widget -> gtk::TextView {
                        add_css_class: "notes-scratchpad",
                        set_left_margin: 8,
                        set_right_margin: 8,
                        set_top_margin: 6,
                        set_bottom_margin: 6,
                        set_wrap_mode: gtk::WrapMode::WordChar,
                        set_monospace: true,
                    },
                },

                // ── Notes tab ───────────────────────────────────
                add_named[Some("notes")] = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 6,

                    gtk::Button {
                        set_css_classes: &["ok-button-surface", "ok-button-cell", "notes-add"],
                        set_label: " + Add note",
                        set_halign: gtk::Align::Start,
                        connect_clicked[sender] => move |_| {
                            sender.input(NotesMenuWidgetInput::AddNote);
                        },
                    },

                    #[name = "fill_list1"]
                    gtk::ScrolledWindow {
                        set_min_content_height: 0,
                        set_hscrollbar_policy: gtk::PolicyType::Never,
                        set_propagate_natural_height: true,

                        #[local_ref]
                        notes_list_widget -> gtk::ListBox {
                            add_css_class: "notes-list",
                            set_selection_mode: gtk::SelectionMode::None,
                        },
                    },
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let s = sender.clone();
        let header = PanelHeader::new("notes-symbolic", "Notes Hub")
            .with_meta()
            .action("view-refresh-symbolic", "Reload from disk", move || {
                s.input(NotesMenuWidgetInput::ReloadFromDisk);
            });
        let header_widget = header.root().clone();
        let labels: Vec<&str> = TABS.iter().map(|(_, l)| *l).collect();
        let s = sender.clone();
        let tabs = Segmented::new(&labels, 0, SegmentedStyle::Capsule, move |i| {
            s.input(NotesMenuWidgetInput::SelectTab(i));
        });
        let tabs_widget = tabs.root().clone();
        let stack_widget = gtk::Stack::new();
        let scratchpad_view_widget = gtk::TextView::new();
        let notes_list_widget = gtk::ListBox::new();
        let todos = TodosTab::new(&sender);
        let todos_root = todos.root.clone();
        let scratchpad_quiet = std::rc::Rc::new(std::cell::Cell::new(false));

        // Scratchpad debounce → PersistNow. Each keystroke
        // restarts a 600 ms timer; the timer that survives
        // triggers a save. Cancels via a token field on a Rc.
        let scratchpad_token = std::rc::Rc::new(std::cell::Cell::new(0u64));
        let buffer = scratchpad_view_widget.buffer();
        {
            let sender_clone = sender.clone();
            let token = scratchpad_token.clone();
            let quiet = scratchpad_quiet.clone();
            buffer.connect_changed(move |_| {
                if quiet.get() {
                    return;
                }
                let next = token.get().wrapping_add(1);
                token.set(next);
                let sender = sender_clone.clone();
                let token = token.clone();
                glib::timeout_add_local_once(SAVE_DEBOUNCE, move || {
                    if token.get() == next {
                        // Fallible: menu teardown can outrun the debounce;
                        // `input()` on a dead runtime aborts the shell.
                        let _ = sender.input_sender().send(NotesMenuWidgetInput::PersistNow);
                    }
                });
            });
        }

        // Initial load, then follow the shared store for the menu's life.
        sender.command(|out, shutdown| async move {
            let _ = tokio::task::spawn_blocking(|| notes_store().ensure_loaded()).await;
            let mut changes = notes_store().subscribe();
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            loop {
                let s = (*changes.borrow_and_update().value).clone();
                if out.send(NotesMenuWidgetCommandOutput::Loaded(s)).is_err() {
                    break;
                }
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    changed = changes.changed() => if changed.is_err() { break },
                }
            }
        });

        let model = NotesMenuWidgetModel {
            state: NotesState::default(),
            header,
            tabs,
            stack: stack_widget.clone(),
            scratchpad_view: scratchpad_view_widget.clone(),
            notes_list: notes_list_widget.clone(),
            todos,
            scratchpad_quiet,
        };

        let widgets = view_output!();
        // ── Todos tab (built by `TodosTab`) ─────────────────
        model.stack.add_named(&todos_root, Some("todos"));
        crate::menus::list_fit::mark_menu_list(&widgets.fill_list, 420);
        crate::menus::list_fit::mark_menu_list(&widgets.fill_list1, 400);
        crate::menus::list_fit::mark_menu_list(&model.todos.scroller, 400);
        sync_view(&model, &sender);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            NotesMenuWidgetInput::ReloadFromDisk => {
                sender.command(|_out, _shutdown| async move {
                    let _ = tokio::task::spawn_blocking(|| notes_store().reload()).await;
                });
            }
            NotesMenuWidgetInput::SelectTab(i) => {
                if let Some((name, _)) = TABS.get(i) {
                    self.tabs.set_active(i);
                    self.stack.set_visible_child_name(name);
                }
            }
            NotesMenuWidgetInput::PersistNow => {
                // Pull the latest scratchpad text out of the
                // TextBuffer (the canonical edit surface).
                let buffer = self.scratchpad_view.buffer();
                let start = buffer.start_iter();
                let end = buffer.end_iter();
                let text = buffer.text(&start, &end, false).to_string();
                self.edit(&sender, false, |s| s.scratchpad.clone_from(&text));
            }
            NotesMenuWidgetInput::AddNote => {
                let note = Note {
                    id: new_id(),
                    title: String::new(),
                    body: String::new(),
                };
                self.edit(&sender, true, |s| s.notes.push(note.clone()));
            }
            NotesMenuWidgetInput::UpdateNote { id, title, body } => {
                self.edit(&sender, false, |s| {
                    if let Some(n) = s.notes.iter_mut().find(|n| n.id == id) {
                        n.title.clone_from(&title);
                        n.body.clone_from(&body);
                    }
                });
            }
            NotesMenuWidgetInput::DeleteNote(id) => {
                self.edit(&sender, true, |s| s.notes.retain(|n| n.id != id));
            }
            NotesMenuWidgetInput::AddTodo(text) => {
                let trimmed = text.trim().to_string();
                if trimmed.is_empty() {
                    return;
                }
                let todo = Todo {
                    id: new_id(),
                    text: trimmed,
                    page: self.state.current_page().to_string(),
                    ..Todo::default()
                };
                self.edit(&sender, true, |s| s.todos.push(todo.clone()));
            }
            NotesMenuWidgetInput::ToggleTodo(id) => {
                // Set (not flip) the shared todo to this menu's new value,
                // so a stale menu can't undo another menu's toggle.
                let Some(done) = self
                    .state
                    .todos
                    .iter()
                    .find(|t| t.id == id)
                    .map(|t| !t.completed)
                else {
                    return;
                };
                self.edit(&sender, true, |s| {
                    if let Some(t) = s.todos.iter_mut().find(|t| t.id == id) {
                        t.completed = done;
                    }
                });
            }
            NotesMenuWidgetInput::UpdateTodo { id, text } => {
                self.edit(&sender, false, |s| {
                    if let Some(t) = s.todos.iter_mut().find(|t| t.id == id) {
                        t.text.clone_from(&text);
                    }
                });
            }
            NotesMenuWidgetInput::UpdateTodoDetails { id, details } => {
                self.edit(&sender, false, |s| {
                    if let Some(t) = s.todos.iter_mut().find(|t| t.id == id) {
                        t.details.clone_from(&details);
                    }
                });
            }
            NotesMenuWidgetInput::MoveTodo { id, delta } => {
                self.edit(&sender, true, |s| {
                    s.move_todo(&id, delta);
                });
            }
            NotesMenuWidgetInput::DeleteTodo(id) => {
                self.edit(&sender, true, |s| s.todos.retain(|t| t.id != id));
            }
            NotesMenuWidgetInput::SelectPage(id) => {
                if self.state.current_page() != id {
                    self.edit(&sender, true, |s| s.current_page.clone_from(&id));
                }
            }
            NotesMenuWidgetInput::BeginPageName(mode) => {
                let name = self.state.page_name(self.state.current_page()).to_string();
                self.todos.begin_page_name(mode, &name);
            }
            NotesMenuWidgetInput::CommitPageName(mode, name) => {
                if name.trim().is_empty() {
                    return;
                }
                self.todos.end_page_name();
                match mode {
                    PageNameMode::New => {
                        let id = format!("page-{}", new_id());
                        self.edit(&sender, true, |s| {
                            s.add_page(id.clone(), &name);
                        });
                    }
                    PageNameMode::Rename => {
                        let id = self.state.current_page().to_string();
                        self.edit(&sender, true, |s| {
                            s.rename_page(&id, &name);
                        });
                    }
                }
            }
            NotesMenuWidgetInput::CancelPageName => self.todos.end_page_name(),
            NotesMenuWidgetInput::DeleteCurrentPage => {
                let id = self.state.current_page().to_string();
                if id != GENERAL_PAGE_ID {
                    self.edit(&sender, true, |s| {
                        s.delete_page(&id);
                    });
                }
            }
            NotesMenuWidgetInput::SetShowCompleted(on) => {
                if self.state.show_completed != on {
                    self.edit(&sender, true, |s| s.show_completed = on);
                }
            }
            NotesMenuWidgetInput::CopyTodos(fmt) => {
                let text = export_todos(&self.state, Some(self.state.current_page()), fmt);
                self.todos.clipboard().set_text(&text);
                push_toast(ToastEvent {
                    icon: "edit-copy-symbolic".to_string(),
                    title: "Todos copied".to_string(),
                    body: Some(format!(
                        "{} page copied to the clipboard",
                        self.state.page_name(self.state.current_page())
                    )),
                    severity: ToastSeverity::Calm,
                });
            }
            NotesMenuWidgetInput::ExportTodos(fmt) => {
                let text = export_todos(&self.state, None, fmt);
                let dir = glib::user_special_dir(glib::UserDirectory::Documents)
                    .or_else(margo_fs::xdg::home)
                    .unwrap_or_else(|| std::path::PathBuf::from("."));
                let path = dir.join(export_file_name(chrono::Local::now().naive_local(), fmt));
                sender.oneshot_command(async move {
                    let res = tokio::task::spawn_blocking(move || {
                        std::fs::create_dir_all(&dir)
                            .and_then(|()| {
                                margo_fs::atomic_write::atomic_write(&path, text.as_bytes())
                            })
                            .map(|()| path)
                            .map_err(|e| e.to_string())
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    NotesMenuWidgetCommandOutput::Exported(res)
                });
            }
        }
    }

    fn update_cmd_with_view(
        &mut self,
        _widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NotesMenuWidgetCommandOutput::Loaded(state) => {
                // Our own edits come back here too; the mirror already has
                // them, so skip the rebuild (it would steal focus mid-typing).
                if state != self.state {
                    self.state = state;
                    sync_view(self, &sender);
                }
            }
            NotesMenuWidgetCommandOutput::Exported(res) => {
                let (title, body, severity, icon) = match res {
                    Ok(path) => (
                        "Todos exported",
                        path.display().to_string(),
                        ToastSeverity::Positive,
                        "document-save-symbolic",
                    ),
                    Err(e) => (
                        "Export failed",
                        e,
                        ToastSeverity::Warn,
                        "dialog-warning-symbolic",
                    ),
                };
                push_toast(ToastEvent {
                    icon: icon.to_string(),
                    title: title.to_string(),
                    body: Some(body),
                    severity,
                });
            }
        }
    }
}

impl NotesMenuWidgetModel {
    /// Apply one edit to this menu's mirror and to the shared document.
    /// The shared store saves it (or refuses, with a toast, while
    /// notes.json does not parse) and tells the other menus.
    fn edit(&mut self, sender: &ComponentSender<Self>, rebuild: bool, f: impl Fn(&mut NotesState)) {
        f(&mut self.state);
        if rebuild {
            sync_view(self, sender);
        }
        let _ = notes_store().update(|s| f(s));
    }
}

fn sync_view(model: &NotesMenuWidgetModel, sender: &ComponentSender<NotesMenuWidgetModel>) {
    let s = &model.state;
    let open = s.todos.iter().filter(|t| !t.completed).count();
    if let Some(meta) = model.header.meta() {
        meta.set_label(&format!(
            "{} notes · {} / {} todos",
            s.notes.len(),
            open,
            s.todos.len()
        ));
    }

    // Scratchpad — block the changed signal so the programmatic
    // `set_text` doesn't pump back into debounce-save.
    let buffer = model.scratchpad_view.buffer();
    let current = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
    if current != s.scratchpad {
        model.scratchpad_quiet.set(true);
        buffer.set_text(&s.scratchpad);
        model.scratchpad_quiet.set(false);
    }

    // Notes list — rebuild. Card edits go through the per-row
    // UpdateNote handler on focus-out / Enter.
    clear_listbox(&model.notes_list);
    if s.notes.is_empty() {
        model.notes_list.append(&placeholder_row("No notes yet"));
    } else {
        for note in &s.notes {
            model.notes_list.append(&make_note_row(note, sender));
        }
    }

    model.todos.sync(s, sender);
}

fn make_note_row(note: &Note, sender: &ComponentSender<NotesMenuWidgetModel>) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_selectable(false);
    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(8)
        .margin_end(8)
        .css_classes(vec!["notes-card"])
        .build();

    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();
    let title_entry = gtk::Entry::builder()
        .text(&note.title)
        .placeholder_text("Title")
        .hexpand(true)
        .css_classes(vec!["notes-card-title"])
        .build();
    header.append(&title_entry);
    let delete = gtk::Button::from_icon_name("trash-symbolic");
    delete.add_css_class("ok-button-flat");
    mshell_common::a11y::name(&delete, "Delete note");
    let id_for_delete = note.id.clone();
    let s_delete = sender.clone();
    delete.connect_clicked(move |_| {
        s_delete.input(NotesMenuWidgetInput::DeleteNote(id_for_delete.clone()));
    });
    header.append(&delete);
    outer.append(&header);

    let body_buf = gtk::TextBuffer::new(None);
    body_buf.set_text(&note.body);
    let body_view = gtk::TextView::with_buffer(&body_buf);
    body_view.add_css_class("notes-card-body");
    body_view.set_wrap_mode(gtk::WrapMode::WordChar);
    body_view.set_top_margin(4);
    body_view.set_bottom_margin(4);
    body_view.set_left_margin(6);
    body_view.set_right_margin(6);
    body_view.set_height_request(80);
    outer.append(&body_view);

    // Persist on focus-out via a debounce so every keystroke
    // isn't a fsync. Title + body share a single debounce token.
    let id = note.id.clone();
    let title_entry_clone = title_entry.clone();
    let body_buf_clone = body_buf.clone();
    let token = std::rc::Rc::new(std::cell::Cell::new(0u64));
    let push_update = {
        let token = token.clone();
        let sender = sender.clone();
        let id = id.clone();
        let title_entry_clone = title_entry_clone.clone();
        let body_buf_clone = body_buf_clone.clone();
        move || {
            let next = token.get().wrapping_add(1);
            token.set(next);
            let sender = sender.clone();
            let id = id.clone();
            let title_entry_clone = title_entry_clone.clone();
            let body_buf_clone = body_buf_clone.clone();
            let token = token.clone();
            glib::timeout_add_local_once(SAVE_DEBOUNCE, move || {
                if token.get() == next {
                    let title = title_entry_clone.text().to_string();
                    let body = body_buf_clone
                        .text(
                            &body_buf_clone.start_iter(),
                            &body_buf_clone.end_iter(),
                            false,
                        )
                        .to_string();
                    let _ = sender
                        .input_sender()
                        .send(NotesMenuWidgetInput::UpdateNote {
                            id: id.clone(),
                            title,
                            body,
                        });
                }
            });
        }
    };
    let push_for_title = push_update.clone();
    title_entry.connect_changed(move |_| push_for_title());
    body_buf.connect_changed(move |_| push_update());

    row.set_child(Some(&outer));
    row
}
