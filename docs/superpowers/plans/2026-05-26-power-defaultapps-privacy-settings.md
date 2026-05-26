# Settings → Power + Default Apps + Privacy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add three top-level Settings pages — Power (battery/profiles/suspend/low-battery/lid+power-button), Default Apps (per-category default application), and Privacy (location, file history, camera/mic indicator, portal permissions) — matching GNOME's panels.

**Architecture:** Three relm4 `Component` pages in `mshell-settings`, mirroring the Network/Bluetooth pages shipped the same day. Power uses existing wayle services (`battery_service`, `power_profile_service`, `line_power_service`) + a `sys/logind.rs` pkexec drop-in writer. Default Apps uses `gio::AppInfo` (no CLI). Privacy uses geoclue (pkexec mask/unmask), `GtkRecentManager`, and `flatpak permission-*` CLI. Privileged ops authenticate through the running mshell-polkit agent.

**Tech Stack:** Rust, GTK4 + relm4, reactive_graph, wayle-battery/wayle-power-profiles, gio::AppInfo, systemd logind drop-in, flatpak permission CLI, tokio process.

**Testing note:** Pure parsers (logind.conf, `flatpak permissions` output) get real `#[cfg(test)]` unit tests (TDD). GTK UI driven by live services/hardware is verified by `cargo clippy` + compile + user manual test after rebuild. Each UI task ends with a clippy checkpoint.

**Spec:** `docs/superpowers/specs/2026-05-26-power-defaultapps-privacy-design.md`

---

## File structure

```
mshell-settings/src/
  power_settings.rs          # Task 1 (core) + Task 2 (lid/power-button UI)
  default_apps_settings.rs   # Task 3
  privacy_settings.rs        # Task 4 (scaffold) + Task 5 (recent) + Task 6 (perms)
  sys/
    mod.rs                   # Task 2
    logind.rs                # Task 2 (parser + tests + drop-in writer)
    geoclue.rs               # Task 4
    permissions.rs           # Task 6 (parser + tests + flatpak CLI)
mshell-config/src/schema/config.rs   # Task 1 (power section), Task 5 (privacy section)
mshell-style/scss/04-components/      # Task 7
  _power_settings.scss  _default_apps_settings.scss  _privacy_settings.scss
```

Registration edits (`settings.rs` 7 sites × 3, `lib.rs`, `_index.scss`) happen inside the relevant tasks.

---

## Task 1: Power page — battery + profiles + suspend + low-battery + config + registration

**Files:**
- Create: `mshell-crates/mshell-settings/src/power_settings.rs`
- Modify: `mshell-crates/mshell-settings/src/lib.rs` (`mod power_settings;`)
- Modify: `mshell-crates/mshell-settings/src/settings.rs` (7 registration sites)
- Modify: `mshell-crates/mshell-config/src/schema/config.rs` (add `power` section)
- Modify: `mshell-crates/mshell-settings/Cargo.toml` (`wayle-battery`, `wayle-power-profiles` workspace deps if not present)

- [ ] **Step 1: Add the config `power` section**

In `mshell-config/src/schema/config.rs`, add (mirror the `network` section added the same day — same Store/Patch derive, `#[serde(default)]`, Default):
```rust
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct PowerConfig {
    pub low_battery_warning: bool,
    pub low_battery_threshold: u32, // percent
}
impl Default for PowerConfig {
    fn default() -> Self { Self { low_battery_warning: true, low_battery_threshold: 15 } }
}
```
Add `pub power: PowerConfig` to the top-level `Config` struct (and to its Default — `#[serde(default)]` + `#[derive(Default)]` on Config handles it if that's the existing pattern; match what `network`/`idle` do exactly). This generates `config_manager().config().power()` + `PowerConfigStoreFields` with `low_battery_warning()` / `low_battery_threshold()` accessors.

> Read how `network`/`idle` are wired in `config.rs` and replicate field-for-field.

- [ ] **Step 2: Create the Power page component**

`power_settings.rs`. Mirror `bluetooth_settings.rs` for structure (hero header, `settings-page` box, map/unmap visibility, `#[watch]`/`#[block_signal]`, `tokio::spawn` for async wayle calls). Confirm every wayle call against `mshell-frame/src/menus/menu_widgets/power/power_menu_widget.rs` and the `power` bar widget.

Type skeleton:
```rust
use mshell_services::{battery_service, line_power_service, power_profile_service};
use mshell_utils::battery::{get_battery_icon, get_charging_battery_icon,
    spawn_battery_watcher, spawn_battery_online_watcher};
use mshell_utils::power_profile::spawn_active_profile_watcher;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, IdleStoreFields, PowerConfigStoreFields};
use reactive_graph::prelude::{Get, GetUntracked};
use wayle_battery::types::DeviceState;
use wayle_power_profiles::types::profile::PowerProfile;

#[derive(Debug, Clone)]
pub(crate) struct PowerSettingsModel {
    has_battery: bool,
    percent: f64,
    state: DeviceState,
    on_ac: bool,
    profile_idx: u32,           // 0 saver / 1 balanced / 2 performance
    has_profiles: bool,
    suspend_enabled: bool,
    suspend_timeout: u32,
    low_batt_warning: bool,
    low_batt_threshold: u32,
    // logind handlers filled in Task 2:
    // power_key/lid/lid_external indices
    _effects: mshell_common::scoped_effects::EffectScope,
}
#[derive(Debug)]
pub(crate) enum PowerSettingsInput {
    BatteryChanged, AcChanged, ProfileChanged,        // watcher refresh
    SetProfile(u32),
    SuspendEnabledChanged(bool), SuspendTimeoutChanged(u32),
    LowBattWarningChanged(bool), LowBattThresholdChanged(u32),
    // effects for the config mirror:
    SuspendEnabledEffect(bool), SuspendTimeoutEffect(u32),
    LowBattWarningEffect(bool), LowBattThresholdEffect(u32),
}
#[derive(Debug)] pub(crate) enum PowerSettingsOutput {}
pub(crate) struct PowerSettingsInit {}
#[derive(Debug)] pub(crate) enum PowerSettingsCommandOutput { BatteryChanged, AcChanged, ProfileChanged }
```

`init`: read `battery_service().device` (percentage/state — confirm field access against the power bar widget), `line_power_service()` online, `power_profile_service().power_profiles.active_profile.get()`; read `idle.suspend_*` + `power.low_battery_*` via `get_untracked()`. Start watchers (`spawn_battery_watcher`, `spawn_battery_online_watcher`, `spawn_active_profile_watcher`) → CommandOutputs. Push `EffectScope` effects mirroring the idle suspend + power config keys (like `idle_settings.rs`'s `push_effect!`).

`view!`: hero (`battery-symbolic` or `power-profile-balanced-symbolic`, "Power"). Sections:
- Battery (visible `model.has_battery`): icon + percent + state label + AC/battery + (if exposed) time/rate.
- Profiles (visible `model.has_profiles`): a `DropDown` (Power Saver/Balanced/Performance) `#[block_signal]` bound to `profile_idx`, change → `SetProfile`.
- Automatic suspend: Switch + SpinButton editing the idle suspend config (same widgets as `idle_settings.rs`), small "shared with Idle" label.
- Low-battery warning: Switch (`low_batt_warning`) + SpinButton threshold (1–100).

Handlers:
```rust
PowerSettingsInput::SetProfile(idx) => {
    let p = match idx { 0 => PowerProfile::PowerSaver, 2 => PowerProfile::Performance, _ => PowerProfile::Balanced };
    tokio::spawn(async move { let _ = power_profile_service().power_profiles.set_active_profile(p).await; });
}
PowerSettingsInput::SuspendEnabledChanged(v) => { config_manager().update_config(|c| c.idle.suspend_enabled = v); }
PowerSettingsInput::SuspendTimeoutChanged(v) => { config_manager().update_config(|c| c.idle.suspend_timeout_minutes = v); }
PowerSettingsInput::LowBattWarningChanged(v) => { config_manager().update_config(|c| c.power.low_battery_warning = v); }
PowerSettingsInput::LowBattThresholdChanged(v) => { config_manager().update_config(|c| c.power.low_battery_threshold = v); }
// *Effect arms set self.* fields; *Changed arms write config + the watcher/effect echoes back.
```
Low-battery toast: in the battery watcher refresh (update_cmd `BatteryChanged`), if `!on_ac && percent <= threshold && warning_enabled` and we weren't already below, fire `mshell_launcher::notify::toast("Battery low", &format!("{}% remaining", percent as u32))`. Track a `warned: bool` to debounce (reset when percent rises above threshold or on AC).

> The exact `battery_service().device` field names (percentage as f64? `DeviceState` variants) MUST be confirmed against `power.rs` bar widget — copy its reads.

- [ ] **Step 3: Register (7 sites in settings.rs + mod in lib.rs)**

Same pattern as the `network`/`bluetooth` registration (read those in `settings.rs`). For `power`:
`use crate::power_settings::{PowerSettingsInit, PowerSettingsModel};`; field `power_settings_controller: Controller<PowerSettingsModel>`; launch `PowerSettingsModel::builder().launch(PowerSettingsInit{}).detach()`; model-literal entry; sidebar `#[name="power_btn"]` (icon `"battery-symbolic"`, label "Power", `set_visible_child_name("power")`, alpha order); `stack.add_titled(..., Some("power"), "Power")`; search `("power","power")`; ActivateSection `"power" => Some(&widgets.power_btn)`. `lib.rs`: `mod power_settings;`.

- [ ] **Step 4: Clippy + commit**

```bash
cargo clippy -p mshell-config -p mshell-settings   # clean
git add mshell-crates/mshell-settings/src/power_settings.rs mshell-crates/mshell-settings/src/lib.rs mshell-crates/mshell-settings/src/settings.rs mshell-crates/mshell-config/src/schema/config.rs mshell-crates/mshell-settings/Cargo.toml Cargo.lock
git commit -m "feat(settings): Power page — battery, power profiles, suspend, low-battery warning"
```
Do NOT push.

**Manual verification:** Power page shows battery + AC; switching profile changes `powerprofilesctl get`; low-battery toast at threshold on battery.

---

## Task 2: logind helper (parser + tests + drop-in writer) + lid/power-button UI

**Files:**
- Create: `mshell-crates/mshell-settings/src/sys/mod.rs`, `mshell-crates/mshell-settings/src/sys/logind.rs`
- Modify: `mshell-crates/mshell-settings/src/lib.rs` (`mod sys;`)
- Modify: `mshell-crates/mshell-settings/src/power_settings.rs` (add lid/power-button section + handlers)

- [ ] **Step 1: Write the failing parser test**

`sys/logind.rs` bottom:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_handlers_with_later_override() {
        let main = "[Login]\n#HandlePowerKey=poweroff\nHandleLidSwitch=suspend\n";
        let dropin = "[Login]\nHandlePowerKey=ignore\nHandleLidSwitchExternalPower=lock\n";
        let h = parse_handlers(&[main.to_string(), dropin.to_string()]);
        assert_eq!(h.power_key, "ignore");       // drop-in overrides commented main
        assert_eq!(h.lid, "suspend");            // from main, not overridden
        assert_eq!(h.lid_external, "lock");      // from drop-in
    }
    #[test]
    fn defaults_when_unset() {
        let h = parse_handlers(&["[Login]\n".to_string()]);
        assert_eq!(h.power_key, "poweroff");     // systemd defaults
        assert_eq!(h.lid, "suspend");
        assert_eq!(h.lid_external, "suspend");
    }
    #[test]
    fn serializes_dropin() {
        let h = LogindHandlers { power_key: "ignore".into(), lid: "lock".into(), lid_external: "ignore".into() };
        let s = render_dropin(&h);
        assert!(s.contains("[Login]"));
        assert!(s.contains("HandlePowerKey=ignore"));
        assert!(s.contains("HandleLidSwitch=lock"));
        assert!(s.contains("HandleLidSwitchExternalPower=ignore"));
    }
}
```

- [ ] **Step 2: Run, confirm fail**

Run: `cargo test -p mshell-settings logind` → FAIL (parse_handlers/render_dropin/LogindHandlers undefined).

- [ ] **Step 3: Implement parser + writer**

```rust
use std::process::Stdio;
use tokio::process::Command;

pub const ACTIONS: [&str; 5] = ["ignore", "poweroff", "suspend", "hibernate", "lock"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogindHandlers { pub power_key: String, pub lid: String, pub lid_external: String }

impl Default for LogindHandlers {
    fn default() -> Self {
        // systemd defaults
        Self { power_key: "poweroff".into(), lid: "suspend".into(), lid_external: "suspend".into() }
    }
}

/// Parse the `Handle*` keys from logind config text fragments, later fragments
/// (drop-ins) overriding earlier. Commented (`#`) lines are ignored.
pub(crate) fn parse_handlers(fragments: &[String]) -> LogindHandlers {
    let mut h = LogindHandlers::default();
    for frag in fragments {
        for line in frag.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') { continue; }
            let Some((k, v)) = line.split_once('=') else { continue; };
            let (k, v) = (k.trim(), v.trim().to_string());
            match k {
                "HandlePowerKey" => h.power_key = v,
                "HandleLidSwitch" => h.lid = v,
                "HandleLidSwitchExternalPower" => h.lid_external = v,
                _ => {}
            }
        }
    }
    h
}

/// The managed drop-in body margo writes to /etc/systemd/logind.conf.d/99-margo.conf.
pub(crate) fn render_dropin(h: &LogindHandlers) -> String {
    format!(
        "# Managed by margo Settings — do not edit by hand.\n[Login]\nHandlePowerKey={}\nHandleLidSwitch={}\nHandleLidSwitchExternalPower={}\n",
        h.power_key, h.lid, h.lid_external
    )
}

const MAIN: &str = "/etc/systemd/logind.conf";
const DROPIN: &str = "/etc/systemd/logind.conf.d/99-margo.conf";

/// Read main conf + every *.conf.d/*.conf (sorted) so the parsed values reflect
/// effective config. Missing files are skipped.
pub async fn read_handlers() -> LogindHandlers {
    let mut frags = Vec::new();
    if let Ok(s) = tokio::fs::read_to_string(MAIN).await { frags.push(s); }
    if let Ok(mut rd) = tokio::fs::read_dir("/etc/systemd/logind.conf.d").await {
        let mut names = Vec::new();
        while let Ok(Some(e)) = rd.next_entry().await {
            if e.path().extension().and_then(|x| x.to_str()) == Some("conf") { names.push(e.path()); }
        }
        names.sort();
        for p in names { if let Ok(s) = tokio::fs::read_to_string(&p).await { frags.push(s); } }
    }
    parse_handlers(&frags)
}

/// Write the managed drop-in via pkexec (mshell-polkit prompts). Does NOT restart
/// logind — changes apply on next login. Returns Err(stderr) on failure/denial.
pub async fn write_dropin(h: &LogindHandlers) -> Result<(), String> {
    let body = render_dropin(h);
    // `pkexec sh -c 'mkdir -p .../logind.conf.d && cat > 99-margo.conf'` fed via stdin.
    let script = format!("mkdir -p /etc/systemd/logind.conf.d && cat > {DROPIN}");
    let mut child = Command::new("pkexec")
        .args(["sh", "-c", &script])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn pkexec: {e}"))?;
    use tokio::io::AsyncWriteExt;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(body.as_bytes()).await;
        let _ = stdin.shutdown().await;
    }
    let out = child.wait_with_output().await.map_err(|e| e.to_string())?;
    if out.status.success() { Ok(()) }
    else { Err(String::from_utf8_lossy(&out.stderr).trim().to_owned()) }
}
```
`sys/mod.rs`: `pub mod logind;`. `lib.rs`: `mod sys;`.

- [ ] **Step 4: Run tests, confirm pass**

Run: `cargo test -p mshell-settings logind` → 3 pass.

- [ ] **Step 5: Add lid/power-button UI to power_settings.rs**

Add model fields `power_key_idx: u32, lid_idx: u32, lid_external_idx: u32` (index into `logind::ACTIONS`), an input `LogindLoaded(LogindHandlers)`, and `SetPowerKey(u32)/SetLid(u32)/SetLidExternal(u32)`. On page map (or in `init`), `glib::spawn_future_local(async move { sender.input(LogindLoaded(sys::logind::read_handlers().await)) })`. A "Power Button & Lid" section with three `DropDown`s over `logind::ACTIONS` (Do nothing/Power off/Suspend/Hibernate/Lock — map index↔string). On any change, rebuild a `LogindHandlers` from the three indices and:
```rust
let h = LogindHandlers { power_key: ACTIONS[pk].into(), lid: ACTIONS[lid].into(), lid_external: ACTIONS[ext].into() };
glib::spawn_future_local(async move {
    if let Err(e) = sys::logind::write_dropin(&h).await { mshell_launcher::notify::toast("Power", &e); }
});
```
Add an inline `label-small`: "Applies on next login."

- [ ] **Step 6: Clippy + commit**

```bash
cargo clippy -p mshell-settings   # clean
git add mshell-crates/mshell-settings/src/sys mshell-crates/mshell-settings/src/lib.rs mshell-crates/mshell-settings/src/power_settings.rs
git commit -m "feat(settings): Power lid/power-button via logind drop-in (pkexec, applies next login)"
```

**Manual verification:** Change lid action → pkexec prompt → `/etc/systemd/logind.conf.d/99-margo.conf` written with the value.

---

## Task 3: Default Apps page (gio::AppInfo) + registration

**Files:**
- Create: `mshell-crates/mshell-settings/src/default_apps_settings.rs`
- Modify: `lib.rs` (`mod default_apps_settings;`), `settings.rs` (7 sites)

- [ ] **Step 1: Create the page**

Use `gio::AppInfo` (via `relm4::gtk::gio`). A category is `(label, &[mimes])`:
```rust
use relm4::gtk::gio::{self, prelude::*};
const CATEGORIES: &[(&str, &str, &[&str])] = &[
    // (label, primary mime for "current default", all mimes to set)
    ("Web Browser", "x-scheme-handler/http", &["x-scheme-handler/http","x-scheme-handler/https","text/html"]),
    ("Email",       "x-scheme-handler/mailto", &["x-scheme-handler/mailto"]),
    ("Calendar",    "text/calendar", &["text/calendar"]),
    ("Music",       "audio/mpeg", &["audio/mpeg","audio/flac","audio/x-vorbis+ogg"]),
    ("Video",       "video/mp4", &["video/mp4","video/x-matroska"]),
    ("Photos",      "image/jpeg", &["image/jpeg","image/png"]),
    ("Files",       "inode/directory", &["inode/directory"]),
];
```
For each category build a row: a `gtk::DropDown` whose model is the list of candidate apps
`gio::AppInfo::all_for_type(primary_mime)` filtered to `info.should_show()`, displayed by
`info.display_name()` (+ icon). Pre-select the current default
`gio::AppInfo::default_for_type(primary_mime, false)` (match by `info.id()`). On
`connect_selected_notify`, set the chosen `AppInfo` as default for ALL mimes in the category:
```rust
for m in mimes { let _ = chosen.set_as_default_for_type(m); }
```
Hold the `Vec<gio::AppInfo>` per row so the selected index maps back to an `AppInfo`. Use a
`gtk::StringList` of display names for the DropDown model (parallel to the AppInfo vec), or a
custom factory — `StringList` parallel vec is simplest. Errors from `set_as_default_for_type`
→ `mshell_launcher::notify::toast("Default Apps", &e.to_string())`.

Hero icon `"application-x-executable-symbolic"`, title "Default Apps". `settings-page` box layout
like `idle_settings.rs`.

- [ ] **Step 2: Register (7 sites + lib.rs mod)**

`default_apps` route, sidebar `#[name="default_apps_btn"]` (icon `"application-x-executable-symbolic"`, label "Default Apps"), `stack.add_titled(..., Some("default_apps"), "Default Apps")`, search `("default apps","default_apps")`, ActivateSection `"default_apps" => Some(&widgets.default_apps_btn)`, `mod default_apps_settings;`.

- [ ] **Step 3: Clippy + commit**

```bash
cargo clippy -p mshell-settings   # clean
git add mshell-crates/mshell-settings/src/default_apps_settings.rs mshell-crates/mshell-settings/src/lib.rs mshell-crates/mshell-settings/src/settings.rs
git commit -m "feat(settings): Default Apps page — per-category default via gio::AppInfo"
```

**Manual verification:** Change Web Browser → `xdg-mime query default x-scheme-handler/http` reflects it.

---

## Task 4: Privacy page scaffold — location (geoclue) + camera/mic indicator + lock summary + registration

**Files:**
- Create: `mshell-crates/mshell-settings/src/privacy_settings.rs`, `mshell-crates/mshell-settings/src/sys/geoclue.rs`
- Modify: `sys/mod.rs` (`pub mod geoclue;`), `lib.rs` (`mod privacy_settings;`), `settings.rs` (7 sites)

- [ ] **Step 1: geoclue helper**

`sys/geoclue.rs`:
```rust
use std::process::Stdio;
use tokio::process::Command;

async fn sc(args: &[&str]) -> Result<String, String> {
    let o = Command::new("systemctl").env("LC_ALL","C").args(args)
        .stdin(Stdio::null()).output().await.map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_owned())
    // systemctl is-enabled/is-active return non-zero for disabled/masked; we read stdout regardless.
}

/// (installed, enabled) — `enabled` is false when masked/disabled.
pub async fn status() -> (bool, bool) {
    // `systemctl list-unit-files geoclue.service` lists it iff installed.
    let listed = Command::new("systemctl").env("LC_ALL","C")
        .args(["list-unit-files", "geoclue.service"]).stdin(Stdio::null())
        .output().await.map(|o| String::from_utf8_lossy(&o.stdout).contains("geoclue.service"))
        .unwrap_or(false);
    if !listed { return (false, false); }
    let state = sc(&["is-enabled", "geoclue.service"]).await.unwrap_or_default();
    let enabled = state != "masked" && state != "disabled";
    (true, enabled)
}

/// pkexec systemctl unmask (enable) / mask (disable) geoclue.service.
pub async fn set_enabled(on: bool) -> Result<(), String> {
    let verb = if on { "unmask" } else { "mask" };
    let o = Command::new("pkexec").args(["systemctl", verb, "geoclue.service"])
        .stdin(Stdio::null()).output().await.map_err(|e| e.to_string())?;
    if o.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&o.stderr).trim().to_owned()) }
}
```

- [ ] **Step 2: Create the Privacy page (scaffold: location + sensors + lock summary)**

`privacy_settings.rs`. Hero `"preferences-system-privacy-symbolic"` (fallback `"security-high-symbolic"` if missing), title "Privacy". Sections:
- **Location Services**: a `Switch`. In `init`, `glib::spawn_future_local` → `sys::geoclue::status()` → input `GeoclueStatus(installed, enabled)`; if `!installed` set the switch insensitive + label "geoclue not installed". On toggle → `sys::geoclue::set_enabled(on)` (spawn, toast on err). Inline note: "Controls the system geoclue location provider."
- **Active sensors** (read-only): Mic — subscribe `audio_service().recording_streams` (use `mshell_common::watch!` like the `privacy` bar widget) → label "In use by N app(s)" / "Not in use". Camera — a 3 s `fuser /dev/video*` poll, started on page map, stopped on unmap (copy the `privacy` bar widget's poll exactly) → "In use" / "Not in use".
- **Screen lock summary**: read `config_manager().config()` lock/idle fields (lock enabled + timeout — find the keys used by `lock_settings.rs`/`idle_settings.rs`); show a one-line summary + a `gtk::Button` "Open Lock settings" → `mshell_settings::open_settings_at_section("widgets/lock")` (the lock page route).
Leave a placeholder `gtk::Box` named `recent_section` and `perms_section` with `// filled in Task 5 / Task 6` so those tasks have an attachment point (or just add those sections wholesale in T5/T6).

Input enum should include `GeoclueStatus(bool,bool)`, `SetLocation(bool)`, `MicChanged`, `CameraTick`, plus the T5/T6 variants added later.

- [ ] **Step 3: Register (7 sites + lib.rs mod)**

`privacy` route, sidebar `#[name="privacy_btn"]` (icon as above, label "Privacy"), stack `Some("privacy"), "Privacy"`, search `("privacy","privacy")`, ActivateSection `"privacy" => Some(&widgets.privacy_btn)`, `mod privacy_settings;`, `pub mod geoclue;` in `sys/mod.rs`.

- [ ] **Step 4: Clippy + commit**

```bash
cargo clippy -p mshell-settings   # clean
git add mshell-crates/mshell-settings/src/privacy_settings.rs mshell-crates/mshell-settings/src/sys mshell-crates/mshell-settings/src/lib.rs mshell-crates/mshell-settings/src/settings.rs
git commit -m "feat(settings): Privacy page — location toggle, camera/mic indicator, lock summary"
```

**Manual verification:** Location toggle masks/unmasks geoclue (pkexec); mic/camera state reflects a running recorder/camera app; Lock button jumps to lock settings.

---

## Task 5: Privacy — File History (GtkRecentManager) + config

**Files:**
- Modify: `mshell-crates/mshell-config/src/schema/config.rs` (add `privacy` section)
- Modify: `mshell-crates/mshell-settings/src/privacy_settings.rs`

- [ ] **Step 1: Add config `privacy` section**

In `config.rs` (mirror the `power` section from Task 1):
```rust
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct PrivacyConfig { pub remember_recent: bool }
impl Default for PrivacyConfig { fn default() -> Self { Self { remember_recent: true } } }
```
Add `pub privacy: PrivacyConfig` to `Config`. Generates `config().privacy().remember_recent()`.

- [ ] **Step 2: File History UI**

In `privacy_settings.rs` add a "File History" section:
- A `Switch` "Remember recently-used files" bound to `config().privacy().remember_recent()`
  (`#[block_signal]`, write via `config_manager().update_config(|c| c.privacy.remember_recent = v)`).
  When toggled off, immediately purge (best-effort): `gtk::RecentManager::default().purge_items().ok();`
  and on page map, if `remember_recent` is false, purge again. (Note: this can't prevent apps from
  writing; it clears.)
- A `gtk::Button` "Clear History" (css `destructive-action`) → `gtk::RecentManager::default().purge_items()` with a toast "Recent files cleared".

- [ ] **Step 3: Clippy + commit**

```bash
cargo clippy -p mshell-config -p mshell-settings   # clean
git add mshell-crates/mshell-config/src/schema/config.rs mshell-crates/mshell-settings/src/privacy_settings.rs Cargo.lock
git commit -m "feat(settings): Privacy file history — remember toggle + clear (GtkRecentManager)"
```

**Manual verification:** Clear History empties `~/.local/share/recently-used.xbel`; toggle persists in config.

---

## Task 6: Privacy — Portal permissions (flatpak permission CLI)

**Files:**
- Create: `mshell-crates/mshell-settings/src/sys/permissions.rs`
- Modify: `sys/mod.rs` (`pub mod permissions;`), `privacy_settings.rs`

- [ ] **Step 1: Write the failing parser test**

`sys/permissions.rs` bottom — `flatpak permissions` prints a table like:
```
Table     Object  App                 Permissions
devices   camera  org.example.App     yes
location  location org.foo.Bar        EXACT,0
```
Test:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_permissions_table() {
        let out = "Table\tObject\tApp\tPermissions\n\
                   devices\tcamera\torg.example.App\tyes\n\
                   location\tlocation\torg.foo.Bar\tEXACT,0\n";
        let rows = parse_permissions(out);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].table, "devices");
        assert_eq!(rows[0].object, "camera");
        assert_eq!(rows[0].app, "org.example.App");
        assert_eq!(rows[1].app, "org.foo.Bar");
    }
    #[test]
    fn skips_header_and_blanks() {
        let rows = parse_permissions("Table\tObject\tApp\tPermissions\n\n");
        assert!(rows.is_empty());
    }
}
```

- [ ] **Step 2: Run, confirm fail** — `cargo test -p mshell-settings permissions` → FAIL.

- [ ] **Step 3: Implement**

```rust
use std::process::Stdio;
use tokio::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermEntry { pub table: String, pub object: String, pub app: String, pub perms: String }

/// Parse `flatpak permissions` output. The header row (starts with "Table") and
/// blank lines are skipped. Columns are whitespace/tab separated.
pub(crate) fn parse_permissions(out: &str) -> Vec<PermEntry> {
    out.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with("Table"))
        .filter_map(|l| {
            let cols: Vec<&str> = l.split_whitespace().collect();
            if cols.len() < 3 { return None; }
            Some(PermEntry {
                table: cols[0].to_string(),
                object: cols[1].to_string(),
                app: cols[2].to_string(),
                perms: cols.get(3).copied().unwrap_or("").to_string(),
            })
        })
        .collect()
}

async fn fp(args: &[&str]) -> Result<String, String> {
    let o = Command::new("flatpak").env("LC_ALL","C").args(args)
        .stdin(Stdio::null()).output().await.map_err(|e| format!("flatpak unavailable: {e}"))?;
    if o.status.success() { Ok(String::from_utf8_lossy(&o.stdout).into_owned()) }
    else { Err(String::from_utf8_lossy(&o.stderr).trim().to_owned()) }
}

/// True if the `flatpak` CLI exists.
pub async fn available() -> bool {
    Command::new("flatpak").arg("--version").stdin(Stdio::null()).output().await
        .map(|o| o.status.success()).unwrap_or(false)
}

pub async fn list() -> Result<Vec<PermEntry>, String> {
    Ok(parse_permissions(&fp(&["permissions"]).await?))
}

pub async fn revoke(table: &str, object: &str, app: &str) -> Result<(), String> {
    fp(&["permission-remove", table, object, app]).await.map(|_| ())
}
```
`sys/mod.rs`: `pub mod permissions;`.

- [ ] **Step 4: Run tests, confirm pass** — `cargo test -p mshell-settings permissions` → 2 pass.

- [ ] **Step 5: Portal permissions UI**

In `privacy_settings.rs` add a "App Permissions" section. On page map: `glib::spawn_future_local`
→ if `sys::permissions::available().await` then `list().await` → input `PermsLoaded(Vec<PermEntry>)`,
else input `PermsUnavailable`. Rebuild a list `gtk::Box`: when unavailable show a `label-small`
"flatpak not installed — portal permissions unavailable"; otherwise one row per entry (app + table/
object + a "Revoke" button css `destructive-action` → `revoke(table,object,app)` then reload). Filter
to the interesting tables (`devices`, `location`, and screencast if present) for signal.

- [ ] **Step 6: Clippy + commit**

```bash
cargo clippy -p mshell-settings   # clean
git add mshell-crates/mshell-settings/src/sys/permissions.rs mshell-crates/mshell-settings/src/sys/mod.rs mshell-crates/mshell-settings/src/privacy_settings.rs
git commit -m "feat(settings): Privacy portal permissions — list + revoke via flatpak CLI"
```

**Manual verification:** With flatpak present, the section lists device/location grants; Revoke removes one (`flatpak permissions` confirms).

---

## Task 7: SCSS partials (DESIGN.md tokens) + index

**Files:**
- Create: `mshell-crates/mshell-style/scss/04-components/_power_settings.scss`, `_default_apps_settings.scss`, `_privacy_settings.scss`
- Modify: `mshell-crates/mshell-style/scss/04-components/_index.scss`

- [ ] **Step 1: Discover the bespoke classes**

```
grep -rhoE 'add_css_class\("[^"]+"\)|add_css_class: "[^"]+"' \
  mshell-crates/mshell-settings/src/power_settings.rs \
  mshell-crates/mshell-settings/src/default_apps_settings.rs \
  mshell-crates/mshell-settings/src/privacy_settings.rs | sort -u
```
Style only NEW classes (skip shared `settings-page`/`settings-hero*`/`label-*`/`ok-button-primary`/
`destructive-action`/`status-error` — already defined). Check `status-error`/`destructive-action`
exist (`grep -rn 'destructive-action\|status-error' mshell-crates/mshell-style/scss/`) — reuse.

- [ ] **Step 2: Write the partials**

Follow `mshell-crates/mshell-frame/DESIGN.md` (matugen `var(--…)` tokens ONLY, no hardcoded hex;
calm/warn/danger severity ladder). Style battery/sensor rows, the profile selector, permission/
app rows (cards: `var(--surface-container-high)`, `var(--radius-sm)`, hover with `var(--primary)`
state layer), and any "low/critical" battery accent using `var(--error)`. Mirror the conventions in
`_network_settings.scss`/`_bluetooth_settings.scss` (just shipped).

- [ ] **Step 3: Register in index**

Append `@use "power_settings";`, `@use "default_apps_settings";`, `@use "privacy_settings";` to
`_index.scss` (match the existing `@use` ordering — they shipped `network_settings`/`bluetooth_settings`
the same way).

- [ ] **Step 4: Build + commit**

```bash
cargo build -p mshell-style   # grass compiles the SCSS; must succeed
git add mshell-crates/mshell-style/scss/04-components/_power_settings.scss mshell-crates/mshell-style/scss/04-components/_default_apps_settings.scss mshell-crates/mshell-style/scss/04-components/_privacy_settings.scss mshell-crates/mshell-style/scss/04-components/_index.scss
git commit -m "style(settings): power + default-apps + privacy page styling (DESIGN.md tokens)"
```

---

## Task 8: Full clippy/build + Cargo.lock + push

- [ ] **Step 1: Clippy the touched crates** — `cargo clippy -p mshell-config -p mshell-settings -p mshell-style` → clean.
- [ ] **Step 2: Build the shell** — `cargo build -p mshell` → succeeds (links all three pages).
- [ ] **Step 3: Cargo.lock** — `git status` shows no unstaged `Cargo.lock`; if `wayle-battery`/`wayle-power-profiles` were newly added to `mshell-settings/Cargo.toml`, the lock change must already be committed (Task 1). Verify clean.
- [ ] **Step 4: Push** — `git push origin main`.

**Manual verification (user, post-rebuild):** Spec verification checklist — all three pages open from the sidebar and via `open_settings_at_section`; battery/profile/suspend/low-batt; lid drop-in; default app changes; geoclue toggle; clear recent; revoke a portal permission.

---

## Self-review

- **Spec coverage:** Power battery/profiles/suspend/low-batt (T1) ✓; logind lid/power-button (T2) ✓;
  Default Apps gio (T3) ✓; Privacy location+sensors+lock (T4) ✓; file history (T5) ✓; portal perms (T6) ✓;
  config power+privacy sections (T1/T5) ✓; SCSS (T7) ✓; registration (T1/T3/T4) ✓; verification (T8) ✓.
- **Placeholders:** none — parsers (logind, permissions), geoclue, gio AppInfo, config schema are full
  code; UI components give full type contracts + key calls and name the in-repo templates to mirror
  (`bluetooth_settings.rs`, `power_menu_widget.rs`, `privacy` bar widget, `idle_settings.rs`). Deliberate.
- **Type consistency:** `LogindHandlers{power_key,lid,lid_external}` + `ACTIONS`/`parse_handlers`/
  `render_dropin`/`read_handlers`/`write_dropin` consistent T2↔power_settings; `PermEntry{table,object,
  app,perms}` + `parse_permissions`/`available`/`list`/`revoke` consistent T6; `geoclue::{status,
  set_enabled}` consistent T4; `PowerConfig.{low_battery_warning,low_battery_threshold}` +
  `PrivacyConfig.remember_recent` match between schema (T1/T5) and UI; profile set API
  `power_profile_service().power_profiles.set_active_profile` matches the spec's grounding.
