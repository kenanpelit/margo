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
    /// Twilight (built-in blue-light filter) pill. Left-click opens
    /// the Twilight panel (toggle + temperature + mode + presets);
    /// right-click flips the filter on/off. State polled from
    /// `mctl twilight status`.
    Twilight,
    KeepAwake,
    LockKeys,
    MargoDock,
    MargoLayoutSwitcher,
    MargoTags,
    ColorPicker,
    Lock,
    Logout,
    MediaPlayer,
    Dns,
    Ip,
    Network,
    Notes,
    Notifications,
    Podman,
    Power,
    Ufw,
    Privacy,
    Reboot,
    RecordingIndicator,
    Screenshot,
    Shutdown,
    SystemUpdate,
    Valent,
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
            BarWidget::Bluetooth => "Bluetooth",
            BarWidget::Clipboard => "Clipboard",
            BarWidget::Clock => "Clock",
            BarWidget::CpuDashboard => "CPU Dashboard",
            BarWidget::Dashboard => "Dashboard",
            BarWidget::DarkMode => "Dark Mode Toggle",
            BarWidget::Twilight => "Twilight (blue-light filter)",
            BarWidget::KeepAwake => "Keep Awake",
            BarWidget::LockKeys => "Lock Keys (Caps/Num/Scroll)",
            BarWidget::MargoDock => "Margo Dock",
            BarWidget::MargoLayoutSwitcher => "Margo Layout Switcher",
            BarWidget::MargoTags => "Margo Tags",
            BarWidget::ColorPicker => "ColorPicker",
            BarWidget::Lock => "Lock",
            BarWidget::Logout => "Logout",
            BarWidget::MediaPlayer => "Media Player",
            BarWidget::Dns => "DNS / VPN",
            BarWidget::Ip => "Public IP",
            BarWidget::Network => "Network Console",
            BarWidget::Notes => "Notes Hub",
            BarWidget::Notifications => "Notifications",
            BarWidget::Podman => "Podman",
            BarWidget::Power => "Power Profile",
            BarWidget::Ufw => "UFW Firewall",
            BarWidget::Privacy => "Privacy",
            BarWidget::Reboot => "Reboot",
            BarWidget::RecordingIndicator => "Recording Indicator",
            BarWidget::Screenshot => "Screenshot",
            BarWidget::Shutdown => "Shutdown",
            BarWidget::SystemUpdate => "System Updates",
            BarWidget::Valent => "Valent Connect",
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
            BarWidget::Bluetooth,
            BarWidget::Clipboard,
            BarWidget::Clock,
            BarWidget::CpuDashboard,
            BarWidget::Dashboard,
            BarWidget::DarkMode,
            BarWidget::Twilight,
            BarWidget::KeepAwake,
            BarWidget::LockKeys,
            BarWidget::MargoDock,
            BarWidget::MargoLayoutSwitcher,
            BarWidget::MargoTags,
            BarWidget::ColorPicker,
            BarWidget::Lock,
            BarWidget::Logout,
            BarWidget::MediaPlayer,
            BarWidget::Dns,
            BarWidget::Ip,
            BarWidget::Network,
            BarWidget::Notes,
            BarWidget::Notifications,
            BarWidget::Podman,
            BarWidget::Power,
            BarWidget::Ufw,
            BarWidget::Privacy,
            BarWidget::Reboot,
            BarWidget::RecordingIndicator,
            BarWidget::Screenshot,
            BarWidget::Shutdown,
            BarWidget::SystemUpdate,
            BarWidget::Valent,
            BarWidget::Tray,
            BarWidget::VpnIndicator,
            BarWidget::Wallpaper,
        ]
    }
}
