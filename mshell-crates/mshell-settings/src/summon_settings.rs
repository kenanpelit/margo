//! Settings → Tag Apps.
//!
//! A friendly front-end for margo's `summon` keybinds — the "bring this
//! app to the current tag, or launch it if it isn't open" gesture (the
//! in-compositor port of mango-here). Each row is one binding:
//!
//! ```text
//! bind = <mods>,<key>,summon,<app-id regex>,<title|none>,<launch command>
//! ```
//!
//! These are ordinary keybinds, so this page shares the keybinds editor's
//! storage: on save it reloads the full bind set, swaps in just the
//! `summon` lines, and rewrites `binds.conf` — anything you set in
//! Settings → Keybinds is preserved, and vice-versa. `mctl reload` applies
//! it live (debounced so a burst of edits is one reload).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

use crate::keybinds_settings::{Bind, load_binds, persist};

/// The live widgets for one summon row — read back at save time so typing
/// never triggers a model rebuild (cursor/focus stay put).
#[derive(Clone)]
struct Row {
    mods: gtk::Entry,
    key: gtk::Entry,
    appid: gtk::Entry,
    title: gtk::Entry,
    cmd: gtk::Entry,
    container: gtk::Box,
}

pub(crate) struct SummonSettingsModel {
    rows: Rc<RefCell<Vec<Row>>>,
    list: gtk::Box,
    /// Debounce guard — coalesces a burst of field edits into one write.
    armed: bool,
}

#[derive(Debug)]
pub(crate) enum SummonSettingsInput {
    Add,
    /// A field changed / lost focus, or a row was added/removed — schedule
    /// a debounced write.
    RequestPersist,
    /// Debounce fired — do the actual write + reload.
    DoPersist,
}

#[derive(Debug)]
pub(crate) enum SummonSettingsOutput {}

pub(crate) struct SummonSettingsInit {}

#[relm4::component(pub(crate))]
impl Component for SummonSettingsModel {
    type CommandOutput = ();
    type Input = SummonSettingsInput;
    type Output = SummonSettingsOutput;
    type Init = SummonSettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_hexpand: true,
            set_vexpand: true,

            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                gtk::Box {
                    add_css_class: "settings-hero",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Start,
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("view-app-grid-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Tag Apps (Summon)",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Bind a key to summon an app to the current tag — or launch it if it isn't open yet. The compositor's “summon” action; these are saved as keybinds.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "App ID is a regex matched against a window's app-id — “^Spotify$” for an exact match, “^(discord|WebCord)$” for alternatives. Title is optional (leave blank). Command runs only when no match is open.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },

                // Column headers.
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    gtk::Label { set_label: "Mod", set_width_request: 80, set_xalign: 0.0, add_css_class: "label-small" },
                    gtk::Label { set_label: "Key", set_width_request: 60, set_xalign: 0.0, add_css_class: "label-small" },
                    gtk::Label { set_label: "App ID", set_hexpand: true, set_xalign: 0.0, add_css_class: "label-small" },
                    gtk::Label { set_label: "Title", set_width_request: 110, set_xalign: 0.0, add_css_class: "label-small" },
                    gtk::Label { set_label: "Launch command", set_hexpand: true, set_xalign: 0.0, add_css_class: "label-small" },
                    gtk::Label { set_label: "", set_width_request: 36 },
                },

                #[name = "list"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 8,
                },

                gtk::Button {
                    set_halign: gtk::Align::Start,
                    add_css_class: "ok-button-surface",
                    set_label: "＋ Add app",
                    connect_clicked => SummonSettingsInput::Add,
                },
            },
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let widgets = view_output!();
        let rows: Rc<RefCell<Vec<Row>>> = Rc::new(RefCell::new(Vec::new()));

        // One row per existing summon bind.
        for b in load_binds().into_iter().filter(Bind::is_summon) {
            let (appid, title, spawn) = b.summon_parts();
            let row = build_row(
                &widgets.list,
                &rows,
                &sender,
                &b.mods(),
                b.key_name(),
                &appid,
                &title,
                &spawn,
            );
            widgets.list.append(&row.container);
            rows.borrow_mut().push(row);
        }

        let model = SummonSettingsModel {
            rows,
            list: widgets.list.clone(),
            armed: false,
        };
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            SummonSettingsInput::Add => {
                let row = build_row(&self.list, &self.rows, &sender, "alt", "", "", "", "");
                self.list.append(&row.container);
                self.rows.borrow_mut().push(row);
                // No write yet — the blank row is skipped until it's filled.
            }
            SummonSettingsInput::RequestPersist => {
                if !self.armed {
                    self.armed = true;
                    let s = sender.clone();
                    gtk::glib::timeout_add_local_once(Duration::from_millis(450), move || {
                        s.input(SummonSettingsInput::DoPersist);
                    });
                }
            }
            SummonSettingsInput::DoPersist => {
                self.armed = false;
                self.write();
            }
        }
    }
}

impl SummonSettingsModel {
    /// Reload the full bind set, replace just the summon lines with the
    /// page's current rows, and rewrite. Empty rows are skipped.
    fn write(&self) {
        let summons: Vec<Bind> = self
            .rows
            .borrow()
            .iter()
            .filter_map(|r| {
                let appid = r.appid.text().to_string();
                let key = r.key.text().to_string();
                let cmd = r.cmd.text().to_string();
                if appid.trim().is_empty() && key.trim().is_empty() && cmd.trim().is_empty() {
                    return None;
                }
                let mods = r.mods.text().to_string();
                let mods = if mods.trim().is_empty() {
                    "alt"
                } else {
                    mods.trim()
                };
                Some(Bind::new_summon(mods, &key, &appid, &r.title.text(), &cmd))
            })
            .collect();

        let mut all = load_binds();
        all.retain(|b| !b.is_summon());
        all.extend(summons);
        persist(&all);
    }
}

/// Build one row of entries + a delete button, wired to debounce-persist on
/// focus-out and to remove itself on delete.
#[allow(clippy::too_many_arguments)]
fn build_row(
    list: &gtk::Box,
    rows: &Rc<RefCell<Vec<Row>>>,
    sender: &ComponentSender<SummonSettingsModel>,
    mods: &str,
    key: &str,
    appid: &str,
    title: &str,
    cmd: &str,
) -> Row {
    let container = gtk::Box::new(gtk::Orientation::Horizontal, 8);

    let mk = |text: &str, placeholder: &str, width: i32, expand: bool| {
        let e = gtk::Entry::new();
        e.set_text(text);
        e.set_placeholder_text(Some(placeholder));
        if width > 0 {
            e.set_width_request(width);
        }
        e.set_hexpand(expand);
        // Persist when the field loses focus or the user hits Enter.
        let s = sender.clone();
        let ec = gtk::EventControllerFocus::new();
        ec.connect_leave(move |_| s.input(SummonSettingsInput::RequestPersist));
        e.add_controller(ec);
        let s2 = sender.clone();
        e.connect_activate(move |_| s2.input(SummonSettingsInput::RequestPersist));
        e
    };

    let mods_e = mk(mods, "alt", 80, false);
    let key_e = mk(key, "1", 60, false);
    let appid_e = mk(appid, "^Spotify$", 0, true);
    let title_e = mk(title, "none", 110, false);
    let cmd_e = mk(cmd, "uwsm app -- …", 0, true);

    container.append(&mods_e);
    container.append(&key_e);
    container.append(&appid_e);
    container.append(&title_e);
    container.append(&cmd_e);

    let del = gtk::Button::from_icon_name("user-trash-symbolic");
    del.add_css_class("flat");
    del.set_valign(gtk::Align::Center);
    del.set_tooltip_text(Some("Remove"));
    {
        let rows = rows.clone();
        let list = list.clone();
        let container_c = container.clone();
        let s = sender.clone();
        del.connect_clicked(move |_| {
            list.remove(&container_c);
            rows.borrow_mut().retain(|r| r.container != container_c);
            s.input(SummonSettingsInput::RequestPersist);
        });
    }
    container.append(&del);

    Row {
        mods: mods_e,
        key: key_e,
        appid: appid_e,
        title: title_e,
        cmd: cmd_e,
        container,
    }
}
