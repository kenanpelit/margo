# mvpn — native Mullvad VPN control (standalone binary) — Design

**Status:** approved design, pending spec review → writing-plans
**Date:** 2026-06-09
**Goal:** Replace the `osc-mullvad` bash script (v3.0.0, ~3000 lines) and the
sandboxed `mullvad` WASM plugin with one native `mvpn` binary: a full CLI
**and** a rich GTK4 layer-shell control panel, themed with margo's matugen
palette, integrated into the bar as a config-only pill. After this lands the
user needs neither `osc-mullvad` nor the WASM plugin.

---

## 1. Architecture

`mvpn` is a **single standalone top-level workspace crate** (like `mctl`,
`mlock`, `mkeys`, `mpicker`). It has no persistent in-process state — the
source of truth is always the live `mullvad` daemon + on-disk files
(`~/.mullvad/favorites.txt`, `~/.mullvad/slot.state`). Therefore there is **no
mshell service, no D-Bus IPC, and no reactive store** — every invocation
(CLI or menu) queries the daemon/files fresh. This is the key simplification
over an embedded design: the VPN's state already lives outside the shell.

```
mvpn/  (top-level crate — cargo build -p mvpn)
  src/
    main.rs        clap CLI dispatch + `menu` / `status --pill` subcommands
    engine/        pure logic, no GTK — the osc-mullvad port
      mod.rs
      status.rs    parse `mullvad status -v` → Status
      relays.rs    parse `mullvad relay list` → catalog (country/city/relay,
                   ownership, counts); owned/rented filters; random pick
      actions.rs   connect/disconnect/reconnect/toggle, set_location,
                   set_protocol (wireguard|openvpn), lockdown, auto-connect
      obf.rs       obfuscation get/set (udp2tcp|shadowsocks|auto|off),
                   hunt443, cycle
      favorites.rs ~/.mullvad/favorites.txt (relay|ping) read/upsert/remove/
                   sort; fastest (ping a candidate set → min); fastest-fav,
                   fastest-fav-sweep
      latency.rs   ping -c N → avg ms (parse), parallel ping helper
      slot.rs      device-slot: list/revoke/recycle/whoami; slot.state
                   (parity w/ osc-mullvad format); pass-store account number
      blocky.rs    DNS guard: systemctl start/stop/is-active; `ensure`
                   fail-safe (VPN unhealthy → resolver fallback)
      diag.rs      leak test (curl IP/DNS checks), split-tunnel list
      timer.rs     auto-switch driver (every N min) — a detached helper proc
      sys.rs       run_cmd / non-interactive sudo / env (PASSWORD_STORE_DIR)
    ui/            GTK4 layer-shell panel (relm4 0.10 + gtk4-layer-shell 0.7)
      mod.rs       window + layer-shell setup (mirrors mkeys)
      panel.rs     the rich panel widgets
      theme.rs     load matugen palette via mshell-matugen + CSS via mshell-style
```

`mvpn` depends on workspace crates `mshell-matugen` (palette extraction) and
`mshell-style` (matugen→CSS) so its panel themes identically to the shell,
plus `relm4`/`gtk4`/`gtk4-layer-shell` pinned to the workspace generation
(copy mkeys' versions). The `engine/` layer is GTK-free and unit-tested.

### Bar pill — zero mshell code

The pill is a **declarative custom widget** using the existing
`bars.widgets.custom_widgets` mechanism (`mshell-config` `CustomWidget`:
`exec`/`template`/`interval`/`on_click`/`on_click_right`/`opens_panel`, rendered
by `mshell-frame/src/bars/bar_widgets/custom.rs`). Config only — no new
`BarWidget` variant, no recompile of mshell:

```yaml
# managed by mvpn; placed in the user's mshell profile
exec: "mvpn status --pill"     # emits "#active\n<icon-or-label>" when up
template: "{output}"
interval: 5
on_click: "mvpn menu"
on_click_right: "mvpn toggle"
```

`mvpn status --pill` prints `#active` as the first line only when connected;
mshell turns a leading `#<state>` into the CSS class `.custom-bar-widget.active`
which tints the icon with `var(--primary)` (the accent) — the exact trick the
WASM manifest already relied on. The pill therefore looks consistent with
`.bar-pill-std` and tints on connect, with no native code.

---

## 2. CLI surface (`mvpn <cmd>`)

One-to-one with `osc-mullvad`'s command names, so it is a drop-in (keybinds
and scripts that call `osc-mullvad X` work as `mvpn X`):

| Group | Commands |
|---|---|
| Basic | `status [--pill\|-v\|--json]`, `connect`, `disconnect`, `toggle [--with-blocky] [--dry-run]`, `reconnect` |
| Location | `<cc>` (e.g. `de`), `<cc> <city>`, `random` |
| Advanced | `fastest [cc]`, `fastest-fav`, `fastest-fav-many [cc] [n]`, `fastest-fav-sweep <group\|cc…> [n]`, `owned [cc]`, `rented [cc]` |
| Protocol | `protocol` (toggle WG/OpenVPN) |
| Obfuscation | `obf` (interactive picker via menu), `obf <udp2tcp\|shadowsocks\|off\|auto>`, `obf hunt443`, `obf cycle` |
| Favorites | `fav add`, `fav remove`, `fav list`, `fav connect`, `fav refresh [cc]` |
| Device-slot | `slot recycle`, `slot status`, `slot whoami`, `slot list`, `slot revoke <dev>`, `slot disconnect` |
| Timer | `timer <min>`, `timer stop` |
| Diagnostics | `test` (leak), `split`, `ensure`, `lockdown on\|off`, `auto-connect on\|off` |
| GUI | `menu` (open the panel), `status --pill` (bar feed) |

`favorite`/`fav` accepted as aliases. Country group codes (`eu`, `na`, …) for
`fastest-fav-sweep` mirror osc-mullvad's `country_group_codes`.

---

## 3. Menu anatomy (`mvpn menu`)

A GTK4 layer-shell panel (anchored top-right by default; configurable), themed
via matugen, following `mshell-frame/DESIGN.md` (calm/warn/danger ladder, card
contract, `.bar-pill-std`-equivalent surfaces). Top → bottom:

1. **Header** — title + Active/Inactive badge + refresh.
2. **Hero card** — connected relay, visible location, protocol, live ping (ms),
   account expiry; or Connecting / Not-connected states.
3. **Stat grid** — Country · City · Relay · Public IPv4.
4. **Primary actions** — Connect, or Disconnect + Reconnect (full-width).
5. **Quick-action chips** — Random · Fastest · Protocol · Obf.
6. **Favorites** — ping-sorted list; click→connect; ★ add/remove current;
   Refresh (re-ping + re-sort).
7. **Country search list** — relay counts; owned/rented filter toggle; click a
   country → set location + connect.
8. **Toggle cards** — Lockdown mode · Auto-connect on startup.
9. **Device-slot** — whoami + device list; Recycle (revoke others→login→
   connect→record); revoke a device.
10. **Blocky DNS guard** — on/off + `ensure` (fail-safe) status.
11. **Diagnostics** — leak test result + split-tunnel excluded processes.

The menu shells out to the same `engine/` functions the CLI uses. Long
operations (ping sweep, fastest, slot login, curl tests) run on a worker thread
with results delivered to the GTK main loop via a oneshot/channel — never a
blocking `recv()` on the main loop (per the GTK-blocking-recv discipline).

---

## 4. Security & threading

- **Privileged operations** (blocky `systemctl`, slot operations that need
  root) use **non-interactive sudo** (`sudo -n`), never `pkexec` — a polkit
  prompt from a layer-shell window with keyboard focus deadlocks (the DNS-freeze
  lesson). If `sudo -n` fails, the action reports "needs sudo" rather than
  hanging.
- **`pass` integration** for the slot account number sets
  `PASSWORD_STORE_DIR` explicitly (a binary launched from the bar does not
  inherit the user's shell-rc env).
- The menu closes (or detaches the op) before a privileged action so the panel
  never holds keyboard focus across a prompt.
- All shell-outs have timeouts; failures degrade to a visible error, not a hang.

---

## 5. File-format parity (data carries over)

- **`~/.mullvad/favorites.txt`** — lines `relay|ping_avg`, numeric-ping-sorted
  (non-numeric → sort key 999999). Identical to osc-mullvad's
  `favorites_sort_stream`, so the user's existing favorites load unchanged.
- **`~/.mullvad/slot.state`** — same key/value pairs and os-id keying as
  osc-mullvad's `slot_state_*`, so multi-machine state carries over.
- Paths overridable via the same `OSC_MULLVAD_*` env vars osc-mullvad honored,
  plus `mvpn`'s own config (below), so a mixed transition period works.

---

## 6. Config

`~/.config/margo/mvpn.toml` (mvpn-owned, like `mkeys.toml`):

```toml
default_country  = ""          # empty = no default
ping_count       = 3
ping_timeout     = 2
favorites_path   = "~/.mullvad/favorites.txt"
slot_state_path  = "~/.mullvad/slot.state"
pass_entry       = "mullvad/account"
slot_revoke_others = true
blocky_unit      = "blocky.service"
menu_anchor      = "top-right"  # layer-shell anchor
menu_width       = 420
```

Defaults reproduce osc-mullvad's behavior. The bar-pill snippet (section 1) is
written into the user's mshell profile by a one-time `mvpn install-pill`
helper (idempotent), so the user doesn't hand-edit YAML.

---

## 7. Testing

- `engine/` parsers are pure functions with fixture-string unit tests:
  `status` (connected/connecting/disconnected, with/without IPv4),
  `relay list` (country/city/relay counting, ownership), favorites sort
  (numeric vs N/A), ping-avg parse, `obfuscation get`, slot.state round-trip,
  country-group expansion.
- A `--dry-run` path on mutating commands (already in osc-mullvad's `toggle`)
  for safe manual verification.
- No live-daemon tests in CI (no `mullvad` there); the daemon-touching code is
  thin wrappers over tested parsers.

---

## 8. Migration / retirement

- The old WASM `mullvad` plugin is retired: removed from the default bar; a note
  in its README points to `mvpn`. (The plugin source stays in
  `~/.kod/margo-plugins/mullvad` as history; not shipped.)
- `osc-mullvad` is left on disk but unused; once the user confirms parity it can
  be deleted. `mvpn` reuses its files/env so nothing is lost on switch.
- A short `docs/` page documents the `mvpn` CLI + pill install.

---

## 9. Phasing (the plan will follow this order)

1. **Crate skeleton + engine: status/actions/relays** + CLI for the basic +
   location groups + `status --pill`. (Replaces daily use immediately.)
2. **favorites + latency + fastest/sweep + protocol + obf.**
3. **GTK4 layer-shell panel** (theme + hero + stats + actions + favorites +
   country list + toggles) — `mvpn menu`.
4. **device-slot + blocky + ensure + diagnostics (test/split)** + their menu
   sections + `timer`.
5. **Pill install helper + WASM-plugin retirement + docs + Settings note** +
   final clippy/fmt/build.

Each phase produces a working, testable binary on its own.

---

## 10. Out of scope (YAGNI)

- No D-Bus service / reactive store / mshell rebuild (deliberately — no shared
  state).
- No GUI for editing `mvpn.toml` beyond the pill-install helper (hand-edit, like
  `mkeys.toml`); a Settings page is a possible later add, not part of this.
- No new compositor protocol work.
