# Settings → Network + Bluetooth (GNOME-parity) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add two top-level Settings sidebar pages — Network and Bluetooth — matching GNOME's panels: Wi-Fi list/connect/forget, wired status, per-connection IPv4/IPv6/DNS editing, VPN, best-effort proxy, and Bluetooth power/device management.

**Architecture:** Two relm4 `Component` pages in `mshell-settings`, following the existing settings-page pattern (`idle_settings.rs`). Live state from the existing reactive wayle services (`network_service()`, `bluetooth_service()`) via `mshell_utils::{network,bluetooth}` watchers. Privileged edits via a new `net/nmcli.rs` shell-out helper (tokio `Command`, `LC_ALL=C`, `-t` parsing), authenticated through the running mshell-polkit agent. Proxy persisted to mshell-config + `~/.config/environment.d/`.

**Tech Stack:** Rust, GTK4 + relm4, reactive_graph, wayle-network/wayle-bluetooth, NetworkManager via `nmcli`, tokio process.

**Testing note:** The nmcli output parser is pure logic → real `#[cfg(test)]` unit tests (TDD). GTK UI components driven by live D-Bus hardware have no unit-test harness in this codebase; they're verified by `cargo clippy` + compile + user manual test after rebuild. Each UI task ends with a clippy checkpoint and a manual-verification note.

**Spec:** `docs/superpowers/specs/2026-05-26-network-bluetooth-settings-design.md`

---

## File structure

```
mshell-crates/mshell-settings/src/
  bluetooth_settings.rs        # Task 1 — BluetoothSettingsModel
  net/
    mod.rs                     # Task 2 — re-exports
    nmcli.rs                   # Task 2 — typed reads + command builders (+ tests)
    proxy.rs                   # Task 6 — env.d writer
  network_settings.rs          # Task 3 — NetworkSettingsModel (Wi-Fi + Wired)
  net/connection_editor.rs     # Task 4 — per-connection editor dialog
  net/vpn_section.rs           # Task 5 — VPN list/import/connect (folded into network page)
mshell-crates/mshell-config/src/schema/
  network.rs                   # Task 6 — proxy config section
mshell-crates/mshell-style/scss/04-components/
  _bluetooth_settings.scss     # Task 7
  _network_settings.scss       # Task 7
```

Registration edits (`settings.rs`, `lib.rs`, `_index.scss`, schema `config.rs`) happen inside the relevant tasks.

---

## Task 1: Bluetooth settings page (wayle-backed) + registration

**Files:**
- Create: `mshell-crates/mshell-settings/src/bluetooth_settings.rs`
- Modify: `mshell-crates/mshell-settings/src/lib.rs` (add `mod bluetooth_settings;`)
- Modify: `mshell-crates/mshell-settings/src/settings.rs` (7 registration sites)

- [ ] **Step 1: Create the page component**

`bluetooth_settings.rs`. Type skeleton (mirror `idle_settings.rs` for the relm4 boilerplate, hero header, and `settings-page` box):

```rust
use mshell_services::bluetooth_service;
use mshell_utils::bluetooth::{get_bluetooth_device_icon, spawn_bluetooth_devices_watcher,
    spawn_bluetooth_enabled_watcher};
use reactive_graph::prelude::{Get, GetUntracked, Set};
use relm4::gtk::prelude::*;
use relm4::gtk::glib;
use relm4::{Component, ComponentParts, ComponentSender, RelmWidgetExt, gtk};
use std::sync::Arc;
use wayle_bluetooth::core::device::Device;

#[derive(Debug, Clone)]
pub(crate) struct BluetoothSettingsModel {
    available: bool,
    enabled: bool,
    devices: Vec<Arc<Device>>,
}

#[derive(Debug)]
pub(crate) enum BluetoothSettingsInput {
    SetEnabled(bool),
    StateChanged,          // enabled/available watcher fired
    DevicesChanged,        // devices watcher fired
    Connect(String),       // device address
    Disconnect(String),
    Pair(String),
    Forget(String),
    SetTrusted(String, bool),
    ParentRevealChanged(bool),
}

#[derive(Debug)]
pub(crate) enum BluetoothSettingsOutput {}
pub(crate) struct BluetoothSettingsInit {}
#[derive(Debug)]
pub(crate) enum BluetoothSettingsCommandOutput { StateChanged, DevicesChanged }
```

`init`: read `bluetooth_service().available.get_untracked()` / `.enabled` / `.devices`; call
`spawn_bluetooth_enabled_watcher(&sender, || BluetoothSettingsCommandOutput::StateChanged)` and
`spawn_bluetooth_devices_watcher(&sender, || BluetoothSettingsCommandOutput::DevicesChanged)`.
Implement `update_cmd` to re-read the service into the model on each CommandOutput.

`view!`: `ScrolledWindow > Box.settings-page` with hero (`bluetooth-active-symbolic`, title
"Bluetooth"), a power-`Switch` row bound to `model.enabled` (`#[block_signal]` + `SetEnabled`),
a hardware-missing `Label` gated on `!model.available`, and a devices `gtk::Box` rebuilt in
`update_with_view` from `model.devices`: per device a row (icon via `get_bluetooth_device_icon`,
alias, connected/paired/trusted state, battery %), with Connect/Disconnect, Pair/Forget, Trust
`Switch`. Device actions call the async `Device` methods via `relm4::spawn`:

```rust
BluetoothSettingsInput::Connect(addr) => {
    if let Some(d) = self.devices.iter().find(|d| d.address.get() == addr).cloned() {
        relm4::spawn(async move { let _ = d.connect().await; });
    }
}
BluetoothSettingsInput::SetEnabled(on) => {
    relm4::spawn(async move { let _ = bluetooth_service().set_enabled(on).await; });
}
```

`ParentRevealChanged(true)` → `relm4::spawn(async { bluetooth_service().start_discovery().await })`;
`(false)` → `stop_discovery()`. (Honours menu-lazy-polling: discover only while visible.)

> Verify the exact `Device` action signatures against
> `mshell-crates/mshell-frame/src/menus/menu_widgets/bluetooth/device_revealed_content.rs`
> (it already calls `.connect()/.disconnect()/.pair()/.forget()/.set_trusted()`); match its
> await/error handling. Same for `set_enabled`/`start_discovery` — confirm against
> `bluetooth_menu_widget.rs`.

- [ ] **Step 2: Register the module**

In `lib.rs`, add alphabetically near line 20:
```rust
mod bluetooth_settings;
```

- [ ] **Step 3: Register in settings.rs (7 sites)**

a) Import at top with the other page imports:
```rust
use crate::bluetooth_settings::{BluetoothSettingsInit, BluetoothSettingsModel};
```
b) Controller field on `SettingsWindowModel` struct (near line 43):
```rust
    bluetooth_settings_controller: Controller<BluetoothSettingsModel>,
```
c) Launch in `init` (near line 722):
```rust
        let bluetooth_settings_controller = BluetoothSettingsModel::builder()
            .launch(BluetoothSettingsInit {})
            .detach();
```
d) Add to the `SettingsWindowModel { ... }` literal (near line 768):
```rust
            bluetooth_settings_controller,
```
e) Sidebar button in the `view!` radio group (place after `bar_btn`, before `date_time_btn`,
keeping alpha order). Copy the `display_btn` block shape:
```rust
                    #[name = "bluetooth_btn"]
                    gtk::ToggleButton {
                        add_css_class: "sidebar-button",
                        set_group: Some(&general_btn),
                        connect_toggled[stack] => move |b| {
                            if b.is_active() { stack.set_visible_child_name("bluetooth"); }
                        },
                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 12,
                            gtk::Image { set_icon_name: Some("bluetooth-active-symbolic") },
                            gtk::Label {
                                add_css_class: "label-medium",
                                set_label: "Bluetooth",
                                set_halign: gtk::Align::Start,
                                set_hexpand: true,
                            },
                        },
                    },
```
f) Stack page (near line 990):
```rust
        widgets.stack.add_titled(
            model.bluetooth_settings_controller.widget(),
            Some("bluetooth"),
            "Bluetooth",
        );
```
g) Search target tuple (in the array near line 743): `("bluetooth", "bluetooth"),`
h) `ActivateSection` arm (near line 1377): `"bluetooth" => Some(&widgets.bluetooth_btn),`

- [ ] **Step 4: Clippy checkpoint**

Run: `cargo clippy -p mshell-settings`
Expected: clean (no warnings), compiles.

- [ ] **Step 5: Commit**

```bash
git add mshell-crates/mshell-settings/src/bluetooth_settings.rs mshell-crates/mshell-settings/src/lib.rs mshell-crates/mshell-settings/src/settings.rs
git commit -m "feat(settings): Bluetooth page — power, device list, pair/connect/trust"
```

**Manual verification (user, post-rebuild):** Settings → Bluetooth opens; power toggle works;
devices list; pair/connect/disconnect/forget/trust act on the right device; battery shows.

---

## Task 2: `net/nmcli.rs` helper + parser unit tests (TDD)

**Files:**
- Create: `mshell-crates/mshell-settings/src/net/mod.rs`
- Create: `mshell-crates/mshell-settings/src/net/nmcli.rs`
- Modify: `mshell-crates/mshell-settings/src/lib.rs` (`mod net;`)

- [ ] **Step 1: Write the failing test for `-t` field splitting**

`nmcli.rs`, at the bottom:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_escaped_terse_fields() {
        // nmcli -t escapes ':' as '\:' and '\' as '\\'
        let line = r"Wired connection 1:abc-123:802-3-ethernet:eth0";
        assert_eq!(
            split_terse(line),
            vec!["Wired connection 1", "abc-123", "802-3-ethernet", "eth0"]
        );
    }

    #[test]
    fn unescapes_colons_in_field() {
        let line = r"My\:SSID:uuid:wifi";
        assert_eq!(split_terse(line), vec!["My:SSID", "uuid", "wifi"]);
    }

    #[test]
    fn parses_connection_rows() {
        let out = "Wired connection 1:abc-123:802-3-ethernet:eth0:yes\n\
                   home-wifi:def-456:802-11-wireless::no\n";
        let rows = parse_connections(out);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "Wired connection 1");
        assert_eq!(rows[0].uuid, "abc-123");
        assert_eq!(rows[0].active, true);
        assert_eq!(rows[1].name, "home-wifi");
        assert_eq!(rows[1].device, "");
        assert_eq!(rows[1].active, false);
    }
}
```

- [ ] **Step 2: Run, verify it fails**

Run: `cargo test -p mshell-settings nmcli`
Expected: FAIL — `split_terse`/`parse_connections`/`ConnRow` not defined.

- [ ] **Step 3: Implement the parser + types + command builders**

```rust
use std::process::Stdio;
use tokio::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnRow {
    pub name: String,
    pub uuid: String,
    pub kind: String,   // 802-3-ethernet, 802-11-wireless, vpn, wireguard, …
    pub device: String, // "" if not active
    pub active: bool,
}

/// Split one `nmcli -t` line on unescaped ':' and unescape `\:` and `\\`.
pub(crate) fn split_terse(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(&n) = chars.peek() {
                    cur.push(n);
                    chars.next();
                }
            }
            ':' => {
                fields.push(std::mem::take(&mut cur));
            }
            other => cur.push(other),
        }
    }
    fields.push(cur);
    fields
}

pub(crate) fn parse_connections(out: &str) -> Vec<ConnRow> {
    out.lines()
        .filter(|l| !l.is_empty())
        .filter_map(|l| {
            let f = split_terse(l);
            if f.len() < 5 { return None; }
            Some(ConnRow {
                name: f[0].clone(),
                uuid: f[1].clone(),
                kind: f[2].clone(),
                device: f[3].clone(),
                active: f[4] == "yes",
            })
        })
        .collect()
}

/// Run nmcli with `LC_ALL=C` for stable parsing. Returns stdout on success,
/// Err(stderr) on non-zero exit (e.g. polkit denial).
async fn run(args: &[&str]) -> Result<String, String> {
    let output = Command::new("nmcli")
        .env("LC_ALL", "C")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| format!("failed to spawn nmcli: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

pub async fn list_connections() -> Result<Vec<ConnRow>, String> {
    let out = run(&["-t", "-f", "NAME,UUID,TYPE,DEVICE,ACTIVE", "connection", "show"]).await?;
    Ok(parse_connections(&out))
}

pub async fn modify(uuid: &str, kv: &[(&str, &str)]) -> Result<(), String> {
    let mut args = vec!["connection", "modify", uuid];
    for (k, v) in kv { args.push(k); args.push(v); }
    run(&args).await.map(|_| ())
}

pub async fn up(uuid: &str)   -> Result<(), String> { run(&["connection", "up", uuid]).await.map(|_| ()) }
pub async fn down(uuid: &str) -> Result<(), String> { run(&["connection", "down", uuid]).await.map(|_| ()) }
pub async fn delete(uuid: &str)-> Result<(), String> { run(&["connection", "delete", uuid]).await.map(|_| ()) }
pub async fn wifi_rescan()    -> Result<(), String> { run(&["device", "wifi", "rescan"]).await.map(|_| ()) }

pub async fn wifi_connect(ssid: &str, password: Option<&str>) -> Result<(), String> {
    let mut args = vec!["device", "wifi", "connect", ssid];
    if let Some(p) = password { args.push("password"); args.push(p); }
    run(&args).await.map(|_| ())
}

pub async fn import_vpn(path: &str, kind: &str) -> Result<(), String> {
    run(&["connection", "import", "type", kind, "file", path]).await.map(|_| ())
}

/// Read one `connection.*`/`ipv4.*`/`ipv6.*` field via terse single-field show.
pub async fn get_field(uuid: &str, field: &str) -> Result<String, String> {
    let out = run(&["-t", "-f", field, "connection", "show", uuid]).await?;
    // single-field terse output is `field:value`; take the value
    Ok(split_terse(out.trim()).into_iter().nth(1).unwrap_or_default())
}
```

`net/mod.rs`:
```rust
pub mod nmcli;
```

`lib.rs`: add `mod net;` (alphabetically).

- [ ] **Step 4: Run tests, verify they pass**

Run: `cargo test -p mshell-settings nmcli`
Expected: PASS (3 tests).

- [ ] **Step 5: Clippy + commit**

```bash
cargo clippy -p mshell-settings
git add mshell-crates/mshell-settings/src/net mshell-crates/mshell-settings/src/lib.rs
git commit -m "feat(settings): nmcli helper — terse parser + connection/wifi/vpn commands"
```

---

## Task 3: Network settings page — Wi-Fi + Wired (wayle live) + VPN list scaffold + registration

**Files:**
- Create: `mshell-crates/mshell-settings/src/network_settings.rs`
- Modify: `lib.rs` (`mod network_settings;`), `settings.rs` (7 sites)

- [ ] **Step 1: Create the page component (live sections)**

Type skeleton:
```rust
use mshell_services::network_service;
use mshell_utils::network::{get_wifi_icon_for_strength, set_network_icon};
use crate::net::nmcli;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::prelude::*;
use relm4::gtk::glib;
use relm4::{Component, ComponentParts, ComponentSender, RelmWidgetExt, gtk};
use wayle_network::core::access_point::AccessPoint;
use wayle_network::types::states::NetworkStatus;

#[derive(Debug, Clone)]
pub(crate) struct NetworkSettingsModel {
    wifi_enabled: bool,
    wifi_present: bool,
    wired_present: bool,
    wired_connected: bool,
    access_points: Vec<AccessPoint>,
    active_ssid: Option<String>,
    connections: Vec<nmcli::ConnRow>,    // for VPN list + editor entry
    editor: Option<crate::net::connection_editor::ConnectionEditor>, // Task 4
}

#[derive(Debug)]
pub(crate) enum NetworkSettingsInput {
    SetWifiEnabled(bool),
    WifiChanged,            // wifi/ap watcher fired
    WiredChanged,
    ConnectionsReloaded(Vec<nmcli::ConnRow>),
    ConnectAp(String),      // ssid
    ConnectApWithPassword(String, String),
    ForgetConn(String),     // uuid
    OpenEditor(String),     // uuid
    UpConn(String), DownConn(String), DeleteConn(String),
    ImportVpnClicked,
    ParentRevealChanged(bool),
    Toast(String),
}
```

`init`: read service into model; `mshell_utils::network::spawn_wifi_available_watcher` +
`spawn_network_watcher` to emit CommandOutputs that re-read state. On `ParentRevealChanged(true)`
fire `nmcli::wifi_rescan()` and reload `nmcli::list_connections()` via `sender.oneshot_command`.

`update_with_view`: rebuild the Wi-Fi list box from `model.access_points` (strength icon via
`get_wifi_icon_for_strength`, SSID, lock if `SecurityType` ≠ open, ✓ if `== active_ssid`); row
click → `ConnectAp(ssid)`. For secured APs with no saved connection, `ConnectAp` opens a password
`gtk::Entry` dialog (reuse the flow in `menus/menu_widgets/network_toggle/network_revealed_content.rs`)
then `ConnectApWithPassword`. Wired section shows status + a gear → `OpenEditor(uuid)`. VPN section
lists `connections` filtered to `kind == "vpn" || kind == "wireguard"` with up/down/delete + an
"Import…" button → `gtk::FileDialog` → `nmcli::import_vpn`.

Async actions use `sender.oneshot_command`:
```rust
NetworkSettingsInput::ConnectApWithPassword(ssid, pw) => {
    let s = sender.clone();
    relm4::spawn_local(async move {
        if let Err(e) = nmcli::wifi_connect(&ssid, Some(&pw)).await {
            s.input(NetworkSettingsInput::Toast(e));
        }
    });
}
NetworkSettingsInput::Toast(msg) => {
    mshell_launcher::notify::toast("Network", &msg);
}
```

> Match the password-entry + `connect` flow against
> `network_toggle/network_revealed_content.rs` and `available_network_revealed_content.rs` so
> the wayle path (saved connections) and the nmcli path (new secured AP) agree.

- [ ] **Step 2: Register module + settings.rs (7 sites)**

`lib.rs`: `mod network_settings;`. `settings.rs`: same 7-site pattern as Task 1 with
`network_settings_controller`, sidebar `#[name="network_btn"]` (icon `network-wireless-symbolic`,
label "Network", `set_visible_child_name("network")`), stack `add_titled(..., Some("network"),
"Network")`, search `("network", "network")`, ActivateSection `"network" => Some(&widgets.network_btn)`.

- [ ] **Step 3: Clippy checkpoint**

Run: `cargo clippy -p mshell-settings` → clean.

- [ ] **Step 4: Commit**

```bash
git add mshell-crates/mshell-settings/src/network_settings.rs mshell-crates/mshell-settings/src/lib.rs mshell-crates/mshell-settings/src/settings.rs
git commit -m "feat(settings): Network page — Wi-Fi list/connect, wired status, VPN list+import"
```

**Manual verification:** Network page opens; Wi-Fi toggle; scan populates APs; connect to a
secured network with password; VPN list shows; import works.

---

## Task 4: Connection editor (nmcli General/IPv4/IPv6/Security)

**Files:**
- Create: `mshell-crates/mshell-settings/src/net/connection_editor.rs`
- Modify: `net/mod.rs` (`pub mod connection_editor;`), `network_settings.rs` (open + embed)

- [ ] **Step 1: Define the editor model + read/apply**

A relm4 `Component` (or a `gtk::Window`/dialog content) keyed by `uuid`. Fields read on open via
`nmcli::get_field`:
- General: `connection.id`, `connection.autoconnect` (yes/no), `connection.metered` (yes/no/unknown).
- IPv4: `ipv4.method` (auto/manual/link-local/shared/disabled), `ipv4.addresses`, `ipv4.gateway`,
  `ipv4.dns`, `ipv4.dns-search`, `ipv4.routes`.
- IPv6: same with `ipv6.*`.
- Security (only when parent `kind == "802-11-wireless"`): `802-11-wireless-security.key-mgmt`,
  and for PSK a write-only password entry → `802-11-wireless-security.psk`.

```rust
#[derive(Debug, Clone)]
pub(crate) struct ConnectionEditor { pub uuid: String /* + loaded field cache */ }
```

`view!`: `settings-page` box with a `gtk::Stack`/notebook of tabs General / IPv4 / IPv6 /
(Security). IPv4/IPv6 manual fields are revealed only when method == manual (a `gtk::Revealer`
gated on the method `DropDown`). An "Apply" button collects changed fields and calls:

```rust
let kv: Vec<(&str, &str)> = /* only changed keys */;
relm4::spawn_local(async move {
    if let Err(e) = nmcli::modify(&uuid, &kv).await {
        toast(e); return;
    }
    let _ = nmcli::up(&uuid).await;  // re-apply
});
```

DNS/addresses/routes are space-separated lists in nmcli; the editor joins the multi-line entry
into the nmcli format (e.g. addresses `"192.168.1.50/24"`, dns `"1.1.1.1 8.8.8.8"`). Validate IPs
with `str::parse::<std::net::IpAddr>()` before enabling Apply; on parse failure show inline error
(`.status-error` class).

- [ ] **Step 2: Wire into the Network page**

`OpenEditor(uuid)` in `network_settings.rs` constructs the editor and presents it (embedded
revealer panel below the section, or a child `gtk::Window` modal to the settings surface). Closing
returns to the list; reload `list_connections()` after Apply so state refreshes.

- [ ] **Step 3: Clippy checkpoint**

Run: `cargo clippy -p mshell-settings` → clean.

- [ ] **Step 4: Commit**

```bash
git add mshell-crates/mshell-settings/src/net/connection_editor.rs mshell-crates/mshell-settings/src/net/mod.rs mshell-crates/mshell-settings/src/network_settings.rs
git commit -m "feat(settings): connection editor — General/IPv4/IPv6/Security via nmcli modify"
```

**Manual verification:** Open a connection's editor; switch IPv4 to manual; set address + DNS;
Apply; confirm with `nmcli -t -f ipv4.method,ipv4.addresses,ipv4.dns connection show <uuid>`.

---

## Task 5: VPN actions hardening (connect/disconnect/delete wiring)

> VPN list + import were scaffolded in Task 3. This task only verifies the up/down/delete buttons
> are wired to `nmcli::up/down/delete` with toast error handling and a post-action reload.

**Files:** Modify `mshell-crates/mshell-settings/src/network_settings.rs`

- [ ] **Step 1: Wire the three actions**

```rust
NetworkSettingsInput::UpConn(uuid) => {
    let s = sender.clone();
    relm4::spawn_local(async move {
        match nmcli::up(&uuid).await {
            Ok(_)  => s.input(NetworkSettingsInput::ParentRevealChanged(true)), // reload
            Err(e) => s.input(NetworkSettingsInput::Toast(e)),
        }
    });
}
// DownConn / DeleteConn analogous with nmcli::down / nmcli::delete
```

- [ ] **Step 2: Clippy + commit**

```bash
cargo clippy -p mshell-settings
git add mshell-crates/mshell-settings/src/network_settings.rs
git commit -m "feat(settings): VPN connect/disconnect/remove wired with toast + reload"
```

**Manual verification:** VPN connect brings it up; disconnect drops; remove deletes the profile.

---

## Task 6: Proxy section (config schema + env.d writer, best-effort)

**Files:**
- Create: `mshell-crates/mshell-config/src/schema/network.rs`
- Modify: `mshell-crates/mshell-config/src/schema/config.rs` (add `network` section + store fields)
- Create: `mshell-crates/mshell-settings/src/net/proxy.rs`
- Modify: `network_settings.rs` (proxy UI), `net/mod.rs`

- [ ] **Step 1: Add the config schema**

`schema/network.rs` — follow an existing simple schema module (e.g. `schema/idle.rs`) for the
serde + reactive-store derive pattern:
```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, /* Store derive */)]
pub struct NetworkConfig {
    pub proxy_mode: ProxyMode,        // None | Manual | Automatic
    pub proxy_http: String,           // host:port
    pub proxy_https: String,
    pub proxy_socks: String,
    pub proxy_ignore: String,         // comma list
    pub proxy_pac_url: String,        // for Automatic
}
// + ProxyMode enum with Default = None, and Default impl for NetworkConfig (all empty).
```
Wire it into `config.rs` exactly like other sections: add `pub network: NetworkConfig` to the
config struct, to the store-fields trait, and to `Default`.

> Confirm the store-derive macro + the `ConfigStoreFields` accessor pattern by reading how
> `idle` is wired in `schema/config.rs`; replicate field-for-field.

- [ ] **Step 2: Write the env.d applier**

`net/proxy.rs`:
```rust
use std::io::Write;

/// Best-effort: write proxy env to ~/.config/environment.d/99-margo-proxy.conf
/// (systemd user environment, applied to apps launched in the next session) and
/// set it on the current process so newly spawned children inherit it.
pub fn apply(http: &str, https: &str, socks: &str, ignore: &str) -> std::io::Result<()> {
    let dir = dirs::config_dir().unwrap_or_default().join("environment.d");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("99-margo-proxy.conf");
    let mut lines = String::new();
    let push = |s: &mut String, k: &str, v: &str| if !v.is_empty() {
        s.push_str(&format!("{k}=http://{v}\n"));
    };
    if !http.is_empty()  { push(&mut lines, "http_proxy", http);  push(&mut lines, "HTTP_PROXY", http); }
    if !https.is_empty() { push(&mut lines, "https_proxy", https); push(&mut lines, "HTTPS_PROXY", https); }
    if !socks.is_empty() { lines.push_str(&format!("all_proxy=socks5://{socks}\n")); }
    if !ignore.is_empty(){ lines.push_str(&format!("no_proxy={ignore}\nNO_PROXY={ignore}\n")); }
    std::fs::File::create(&path)?.write_all(lines.as_bytes())?;
    // current process (children inherit)
    unsafe {
        if !http.is_empty()  { std::env::set_var("http_proxy",  format!("http://{http}")); }
        if !https.is_empty() { std::env::set_var("https_proxy", format!("http://{https}")); }
    }
    Ok(())
}

pub fn clear() -> std::io::Result<()> {
    let path = dirs::config_dir().unwrap_or_default().join("environment.d/99-margo-proxy.conf");
    if path.exists() { std::fs::remove_file(path)?; }
    Ok(())
}
```

- [ ] **Step 3: Proxy UI in the network page**

Add a Proxy section: a mode `DropDown` (None/Manual/Automatic); manual host:port entries revealed
when Manual; an inline `label-small` note: *"Applies to apps launched after this — margo has no
runtime proxy applier."* On change → `config_manager().update_config(|c| c.network.proxy_* = …)`
and call `crate::net::proxy::apply(...)` (or `clear()` for None).

- [ ] **Step 4: Clippy + commit**

```bash
cargo clippy -p mshell-config -p mshell-settings
git add mshell-crates/mshell-config/src/schema/network.rs mshell-crates/mshell-config/src/schema/config.rs mshell-crates/mshell-settings/src/net/proxy.rs mshell-crates/mshell-settings/src/net/mod.rs mshell-crates/mshell-settings/src/network_settings.rs
git commit -m "feat(settings): proxy section — config schema + environment.d applier (best-effort)"
```

**Manual verification:** Set Manual proxy; confirm `~/.config/environment.d/99-margo-proxy.conf`
contents; set None → file removed.

---

## Task 7: SCSS partials (DESIGN.md tokens) + index

**Files:**
- Create: `mshell-crates/mshell-style/scss/04-components/_network_settings.scss`
- Create: `mshell-crates/mshell-style/scss/04-components/_bluetooth_settings.scss`
- Modify: `mshell-crates/mshell-style/scss/04-components/_index.scss`

- [ ] **Step 1: Write the partials**

Style the device rows, Wi-Fi rows, status chips, and `.status-error` editor validation using only
matugen CSS vars + the calm/warn/danger severity ladder per
`mshell-crates/mshell-frame/DESIGN.md`. Reuse existing `settings-page`/`label-*` classes for layout;
add only what's new (connected accent on a row, signal/lock icon spacing, battery chip).

- [ ] **Step 2: Add to index**

Append `@use "network_settings";` and `@use "bluetooth_settings";` to `_index.scss` in the existing
order.

- [ ] **Step 3: Build (SCSS compiles at build time) + commit**

Run: `cargo build -p mshell-style`
Expected: build succeeds (grass compiles the SCSS).
```bash
git add mshell-crates/mshell-style/scss/04-components/_network_settings.scss mshell-crates/mshell-style/scss/04-components/_bluetooth_settings.scss mshell-crates/mshell-style/scss/04-components/_index.scss
git commit -m "style(settings): network + bluetooth page styling (DESIGN.md tokens)"
```

---

## Task 8: Full workspace clippy + final wiring check

- [ ] **Step 1: Clippy the touched crates**

Run: `cargo clippy -p mshell-config -p mshell-settings -p mshell-style`
Expected: clean.

- [ ] **Step 2: Build the shell binary**

Run: `cargo build -p mshell`
Expected: succeeds.

- [ ] **Step 3: Cargo.lock check**

If any crate's deps changed, the workspace `Cargo.lock` must be committed (AUR `prepare()` runs
`cargo fetch --locked`). No new external deps are expected; verify `git status` shows no unstaged
`Cargo.lock`, and if it changed, commit it.

- [ ] **Step 4: Final commit + push**

```bash
git add -A
git commit -m "chore(settings): network/bluetooth wiring + Cargo.lock"   # only if anything remains
git push origin main
```

**Manual verification (user, post-rebuild):** Full pass of the spec's verification checklist —
both pages open from the sidebar and via `open_settings_at_section`; Wi-Fi/BT toggles; scan;
secured connect; DNS edit confirmed via nmcli; pair/connect a device; VPN import + connect; proxy
file written.

---

## Self-review

- **Spec coverage:** Bluetooth page (T1) ✓; nmcli mechanism (T2) ✓; Wi-Fi list/connect/forget +
  wired + VPN (T3) ✓; connection editor IPv4/IPv6/Security (T4) ✓; VPN actions (T5) ✓; proxy
  best-effort (T6) ✓; SCSS/DESIGN.md (T7) ✓; registration in settings.rs/lib.rs (T1/T3) ✓;
  verification (T8) ✓. Stretch EAP-cert explicitly out of scope (spec) — not a gap.
- **Placeholders:** none — parser/commands/proxy/config are full code; UI components give full type
  contracts + wiring snippets and name the exact in-repo file to copy repetitive row markup from
  (`idle_settings.rs`, `network_toggle/*`, `bluetooth/device_revealed_content.rs`). This is
  deliberate (those files are the binding template) — not a TODO.
- **Type consistency:** `ConnRow{name,uuid,kind,device,active}` used identically in T2/T3/T4/T5;
  `nmcli::{list_connections,modify,up,down,delete,wifi_connect,wifi_rescan,import_vpn,get_field}`
  signatures stable across tasks; `NetworkConfig.proxy_*` names match between schema (T6 S1) and
  applier/UI (T6 S2–S3).
