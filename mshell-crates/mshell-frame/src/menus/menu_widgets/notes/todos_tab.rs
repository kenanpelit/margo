//! The Notes Hub "Todos" tab: a page picker + page actions, the
//! show-completed filter, copy/export, and the reorderable todo rows
//! (grip drag or Alt+↑/↓, checkbox, title, expandable details, delete).

use super::model::{ExportFormat, NotesState, Todo};
use super::notes_menu_widget::{NotesMenuWidgetInput, NotesMenuWidgetModel, PageNameMode};
use relm4::gtk::prelude::*;
use relm4::gtk::{gdk, glib};
use relm4::{ComponentSender, gtk};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use super::notes_menu_widget::SAVE_DEBOUNCE;

type Sender = ComponentSender<NotesMenuWidgetModel>;

pub(super) struct TodosTab {
    pub(super) root: gtk::Box,
    pub(super) scroller: gtk::ScrolledWindow,
    list: gtk::ListBox,
    pages: gtk::StringList,
    page_ids: Rc<RefCell<Vec<String>>>,
    page_drop: gtk::DropDown,
    /// Set while the picker is refilled, so it doesn't echo a selection.
    page_quiet: Rc<Cell<bool>>,
    delete_page: gtk::Button,
    rename_page: gtk::Button,
    name_revealer: gtk::Revealer,
    name_entry: gtk::Entry,
    name_mode: Rc<Cell<PageNameMode>>,
    show_done: gtk::ToggleButton,
    show_done_quiet: Rc<Cell<bool>>,
    /// Todo ids whose details are expanded (kept across rebuilds).
    expanded: Rc<RefCell<HashSet<String>>>,
}

fn icon_button(icon: &str, name: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.add_css_class("ok-button-flat");
    b.add_css_class("notes-tool-btn");
    mshell_common::a11y::name(&b, name);
    b
}

impl TodosTab {
    pub(super) fn new(sender: &Sender) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);

        // ── Page row: picker + page actions + view / export ─────────
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        toolbar.add_css_class("notes-toolbar");
        let pages = gtk::StringList::new(&[]);
        let page_drop = gtk::DropDown::new(Some(pages.clone()), None::<gtk::Expression>);
        page_drop.set_hexpand(true);
        page_drop.set_tooltip_text(Some("Todo page"));
        page_drop.update_property(&[gtk::accessible::Property::Label("Todo page")]);
        let page_ids: Rc<RefCell<Vec<String>>> = Rc::default();
        let page_quiet = Rc::new(Cell::new(false));
        {
            let ids = page_ids.clone();
            let quiet = page_quiet.clone();
            let s = sender.clone();
            page_drop.connect_selected_notify(move |d| {
                if quiet.get() {
                    return;
                }
                if let Some(id) = ids.borrow().get(d.selected() as usize) {
                    s.input(NotesMenuWidgetInput::SelectPage(id.clone()));
                }
            });
        }
        toolbar.append(&page_drop);

        let add_page = icon_button("list-add-symbolic", "New page");
        let rename_page = icon_button("document-edit-symbolic", "Rename page");
        let delete_page = icon_button(
            "user-trash-symbolic",
            "Delete page (its todos move to General)",
        );
        let s = sender.clone();
        add_page.connect_clicked(move |_| {
            s.input(NotesMenuWidgetInput::BeginPageName(PageNameMode::New));
        });
        let s = sender.clone();
        rename_page.connect_clicked(move |_| {
            s.input(NotesMenuWidgetInput::BeginPageName(PageNameMode::Rename));
        });
        let s = sender.clone();
        delete_page.connect_clicked(move |_| s.input(NotesMenuWidgetInput::DeleteCurrentPage));
        toolbar.append(&add_page);
        toolbar.append(&rename_page);
        toolbar.append(&delete_page);

        let show_done = gtk::ToggleButton::new();
        show_done.set_icon_name("object-select-symbolic");
        show_done.add_css_class("ok-button-flat");
        show_done.add_css_class("notes-tool-btn");
        mshell_common::a11y::name(&show_done, "Show completed todos");
        let show_done_quiet = Rc::new(Cell::new(false));
        {
            let quiet = show_done_quiet.clone();
            let s = sender.clone();
            show_done.connect_toggled(move |b| {
                if !quiet.get() {
                    s.input(NotesMenuWidgetInput::SetShowCompleted(b.is_active()));
                }
            });
        }
        toolbar.append(&show_done);
        toolbar.append(&export_menu(sender));
        root.append(&toolbar);

        // ── Inline page-name entry (new / rename) ───────────────────
        let name_entry = gtk::Entry::new();
        name_entry.add_css_class("notes-add");
        name_entry.set_placeholder_text(Some("Page name — Enter to save, Esc to cancel"));
        let name_revealer = gtk::Revealer::new();
        name_revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        name_revealer.set_child(Some(&name_entry));
        let name_mode = Rc::new(Cell::new(PageNameMode::New));
        {
            let s = sender.clone();
            let mode = name_mode.clone();
            name_entry.connect_activate(move |e| {
                s.input(NotesMenuWidgetInput::CommitPageName(
                    mode.get(),
                    e.text().to_string(),
                ));
            });
            let keys = gtk::EventControllerKey::new();
            let s = sender.clone();
            keys.connect_key_pressed(move |_, key, _, _| {
                if key == gdk::Key::Escape {
                    s.input(NotesMenuWidgetInput::CancelPageName);
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            name_entry.add_controller(keys);
        }
        root.append(&name_revealer);

        // ── Add todo ────────────────────────────────────────────────
        let todo_entry = gtk::Entry::new();
        todo_entry.add_css_class("notes-add");
        todo_entry.set_placeholder_text(Some("Add a todo and press Enter…"));
        let s = sender.clone();
        todo_entry.connect_activate(move |entry| {
            let text = entry.text().to_string();
            if !text.trim().is_empty() {
                entry.set_text("");
                s.input(NotesMenuWidgetInput::AddTodo(text));
            }
        });
        root.append(&todo_entry);

        let list = gtk::ListBox::new();
        list.add_css_class("notes-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .min_content_height(0)
            .child(&list)
            .build();
        root.append(&scroller);

        Self {
            root,
            scroller,
            list,
            pages,
            page_ids,
            page_drop,
            page_quiet,
            delete_page,
            rename_page,
            name_revealer,
            name_entry,
            name_mode,
            show_done,
            show_done_quiet,
            expanded: Rc::default(),
        }
    }

    /// Open the inline name entry for a new page or a rename.
    pub(super) fn begin_page_name(&self, mode: PageNameMode, current_name: &str) {
        self.name_mode.set(mode);
        self.name_entry.set_text(match mode {
            PageNameMode::New => "",
            PageNameMode::Rename => current_name,
        });
        self.name_revealer.set_reveal_child(true);
        self.name_entry.grab_focus();
    }

    pub(super) fn end_page_name(&self) {
        self.name_revealer.set_reveal_child(false);
        self.name_entry.set_text("");
    }

    pub(super) fn clipboard(&self) -> gdk::Clipboard {
        self.root.clipboard()
    }

    pub(super) fn sync(&self, s: &NotesState, sender: &Sender) {
        let current = s.current_page();
        // Page picker: refill only when the page list changed, so an
        // unrelated edit doesn't flicker the dropdown.
        let pages = s.pages();
        let ids: Vec<String> = pages.iter().map(|(id, _)| (*id).to_string()).collect();
        let names: Vec<String> = pages
            .iter()
            .map(|(id, name)| {
                let (open, _) = s.page_counts(id);
                if open > 0 {
                    format!("{name} ({open})")
                } else {
                    (*name).to_string()
                }
            })
            .collect();
        self.page_quiet.set(true);
        let shown: Vec<String> = (0..self.pages.n_items())
            .filter_map(|i| self.pages.string(i).map(|g| g.to_string()))
            .collect();
        if shown != names {
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            self.pages.splice(0, self.pages.n_items(), &refs);
        }
        *self.page_ids.borrow_mut() = ids.clone();
        let idx = ids.iter().position(|i| i == current).unwrap_or(0) as u32;
        if self.page_drop.selected() != idx {
            self.page_drop.set_selected(idx);
        }
        self.page_quiet.set(false);
        let named = current != super::model::GENERAL_PAGE_ID;
        self.delete_page.set_sensitive(named);
        self.rename_page.set_sensitive(named);

        self.show_done_quiet.set(true);
        self.show_done.set_active(s.show_completed);
        self.show_done_quiet.set(false);
        self.show_done.set_tooltip_text(Some(if s.show_completed {
            "Hide completed todos"
        } else {
            "Show completed todos"
        }));

        clear_listbox(&self.list);
        let todos = s.page_todos(current);
        if todos.is_empty() {
            let (_, total) = s.page_counts(current);
            let text = if total > 0 {
                "All done — completed todos are hidden"
            } else {
                "No todos on this page"
            };
            self.list.append(&placeholder_row(text));
        }
        self.expanded
            .borrow_mut()
            .retain(|id| s.todos.iter().any(|t| &t.id == id));
        for todo in todos {
            self.list.append(&self.todo_row(todo, sender));
        }
    }

    fn todo_row(&self, todo: &Todo, sender: &Sender) -> gtk::ListBoxRow {
        let row = gtk::ListBoxRow::new();
        row.set_activatable(false);
        row.set_selectable(false);
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.add_css_class("notes-todo-row");
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 4);

        let grip = gtk::Image::from_icon_name("list-drag-handle-symbolic");
        grip.add_css_class("reorder-grip");
        grip.set_tooltip_text(Some("Drag to reorder (or Alt+↑/↓ in the text)"));
        let s = sender.clone();
        let id = todo.id.clone();
        mshell_settings::reorder_dnd::attach_grip_drag(&grip, &row, move |delta| {
            s.input(NotesMenuWidgetInput::MoveTodo {
                id: id.clone(),
                delta,
            });
        });
        line.append(&grip);

        let check = gtk::CheckButton::new();
        check.set_active(todo.completed);
        check.set_valign(gtk::Align::Center);
        mshell_common::a11y::name(&check, "Done");
        let s = sender.clone();
        let id = todo.id.clone();
        check.connect_toggled(move |_| s.input(NotesMenuWidgetInput::ToggleTodo(id.clone())));
        line.append(&check);

        let entry = gtk::Entry::builder()
            .text(&todo.text)
            .hexpand(true)
            .css_classes(vec!["notes-todo-entry"])
            .build();
        if todo.completed {
            entry.add_css_class("completed");
        }
        debounce_entry(&entry, sender, todo.id.clone());
        // Alt+↑/↓: the keyboard path for the grip drag (DESIGN §13.9).
        let keys = gtk::EventControllerKey::new();
        let s = sender.clone();
        let id = todo.id.clone();
        keys.connect_key_pressed(move |_, key, _, mods| {
            if !mods.contains(gdk::ModifierType::ALT_MASK) {
                return glib::Propagation::Proceed;
            }
            let delta = match key {
                gdk::Key::Up => -1,
                gdk::Key::Down => 1,
                _ => return glib::Propagation::Proceed,
            };
            s.input(NotesMenuWidgetInput::MoveTodo {
                id: id.clone(),
                delta,
            });
            glib::Propagation::Stop
        });
        entry.add_controller(keys);
        line.append(&entry);

        let expanded = self.expanded.borrow().contains(&todo.id);
        let details_btn = gtk::ToggleButton::new();
        details_btn.set_icon_name(if todo.details.trim().is_empty() {
            "pan-down-symbolic"
        } else {
            "document-properties-symbolic"
        });
        details_btn.add_css_class("ok-button-flat");
        details_btn.set_active(expanded);
        mshell_common::a11y::name(&details_btn, "Details");
        line.append(&details_btn);

        let delete = gtk::Button::from_icon_name("trash-symbolic");
        delete.add_css_class("ok-button-flat");
        mshell_common::a11y::name(&delete, "Delete todo");
        let s = sender.clone();
        let id = todo.id.clone();
        delete.connect_clicked(move |_| s.input(NotesMenuWidgetInput::DeleteTodo(id.clone())));
        line.append(&delete);
        column.append(&line);

        // ── Details (revealed) ──────────────────────────────────────
        let buf = gtk::TextBuffer::new(None);
        buf.set_text(&todo.details);
        let view = gtk::TextView::with_buffer(&buf);
        view.add_css_class("notes-todo-details");
        view.set_wrap_mode(gtk::WrapMode::WordChar);
        view.set_top_margin(4);
        view.set_bottom_margin(4);
        view.set_left_margin(8);
        view.set_right_margin(8);
        view.set_height_request(56);
        view.update_property(&[gtk::accessible::Property::Label("Todo details")]);
        debounce_details(&buf, sender, todo.id.clone());
        let revealer = gtk::Revealer::new();
        revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        revealer.set_child(Some(&view));
        revealer.set_reveal_child(expanded);
        let set = self.expanded.clone();
        let id = todo.id.clone();
        let rv = revealer.clone();
        details_btn.connect_toggled(move |b| {
            let open = b.is_active();
            rv.set_reveal_child(open);
            if open {
                set.borrow_mut().insert(id.clone());
            } else {
                set.borrow_mut().remove(&id);
            }
        });
        column.append(&revealer);

        row.set_child(Some(&column));
        row
    }
}

/// The ⤓ menu: copy the current page or save every page to a file.
fn export_menu(sender: &Sender) -> gtk::MenuButton {
    let pop_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
    pop_box.add_css_class("notes-popover-list");
    let popover = gtk::Popover::new();
    let item = |label: &str, input: fn() -> NotesMenuWidgetInput| {
        let b = gtk::Button::with_label(label);
        b.add_css_class("ok-button-flat");
        if let Some(l) = b.child().and_downcast::<gtk::Label>() {
            l.set_xalign(0.0);
        }
        let s = sender.clone();
        let pop = popover.clone();
        b.connect_clicked(move |_| {
            pop.popdown();
            s.input(input());
        });
        pop_box.append(&b);
    };
    item("Copy page as Markdown", || {
        NotesMenuWidgetInput::CopyTodos(ExportFormat::Markdown)
    });
    item("Copy page as plain text", || {
        NotesMenuWidgetInput::CopyTodos(ExportFormat::Text)
    });
    item("Save all pages to Documents (.md)", || {
        NotesMenuWidgetInput::ExportTodos(ExportFormat::Markdown)
    });
    item("Save all pages to Documents (.txt)", || {
        NotesMenuWidgetInput::ExportTodos(ExportFormat::Text)
    });
    popover.set_child(Some(&pop_box));
    let button = gtk::MenuButton::new();
    button.set_icon_name("document-send-symbolic");
    button.add_css_class("ok-button-flat");
    button.add_css_class("notes-tool-btn");
    button.set_popover(Some(&popover));
    mshell_common::a11y::name(&button, "Copy or export todos");
    button
}

fn debounce_entry(entry: &gtk::Entry, sender: &Sender, id: String) {
    let token = Rc::new(Cell::new(0u64));
    let s = sender.clone();
    entry.connect_changed(move |e| {
        let next = token.get().wrapping_add(1);
        token.set(next);
        let (s, id, e, token) = (s.clone(), id.clone(), e.clone(), token.clone());
        glib::timeout_add_local_once(SAVE_DEBOUNCE, move || {
            if token.get() == next {
                let _ = s.input_sender().send(NotesMenuWidgetInput::UpdateTodo {
                    id,
                    text: e.text().to_string(),
                });
            }
        });
    });
}

fn debounce_details(buf: &gtk::TextBuffer, sender: &Sender, id: String) {
    let token = Rc::new(Cell::new(0u64));
    let s = sender.clone();
    buf.connect_changed(move |b| {
        let next = token.get().wrapping_add(1);
        token.set(next);
        let (s, id, b, token) = (s.clone(), id.clone(), b.clone(), token.clone());
        glib::timeout_add_local_once(SAVE_DEBOUNCE, move || {
            if token.get() == next {
                let details = b.text(&b.start_iter(), &b.end_iter(), false).to_string();
                let _ = s
                    .input_sender()
                    .send(NotesMenuWidgetInput::UpdateTodoDetails { id, details });
            }
        });
    });
}

pub(super) fn clear_listbox(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

pub(super) fn placeholder_row(text: &str) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_selectable(false);
    let label = gtk::Label::new(Some(text));
    label.add_css_class("notes-empty");
    label.set_xalign(0.0);
    row.set_child(Some(&label));
    row
}
