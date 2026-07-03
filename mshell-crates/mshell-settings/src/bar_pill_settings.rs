//! Per-bar-pill info pages.
//!
//! Bar-only widgets (Active Window, Margo Tags, Battery, etc.)
//! don't have their own menu surface — they're just pills that
//! live in the bar. Their placement is driven by the Bar's
//! widget list (Bar → Top / Bottom widget arrays), not a
//! position/min-width pair like menu surfaces have.
//!
//! These pages surface the widget so the Settings sidebar is
//! complete, and point the user to the right place to edit its
//! placement. Widgets that DO have a menu surface (System
//! Updates, CPU Dashboard, …) are configured under Widgets →
//! Menus instead, not here.

use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{AudioConfigStoreFields, ConfigStoreFields};
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BarPillKind {
    ActiveWindow,
    AudioRoute,
    AudioVisualizer,
    Countdown,
    DarkMode,
    KeyboardLayout,
    ColorPicker,
    // Lock has its own rich page (lock-screen background) — see
    // `lock_settings.rs` — so it's not a generic bar-pill info page.
    Logout,
    MargoTags,
    Reboot,
    RecordingIndicator,
    Shutdown,
    VpnIndicator,
}

impl BarPillKind {
    pub(crate) fn display_name(self) -> &'static str {
        match self {
            Self::ActiveWindow => "Active Window",
            Self::AudioRoute => "Audio Route",
            Self::AudioVisualizer => "Audio Visualizer",
            Self::Countdown => "Countdown",
            Self::DarkMode => "Dark Mode Toggle",
            Self::KeyboardLayout => "Keyboard Layout",
            Self::ColorPicker => "ColorPicker",
            Self::Logout => "Logout",
            Self::MargoTags => "Margo Tags",
            Self::Reboot => "Reboot",
            Self::RecordingIndicator => "Recording Indicator",
            Self::Shutdown => "Shutdown",
            Self::VpnIndicator => "VPN Indicator",
        }
    }

    /// One-line description for the page body. Cuts to the
    /// chase: what is this widget, and what makes it useful?
    fn description(self) -> &'static str {
        match self {
            Self::ActiveWindow => {
                "Shows the title of the currently focused window. Click to cycle through windows on the active tag."
            }
            Self::AudioRoute => {
                "Switches the default audio output. Left-click cycles to the next output (Bluetooth / USB / speakers …), skipping HDMI; right-click opens a picker menu to jump straight to one. Shown only when there are at least two outputs to switch between. With the toggle below, the microphone follows across the headset boundary — detected from PipeWire's portable device metadata, not hardcoded names."
            }
            Self::AudioVisualizer => {
                "Live audio spectrum — a strip of bars driven by the `cava` CLI (raw mode). Pulses with whatever is playing; sits as a flat resting strip on silence. Needs `cava` installed."
            }
            Self::Countdown => {
                "Shows the soonest enabled countdown from the Alarm Clock (a schedule/hourglass glyph + remaining time). Click opens the Alarm Clock menu on its Countdown tab. Hidden when no enabled, parseable target remains."
            }
            Self::DarkMode => {
                "One-click flip between Light and Dark matugen modes. Icon reflects the mode you'd switch *to*."
            }
            Self::KeyboardLayout => {
                "Shows the active xkb keyboard layout (e.g. US, TR). Click cycles to the next configured layout — set multiple via `xkb_rules_layout = tr,us` in config.conf."
            }
            Self::ColorPicker => {
                "Picks a colour from the screen and copies hex/rgb to the clipboard. Click to start picking."
            }
            Self::Logout => "Logs out of the session. Confirms with a dialog.",
            Self::MargoTags => {
                "1–9 tag pills with focus / occupied / urgent states. Click to switch tags, scroll to cycle."
            }
            Self::Reboot => "Reboots the system. Confirms with a dialog.",
            Self::RecordingIndicator => {
                "Lights up while a screen-recording is in progress. Click stops the recording."
            }
            Self::Shutdown => "Powers off the system. Confirms with a dialog.",
            Self::VpnIndicator => {
                "Visual cue when a VPN tunnel is up (NetworkManager / wg-quick / openvpn)."
            }
        }
    }
}

pub(crate) struct BarPillSettingsModel {
    kind: BarPillKind,
    /// Audio Route only: mirror of `audio.route_switch_microphone` backing the
    /// page's toggle. The config value is only ever written from this page, so
    /// a plain mirror (no reactive resync) is enough.
    route_switch_microphone: bool,
}

#[derive(Debug)]
pub(crate) enum BarPillSettingsInput {
    /// Audio Route only: "also switch the microphone" toggled.
    SetRouteSwitchMicrophone(bool),
}

#[derive(Debug)]
pub(crate) enum BarPillSettingsOutput {}

pub(crate) struct BarPillSettingsInit {
    pub(crate) kind: BarPillKind,
}

#[relm4::component(pub(crate))]
impl Component for BarPillSettingsModel {
    type CommandOutput = ();
    type Input = BarPillSettingsInput;
    type Output = BarPillSettingsOutput;
    type Init = BarPillSettingsInit;

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
                        set_icon_name: Some("view-grid-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Bar pill",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Per-pill placement and behaviour.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    #[watch]
                    set_label: model.kind.display_name(),
                    set_halign: gtk::Align::Start,
                },

                gtk::Label {
                    add_css_class: "label-medium",
                    #[watch]
                    set_label: model.kind.description(),
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },

                // Audio Route is the one bar-only pill with behaviour worth
                // tuning, so it gets a real control here. Hidden for every
                // other kind, keeping the generic page a plain info page.
                gtk::Box {
                    set_visible: model.kind == BarPillKind::AudioRoute,
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 8,

                    gtk::Label {
                        add_css_class: "label-large-bold",
                        set_label: "Options",
                        set_halign: gtk::Align::Start,
                    },

                    gtk::Box {
                        add_css_class: "boxed-list",
                        set_orientation: gtk::Orientation::Vertical,

                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 12,
                            set_margin_start: 12,
                            set_margin_end: 12,
                            set_margin_top: 8,
                            set_margin_bottom: 8,

                            gtk::Box {
                                set_orientation: gtk::Orientation::Vertical,
                                set_hexpand: true,
                                gtk::Label {
                                    add_css_class: "label-medium",
                                    set_label: "Switch the microphone too",
                                    set_halign: gtk::Align::Start,
                                },
                                gtk::Label {
                                    add_css_class: "label-small",
                                    set_label: "Move the default mic to the headset as well, not just the speaker. Turn off to keep your current capture device (e.g. a good USB mic).",
                                    set_halign: gtk::Align::Start,
                                    set_xalign: 0.0,
                                    set_wrap: true,
                                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                                },
                            },

                            gtk::Switch {
                                set_valign: gtk::Align::Center,
                                set_active: model.route_switch_microphone,
                                connect_state_set[sender] => move |_, on| {
                                    sender.input(BarPillSettingsInput::SetRouteSwitchMicrophone(on));
                                    gtk::glib::Propagation::Proceed
                                },
                            },
                        },
                    },
                },

                gtk::Separator {},

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Placement",
                    set_halign: gtk::Align::Start,
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "Bar pills don't have their own menu surface — they live in the bar itself. To change which side of the bar this widget appears on (start / center / end) or to add / remove it, head to Bar → Top or Bottom bar widget lists.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let route_switch_microphone = config_manager()
            .config()
            .audio()
            .route_switch_microphone()
            .get_untracked();
        let model = BarPillSettingsModel {
            kind: params.kind,
            route_switch_microphone,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            BarPillSettingsInput::SetRouteSwitchMicrophone(on) => {
                self.route_switch_microphone = on;
                config_manager().update_config(|c| c.audio.route_switch_microphone = on);
            }
        }
    }
}
