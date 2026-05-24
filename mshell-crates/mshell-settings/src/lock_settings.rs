//! Settings → Widgets → Lock.
//!
//! The lock-screen (mlock) settings. Unlike the other shell pages, mlock
//! is a standalone binary that deliberately does **not** read the shell's
//! YAML config — it stays usable even if the shell is misconfigured. So
//! its background choice lives in a tiny key=value file,
//! `~/.config/margo/mlock.conf`, that mlock hand-parses (see
//! `mlock/src/background.rs`). This page reads / writes that file directly.

use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::path::PathBuf;

#[derive(Debug)]
pub(crate) struct LockSettingsModel {
    /// Background mode index: 0 = wallpaper, 1 = solid colour, 2 = image.
    mode: u32,
    color: String,
    image: String,
    mode_model: gtk::StringList,
}

#[derive(Debug)]
pub(crate) enum LockSettingsInput {
    SetMode(u32),
    SetColor(String),
    SetImage(String),
}

#[derive(Debug)]
pub(crate) enum LockSettingsOutput {}

pub(crate) struct LockSettingsInit {}

#[derive(Debug)]
pub(crate) enum LockSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for LockSettingsModel {
    type CommandOutput = LockSettingsCommandOutput;
    type Input = LockSettingsInput;
    type Output = LockSettingsOutput;
    type Init = LockSettingsInit;

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
                        set_icon_name: Some("system-lock-screen-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Lock Screen",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "The mlock lock screen — what shows behind the clock and password field.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Background",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Mode",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Desktop wallpaper, a flat colour, or a fixed image. A slight dim + vignette is always applied so the clock and prompt stay legible.",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    #[name = "mode_dd"]
                    gtk::DropDown {
                        set_valign: gtk::Align::Center,
                        set_width_request: 200,
                        set_model: Some(&model.mode_model),
                        #[block_signal(mode_handler)]
                        set_selected: model.mode,
                        connect_selected_notify[sender] => move |d| {
                            sender.input(LockSettingsInput::SetMode(d.selected()));
                        } @mode_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Solid colour",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Hex colour for the “Solid colour” mode.",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    #[name = "color_entry"]
                    gtk::Entry {
                        set_valign: gtk::Align::Center,
                        set_width_request: 200,
                        set_placeholder_text: Some("#1e1e2e"),
                        #[block_signal(color_handler)]
                        set_text: &model.color,
                        connect_changed[sender] => move |e| {
                            sender.input(LockSettingsInput::SetColor(e.text().to_string()));
                        } @color_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Custom image",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Image path for the “Custom image” mode. Falls back to the desktop wallpaper if missing.",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    #[name = "image_entry"]
                    gtk::Entry {
                        set_valign: gtk::Align::Center,
                        set_width_request: 200,
                        set_placeholder_text: Some("~/Pictures/lock.jpg"),
                        #[block_signal(image_handler)]
                        set_text: &model.image,
                        connect_changed[sender] => move |e| {
                            sender.input(LockSettingsInput::SetImage(e.text().to_string()));
                        } @image_handler,
                    },
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "The bar's Lock pill itself (placement / add / remove) is configured under Bar → Top or Bottom bar widget lists.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                    set_margin_top: 12,
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let (mode, color, image) = read_mlock_conf();
        let model = LockSettingsModel {
            mode,
            color,
            image,
            mode_model: gtk::StringList::new(&["Wallpaper", "Solid colour", "Custom image"]),
        };
        let widgets = view_output!();
        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            LockSettingsInput::SetMode(v) => {
                self.mode = v;
                self.write();
            }
            LockSettingsInput::SetColor(v) => {
                self.color = v;
                self.write();
            }
            LockSettingsInput::SetImage(v) => {
                self.image = v;
                self.write();
            }
        }
    }
}

impl LockSettingsModel {
    fn write(&self) {
        write_mlock_conf(self.mode, &self.color, &self.image);
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
                    "background" => mode = mode_index(v),
                    "background_color" => color = v.to_string(),
                    "background_image" => image = v.to_string(),
                    _ => {}
                }
            }
        }
    }
    (mode, color, image)
}

fn mode_index(v: &str) -> u32 {
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
        "# Lock-screen background — written by Settings \u{2192} Lock.\n\
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
