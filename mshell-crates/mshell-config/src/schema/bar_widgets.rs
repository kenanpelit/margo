use reactive_stores::{KeyMap, PatchField, Store, StorePath};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, JsonSchema)]
pub enum BarWidget {
    AudioInput,
    AudioOutput,
    Battery,
    Bluetooth,
    Clipboard,
    Clock,
    MargoDock,
    MargoLayoutSwitcher,
    MargoTags,
    HyprPicker,
    Lock,
    Logout,
    Ndns,
    Network,
    Nip,
    Notifications,
    Npodman,
    Nufw,
    PowerProfile,
    QuickSettings,
    Reboot,
    RecordingIndicator,
    Screenshot,
    Shutdown,
    Tray,
    VpnIndicator,
    Wallpaper,
}

impl PatchField for BarWidget {
    fn patch_field(
        &mut self,
        new: Self,
        path: &StorePath,
        notify: &mut dyn FnMut(&StorePath),
        _keys: Option<&KeyMap>,
    ) {
        if *self != new {
            *self = new;
            notify(path);
        }
    }
}

impl BarWidget {
    pub fn display_name(&self) -> &'static str {
        match self {
            BarWidget::AudioInput => "Audio Input",
            BarWidget::AudioOutput => "Audio Output",
            BarWidget::Battery => "Battery",
            BarWidget::Bluetooth => "Bluetooth",
            BarWidget::Clipboard => "Clipboard",
            BarWidget::Clock => "Clock",
            BarWidget::MargoDock => "Margo Dock",
            BarWidget::MargoLayoutSwitcher => "Margo Layout Switcher",
            BarWidget::MargoTags => "Margo Tags",
            BarWidget::HyprPicker => "HyprPicker",
            BarWidget::Lock => "Lock",
            BarWidget::Logout => "Logout",
            BarWidget::Ndns => "DNS / VPN",
            BarWidget::Network => "Network",
            BarWidget::Nip => "Public IP",
            BarWidget::Notifications => "Notifications",
            BarWidget::Npodman => "Podman",
            BarWidget::Nufw => "UFW Firewall",
            BarWidget::PowerProfile => "Power Profile",
            BarWidget::QuickSettings => "Quick Settings",
            BarWidget::Reboot => "Reboot",
            BarWidget::RecordingIndicator => "Recording Indicator",
            BarWidget::Screenshot => "Screenshot",
            BarWidget::Shutdown => "Shutdown",
            BarWidget::Tray => "Tray",
            BarWidget::VpnIndicator => "VPN Indicator",
            BarWidget::Wallpaper => "Wallpaper",
        }
    }

    pub fn action_name(&self) -> String {
        format!("{:?}", self).to_lowercase().replace(' ', "-")
    }

    pub fn all() -> &'static [BarWidget] {
        &[
            BarWidget::AudioInput,
            BarWidget::AudioOutput,
            BarWidget::Battery,
            BarWidget::Bluetooth,
            BarWidget::Clipboard,
            BarWidget::Clock,
            BarWidget::MargoDock,
            BarWidget::MargoLayoutSwitcher,
            BarWidget::MargoTags,
            BarWidget::HyprPicker,
            BarWidget::Lock,
            BarWidget::Logout,
            BarWidget::Ndns,
            BarWidget::Network,
            BarWidget::Nip,
            BarWidget::Notifications,
            BarWidget::Npodman,
            BarWidget::Nufw,
            BarWidget::PowerProfile,
            BarWidget::QuickSettings,
            BarWidget::Reboot,
            BarWidget::RecordingIndicator,
            BarWidget::Screenshot,
            BarWidget::Shutdown,
            BarWidget::Tray,
            BarWidget::VpnIndicator,
            BarWidget::Wallpaper,
        ]
    }
}
