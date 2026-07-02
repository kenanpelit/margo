# Countdown + Audio Route — native ports of two DMS plugins

**Date:** 2026-07-02
**Status:** approved (design)

Port two out-of-tree DankMaterialShell (DMS) QML plugins to margo as
**native** shell features (no QML, no `pactl` shell-out). The two are
independent; this single spec covers both because they are small and
share the bar-widget registration machinery.

Source plugins (reference only, not vendored):

- `TimeUntil` — a bar pill counting down to a target date.
- `Audio Port Switcher` — a bar pill toggling the mic input port on a
  3.5 mm combo jack (internal ↔ headset).

---

## Goals

- **Countdown**: track one or more target dates; show the soonest as a
  glance bar pill ("42 days remaining" / "3 days overdue") and manage
  the list from a new **Countdown** tab in the existing Alarm Clock menu.
- **Audio Route**: a one-click bar pill that flips the whole default
  audio path — **both** the default input (mic) and default output
  (speaker) — between the built-in device port and a headset/external
  port, reusing the `wayle_audio` `set_port` plumbing that already backs
  the audio dashboard's port switcher.

## Non-goals (YAGNI)

- No QML, no Quickshell, no `pactl`/`notify-send` shell-outs.
- Countdown does **not** ring, alert, or play sound — it is a passive
  display. It reuses the Alarm hub's *menu chrome + config store*, not
  its scheduler/tone engine.
- Audio Route does not add per-port UI or profiles — that already exists
  in the audio dashboard menu (`port_switcher.rs`). This is only the
  quick one-click bar pill.
- No new config section for countdown — it extends `[alarm]`.

## Naming (decided)

- Countdown: internal `BarWidget::Countdown` pill + a **Countdown** tab
  in the Alarm Clock menu. TimeUntil folds into the Alarm hub; it gets
  no standalone product name.
- Audio Port Switcher → **`audio_route`** (`BarWidget::AudioRoute`),
  generalized to route both mic and output together.

---

## Feature A — Countdown

### Config (extends `[alarm]`)

Add to the existing alarm config section in
`mshell-crates/mshell-config/src/schema/config.rs` (alongside
`alarms: Vec<Alarm>`):

```rust
countdowns: Vec<Countdown>   // #[serde(default)], skip_serializing_if empty

struct Countdown {
    target: String,          // "2027-01-01 21:37" (time optional → midnight)
    unit: CountdownUnit,     // Hours | Days | Weeks | Months  (default Days)
    label: String,           // suffix; empty → "remaining" / "overdue"
    enabled: bool,           // per-row enable, mirrors Alarm.enabled
}

enum CountdownUnit { Hours, Days, Weeks, Months }
```

Persist + reload via `config_manager().update_config`, exactly like
alarms. `serde(default)` + `skip_serializing_if` per the project's
serde-default rebake caveat (see `docs/config-conventions.md`).

### Pure calc module — `mshell-frame/src/countdown.rs`

Mirrors `crate::stopwatch`. A pure, unit-testable function ports the
TimeUntil arithmetic:

- `fn remaining(target: &str, unit, now: SystemTime) -> Option<f64>` —
  parses the timestamp (time optional), returns signed value rounded to
  0.1 in the chosen unit (`None` on unparseable date). Negative =
  overdue. Months = 30.44-day approximation, matching TimeUntil.
- `fn format_long(value, unit, label) -> String` → "42 days remaining"
  / "1 day overdue" (singular/plural, "overdue" when negative).
- `fn format_short(value, unit) -> String` → "42d" / "3d!" (overdue
  suffix). Unit short forms: h / d / w / mo.
- `fn soonest(countdowns, now) -> Option<usize>` — index of the target
  with the smallest positive remaining; if none upcoming, the least
  overdue (largest, i.e. closest-to-zero negative). Skips disabled and
  unparseable entries.

`now` is a parameter so tests are deterministic (no wall-clock in the
pure layer).

### Menu — third tab

In `alarm_clock_menu_widget.rs`, add a third stack child to the existing
`StackSwitcher` + `Stack`:
`add_titled[Some("countdown"), "Countdown"]`. Content mirrors the Alarms
tab:

- A reactive list rebuilt from `config.alarm().countdowns()` (row =
  enable switch · event name/label · target date · live "X days
  remaining/overdue" · delete), plus an empty-state hint.
- An add row: a date entry (validated against the calc parser), a unit
  dropdown, a label entry, an add button.
- Edits go through `config_manager().update_config`; a reactive
  `EffectScope` on `config.alarm().countdowns()` repaints the list (same
  pattern the Alarms tab already uses).

### Glance pill — `BarWidget::Countdown`

New `mshell-frame/src/bars/bar_widgets/countdown.rs`, sibling of
`alarm_clock.rs`:

- Calendar/hourglass glyph + inline label. Reads
  `config.alarm().countdowns()` reactively; picks `countdown::soonest`.
- Horizontal bar → `format_long`; vertical bar → `format_short`. Hidden
  when the list is empty (no enabled, parseable target).
- Recompute on a coarse heartbeat (once a minute is ample — the display
  only ticks in 0.1-unit steps) and on config change. Follows the alarm
  pill's "quiet pill is free" command-loop shape.
- Click → open the Alarm Clock menu **with the Countdown tab selected**.
  Implemented via the existing menu-open path plus a tab hint the menu
  reads on reveal (exact mechanism pinned in the plan; candidates: a
  small module-level `Cell`/atomic set before open, or an extra field on
  the menu-open input). Default tab for a plain Alarm pill click stays
  "Alarms".

---

## Feature B — `audio_route` pill

New `mshell-frame/src/bars/bar_widgets/audio_route.rs`. A pill-sized,
"both devices together" descendant of `audio_dashboard/port_switcher.rs`.
All `wayle_audio`, no `pactl`.

### Data

- Watch default input + default output devices via
  `mshell_utils::audio::spawn_default_{input,output}_watcher`, and each
  device's ports via `spawn_{input,output}_device_ports_watcher`.
- Per device: `dev.ports.get() -> Vec<DevicePort{name, description,
  available}>` and `dev.active_port.get() -> Option<String>`.

### Port classification (heuristic)

For each device, classify its ports into two buckets by name +
description (case-insensitive substring), porting the original plugin's
approach:

- **headset / external**: `headset`, `headphone`, `bluez`, `usb`, `hdmi`
  (external-ish).
- **internal / built-in**: `internal`, `speaker`, `builtin`, `front`,
  `analog-output-speaker`, `analog-input-internal-mic`.

Pick, per device, a headset candidate + an internal candidate. If a
device exposes ≥2 ports but the heuristic can't split them, fall back to
"the two ports as-is" (toggle between whichever two).

### Toggle

- Current route = "is the active port the headset candidate?" (evaluated
  per device; the pill's displayed state uses whichever device is
  routable).
- Click: if currently internal → `set_port(headset)` on **both** input
  and output devices (each `tokio::spawn`ed, `available` ports only);
  else → `set_port(internal)` on both. Devices that only have one port,
  or whose target is unavailable, are skipped individually.
- Icon reflects the route: `headset_mic` / `headphones` when on headset,
  `computer` / `speaker` when internal. Optional tooltip names the route.

### Auto-hide

`set_visible(false)` when **neither** the input nor the output device
exposes ≥2 ports (nothing to switch). Visible when at least one side is
routable, and it switches whichever side(s) are.

---

## Shared wiring (both pills)

Per `mshell-frame/DESIGN.md` bar→pill checklist:

1. `BarWidget` enum (`mshell-config/src/schema/bar_widgets.rs`): add
   `AudioRoute` and `Countdown` variants with doc comments.
2. Pill modules: `bars/bar_widgets/{countdown,audio_route}.rs` + `mod`
   entries.
3. Dispatch arm in `bars/bar.rs` (construct each pill for its slot).
4. `BarPillKind` + Settings sidebar registration
   (`mshell-settings/src/bar_settings/bar_widget_section.rs`, and
   `settings.rs` as needed) so both appear in Settings → Bar widgets.
5. Icons under `assets/icons/MargoMaterial/symbolic/` (countdown: a
   schedule/hourglass glyph; audio_route: reuse/adjacent headset glyph).
6. SCSS in `mshell-style/scss/04-components/` reusing existing
   `ok-bar-widget` / bar-pill tokens — **no hardcoded colours** (matugen
   CSS vars only, per DESIGN.md).

Both pills are **opt-in**: present in the enum + Settings, but **not**
added to any default bar layout. The user adds them.

---

## Testing

- `countdown.rs` pure fns: remaining (per unit; exact-1 singular; past →
  overdue; unparseable → None), `format_long`/`format_short` wording,
  `soonest` selection (mix of upcoming/overdue/disabled/invalid). All
  with an injected `now`.
- `audio_route` classification: from sample `DevicePort` lists, assert
  the headset/internal split and the chosen toggle target; assert
  auto-hide when <2 ports both sides.
- `just check` green: fmt + clippy `--all-targets -D warnings` +
  panic-ratchet + `mctl check-config` example parse + test.

---

## Decisions locked

- **Fold vs standalone**: TimeUntil → Countdown tab in the Alarm menu +
  a separate opt-in glance pill. (User.)
- **Countdown multiplicity**: multiple targets (list); pill shows the
  soonest upcoming. (User.)
- **audio_route scope**: one click routes **both** mic + output together
  (headset ↔ internal). (User.)
- **Countdown config location**: extend `[alarm]` (`config.alarm.
  countdowns`), no new section. (User.)
- **Countdown pill click**: opens the Alarm menu on the Countdown tab
  (not a standalone popout). (User.)
- **Default visibility**: both pills opt-in, not in any default bar
  layout. (User.)

## Out of scope

- Migrating existing DMS `settings.json` plugin data — fresh native
  config, no import.
- Bilingual (es/en) settings text from the original Audio Port Switcher
  — margo shell strings stay English (user-facing Turkish is a separate
  concern, not applicable to internal bar labels here).
