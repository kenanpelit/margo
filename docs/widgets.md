# Bar widgets

The mshell bar is a row of **widgets** ("pills"). Some are *indicators*
(render-only — a clock, a CPU chip, a Wi-Fi glyph); others are *openers* that
toggle a layer-shell menu (the dashboard, the network panel, the VPN controls);
a few are *actions* (lock, screenshot, dark-mode toggle).

Every bar has three slots — **start / center / end** — and each holds an ordered
list of widgets. You almost never edit these by hand:

- **Settings → Bar → Top / Bottom bar** is the visual editor: add, remove, and
  reorder widgets per slot, and toggle the bar itself.
- **Settings → Bar pill** tunes per-pill placement and behaviour for the
  passive pills.

Under the hood each slot is a list in the shell profile
(`bars.widgets.{top,bottom}.{start,center,end}`). In that list a widget is its
**token** — the unit name (`Clock`, `Network`, `Vpn`, …). Three widgets take a
parameter and use YAML tag syntax:

```yaml
- !Custom my_button        # a user-defined pill (see Custom, below)
- !Spacer 8                # an 8 px gap
- !HiddenBarNamed media    # a named drawer
```

!!! tip "Live reload"
    Bar changes from Settings apply immediately. There is no compositor
    re-login needed — widgets are a shell concern.

---

## Time, dashboard & quick settings

| Token | Widget | What it does |
| --- | --- | --- |
| `Clock` | Clock | Date/time pill. Cycles through a list of strftime formats (`[tempo]`) on right-click; left-click opens the clock/calendar menu. |
| `Mdash` | Mdash | The dashboard opener — same clock label as `Clock`, but a click opens **mdash**: greeting hero + calendar + weather + media player + a quick-settings tile stack + a menu-shortcut grid. |
| `ControlCenter` | Control Center | System-preferences glyph that opens the Control Center menu (header + sliders + a grid of quick toggles). |

## System monitors

| Token | Widget | What it does |
| --- | --- | --- |
| `CpuDashboard` | CPU Dashboard | One chip showing live CPU load + package temperature with calm/warn/danger colour states. Click opens the dashboard (per-core bars, RAM, load averages); right-click toggles RAM% in the chip. |
| `AudioDashboard` | Audio Dashboard | Default output **and** input volume in one cluster (`🔊 42% · 🎙 5%`). Right-click cycles which side shows; click opens the mixer menu (sliders, mute, per-app streams, device + port pickers). |
| `AudioVisualizer` | Audio Visualizer | Live audio spectrum — a strip of bars driven by the `cava` CLI. Pulses with whatever is playing; flat strip on silence. Needs `cava` installed. |
| `AudioRoute` | Audio Route | Switches the default audio output. **Left-click** cycles to the next output (Bluetooth / USB / speakers …, skipping HDMI); **right-click** opens a picker menu to jump straight to one. Shown only when there are ≥2 outputs. The mic optionally follows across the headset boundary (Settings → Widgets → Audio Route, where the picker menu's position + size also live); headset detection uses PipeWire's portable device metadata (`form_factor` / `icon_name` / `bus`), not hardcoded names. Scriptable: `mshellctl menu audio-route` opens the picker, `mshellctl audio route-next` cycles. |
| `Catwalk` | Catwalk | A CPU-reactive animated cat (noctalia / RunCat sprites). Idles below a CPU threshold, walks faster as load climbs. Click opens the CPU dashboard. |
| `Weather` | Weather | Current condition icon + temperature for the configured location. Click opens the Current / Hourly / Daily weather menu. Location is set in Settings → General. |
| `SystemUpdate` | System Updates | Count of pending updates across official repo / AUR / Flatpak. Click opens the package list; right-click re-probes. Sources + interval in Settings → System Updates. |
| `Podman` | Podman | Running containers / pods / images summary from `podman ps`. Click opens the Podman menu. |
| `SshSessions` | SSH Sessions | Live count of active `ssh` clients. Click opens a searchable host list from `~/.ssh/config` (click a host to connect in a new terminal); right-click re-polls. |
| `Power` | Power Profile | Active power profile (performance / balanced / power-saver) + battery / power source, from power-profiles-daemon + UPower. Click opens the profile switcher panel. |

## Network & connectivity

| Token | Widget | What it does |
| --- | --- | --- |
| `Network` | Network Console | Wi-Fi / wired link state + live throughput, from NetworkManager (no `nmcli` polling). Click opens the network menu (Wi-Fi list, connect/disconnect, rescan, radio toggle); right-click flips between speed and icon display. |
| `Vpn` | VPN (Mullvad) | Native Mullvad pill driving the `mvpn` binary. Shield tints when the tunnel is up (relay + location in the tooltip). Click opens the VPN menu — connect / random / fastest, lockdown, auto-connect, quantum, plus a collapsible DNS section. |
| `VpnIndicator` | VPN Indicator | Minimal "a VPN is up" cue for generic tunnels (NetworkManager / wg-quick / openvpn). No menu. |
| `Dns` | DNS | Standalone DNS / Blocky panel (separate from the combined VPN pill). Polls DNS/VPN/Blocky state; click opens the DNS menu. |
| `Ip` | Public IP | Public IP from ipinfo.io (polled). Click opens a detail panel (city, ASN, …). |
| `Bluetooth` | Bluetooth | Adapter state icon; tints when a paired device is connected (a "hooked up to my headphones" cue). Opens the Bluetooth panel. |

## Media

| Token | Widget | What it does |
| --- | --- | --- |
| `MediaPlayer` | Media Player | Mirrors whichever MPRIS player is *currently playing* (Spotify, mpd, browsers, mpv, …) with transport controls. |
| `Lyrics` | Lyrics | Current synced lyric line of the now-playing track, scrolling in the bar. Click opens the full scrolling lyrics panel (lrclib.net, disk-cached). |

## Windows, tags & layout

| Token | Widget | What it does |
| --- | --- | --- |
| `ActiveWindow` | Active Window | Title of the globally focused window next to an app glyph; long titles marquee. |
| `MargoTags` | Margo Tags | 1–9 tag pills with focus / occupied / urgent states. Click to switch tags, scroll to cycle. |
| `MargoLayoutSwitcher` | Margo Layout Switcher | Trigger button that opens the layout menu to pick one of the 15 tiling layouts for the active tag. |
| `MargoDock` | Margo Dock | Per-app dock / taskbar — running + pinned apps as buttons; click focuses or launches. See [Dock (mdock)](mdock.md). |

## Indicators

| Token | Widget | What it does |
| --- | --- | --- |
| `LockKeys` | Lock Keys | Caps / Num / Scroll lock capsules (A / N / S) that light when engaged; the whole pill hides when all three are off. Read from `/sys/class/leds`. |
| `KeyboardLayout` | Keyboard Layout | Active xkb layout (e.g. US, TR). Click cycles to the next configured layout (`cyclekblayout`). |
| `Privacy` | Privacy | Lights up while the mic, camera, or screen is in use; click shows a per-sensor access log. |
| `RecordingIndicator` | Recording Indicator | Lights up while a screen recording is in progress; click stops the recording. |
| `Tray` | System Tray | StatusNotifierItem tray icons for apps that publish one. |
| `Notifications` | Notifications | Bell pill with unread count; click opens the notification center (history + Do Not Disturb). |

## Toggles & tools

| Token | Widget | What it does |
| --- | --- | --- |
| `DarkMode` | Dark Mode Toggle | One-click flip between Light and Dark matugen modes; the icon previews the mode you'd switch *to*. |
| `Twilight` | Twilight | margo's built-in blue-light filter. Click opens the panel (toggle + temperature + mode + schedule); right-click flips it on/off; the icon shows the live colour temperature while filtering. |
| `KeepAwake` | Keep Awake | Timed idle inhibitor. Left-click opens the duration grid + countdown; right-click ends a running session. Shows a live countdown while active. |
| `Keyboard` | On-Screen Keyboard | Toggles `mkeys`, margo's GTK on-screen keyboard. |
| `ColorPicker` | ColorPicker | Picks a colour from the screen and copies hex/rgb to the clipboard. |
| `Screenshot` | Screenshot | Takes a screenshot (region / window / output → file + clipboard). |
| `Wallpaper` | Wallpaper | Opens the wallpaper picker / rotates the wallpaper. |
| `Clipboard` | Clipboard | Clipboard history (wl-clipboard); click opens the searchable history menu. |
| `Notes` | Notes Hub | Scratchpad + notes + todos with counts; click opens the Notes menu. |
| `AlarmClock` | Alarm Clock | Alarm-bell pill that opens the Alarm Clock menu (alarms + stopwatch). Shows a running stopwatch inline and pulses while a tone rings. |
| `Countdown` | Countdown | Shows the soonest enabled Alarm Clock countdown (hourglass glyph + remaining time). Click opens the Alarm Clock menu on its Countdown tab. Hidden when no enabled, parseable target remains. |
| `Ai` | AI | Opens the native streaming-chat menu. Provider / model / key live in Settings → AI. |
| `Keybinds` | Keyboard Shortcuts | Opens a searchable cheatsheet of every shortcut, parsed live from `config.conf`. |
| `Ufw` | UFW Firewall | UFW status (privilege-free poll); click opens the firewall panel. |

## Session

| Token | Widget | What it does |
| --- | --- | --- |
| `Lock` | Lock | Locks the screen (mlock). |
| `Logout` | Logout | Logs out of the session (confirms with a dialog). |
| `Reboot` | Reboot | Reboots the system (confirms with a dialog). |
| `Shutdown` | Shutdown | Powers off the system (confirms with a dialog). |
| `Setup` | Setup | Opens the Settings panel straight to the in-shell setup wizard. |

## Layout helpers & custom

| Token | Widget | What it does |
| --- | --- | --- |
| `!Spacer <px>` | Spacer | A blank gap of the given pixel width, to push widgets apart. |
| `Separator` | Separator | A thin vertical divider line between widgets. |
| `HiddenBar` | Hidden Bar | A collapsible "drawer" pill that renders this bar's `hidden_widgets` behind a trigger — hover (auto-expand) or click to reveal, right-click to pin, auto-collapse on leave. |
| `!HiddenBarNamed <name>` | Hidden Bar (named) | An independently-addressable drawer with its own widget list + behaviour (entry under `bars.widgets.hidden_bars`). Target it with `mshellctl hidden-bar <verb> <name>`. |
| `!Custom <name>` | Custom Widget | A user-defined pill: icon/image + optional label, left/right click commands, and an optional `exec` poller whose stdout fills the label via `{output}`. Defined under `bars.widgets.custom_widgets`. |

---

## Adding a custom widget

A `Custom` pill is the escape hatch for anything not covered above — a button
that runs a command, or a tiny status readout from a script:

```yaml
bars:
  widgets:
    custom_widgets:
      weather_alert:
        icon: "weather-storm-symbolic"
        exec: "~/.config/margo/scripts/storm.sh"   # stdout → {output}
        label: "{output}"
        interval_secs: 300
        on_click: "notify-send 'Storm watch' \"$(cat /tmp/storm)\""
    top:
      end:
        - !Custom weather_alert
```

See [Configuration](configuration.md) for the full profile layout, and
[Scripting](scripting.md) for driving the shell and compositor from scripts.
