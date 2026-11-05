//! The Notes Hub "Notes" tab: sticky cards with a theme-token tint,
//! pinning, a checklist under the body, and an archive (put away instead
//! of delete; restore or delete for good from the archive view).

use super::model::{CheckItem, Note, NoteColor, NotesState};
use super::notes_menu_widget::{NotesMenuWidgetInput, NotesMenuWidgetModel, SAVE_DEBOUNCE};
use super::todos_tab::{clear_listbox, placeholder_row};
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::{ComponentSender, gtk};
use std::cell::Cell;
use std::rc::Rc;

type Sender = ComponentSender<NotesMenuWidgetModel>;

pub(super) struct NotesTab {
    pub(super) root: gtk::Box,
    pub(super) scroller: gtk::ScrolledWindow,
    list: gtk::ListBox,
    add: gtk::Button,
    archive_toggle: gtk::ToggleButton,
    archive_quiet: Rc<Cell<bool>>,
}

impl NotesTab {
    pub(super) fn new(sender: &Sender) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        toolbar.add_css_class("notes-toolbar");
        let add = gtk::Button::with_label(" + Add note");
        add.set_css_classes(&["ok-button-surface", "ok-button-cell", "notes-add"]);
        add.set_hexpand(true);
        add.set_halign(gtk::Align::Start);
        let s = sender.clone();
        add.connect_clicked(move |_| s.input(NotesMenuWidgetInput::AddNote));
        toolbar.append(&add);

        let archive_toggle = gtk::ToggleButton::with_label("Archive");
        archive_toggle.add_css_class("ok-button-flat");
        archive_toggle.add_css_class("notes-archive-toggle");
        archive_toggle.set_tooltip_text(Some("Show archived notes"));
        let archive_quiet = Rc::new(Cell::new(false));
        {
            let quiet = archive_quiet.clone();
            let s = sender.clone();
            archive_toggle.connect_toggled(move |b| {
                if !quiet.get() {
                    s.input(NotesMenuWidgetInput::SetArchiveView(b.is_active()));
                }
            });
        }
        toolbar.append(&archive_toggle);
        root.append(&toolbar);

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
            add,
            archive_toggle,
            archive_quiet,
        }
    }

    pub(super) fn sync(&self, s: &NotesState, archive: bool, sender: &Sender) {
        let archived = s.notes.len() - s.active_note_count();
        self.archive_quiet.set(true);
        self.archive_toggle.set_active(archive);
        self.archive_quiet.set(false);
        self.archive_toggle.set_label(&if archived > 0 {
            format!("Archive ({archived})")
        } else {
            "Archive".to_string()
        });
        self.archive_toggle.set_tooltip_text(Some(if archive {
            "Back to notes"
        } else {
            "Show archived notes"
        }));
        self.add.set_sensitive(!archive);

        clear_listbox(&self.list);
        let notes = s.visible_notes(archive);
        if notes.is_empty() {
            self.list.append(&placeholder_row(if archive {
                "The archive is empty"
            } else {
                "No notes yet"
            }));
        }
        for note in notes {
            let row = if archive {
                archived_row(note, sender)
            } else {
                note_row(note, sender)
            };
            self.list.append(&row);
        }
    }
}

fn card(note: &Note) -> (gtk::ListBoxRow, gtk::Box) {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_selectable(false);
    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .css_classes(vec!["notes-card", note.color.css_class()])
        .build();
    row.set_child(Some(&outer));
    (row, outer)
}

fn flat_icon(icon: &str, name: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.add_css_class("ok-button-flat");
    mshell_common::a11y::name(&b, name);
    b
}

fn note_row(note: &Note, sender: &Sender) -> gtk::ListBoxRow {
    let (row, outer) = card(note);

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    let title_entry = gtk::Entry::builder()
        .text(&note.title)
        .placeholder_text("Title")
        .hexpand(true)
        .css_classes(vec!["notes-card-title"])
        .build();
    header.append(&title_entry);

    let (done, total) = note.checklist_progress();
    if total > 0 {
        let progress = gtk::Label::new(Some(&format!("{done}/{total}")));
        progress.add_css_class("notes-card-meta");
        progress.set_tooltip_text(Some("Checklist items done"));
        header.append(&progress);
    }

    let pin = gtk::ToggleButton::new();
    pin.set_icon_name(if note.pinned {
        "starred-symbolic"
    } else {
        "non-starred-symbolic"
    });
    pin.add_css_class("ok-button-flat");
    pin.set_active(note.pinned);
    if note.pinned {
        pin.add_css_class("pinned");
    }
    mshell_common::a11y::name(&pin, if note.pinned { "Unpin" } else { "Pin to top" });
    let s = sender.clone();
    let id = note.id.clone();
    pin.connect_toggled(move |b| {
        s.input(NotesMenuWidgetInput::SetPinned {
            id: id.clone(),
            pinned: b.is_active(),
        });
    });
    header.append(&pin);
    header.append(&colour_menu(note, sender));

    let check_btn = flat_icon("checkbox-checked-symbolic", "Add a checklist item");
    header.append(&check_btn);

    let archive = flat_icon("folder-download-symbolic", "Archive note");
    let s = sender.clone();
    let id = note.id.clone();
    archive.connect_clicked(move |_| s.input(NotesMenuWidgetInput::ArchiveNote(id.clone())));
    header.append(&archive);
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
    body_view.set_height_request(if total > 0 { 40 } else { 80 });
    body_view.update_property(&[gtk::accessible::Property::Label("Note text")]);
    outer.append(&body_view);

    // Title + body share one debounce token so a burst of typing is one save.
    let token = Rc::new(Cell::new(0u64));
    let push_update = {
        let (token, sender, id) = (token.clone(), sender.clone(), note.id.clone());
        let (title_entry, body_buf) = (title_entry.clone(), body_buf.clone());
        move || {
            let next = token.get().wrapping_add(1);
            token.set(next);
            let (sender, id, token) = (sender.clone(), id.clone(), token.clone());
            let (title_entry, body_buf) = (title_entry.clone(), body_buf.clone());
            glib::timeout_add_local_once(SAVE_DEBOUNCE, move || {
                if token.get() == next {
                    let title = title_entry.text().to_string();
                    let body = body_buf
                        .text(&body_buf.start_iter(), &body_buf.end_iter(), false)
                        .to_string();
                    let _ = sender
                        .input_sender()
                        .send(NotesMenuWidgetInput::UpdateNote { id, title, body });
                }
            });
        }
    };
    let push_for_title = push_update.clone();
    title_entry.connect_changed(move |_| push_for_title());
    body_buf.connect_changed(move |_| push_update());

    // ── Checklist ───────────────────────────────────────────────
    let items = gtk::Box::new(gtk::Orientation::Vertical, 0);
    items.add_css_class("notes-checklist");
    for item in &note.checklist {
        items.append(&check_row(&note.id, item, sender));
    }
    outer.append(&items);

    let add_item = gtk::Entry::new();
    add_item.add_css_class("notes-check-add");
    add_item.set_placeholder_text(Some("Add an item and press Enter…"));
    let s = sender.clone();
    let id = note.id.clone();
    add_item.connect_activate(move |e| {
        let text = e.text().to_string();
        if !text.trim().is_empty() {
            e.set_text("");
            s.input(NotesMenuWidgetInput::AddCheckItem {
                note: id.clone(),
                text,
            });
        }
    });
    let add_reveal = gtk::Revealer::new();
    add_reveal.set_transition_type(gtk::RevealerTransitionType::SlideDown);
    add_reveal.set_child(Some(&add_item));
    add_reveal.set_reveal_child(total > 0);
    let (rv, entry) = (add_reveal.clone(), add_item.clone());
    check_btn.connect_clicked(move |_| {
        rv.set_reveal_child(true);
        entry.grab_focus();
    });
    outer.append(&add_reveal);
    row
}

fn check_row(note_id: &str, item: &CheckItem, sender: &Sender) -> gtk::Box {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    line.add_css_class("notes-check-row");
    let check = gtk::CheckButton::new();
    check.set_active(item.done);
    check.set_valign(gtk::Align::Center);
    mshell_common::a11y::name(&check, "Done");
    let s = sender.clone();
    let (note, id) = (note_id.to_string(), item.id.clone());
    check.connect_toggled(move |b| {
        s.input(NotesMenuWidgetInput::SetCheckItem {
            note: note.clone(),
            item: id.clone(),
            done: b.is_active(),
        });
    });
    line.append(&check);

    let entry = gtk::Entry::builder()
        .text(&item.text)
        .hexpand(true)
        .css_classes(vec!["notes-todo-entry"])
        .build();
    if item.done {
        entry.add_css_class("completed");
    }
    let token = Rc::new(Cell::new(0u64));
    let s = sender.clone();
    let (note, id) = (note_id.to_string(), item.id.clone());
    entry.connect_changed(move |e| {
        let next = token.get().wrapping_add(1);
        token.set(next);
        let (s, note, id, e, token) = (
            s.clone(),
            note.clone(),
            id.clone(),
            e.clone(),
            token.clone(),
        );
        glib::timeout_add_local_once(SAVE_DEBOUNCE, move || {
            if token.get() == next {
                let _ = s
                    .input_sender()
                    .send(NotesMenuWidgetInput::UpdateCheckItem {
                        note,
                        item: id,
                        text: e.text().to_string(),
                    });
            }
        });
    });
    line.append(&entry);

    let remove = flat_icon("list-remove-symbolic", "Remove item");
    let s = sender.clone();
    let (note, id) = (note_id.to_string(), item.id.clone());
    remove.connect_clicked(move |_| {
        s.input(NotesMenuWidgetInput::RemoveCheckItem {
            note: note.clone(),
            item: id.clone(),
        });
    });
    line.append(&remove);
    line
}

/// The colour picker: a swatch button opening one swatch per theme role.
fn colour_menu(note: &Note, sender: &Sender) -> gtk::MenuButton {
    let popover = gtk::Popover::new();
    let grid = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    grid.add_css_class("notes-swatches");
    for colour in NoteColor::ALL {
        let b = gtk::Button::new();
        b.set_css_classes(&["notes-swatch", colour.css_class()]);
        if colour == note.color {
            b.add_css_class("selected");
            b.set_icon_name("object-select-symbolic");
        }
        mshell_common::a11y::name(&b, colour.label());
        let s = sender.clone();
        let id = note.id.clone();
        let pop = popover.clone();
        b.connect_clicked(move |_| {
            pop.popdown();
            s.input(NotesMenuWidgetInput::SetNoteColor {
                id: id.clone(),
                color: colour,
            });
        });
        grid.append(&b);
    }
    popover.set_child(Some(&grid));
    let button = gtk::MenuButton::new();
    button.set_icon_name("palette-symbolic");
    button.add_css_class("ok-button-flat");
    button.set_popover(Some(&popover));
    mshell_common::a11y::name(&button, &format!("Note colour: {}", note.color.label()));
    button
}

/// A read-only archived card: title, body and checklist as text, plus
/// Restore and Delete-for-good.
fn archived_row(note: &Note, sender: &Sender) -> gtk::ListBoxRow {
    let (row, outer) = card(note);
    outer.add_css_class("archived");
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    let title = gtk::Label::new(Some(if note.title.trim().is_empty() {
        "Untitled"
    } else {
        note.title.as_str()
    }));
    title.add_css_class("notes-card-title");
    title.set_hexpand(true);
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    header.append(&title);
    let restore = flat_icon("document-revert-symbolic", "Restore note");
    let s = sender.clone();
    let id = note.id.clone();
    restore.connect_clicked(move |_| s.input(NotesMenuWidgetInput::RestoreNote(id.clone())));
    header.append(&restore);
    let delete = flat_icon("user-trash-symbolic", "Delete note for good");
    let s = sender.clone();
    let id = note.id.clone();
    delete.connect_clicked(move |_| s.input(NotesMenuWidgetInput::DeleteNote(id.clone())));
    header.append(&delete);
    outer.append(&header);

    let mut text = note.body.trim().to_string();
    for item in &note.checklist {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(if item.done { "☑ " } else { "☐ " });
        text.push_str(item.text.trim());
    }
    if !text.is_empty() {
        let body = gtk::Label::new(Some(&text));
        body.add_css_class("notes-card-preview");
        body.set_xalign(0.0);
        body.set_wrap(true);
        body.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        body.set_lines(6);
        body.set_ellipsize(gtk::pango::EllipsizeMode::End);
        body.set_selectable(true);
        outer.append(&body);
    }
    row
}
