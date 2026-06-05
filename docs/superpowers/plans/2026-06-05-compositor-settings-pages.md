# Compositor Settings Pages Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (or subagent-driven-development) to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Expose the remaining hand-edited `config.conf` compositor knobs in mshell Settings — three new pages (Appearance, Effects, Behaviour) + a small Overview augment — so the user never has to hand-edit those sections again.

**Architecture:** Each page reads the live `config.conf` (`margo_config::parse_config_with_defaults`) and, on every control change, writes the changed `key = value` line(s) back via a shared in-place patcher and runs `mctl reload` (margo re-parses live). This is the existing animations/input/overview mechanism, refactored into one shared helper.

**Tech Stack:** Rust, relm4 `Component` (the `region_settings.rs` / `animations_settings.rs` page model), `margo_config` for reads, `mctl reload` for apply.

**User decisions (2026-06-05):** 3 pages (Appearance + Effects + Behaviour); static colour keys stay matugen-owned (NO manual Colours page); risky keys (`syncobj_enable`, `allow_tearing`, `allow_shortcuts_inhibit`, `idleinhibit_ignore_visible`) live under an **Advanced** expander inside Behaviour.

---

## Coverage map (config.conf section → Settings)
| config.conf section | Settings page | Status |
|---|---|---|
| 1. Look — borders, gaps, opacity, cursor | **Appearance** | NEW (Task 1) |
| 1. Look — shadows, blur | **Effects** | NEW (Task 2) |
| 2. Animations | `animations` | done |
| 3. Behaviour — focus/drag/snap/hotcorner/scroll/scratchpad/sync | **Behaviour** | NEW (Task 3) |
| 3. Behaviour — overview_* | `overview` | done; augment (Task 4) |
| 4. Input — xkb/libinput | `input` | done |
| colours (rootcolor/bordercolor/…) | matugen / `theme` | by design, out of scope |
| window rules / monitors / startup / binds | (structured / managed) | out of scope |

## Per-page wiring checklist (each new page needs all of these)
In `mshell-crates/mshell-settings/src/`:
1. `lib.rs` — `mod <page>_settings;`
2. `settings.rs` import — `use crate::<page>_settings::{<Page>Init, <Page>Model};`
3. `settings.rs` struct field — `<page>_settings_controller: Controller<<Page>Model>,`
4. `settings.rs` launch — `let <page>_settings_controller = <Page>Model::builder().launch(<Page>Init {}).detach();`
5. `settings.rs` struct-init — add `<page>_settings_controller,`
6. `settings.rs` `add_titled(model.<page>_settings_controller.widget(), Some("<page>"), "<Label>")`
7. `settings.rs` sidebar button — `#[name="<page>_btn"]` ToggleButton (`set_group: Some(&general_btn)`, icon, label, `connect_toggled → set_visible_child_name("<page>")`)
8. `settings.rs` button lookup — `"<page>" => Some(&widgets.<page>_btn),`
9. `settings.rs` search registry tuple(s) — `("<label words>", "<page>"),`
10. `settings.rs` search keywords — `"<page>" => "…",`

## Control-type mapping
- bool `0/1` → `gtk::Switch` (`set_active` from `value != 0`, write `if active {"1"} else {"0"}`)
- int (px/ms/distance) → `gtk::SpinButton` (`set_digits: 0`)
- float (opacity/ratio/factor) → `gtk::SpinButton` (`set_digits: 2`)
- enum int → `gtk::DropDown` over a fixed `StringList`, index = value

---

## Task 0: Shared `compositor_conf` helper (DRY foundation)

**Files:**
- Create: `mshell-crates/mshell-settings/src/compositor_conf.rs`
- Modify: `lib.rs` (`mod compositor_conf;`)
- Refactor (optional, low-risk): `animations_settings.rs` / `input_settings.rs` / `overview_settings.rs` to call the shared fns instead of their private copies.

- [ ] **Step 1: Write `compositor_conf.rs`** (lifted from `animations_settings.rs`'s `conf_path`/`read_config`/`patch_conf`/`reload`):

```rust
//! Shared read/patch/reload for margo's `config.conf` from Settings pages.
//! `config.conf` is frequently a dotfiles symlink — `std::fs::write` follows
//! it, so we edit in place (content changes, the symlink stays).
use std::path::PathBuf;

pub(crate) fn conf_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_default();
    base.join("margo").join("config.conf")
}

pub(crate) fn read_config() -> margo_config::Config {
    margo_config::parse_config_with_defaults(Some(&conf_path())).unwrap_or_default()
}

/// Replace each `key = value` line in place (first occurrence), or append if
/// the key is absent. Preserves the rest of the file (comments, layout).
pub(crate) fn patch_conf(updates: &[(&str, String)]) -> std::io::Result<()> {
    let path = conf_path();
    let body = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = body.lines().map(|s| s.to_string()).collect();
    for (key, val) in updates {
        let mut found = false;
        for line in lines.iter_mut() {
            let trimmed = line.trim_start();
            if trimmed.starts_with(key)
                && trimmed[key.len()..].trim_start().starts_with('=')
            {
                *line = format!("{key} = {val}");
                found = true;
                break;
            }
        }
        if !found {
            lines.push(format!("{key} = {val}"));
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    std::fs::write(&path, out)
}

pub(crate) fn reload() {
    if let Err(e) = std::process::Command::new("mctl").args(["reload"]).spawn() {
        tracing::warn!(error = %e, "settings: `mctl reload` failed to spawn");
    }
}
```
NOTE: copy the EXACT `patch_conf` key-matching logic from `animations_settings.rs` if it differs (some configs align `=` with padding — the matcher must tolerate `key   = val`). The version above tolerates leading spaces + padded `=`.

- [ ] **Step 2:** `mod compositor_conf;` in `lib.rs`.
- [ ] **Step 3:** Build `cargo build -p mshell` → compiles.
- [ ] **Step 4:** Commit `feat(mshell-settings): shared compositor_conf read/patch/reload helper`.

(Refactoring the 3 existing pages onto it is optional polish — do it only if it stays trivial; otherwise leave them and just use the helper for the new pages.)

---

## Task 1: Appearance page

**Files:** Create `appearance_settings.rs`; wire per the checklist (label "Appearance", icon `preferences-desktop-display-symbolic` or `view-grid-symbolic`, search `"appearance" => "border radius gap gaps opacity cursor size window"`).

Controls (read current from `read_config()`, write via `patch_conf` + `reload` on change):

| Section | Key | Control | Range/Notes |
|---|---|---|---|
| Border | `borderpx` | spin int | 0–32 |
| Border | `border_radius` | spin int | 0–32 |
| Border | `no_border_when_single` | switch | 0/1 |
| Border | `no_radius_when_single` | switch | 0/1 |
| Gaps | `gappih` `gappiv` `gappoh` `gappov` | spin int ×4 | 0–64 |
| Gaps | `smartgaps` | switch | 0/1 |
| Opacity | `focused_opacity` | spin float | 0.0–1.0, digits 2 |
| Opacity | `unfocused_opacity` | spin float | 0.0–1.0, digits 2 |
| Cursor | `cursor_size` | spin int | 8–96 |

- [ ] Step 1: page model + `view!` (hero + Rows), reading `read_config()` into the model.
- [ ] Step 2: each control's `update` arm mutates + `patch_conf(&[(key, val)])` + `reload()`.
- [ ] Step 3: full sidebar wiring (checklist 1–10).
- [ ] Step 4: `cargo build -p mshell`.
- [ ] Step 5: manual — change borderpx in the page, confirm `config.conf` updates + border changes live (needs the borderpx-reload fix `0bf1ff1a` in the running margo).
- [ ] Step 6: commit.

---

## Task 2: Effects page

**Files:** Create `effects_settings.rs`; wire (label "Effects", icon `weather-clear-night-symbolic`/`emblem-photos-symbolic`, search `"effects" => "shadow shadows blur drop blur layer floating"`).

| Section | Key | Control | Range/Notes |
|---|---|---|---|
| Shadows | `shadows` | switch | 0/1 |
| Shadows | `shadow_only_floating` | switch | 0/1 |
| Shadows | `layer_shadows` | switch | 0/1 |
| Shadows | `shadows_size` | spin int | 0–64 |
| Shadows | `shadows_blur` | spin int | 0–64 |
| Shadows | `shadows_position_x` | spin int | −32–32 |
| Shadows | `shadows_position_y` | spin int | −32–32 |
| Blur | `blur` | switch | 0/1 (note: Kawase not implemented yet — leave a subtitle saying it's a no-op for now) |
| Blur | `blur_layer` | switch | 0/1 |
| Blur | `blur_optimized` | switch | 0/1 |

- [ ] Steps mirror Task 1 (model, control arms, wiring, build, manual, commit).

---

## Task 3: Behaviour page (+ Advanced expander)

**Files:** Create `behaviour_settings.rs`; wire (label "Behaviour", icon `preferences-system-symbolic`, search `"behaviour" => "focus sloppy warp drag snap hot corner scratchpad scroll tearing sync"`).

| Group | Key | Control | Range/Notes |
|---|---|---|---|
| Focus | `focus_on_activate` | switch | 0/1 |
| Focus | `focus_cross_monitor` | switch | 0/1 |
| Focus | `exchange_cross_monitor` | switch | 0/1 |
| Focus | `focus_cross_tag` | switch | 0/1 |
| Focus | `view_current_to_back` | switch | 0/1 |
| Focus | `sloppyfocus` | switch | 0/1 |
| Focus | `warpcursor` | switch | 0/1 (subtitle: avoid with sloppyfocus=1 — ping-pong) |
| Focus | `cursor_hide_timeout` | spin int | 0–30 (s) |
| Focus | `xwayland_persistence` | switch | 0/1 |
| Drag | `drag_tile_to_tile` | switch | 0/1 |
| Drag | `drag_corner` | dropdown | 0 TL / 1 TR / 2 BL / 3 BR / 4 Auto |
| Drag | `drag_warp_cursor` | switch | 0/1 |
| Drag | `drag_tile_refresh_interval` | spin float | 1–60, digits 1 |
| Drag | `drag_floating_refresh_interval` | spin float | 1–60, digits 1 |
| Snap | `enable_floating_snap` | switch | 0/1 |
| Snap | `snap_distance` | spin int | 0–128 |
| Hot corner | `enable_hotarea` | switch | 0/1 |
| Hot corner | `hotarea_size` | spin int | 1–64 |
| Hot corner | `hotarea_corner` | dropdown | 0 TL / 1 TR / 2 BL / 3 BR |
| Scroll | `axis_bind_apply_timeout` | spin int | 0–1000 (ms) |
| Scroll | `axis_scroll_factor` | spin float | 0.1–5.0, digits 2 |
| Scratchpad | `scratchpad_cross_monitor` | switch | 0/1 |
| Scratchpad | `single_scratchpad` | switch | 0/1 |
| **Advanced** (expander) | `syncobj_enable` | switch | 0/1 (subtitle: explicit-sync, for DXVK/Vulkan) |
| **Advanced** | `allow_tearing` | dropdown | 0 off / 1 on / 2 rule-only |
| **Advanced** | `allow_shortcuts_inhibit` | switch | 0/1 |
| **Advanced** | `idleinhibit_ignore_visible` | switch | 0/1 |

- [ ] Use a `gtk::Expander` (or a revealer "Advanced" row) holding the last group.
- [ ] Steps mirror Task 1; manual: flip `sloppyfocus`, confirm live.

---

## Task 4: Overview augment (small)

**Files:** Modify `overview_settings.rs` — add any keys not already present:
`ov_tab_mode` (switch), `overview_transition_ms` (spin int), `overview_zoom` (spin float 0–1), `overview_selected_border_multiplier` (spin float 1.0–4.0).

- [ ] Check which are already wired (the page already covers overview gap/dim/cycle_order/backdrop); add only the missing ones via the shared helper.
- [ ] Build + commit.

---

## Task 5: Final pass

- [ ] `cargo fmt --all` (the CI gate is `cargo fmt --check` — run it: `cargo fmt --all --check`).
- [ ] `cargo clippy -p mshell -p mshell-settings -- -D warnings` clean.
- [ ] `cargo build --release -p mshell`.
- [ ] Push; user rebuilds + restarts mshell; verify each page reads current values and applies live via `mctl reload`.

## Self-review notes
- **Mechanism:** one shared `compositor_conf` (read/patch/reload) — no per-page duplication for the new pages.
- **Live-apply:** every change → `patch_conf` + `mctl reload`; borderpx is live thanks to `0bf1ff1a`. Reads use `margo_config` so the page always reflects the file (including hand-edits).
- **Out of scope (deliberate):** colours (matugen), window rules / monitors / startup / binds (structured or managed by other tools), animations/input/overview-core (already done).
- **Symlink caveat:** `config.conf` is a dotfiles symlink; `fs::write` follows it (edits land in the dotfiles repo) — intended.
- **Debounce:** SpinButton `connect_value_changed` fires per step; that's fine (each writes + reloads). If it feels chatty, debounce later — not required for v1.
