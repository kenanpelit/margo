//! Settings → Widgets → Clipboard.
//!
//! The clipboard's own page: menu size/position on top, then the
//! history behaviour knobs (size, persistence, auto-clear,
//! password-manager skip, image history) that used to live under
//! Launcher. Writes flow to `config.menus.clipboard_menu` (size)
//! and `config.clipboard` (behaviour), and behaviour changes are
//! live-applied to the running watcher.

use mshell_config::config_manager::config_manager;
use mshell_config::schema::clipboard::{ClipboardClearPolicy, ClipboardPersist};
use mshell_config::schema::position::Position;
use reactive_graph::traits::GetUntracked;
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

/// Push the current clipboard config to the running watcher (live
/// size / persist / clear-policy / sensitive / image change).
fn apply_clipboard_config() {
    use mshell_clipboard::{ClearPolicy, ClipboardSettings, PersistMode};
    let c = config_manager().config().get_untracked().clipboard;
    mshell_clipboard::clipboard_service().apply_settings(ClipboardSettings {
        max_entries: c.max_entries.max(1),
        persist: match c.persist {
            ClipboardPersist::None => PersistMode::None,
            ClipboardPersist::FavoritesOnly => PersistMode::FavoritesOnly,
            ClipboardPersist::All => PersistMode::All,
        },
        clear_policy: match c.clear_policy {
            ClipboardClearPolicy::Never => ClearPolicy::Never,
            ClipboardClearPolicy::AfterHours => ClearPolicy::AfterHours,
            ClipboardClearPolicy::OnLogout => ClearPolicy::OnLogout,
        },
        clear_after_hours: c.clear_after_hours,
        skip_sensitive: c.skip_sensitive,
        image_history: c.image_history,
    });
}

#[derive(Debug)]
pub(crate) struct ClipboardSettingsModel {
    position_model: gtk::StringList,
}

#[derive(Debug)]
pub(crate) enum ClipboardSettingsInput {
    // Menu surface size / position (config.menus.clipboard_menu).
    SetPosition(u32),
    SetMinWidth(i32),
    SetMaxHeight(i32),
    // History behaviour (config.clipboard).
    SetMaxEntries(i32),
    SetPersist(u32),
    SetClearPolicy(u32),
    SetClearHours(i32),
    SetSkipSensitive(bool),
    SetImageHistory(bool),
    ClearAll,
    ClearUnpinned,
}

#[derive(Debug)]
pub(crate) enum ClipboardSettingsOutput {}

pub(crate) struct ClipboardSettingsInit {}

#[derive(Debug)]
pub(crate) enum ClipboardSettingsCommandOutput {}

#[relm4::component(pub(crate))]
impl Component for ClipboardSettingsModel {
    type CommandOutput = ClipboardSettingsCommandOutput;
    type Input = ClipboardSettingsInput;
    type Output = ClipboardSettingsOutput;
    type Init = ClipboardSettingsInit;

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

                // Hero ─────────────────────────────────────────
                gtk::Box {
                    add_css_class: "settings-hero",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Start,
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("edit-paste-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Clipboard",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Tracks every copy from any app via the \
                                        Wayland clipboard. Pin (★) entries to keep \
                                        them; favourites survive restarts and \
                                        auto-clear.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                // Menu size / position ─────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Menu size & position",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 12,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    gtk::Label { set_label: "Position", set_hexpand: true, set_halign: gtk::Align::Start },
                    #[name = "pos_dd"]
                    gtk::DropDown {
                        set_model: Some(&model.position_model),
                        connect_selected_notify[sender] => move |d| {
                            sender.input(ClipboardSettingsInput::SetPosition(d.selected()));
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    gtk::Label { set_label: "Width (px)", set_hexpand: true, set_halign: gtk::Align::Start },
                    #[name = "width_spin"]
                    gtk::SpinButton {
                        set_adjustment: &gtk::Adjustment::new(420.0, 280.0, 1200.0, 10.0, 50.0, 0.0),
                        set_digits: 0,
                        connect_value_changed[sender] => move |s| {
                            sender.input(ClipboardSettingsInput::SetMinWidth(s.value() as i32));
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    gtk::Label { set_label: "Max height (px, 0 = no cap)", set_hexpand: true, set_halign: gtk::Align::Start },
                    #[name = "height_spin"]
                    gtk::SpinButton {
                        set_adjustment: &gtk::Adjustment::new(600.0, 0.0, 2000.0, 20.0, 100.0, 0.0),
                        set_digits: 0,
                        connect_value_changed[sender] => move |s| {
                            sender.input(ClipboardSettingsInput::SetMaxHeight(s.value() as i32));
                        },
                    },
                },

                // History behaviour ────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "History",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 12,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    gtk::Label { set_label: "History size", set_hexpand: true, set_halign: gtk::Align::Start },
                    #[name = "cb_max"]
                    gtk::SpinButton {
                        set_adjustment: &gtk::Adjustment::new(100.0, 5.0, 10000.0, 5.0, 100.0, 0.0),
                        set_digits: 0,
                        connect_value_changed[sender] => move |s| {
                            sender.input(ClipboardSettingsInput::SetMaxEntries(s.value() as i32));
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    gtk::Label { set_label: "Persist to disk", set_hexpand: true, set_halign: gtk::Align::Start },
                    #[name = "cb_persist"]
                    gtk::DropDown {
                        connect_selected_notify[sender] => move |d| {
                            sender.input(ClipboardSettingsInput::SetPersist(d.selected()));
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    gtk::Label { set_label: "Auto-clear", set_hexpand: true, set_halign: gtk::Align::Start },
                    #[name = "cb_clear"]
                    gtk::DropDown {
                        connect_selected_notify[sender] => move |d| {
                            sender.input(ClipboardSettingsInput::SetClearPolicy(d.selected()));
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    gtk::Label { set_label: "Clear after (hours)", set_hexpand: true, set_halign: gtk::Align::Start },
                    #[name = "cb_hours"]
                    gtk::SpinButton {
                        set_adjustment: &gtk::Adjustment::new(24.0, 1.0, 720.0, 1.0, 6.0, 0.0),
                        set_digits: 0,
                        connect_value_changed[sender] => move |s| {
                            sender.input(ClipboardSettingsInput::SetClearHours(s.value() as i32));
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    gtk::Label { set_label: "Skip password-manager copies", set_hexpand: true, set_halign: gtk::Align::Start },
                    #[name = "cb_sensitive"]
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        connect_state_set[sender] => move |_, state| {
                            sender.input(ClipboardSettingsInput::SetSkipSensitive(state));
                            glib::Propagation::Proceed
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    gtk::Label { set_label: "Keep image copies", set_hexpand: true, set_halign: gtk::Align::Start },
                    #[name = "cb_images"]
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        connect_state_set[sender] => move |_, state| {
                            sender.input(ClipboardSettingsInput::SetImageHistory(state));
                            glib::Propagation::Proceed
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        set_label: "Clear unpinned",
                        connect_clicked[sender] => move |_| {
                            sender.input(ClipboardSettingsInput::ClearUnpinned);
                        },
                    },
                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        set_label: "Clear all",
                        connect_clicked[sender] => move |_| {
                            sender.input(ClipboardSettingsInput::ClearAll);
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
        let position_refs: Vec<&str> = Position::all().iter().map(|p| p.display_name()).collect();
        let position_model = gtk::StringList::new(&position_refs);

        let model = ClipboardSettingsModel { position_model };
        let widgets = view_output!();

        // Prime all controls from config.
        let cfg = config_manager().config().get_untracked();
        let m = &cfg.menus.clipboard_menu;
        let pos_idx = Position::all()
            .iter()
            .position(|p| *p == m.position)
            .unwrap_or(0) as u32;
        widgets.pos_dd.set_selected(pos_idx);
        widgets.width_spin.set_value(m.minimum_width as f64);
        widgets.height_spin.set_value(m.maximum_height as f64);

        let cb = &cfg.clipboard;
        widgets.cb_persist.set_model(Some(&gtk::StringList::new(
            &ClipboardPersist::display_names(),
        )));
        widgets.cb_persist.set_selected(cb.persist.to_index());
        widgets.cb_clear.set_model(Some(&gtk::StringList::new(
            &ClipboardClearPolicy::display_names(),
        )));
        widgets.cb_clear.set_selected(cb.clear_policy.to_index());
        widgets.cb_max.set_value(cb.max_entries as f64);
        widgets.cb_hours.set_value(cb.clear_after_hours as f64);
        widgets.cb_sensitive.set_active(cb.skip_sensitive);
        widgets.cb_images.set_active(cb.image_history);

        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            ClipboardSettingsInput::SetPosition(i) => {
                config_manager()
                    .update_config(|c| c.menus.clipboard_menu.position = Position::from_index(i));
            }
            ClipboardSettingsInput::SetMinWidth(v) => {
                config_manager().update_config(|c| c.menus.clipboard_menu.minimum_width = v.max(1));
            }
            ClipboardSettingsInput::SetMaxHeight(v) => {
                config_manager().update_config(|c| c.menus.clipboard_menu.maximum_height = v.max(0));
            }
            ClipboardSettingsInput::SetMaxEntries(v) => {
                config_manager().update_config(|c| c.clipboard.max_entries = v.max(1) as usize);
                apply_clipboard_config();
            }
            ClipboardSettingsInput::SetPersist(i) => {
                config_manager()
                    .update_config(|c| c.clipboard.persist = ClipboardPersist::from_index(i));
                apply_clipboard_config();
            }
            ClipboardSettingsInput::SetClearPolicy(i) => {
                config_manager()
                    .update_config(|c| c.clipboard.clear_policy = ClipboardClearPolicy::from_index(i));
                apply_clipboard_config();
            }
            ClipboardSettingsInput::SetClearHours(v) => {
                config_manager().update_config(|c| c.clipboard.clear_after_hours = v.max(1) as u32);
                apply_clipboard_config();
            }
            ClipboardSettingsInput::SetSkipSensitive(on) => {
                config_manager().update_config(|c| c.clipboard.skip_sensitive = on);
                apply_clipboard_config();
            }
            ClipboardSettingsInput::SetImageHistory(on) => {
                config_manager().update_config(|c| c.clipboard.image_history = on);
                apply_clipboard_config();
            }
            ClipboardSettingsInput::ClearAll => {
                mshell_clipboard::clipboard_service().clear_history();
                mshell_launcher::notify::toast("Clipboard cleared", "All entries removed.");
            }
            ClipboardSettingsInput::ClearUnpinned => {
                mshell_clipboard::clipboard_service().clear_unpinned();
                mshell_launcher::notify::toast(
                    "Clipboard cleared",
                    "Unpinned entries removed; favorites kept.",
                );
            }
        }
    }
}
