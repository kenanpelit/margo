# mkeys — On-Screen Keyboard (Phase 1) Design

**Goal:** Port `~/.kod/wkeys` (ptazithos/wkeys, MIT) into the margo workspace as
a new standalone crate `mkeys` — a Wayland on-screen keyboard — and make it
margo-native through mshell integration (bar pill, Settings page, margo bind).

**Architecture:** Standalone GTK4 + `gtk4-layer-shell` + `relm4` binary that
draws a keyboard from a TOML layout and injects keystrokes via
`zwp_virtual_keyboard_v1` (margo already implements this manager). A
single-instance Unix-socket IPC gives it `toggle` / `show` / `hide` verbs (the
mlock/mplay pattern — the binary owns its own verbs, no mshellctl change).
mshell gets a thin bar pill that runs `mkeys toggle` and a Settings page that
writes `~/.config/margo/mkeys.toml`, which mkeys re-reads on each `show`.

**Tech stack:** Rust (edition 2024, workspace), gtk4 + gtk4-layer-shell 0.7.x +
relm4 0.9 (workspace generation — bumped from wkeys' 0.5/gdk4-0.9),
`wayland-client` + `wayland-protocols-misc` (virtual-keyboard), `xkbcommon`,
`evdev` (keycodes), `rust-embed` (embedded layouts + CSS), `toml`, `clap`.

---

## Scope

**In (Phase 1):**
- Standalone `mkeys` crate ported from wkeys core (drop the cosmic-applet).
- `zwp_virtual_keyboard_v1` key injection (kept from wkeys).
- Static / embedded CSS theming (wkeys-style) — **no live matugen yet**.
- Embedded layouts: `en` (US QWERTY) **and `tr` (Turkish-Q: ç ğ ı ö ş ü)`**.
- Modifiers: Shift / Ctrl / Alt / Super / Caps (wkeys `button_ex` model).
- Single-instance IPC verbs: `mkeys toggle | show | hide`.
- mshell bar pill (`BarWidget::Keyboard`) → `mkeys toggle`.
- mshell Settings page → writes `~/.config/margo/mkeys.toml`.
- Workspace + PKGBUILD wiring (gtk4/gtk4-layer-shell deps already present).

**Out (later / separate specs):**
- **mlock OSK** (lock-screen keyboard) — Phase 2, separate spec (cairo, not GTK;
  a separate layer-shell client cannot receive input while `session_locked`).
- Live matugen theming.
- Auto show/hide on text-field focus (`zwp_input_method_v2` / `wp_text_input_v3`).
- Emoji / numpad / symbol pages; touch gestures; haptic/sound feedback.

---

## File structure

### New crate `mkeys/` (top-level, like `mlock/`, `mplay/`)
| File | Responsibility |
|---|---|
| `Cargo.toml` | Workspace member; deps bumped to workspace gtk-rs generation. |
| `src/main.rs` | clap args → single-instance IPC → run app service or message client. |
| `src/ipc.rs` | Single-instance Unix socket: init, is_single_instance, clean_up. |
| `src/service/{host,client,mod}.rs` | App service (owns GTK + keyboard) vs message client (`toggle/show/hide`). |
| `src/native/{virtual_keyboard,session,mod}.rs` | `zwp_virtual_keyboard_v1` injection (ported ~as-is). |
| `src/ui/{main_view,mod}.rs` + `ui/components/button_ex.rs` + `ui/style/` | relm4 keyboard grid + key button (modifier state) + CSS asset. |
| `src/layout/{parse,assets,mod}.rs` | TOML layout parse + embedded `en.toml` / `tr.toml`. |
| `src/config.rs` | Reads `~/.config/margo/mkeys.toml` (layout, scale, position, opacity, margin). |
| `assets/layouts/{en,tr}.toml`, `assets/style/default.css` | Embedded via `rust-embed`. |

### mshell changes (thin)
| File | Change |
|---|---|
| `mshell-crates/mshell-config/src/schema/bar_widgets.rs` | `BarWidget::Keyboard` variant (+ `BarPillKind` entry). |
| `mshell-crates/mshell-frame/src/bars/bar_widgets/keyboard.rs` (new) | Pill: keyboard icon, click → spawn `mkeys toggle`. |
| `mshell-crates/mshell-frame/src/bars/bar.rs` | Dispatch arm for the new variant. |
| `mshell-crates/mshell-settings/src/keyboard_settings/` (new) | "On-Screen Keyboard" page → writes `mkeys.toml`. |
| `mshell-crates/mshell-settings/src/...` (registration) | Sidebar entry + page registration. |

### Repo wiring
- Root `Cargo.toml` `members += "mkeys"`.
- `PKGBUILD`: add `mkeys` to the build group + `install -Dm755 .../mkeys`
  (depends gtk4/gtk4-layer-shell already present).
- Optional: ship a default `bind = …,spawn,mkeys toggle` example in
  `config.example.conf` / docs.

---

## Data flow

1. **Toggle:** bar pill / margo bind runs `mkeys toggle`. If no instance →
   first process becomes the app service (shows the keyboard). If an instance
   exists → the second process is a message client that sends `toggle` over the
   socket and exits; the service shows/hides its layer surface.
2. **Show:** on show, the service **re-reads `~/.config/margo/mkeys.toml`**
   (so Settings changes apply next open), rebuilds layout/size/position, maps
   the bottom-anchored `gtk4-layer-shell` surface (overlay layer, no keyboard
   interactivity so it never steals focus from the target app).
3. **Keypress:** a key button press → `KeyboardHandle::key_press/release` →
   `zwp_virtual_keyboard_v1.key()` with the evdev keycode → margo delivers to
   the focused client. Modifiers latch via the button_ex model.
4. **Config write:** mshell Settings page edits `mkeys.toml` on disk; no live
   IPC reload needed — picked up on next `show`.

## Config schema (`~/.config/margo/mkeys.toml`)
```toml
layout   = "tr"        # "en" | "tr"  (embedded; or a path to a custom toml)
scale    = 1.0         # key size multiplier
position = "bottom"    # "bottom" | "top"
opacity  = 0.95        # 0.0–1.0
margin   = 8           # px gap from the screen edge
show_pill = true       # whether the bar pill is enabled (read by mshell)
```
mkeys owns/reads this; mshell Settings writes it (serde + toml, defaults for
every field so a missing/partial file is valid).

## Error handling
- No `zwp_virtual_keyboard_manager` advertised → log + exit non-zero (margo
  ships it, so this is a "wrong compositor" guard).
- Missing/garbage `mkeys.toml` → fall back to compiled defaults (serde
  `#[serde(default)]` per field), never panic.
- Stale single-instance socket (crash) → `ipc.rs` cleans up on SIGINT and on
  bind-failure detection (port wkeys' behaviour).
- Unknown layout name → fall back to embedded `en`.

## Testing
- Layout TOML parse tests (port wkeys' `layout/parse` tests) + a `tr.toml`
  round-trip asserting the Turkish glyph rows + their keycodes.
- Config deserialize: missing file / partial file → defaults.
- GTK UI is not unit-tested (headless GTK is impractical); covered by manual
  on-device verification.

## Risks / notes
- **Dependency-generation bump** is the main porting risk: wkeys is on
  gtk4-layer-shell 0.5 / gdk4 0.9; the workspace is on gtk4-layer-shell 0.7.x /
  gtk4 0.10-gen (cairo/pango 0.21). API drift (layer-shell init, gdk types)
  must be reconciled — expect small changes in `service/host.rs` + `ui/`.
- mkeys is a **GTK crate** (not the no-GTK compositor build group) — group it
  with mshell in the PKGBUILD build set.
- License: wkeys is MIT — preserve its `LICENSE`/attribution in the new crate.

## Out-of-scope follow-ups (named for Phase 2+)
- `mlock` OSK (lock-screen typing) — cairo-rendered keys + `wl_pointer` taps →
  PAM buffer, inside the trusted locker.
- matugen live theming for mkeys (emit an mkeys colour file like
  `mlogind-variables.toml`; mkeys watches/loads it).
