//! Notes Hub menu widget — three tabs (Scratchpad / Notes /
//! Todos) over the persistent `notes.json` document.
//!
//! Layout:
//!   * **Header** — the §12 [`PanelHeader`]: title, a counters line
//!     (notes · open todos) and a reload-from-disk action.
//!   * **Tabs** — the shared segmented capsule driving a `gtk::Stack`.
//!     - Scratchpad: a multi-line `gtk::TextView`. Edits debounce-
//!       save every ~600 ms after the user stops typing.
//!     - Notes: sticky cards — a theme-role tint, pin-to-top, a
//!       checklist under the body, and archive instead of delete
//!       ([`super::notes_tab`]).
//!     - Todos: named pages, a show-completed filter, copy/export,
//!       and reorderable rows with expandable details
//!       ([`super::todos_tab`]).
//!     - Files: Markdown notes in a folder with search, capture, an
//!       editor, git history and AI actions ([`super::files_tab`]).
//!
//! Every menu (one per monitor) works on the one shared
//! [`notes_store`]: an edit is applied to the shared document, which
//! notifies the other menus and the bar pills and is written by the
//! store's single writer (atomic write, last edit wins). A notes.json
//! that fails to parse is never saved over.

use super::files_tab::{FilesTabInit, FilesTabInput, FilesTabModel};
use super::model::{
    CheckItem, ExportFormat, GENERAL_PAGE_ID, Note, NoteColor, NotesState, Todo, export_file_name,
    export_todos,
};
use super::notes_tab::NotesTab;
use super::todos_tab::TodosTab;
use crate::bars::bar_widgets::notes::{new_id, notes_store};
use crate::common_widgets::segmented::{Segmented, SegmentedStyle};
use crate::menus::menu_widgets::panel_header::PanelHeader;
use mshell_config::toast::{ToastEvent, ToastSeverity, push_toast};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, OrientableExt, TextBufferExt, TextViewExt, WidgetExt};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};
use std::time::Duration;

pub(super) const SAVE_DEBOUNCE: Duration = Duration::from_millis(600);

/// The tabs, in segmented-control order (stack child names).
const TABS: [(&str, &str); 4] = [
    ("scratchpad", "Scratchpad"),
    ("notes", "Notes"),
    ("todos", "Todos"),
    ("files", "Files"),
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
    notes: NotesTab,
    /// The Notes tab shows the archive instead of the active notes.
    show_archive: bool,
    todos: TodosTab,
    /// The Markdown-folder tab (its own component: file, git and AI work).
    files: Controller<FilesTabModel>,
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
    /// Delete a note for good (from the archive) and persist.
    DeleteNote(String),
    /// Put a note in the archive (a blank note is just removed).
    ArchiveNote(String),
    /// Bring an archived note back.
    RestoreNote(String),
    /// Show the archive (true) or the active notes.
    SetArchiveView(bool),
    SetPinned {
        id: String,
        pinned: bool,
    },
    SetNoteColor {
        id: String,
        color: NoteColor,
    },
    AddCheckItem {
        note: String,
        text: String,
    },
    SetCheckItem {
        note: String,
        item: String,
        done: bool,
    },
    UpdateCheckItem {
        note: String,
        item: String,
        text: String,
    },
    RemoveCheckItem {
        note: String,
        item: String,
    },
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
                        add_css_class: mshell_common::autofocus::CLASS,
                        add_css_class: "notes-scratchpad",
                        set_left_margin: 8,
                        set_right_margin: 8,
                        set_top_margin: 6,
                        set_bottom_margin: 6,
                        set_wrap_mode: gtk::WrapMode::WordChar,
                        set_monospace: true,
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
        let notes = NotesTab::new(&sender);
        let notes_root = notes.root.clone();
        let todos = TodosTab::new(&sender);
        let todos_root = todos.root.clone();
        let files = FilesTabModel::builder().launch(FilesTabInit {}).detach();
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
            notes,
            show_archive: false,
            todos,
            files,
            scratchpad_quiet,
        };

        let widgets = view_output!();
        // ── Notes / Todos tabs (built by `NotesTab` / `TodosTab`) ──
        model.stack.add_named(&notes_root, Some("notes"));
        model.stack.add_named(&todos_root, Some("todos"));
        model.stack.add_named(model.files.widget(), Some("files"));
        crate::menus::list_fit::mark_menu_list(&widgets.fill_list, 420);
        crate::menus::list_fit::mark_menu_list(&model.notes.scroller, 400);
        crate::menus::list_fit::mark_menu_list(&model.todos.scroller, 400);
        crate::menus::list_fit::mark_menu_list(&model.files.model().scroller, 400);
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
                    if *name == "files" {
                        self.files.emit(FilesTabInput::Shown);
                    }
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
                    ..Note::default()
                };
                self.show_archive = false;
                // New notes go to the top of the unpinned notes.
                self.edit(&sender, true, |s| {
                    let at = s
                        .notes
                        .iter()
                        .position(|n| !n.pinned && !n.archived)
                        .unwrap_or(s.notes.len());
                    s.notes.insert(at, note.clone());
                });
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
                self.edit(&sender, true, |s| {
                    s.delete_note(&id);
                });
            }
            NotesMenuWidgetInput::ArchiveNote(id) => {
                self.edit(&sender, true, |s| {
                    s.archive_note(&id);
                });
            }
            NotesMenuWidgetInput::RestoreNote(id) => {
                self.edit(&sender, true, |s| {
                    s.restore_note(&id);
                });
            }
            NotesMenuWidgetInput::SetArchiveView(on) => {
                if self.show_archive != on {
                    self.show_archive = on;
                    sync_view(self, &sender);
                }
            }
            NotesMenuWidgetInput::SetPinned { id, pinned } => {
                self.edit(&sender, true, |s| {
                    if let Some(n) = s.note_mut(&id) {
                        n.pinned = pinned;
                    }
                });
            }
            NotesMenuWidgetInput::SetNoteColor { id, color } => {
                self.edit(&sender, true, |s| {
                    if let Some(n) = s.note_mut(&id) {
                        n.color = color;
                    }
                });
            }
            NotesMenuWidgetInput::AddCheckItem { note, text } => {
                let item = CheckItem {
                    id: new_id(),
                    text: text.trim().to_string(),
                    done: false,
                };
                if item.text.is_empty() {
                    return;
                }
                self.edit(&sender, true, |s| {
                    if let Some(n) = s.note_mut(&note) {
                        n.checklist.push(item.clone());
                    }
                });
            }
            NotesMenuWidgetInput::SetCheckItem { note, item, done } => {
                self.edit(&sender, true, |s| {
                    if let Some(c) = s
                        .note_mut(&note)
                        .and_then(|n| n.checklist.iter_mut().find(|c| c.id == item))
                    {
                        c.done = done;
                    }
                });
            }
            NotesMenuWidgetInput::UpdateCheckItem { note, item, text } => {
                self.edit(&sender, false, |s| {
                    if let Some(c) = s
                        .note_mut(&note)
                        .and_then(|n| n.checklist.iter_mut().find(|c| c.id == item))
                    {
                        c.text.clone_from(&text);
                    }
                });
            }
            NotesMenuWidgetInput::RemoveCheckItem { note, item } => {
                self.edit(&sender, true, |s| {
                    if let Some(n) = s.note_mut(&note) {
                        n.checklist.retain(|c| c.id != item);
                    }
                });
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
            s.active_note_count(),
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

    // Notes list — rebuilt; card edits are debounced per card.
    model.notes.sync(s, model.show_archive, sender);

    model.todos.sync(s, sender);
}
