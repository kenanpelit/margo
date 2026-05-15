//! Display settings page — controls for margo's built-in
//! **twilight** blue-light filter / colour-temperature scheduler.
//!
//! Twilight state lives in margo's own config (`config.conf`)
//! rather than mshell's yaml profile, because the compositor —
//! not the shell — owns the gamma pipeline. Every control here
//! therefore takes a two-step path:
//!
//!   1. Rewrite the matching `twilight_*` line in
//!      `~/.config/margo/config.conf` in-place (preserving
//!      comments and surrounding lines).
//!   2. Spawn `mctl reload` so margo picks the new value up
//!      without a restart.
//!
//! The widget reads the initial state via [`margo_config::parse_config`]
//! so a freshly-opened settings window always reflects what's on
//! disk, including hand-edits the user made outside this UI.

use margo_config::TwilightMode;
use relm4::gtk::glib;
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, EditableExt, EntryExt, OrientableExt, ToggleButtonExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::path::PathBuf;
use std::time::Duration;
use tracing::warn;

const DEBOUNCE_MS: u64 = 400;

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct TwilightSnapshot {
    enabled: bool,
    mode: ModeKey,
    day_temp: u32,
    night_temp: u32,
    day_gamma: u32,
    night_gamma: u32,
    transition_s: u32,
    update_interval: u32,
    latitude: f32,
    longitude: f32,
    sunrise_sec: u32,
    sunset_sec: u32,
    static_temp: u32,
    static_gamma: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModeKey {
    #[default]
    Geo,
    Manual,
    Static,
    /// Multi-step preset schedule. Reads presets +
    /// schedule.conf from `twilight_schedule_dir` (default
    /// `~/.config/margo/twilight`) and interpolates between
    /// consecutive entries through the day. Margo seeds the
    /// directory with six starter presets on first run.
    Schedule,
}

impl ModeKey {
    fn label(self) -> &'static str {
        match self {
            ModeKey::Geo => "Geo (sun position)",
            ModeKey::Manual => "Manual (clock times)",
            ModeKey::Static => "Static (one fixed sample)",
            ModeKey::Schedule => "Schedule (time-of-day presets)",
        }
    }
    fn key(self) -> &'static str {
        match self {
            ModeKey::Geo => "geo",
            ModeKey::Manual => "manual",
            ModeKey::Static => "static",
            ModeKey::Schedule => "schedule",
        }
    }
    fn all() -> [Self; 4] {
        [Self::Geo, Self::Manual, Self::Static, Self::Schedule]
    }
    fn from_index(i: u32) -> Self {
        match i {
            1 => Self::Manual,
            2 => Self::Static,
            3 => Self::Schedule,
            _ => Self::Geo,
        }
    }
    fn index(self) -> u32 {
        match self {
            ModeKey::Geo => 0,
            ModeKey::Manual => 1,
            ModeKey::Static => 2,
            ModeKey::Schedule => 3,
        }
    }
}

/// One row in the loaded preset schedule. Mirrors margo's own
/// `ScheduledPreset` shape — copied here instead of imported so
/// mshell-settings doesn't need to depend on the `margo` binary
/// crate.
#[derive(Debug, Clone)]
pub(crate) struct ScheduleRow {
    pub time_hhmm: String,
    pub preset_name: String,
    pub temp_k: u32,
    pub gamma_pct: u32,
}

#[derive(Debug)]
pub(crate) struct DisplaySettingsModel {
    state: TwilightSnapshot,
    mode_model: gtk::StringList,
    /// Debounce handle for the `mctl reload` poke. A burst of
    /// slider drags should land one reload, not 30.
    reload_debounce: Option<glib::JoinHandle<()>>,
    /// Loaded preset schedule (`~/.config/margo/twilight/`).
    /// Sorted by time. Empty when the directory doesn't exist
    /// yet (margo seeds it on first switch to Schedule mode).
    schedule_rows: Vec<ScheduleRow>,
    /// Where margo reads the schedule from. Surfaced in the UI
    /// so the user knows which files to edit.
    schedule_dir_display: String,
}

#[derive(Debug)]
pub(crate) enum DisplaySettingsInput {
    EnabledChanged(bool),
    ModeChanged(ModeKey),
    DayTempChanged(u32),
    NightTempChanged(u32),
    DayGammaChanged(u32),
    NightGammaChanged(u32),
    TransitionChanged(u32),
    UpdateIntervalChanged(u32),
    LatitudeChanged(f32),
    LongitudeChanged(f32),
    SunriseChanged(String),
    SunsetChanged(String),
    StaticTempChanged(u32),
    StaticGammaChanged(u32),
    /// Internal: a debounced timer fired — actually run `mctl reload`.
    ReloadNow,
    /// Test the current settings live (no persist): runs `mctl
    /// twilight test 5`.
    PreviewSweep,
    /// Clear any preview/test override and resume the schedule.
    ResetOverride,
}

#[derive(Debug)]
pub(crate) enum DisplaySettingsOutput {}

pub(crate) struct DisplaySettingsInit {}

#[derive(Debug)]
pub(crate) enum DisplaySettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for DisplaySettingsModel {
    type CommandOutput = DisplaySettingsCommandOutput;
    type Input = DisplaySettingsInput;
    type Output = DisplaySettingsOutput;
    type Init = DisplaySettingsInit;

    view! {
        // Display is split into sub-sections via an inner sidebar
        // on the left + a Stack on the right. Twilight is the only
        // sub-section today; future Display features (output scale,
        // colour calibration, …) can land as new toggle buttons +
        // stack pages without touching the outer settings layout.
        #[root]
        gtk::Box {
            set_orientation: gtk::Orientation::Horizontal,
            set_hexpand: true,
            set_vexpand: true,

            gtk::Box {
                add_css_class: "settings-subsidebar",
                set_orientation: gtk::Orientation::Vertical,
                set_width_request: 140,
                set_spacing: 4,
                set_hexpand: false,

                gtk::Label {
                    add_css_class: "label-medium-bold",
                    set_margin_start: 8,
                    set_margin_top: 12,
                    set_margin_bottom: 6,
                    set_margin_end: 8,
                    set_label: "Display",
                    set_halign: gtk::Align::Start,
                },

                gtk::Separator {},

                #[name = "twilight_btn"]
                gtk::ToggleButton {
                    add_css_class: "sidebar-button",
                    set_active: true,
                    connect_toggled[sub_stack] => move |b| {
                        if b.is_active() { sub_stack.set_visible_child_name("twilight"); }
                    },

                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 12,
                        gtk::Image { set_icon_name: Some("nightlight-symbolic") },
                        gtk::Label {
                            add_css_class: "label-medium",
                            set_label: "Twilight",
                            set_halign: gtk::Align::Start,
                            set_hexpand: true,
                        },
                    },
                },
            },

            #[name = "sub_stack"]
            gtk::Stack {
                set_transition_type: gtk::StackTransitionType::Crossfade,
                set_transition_duration: 50,
                set_hexpand: true,
                set_vexpand: true,

                add_named[Some("twilight")] = &gtk::ScrolledWindow {
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
                            add_css_class: "label-small",
                            set_label: "Blue-light filter — bakes its colour-temperature schedule into margo's gamma pipeline, so a single tweak here covers every output. Changes write back to ~/.config/margo/config.conf and ping mctl reload.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },

                // ── Master switch + mode ───────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Twilight",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Enabled",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Master switch. Off ⇒ no gamma writes at all.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(enabled_handler)]
                        set_active: model.state.enabled,
                        connect_state_set[sender] => move |_, v| {
                            sender.input(DisplaySettingsInput::EnabledChanged(v));
                            glib::Propagation::Proceed
                        } @enabled_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Mode",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Geo: derive sunrise / sunset from lat/lng. Manual: explicit clock times. Static: hold one fixed sample 24/7.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    gtk::DropDown {
                        set_width_request: 220,
                        set_valign: gtk::Align::Center,
                        set_model: Some(&model.mode_model),
                        #[watch]
                        #[block_signal(mode_handler)]
                        set_selected: model.state.mode.index(),
                        connect_selected_notify[sender] => move |dd| {
                            sender.input(DisplaySettingsInput::ModeChanged(
                                ModeKey::from_index(dd.selected())
                            ));
                        } @mode_handler,
                    },
                },

                gtk::Separator {},

                // ── Day / Night temps + gammas ─────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Day / Night",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Day temperature (K)",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "6500 K = D65 daylight reference.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (1000.0, 25000.0),
                        set_increments: (100.0, 500.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(day_temp_handler)]
                        set_value: model.state.day_temp as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(DisplaySettingsInput::DayTempChanged(s.value() as u32));
                        } @day_temp_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Night temperature (K)",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Warm evening; typical 2800–3500 K.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (1000.0, 25000.0),
                        set_increments: (100.0, 500.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(night_temp_handler)]
                        set_value: model.state.night_temp as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(DisplaySettingsInput::NightTempChanged(s.value() as u32));
                        } @night_temp_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Day gamma (%)",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "100 = pass-through brightness.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (10.0, 200.0),
                        set_increments: (1.0, 5.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(day_gamma_handler)]
                        set_value: model.state.day_gamma as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(DisplaySettingsInput::DayGammaChanged(s.value() as u32));
                        } @day_gamma_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Night gamma (%)",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Slight dim at night reduces eye strain.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (10.0, 200.0),
                        set_increments: (1.0, 5.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(night_gamma_handler)]
                        set_value: model.state.night_gamma as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(DisplaySettingsInput::NightGammaChanged(s.value() as u32));
                        } @night_gamma_handler,
                    },
                },

                gtk::Separator {},

                // ── Timing ─────────────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Timing",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Transition (s)",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Total day↔night ramp window. 2700 = 45 min.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (30.0, 7200.0),
                        set_increments: (30.0, 300.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(transition_handler)]
                        set_value: model.state.transition_s as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(DisplaySettingsInput::TransitionChanged(s.value() as u32));
                        } @transition_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Idle update interval (s)",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "How often to wake at stable Day/Night phases. Transitions tick every 250 ms regardless.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (10.0, 300.0),
                        set_increments: (5.0, 30.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(interval_handler)]
                        set_value: model.state.update_interval as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(DisplaySettingsInput::UpdateIntervalChanged(s.value() as u32));
                        } @interval_handler,
                    },
                },

                gtk::Separator {},

                // ── Geo mode coords ────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Geo mode",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Latitude (°)",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "North positive. Used by Geo mode's sun-elevation math.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (-90.0, 90.0),
                        set_increments: (0.1, 1.0),
                        set_digits: 4,
                        #[watch]
                        #[block_signal(lat_handler)]
                        set_value: model.state.latitude as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(DisplaySettingsInput::LatitudeChanged(s.value() as f32));
                        } @lat_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Longitude (°)",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "East positive.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (-180.0, 180.0),
                        set_increments: (0.1, 1.0),
                        set_digits: 4,
                        #[watch]
                        #[block_signal(lon_handler)]
                        set_value: model.state.longitude as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(DisplaySettingsInput::LongitudeChanged(s.value() as f32));
                        } @lon_handler,
                    },
                },

                gtk::Separator {},

                // ── Manual mode times ──────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Manual mode",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Sunrise",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "HH:MM, local clock. Manual mode only.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    #[name = "sunrise_entry"]
                    gtk::Entry {
                        set_valign: gtk::Align::Center,
                        set_width_request: 100,
                        #[watch]
                        #[block_signal(sunrise_handler)]
                        set_text: &hhmm_from_seconds(model.state.sunrise_sec),
                        set_placeholder_text: Some("06:30"),
                        connect_changed[sender] => move |e| {
                            sender.input(DisplaySettingsInput::SunriseChanged(e.text().to_string()));
                        } @sunrise_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Sunset",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "HH:MM, local clock.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    #[name = "sunset_entry"]
                    gtk::Entry {
                        set_valign: gtk::Align::Center,
                        set_width_request: 100,
                        #[watch]
                        #[block_signal(sunset_handler)]
                        set_text: &hhmm_from_seconds(model.state.sunset_sec),
                        set_placeholder_text: Some("19:00"),
                        connect_changed[sender] => move |e| {
                            sender.input(DisplaySettingsInput::SunsetChanged(e.text().to_string()));
                        } @sunset_handler,
                    },
                },

                gtk::Separator {},

                // ── Static mode ────────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Static mode",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Static temperature (K)",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "One sample held 24/7 in Static mode.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (1000.0, 25000.0),
                        set_increments: (100.0, 500.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(static_temp_handler)]
                        set_value: model.state.static_temp as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(DisplaySettingsInput::StaticTempChanged(s.value() as u32));
                        } @static_temp_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Static gamma (%)",
                            set_hexpand: true,
                        },
                    },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (10.0, 200.0),
                        set_increments: (1.0, 5.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(static_gamma_handler)]
                        set_value: model.state.static_gamma as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(DisplaySettingsInput::StaticGammaChanged(s.value() as u32));
                        } @static_gamma_handler,
                    },
                },

                gtk::Separator {},

                // ── Schedule mode preset list ──────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Schedule presets",
                    set_halign: gtk::Align::Start,
                },

                gtk::Label {
                    add_css_class: "label-small",
                    #[watch]
                    set_label: &format!(
                        "Active when Mode = Schedule. Margo interpolates between consecutive presets in mired space as the day progresses. Files live in {} — edit by hand and `mctl reload` to take effect.",
                        model.schedule_dir_display
                    ),
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },

                gtk::Label {
                    add_css_class: "label-small",
                    #[watch]
                    set_visible: model.schedule_rows.is_empty(),
                    set_label: "No presets loaded yet. Switch Mode to Schedule and reload — Margo seeds the directory with a six-preset starter set on first run.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },

                #[name = "schedule_grid"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    add_css_class: "twilight-schedule-list",
                    set_spacing: 4,
                    #[watch]
                    set_visible: !model.schedule_rows.is_empty(),
                },

                gtk::Separator {},

                // ── Live preview controls ──────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Preview",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    set_halign: gtk::Align::Start,

                    gtk::Button {
                        set_label: "Sweep day → night",
                        connect_clicked[sender] => move |_| {
                            sender.input(DisplaySettingsInput::PreviewSweep);
                        },
                    },
                    gtk::Button {
                        set_label: "Reset override",
                        connect_clicked[sender] => move |_| {
                            sender.input(DisplaySettingsInput::ResetOverride);
                        },
                    },
                },
                    }, // inner gtk::Box (page contents)
                }, // ScrolledWindow named "twilight"
            }, // sub_stack
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let state = load_current_config();

        let mode_label_refs: Vec<&str> =
            ModeKey::all().iter().map(|m| m.label()).collect();
        let mode_model = gtk::StringList::new(&mode_label_refs);

        // Twilight schedule + presets from disk. Read-only here
        // — margo owns the on-disk format; the user edits the
        // files and `mctl reload` to pick up changes. Empty
        // vector and a hint string when the directory hasn't
        // been seeded yet (i.e. user hasn't run Schedule mode).
        let schedule_dir = twilight_schedule_dir();
        let schedule_rows = load_schedule_rows(&schedule_dir);
        let schedule_dir_display = schedule_dir.display().to_string();

        let model = DisplaySettingsModel {
            state,
            mode_model,
            reload_debounce: None,
            schedule_rows,
            schedule_dir_display,
        };

        let widgets = view_output!();

        // Populate the schedule list once at init. Re-runs aren't
        // wired yet — user clicks "Sweep day → night" and
        // reopens Settings to see new state. Future: refresh on
        // reload.
        for row in &model.schedule_rows {
            let row_box = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            row_box.add_css_class("twilight-schedule-row");
            let time = gtk::Label::new(Some(&row.time_hhmm));
            time.add_css_class("label-medium-bold");
            time.set_width_chars(6);
            time.set_xalign(0.0);
            row_box.append(&time);
            let name = gtk::Label::new(Some(&row.preset_name));
            name.add_css_class("label-medium");
            name.set_hexpand(true);
            name.set_xalign(0.0);
            row_box.append(&name);
            let temp = gtk::Label::new(Some(&format!("{} K", row.temp_k)));
            temp.add_css_class("label-small");
            row_box.append(&temp);
            let gamma = gtk::Label::new(Some(&format!("{}%", row.gamma_pct)));
            gamma.add_css_class("label-small");
            row_box.append(&gamma);
            widgets.schedule_grid.append(&row_box);
        }

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        let mut dirty: Option<(&'static str, String)> = None;
        match message {
            DisplaySettingsInput::EnabledChanged(v) => {
                if self.state.enabled == v {
                    return;
                }
                self.state.enabled = v;
                dirty = Some(("twilight", (v as u32).to_string()));
            }
            DisplaySettingsInput::ModeChanged(m) => {
                if self.state.mode == m {
                    return;
                }
                self.state.mode = m;
                dirty = Some(("twilight_mode", m.key().to_string()));
            }
            DisplaySettingsInput::DayTempChanged(v) => {
                if self.state.day_temp == v {
                    return;
                }
                self.state.day_temp = v;
                dirty = Some(("twilight_day_temp", v.to_string()));
            }
            DisplaySettingsInput::NightTempChanged(v) => {
                if self.state.night_temp == v {
                    return;
                }
                self.state.night_temp = v;
                dirty = Some(("twilight_night_temp", v.to_string()));
            }
            DisplaySettingsInput::DayGammaChanged(v) => {
                if self.state.day_gamma == v {
                    return;
                }
                self.state.day_gamma = v;
                dirty = Some(("twilight_day_gamma", v.to_string()));
            }
            DisplaySettingsInput::NightGammaChanged(v) => {
                if self.state.night_gamma == v {
                    return;
                }
                self.state.night_gamma = v;
                dirty = Some(("twilight_night_gamma", v.to_string()));
            }
            DisplaySettingsInput::TransitionChanged(v) => {
                if self.state.transition_s == v {
                    return;
                }
                self.state.transition_s = v;
                dirty = Some(("twilight_transition_s", v.to_string()));
            }
            DisplaySettingsInput::UpdateIntervalChanged(v) => {
                if self.state.update_interval == v {
                    return;
                }
                self.state.update_interval = v;
                dirty = Some(("twilight_update_interval", v.to_string()));
            }
            DisplaySettingsInput::LatitudeChanged(v) => {
                if (self.state.latitude - v).abs() < f32::EPSILON {
                    return;
                }
                self.state.latitude = v;
                dirty = Some(("twilight_latitude", format!("{v:.4}")));
            }
            DisplaySettingsInput::LongitudeChanged(v) => {
                if (self.state.longitude - v).abs() < f32::EPSILON {
                    return;
                }
                self.state.longitude = v;
                dirty = Some(("twilight_longitude", format!("{v:.4}")));
            }
            DisplaySettingsInput::SunriseChanged(s) => {
                if let Some(secs) = parse_hhmm(&s) {
                    if self.state.sunrise_sec != secs {
                        self.state.sunrise_sec = secs;
                        dirty = Some(("twilight_sunrise", s));
                    }
                }
            }
            DisplaySettingsInput::SunsetChanged(s) => {
                if let Some(secs) = parse_hhmm(&s) {
                    if self.state.sunset_sec != secs {
                        self.state.sunset_sec = secs;
                        dirty = Some(("twilight_sunset", s));
                    }
                }
            }
            DisplaySettingsInput::StaticTempChanged(v) => {
                if self.state.static_temp == v {
                    return;
                }
                self.state.static_temp = v;
                dirty = Some(("twilight_static_temp", v.to_string()));
            }
            DisplaySettingsInput::StaticGammaChanged(v) => {
                if self.state.static_gamma == v {
                    return;
                }
                self.state.static_gamma = v;
                dirty = Some(("twilight_static_gamma", v.to_string()));
            }
            DisplaySettingsInput::ReloadNow => {
                self.reload_debounce = None;
                spawn_mctl(&["reload"]);
                return;
            }
            DisplaySettingsInput::PreviewSweep => {
                spawn_mctl(&["twilight", "test", "5"]);
                return;
            }
            DisplaySettingsInput::ResetOverride => {
                spawn_mctl(&["twilight", "reset"]);
                return;
            }
        }

        if let Some((key, value)) = dirty {
            if let Err(e) = write_config_field(key, &value) {
                warn!(key, value, error = %e, "twilight: config write failed");
                return;
            }
            // Debounce the reload — a slider drag fires many tiny
            // updates back-to-back; one mctl reload at the tail of
            // the burst is plenty.
            if let Some(h) = self.reload_debounce.take() {
                h.abort();
            }
            let sender_clone = sender.clone();
            self.reload_debounce = Some(glib::spawn_future_local(async move {
                glib::timeout_future(Duration::from_millis(DEBOUNCE_MS)).await;
                sender_clone.input(DisplaySettingsInput::ReloadNow);
            }));
        }
    }
}

fn load_current_config() -> TwilightSnapshot {
    match margo_config::parse_config(None) {
        Ok(cfg) => TwilightSnapshot {
            enabled: cfg.twilight,
            mode: match cfg.twilight_mode {
                TwilightMode::Geo => ModeKey::Geo,
                TwilightMode::Manual => ModeKey::Manual,
                TwilightMode::Static => ModeKey::Static,
                TwilightMode::Schedule => ModeKey::Schedule,
            },
            day_temp: cfg.twilight_day_temp,
            night_temp: cfg.twilight_night_temp,
            day_gamma: cfg.twilight_day_gamma,
            night_gamma: cfg.twilight_night_gamma,
            transition_s: cfg.twilight_transition_s,
            update_interval: cfg.twilight_update_interval,
            latitude: cfg.twilight_latitude,
            longitude: cfg.twilight_longitude,
            sunrise_sec: cfg.twilight_sunrise_sec,
            sunset_sec: cfg.twilight_sunset_sec,
            static_temp: cfg.twilight_static_temp,
            static_gamma: cfg.twilight_static_gamma,
        },
        Err(e) => {
            warn!(error = %e, "twilight: could not parse margo config; using defaults");
            TwilightSnapshot {
                enabled: false,
                mode: ModeKey::Geo,
                day_temp: 6500,
                night_temp: 3300,
                day_gamma: 100,
                night_gamma: 90,
                transition_s: 2700,
                update_interval: 60,
                latitude: 0.0,
                longitude: 0.0,
                sunrise_sec: 0,
                sunset_sec: 0,
                static_temp: 4000,
                static_gamma: 95,
            }
        }
    }
}

fn margo_config_path() -> PathBuf {
    if let Ok(env) = std::env::var("MARGO_CONFIG") {
        return PathBuf::from(env);
    }
    if let Some(home) = dirs::config_dir() {
        return home.join("margo").join("config.conf");
    }
    PathBuf::from("config.conf")
}

/// In-place rewrite of one `key = value` line in margo's
/// `config.conf`. Matches lines that look like
/// `[#\s]*KEY\s*=\s*...` so a previously-commented default
/// (`# twilight_latitude = 0.0`) gets uncommented on first edit.
/// If no line matches at all the field is appended at the end of
/// the file under a `# managed by mshell display settings` header.
fn write_config_field(key: &str, value: &str) -> std::io::Result<()> {
    let path = margo_config_path();
    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        std::fs::create_dir_all(parent)?;
    }
    let src = std::fs::read_to_string(&path).unwrap_or_default();
    let replacement = format!("{key} = {value}");

    let mut found = false;
    let mut out = String::with_capacity(src.len() + replacement.len());
    for line in src.lines() {
        if !found && line_targets_key(line, key) {
            // Preserve any inline comment that followed the value.
            let trailing = match line.split_once('#') {
                Some((_, c)) if !line.trim_start().starts_with('#') => {
                    format!("  #{c}")
                }
                _ => String::new(),
            };
            out.push_str(&replacement);
            out.push_str(&trailing);
            out.push('\n');
            found = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !found {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("\n# added by mshell display settings\n");
        out.push_str(&replacement);
        out.push('\n');
    }

    let tmp = path.with_extension("conf.mshell-tmp");
    std::fs::write(&tmp, out)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Does this line, ignoring leading whitespace and an optional
/// leading `#`, declare an assignment for `key`?
fn line_targets_key(line: &str, key: &str) -> bool {
    let trimmed = line.trim_start();
    let trimmed = trimmed.strip_prefix('#').unwrap_or(trimmed).trim_start();
    let Some((lhs, _)) = trimmed.split_once('=') else {
        return false;
    };
    lhs.trim() == key
}

fn parse_hhmm(s: &str) -> Option<u32> {
    let s = s.trim();
    if s.is_empty() {
        return Some(0);
    }
    let (h, m) = s.split_once(':')?;
    let h: u32 = h.trim().parse().ok()?;
    let m: u32 = m.trim().parse().ok()?;
    if h >= 24 || m >= 60 {
        return None;
    }
    Some(h * 3600 + m * 60)
}

fn hhmm_from_seconds(secs: u32) -> String {
    if secs == 0 {
        return String::new();
    }
    let h = (secs / 3600) % 24;
    let m = (secs % 3600) / 60;
    format!("{h:02}:{m:02}")
}

fn spawn_mctl(args: &[&str]) {
    let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    relm4::spawn(async move {
        match tokio::process::Command::new("mctl")
            .args(&owned)
            .status()
            .await
        {
            Ok(s) if s.success() => {}
            Ok(s) => warn!(?s, args = ?owned, "mctl returned non-zero"),
            Err(e) => warn!(error = %e, args = ?owned, "mctl spawn failed"),
        }
    });
}

// ── Schedule-mode preset loader ─────────────────────────────────
//
// Mirrors margo's own `twilight::preset::ScheduleData::load`. We
// reimplement instead of importing because `margo` is a binary
// crate, not a library. The format is small and stable —
// `schedule.conf` is line-by-line `HH:MM PRESET_NAME`, presets
// are `presets/<name>.toml` with `static_temp` + `static_gamma`.

fn twilight_schedule_dir() -> PathBuf {
    // Mirrors margo's default. Empty `twilight_schedule_dir` =
    // `$XDG_CONFIG_HOME/margo/twilight` (or `~/.config/margo/
    // twilight`).
    let raw = match margo_config::parse_config(None) {
        Ok(cfg) => cfg.twilight_schedule_dir,
        Err(_) => String::new(),
    };
    if !raw.trim().is_empty() {
        if let Some(stripped) = raw.strip_prefix("~/")
            && let Some(home) = dirs::home_dir()
        {
            return home.join(stripped);
        }
        return PathBuf::from(raw);
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("margo").join("twilight");
    }
    if let Some(home) = dirs::home_dir() {
        return home.join(".config").join("margo").join("twilight");
    }
    PathBuf::from(".config/margo/twilight")
}

fn load_schedule_rows(dir: &std::path::Path) -> Vec<ScheduleRow> {
    let schedule = match std::fs::read_to_string(dir.join("schedule.conf")) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let presets_dir = dir.join("presets");
    let mut rows: Vec<ScheduleRow> = Vec::new();
    for line in schedule.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(time_str) = parts.next() else { continue };
        let Some(name) = parts.next() else { continue };
        if parse_hhmm(time_str).is_none() {
            continue;
        }
        let p = presets_dir.join(format!("{name}.toml"));
        if let Some((temp_k, gamma_pct)) = load_preset_file(&p) {
            rows.push(ScheduleRow {
                time_hhmm: time_str.to_string(),
                preset_name: name.to_string(),
                temp_k,
                gamma_pct,
            });
        }
    }
    rows.sort_by(|a, b| a.time_hhmm.cmp(&b.time_hhmm));
    rows
}

fn load_preset_file(path: &std::path::Path) -> Option<(u32, u32)> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut temp_k: Option<u32> = None;
    let mut gamma_pct: Option<u32> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let key = k.trim();
        let val = v.trim().trim_matches('"');
        match key {
            "static_temp" => temp_k = val.parse().ok().map(|n: u32| n.clamp(1000, 25000)),
            "static_gamma" => gamma_pct = val.parse().ok().map(|n: u32| n.clamp(10, 200)),
            _ => {}
        }
    }
    Some((temp_k?, gamma_pct?))
}
