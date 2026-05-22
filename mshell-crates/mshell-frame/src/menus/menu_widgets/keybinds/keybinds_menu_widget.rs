//! Keybind cheatsheet menu — the panel content for
//! `MenuType::Keybinds`. Renders the shortcuts parsed from
//! `config.conf` (see [`crate::keybinds`]) as searchable, grouped
//! rows: a colour-coded key-combo cluster on the left, the
//! description on the right. Re-parsed each time the panel opens.

use crate::keybinds::{self, Keybind, Section};
use relm4::gtk::prelude::{BoxExt, EditableExt, EntryExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct KeybindsMenuWidgetModel {
    sections: Vec<Section>,
    content: gtk::Box,
}

#[derive(Debug)]
pub(crate) enum KeybindsMenuWidgetInput {
    /// Filter text changed.
    Search(String),
}

#[derive(Debug)]
pub(crate) enum KeybindsMenuWidgetOutput {}

pub(crate) struct KeybindsMenuWidgetInit {}

#[relm4::component(pub(crate))]
impl Component for KeybindsMenuWidgetModel {
    type CommandOutput = ();
    type Input = KeybindsMenuWidgetInput;
    type Output = KeybindsMenuWidgetOutput;
    type Init = KeybindsMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "keybinds-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 8,

            // ── §12 panel header ──
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,
                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("input-keyboard-symbolic"),
                },
                gtk::Label {
                    add_css_class: "panel-title",
                    set_halign: gtk::Align::Start,
                    set_hexpand: true,
                    set_label: "Keyboard Shortcuts",
                },
            },

            #[name = "search_entry"]
            gtk::Entry {
                add_css_class: "keybinds-search",
                set_placeholder_text: Some("Filter shortcuts…"),
                connect_changed[sender] => move |e| {
                    sender.input(KeybindsMenuWidgetInput::Search(e.text().to_string()));
                },
            },

            // The list is appended directly to the menu's own
            // ScrolledWindow (see `MenuModel`), which clamps the
            // viewport at `keybinds_menu.maximum_height` and scrolls.
            // An inner ScrolledWindow here would pin a constant
            // natural height (so the outer `propagate_natural_height`
            // never saw the real list size — the height setting did
            // nothing) and would also swallow scroll-wheel events.
            #[local_ref]
            content -> gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 4,
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let sections = keybinds::load_sections();

        let model = KeybindsMenuWidgetModel {
            sections,
            content: content.clone(),
        };
        let widgets = view_output!();

        rebuild(&model.content, &model.sections, "");

        // Focus the filter each time the panel is shown so the user can
        // type immediately (the frame grants the menu keyboard focus).
        {
            let entry = widgets.search_entry.clone();
            root.connect_map(move |_| {
                entry.grab_focus();
            });
        }

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            KeybindsMenuWidgetInput::Search(term) => {
                rebuild(&self.content, &self.sections, &term);
            }
        }
    }
}

/// (Re)build the grouped list, keeping only binds matching `filter`.
fn rebuild(container: &gtk::Box, sections: &[Section], filter: &str) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    let needle = filter.trim().to_ascii_lowercase();
    let mut any = false;

    for section in sections {
        let matches: Vec<&Keybind> = section
            .binds
            .iter()
            .filter(|b| matches_filter(b, &needle))
            .collect();
        if matches.is_empty() {
            continue;
        }
        any = true;

        let header = gtk::Label::new(Some(section.title));
        header.add_css_class("keybinds-section-label");
        header.set_halign(gtk::Align::Start);
        header.set_xalign(0.0);
        container.append(&header);

        for kb in matches {
            container.append(&make_row(kb));
        }
    }

    if !any {
        let empty = gtk::Label::new(Some(if needle.is_empty() {
            "No keybindings found in config.conf"
        } else {
            "No matching shortcuts"
        }));
        empty.add_css_class("label-small");
        empty.set_halign(gtk::Align::Start);
        container.append(&empty);
    }
}

fn matches_filter(b: &Keybind, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    b.desc.to_ascii_lowercase().contains(needle)
        || b.key.to_ascii_lowercase().contains(needle)
        || b.mods.iter().any(|m| m.to_ascii_lowercase().contains(needle))
}

/// One shortcut row: key-combo cluster + description.
fn make_row(kb: &Keybind) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("keybinds-row");

    let combo = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    combo.add_css_class("keybinds-combo");
    combo.set_halign(gtk::Align::Start);
    for m in &kb.mods {
        combo.append(&mod_chip(m));
    }
    if !kb.key.is_empty() {
        combo.append(&key_chip(&kb.key));
    }
    row.append(&combo);

    let desc = gtk::Label::new(Some(&kb.desc));
    desc.add_css_class("keybinds-desc");
    desc.set_hexpand(true);
    desc.set_halign(gtk::Align::End);
    desc.set_xalign(1.0);
    desc.set_wrap(true);
    desc.set_ellipsize(gtk::pango::EllipsizeMode::End);
    row.append(&desc);

    row
}

/// A modifier chip, coloured per modifier via a CSS class.
fn mod_chip(name: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(name));
    let cls = match name {
        "Super" => "mod-super",
        "Ctrl" => "mod-ctrl",
        "Shift" => "mod-shift",
        "Alt" => "mod-alt",
        _ => "mod-other",
    };
    l.set_css_classes(&["keybind-chip", "keybind-mod", cls]);
    l
}

/// The trigger-key chip.
fn key_chip(key: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(key));
    l.set_css_classes(&["keybind-chip", "keybind-key"]);
    l
}
