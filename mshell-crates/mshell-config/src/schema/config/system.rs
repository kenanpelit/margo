//! System-service config: proxy/network, login network, logging, Bluetooth,
//! power, privacy, audio and the control-center tiles.

use super::*;

/// Proxy mode for the network settings.
///
/// `None` disables the proxy (removes the environment.d file).
/// `Manual` writes explicit `http_proxy` / `https_proxy` / `all_proxy` /
/// `no_proxy` env vars to `~/.config/environment.d/99-margo-proxy.conf`.
/// `Automatic` stores a PAC URL for reference; margo has no runtime PAC
/// interpreter — apps launched after setting this mode need to support
/// `auto_proxy`/`WPAD` on their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, Store, JsonSchema)]
pub enum ProxyMode {
    /// No proxy — the environment.d file is removed.
    #[default]
    None,
    /// Manual proxy — host:port strings are written as env vars.
    Manual,
    /// Automatic (PAC URL) — stored for reference only (not applied as env).
    Automatic,
}

impl PatchField for ProxyMode {
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

impl ProxyMode {
    pub fn to_index(self) -> u32 {
        match self {
            ProxyMode::None => 0,
            ProxyMode::Manual => 1,
            ProxyMode::Automatic => 2,
        }
    }

    pub fn from_index(idx: u32) -> Self {
        match idx {
            1 => ProxyMode::Manual,
            2 => ProxyMode::Automatic,
            _ => ProxyMode::None,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            ProxyMode::None => "None",
            ProxyMode::Manual => "Manual",
            ProxyMode::Automatic => "Automatic (PAC)",
        }
    }

    pub fn display_names() -> Vec<&'static str> {
        vec!["None", "Manual", "Automatic (PAC)"]
    }
}

/// Network-level settings — currently proxy only.
///
/// Proxy fields are written to
/// `~/.config/environment.d/99-margo-proxy.conf` so apps launched in the
/// next session (and the current session via `set_var`) inherit them.
/// This is a best-effort applier: margo has no runtime system-wide proxy
/// daemon; running apps keep their current proxy environment.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
#[derive(Default)]
pub struct NetworkConfig {
    /// Proxy mode selection.
    pub proxy_mode: ProxyMode,
    /// HTTP proxy in `host:port` form (no scheme prefix).
    pub proxy_http: String,
    /// HTTPS proxy in `host:port` form.
    pub proxy_https: String,
    /// SOCKS5 proxy in `host:port` form.
    pub proxy_socks: String,
    /// Comma-separated list of hosts/domains that bypass the proxy.
    pub proxy_ignore: String,
    /// PAC URL used when `proxy_mode == Automatic`.
    pub proxy_pac_url: String,
    /// Wi-Fi hotspot network name (Network menu · Hotspot, Control Center
    /// Hotspot tile). Empty = `<hostname>-hotspot`.
    pub hotspot_ssid: String,
    /// Legacy hotspot password, read only for migration. mshell moves it
    /// into NetworkManager's `margo-hotspot` connection secrets once and
    /// clears this key. Settings never writes a new password here.
    pub hotspot_password: String,
    /// Wi-Fi interface for the hotspot (e.g. `wlan0`). Empty = the first
    /// NetworkManager Wi-Fi device.
    pub hotspot_interface: String,
    /// Seconds to confirm an IP / gateway / DNS change made in Settings on
    /// a connection that is in use; unconfirmed, the saved profile comes
    /// back. `0` = off (the change is just applied).
    pub ip_confirm_seconds: u32,
    /// Named SSH local port forwards (SSH Sessions menu · Forwards). Each
    /// runs as a systemd user service while started; none start on their
    /// own.
    pub ssh_forwards: Vec<SshForward>,
    /// Host monitor (Host Monitor bar pill and menu): hosts pinged on an
    /// interval, with a notification when one goes down or comes back.
    pub host_monitor: HostMonitorConfig,
}

/// The Host Monitor's host list and timing. Inert until a host is added
/// and the `HostMonitor` pill is on a bar (the pings run while the pill or
/// its menu is shown).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct HostMonitorConfig {
    /// The hosts, in display order.
    pub hosts: Vec<MonitoredHost>,
    /// Seconds between checks (5–3600).
    pub interval_seconds: u32,
    /// Seconds to wait for a ping reply (1–10).
    pub timeout_seconds: u32,
    /// Notify when a host stops answering.
    pub notify_down: bool,
    /// Notify when a host that was down answers again.
    pub notify_recovery: bool,
}

impl Default for HostMonitorConfig {
    fn default() -> Self {
        Self {
            hosts: Vec::new(),
            interval_seconds: 30,
            timeout_seconds: 1,
            notify_down: true,
            notify_recovery: true,
        }
    }
}

/// One monitored host.
#[derive(
    Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize, Store, Patch, JsonSchema,
)]
#[serde(default)]
pub struct MonitoredHost {
    /// An IP address or host name (a pasted URL is reduced to its host).
    pub host: String,
    /// Optional display name; empty = the host.
    pub label: String,
}

/// One `ssh -L` forward: `127.0.0.1:<local_port>` on this machine reaches
/// `<remote_host>:<remote_port>` as seen from the SSH destination `host`.
#[derive(
    Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize, Store, Patch, JsonSchema,
)]
#[serde(default)]
pub struct SshForward {
    /// Display name; also names the systemd unit (`margo-ssh-forward-<name>`).
    pub name: String,
    /// SSH destination: an alias from `~/.ssh/config`, a host or `user@host`.
    pub host: String,
    /// Port on this machine (bound on 127.0.0.1 only).
    pub local_port: u16,
    /// Host to reach from the SSH destination (`localhost` = the destination
    /// itself).
    pub remote_host: String,
    pub remote_port: u16,
    /// Open the forwarded port as `https://` instead of `http://`.
    pub https: bool,
}

/// Home-network login automation (Settings → Network · Network Console menu).
///
/// Native replacement for the external `home-net-vpn` login script: at login,
/// bring up a saved Wi-Fi connection then connect Mullvad (with Blocky as the
/// no-VPN DNS fallback). The same engine backs the menu's "Connect home network
/// now" button. Off by default.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct LoginNetworkConfig {
    /// Master switch for the at-login reconcile.
    pub enabled: bool,
    /// NetworkManager connection NAME to bring up (e.g. "Ken_5"). Empty = skip
    /// the Wi-Fi step (only do the VPN part).
    pub wifi_connection: String,
    /// After Wi-Fi is up, connect Mullvad.
    pub connect_vpn: bool,
    /// Couple Blocky as the DNS fallback — stop it while the VPN is up
    /// (needs passwordless sudo for `systemctl`; skipped with a toast otherwise).
    pub couple_blocky: bool,
    /// Seconds to wait after shell start before reconciling (let NM settle).
    pub delay_secs: u32,
}

impl Default for LoginNetworkConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            wifi_connection: String::new(),
            connect_vpn: true,
            couple_blocky: true,
            delay_secs: 4,
        }
    }
}

/// Trusted networks (Settings → VPN): bring Mullvad up when joining a
/// Wi-Fi network that is not on the trusted list, and optionally take a
/// tunnel it started down again on a trusted one. Off by default.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct TrustedNetworksConfig {
    /// Connect the VPN (`mvpn connect`) on joining an untrusted network.
    pub auto_connect: bool,
    /// Disconnect a tunnel this rule connected once back on a trusted
    /// network (a tunnel you started yourself is never touched).
    pub disconnect_on_trusted: bool,
    /// Treat any wired connection as trusted.
    pub trust_wired: bool,
    /// Trusted Wi-Fi network names (SSIDs, exact and case-sensitive).
    pub trusted_ssids: Vec<String>,
}

impl Default for TrustedNetworksConfig {
    fn default() -> Self {
        Self {
            auto_connect: false,
            disconnect_on_trusted: false,
            trust_wired: true,
            trusted_ssids: Vec::new(),
        }
    }
}

/// Shell file-logging (Settings → Logging). Mirrors margo's compositor knobs:
/// the shell writes per-session files to ~/.local/state/margo/logs (mshell-*.log,
/// last `keep_sessions` kept), driven by the shared `margo-logging` engine.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct LoggingConfig {
    /// Write log files at all. On by default so the last few sessions are
    /// always on disk for diagnosis.
    pub enabled: bool,
    /// Level: error | warn | info | debug | trace.
    pub level: String,
    /// How many session files to keep on disk.
    pub keep_sessions: u32,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            level: "info".to_string(),
            keep_sessions: 3,
        }
    }
}

/// Bluetooth auto-connect + audio-routing settings (Settings → Bluetooth).
///
/// Replaces the external `bt-autoconnect.service` + `bt-autoconnect-once`
/// + `bluetooth_toggle` scripts: at login the shell waits
/// `autoconnect_delay_secs`, then tries each device in `devices` in order
/// until one connects, and (optionally) routes audio to it. The same
/// engine backs the `mshellctl bluetooth toggle` keybind action.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct BluetoothConfig {
    /// Master switch for auto-connecting at login.
    pub autoconnect_enabled: bool,
    /// Seconds to wait after shell start before the first connect attempt —
    /// lets the adapter + PipeWire/WirePlumber settle. Default 6.
    pub autoconnect_delay_secs: u32,
    /// Devices to try, in order, until one connects. First success wins.
    pub devices: Vec<BluetoothDevice>,
    /// On connect, make the device the default audio OUTPUT (sink).
    pub route_audio_output: bool,
    /// Also make it the default INPUT (mic). Off by default: forcing a
    /// headset mic on usually drops the codec from A2DP to HSP/HFP and
    /// noticeably degrades playback quality.
    pub route_audio_input: bool,
}

impl Default for BluetoothConfig {
    fn default() -> Self {
        Self {
            autoconnect_enabled: false,
            autoconnect_delay_secs: 6,
            devices: Vec::new(),
            route_audio_output: true,
            route_audio_input: false,
        }
    }
}

/// One auto-connect Bluetooth device: a MAC address plus a friendly label
/// (the label is display-only; matching is by MAC, case-insensitive).
#[derive(
    Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize, Store, Patch, JsonSchema,
)]
#[serde(default)]
pub struct BluetoothDevice {
    /// Device MAC address, `AA:BB:CC:DD:EE:FF` (case-insensitive).
    pub mac: String,
    /// Friendly name shown in Settings (e.g. `SL4P`).
    pub name: String,
}

/// Power-management settings surfaced on the Settings → Power page.
///
/// `low_battery_warning` enables a desktop-notification toast when the
/// battery falls at or below `low_battery_threshold` percent while on
/// battery power. The threshold is a percent value (1–100); the default
/// of 15 matches common platform defaults.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct PowerConfig {
    /// Pop a toast notification when battery drops to or below
    /// `low_battery_threshold` while on battery.
    pub low_battery_warning: bool,
    /// Percent threshold for the low-battery toast (1–100, default 15).
    pub low_battery_threshold: u32,
    /// Timeframe of the power menu's battery-level graph, in hours
    /// (UPower history; clamped to 1–168, default 6). The menu's
    /// 6h / 24h / 7d chips write it.
    #[serde(default = "default_history_hours")]
    pub history_hours: u32,
}

fn default_history_hours() -> u32 {
    6
}

impl Default for PowerConfig {
    fn default() -> Self {
        Self {
            low_battery_warning: true,
            low_battery_threshold: 15,
            history_hours: default_history_hours(),
        }
    }
}

/// Privacy settings surfaced on the Settings → Privacy page.
///
/// `remember_recent` controls whether recently-used files are retained in the
/// GTK `RecentManager` list. When `false` the list is cleared immediately on
/// the page being shown and when the setting is toggled off. This is
/// best-effort: individual apps may record their own recent-file lists
/// independently of GtkRecentManager.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct PrivacyConfig {
    /// Remember recently-used files in GtkRecentManager. Default `true`.
    pub remember_recent: bool,
}

impl Default for PrivacyConfig {
    fn default() -> Self {
        Self {
            remember_recent: true,
        }
    }
}

/// Audio settings. Currently controls the optional HDMI / DisplayPort output
/// filter — when `hide_hdmi_outputs` is `true`, sinks whose node name or
/// description contains "hdmi", "displayport", or "display port" are hidden
/// from the output-device list, the output-switch cycle, and the audio
/// dashboard menu's device picker. Default `false` so the behaviour is
/// unchanged for users who haven't opted in.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct AudioConfig {
    /// Hide HDMI / DisplayPort audio sinks from the output list and switcher.
    /// Matched by node name or description; default `false`.
    pub hide_hdmi_outputs: bool,
    /// Restore the default output + input volumes to the levels below on shell
    /// startup. PipeWire doesn't persist volumes across reboots, so without
    /// this the levels drift; with it, every login lands on your chosen
    /// defaults. Off by default (no surprise volume changes).
    pub restore_volume_on_start: bool,
    /// Default output (speaker) level as a percentage `0..=100`. Applied at
    /// startup when `restore_volume_on_start` is on.
    pub default_output_volume: i32,
    /// Default input (microphone) level as a percentage `0..=100`.
    pub default_input_volume: i32,
    /// Audio Route bar pill: when it flips the audio path to/from a headset,
    /// also move the default **microphone**, not just the speaker. On by
    /// default (a headset's mic follows its speaker); turn off to keep your
    /// current capture device (e.g. a good USB mic) while only the speaker
    /// hops to the headset.
    pub route_switch_microphone: bool,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            hide_hdmi_outputs: false,
            restore_volume_on_start: false,
            default_output_volume: 50,
            default_input_volume: 50,
            route_switch_microphone: true,
        }
    }
}

/// Control Center tile visibility and order.
///
/// All tiles are shown by default (`true`) except the opt-in ones
/// (`mirror`, `hotspot`, `mic_denoise`, `thinkpad_led`, `custom_shortcut`), which default to `false`. Set a tile to `false` via the
/// Control Center's edit mode (the pencil icon in the header) to hide it
/// from the normal grid view. `tile_order` controls the order tiles appear in
/// the grid (first entry = top-left, reading left-to-right). Tiles not present
/// in `tile_order` (e.g. added in a future release) append at the end so
/// nothing silently disappears after an upgrade.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct ControlCenterConfig {
    pub wifi: bool,
    pub bluetooth: bool,
    pub audio_out: bool,
    pub mic: bool,
    pub battery: bool,
    pub keep_awake: bool,
    pub dnd: bool,
    pub dark_mode: bool,
    pub night_light: bool,
    pub color_picker: bool,
    pub disk: bool,
    pub airplane_mode: bool,
    pub vpn: bool,
    pub valent: bool,
    pub ufw: bool,
    pub podman: bool,
    /// Output-mirroring toggle (margo `toggle_mirror`). Off by default so
    /// an existing grid is unchanged; enable it in edit mode / Settings.
    pub mirror: bool,
    /// Wi-Fi hotspot toggle tile. Off by default (opt in via edit mode).
    pub hotspot: bool,
    /// Mic noise-suppression (RNNoise) toggle tile. Off by default.
    pub mic_denoise: bool,
    /// ThinkPad lid logo LED (`tpacpi::lid_logo_dot`) on/off tile. Off by
    /// default; hidden on machines without the LED.
    pub thinkpad_led: bool,
    /// Your own tile (label, icon, command; see `custom_shortcut_tile`).
    /// Off by default.
    pub custom_shortcut: bool,
    /// Display order of tiles in the grid. Each entry is a tile-id string
    /// (e.g. `"wifi"`, `"bluetooth"`, …). Unknown ids are skipped; ids
    /// not listed here are appended at the end in canonical order.
    #[serde(default = "default_cc_tile_order")]
    pub tile_order: Vec<String>,
    /// Tile ids that should span 2 columns in the grid (GNOME-style wide).
    /// Default is empty (all tiles are 1 column = uniform width). A tile
    /// whose id appears here spans both columns and starts on a fresh row.
    #[serde(default)]
    pub wide_tiles: Vec<String>,
    /// Header battery chip (icon + %). On by default; hidden anyway when
    /// the machine has no battery.
    #[serde(default = "default_true")]
    pub show_battery_chip: bool,
    /// Power-profile segmented control (Saver / Balanced / Performance)
    /// above the sliders. On by default.
    #[serde(default = "default_true")]
    pub show_power_profile: bool,
    /// Compact media player (title + transport) above the tile grid,
    /// shown only while a player is active. On by default.
    #[serde(default = "default_true")]
    pub show_media: bool,
    /// What the `custom_shortcut` tile shows and runs.
    #[serde(default)]
    pub custom_shortcut_tile: CustomShortcutTile,
}

/// The user-defined Control Center tile. Blank `label` / `icon` fall back
/// to "Custom" / `utilities-terminal-symbolic`; an empty `command` leaves
/// the tile inert (a click opens its Settings).
#[derive(
    Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize, Store, Patch, JsonSchema,
)]
#[serde(default)]
pub struct CustomShortcutTile {
    /// Tile title.
    pub label: String,
    /// Symbolic icon name.
    pub icon: String,
    /// Run with `sh -c` on click.
    pub command: String,
    /// Optional, run with `sh -c` when the Control Center opens and after
    /// a click: exit status 0 = active (accent fill), the first output line
    /// = the subtitle. Empty = a plain launcher tile.
    pub state_command: String,
}

pub(super) fn default_cc_tile_order() -> Vec<String> {
    vec![
        "wifi".to_string(),
        "bluetooth".to_string(),
        "audio_out".to_string(),
        "mic".to_string(),
        "vpn".to_string(),
        "valent".to_string(),
        "battery".to_string(),
        "keep_awake".to_string(),
        "dnd".to_string(),
        "airplane_mode".to_string(),
        "dark_mode".to_string(),
        "night_light".to_string(),
        "color_picker".to_string(),
        "disk".to_string(),
        "ufw".to_string(),
        "podman".to_string(),
        "mirror".to_string(),
    ]
}

impl Default for ControlCenterConfig {
    fn default() -> Self {
        Self {
            wifi: true,
            bluetooth: true,
            audio_out: true,
            mic: true,
            battery: true,
            keep_awake: true,
            dnd: true,
            dark_mode: true,
            night_light: true,
            color_picker: true,
            disk: true,
            airplane_mode: true,
            vpn: true,
            valent: true,
            ufw: true,
            podman: true,
            mirror: false,
            hotspot: false,
            mic_denoise: false,
            thinkpad_led: false,
            custom_shortcut: false,
            tile_order: default_cc_tile_order(),
            wide_tiles: Vec::new(),
            show_battery_chip: true,
            show_power_profile: true,
            show_media: true,
            custom_shortcut_tile: CustomShortcutTile::default(),
        }
    }
}
