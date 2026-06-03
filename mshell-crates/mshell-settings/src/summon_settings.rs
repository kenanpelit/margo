//! Settings → Tags.
//!
//! Friendly front-ends for the two tag-related keybind families:
//!
//!   * **Summon apps** — `summon`: bring an app to the current tag, or
//!     launch it if it isn't open (the in-compositor port of mango-here).
//!     `bind = <mods>,<key>,summon,<app-id>,<title|none>,<command>`.
//!   * **Move window to a tag** — `tag` / `toggletag`: send (or toggle) the
//!     focused window onto a tag. `bind = <mods>,<key>,tag,<mask>`.
//!
//! Both are ordinary keybinds, so this page shares the keybinds editor's
//! storage: on save it reloads the full bind set, swaps in just the lines
//! it manages (summon + single-tag `tag`/`toggletag`), and rewrites
//! `binds.conf`. Anything in Settings → Keybinds — including multi-tag
//! masks like `tag,6` this page leaves alone — is preserved. `mctl reload`
//! applies it live (debounced).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

use crate::keybinds_settings::{Bind, load_binds, persist};

const TAG_CHOICES: [&str; 10] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "All"];
const ACTION_CHOICES: [&str; 2] = ["Move here", "Toggle"];

/// Summon row widgets, read back at save time.
#[derive(Clone)]
struct Row {
    mods: gtk::Entry,
    key: gtk::Entry,
    appid: gtk::Entry,
    title: gtk::Entry,
    cmd: gtk::Entry,
    container: gtk::Box,
}

/// Move-to-tag row widgets.
#[derive(Clone)]
struct TagRow {
    mods: gtk::Entry,
    key: gtk::Entry,
    action: gtk::DropDown,
    tag: gtk::DropDown,
    container: gtk::Box,
}

pub(crate) struct SummonSettingsModel {
    rows: Rc<RefCell<Vec<Row>>>,
    list: gtk::Box,
    tag_rows: Rc<RefCell<Vec<TagRow>>>,
    tag_list: gtk::Box,
    /// Debounce guard — coalesces a burst of field edits into one write.
    armed: bool,
}

#[derive(Debug)]
pub(crate) enum SummonSettingsInput {
    AddSummon,
    AddTag,
    RequestPersist,
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
                            set_label: "Tags",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Tag-related keybinds: summon an app to the current tag, and move the focused window onto a tag. Saved as keybinds; live via mctl reload.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                // ── Summon apps ────────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Summon apps",
                    set_halign: gtk::Align::Start,
                },
                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "Bind a key to bring an app to the current tag — or launch it if it isn't open. App ID is a regex (“^Spotify$” exact, “^(discord|WebCord)$” alternatives). Title optional; command runs only when no match is open.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },
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
                    connect_clicked => SummonSettingsInput::AddSummon,
                },

                gtk::Separator { set_margin_top: 8 },

                // ── Move window to a tag ───────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Move window to a tag",
                    set_halign: gtk::Align::Start,
                },
                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "Bind a key to send the focused window to a tag (“Move here”) or add/remove it from a tag (“Toggle” — a window can live on several tags at once).",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    gtk::Label { set_label: "Mod", set_width_request: 110, set_xalign: 0.0, add_css_class: "label-small" },
                    gtk::Label { set_label: "Key", set_width_request: 60, set_xalign: 0.0, add_css_class: "label-small" },
                    gtk::Label { set_label: "Action", set_hexpand: true, set_xalign: 0.0, add_css_class: "label-small" },
                    gtk::Label { set_label: "Tag", set_hexpand: true, set_xalign: 0.0, add_css_class: "label-small" },
                    gtk::Label { set_label: "", set_width_request: 36 },
                },
                #[name = "tag_list"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 8,
                },
                gtk::Button {
                    set_halign: gtk::Align::Start,
                    add_css_class: "ok-button-surface",
                    set_label: "＋ Add tag key",
                    connect_clicked => SummonSettingsInput::AddTag,
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
        let tag_rows: Rc<RefCell<Vec<TagRow>>> = Rc::new(RefCell::new(Vec::new()));

        let binds = load_binds();

        for b in binds.iter().filter(|b| b.is_summon()) {
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

        for b in binds.iter().filter(|b| b.is_simple_tag_key()) {
            let row = build_tag_row(
                &widgets.tag_list,
                &tag_rows,
                &sender,
                &b.mods(),
                b.key_name(),
                b.action_str(),
                b.tag_mask(),
            );
            widgets.tag_list.append(&row.container);
            tag_rows.borrow_mut().push(row);
        }

        let model = SummonSettingsModel {
            rows,
            list: widgets.list.clone(),
            tag_rows,
            tag_list: widgets.tag_list.clone(),
            armed: false,
        };
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            SummonSettingsInput::AddSummon => {
                let row = build_row(&self.list, &self.rows, &sender, "alt", "", "", "", "");
                self.list.append(&row.container);
                self.rows.borrow_mut().push(row);
            }
            SummonSettingsInput::AddTag => {
                let row = build_tag_row(
                    &self.tag_list,
                    &self.tag_rows,
                    &sender,
                    "super+shift",
                    "",
                    "tag",
                    1,
                );
                self.tag_list.append(&row.container);
                self.tag_rows.borrow_mut().push(row);
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
    /// Reload the full bind set, replace just the lines this page manages
    /// (summon + single-tag `tag`/`toggletag`), and rewrite. Empty rows are
    /// skipped; multi-tag masks and all other binds are preserved.
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

        let tags: Vec<Bind> = self
            .tag_rows
            .borrow()
            .iter()
            .filter_map(|r| {
                let key = r.key.text().to_string();
                if key.trim().is_empty() {
                    return None;
                }
                let mods = r.mods.text().to_string();
                let mods = if mods.trim().is_empty() {
                    "super+shift"
                } else {
                    mods.trim()
                };
                let action = index_to_action(r.action.selected());
                let mask = tag_index_to_mask(r.tag.selected());
                Some(Bind::new_tag(mods, &key, action, mask))
            })
            .collect();

        let mut all = load_binds();
        all.retain(|b| !b.is_summon() && !b.is_simple_tag_key());
        all.extend(summons);
        all.extend(tags);
        persist(&all);
    }
}

// ── Conversions between the friendly Tag dropdown and the raw mask ──────────
fn tag_index_to_mask(i: u32) -> u32 {
    if i >= 9 { u32::MAX } else { 1 << i }
}
fn mask_to_tag_index(mask: u32) -> u32 {
    if mask == u32::MAX {
        9
    } else if mask != 0 && mask.is_power_of_two() {
        mask.trailing_zeros().min(8)
    } else {
        0
    }
}
fn index_to_action(i: u32) -> &'static str {
    if i == 1 { "toggletag" } else { "tag" }
}
fn action_to_index(a: &str) -> u32 {
    if a.eq_ignore_ascii_case("toggletag") {
        1
    } else {
        0
    }
}

/// An entry wired to debounce-persist on focus-out / Enter.
fn persisting_entry(
    text: &str,
    placeholder: &str,
    width: i32,
    expand: bool,
    sender: &ComponentSender<SummonSettingsModel>,
) -> gtk::Entry {
    let e = gtk::Entry::new();
    e.set_text(text);
    e.set_placeholder_text(Some(placeholder));
    if width > 0 {
        e.set_width_request(width);
    }
    e.set_hexpand(expand);
    let s = sender.clone();
    let ec = gtk::EventControllerFocus::new();
    ec.connect_leave(move |_| s.input(SummonSettingsInput::RequestPersist));
    e.add_controller(ec);
    let s2 = sender.clone();
    e.connect_activate(move |_| s2.input(SummonSettingsInput::RequestPersist));
    e
}

fn delete_button(
    list: &gtk::Box,
    container: &gtk::Box,
    sender: &ComponentSender<SummonSettingsModel>,
    on_removed: impl Fn() + 'static,
) -> gtk::Button {
    let del = gtk::Button::from_icon_name("user-trash-symbolic");
    del.add_css_class("flat");
    del.set_valign(gtk::Align::Center);
    del.set_tooltip_text(Some("Remove"));
    let list = list.clone();
    let container_c = container.clone();
    let s = sender.clone();
    del.connect_clicked(move |_| {
        list.remove(&container_c);
        on_removed();
        s.input(SummonSettingsInput::RequestPersist);
    });
    del
}

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

    let mods_e = persisting_entry(mods, "alt", 80, false, sender);
    let key_e = persisting_entry(key, "1", 60, false, sender);
    let appid_e = persisting_entry(appid, "^Spotify$", 0, true, sender);
    let title_e = persisting_entry(title, "none", 110, false, sender);
    let cmd_e = persisting_entry(cmd, "uwsm app -- …", 0, true, sender);

    container.append(&mods_e);
    container.append(&key_e);
    container.append(&appid_e);
    container.append(&title_e);
    container.append(&cmd_e);

    let rows_c = rows.clone();
    let container_c = container.clone();
    let del = delete_button(list, &container, sender, move || {
        rows_c.borrow_mut().retain(|r| r.container != container_c);
    });
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

#[allow(clippy::too_many_arguments)]
fn build_tag_row(
    list: &gtk::Box,
    rows: &Rc<RefCell<Vec<TagRow>>>,
    sender: &ComponentSender<SummonSettingsModel>,
    mods: &str,
    key: &str,
    action: &str,
    mask: u32,
) -> TagRow {
    let container = gtk::Box::new(gtk::Orientation::Horizontal, 8);

    let mods_e = persisting_entry(mods, "super+shift", 110, false, sender);
    let key_e = persisting_entry(key, "1", 60, false, sender);

    let action_dd = gtk::DropDown::from_strings(&ACTION_CHOICES);
    action_dd.set_selected(action_to_index(action));
    action_dd.set_hexpand(true);
    action_dd.set_valign(gtk::Align::Center);
    {
        let s = sender.clone();
        action_dd.connect_selected_notify(move |_| s.input(SummonSettingsInput::RequestPersist));
    }

    let tag_dd = gtk::DropDown::from_strings(&TAG_CHOICES);
    tag_dd.set_selected(mask_to_tag_index(mask));
    tag_dd.set_hexpand(true);
    tag_dd.set_valign(gtk::Align::Center);
    {
        let s = sender.clone();
        tag_dd.connect_selected_notify(move |_| s.input(SummonSettingsInput::RequestPersist));
    }

    container.append(&mods_e);
    container.append(&key_e);
    container.append(&action_dd);
    container.append(&tag_dd);

    let rows_c = rows.clone();
    let container_c = container.clone();
    let del = delete_button(list, &container, sender, move || {
        rows_c.borrow_mut().retain(|r| r.container != container_c);
    });
    container.append(&del);

    TagRow {
        mods: mods_e,
        key: key_e,
        action: action_dd,
        tag: tag_dd,
        container,
    }
}
