//! Session settings — per-action command overrides for the
//! power menu. Each entry is the command run for that action;
//! leaving it empty falls back to the built-in default
//! (`systemctl …` / the in-process session lock). Non-empty
//! commands run via `sh -c`.
//!
//! The entries are seeded once at init and then own their state
//! — they're deliberately not `#[watch]`-bound to the config
//! store. A reactive `set_text` round-trip resets the cursor to
//! position 0 on every keystroke, which makes typing read
//! right-to-left; the entry is the sole editor here, so a
//! one-shot seed is both correct and bug-free.

use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, SessionStoreFields};
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub(crate) struct SessionSettingsModel {
    lock_command: String,
    logout_command: String,
    suspend_command: String,
    reboot_command: String,
    shutdown_command: String,
    /// Lock-screen background (mlock.conf, not the YAML config): mode
    /// index 0=wallpaper / 1=color / 2=image, plus the colour + image.
    bg_mode: u32,
    bg_color: String,
    bg_image: String,
    bg_mode_model: gtk::StringList,
}

#[derive(Debug)]
pub(crate) enum SessionSettingsInput {
    LockChanged(String),
    LogoutChanged(String),
    SuspendChanged(String),
    RebootChanged(String),
    ShutdownChanged(String),
    BgModeChanged(u32),
    BgColorChanged(String),
    BgImageChanged(String),
}

#[derive(Debug)]
pub(crate) enum SessionSettingsOutput {}

pub(crate) struct SessionSettingsInit {}

#[derive(Debug)]
pub(crate) enum SessionSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for SessionSettingsModel {
    type CommandOutput = SessionSettingsCommandOutput;
    type Input = SessionSettingsInput;
    type Output = SessionSettingsOutput;
    type Init = SessionSettingsInit;

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

                gtk::Box {
                    add_css_class: "settings-hero",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Start,
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("system-shutdown-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Session",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Lock / Logout / Suspend / Reboot / Shutdown commands, confirmation countdown, and the super+delete keybind.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "Commands run by the session menu and `mshellctl menu session …`. Leave a field empty to use the built-in default. Non-empty commands run via `sh -c`.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },

                // ── Lock ────────────────────────────────────────
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Lock",
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_label: "Empty = built-in session lock.",
                        },
                    },
                    gtk::Entry {
                        set_valign: gtk::Align::Center,
                        set_width_request: 240,
                        set_placeholder_text: Some("built-in"),
                        set_text: &model.lock_command,
                        connect_changed[sender] => move |e| {
                            sender.input(SessionSettingsInput::LockChanged(e.text().to_string()));
                        },
                    },
                },

                // ── Logout ──────────────────────────────────────
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Logout",
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_label: "Empty = systemctl --user exit.",
                        },
                    },
                    gtk::Entry {
                        set_valign: gtk::Align::Center,
                        set_width_request: 240,
                        set_placeholder_text: Some("systemctl --user exit"),
                        set_text: &model.logout_command,
                        connect_changed[sender] => move |e| {
                            sender.input(SessionSettingsInput::LogoutChanged(e.text().to_string()));
                        },
                    },
                },

                // ── Suspend ─────────────────────────────────────
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Suspend",
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_label: "Empty = systemctl suspend.",
                        },
                    },
                    gtk::Entry {
                        set_valign: gtk::Align::Center,
                        set_width_request: 240,
                        set_placeholder_text: Some("systemctl suspend"),
                        set_text: &model.suspend_command,
                        connect_changed[sender] => move |e| {
                            sender.input(SessionSettingsInput::SuspendChanged(e.text().to_string()));
                        },
                    },
                },

                // ── Reboot ──────────────────────────────────────
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Reboot",
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_label: "Empty = systemctl reboot. e.g. osc-safe-reboot",
                        },
                    },
                    gtk::Entry {
                        set_valign: gtk::Align::Center,
                        set_width_request: 240,
                        set_placeholder_text: Some("systemctl reboot"),
                        set_text: &model.reboot_command,
                        connect_changed[sender] => move |e| {
                            sender.input(SessionSettingsInput::RebootChanged(e.text().to_string()));
                        },
                    },
                },

                // ── Shutdown ────────────────────────────────────
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Shutdown",
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_label: "Empty = systemctl poweroff.",
                        },
                    },
                    gtk::Entry {
                        set_valign: gtk::Align::Center,
                        set_width_request: 240,
                        set_placeholder_text: Some("systemctl poweroff"),
                        set_text: &model.shutdown_command,
                        connect_changed[sender] => move |e| {
                            sender.input(SessionSettingsInput::ShutdownChanged(
                                e.text().to_string(),
                            ));
                        },
                    },
                },

                // ── Lock screen background ──────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Lock screen background",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 12,
                },
                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "Backdrop behind the lock screen (mlock). The colour / image fields apply only in their matching mode.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Label {
                        add_css_class: "label-medium-bold",
                        set_halign: gtk::Align::Start,
                        set_hexpand: true,
                        set_label: "Mode",
                    },
                    #[name = "bg_mode_dd"]
                    gtk::DropDown {
                        set_valign: gtk::Align::Center,
                        set_width_request: 240,
                        set_model: Some(&model.bg_mode_model),
                        connect_selected_notify[sender] => move |d| {
                            sender.input(SessionSettingsInput::BgModeChanged(d.selected()));
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Label {
                        add_css_class: "label-medium-bold",
                        set_halign: gtk::Align::Start,
                        set_hexpand: true,
                        set_label: "Solid colour",
                    },
                    gtk::Entry {
                        set_valign: gtk::Align::Center,
                        set_width_request: 240,
                        set_placeholder_text: Some("#1e1e2e"),
                        set_text: &model.bg_color,
                        connect_changed[sender] => move |e| {
                            sender.input(SessionSettingsInput::BgColorChanged(e.text().to_string()));
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Label {
                        add_css_class: "label-medium-bold",
                        set_halign: gtk::Align::Start,
                        set_hexpand: true,
                        set_label: "Custom image",
                    },
                    gtk::Entry {
                        set_valign: gtk::Align::Center,
                        set_width_request: 240,
                        set_placeholder_text: Some("~/Pictures/lock.jpg"),
                        set_text: &model.bg_image,
                        connect_changed[sender] => move |e| {
                            sender.input(SessionSettingsInput::BgImageChanged(e.text().to_string()));
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
        // The `SessionStoreFields` accessors consume `self`, so the
        // `config().session()` chain has to be re-walked per field.
        let (bg_mode, bg_color, bg_image) = read_mlock_conf();
        let model = SessionSettingsModel {
            lock_command: config_manager()
                .config()
                .session()
                .lock_command()
                .get_untracked(),
            logout_command: config_manager()
                .config()
                .session()
                .logout_command()
                .get_untracked(),
            suspend_command: config_manager()
                .config()
                .session()
                .suspend_command()
                .get_untracked(),
            reboot_command: config_manager()
                .config()
                .session()
                .reboot_command()
                .get_untracked(),
            shutdown_command: config_manager()
                .config()
                .session()
                .shutdown_command()
                .get_untracked(),
            bg_mode,
            bg_color,
            bg_image,
            bg_mode_model: gtk::StringList::new(&["Wallpaper", "Solid colour", "Custom image"]),
        };

        let widgets = view_output!();
        widgets.bg_mode_dd.set_selected(model.bg_mode);

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            SessionSettingsInput::LockChanged(v) => {
                config_manager().update_config(|c| c.session.lock_command = v);
            }
            SessionSettingsInput::LogoutChanged(v) => {
                config_manager().update_config(|c| c.session.logout_command = v);
            }
            SessionSettingsInput::SuspendChanged(v) => {
                config_manager().update_config(|c| c.session.suspend_command = v);
            }
            SessionSettingsInput::RebootChanged(v) => {
                config_manager().update_config(|c| c.session.reboot_command = v);
            }
            SessionSettingsInput::ShutdownChanged(v) => {
                config_manager().update_config(|c| c.session.shutdown_command = v);
            }
            SessionSettingsInput::BgModeChanged(m) => {
                self.bg_mode = m;
                self.write_bg();
            }
            SessionSettingsInput::BgColorChanged(c) => {
                self.bg_color = c;
                self.write_bg();
            }
            SessionSettingsInput::BgImageChanged(i) => {
                self.bg_image = i;
                self.write_bg();
            }
        }
    }
}

impl SessionSettingsModel {
    fn write_bg(&self) {
        write_mlock_conf(self.bg_mode, &self.bg_color, &self.bg_image);
    }
}

/// `~/.config/margo/mlock.conf` — the locker's own background config
/// (mlock can't read the shell's YAML, so this is a small key=value file
/// it hand-parses; see `mlock/src/background.rs`).
fn mlock_conf_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("margo").join("mlock.conf")
}

/// Read (mode index, colour, image) from mlock.conf. Missing file → the
/// Wallpaper default (0, empty, empty).
fn read_mlock_conf() -> (u32, String, String) {
    let (mut mode, mut color, mut image) = (0u32, String::new(), String::new());
    if let Ok(text) = std::fs::read_to_string(mlock_conf_path()) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let (k, v) = (k.trim(), v.trim());
                match k {
                    "background" => mode = matches_mode(v),
                    "background_color" => color = v.to_string(),
                    "background_image" => image = v.to_string(),
                    _ => {}
                }
            }
        }
    }
    (mode, color, image)
}

fn matches_mode(v: &str) -> u32 {
    match v {
        "color" => 1,
        "image" => 2,
        _ => 0,
    }
}

fn write_mlock_conf(mode: u32, color: &str, image: &str) {
    let mode_str = match mode {
        1 => "color",
        2 => "image",
        _ => "wallpaper",
    };
    let color = match color.trim() {
        "" => "#1e1e2e",
        c => c,
    };
    let body = format!(
        "# Lock-screen background — written by Settings \u{2192} Session.\n\
         background = {mode_str}\n\
         background_color = {color}\n\
         background_image = {}\n",
        image.trim(),
    );
    let path = mlock_conf_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, body);
}
