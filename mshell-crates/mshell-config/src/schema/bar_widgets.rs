use reactive_stores::{KeyMap, PatchField, Store, StorePath};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, JsonSchema)]
pub enum BarWidget {
    ActiveWindow,
    /// Combined audio dashboard pill — surfaces both default
    /// output (sink) and default input (source) volumes in one
    /// cluster with right-click cycle (Both/OutputOnly/InputOnly).
    /// Click opens the audio dashboard menu with sliders, mute
    /// toggles, and device pickers for both sides. Replaces the
    /// standalone AudioInput / AudioOutput pills.
    AudioDashboard,
    Battery,
    Bluetooth,
    Clipboard,
    Clock,
    /// Combined CPU dashboard pill — single chip showing live
    /// CPU load + package temperature with threshold-driven
    /// colour states (calm / warn / danger). Left-click opens
    /// the rich CPU dashboard menu with per-core bars + memory
    /// + load averages. Replaces the standalone CpuMonitor /
    /// CpuTemp / RamMonitor pills.
    CpuDashboard,
    /// Compound clock-style pill that opens the **dashboard** menu
    /// (clock hero + calendar + weather + media player + the QS
    /// tile stack). Shares Clock's `[tempo]` format list — the
    /// label cycles through the same chrono-strftime presets on
    /// right-click — so a user who switches from Clock to Dashboard
    /// keeps their preferred date/time wording without any extra
    /// config.
    Dashboard,
    DarkMode,
    KeepAwake,
    LockKeys,
    MargoDock,
    MargoLayoutSwitcher,
    MargoTags,
    ColorPicker,
    Lock,
    Logout,
    MediaPlayer,
    Ndns,
    Nip,
    Nnetwork,
    Nnotes,
    Notifications,
    Npodman,
    Npower,
    Nufw,
    PowerProfile,
    Privacy,
    QuickSettings,
    Reboot,
    RecordingIndicator,
    Screenshot,
    Shutdown,
    SystemUpdate,
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
            BarWidget::AudioDashboard => "Audio Dashboard",
            BarWidget::Battery => "Battery",
            BarWidget::Bluetooth => "Bluetooth",
            BarWidget::Clipboard => "Clipboard",
            BarWidget::Clock => "Clock",
            BarWidget::CpuDashboard => "CPU Dashboard",
            BarWidget::Dashboard => "Dashboard",
            BarWidget::DarkMode => "Dark Mode Toggle",
            BarWidget::KeepAwake => "Keep Awake",
            BarWidget::LockKeys => "Lock Keys (Caps/Num/Scroll)",
            BarWidget::MargoDock => "Margo Dock",
            BarWidget::MargoLayoutSwitcher => "Margo Layout Switcher",
            BarWidget::MargoTags => "Margo Tags",
            BarWidget::ColorPicker => "ColorPicker",
            BarWidget::Lock => "Lock",
            BarWidget::Logout => "Logout",
            BarWidget::MediaPlayer => "Media Player",
            BarWidget::Ndns => "DNS / VPN",
            BarWidget::Nip => "Public IP",
            BarWidget::Nnetwork => "Network Console",
            BarWidget::Nnotes => "Notes Hub",
            BarWidget::Notifications => "Notifications",
            BarWidget::Npodman => "Podman",
            BarWidget::Npower => "Power Profile Menu",
            BarWidget::Nufw => "UFW Firewall",
            BarWidget::PowerProfile => "Power Profile",
            BarWidget::Privacy => "Privacy",
            BarWidget::QuickSettings => "Quick Settings",
            BarWidget::Reboot => "Reboot",
            BarWidget::RecordingIndicator => "Recording Indicator",
            BarWidget::Screenshot => "Screenshot",
            BarWidget::Shutdown => "Shutdown",
            BarWidget::SystemUpdate => "System Updates",
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
            BarWidget::AudioDashboard,
            BarWidget::Battery,
            BarWidget::Bluetooth,
            BarWidget::Clipboard,
            BarWidget::Clock,
            BarWidget::CpuDashboard,
            BarWidget::Dashboard,
            BarWidget::DarkMode,
            BarWidget::KeepAwake,
            BarWidget::LockKeys,
            BarWidget::MargoDock,
            BarWidget::MargoLayoutSwitcher,
            BarWidget::MargoTags,
            BarWidget::ColorPicker,
            BarWidget::Lock,
            BarWidget::Logout,
            BarWidget::MediaPlayer,
            BarWidget::Ndns,
            BarWidget::Nip,
            BarWidget::Nnetwork,
            BarWidget::Nnotes,
            BarWidget::Notifications,
            BarWidget::Npodman,
            BarWidget::Npower,
            BarWidget::Nufw,
            BarWidget::PowerProfile,
            BarWidget::Privacy,
            BarWidget::QuickSettings,
            BarWidget::Reboot,
            BarWidget::RecordingIndicator,
            BarWidget::Screenshot,
            BarWidget::Shutdown,
            BarWidget::SystemUpdate,
            BarWidget::Tray,
            BarWidget::VpnIndicator,
            BarWidget::Wallpaper,
        ]
    }
}
