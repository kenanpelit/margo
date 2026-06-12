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

/// The eight lock-screen elements whose visibility is user-toggleable.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Toggle {
    Avatar,
    Greeting,
    Date,
    Battery,
    Layout,
    Notifications,
    Weather,
    Media,
}

impl Toggle {
    /// `(mlock.conf key, label, description)`.
    const ALL: [(Toggle, &'static str, &'static str, &'static str); 8] = [
        (
            Toggle::Avatar,
            "show_avatar",
            "Avatar",
            "Your user picture (~/.face or AccountsService).",
        ),
        (
            Toggle::Greeting,
            "show_greeting",
            "Greeting",
            "“Good morning, Name” line above the clock.",
        ),
        (
            Toggle::Date,
            "show_date",
            "Date",
            "Full weekday + date under the clock.",
        ),
        (
            Toggle::Battery,
            "show_battery",
            "Battery",
            "Top-right charge indicator (laptops).",
        ),
        (
            Toggle::Layout,
            "show_layout",
            "Keyboard layout",
            "Top-left active xkb layout (multi-layout setups).",
        ),
        (
            Toggle::Notifications,
            "show_notifications",
            "Notifications",
            "Unread notification count (from the shell).",
        ),
        (
            Toggle::Weather,
            "show_weather",
            "Weather",
            "Current temperature (from the shell).",
        ),
        (
            Toggle::Media,
            "show_media",
            "Now playing",
            "Title — artist of the active media player.",
        ),
    ];
}

#[derive(Debug, Clone, Copy)]
struct Toggles {
    avatar: bool,
    greeting: bool,
    date: bool,
    battery: bool,
    layout: bool,
    notifications: bool,
    weather: bool,
    media: bool,
}

impl Default for Toggles {
    fn default() -> Self {
        Self {
            avatar: true,
            greeting: true,
            date: true,
            battery: true,
            layout: true,
            notifications: true,
            weather: true,
            media: true,
        }
    }
}

impl Toggles {
    fn get(&self, t: Toggle) -> bool {
        match t {
            Toggle::Avatar => self.avatar,
            Toggle::Greeting => self.greeting,
            Toggle::Date => self.date,
            Toggle::Battery => self.battery,
            Toggle::Layout => self.layout,
            Toggle::Notifications => self.notifications,
            Toggle::Weather => self.weather,
            Toggle::Media => self.media,
        }
    }
    fn set(&mut self, t: Toggle, v: bool) {
        match t {
            Toggle::Avatar => self.avatar = v,
            Toggle::Greeting => self.greeting = v,
            Toggle::Date => self.date = v,
            Toggle::Battery => self.battery = v,
            Toggle::Layout => self.layout = v,
            Toggle::Notifications => self.notifications = v,
            Toggle::Weather => self.weather = v,
            Toggle::Media => self.media = v,
        }
    }
}

#[derive(Debug)]
pub(crate) struct LockSettingsModel {
    /// Background mode index: 0 = wallpaper, 1 = solid colour, 2 = image.
    mode: u32,
    color: String,
    image: String,
    toggles: Toggles,
    mode_model: gtk::StringList,
}

#[derive(Debug)]
pub(crate) enum LockSettingsInput {
    SetMode(u32),
    SetColor(String),
    SetImage(String),
    SetToggle(Toggle, bool),
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
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
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
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
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
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
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
                },

                gtk::Separator { set_margin_top: 8 },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Show on lock screen",
                    set_halign: gtk::Align::Start,
                },
                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "Pick what the lock screen shows alongside the clock and password field. Notifications / weather / now-playing come live from the shell.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },

                #[name = "toggles_box"]
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,
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
        let (mode, color, image, toggles) = read_mlock_conf();
        let model = LockSettingsModel {
            mode,
            color,
            image,
            toggles,
            mode_model: gtk::StringList::new(&["Wallpaper", "Solid colour", "Custom image"]),
        };
        let widgets = view_output!();

        // Build the eight visibility toggle rows.
        for (toggle, _key, label, desc) in Toggle::ALL {
            widgets.toggles_box.append(&toggle_row(
                label,
                desc,
                model.toggles.get(toggle),
                toggle,
                &sender,
            ));
        }

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
            LockSettingsInput::SetToggle(t, v) => {
                self.toggles.set(t, v);
                self.write();
            }
        }
    }
}

impl LockSettingsModel {
    fn write(&self) {
        write_mlock_conf(self.mode, &self.color, &self.image, &self.toggles);
    }
}

/// A label + description + trailing switch row for one visibility toggle.
fn toggle_row(
    label: &str,
    desc: &str,
    initial: bool,
    toggle: Toggle,
    sender: &ComponentSender<LockSettingsModel>,
) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 20);
    row.add_css_class("action-row");
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let title = gtk::Label::new(Some(label));
    title.add_css_class("label-medium-bold");
    title.set_halign(gtk::Align::Start);
    let sub = gtk::Label::new(Some(desc));
    sub.add_css_class("label-small");
    sub.set_halign(gtk::Align::Start);
    sub.set_xalign(0.0);
    sub.set_wrap(true);
    text.append(&title);
    text.append(&sub);
    row.append(&text);

    let sw = gtk::Switch::new();
    sw.set_valign(gtk::Align::Center);
    sw.set_active(initial);
    let s = sender.clone();
    sw.connect_state_set(move |_, v| {
        s.input(LockSettingsInput::SetToggle(toggle, v));
        gtk::glib::Propagation::Proceed
    });
    row.append(&sw);
    row
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

/// Read (mode index, colour, image, toggles) from mlock.conf. Missing file
/// → Wallpaper default + everything shown.
fn read_mlock_conf() -> (u32, String, String, Toggles) {
    let (mut mode, mut color, mut image) = (0u32, String::new(), String::new());
    let mut toggles = Toggles::default();
    if let Ok(text) = std::fs::read_to_string(mlock_conf_path()) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let (k, v) = (k.trim(), v.trim());
                let on = matches!(v, "true" | "1" | "yes" | "on");
                match k {
                    "background" => mode = mode_index(v),
                    "background_color" => color = v.to_string(),
                    "background_image" => image = v.to_string(),
                    "show_avatar" => toggles.avatar = on,
                    "show_greeting" => toggles.greeting = on,
                    "show_date" => toggles.date = on,
                    "show_battery" => toggles.battery = on,
                    "show_layout" => toggles.layout = on,
                    "show_notifications" => toggles.notifications = on,
                    "show_weather" => toggles.weather = on,
                    "show_media" => toggles.media = on,
                    _ => {}
                }
            }
        }
    }
    (mode, color, image, toggles)
}

fn mode_index(v: &str) -> u32 {
    match v {
        "color" => 1,
        "image" => 2,
        _ => 0,
    }
}

fn write_mlock_conf(mode: u32, color: &str, image: &str, toggles: &Toggles) {
    let mode_str = match mode {
        1 => "color",
        2 => "image",
        _ => "wallpaper",
    };
    let color = match color.trim() {
        "" => "#1e1e2e",
        c => c,
    };
    let mut body = format!(
        "# Lock-screen config — written by Settings \u{2192} Lock.\n\
         background = {mode_str}\n\
         background_color = {color}\n\
         background_image = {}\n",
        image.trim(),
    );
    for (toggle, key, _, _) in Toggle::ALL {
        body.push_str(&format!("{key} = {}\n", toggles.get(toggle)));
    }
    let path = mlock_conf_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, body);
}
