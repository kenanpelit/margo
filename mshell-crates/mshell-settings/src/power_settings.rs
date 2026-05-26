//! Settings → Power page.
//!
//! Sections:
//!   * **Battery** — icon, percentage, state label, power source.
//!     Hidden when no battery device is present (desktop systems).
//!   * **Power profiles** — DropDown over Power Saver / Balanced /
//!     Performance. Hidden when power-profiles-daemon is absent.
//!   * **Automatic suspend** — Switch + SpinButton, editing the
//!     `idle.suspend_enabled` / `idle.suspend_timeout_minutes` keys
//!     (shared with the Idle page).
//!   * **Low-battery warning** — Switch + SpinButton for
//!     `power.low_battery_warning` / `power.low_battery_threshold`.
//!
//! Watcher discipline: battery watcher fires `BatteryChanged`,
//! line-power watcher fires `OnlineChanged`, profile watcher fires
//! `ProfileChanged`. All re-read service state in `update_cmd` and
//! forward a `RefreshState` input so the view re-renders.

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, IdleStoreFields, PowerConfigStoreFields};
use mshell_launcher::notify;
use mshell_services::{battery_service, line_power_service, power_profile_service};
use mshell_utils::battery::{
    get_battery_icon, get_charging_battery_icon, spawn_battery_online_watcher,
    spawn_battery_watcher,
};
use mshell_utils::power_profile::spawn_active_profile_watcher;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

use crate::sys;
use wayle_battery::types::DeviceState;
use wayle_power_profiles::types::profile::PowerProfile;

// ── Profile enum (local mirror of the bar widget's) ──────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Profile {
    PowerSaver,
    Balanced,
    Performance,
    Unknown,
}

impl Profile {
    fn from_wayle(p: &PowerProfile) -> Self {
        match p {
            PowerProfile::PowerSaver => Profile::PowerSaver,
            PowerProfile::Balanced => Profile::Balanced,
            PowerProfile::Performance => Profile::Performance,
            PowerProfile::Unknown => Profile::Unknown,
        }
    }

    fn to_wayle(self) -> PowerProfile {
        match self {
            Profile::PowerSaver => PowerProfile::PowerSaver,
            Profile::Balanced => PowerProfile::Balanced,
            Profile::Performance => PowerProfile::Performance,
            Profile::Unknown => PowerProfile::Balanced,
        }
    }

    fn to_index(self) -> u32 {
        match self {
            Profile::PowerSaver => 0,
            Profile::Balanced => 1,
            Profile::Performance => 2,
            Profile::Unknown => 1,
        }
    }

    fn from_index(idx: u32) -> Self {
        match idx {
            0 => Profile::PowerSaver,
            2 => Profile::Performance,
            _ => Profile::Balanced,
        }
    }
}

// ── Model ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) struct PowerSettingsModel {
    // Battery state
    battery_available: bool,
    battery_percent: u8,
    battery_status: String,
    on_ac: bool,
    // Profile (None = ppd unavailable)
    profile: Option<Profile>,
    // Automatic suspend (shared with Idle page)
    suspend_enabled: bool,
    suspend_timeout: u32,
    // Low-battery warning
    low_battery_warning: bool,
    low_battery_threshold: u32,
    // Low-battery toast state — resets when charging or above threshold
    warned: bool,
    // Logind power-button / lid handlers
    power_key_idx: u32,
    lid_idx: u32,
    lid_external_idx: u32,
    // EffectScope keeps config-watcher effects alive for the lifetime
    // of this component.
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum PowerSettingsInput {
    // Live-update from watchers
    RefreshState,
    // Automatic suspend
    SuspendEnabledChanged(bool),
    SuspendTimeoutChanged(u32),
    // Low-battery warning
    LowBatteryWarningChanged(bool),
    LowBatteryThresholdChanged(u32),
    // Power profile dropdown
    ProfileSelected(u32),
    // Effects from config reactive store
    SuspendEnabledEffect(bool),
    SuspendTimeoutEffect(u32),
    LowBatteryWarningEffect(bool),
    LowBatteryThresholdEffect(u32),
    // Logind handlers loaded asynchronously
    LogindLoaded(sys::logind::LogindHandlers),
    // Logind handler selectors
    SetPowerKey(u32),
    SetLid(u32),
    SetLidExternal(u32),
}

#[derive(Debug)]
pub(crate) enum PowerSettingsOutput {}

pub(crate) struct PowerSettingsInit {}

#[derive(Debug)]
pub(crate) enum PowerSettingsCommandOutput {
    BatteryChanged,
    OnlineChanged,
    ProfileChanged,
}

// ── Component ─────────────────────────────────────────────────────────────────

#[relm4::component(pub)]
impl Component for PowerSettingsModel {
    type CommandOutput = PowerSettingsCommandOutput;
    type Input = PowerSettingsInput;
    type Output = PowerSettingsOutput;
    type Init = PowerSettingsInit;

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

                // ── Hero ──────────────────────────────────────────
                gtk::Box {
                    add_css_class: "settings-hero",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Start,
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("battery-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Power",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Battery status, power profile, idle suspend, and low-battery alerts.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                // ── Battery ───────────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Battery",
                    set_halign: gtk::Align::Start,
                    #[watch]
                    set_visible: model.battery_available,
                },

                // Battery info row
                #[name = "battery_row"]
                gtk::Box {
                    add_css_class: "power-battery-row",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    #[watch]
                    set_visible: model.battery_available,

                    #[name = "battery_icon"]
                    gtk::Image {
                        set_valign: gtk::Align::Center,
                        set_pixel_size: 32,
                    },

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        set_hexpand: true,

                        #[name = "battery_percent_label"]
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                        },

                        #[name = "battery_status_label"]
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },

                        #[name = "battery_source_label"]
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                // Battery not present banner
                gtk::Box {
                    add_css_class: "power-no-battery",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    set_halign: gtk::Align::Start,
                    #[watch]
                    set_visible: !model.battery_available,

                    gtk::Image {
                        set_icon_name: Some("battery-missing-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "No battery detected — running on AC or desktop.",
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                        set_wrap: true,
                    },
                },

                // ── Power Profiles ─────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Power Profile",
                    set_halign: gtk::Align::Start,
                    #[watch]
                    set_visible: model.profile.is_some(),
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    #[watch]
                    set_visible: model.profile.is_some(),

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Active profile",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Managed by power-profiles-daemon over D-Bus.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    #[name = "profile_dropdown"]
                    gtk::DropDown {
                        set_valign: gtk::Align::Center,
                        set_model: Some(&gtk::StringList::new(&["Power Saver", "Balanced", "Performance"])),
                        #[watch]
                        #[block_signal(profile_selected_handler)]
                        set_selected: model.profile.map(|p| p.to_index()).unwrap_or(1),
                        connect_selected_notify[sender] => move |dd| {
                            sender.input(PowerSettingsInput::ProfileSelected(dd.selected()));
                        } @profile_selected_handler,
                    },
                },

                // ppd unavailable note
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    set_halign: gtk::Align::Start,
                    #[watch]
                    set_visible: model.profile.is_none(),

                    gtk::Image {
                        set_icon_name: Some("dialog-information-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "power-profiles-daemon is not available on this system.",
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                        set_wrap: true,
                    },
                },

                // ── Automatic Suspend ──────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Automatic Suspend",
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
                            set_label: "Suspend the system (systemctl suspend) when idle.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(suspend_enabled_handler)]
                        set_active: model.suspend_enabled,
                        connect_state_set[sender] => move |_, enabled| {
                            sender.input(PowerSettingsInput::SuspendEnabledChanged(enabled));
                            glib::Propagation::Proceed
                        } @suspend_enabled_handler,
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
                            set_label: "Timeout (minutes)",
                            set_hexpand: true,
                        },
                    },

                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (1.0, 1440.0),
                        set_increments: (1.0, 5.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(suspend_timeout_handler)]
                        set_value: model.suspend_timeout as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(PowerSettingsInput::SuspendTimeoutChanged(s.value() as u32));
                        } @suspend_timeout_handler,
                    },
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "Shared with the Idle page.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                },

                // ── Low-battery warning ────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Low-Battery Warning",
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
                            set_label: "Show a notification when battery falls to or below the threshold.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(low_battery_warning_handler)]
                        set_active: model.low_battery_warning,
                        connect_state_set[sender] => move |_, enabled| {
                            sender.input(PowerSettingsInput::LowBatteryWarningChanged(enabled));
                            glib::Propagation::Proceed
                        } @low_battery_warning_handler,
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
                            set_label: "Threshold (%)",
                            set_hexpand: true,
                        },
                    },

                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (1.0, 100.0),
                        set_increments: (1.0, 5.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(low_battery_threshold_handler)]
                        set_value: model.low_battery_threshold as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(PowerSettingsInput::LowBatteryThresholdChanged(s.value() as u32));
                        } @low_battery_threshold_handler,
                    },
                },

                // ── Power Button & Lid ─────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Power Button & Lid",
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
                            set_label: "Power button",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Action when the power button is pressed.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    #[name = "power_key_dropdown"]
                    gtk::DropDown {
                        set_valign: gtk::Align::Center,
                        set_model: Some(&gtk::StringList::new(&["Do nothing", "Power off", "Suspend", "Hibernate", "Lock"])),
                        #[watch]
                        #[block_signal(power_key_handler)]
                        set_selected: model.power_key_idx,
                        connect_selected_notify[sender] => move |dd| {
                            sender.input(PowerSettingsInput::SetPowerKey(dd.selected()));
                        } @power_key_handler,
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
                            set_label: "Lid close (on battery)",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Action when the laptop lid is closed while on battery.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    #[name = "lid_dropdown"]
                    gtk::DropDown {
                        set_valign: gtk::Align::Center,
                        set_model: Some(&gtk::StringList::new(&["Do nothing", "Power off", "Suspend", "Hibernate", "Lock"])),
                        #[watch]
                        #[block_signal(lid_handler)]
                        set_selected: model.lid_idx,
                        connect_selected_notify[sender] => move |dd| {
                            sender.input(PowerSettingsInput::SetLid(dd.selected()));
                        } @lid_handler,
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
                            set_label: "Lid close (on AC)",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Action when the laptop lid is closed while plugged in.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    #[name = "lid_external_dropdown"]
                    gtk::DropDown {
                        set_valign: gtk::Align::Center,
                        set_model: Some(&gtk::StringList::new(&["Do nothing", "Power off", "Suspend", "Hibernate", "Lock"])),
                        #[watch]
                        #[block_signal(lid_external_handler)]
                        set_selected: model.lid_external_idx,
                        connect_selected_notify[sender] => move |dd| {
                            sender.input(PowerSettingsInput::SetLidExternal(dd.selected()));
                        } @lid_external_handler,
                    },
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "Applies on next login.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        _root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // Set up config reactive effects
        let mut effects = EffectScope::new();

        macro_rules! push_effect {
            ($section:ident, $field:ident, $variant:ident) => {{
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let value = config_manager().config().$section().$field().get();
                    sender_clone.input(PowerSettingsInput::$variant(value));
                });
            }};
        }
        push_effect!(idle, suspend_enabled, SuspendEnabledEffect);
        push_effect!(idle, suspend_timeout_minutes, SuspendTimeoutEffect);
        push_effect!(power, low_battery_warning, LowBatteryWarningEffect);
        push_effect!(power, low_battery_threshold, LowBatteryThresholdEffect);

        // D-Bus service watchers
        spawn_battery_watcher(&sender, || PowerSettingsCommandOutput::BatteryChanged);
        spawn_battery_online_watcher(&sender, || PowerSettingsCommandOutput::OnlineChanged);
        spawn_active_profile_watcher(&sender, None, || PowerSettingsCommandOutput::ProfileChanged);

        // Load logind handlers asynchronously
        {
            let s = sender.clone();
            glib::spawn_future_local(async move {
                s.input(PowerSettingsInput::LogindLoaded(
                    sys::logind::read_handlers().await,
                ));
            });
        }

        let model = PowerSettingsModel {
            battery_available: read_battery_available(),
            battery_percent: read_battery_percent(),
            battery_status: read_battery_status(),
            on_ac: read_on_ac(),
            profile: read_profile(),
            suspend_enabled: config_manager()
                .config()
                .idle()
                .suspend_enabled()
                .get_untracked(),
            suspend_timeout: config_manager()
                .config()
                .idle()
                .suspend_timeout_minutes()
                .get_untracked(),
            low_battery_warning: config_manager()
                .config()
                .power()
                .low_battery_warning()
                .get_untracked(),
            low_battery_threshold: config_manager()
                .config()
                .power()
                .low_battery_threshold()
                .get_untracked(),
            warned: false,
            // Logind handlers — populated async via LogindLoaded; default until then
            power_key_idx: idx_of("poweroff"),
            lid_idx: idx_of("suspend"),
            lid_external_idx: idx_of("suspend"),
            _effects: effects,
        };

        let widgets = view_output!();

        // Apply battery icon + labels imperatively (can't #[watch] an
        // imperative set_icon_name that depends on multiple model fields).
        apply_battery_visuals(&widgets, &model);

        ComponentParts { model, widgets }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            PowerSettingsCommandOutput::BatteryChanged
            | PowerSettingsCommandOutput::OnlineChanged
            | PowerSettingsCommandOutput::ProfileChanged => {
                self.battery_available = read_battery_available();
                self.battery_percent = read_battery_percent();
                self.battery_status = read_battery_status();
                self.on_ac = read_on_ac();
                self.profile = read_profile();

                // Low-battery toast
                let enabled = self.low_battery_warning;
                let threshold = self.low_battery_threshold;
                let pct = self.battery_percent;

                if self.on_ac || (pct as u32) > threshold {
                    self.warned = false;
                } else if enabled && !self.on_ac && (pct as u32) <= threshold && !self.warned {
                    self.warned = true;
                    notify::toast(
                        "Battery low",
                        format!("{}% remaining", pct),
                    );
                }

                sender.input(PowerSettingsInput::RefreshState);
            }
        }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            PowerSettingsInput::RefreshState => {
                // Model already updated in update_cmd; just let
                // update_view below re-render the watched properties.
            }

            PowerSettingsInput::SuspendEnabledChanged(v) => {
                config_manager().update_config(|c| c.idle.suspend_enabled = v);
            }
            PowerSettingsInput::SuspendTimeoutChanged(v) => {
                config_manager().update_config(|c| c.idle.suspend_timeout_minutes = v);
            }
            PowerSettingsInput::LowBatteryWarningChanged(v) => {
                config_manager().update_config(|c| c.power.low_battery_warning = v);
            }
            PowerSettingsInput::LowBatteryThresholdChanged(v) => {
                config_manager().update_config(|c| c.power.low_battery_threshold = v);
            }

            PowerSettingsInput::ProfileSelected(idx) => {
                let profile = Profile::from_index(idx);
                tokio::spawn(async move {
                    if let Err(e) = power_profile_service()
                        .power_profiles
                        .set_active_profile(profile.to_wayle())
                        .await
                    {
                        tracing::warn!(error = %e, "power-settings: set_active_profile failed");
                    }
                });
            }

            // Effects — update model, let update_view re-render
            PowerSettingsInput::SuspendEnabledEffect(v) => self.suspend_enabled = v,
            PowerSettingsInput::SuspendTimeoutEffect(v) => self.suspend_timeout = v,
            PowerSettingsInput::LowBatteryWarningEffect(v) => self.low_battery_warning = v,
            PowerSettingsInput::LowBatteryThresholdEffect(v) => self.low_battery_threshold = v,

            // Logind handlers
            PowerSettingsInput::LogindLoaded(h) => {
                self.power_key_idx = idx_of(&h.power_key);
                self.lid_idx = idx_of(&h.lid);
                self.lid_external_idx = idx_of(&h.lid_external);
            }

            PowerSettingsInput::SetPowerKey(idx) => {
                self.power_key_idx = idx;
                let h = self.build_handlers();
                glib::spawn_future_local(async move {
                    if let Err(e) = sys::logind::write_dropin(&h).await {
                        notify::toast("Power", &e);
                    }
                });
            }

            PowerSettingsInput::SetLid(idx) => {
                self.lid_idx = idx;
                let h = self.build_handlers();
                glib::spawn_future_local(async move {
                    if let Err(e) = sys::logind::write_dropin(&h).await {
                        notify::toast("Power", &e);
                    }
                });
            }

            PowerSettingsInput::SetLidExternal(idx) => {
                self.lid_external_idx = idx;
                let h = self.build_handlers();
                glib::spawn_future_local(async move {
                    if let Err(e) = sys::logind::write_dropin(&h).await {
                        notify::toast("Power", &e);
                    }
                });
            }
        }

        apply_battery_visuals(widgets, self);
        self.update_view(widgets, sender);
    }
}

// ── Logind helpers ────────────────────────────────────────────────────────────

fn idx_of(s: &str) -> u32 {
    sys::logind::ACTIONS
        .iter()
        .position(|a| *a == s)
        .unwrap_or(0) as u32
}

impl PowerSettingsModel {
    fn build_handlers(&self) -> sys::logind::LogindHandlers {
        sys::logind::LogindHandlers {
            power_key: sys::logind::ACTIONS[self.power_key_idx as usize].into(),
            lid: sys::logind::ACTIONS[self.lid_idx as usize].into(),
            lid_external: sys::logind::ACTIONS[self.lid_external_idx as usize].into(),
        }
    }
}

// ── Imperative visual helpers ─────────────────────────────────────────────────

fn apply_battery_visuals(widgets: &PowerSettingsModelWidgets, model: &PowerSettingsModel) {
    if !model.battery_available {
        return;
    }
    let pct_f = model.battery_percent as f64;
    let icon = if model.on_ac {
        get_charging_battery_icon(pct_f)
    } else {
        get_battery_icon(pct_f)
    };
    widgets.battery_icon.set_icon_name(Some(icon));
    widgets
        .battery_percent_label
        .set_label(&format!("{}%", model.battery_percent));

    let status = if model.battery_status.is_empty() {
        "Unknown".to_string()
    } else {
        model.battery_status.clone()
    };
    widgets.battery_status_label.set_label(&status);

    let source = if model.on_ac {
        "Power source: AC adapter"
    } else {
        "Power source: Battery"
    };
    widgets.battery_source_label.set_label(source);
}

// ── Service read helpers ──────────────────────────────────────────────────────

fn read_battery_available() -> bool {
    battery_service().device.is_present.get()
}

fn read_battery_percent() -> u8 {
    let pct = battery_service().device.percentage.get();
    pct.round().clamp(0.0, 100.0) as u8
}

fn read_battery_status() -> String {
    match battery_service().device.state.get() {
        DeviceState::Charging => "Charging".to_string(),
        DeviceState::Discharging => "Discharging".to_string(),
        DeviceState::FullyCharged => "Full".to_string(),
        DeviceState::Empty => "Empty".to_string(),
        DeviceState::PendingCharge | DeviceState::PendingDischarge => "Not charging".to_string(),
        DeviceState::Unknown => String::new(),
    }
}

fn read_on_ac() -> bool {
    let dev_state = battery_service().device.state.get();
    line_power_service()
        .map(|s| s.device.online.get())
        .unwrap_or(
            dev_state == DeviceState::Charging || dev_state == DeviceState::FullyCharged,
        )
}

fn read_profile() -> Option<Profile> {
    // If power-profiles-daemon is unavailable, `active_profile` returns
    // `PowerProfile::Unknown`. We surface `None` (hidden) in that case so
    // the UI hides the section rather than showing a misleading value.
    let p = power_profile_service()
        .power_profiles
        .active_profile
        .get();
    match p {
        PowerProfile::Unknown => None,
        other => Some(Profile::from_wayle(&other)),
    }
}
