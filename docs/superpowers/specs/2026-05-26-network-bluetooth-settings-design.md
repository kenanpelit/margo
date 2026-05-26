# Settings → Network + Bluetooth (GNOME-parity) — Design

**Date:** 2026-05-26
**Status:** Approved (design); implementation pending
**Scope:** Two new top-level Settings sidebar pages — **Network** and **Bluetooth** —
matching GNOME's Settings panels, built on the existing reactive services and the
codebase's nmcli-shell-out idiom. Single spec, single PR ("all at once").

## Goal

Add GNOME-equivalent device/connection management to the mshell Settings window.
Today the shell only has quick-toggle *menus* (popovers) for network/bluetooth, and
Settings → **Widgets** has "Network Console" / "Bluetooth" entries that configure the
*bar pill/menu appearance* — not device management. This adds the real management
surface as two dedicated sidebar sections.

## Decisions (locked)

- **Privileged edit mechanism:** `nmcli` shell-outs (NOT zbus), authenticated through
  the already-running **mshell-polkit** agent (NetworkManager uses polkit). Matches the
  established mshell-settings idiom (`timedatectl`/`mctl`/`pkexec`).
- **Proxy:** best-effort. Store mode + host/port/ignore-list in mshell-config; on apply
  write `~/.config/environment.d/99-margo-proxy.conf` + the mshell process env. Clearly
  labelled inline as "applies to newly launched apps / next session" — there is no
  gnome-settings-daemon on margo to apply gsettings proxy at runtime.
- **Delivery:** one spec, one PR. Heaviest/most fragile part (connection editor + EAP)
  ships together with the rest.

## Existing infrastructure (build on this)

Both services already exist and are fully capable (wayle, reactive `reactive_graph`
properties via `mshell_services`):

### `network_service()` → `wayle_network::NetworkService`
- `.primary` — `ConnectionType` (Wired/Wifi/…)
- `.wifi` → `Option<Wifi>`: `.enabled`, `.connectivity` (`NetworkStatus`
  Connecting/Disconnected/Connected), `.ssid`, `.strength: Option<u8>`, `.access_points`
- `.wired` → `Option<Wired>`: `.connectivity`
- AccessPoint (`wayle_network::core::access_point`): `ssid: Ssid`, `strength`,
  `SecurityType`. Connect with password; `connections_for_ssid(...)`; `connect`/`disconnect`.
- Helpers in `mshell-utils/src/network.rs`: `set_network_icon`, `get_wifi_icon_for_strength`,
  `set_network_label`, and `spawn_*_watcher` families (state/wifi/wired/available networks).

### `bluetooth_service()` → `wayle_bluetooth::BluetoothService`
- `.available: bool`, `.enabled: bool` (power toggle)
- `.devices` → `Vec<Arc<Device>>`
- `Device` (`wayle_bluetooth::core::device`): `.alias`, `.connected`, `.paired`, `.trusted`,
  `.icon`, `.battery_percentage`; actions `.connect()`, `.disconnect()`, `.pair()`,
  `.forget()`, `.set_trusted(..)`; service `.start_discovery()`, `.stop_discovery()`.
- Helpers in `mshell-utils/src/bluetooth.rs`: `set_bluetooth_icon`, `set_bluetooth_label`,
  `get_bluetooth_device_icon`, `spawn_bluetooth_*_watcher` families.

### Settings page pattern (mirror existing pages)
- Each page is a relm4 `Component` (e.g. `display_settings.rs`, `sound_settings.rs`).
- `settings.rs` holds: a sidebar `ToggleButton` per section (radio group), a `gtk::Stack`
  page per section keyed by `set_visible_child_name("<name>")`, a `Controller<Model>` field,
  launched in `init`, a search target `("<label lc>", "<route>")`, and an `ActivateSection`
  route. `mshell_settings::open_settings_at_section("<route>")` opens directly.
- Sidebar order is roughly alphabetical → insert **Bluetooth** and **Network** accordingly.

## Architecture

```
mshell-settings/src/
  bluetooth_settings.rs        # BluetoothSettingsModel (wayle-backed)
  network_settings.rs          # NetworkSettingsModel (orchestrates sections)
  net/
    mod.rs
    nmcli.rs                   # typed reads + command builders (tokio Command)
    wifi_section.rs            # Wi-Fi list + connect/forget/hidden
    connection_editor.rs       # per-connection General/IPv4/IPv6/Security
    vpn_section.rs             # VPN list/import/connect
    proxy_section.rs           # best-effort proxy (config + env.d)
```

- **Live state** from wayle services via existing `mshell_utils` watchers → reactive,
  lazy-started on page reveal (honours the menu-lazy-polling rule: start watchers/discovery
  only while the page is visible; stop on hide).
- **Edits** via `net/nmcli.rs` async helpers; failures surface as toasts
  (`mshell_launcher::notify::toast`). Auth handled by the running mshell-polkit agent; an
  auth failure produces a clear "authorization required/denied" toast.
- **Icons** reuse `mshell_utils::{network,bluetooth}` helpers.
- **SCSS** new partials `_network_settings.scss` + `_bluetooth_settings.scss` under
  `mshell-style/scss/04-components/`, added to `_index.scss`; DESIGN.md tokens + severity
  ladder only (no hardcoded colours). SCSS edits require recompile + restart.

## Bluetooth page (wayle)

- **Header:** power `Switch` bound to `bluetooth_service().enabled`; if `!available`, show
  "Bluetooth hardware missing" and disable the rest.
- **Devices** section: live list from `.devices`. Row = device icon + alias + status
  (connected/paired/trusted chips) + battery %. Expander row → actions: Connect/Disconnect,
  Pair/Forget, Trust `Switch`, details (address, type).
- **Discovery:** `start_discovery()` on page reveal, `stop_discovery()` on hide; a
  "Searching…" spinner while discovering.
- **States:** disabled (radio off), empty (no devices), hardware-missing.
- **Out of scope (flagged):** OBEX "Send Files"; adapter discoverable/visibility toggle.

## Network page (wayle + nmcli)

Top-level sections mirroring GNOME's panel:

1. **Wired** (if a wired device exists): status row (connected/speed) + cog →
   connection editor for the wired connection.
2. **Wi-Fi:** enable `Switch` + airplane-mode (rfkill) toggle. **Visible networks** live
   list from `wifi.access_points`: strength icon + SSID + lock (secured) + connected marker.
   Click → connect (password dialog if secured); cog on active/saved → editor; rows for
   "Connect to Hidden Network…" and "Forget".
3. **VPN:** NM VPN connections (nmcli) — connect/disconnect toggle each; "+" import
   (`.ovpn` / WireGuard `.conf`) via `nmcli connection import`; remove.
4. **Proxy** (best-effort): mode None/Manual/Automatic; manual http/https/socks `host:port`
   + ignore-hosts. Persisted to mshell-config; applied to `~/.config/environment.d/` +
   process env. Inline note about limited (next-session) effect.

### Connection editor (shared sub-surface, nmcli)
For a given connection id/uuid:
- **General:** name, autoconnect, metered.
- **IPv4:** method `auto|manual|link-local|shared|disabled`; when manual: addresses (CIDR),
  gateway, DNS, search domains, routes.
- **IPv6:** same matrix.
- **Security** (Wi-Fi): WPA-PSK password; basic WPA-Enterprise PEAP/TTLS (identity + password).
- Read: `nmcli -t -f <fields> connection show <uuid>`. Write: `nmcli connection modify
  <uuid> <key> <val> …`. Apply: `nmcli connection up <uuid>`.
- **Stretch (flagged):** certificate-based EAP (CA/client certs).

## `net/nmcli.rs` surface (sketch)

All async (`tokio::process::Command`), parse `-t` (colon-separated, `\:`-escaped) output:

```
list_connections() -> Vec<ConnRow>          # NAME,UUID,TYPE,DEVICE,ACTIVE
connection_detail(uuid) -> ConnDetail        # ipv4.*, ipv6.*, connection.*, 802-11-*
modify(uuid, &[(key, val)]) -> Result<()>
up(uuid) / down(uuid) -> Result<()>
delete(uuid) -> Result<()>
wifi_rescan() -> Result<()>
wifi_connect(ssid, password: Option<&str>) -> Result<()>
import_vpn(path, kind) -> Result<()>         # nmcli connection import type <kind> file <p>
```

Errors map to user-facing toasts; non-zero exit → stderr surfaced.

## settings.rs registration (per section)

1. sidebar `ToggleButton` (icon + label) in the radio group, alpha-ordered.
2. `gtk::Stack` page: `controller.widget()`, `set_visible_child_name("network"|"bluetooth")`.
3. `Controller<Model>` field on `SettingsWindowModel`; launch in `init`.
4. search target tuples: `("network", "network")`, `("bluetooth", "bluetooth")`.
5. `ActivateSection` arm + `open_settings_at_section` route in `lib.rs`.
6. icons: `network-workgroup-symbolic` (or `network-wireless-symbolic`),
   `bluetooth-active-symbolic`.

## Files

**New:** `network_settings.rs`, `bluetooth_settings.rs`, `net/{mod,nmcli,wifi_section,
connection_editor,vpn_section,proxy_section}.rs`, `scss/04-components/_network_settings.scss`,
`_bluetooth_settings.scss`.
**Modified:** `mshell-settings/src/lib.rs` (mod decls + routes), `settings.rs` (registration),
`mshell-style/.../_index.scss`, `mshell-config` schema (proxy section + reactive store),
possibly `mshell-settings/Cargo.toml` (no new external crate expected; tokio process already
available transitively — verify).

## Verification

- `cargo clippy -p mshell-settings` (and `-p mshell-config` if schema touched) clean.
- Build `mshell`; SCSS recompiled.
- Manual (live hardware/NM, user-run after rebuild):
  - Settings → Network / Bluetooth open from sidebar + `open_settings_at_section`.
  - Toggle Wi-Fi + Bluetooth power; scan; connect to a secured Wi-Fi (password); verify.
  - Edit a connection's DNS → confirm with `nmcli connection show <uuid>`.
  - Pair/connect/forget a BT device; battery shows.
  - VPN import + connect.
  - Proxy: set manual, confirm `~/.config/environment.d/99-margo-proxy.conf` written.

## Risks

- **Connection editor matrix** (IPv4/IPv6 method × fields) + **EAP-Enterprise** are the bulk
  of the code and the most fragile parsing/quoting surface in one PR.
- **Proxy** is intentionally weak (no runtime applier on margo) — scoped as best-effort.
- nmcli output parsing must handle `-t` escaping and locale; pin `LC_ALL=C` for commands.
- Privileged nmcli ops depend on the mshell-polkit agent running; degrade gracefully.

## Out of scope

OBEX file transfer, BT adapter visibility toggle, certificate-based EAP (stretch), full
GNOME proxy runtime semantics.
