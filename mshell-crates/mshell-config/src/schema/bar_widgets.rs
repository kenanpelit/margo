use reactive_stores::{KeyMap, PatchField, Store, StorePath};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, JsonSchema)]
pub enum BarWidget {
    ActiveWindow,
    AudioInput,
    AudioOutput,
    Battery,
    Bluetooth,
    Clipboard,
    Clock,
    CpuMonitor,
    CpuTemp,
    DarkMode,
    KeepAwake,
    LockKeys,
    MargoDock,
    RamMonitor,
    MargoLayoutSwitcher,
    MargoTags,
    HyprPicker,
    Lock,
    Logout,
    MediaPlayer,
    Ndns,
    Network,
    Nip,
    Nnetwork,
    Nnotes,
    Notifications,
    Npodman,
    Npower,
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
            BarWidget::ActiveWindow => "Active Window",
            BarWidget::AudioInput => "Audio Input",
            BarWidget::AudioOutput => "Audio Output",
            BarWidget::Battery => "Battery",
            BarWidget::Bluetooth => "Bluetooth",
            BarWidget::Clipboard => "Clipboard",
            BarWidget::Clock => "Clock",
            BarWidget::DarkMode => "Dark Mode Toggle",
            BarWidget::CpuMonitor => "CPU Load",
            BarWidget::CpuTemp => "CPU Temperature",
            BarWidget::KeepAwake => "Keep Awake",
            BarWidget::LockKeys => "Lock Keys (Caps/Num/Scroll)",
            BarWidget::MargoDock => "Margo Dock",
            BarWidget::RamMonitor => "RAM Used",
            BarWidget::MargoLayoutSwitcher => "Margo Layout Switcher",
            BarWidget::MargoTags => "Margo Tags",
            BarWidget::HyprPicker => "HyprPicker",
            BarWidget::Lock => "Lock",
            BarWidget::Logout => "Logout",
            BarWidget::MediaPlayer => "Media Player",
            BarWidget::Ndns => "DNS / VPN",
            BarWidget::Network => "Network",
            BarWidget::Nip => "Public IP",
            BarWidget::Nnetwork => "Network Console",
            BarWidget::Nnotes => "Notes Hub",
            BarWidget::Notifications => "Notifications",
            BarWidget::Npodman => "Podman",
            BarWidget::Npower => "Power Profile Menu",
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
            BarWidget::ActiveWindow,
            BarWidget::AudioInput,
            BarWidget::AudioOutput,
            BarWidget::Battery,
            BarWidget::Bluetooth,
            BarWidget::Clipboard,
            BarWidget::Clock,
            BarWidget::CpuMonitor,
            BarWidget::CpuTemp,
            BarWidget::DarkMode,
            BarWidget::KeepAwake,
            BarWidget::LockKeys,
            BarWidget::MargoDock,
            BarWidget::RamMonitor,
            BarWidget::MargoLayoutSwitcher,
            BarWidget::MargoTags,
            BarWidget::HyprPicker,
            BarWidget::Lock,
            BarWidget::Logout,
            BarWidget::MediaPlayer,
            BarWidget::Ndns,
            BarWidget::Network,
            BarWidget::Nip,
            BarWidget::Nnetwork,
            BarWidget::Nnotes,
            BarWidget::Notifications,
            BarWidget::Npodman,
            BarWidget::Npower,
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
