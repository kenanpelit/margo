//! Settings → Widgets → Notes Hub — the Markdown "Files" tab: the notes
//! folder, git history, AI capture and how many notes a question sends.
//! Composed with the generic per-menu geometry page (position / width /
//! height), the same way Lyrics is.

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, NotesConfigStoreFields};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, EditableExt, EntryExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct NotesSettingsModel {
    folder: String,
    git_history: bool,
    ai_capture: bool,
    context_notes: f64,
    git_missing: bool,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum NotesSettingsInput {
    FolderCommitted(String),
    FolderEffect(String),
    GitChanged(bool),
    GitEffect(bool),
    AiCaptureChanged(bool),
    AiCaptureEffect(bool),
    ContextChanged(f64),
    ContextEffect(u32),
}

#[derive(Debug)]
pub(crate) enum NotesSettingsOutput {}

pub(crate) struct NotesSettingsInit {}

fn row_text(title: &str, help: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.set_valign(gtk::Align::Center);
    b.set_hexpand(true);
    let t = gtk::Label::new(Some(title));
    t.add_css_class("label-medium-bold");
    t.set_halign(gtk::Align::Start);
    b.append(&t);
    let h = gtk::Label::new(Some(help));
    h.add_css_class("label-small");
    h.set_halign(gtk::Align::Start);
    h.set_xalign(0.0);
    h.set_wrap(true);
    h.set_natural_wrap_mode(gtk::NaturalWrapMode::None);
    b.append(&h);
    b
}

#[relm4::component(pub)]
impl Component for NotesSettingsModel {
    type CommandOutput = ();
    type Input = NotesSettingsInput;
    type Output = NotesSettingsOutput;
    type Init = NotesSettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_propagate_natural_height: false,
            set_propagate_natural_width: false,
            set_hexpand: true,
            set_vexpand: true,

            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Markdown notes (Files tab)",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        append: &row_text(
                            "Notes folder",
                            "A folder of .md files (one note per file, the first # heading is its title; captures go to Inbox.md). Blank turns the Files tab off. Press Enter to apply.",
                        ),
                        #[name = "folder_entry"]
                        gtk::Entry {
                            set_valign: gtk::Align::Center,
                            set_width_chars: 22,
                            set_placeholder_text: Some("~/notes"),
                            #[watch]
                            #[block_signal(folder_handler)]
                            set_text: &model.folder,
                            connect_activate[sender] => move |e| {
                                sender.input(NotesSettingsInput::FolderCommitted(e.text().to_string()));
                            } @folder_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        append: &row_text(
                            "Git history",
                            "Commit the folder a few seconds after each change (git init on first use; never pushes, never signs). Refused for the home folder itself.",
                        ),
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            set_sensitive: !model.git_missing,
                            #[watch]
                            set_tooltip_text: model.git_missing.then_some("git is not installed"),
                            #[watch]
                            #[block_signal(git_handler)]
                            set_active: model.git_history,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(NotesSettingsInput::GitChanged(v));
                                glib::Propagation::Proceed
                            } @git_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        append: &row_text(
                            "AI capture",
                            "Let the AI provider (Settings → AI) tidy a capture into one todo line before it lands in Inbox.md. If the AI fails, the raw text is kept.",
                        ),
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(ai_handler)]
                            set_active: model.ai_capture,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(NotesSettingsInput::AiCaptureChanged(v));
                                glib::Propagation::Proceed
                            } @ai_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        append: &row_text(
                            "Notes sent with a question",
                            "How many of the best-matching notes \"Ask your notes\" sends to the AI provider. Only those notes leave the machine.",
                        ),
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_numeric: true,
                            set_adjustment: &gtk::Adjustment::new(4.0, 1.0, 12.0, 1.0, 2.0, 0.0),
                            #[watch]
                            #[block_signal(ctx_handler)]
                            set_value: model.context_notes,
                            connect_value_changed[sender] => move |sb| {
                                sender.input(NotesSettingsInput::ContextChanged(sb.value()));
                            } @ctx_handler,
                        },
                    },
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut effects = EffectScope::new();
        let s = sender.clone();
        effects.push(move |_| {
            s.input(NotesSettingsInput::FolderEffect(
                config_manager().config().notes().folder().get(),
            ));
        });
        let s = sender.clone();
        effects.push(move |_| {
            s.input(NotesSettingsInput::GitEffect(
                config_manager().config().notes().git_history().get(),
            ));
        });
        let s = sender.clone();
        effects.push(move |_| {
            s.input(NotesSettingsInput::AiCaptureEffect(
                config_manager().config().notes().ai_capture().get(),
            ));
        });
        let s = sender.clone();
        effects.push(move |_| {
            s.input(NotesSettingsInput::ContextEffect(
                config_manager().config().notes().ai_context_notes().get(),
            ));
        });
        let notes = config_manager().config().notes().get_untracked();
        let model = NotesSettingsModel {
            folder: notes.folder,
            git_history: notes.git_history,
            ai_capture: notes.ai_capture,
            context_notes: f64::from(notes.ai_context_notes.clamp(1, 12)),
            git_missing: !margo_fs::path::on_path("git"),
            _effects: effects,
        };
        let widgets = view_output!();
        // Apply the folder on focus-out too, not only Enter.
        let focus = gtk::EventControllerFocus::new();
        let (s, e) = (sender.clone(), widgets.folder_entry.clone());
        focus.connect_leave(move |_| {
            s.input(NotesSettingsInput::FolderCommitted(e.text().to_string()));
        });
        widgets.folder_entry.add_controller(focus);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            NotesSettingsInput::FolderCommitted(v) => {
                let v = v.trim().to_string();
                if v != self.folder {
                    config_manager().update_config(|c| c.notes.folder = v);
                }
            }
            NotesSettingsInput::FolderEffect(v) => self.folder = v,
            NotesSettingsInput::GitChanged(v) => {
                config_manager().update_config(|c| c.notes.git_history = v);
            }
            NotesSettingsInput::GitEffect(v) => self.git_history = v,
            NotesSettingsInput::AiCaptureChanged(v) => {
                config_manager().update_config(|c| c.notes.ai_capture = v);
            }
            NotesSettingsInput::AiCaptureEffect(v) => self.ai_capture = v,
            NotesSettingsInput::ContextChanged(v) => {
                let n = (v as u32).clamp(1, 12);
                config_manager().update_config(|c| c.notes.ai_context_notes = n);
            }
            NotesSettingsInput::ContextEffect(v) => self.context_notes = f64::from(v.clamp(1, 12)),
        }
    }
}
