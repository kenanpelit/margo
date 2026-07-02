use reactive_stores::{KeyMap, PatchField, Store, StorePath};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, JsonSchema)]
pub enum BarWidget {
    ActiveWindow,
    /// AI assistant pill — opens the native streaming chat menu
    /// (`MenuType::Ai`). Provider / model / key live in Settings → AI.
    Ai,
    /// Alarm clock pill — alarm-bell glyph that opens the Alarm Clock
    /// menu (alarms list + add/edit + stopwatch). Shows the running
    /// stopwatch time inline when one is going, and a ringing badge
    /// while an alarm tone is sounding.
    AlarmClock,
    /// Countdown pill — a schedule/hourglass glyph + the soonest target's
    /// remaining time ("42 days remaining" / "3 days overdue"). Reads the
    /// (ring-less) `alarm.countdowns` list; click opens the Alarm Clock
    /// menu on its Countdown tab. Hidden when no enabled, parseable target
    /// exists. Port of the DMS `TimeUntil` plugin, folded into the alarm
    /// hub.
    Countdown,
    /// Control Center pill — system-preferences glyph that opens the
    /// Control Center menu.
    ControlCenter,
    /// Hidden Bar — a collapsible "drawer" pill. Renders the widgets
    /// listed in this bar's `hidden_widgets` inside a slide revealer
    /// behind a trigger: hover (when auto-expand) or left-click to
    /// reveal, right-click to pin open, auto-collapse on leave. Port of
    /// the DMS hidden-bar plugin, native to mshell's bar.
    ///
    /// The bare (unit) form is the bar's default drawer — it renders that
    /// bar's `hidden_widgets` list and uses the global `hidden_bar`
    /// behaviour. For additional, independently-addressable drawers (each
    /// with its own widgets + behaviour) use [`BarWidget::HiddenBarNamed`].
    HiddenBar,
    /// A named Hidden Bar drawer. The `String` is the drawer's `name`,
    /// matching an entry in `bars.widgets.hidden_bars`; that entry supplies
    /// the drawer's own widget list and behaviour. Multiple named drawers
    /// can sit in the same bar, and `mshellctl hidden-bar <verb> <name>`
    /// targets one by name. Mirrors the `!Custom <name>` reference pattern.
    HiddenBarNamed(String),
    /// Catwalk — a CPU-reactive animated cat (port of the noctalia plugin).
    /// Idles ("Zz") below a CPU threshold, walks faster as load climbs;
    /// click opens the CPU dashboard.
    Catwalk,
    /// Combined audio dashboard pill — surfaces both default
    /// output (sink) and default input (source) volumes in one
    /// cluster with right-click cycle (Both/OutputOnly/InputOnly).
    /// Click opens the audio dashboard menu with sliders, mute
    /// toggles, and device pickers for both sides. Replaces the
    /// standalone AudioInput / AudioOutput pills.
    AudioDashboard,
    /// Audio Route pill — one click flips the whole default audio path
    /// (both the default input/mic **and** the default output/speaker)
    /// between the built-in device port and a headset/external port, via
    /// `wayle_audio`'s `set_port`. The glyph reflects the current route.
    /// Hidden when neither device exposes ≥2 ports (nothing to switch).
    /// Port of the DMS `Audio Port Switcher` plugin, generalized to route
    /// input + output together.
    AudioRoute,
    /// Audio spectrum visualizer — a strip of live cava-driven bars.
    /// Render-only (no menu); degrades to flat bars if `cava` isn't
    /// installed or there's no audio.
    AudioVisualizer,
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
    /// Compound clock-style pill that opens the **mdash** menu (greeting
    /// hero + calendar + weather + media player + the QS tile stack +
    /// menu-shortcut grid). Shares Clock's `[tempo]` format list — the
    /// label cycles through the same chrono-strftime presets on
    /// right-click — so a user who switches from Clock to Mdash keeps
    /// their preferred date/time wording without any extra config.
    Mdash,
    DarkMode,
    /// Twilight (built-in blue-light filter) pill. Left-click opens
    /// the Twilight panel (toggle + temperature + mode + presets);
    /// right-click flips the filter on/off. State polled from
    /// `mctl twilight status`.
    Twilight,
    KeepAwake,
    /// Keybind cheatsheet pill — a keyboard glyph; click opens a
    /// searchable cheatsheet of every shortcut parsed live from
    /// margo's `config.conf` (`bind = …` lines, including `source`
    /// pulls), grouped by action category.
    Keybinds,
    /// SSH Sessions pill — terminal glyph + live count of active `ssh`
    /// clients; click opens a searchable host list from `~/.ssh/config`
    /// (active first), click a host to connect in a new terminal.
    SshSessions,
    LockKeys,
    /// Keyboard-layout pill — shows the active xkb layout (e.g. "US",
    /// "TR") read from margo's state.json. Click cycles to the next
    /// configured layout via `mctl dispatch cyclekblayout`.
    KeyboardLayout,
    /// On-screen keyboard pill — click toggles `mkeys` (margo's GTK
    /// on-screen keyboard) via `mkeys toggle`.
    Keyboard,
    MargoDock,
    MargoLayoutSwitcher,
    MargoTags,
    ColorPicker,
    Lock,
    Logout,
    /// Opens the Settings panel straight to the Setup page (the in-shell
    /// setup wizard) — a layer-shell surface, not a separate window.
    Setup,
    MediaPlayer,
    /// Lyrics pill — current synced line of the now-playing track. Opens the
    /// scrolling lyrics panel (`MenuType::Lyrics`).
    Lyrics,
    /// Mullvad VPN pill — native status pill driving the `mvpn` binary. Opens
    /// the combined VPN menu (VPN controls + a collapsible DNS section).
    Vpn,
    /// DNS pill — opens the standalone DNS menu (`mshellctl menu dns`); a
    /// dedicated DNS / Blocky entry, separate from the combined `Vpn` pill.
    Dns,
    Ip,
    Network,
    /// A user-defined pill; the `String` is the `custom_widgets` entry name.
    Custom(String),
    /// Blank gap of the given pixel width, for spacing widgets apart.
    Spacer(u32),
    /// A thin vertical divider line between widgets.
    Separator,
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
    Weather,
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
            BarWidget::AlarmClock => "Alarm Clock",
            BarWidget::Countdown => "Countdown",
            BarWidget::ControlCenter => "Control Center",
            BarWidget::HiddenBar => "Hidden Bar",
            BarWidget::HiddenBarNamed(_) => "Hidden Bar",
            BarWidget::Catwalk => "Catwalk (animated cat)",
            BarWidget::AudioDashboard => "Audio Dashboard",
            BarWidget::AudioRoute => "Audio Route",
            BarWidget::AudioVisualizer => "Audio Visualizer",
            BarWidget::Bluetooth => "Bluetooth",
            BarWidget::Clipboard => "Clipboard",
            BarWidget::Clock => "Clock",
            BarWidget::CpuDashboard => "CPU Dashboard",
            BarWidget::Mdash => "Mdash",
            BarWidget::DarkMode => "Dark Mode Toggle",
            BarWidget::Twilight => "Twilight (blue-light filter)",
            BarWidget::KeepAwake => "Keep Awake",
            BarWidget::Keybinds => "Keyboard Shortcuts",
            BarWidget::SshSessions => "SSH Sessions",
            BarWidget::LockKeys => "Lock Keys (Caps/Num/Scroll)",
            BarWidget::KeyboardLayout => "Keyboard Layout",
            BarWidget::Keyboard => "On-Screen Keyboard",
            BarWidget::MargoDock => "Margo Dock",
            BarWidget::MargoLayoutSwitcher => "Margo Layout Switcher",
            BarWidget::MargoTags => "Margo Tags",
            BarWidget::ColorPicker => "ColorPicker",
            BarWidget::Lock => "Lock",
            BarWidget::Logout => "Logout",
            BarWidget::Setup => "Setup",
            BarWidget::MediaPlayer => "Media Player",
            BarWidget::Lyrics => "Lyrics",
            BarWidget::Vpn => "VPN",
            BarWidget::Dns => "DNS",
            BarWidget::Ai => "AI",
            BarWidget::Ip => "Public IP",
            BarWidget::Network => "Network Console",
            BarWidget::Custom(_) => "Custom Widget",
            BarWidget::Spacer(_) => "Spacer",
            BarWidget::Separator => "Separator",
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
            BarWidget::Weather => "Weather",
        }
    }

    pub fn action_name(&self) -> String {
        format!("{:?}", self).to_lowercase().replace(' ', "-")
    }

    pub fn all() -> &'static [BarWidget] {
        &[
            BarWidget::ActiveWindow,
            BarWidget::AlarmClock,
            BarWidget::Countdown,
            BarWidget::ControlCenter,
            BarWidget::HiddenBar,
            BarWidget::Catwalk,
            BarWidget::AudioDashboard,
            BarWidget::AudioRoute,
            BarWidget::AudioVisualizer,
            BarWidget::Bluetooth,
            BarWidget::Clipboard,
            BarWidget::Clock,
            BarWidget::CpuDashboard,
            BarWidget::Mdash,
            BarWidget::DarkMode,
            BarWidget::Twilight,
            BarWidget::KeepAwake,
            BarWidget::Keybinds,
            BarWidget::SshSessions,
            BarWidget::LockKeys,
            BarWidget::KeyboardLayout,
            BarWidget::Keyboard,
            BarWidget::MargoDock,
            BarWidget::MargoLayoutSwitcher,
            BarWidget::MargoTags,
            BarWidget::ColorPicker,
            BarWidget::Lock,
            BarWidget::Logout,
            BarWidget::Setup,
            BarWidget::MediaPlayer,
            BarWidget::Lyrics,
            BarWidget::Vpn,
            BarWidget::Dns,
            BarWidget::Ai,
            BarWidget::Ip,
            BarWidget::Network,
            BarWidget::Spacer(8),
            BarWidget::Separator,
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
            BarWidget::Weather,
        ]
    }
}
