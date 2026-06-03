//! Privacy pill settings — choose what the indicator watches (mic /
//! camera / screen-share), how it behaves (hide-when-idle, toasts,
//! accent), ignore filters, and how long the access log is kept
//! (`bars.widgets.privacy`).

use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, PrivacyAccent,
    PrivacyWidgetConfigStoreFields,
};
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct PrivacyPillSettingsModel {
    track_mic: bool,
    track_camera: bool,
    track_screen: bool,
    hide_inactive: bool,
    enable_toast: bool,
    accent: PrivacyAccent,
    history_limit: u32,
}

#[derive(Debug)]
pub(crate) enum PrivacyPillSettingsInput {
    TrackMic(bool),
    TrackCamera(bool),
    TrackScreen(bool),
    HideInactive(bool),
    EnableToast(bool),
    Accent(u32),
    MicFilter(String),
    CamFilter(String),
    HistoryLimit(u32),
}

#[derive(Debug)]
pub(crate) enum PrivacyPillSettingsOutput {}

pub(crate) struct PrivacyPillSettingsInit {}

macro_rules! read {
    ($f:ident) => {
        config_manager()
            .config()
            .bars()
            .widgets()
            .privacy()
            .$f()
            .get_untracked()
    };
}

#[relm4::component(pub)]
impl Component for PrivacyPillSettingsModel {
    type CommandOutput = ();
    type Input = PrivacyPillSettingsInput;
    type Output = PrivacyPillSettingsOutput;
    type Init = PrivacyPillSettingsInit;

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
                        set_icon_name: Some("security-high-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_halign: gtk::Align::Start,
                            set_label: "Privacy",
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_halign: gtk::Align::Start,
                            set_label: "A watchdog pill that lights up when an app uses your microphone, a camera, or screen-shares. Left-click it for the access log.",
                            set_wrap: true,
                            set_xalign: 0.0,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "What to watch",
                    set_halign: gtk::Align::Start,
                },

                // Track microphone
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Microphone",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Apps recording audio (via PipeWire's capture-stream list — no overhead).",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(mic_handler)]
                        set_active: model.track_mic,
                        connect_state_set[sender] => move |_, v| {
                            sender.input(PrivacyPillSettingsInput::TrackMic(v));
                            glib::Propagation::Proceed
                        } @mic_handler,
                    },
                },

                // Track camera
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Camera",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Apps holding a /dev/video* capture node open (polled via /proc).",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(cam_handler)]
                        set_active: model.track_camera,
                        connect_state_set[sender] => move |_, v| {
                            sender.input(PrivacyPillSettingsInput::TrackCamera(v));
                            glib::Propagation::Proceed
                        } @cam_handler,
                    },
                },

                // Track screen sharing
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Screen sharing",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Screencast sessions (polls `pw-dump` every 2 s). The heaviest check — turn it off on weak machines.",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(scr_handler)]
                        set_active: model.track_screen,
                        connect_state_set[sender] => move |_, v| {
                            sender.input(PrivacyPillSettingsInput::TrackScreen(v));
                            glib::Propagation::Proceed
                        } @scr_handler,
                    },
                },

                gtk::Separator {},

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Behaviour",
                    set_halign: gtk::Align::Start,
                },

                // Hide when idle
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Hide when idle",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Hide the pill entirely while nothing is in use. Off keeps it visible but dimmed.",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(hide_handler)]
                        set_active: model.hide_inactive,
                        connect_state_set[sender] => move |_, v| {
                            sender.input(PrivacyPillSettingsInput::HideInactive(v));
                            glib::Propagation::Proceed
                        } @hide_handler,
                    },
                },

                // Activation toast
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Activation toast",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Send a notification when a sensor first goes active.",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(toast_handler)]
                        set_active: model.enable_toast,
                        connect_state_set[sender] => move |_, v| {
                            sender.input(PrivacyPillSettingsInput::EnableToast(v));
                            glib::Propagation::Proceed
                        } @toast_handler,
                    },
                },

                // Accent
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Active colour",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Themed accent the glyphs light up with when a sensor is in use.",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    #[name = "accent_dd"]
                    gtk::DropDown {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(accent_handler)]
                        set_selected: accent_index(model.accent),
                        connect_selected_notify[sender] => move |d| {
                            sender.input(PrivacyPillSettingsInput::Accent(d.selected()));
                        } @accent_handler,
                    },
                },

                gtk::Separator {},

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Filters & log",
                    set_halign: gtk::Align::Start,
                },

                // Mic ignore filter
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 4,
                    gtk::Label {
                        add_css_class: "label-medium-bold",
                        set_halign: gtk::Align::Start,
                        set_label: "Microphone ignore filter",
                    },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_halign: gtk::Align::Start,
                        set_label: "Regex of app names to ignore for the mic (e.g. an always-on assistant). Empty = none.",
                        set_xalign: 0.0,
                        set_wrap: true,
                    },
                    #[name = "mic_filter_entry"]
                    gtk::Entry {
                        set_hexpand: true,
                        set_placeholder_text: Some("e.g. easyeffects|noise-suppression"),
                        connect_changed[sender] => move |e| {
                            sender.input(PrivacyPillSettingsInput::MicFilter(e.text().to_string()));
                        },
                    },
                },

                // Cam ignore filter
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 4,
                    gtk::Label {
                        add_css_class: "label-medium-bold",
                        set_halign: gtk::Align::Start,
                        set_label: "Camera ignore filter",
                    },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_halign: gtk::Align::Start,
                        set_label: "Regex of process names to ignore for cameras. Empty = none.",
                        set_xalign: 0.0,
                        set_wrap: true,
                    },
                    #[name = "cam_filter_entry"]
                    gtk::Entry {
                        set_hexpand: true,
                        set_placeholder_text: Some("e.g. pipewire|wireplumber"),
                        connect_changed[sender] => move |e| {
                            sender.input(PrivacyPillSettingsInput::CamFilter(e.text().to_string()));
                        },
                    },
                },

                // History limit
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Access-log entries",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "How many recent started/stopped events to keep (and persist). 0 disables the log.",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (0.0, 500.0),
                        set_increments: (10.0, 50.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(limit_handler)]
                        set_value: model.history_limit as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(PrivacyPillSettingsInput::HistoryLimit(s.value() as u32));
                        } @limit_handler,
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
        let model = PrivacyPillSettingsModel {
            track_mic: read!(track_mic),
            track_camera: read!(track_camera),
            track_screen: read!(detect_screen_share),
            hide_inactive: read!(hide_inactive),
            enable_toast: read!(enable_toast),
            accent: read!(accent),
            history_limit: read!(history_limit),
        };

        let widgets = view_output!();

        // DropDown model + the entries' initial text are set once here
        // (not #[watch], to avoid clobbering the cursor / re-emitting).
        let labels: Vec<&str> = PrivacyAccent::all().iter().map(|a| a.label()).collect();
        widgets
            .accent_dd
            .set_model(Some(&gtk::StringList::new(&labels)));
        widgets.accent_dd.set_selected(accent_index(model.accent));
        widgets.mic_filter_entry.set_text(&read!(mic_filter));
        widgets.cam_filter_entry.set_text(&read!(cam_filter));

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            PrivacyPillSettingsInput::TrackMic(v) => {
                self.track_mic = v;
                config_manager().update_config(move |c| c.bars.widgets.privacy.track_mic = v);
            }
            PrivacyPillSettingsInput::TrackCamera(v) => {
                self.track_camera = v;
                config_manager().update_config(move |c| c.bars.widgets.privacy.track_camera = v);
            }
            PrivacyPillSettingsInput::TrackScreen(v) => {
                self.track_screen = v;
                config_manager()
                    .update_config(move |c| c.bars.widgets.privacy.detect_screen_share = v);
            }
            PrivacyPillSettingsInput::HideInactive(v) => {
                self.hide_inactive = v;
                config_manager().update_config(move |c| c.bars.widgets.privacy.hide_inactive = v);
            }
            PrivacyPillSettingsInput::EnableToast(v) => {
                self.enable_toast = v;
                config_manager().update_config(move |c| c.bars.widgets.privacy.enable_toast = v);
            }
            PrivacyPillSettingsInput::Accent(i) => {
                let accent = PrivacyAccent::all()
                    .get(i as usize)
                    .copied()
                    .unwrap_or(PrivacyAccent::Error);
                self.accent = accent;
                config_manager().update_config(move |c| c.bars.widgets.privacy.accent = accent);
            }
            PrivacyPillSettingsInput::MicFilter(s) => {
                config_manager().update_config(move |c| c.bars.widgets.privacy.mic_filter = s);
            }
            PrivacyPillSettingsInput::CamFilter(s) => {
                config_manager().update_config(move |c| c.bars.widgets.privacy.cam_filter = s);
            }
            PrivacyPillSettingsInput::HistoryLimit(v) => {
                self.history_limit = v;
                config_manager().update_config(move |c| c.bars.widgets.privacy.history_limit = v);
            }
        }
    }
}

/// Index of `accent` within [`PrivacyAccent::all`] for the DropDown.
fn accent_index(accent: PrivacyAccent) -> u32 {
    PrivacyAccent::all()
        .iter()
        .position(|a| *a == accent)
        .unwrap_or(0) as u32
}
