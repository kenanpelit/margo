//! Per-bar-pill info pages.
//!
//! Bar-only widgets (Active Window, Margo Tags, Battery, etc.)
//! don't have their own menu surface — they're just pills that
//! live in the bar. Their placement is driven by the Bar's
//! widget list (Bar → Top / Bottom widget arrays), not a
//! position/min-width pair like menu surfaces have.
//!
//! These pages surface the widget so the Settings sidebar is
//! complete, and pointer users to the right place to edit its
//! placement. Future per-pill knobs (e.g. Battery's percentage
//! visibility, Active Window's title truncation length) land
//! here without a new file — extend the view with a match arm
//! against `BarPillKind`.

use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BarPillKind {
    ActiveWindow,
    AudioInput,
    AudioOutput,
    Battery,
    Bluetooth,
    DarkMode,
    HyprPicker,
    KeepAwake,
    Lock,
    Logout,
    MargoDock,
    MargoLayoutSwitcher,
    MargoTags,
    Network,
    PowerProfile,
    Reboot,
    RecordingIndicator,
    Shutdown,
    Tray,
    VpnIndicator,
}

impl BarPillKind {
    pub(crate) fn display_name(self) -> &'static str {
        match self {
            Self::ActiveWindow => "Active Window",
            Self::AudioInput => "Audio Input",
            Self::AudioOutput => "Audio Output",
            Self::Battery => "Battery",
            Self::Bluetooth => "Bluetooth",
            Self::DarkMode => "Dark Mode Toggle",
            Self::HyprPicker => "HyprPicker",
            Self::KeepAwake => "Keep Awake",
            Self::Lock => "Lock",
            Self::Logout => "Logout",
            Self::MargoDock => "Margo Dock",
            Self::MargoLayoutSwitcher => "Margo Layout Switcher",
            Self::MargoTags => "Margo Tags",
            Self::Network => "Network",
            Self::PowerProfile => "Power Profile",
            Self::Reboot => "Reboot",
            Self::RecordingIndicator => "Recording Indicator",
            Self::Shutdown => "Shutdown",
            Self::Tray => "System Tray",
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
            Self::AudioInput => {
                "Mic input level + mute toggle. Click opens the audio-input menu to pick a source device."
            }
            Self::AudioOutput => {
                "Speaker volume + mute toggle. Click opens the audio-output menu to pick a sink device."
            }
            Self::Battery => {
                "Charge percentage + charging state. Right-click flips between percentage label and minimal icon-only."
            }
            Self::Bluetooth => {
                "Adapter state + connected device count. Click opens the Bluetooth menu for pairing / disconnect."
            }
            Self::DarkMode => {
                "One-click flip between Light and Dark matugen modes. Icon reflects the mode you'd switch *to*."
            }
            Self::HyprPicker => {
                "Picks a colour from the screen and copies hex/rgb to the clipboard. Click to start picking."
            }
            Self::KeepAwake => {
                "Toggles the system idle inhibitor. Active = no auto-lock / suspend / dim. Same backend as `mctl idle inhibit`."
            }
            Self::Lock => "Locks the session immediately (no confirmation).",
            Self::Logout => "Logs out of the session. Confirms with a dialog.",
            Self::MargoDock => {
                "Pinned-app dock surfaced through margo's foreign-toplevel list. Click to launch / focus a window."
            }
            Self::MargoLayoutSwitcher => {
                "Current tag's tiling layout (tile / scroller / monocle / dwindle / …). Click cycles forward."
            }
            Self::MargoTags => {
                "1–9 tag pills with focus / occupied / urgent states. Click to switch tags, scroll to cycle."
            }
            Self::Network => {
                "Connectivity state (wired / wifi / offline). Click opens the network menu for SSID selection."
            }
            Self::PowerProfile => {
                "power-profiles-daemon state (Performance / Balanced / Power Saver). Click cycles forward."
            }
            Self::Reboot => "Reboots the system. Confirms with a dialog.",
            Self::RecordingIndicator => {
                "Lights up while a screen-recording is in progress. Click stops the recording."
            }
            Self::Shutdown => "Powers off the system. Confirms with a dialog.",
            Self::Tray => {
                "Hosts StatusNotifierItem clients (Discord, Steam, syncthing, …). Each app paints its own icon."
            }
            Self::VpnIndicator => {
                "Visual cue when a VPN tunnel is up (NetworkManager / wg-quick / openvpn)."
            }
        }
    }
}

pub(crate) struct BarPillSettingsModel {
    kind: BarPillKind,
}

#[derive(Debug)]
pub(crate) enum BarPillSettingsInput {}

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
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = BarPillSettingsModel { kind: params.kind };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        _message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
    }
}
