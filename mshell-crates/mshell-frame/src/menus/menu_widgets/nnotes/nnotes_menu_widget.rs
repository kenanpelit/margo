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
//! All mutations go through the same write path
//! (`save_notes`), which writes-and-renames a sibling `.tmp`
//! file so a crash can't corrupt the document.

use crate::bars::bar_widgets::nnotes::{
    NotesState, Note, Todo, load_notes, new_id, save_notes,
};
use relm4::gtk::glib;
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, CheckButtonExt, EditableExt, EntryExt, ListBoxRowExt,
    OrientableExt, TextBufferExt, TextViewExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;
use tracing::warn;

const SAVE_DEBOUNCE: Duration = Duration::from_millis(600);

pub(crate) struct NnotesMenuWidgetModel {
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

impl std::fmt::Debug for NnotesMenuWidgetModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NnotesMenuWidgetModel")
            .field("state", &self.state)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum NnotesMenuWidgetInput {
    /// Re-read the JSON file from disk and re-sync the view.
    ReloadFromDisk,
    /// Persist current scratchpad/notes/todos to disk.
    PersistNow,
    /// Append an empty note card and persist.
    AddNote,
    /// Replace an entire note (title/body) and persist.
    UpdateNote { id: String, title: String, body: String },
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
pub(crate) enum NnotesMenuWidgetOutput {}

pub(crate) struct NnotesMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum NnotesMenuWidgetCommandOutput {
    Loaded(NotesState),
    Saved,
}

#[relm4::component(pub(crate))]
impl Component for NnotesMenuWidgetModel {
    type CommandOutput = NnotesMenuWidgetCommandOutput;
    type Input = NnotesMenuWidgetInput;
    type Output = NnotesMenuWidgetOutput;
    type Init = NnotesMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "nnotes-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            // ── Header ──────────────────────────────────────────
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,

                gtk::Image {
                    set_icon_name: Some("notes-symbolic"),
                    set_pixel_size: 24,
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Notes Hub",
                    set_hexpand: true,
                    set_xalign: 0.0,
                },

                #[local_ref]
                notes_counter_widget -> gtk::Label {
                    add_css_class: "nnotes-counter",
                    set_valign: gtk::Align::Center,
                },

                #[local_ref]
                todos_counter_widget -> gtk::Label {
                    add_css_class: "nnotes-counter",
                    set_valign: gtk::Align::Center,
                },

                gtk::Button {
                    set_css_classes: &["ok-button-surface"],
                    set_tooltip_text: Some("Reload from disk"),
                    set_icon_name: "view-refresh-symbolic",
                    connect_clicked[sender] => move |_| {
                        sender.input(NnotesMenuWidgetInput::ReloadFromDisk);
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
                add_titled[Some("scratchpad"), "Scratchpad"] = &gtk::ScrolledWindow {
                    set_min_content_height: 240,
                    set_max_content_height: 420,
                    set_hscrollbar_policy: gtk::PolicyType::Never,
                    set_propagate_natural_height: true,

                    #[local_ref]
                    scratchpad_view_widget -> gtk::TextView {
                        add_css_class: "nnotes-scratchpad",
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
                        set_css_classes: &["ok-button-surface", "nnotes-add"],
                        set_label: " + Add note",
                        set_halign: gtk::Align::Start,
                        connect_clicked[sender] => move |_| {
                            sender.input(NnotesMenuWidgetInput::AddNote);
                        },
                    },

                    gtk::ScrolledWindow {
                        set_min_content_height: 220,
                        set_max_content_height: 400,
                        set_hscrollbar_policy: gtk::PolicyType::Never,
                        set_propagate_natural_height: true,

                        #[local_ref]
                        notes_list_widget -> gtk::ListBox {
                            add_css_class: "nnotes-list",
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
                        add_css_class: "nnotes-add",
                        set_placeholder_text: Some("Add a todo and press Enter…"),
                        connect_activate[sender] => move |entry| {
                            let text = entry.text().to_string();
                            if !text.trim().is_empty() {
                                entry.set_text("");
                                sender.input(NnotesMenuWidgetInput::AddTodo(text));
                            }
                        },
                    },

                    gtk::ScrolledWindow {
                        set_min_content_height: 220,
                        set_max_content_height: 400,
                        set_hscrollbar_policy: gtk::PolicyType::Never,
                        set_propagate_natural_height: true,

                        #[local_ref]
                        todos_list_widget -> gtk::ListBox {
                            add_css_class: "nnotes-list",
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
                        sender.input(NnotesMenuWidgetInput::PersistNow);
                    }
                });
            });
        }

        // Initial load.
        sender.command(|out, _shutdown| async move {
            let s = load_notes().await;
            let _ = out.send(NnotesMenuWidgetCommandOutput::Loaded(s));
        });

        let model = NnotesMenuWidgetModel {
            state: NotesState::default(),
            notes_counter: notes_counter_widget.clone(),
            todos_counter: todos_counter_widget.clone(),
            scratchpad_view: scratchpad_view_widget.clone(),
            notes_list: notes_list_widget.clone(),
            todos_list: todos_list_widget.clone(),
            scratchpad_quiet,
        };

        let widgets = view_output!();
        sync_view(&model, &sender);

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NnotesMenuWidgetInput::ReloadFromDisk => {
                sender.command(|out, _shutdown| async move {
                    let s = load_notes().await;
                    let _ = out.send(NnotesMenuWidgetCommandOutput::Loaded(s));
                });
            }
            NnotesMenuWidgetInput::PersistNow => {
                // Pull the latest scratchpad text out of the
                // TextBuffer (the canonical edit surface) before
                // we serialise — the in-memory `state.scratchpad`
                // only updates on Loaded.
                let buffer = self.scratchpad_view.buffer();
                let start = buffer.start_iter();
                let end = buffer.end_iter();
                self.state.scratchpad = buffer.text(&start, &end, false).to_string();
                persist(self.state.clone(), sender.clone());
            }
            NnotesMenuWidgetInput::AddNote => {
                self.state.notes.push(Note {
                    id: new_id(),
                    title: String::new(),
                    body: String::new(),
                });
                sync_view(self, &sender);
                persist(self.state.clone(), sender.clone());
            }
            NnotesMenuWidgetInput::UpdateNote { id, title, body } => {
                if let Some(n) = self.state.notes.iter_mut().find(|n| n.id == id) {
                    n.title = title;
                    n.body = body;
                }
                persist(self.state.clone(), sender.clone());
            }
            NnotesMenuWidgetInput::DeleteNote(id) => {
                self.state.notes.retain(|n| n.id != id);
                sync_view(self, &sender);
                persist(self.state.clone(), sender.clone());
            }
            NnotesMenuWidgetInput::AddTodo(text) => {
                let trimmed = text.trim().to_string();
                if trimmed.is_empty() {
                    return;
                }
                self.state.todos.push(Todo {
                    id: new_id(),
                    text: trimmed,
                    completed: false,
                });
                sync_view(self, &sender);
                persist(self.state.clone(), sender.clone());
            }
            NnotesMenuWidgetInput::ToggleTodo(id) => {
                if let Some(t) = self.state.todos.iter_mut().find(|t| t.id == id) {
                    t.completed = !t.completed;
                }
                sync_view(self, &sender);
                persist(self.state.clone(), sender.clone());
            }
            NnotesMenuWidgetInput::UpdateTodo { id, text } => {
                if let Some(t) = self.state.todos.iter_mut().find(|t| t.id == id) {
                    t.text = text;
                }
                persist(self.state.clone(), sender.clone());
            }
            NnotesMenuWidgetInput::DeleteTodo(id) => {
                self.state.todos.retain(|t| t.id != id);
                sync_view(self, &sender);
                persist(self.state.clone(), sender.clone());
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
            NnotesMenuWidgetCommandOutput::Loaded(state) => {
                self.state = state;
                sync_view(self, &sender);
            }
            NnotesMenuWidgetCommandOutput::Saved => {}
        }
    }
}

fn persist(state: NotesState, sender: ComponentSender<NnotesMenuWidgetModel>) {
    sender.command(move |out, _shutdown| async move {
        if let Err(e) = save_notes(&state).await {
            warn!(error = %e, "notes save failed");
        }
        let _ = out.send(NnotesMenuWidgetCommandOutput::Saved);
    });
}

fn sync_view(model: &NnotesMenuWidgetModel, sender: &ComponentSender<NnotesMenuWidgetModel>) {
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

fn make_note_row(note: &Note, sender: &ComponentSender<NnotesMenuWidgetModel>) -> gtk::ListBoxRow {
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
        .css_classes(vec!["nnotes-card"])
        .build();

    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();
    let title_entry = gtk::Entry::builder()
        .text(&note.title)
        .placeholder_text("Title")
        .hexpand(true)
        .css_classes(vec!["nnotes-card-title"])
        .build();
    header.append(&title_entry);
    let delete = gtk::Button::from_icon_name("trash-symbolic");
    delete.add_css_class("ok-button-flat");
    delete.set_tooltip_text(Some("Delete note"));
    let id_for_delete = note.id.clone();
    let s_delete = sender.clone();
    delete.connect_clicked(move |_| {
        s_delete.input(NnotesMenuWidgetInput::DeleteNote(id_for_delete.clone()));
    });
    header.append(&delete);
    outer.append(&header);

    let body_buf = gtk::TextBuffer::new(None);
    body_buf.set_text(&note.body);
    let body_view = gtk::TextView::with_buffer(&body_buf);
    body_view.add_css_class("nnotes-card-body");
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
                    sender.input(NnotesMenuWidgetInput::UpdateNote {
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

fn make_todo_row(todo: &Todo, sender: &ComponentSender<NnotesMenuWidgetModel>) -> gtk::ListBoxRow {
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
        s_check.input(NnotesMenuWidgetInput::ToggleTodo(id_for_check.clone()));
    });
    outer.append(&check);

    let entry = gtk::Entry::builder()
        .text(&todo.text)
        .hexpand(true)
        .css_classes(vec!["nnotes-todo-entry"])
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
                s.input(NnotesMenuWidgetInput::UpdateTodo {
                    id: id.clone(),
                    text: entry_clone.text().to_string(),
                });
            }
        });
    });
    outer.append(&entry);

    let delete = gtk::Button::from_icon_name("trash-symbolic");
    delete.add_css_class("ok-button-flat");
    delete.set_tooltip_text(Some("Delete todo"));
    let id_for_delete = todo.id.clone();
    let s_delete = sender.clone();
    delete.connect_clicked(move |_| {
        s_delete.input(NnotesMenuWidgetInput::DeleteTodo(id_for_delete.clone()));
    });
    outer.append(&delete);

    row.set_child(Some(&outer));
    row
}
