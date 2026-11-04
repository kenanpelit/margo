//! Notes Hub menu widget — three tabs (Scratchpad / Notes /
//! Todos) over the persistent `notes.json` document.
//!
//! Layout:
//!   * **Header** — title + counters (notes / open-todos) +
//!     reload-from-disk button.
//!   * **Tabs** — gtk::StackSwitcher driving the three panels.
//!     - Scratchpad: a multi-line `gtk::TextView`. Edits debounce-
//!       save every ~600 ms after the user stops typing.
//!     - Notes: scrollable card list. Each card: title + body
//!       + delete. Inline "+ Add note" row appends a new card
//!       with a fresh id.
//!     - Todos: scrollable check-row list. Each row: checkbox
//!       + editable text + delete. Inline "+ Add todo" entry.
//!
//! Every menu (one per monitor) works on the one shared
//! [`notes_store`]: an edit is applied to the shared document, which
//! notifies the other menus and the bar pills and is written by the
//! store's single writer (atomic write, last edit wins). A notes.json
//! that fails to parse is never saved over.

use crate::bars::bar_widgets::notes::{Note, NotesState, Todo, new_id, notes_store};
use relm4::gtk::glib;
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, CheckButtonExt, EditableExt, EntryExt, ListBoxRowExt, OrientableExt,
    TextBufferExt, TextViewExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

const SAVE_DEBOUNCE: Duration = Duration::from_millis(600);

pub(crate) struct NotesMenuWidgetModel {
    state: NotesState,
    notes_counter: gtk::Label,
    todos_counter: gtk::Label,
    scratchpad_view: gtk::TextView,
    notes_list: gtk::ListBox,
    todos_list: gtk::ListBox,
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
    /// Append a todo with the given text and persist.
    AddTodo(String),
    /// Flip a todo's completed state and persist.
    ToggleTodo(String),
    /// Edit a todo's text and persist.
    UpdateTodo { id: String, text: String },
    /// Delete a todo and persist.
    DeleteTodo(String),
}

#[derive(Debug)]
pub(crate) enum NotesMenuWidgetOutput {}

pub(crate) struct NotesMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum NotesMenuWidgetCommandOutput {
    /// The shared document changed (any menu's edit, or a reload).
    Loaded(NotesState),
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
            set_spacing: 10,

            // ── §12 panel header ────────────────────────────────
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,

                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_icon_name: Some("notes-symbolic"),
                    set_valign: gtk::Align::Center,
                },

                gtk::Label {
                    add_css_class: "panel-title",
                    set_label: "Notes Hub",
                    set_hexpand: true,
                    set_xalign: 0.0,
                },

                #[local_ref]
                notes_counter_widget -> gtk::Label {
                    add_css_class: "notes-counter",
                    set_valign: gtk::Align::Center,
                },

                #[local_ref]
                todos_counter_widget -> gtk::Label {
                    add_css_class: "notes-counter",
                    set_valign: gtk::Align::Center,
                },

                gtk::Button {
                    set_css_classes: &["ok-button-surface"],
                    set_tooltip_text: Some("Reload from disk"),
                    set_icon_name: "view-refresh-symbolic",
                    connect_clicked[sender] => move |_| {
                        sender.input(NotesMenuWidgetInput::ReloadFromDisk);
                    },
                },
            },

            // ── Tabs ───────────────────────────────────────────
            #[name = "stack_switcher"]
            gtk::StackSwitcher {
                set_stack: Some(&stack),
                set_halign: gtk::Align::Start,
            },

            #[name = "stack"]
            gtk::Stack {
                set_transition_type: gtk::StackTransitionType::SlideLeftRight,
                set_transition_duration: 200,

                // ── Scratchpad tab ──────────────────────────────
                #[name = "fill_list"]
                add_titled[Some("scratchpad"), "Scratchpad"] = &gtk::ScrolledWindow {
                    set_min_content_height: 240,
                    set_max_content_height: 420,
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
                add_titled[Some("notes"), "Notes"] = &gtk::Box {
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
                        set_min_content_height: 220,
                        set_max_content_height: 400,
                        set_hscrollbar_policy: gtk::PolicyType::Never,
                        set_propagate_natural_height: true,

                        #[local_ref]
                        notes_list_widget -> gtk::ListBox {
                            add_css_class: "notes-list",
                            set_selection_mode: gtk::SelectionMode::None,
                        },
                    },
                },

                // ── Todos tab ───────────────────────────────────
                add_titled[Some("todos"), "Todos"] = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 6,

                    #[name = "todo_entry"]
                    gtk::Entry {
                        add_css_class: "notes-add",
                        set_placeholder_text: Some("Add a todo and press Enter…"),
                        connect_activate[sender] => move |entry| {
                            let text = entry.text().to_string();
                            if !text.trim().is_empty() {
                                entry.set_text("");
                                sender.input(NotesMenuWidgetInput::AddTodo(text));
                            }
                        },
                    },

                    #[name = "fill_list2"]
                    gtk::ScrolledWindow {
                        set_min_content_height: 220,
                        set_max_content_height: 400,
                        set_hscrollbar_policy: gtk::PolicyType::Never,
                        set_propagate_natural_height: true,

                        #[local_ref]
                        todos_list_widget -> gtk::ListBox {
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
        let notes_counter_widget = gtk::Label::new(Some("0 notes"));
        let todos_counter_widget = gtk::Label::new(Some("0 todos"));
        let scratchpad_view_widget = gtk::TextView::new();
        let notes_list_widget = gtk::ListBox::new();
        let todos_list_widget = gtk::ListBox::new();
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
            notes_counter: notes_counter_widget.clone(),
            todos_counter: todos_counter_widget.clone(),
            scratchpad_view: scratchpad_view_widget.clone(),
            notes_list: notes_list_widget.clone(),
            todos_list: todos_list_widget.clone(),
            scratchpad_quiet,
        };

        let widgets = view_output!();
        crate::menus::list_fit::mark_menu_list(&widgets.fill_list, 420);
        crate::menus::list_fit::mark_menu_list(&widgets.fill_list1, 400);
        crate::menus::list_fit::mark_menu_list(&widgets.fill_list2, 400);
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
                    completed: false,
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
            NotesMenuWidgetInput::DeleteTodo(id) => {
                self.edit(&sender, true, |s| s.todos.retain(|t| t.id != id));
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
    model
        .notes_counter
        .set_label(&format!("{} notes", s.notes.len()));
    let open = s.todos.iter().filter(|t| !t.completed).count();
    model
        .todos_counter
        .set_label(&format!("{} / {} todos", open, s.todos.len()));

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
        model.notes_list.append(&placeholder_row("(no notes yet)"));
    } else {
        for note in &s.notes {
            model.notes_list.append(&make_note_row(note, sender));
        }
    }

    // Todos list.
    clear_listbox(&model.todos_list);
    if s.todos.is_empty() {
        model.todos_list.append(&placeholder_row("(no todos)"));
    } else {
        for todo in &s.todos {
            model.todos_list.append(&make_todo_row(todo, sender));
        }
    }
}

fn clear_listbox(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

fn placeholder_row(text: &str) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_selectable(false);
    let label = gtk::Label::new(Some(text));
    label.add_css_class("label-small");
    label.set_xalign(0.0);
    label.set_margin_top(8);
    label.set_margin_bottom(8);
    label.set_margin_start(12);
    row.set_child(Some(&label));
    row
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

fn make_todo_row(todo: &Todo, sender: &ComponentSender<NotesMenuWidgetModel>) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_selectable(false);

    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(8)
        .margin_end(8)
        .build();

    let check = gtk::CheckButton::new();
    check.set_active(todo.completed);
    let id_for_check = todo.id.clone();
    let s_check = sender.clone();
    check.connect_toggled(move |_| {
        s_check.input(NotesMenuWidgetInput::ToggleTodo(id_for_check.clone()));
    });
    outer.append(&check);

    let entry = gtk::Entry::builder()
        .text(&todo.text)
        .hexpand(true)
        .css_classes(vec!["notes-todo-entry"])
        .build();
    if todo.completed {
        entry.add_css_class("completed");
    }
    let id_for_edit = todo.id.clone();
    let s_edit = sender.clone();
    let edit_token = std::rc::Rc::new(std::cell::Cell::new(0u64));
    let entry_clone = entry.clone();
    entry.connect_changed(move |_| {
        let next = edit_token.get().wrapping_add(1);
        edit_token.set(next);
        let id = id_for_edit.clone();
        let s = s_edit.clone();
        let entry_clone = entry_clone.clone();
        let token = edit_token.clone();
        glib::timeout_add_local_once(SAVE_DEBOUNCE, move || {
            if token.get() == next {
                let _ = s.input_sender().send(NotesMenuWidgetInput::UpdateTodo {
                    id: id.clone(),
                    text: entry_clone.text().to_string(),
                });
            }
        });
    });
    outer.append(&entry);

    let delete = gtk::Button::from_icon_name("trash-symbolic");
    delete.add_css_class("ok-button-flat");
    mshell_common::a11y::name(&delete, "Delete todo");
    let id_for_delete = todo.id.clone();
    let s_delete = sender.clone();
    delete.connect_clicked(move |_| {
        s_delete.input(NotesMenuWidgetInput::DeleteTodo(id_for_delete.clone()));
    });
    outer.append(&delete);

    row.set_child(Some(&outer));
    row
}
