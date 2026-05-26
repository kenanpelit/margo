# Settings → Power + Default Apps + Privacy (GNOME-parity) — Design

**Date:** 2026-05-26
**Status:** Approved (design); implementation pending
**Scope:** Three new top-level Settings sidebar pages — **Power**, **Default Apps**, and
**Privacy** — matching GNOME's panels. One spec, one PR ("all at once").

## Goal

Close three GNOME `gnome-control-center` gaps in margo's Settings: battery/power-profile/suspend
management (Power), default-application selection (Default Apps), and location/file-history/
permission controls (Privacy). Built on existing reactive services where they exist, GIO for
default apps, and CLI/pkexec for the privileged bits — matching the established mshell-settings
idiom (see the Network/Bluetooth pages shipped the same day).

## Decisions (locked)

- **Power:** full scope **including** logind lid/power-button behaviour.
- **Privacy:** all four subsets — location services, file history, camera/mic indicator + lock
  summary, portal permission store.
- **Delivery:** one spec, one PR.
- **Mechanisms:** Power → wayle services + logind drop-in (pkexec); Default Apps → `gio::AppInfo`
  (no CLI, no pkexec); Privacy → geoclue service toggle (pkexec), `GtkRecentManager`,
  `flatpak permission-*` CLI, and the existing privacy data sources.

## Existing infrastructure (build on this)

### Power (wayle — already present)
- `mshell_services::{battery_service, power_profile_service, line_power_service}`.
- Active profile: `power_profile_service().power_profiles.active_profile.get()` (a wayle
  `PowerProfile`). Set: `power_profile_service().power_profiles.set_active_profile(p).await`
  (see `mshell-frame/src/menus/menu_widgets/power/power_menu_widget.rs:571` — `set_profile`).
- Battery: `battery_service().device` (wayle device — percentage, `DeviceState`, energy/rate/
  time fields; `wayle_battery::types::DeviceState`). AC: `line_power_service()`.
- Helpers `mshell_utils::battery`: `get_battery_icon(f64)`, `get_charging_battery_icon(f64)`,
  `spawn_battery_watcher`, `spawn_battery_online_watcher`. `mshell_utils::power_profile::spawn_active_profile_watcher`.
- The `power` bar widget + `MenuType::Power` menu already consume all of this — mirror their calls.

### Suspend config (reuse, don't duplicate)
- `config_manager().config().idle().suspend_enabled()` + `suspend_timeout_minutes()` already
  exist (see `idle_settings.rs`). The Power page reads/writes the SAME keys so the two pages stay
  in sync via the reactive store.

### Privacy data sources (reuse)
- Mic in use: `audio_service().recording_streams` (reactive). Camera: `fuser /dev/video*` poll
  (see the `privacy` bar widget for the exact pattern). Lock settings live at route
  `widgets/lock` (deep-link target for the Privacy lock summary).

### Settings page pattern + registration (mirror Network/Bluetooth)
Each page is a relm4 `Component` like `idle_settings.rs`. Registration in `settings.rs` has the
7 sites used for `network`/`bluetooth` (commit history this day): controller field, sidebar
`ToggleButton`, launch, model literal, stack `add_titled`, search tuple, `ActivateSection` arm;
plus `mod <page>;` in `lib.rs`. `open_settings_at_section("power"|"default_apps"|"privacy")`.

## Architecture

```
mshell-settings/src/
  power_settings.rs          # Power — battery + profiles + suspend + low-batt + lid/power-button
  default_apps_settings.rs   # Default Apps — gio::AppInfo per category
  privacy_settings.rs        # Privacy — orchestrates the four subsections
  sys/
    mod.rs
    logind.rs                # read logind.conf[.d], write 99-margo drop-in via pkexec (+ tests)
    permissions.rs           # `flatpak permission-*` list/parse/revoke (+ tests)
    geoclue.rs               # geoclue.service status + mask/unmask via pkexec
mshell-config/src/schema/
  config.rs                  # extend: privacy (remember_recent) + power (low_batt_warn, threshold)
mshell-style/scss/04-components/
  _power_settings.scss  _default_apps_settings.scss  _privacy_settings.scss
```

Live state from wayle (Power) + reactive `audio_service` (Privacy mic) lazy-started on page
reveal (map/unmap, honouring the menu-lazy-polling rule). Privileged ops via pkexec → the running
mshell-polkit agent. Errors → `mshell_launcher::notify::toast`.

## 1) Power page

- **Battery** (hidden if no battery device): percentage, state (Charging/Discharging/Full/Empty),
  capacity/health if exposed, time-to-full/empty, energy rate, power source (AC/battery). Live via
  `spawn_battery_watcher` + `spawn_battery_online_watcher`; icon via `get_battery_icon`/charging.
- **Power profiles**: a segmented selector / DropDown (Power Saver / Balanced / Performance) bound
  to `active_profile`; change → `set_active_profile(p).await` (spawn). Watch via
  `spawn_active_profile_watcher`. Hidden if power-profiles-daemon absent (active_profile None).
- **Automatic suspend**: a Switch + SpinButton editing `idle.suspend_enabled` /
  `idle.suspend_timeout_minutes` (same reactive store as the Idle page), with a small "shared with
  Idle" note.
- **Low-battery warning**: config toggle `power.low_battery_warning` + threshold
  `power.low_battery_threshold` (percent). A battery watcher fires `toast` once when crossing below
  the threshold on battery power (debounced: only re-warn after rising back above).
- **Lid / power-button behaviour** (logind): read current `HandlePowerKey`, `HandleLidSwitch`,
  `HandleLidSwitchExternalPower` from `/etc/systemd/logind.conf` + any `*.conf.d` drop-ins
  (last-wins). DropDowns with values `ignore|poweroff|suspend|hibernate|lock`. On change, write a
  drop-in `/etc/systemd/logind.conf.d/99-margo.conf` via `pkexec` (a small helper that writes the
  whole managed drop-in with the current selections). **Do NOT restart systemd-logind** (can drop
  the session) — show an inline note "applies on next login". This is the only fragile/privileged
  part.

## 2) Default Apps page (gio::AppInfo)

Categories, each a DropDown over `gio::AppInfo::all_for_type(<mime>)` (filtered to
`should_show()`), current default pre-selected, change → `AppInfo::set_as_default_for_type(<mime>)`
(writes user-level `~/.config/mimeapps.list`; no pkexec). Categories + mimes:

| Category | Mime / scheme |
|---|---|
| Web Browser | `x-scheme-handler/http` (also set `https`, `text/html`) |
| Email | `x-scheme-handler/mailto` |
| Calendar | `text/calendar` |
| Music | `audio/mpeg` (also `audio/flac`, `audio/x-vorbis+ogg`) |
| Video | `video/mp4` (also `video/x-matroska`) |
| Photos | `image/jpeg` (also `image/png`) |
| Files | `inode/directory` |

Each row: app icon + name in the DropDown. When a category has multiple mimes, set the default for
all of them on change. Read current via `AppInfo::default_for_type(mime, false)`.
**Terminal is out of scope** — there is no margo-config terminal key and no standard freedesktop
terminal mime; revisit later.

## 3) Privacy page (four subsections)

1. **Location Services** (`sys/geoclue.rs`): detect `geoclue.service` (e.g. `systemctl status`/
   `is-enabled`). A Switch: enable → `pkexec systemctl unmask geoclue.service`; disable →
   `pkexec systemctl mask geoclue.service`. If geoclue not installed, show "not available" and
   disable the row. Inline note that this gates the system geoclue provider. *Best-effort.*
2. **File History** (`GtkRecentManager`): a "Remember recently-used files" Switch backed by config
   `privacy.remember_recent` (when off, purge on page open + note it's best-effort since apps may
   write directly), and a **"Clear History"** button → `gtk::RecentManager::default().purge_items()`
   (works reliably).
3. **Active sensors + Screen lock**: read-only. Mic: subscribe `audio_service().recording_streams`
   → "In use by X" / "Not in use". Camera: 3 s `fuser /dev/video*` poll (only while page visible)
   → "In use" / "Not in use". Plus a screen-lock summary (read lock/idle config — lock enabled +
   timeout) and a button → `open_settings_at_section("widgets/lock")`.
4. **Portal permissions** (`sys/permissions.rs`): `flatpak permissions` (parse the table) lists
   per-app grants for the `devices` (camera/microphone/speakers), `location`, and screencast
   tables. Each entry: app id + table + permission, with a "Revoke" button →
   `flatpak permission-remove <table> <id> <app>`. If `flatpak` CLI absent, show "not available".
   *Mostly meaningful for flatpak apps.*

## Config schema additions (`mshell-config`)

Add a `power` section (`low_battery_warning: bool` default true, `low_battery_threshold: u32`
default 15) and a `privacy` section (`remember_recent: bool` default true), wired into `config.rs`
exactly like the `network` section added earlier (Store/Patch derive, `#[serde(default)]`, Default).

## Testable units (TDD)

- `sys/logind.rs`: parse logind.conf key=value (with `[Login]` section, comments, `*.conf.d`
  last-wins precedence) → a `LogindHandlers { power_key, lid, lid_external }`. Unit-test the parser
  against sample conf text. Also test the drop-in serializer output.
- `sys/permissions.rs`: parse `flatpak permissions` tabular output → `Vec<PermEntry { table, app,
  permission }>`. Unit-test the parser against sample output.
- GIO / wayle / GTK UI verified by clippy + compile + manual test.

## File structure (new + modified)

**New:** `power_settings.rs`, `default_apps_settings.rs`, `privacy_settings.rs`,
`sys/{mod,logind,permissions,geoclue}.rs`, three SCSS partials.
**Modified:** `lib.rs` (mod decls), `settings.rs` (3× the 7-site registration),
`mshell-config/src/schema/config.rs` (power + privacy sections), `_index.scss`.

## Verification

- `cargo test -p mshell-settings` (logind + permissions parsers) + existing tests pass.
- `cargo clippy -p mshell-config -p mshell-settings -p mshell-style` clean; `cargo build -p mshell`.
- Manual (user, post-rebuild): Power shows battery + switches profile + lid/power-button writes the
  drop-in; Default Apps changes the browser/etc. (confirm with `xdg-mime query default`); Privacy
  toggles geoclue, clears recent files, shows mic/camera state, lists+revokes a portal permission.

## Risks / honest limits

- **logind**: drop-in is safe but takes effect on next login (no logind restart). Not instant.
- **geoclue**: non-GNOME setups vary; mask/unmask is a coarse system-level toggle, best-effort.
- **Portal permissions**: needs the `flatpak` CLI; mostly affects sandboxed apps.
- **Recent "remember" toggle**: purge is reliable; preventing apps from recording is best-effort.
- pkexec ops (logind drop-in, geoclue mask) depend on the mshell-polkit agent; degrade gracefully
  with a toast on denial.

## Out of scope

Terminal default app, per-app non-portal permissions, hibernate image config, battery
charge-threshold (vendor-specific), thunderbolt authorization, Orca/accessibility (separate panel).
