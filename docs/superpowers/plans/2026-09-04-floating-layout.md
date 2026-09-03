# Floating Layout Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `floating` layout to margo that gives a classic stacking / floating desktop (GNOME / Plasma / XFCE style), selectable per-tag or globally, alongside the existing tiling layouts.

**Architecture:** `floating` is a `LayoutId` variant whose arrange function returns nothing (like `Canvas`). A new `MargoState::reconcile_floating_layout` runs at the top of every `arrange_monitor` pass: on a tag whose layout is `Floating` it auto-floats every governed tiled client and seeds a cascaded `float_geom`; on any other layout it re-tiles the clients it previously auto-floated. Hand-floated windows are never touched. The existing float-apply pass in `arrange_monitor` then writes each `float_geom` to `geom` unchanged.

**Tech Stack:** Rust (edition 2024, rustc pinned 1.95.0), the `margo-layouts` pure-function crate, the `margo` compositor crate, the `margo/src/tests` fixture harness (real `MargoState` + headless outputs + Wayland clients), `insta` text snapshots, GTK4 (`mvisual`, `mshell-settings`).

**Spec:** `docs/superpowers/specs/2026-09-04-floating-layout-design.md`

## Global Constraints

- **rustc pinned 1.95.0.** CI fmt uses `cargo +1.95.0 fmt --all` (pacman rust ignores `rust-toolchain.toml`).
- **CI gate before push (`just check`):** `cargo +1.95.0 fmt --all` + `cargo clippy --all-targets -D warnings` + `./scripts/panic-ratchet.sh` + `./scripts/design-lint.sh` + `cargo test`. Per repo workflow the **human runs the full compile/test/`just check` cycle and the push**; each task below lists the exact targeted `cargo test -p <crate>` command — run it if you have build access, otherwise hand the batch to the human. Never run a full `cargo build --release -p margo` as a verification step.
- **panic-ratchet:** `.unwrap()` / `.expect(` / `panic!(` / `unreachable!(` / `todo!(` / `unimplemented!(` in **non-test** Rust may not grow past `scripts/panic-baseline.txt`. Files under any `/tests/` path, `#[test]` fn bodies, and `#[cfg(test)] mod foo;` gated files are exempt — `margo/src/tests/*` and `margo/src/layout/snapshot_tests.rs` are all exempt.
- **design-lint:** SCSS must not hardcode colours or `border-radius: <px>` — not relevant here (no SCSS changes), but keep it in mind if you touch style.
- **Commits:** English. End every commit message with:
  ```
  Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01QtWkq4eTWDHYM32zyD8vdZ
  ```
  The **Task 3 commit** (feature functionally complete) MUST contain `Closes #1` in its body — this closes <https://github.com/kenanpelit/margo/issues/1> on push to `main`. Other commits may add `Refs #1`.
- **`LayoutId` is `#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]`** — no numeric discriminant is relied on anywhere; `Pertag.ltidxs` stores `LayoutId` values directly, never indices.
- **The only exhaustive `match` on `LayoutId` variants in the whole workspace** are: `margo-layouts/src/lib.rs` `symbol()` + `name()`, `margo-layouts/src/algorithms.rs` `arrange()`, and `margo/src/layout/snapshot_tests.rs::arrange_dispatcher_matches_direct_call_all_layouts`. Every other `match` on a layout has a `_` arm or matches a `String`. Task 1 fixes all four.

---

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `margo-layouts/src/lib.rs` | `LayoutId::Floating` variant + `symbol`/`name`/`from_symbol`/`from_name`/`all_tileable` metadata | 1 |
| `margo-layouts/src/algorithms.rs` | `floating()` no-op arrange fn + `arrange()` dispatch arm + `place_floating_cascade()` pure helper | 1, 2 |
| `margo-layouts/tests/layouts.rs` | `floating()` returns empty; `place_floating_cascade()` cascade math; metadata round-trips; `all_tileable` count → 11 | 1, 2 |
| `margo/src/layout/snapshot_tests.rs` | exhaustive-match fix; `floating` no-op snapshot scenario | 1 |
| `margo/src/state/data.rs` | `MargoClient.floated_by_layout: bool` field + `new()` init | 3 |
| `margo/src/state/groups.rs` | `pub(crate) fn dissolve_group(&mut self, gid: u32)` | 3 |
| `margo/src/state/arrange.rs` | `reconcile_floating_layout(mon_idx)` method; call site in `arrange_monitor`; `smartgaps`/`monly` guards | 3 |
| `margo/src/state/debug_dump.rs` | `floated_by_layout` in the per-client dump line | 3 |
| `margo/src/state/state_file.rs` | `"floated_by_layout"` in the per-client JSON; `"floating"` in `LAYOUT_NAMES` | 3, 5 |
| `margo/src/tests/floating_layout.rs` (new) + `margo/src/tests/mod.rs` | integration tests for `reconcile_floating_layout` via the fixture | 3 |
| `margo/src/dispatch/mod.rs` | `LayoutId::Floating` in `ALL_LAYOUTS` (index space for `setlayoutindex` + snapshot `layouts[]`) | 5 |
| `mctl/src/actions.rs` | `"floating"` in `LAYOUT_NAMES` + the `setlayout` detail string | 5 |
| `mvisual/src/main.rs` | `render_layout` special-case: draw 3 cascade rects for `LayoutId::Floating` | 4 |
| `mshell-crates/mshell-settings/src/tag_layout_settings.rs` | `"floating"` appended to `LAYOUTS` | 4 |
| `road_map.md`, `README.md`, `docs/config-conventions.md`, `docs/protocol-comparison.md` | layout list / precedence note | 6 |

---

## Task 1: `LayoutId::Floating` variant + no-op arrange + metadata

**Files:**
- Modify: `margo-layouts/src/lib.rs` (enum ~L124; `symbol()` ~L140; `name()` ~L156; `from_symbol()` ~L172; `from_name()` ~L188; `all_tileable()` ~L207)
- Modify: `margo-layouts/src/algorithms.rs` (add `floating()` after `canvas()` ~L480; `arrange()` dispatch ~L534)
- Modify: `margo-layouts/tests/layouts.rs` (`ALL_LAYOUTS` ~L42; `each_layout_places_every_client_exactly_once` ~L98; `all_tileable_has_10_entries_and_excludes_overview` ~L265; new test)
- Modify: `margo/src/layout/snapshot_tests.rs` (`arrange_dispatcher_matches_direct_call_all_layouts` match ~L509; `empty_input_yields_empty_output_for_every_layout` ~L694; new test)

**Interfaces:**
- Produces:
  - `LayoutId::Floating` — new variant, placed **immediately before `LayoutId::Overview`**.
  - `LayoutId::Floating.name() == "floating"`, `LayoutId::Floating.symbol() == "F"`.
  - `LayoutId::from_name("floating") == Some(LayoutId::Floating)`, `LayoutId::from_symbol("F") == Some(LayoutId::Floating)`.
  - `LayoutId::all_tileable()` now has 11 entries, `Floating` last, still excludes `Overview`.
  - `margo_layouts::floating(ctx: &ArrangeCtx) -> ArrangeResult` — always `vec![]`.
  - `margo_layouts::arrange(LayoutId::Floating, _) == vec![]`.

- [ ] **Step 1: Write the failing tests in `margo-layouts/tests/layouts.rs`**

Add a new test and update the count assertion:

```rust
#[test]
fn floating_arranges_nothing_through_the_normal_path() {
    let gaps = GapConfig::default();
    for n in [0usize, 1, 3, 8] {
        let tiled: Vec<usize> = (0..n).collect();
        let props = props_for(&tiled);
        assert!(
            arrange(LayoutId::Floating, &ctx(&tiled, &gaps, &props, 1, 0.55)).is_empty(),
            "floating produced rects for n={n}"
        );
    }
}

#[test]
fn floating_name_and_symbol_round_trip() {
    assert_eq!(LayoutId::from_name("floating"), Some(LayoutId::Floating));
    assert_eq!(LayoutId::from_symbol("F"), Some(LayoutId::Floating));
    assert_eq!(LayoutId::Floating.name(), "floating");
    assert_eq!(LayoutId::Floating.symbol(), "F");
}
```

Change the existing `all_tileable_has_10_entries_and_excludes_overview` (rename + bump):

```rust
#[test]
fn all_tileable_has_11_entries_and_excludes_overview() {
    let tileable = LayoutId::all_tileable();
    assert_eq!(tileable.len(), 11);
    assert!(!tileable.contains(&LayoutId::Overview));
    assert!(tileable.contains(&LayoutId::Floating));
}
```

In `ALL_LAYOUTS` (the const at ~L42) add `LayoutId::Floating,` immediately before `LayoutId::Overview,`.

In `each_layout_places_every_client_exactly_once` (~L98), widen the Canvas skip:

```rust
        // Canvas and Floating position clients outside the arrange path
        // and return nothing.
        if layout == LayoutId::Canvas || layout == LayoutId::Floating {
            assert!(arrange(layout, &c).is_empty());
            continue;
        }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p margo-layouts`
Expected: FAIL — `LayoutId::Floating` does not exist (compile error), `all_tileable` len is 10.

- [ ] **Step 3: Add the `LayoutId::Floating` variant + metadata in `margo-layouts/src/lib.rs`**

Enum (~L124) — insert before `Overview`:

```rust
    Dwindle,
    /// Stacking / floating desktop — the tiler produces no geometry;
    /// every client keeps its own `float_geom`. The compositor
    /// auto-floats tiled clients and cascades their placement in
    /// `reconcile_floating_layout`.
    Floating,
    Overview,
```

`symbol()` (~L151) — add before the `Overview` arm:

```rust
            LayoutId::Dwindle => "DW",
            LayoutId::Floating => "F",
            LayoutId::Overview => "󰃇",
```

`name()` (~L167) — add before the `Overview` arm:

```rust
            LayoutId::Dwindle => "dwindle",
            LayoutId::Floating => "floating",
            LayoutId::Overview => "overview",
```

`from_symbol()` (~L183) and `from_name()` (~L199) — add `LayoutId::Floating,` after `LayoutId::Dwindle,` in **both** local `all` arrays.

`all_tileable()` (~L218) — add `LayoutId::Floating,` after `LayoutId::Dwindle,`.

- [ ] **Step 4: Add `floating()` + the dispatch arm in `margo-layouts/src/algorithms.rs`**

After `canvas()` (~L480):

```rust
// ── Floating (stacking desktop — positions set by the compositor) ────────────

/// Floating layout — the tiler produces nothing; every client keeps
/// its own `float_geom`. The compositor auto-floats tiled clients and
/// cascades their placement in `reconcile_floating_layout`, and the
/// existing float-apply pass in `arrange_monitor` writes `float_geom`
/// to `geom`.
pub fn floating(_ctx: &ArrangeCtx) -> ArrangeResult {
    vec![]
}
```

`arrange()` (~L544) — add before the `Overview` arm:

```rust
        LayoutId::Dwindle => dwindle(ctx),
        LayoutId::Floating => floating(ctx),
        LayoutId::Overview => monocle(ctx), // overview handled elsewhere
```

- [ ] **Step 5: Fix the exhaustive match in `margo/src/layout/snapshot_tests.rs`**

In `arrange_dispatcher_matches_direct_call_all_layouts` (~L509) change the Canvas arm:

```rust
            LayoutId::Canvas | LayoutId::Floating => {
                unreachable!("Canvas / Floating are filtered out of the test loop earlier")
            }
```

In `empty_input_yields_empty_output_for_every_layout` (~L694), add next to the Canvas assertion:

```rust
    assert!(arrange(LayoutId::Canvas, &ctx).is_empty());
    assert!(arrange(LayoutId::Floating, &ctx).is_empty());
```

Add a scenario snapshot test near the other single-layout tests:

```rust
// ── floating ───────────────────────────────────────────────────────────────

#[test]
fn floating_three_windows_arranges_nothing() {
    // The floating layout produces no tiled geometry — clients keep
    // their own float_geom, seeded by the compositor's reconcile pass.
    let f = Fixture::with_windows(HD_1080P, 3);
    let ctx = f.ctx().build();
    assert_snapshot!(format_arranged(&arrange(LayoutId::Floating, &ctx)));
}
```

- [ ] **Step 6: Write the new snapshot baseline**

Run: `INSTA_UPDATE=always cargo test -p margo --lib layout::snapshot_tests::floating_three_windows_arranges_nothing`
Then: `git diff -- margo/src/layout/snapshots/` — confirm `floating_three_windows_arranges_nothing.snap` contains an empty arrange body (header only, no rect lines).

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p margo-layouts` and `cargo test -p margo --lib layout::snapshot_tests`
Expected: PASS — including `floating_arranges_nothing_through_the_normal_path`, `floating_name_and_symbol_round_trip`, `all_tileable_has_11_entries_and_excludes_overview`, `layout_names_round_trip`, `layout_symbols_round_trip`, `arrange_dispatcher_matches_direct_call_all_layouts`.

- [ ] **Step 8: Commit**

```bash
git add margo-layouts/src/lib.rs margo-layouts/src/algorithms.rs margo-layouts/tests/layouts.rs \
        margo/src/layout/snapshot_tests.rs margo/src/layout/snapshots/
git commit -m "$(cat <<'EOF'
feat(layouts): add LayoutId::Floating (no-op tiler)

New layout variant that produces no tiled geometry, the same shape as
Canvas. name()="floating", symbol()="F", in all_tileable() (11 entries)
but deliberately not in any cyclelayout rotation. The compositor-side
auto-float / cascade behaviour lands in the next commits.

Refs #1

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01QtWkq4eTWDHYM32zyD8vdZ
EOF
)"
```

---

## Task 2: `place_floating_cascade()` pure helper

**Files:**
- Modify: `margo-layouts/src/algorithms.rs` (add after `floating()`)
- Modify: `margo-layouts/tests/layouts.rs` (new tests)

**Interfaces:**
- Consumes: `Rect` (from `margo_layouts`).
- Produces:
  ```rust
  pub fn place_floating_cascade(
      work_area: Rect,
      preferred: Option<(i32, i32)>, // client's committed toplevel size, if any
      min: (i32, i32),              // 0 = unset
      max: (i32, i32),              // 0 = unset
      cascade_index: usize,         // count of floating clients already placed on this tag
  ) -> Rect
  ```
  Rules: size = `preferred` when both dims > 0 and it fits `work_area`, else 60% of `work_area`; then clamp to `min`/`max` (ignoring 0s) and to `[1, work_area dim]`. Position = `work_area` top-left + a 24px inset + `cascade_index * 32` px down-right, the diagonal offset wrapping as a unit before the window's bottom-right would leave `work_area`. Final rect clamped fully inside `work_area`. Deterministic and idempotent (same inputs → same rect).

- [ ] **Step 1: Write the failing tests in `margo-layouts/tests/layouts.rs`**

```rust
use margo_layouts::place_floating_cascade;

const FWA: Rect = Rect { x: 0, y: 0, width: 1000, height: 600 };

#[test]
fn place_floating_falls_back_to_60_percent_when_no_preferred_size() {
    let r = place_floating_cascade(FWA, None, (0, 0), (0, 0), 0);
    assert_eq!(r, Rect::new(24, 24, 600, 360));
}

#[test]
fn place_floating_cascades_down_and_right_by_32px() {
    assert_eq!(place_floating_cascade(FWA, None, (0, 0), (0, 0), 1), Rect::new(56, 56, 600, 360));
    assert_eq!(place_floating_cascade(FWA, None, (0, 0), (0, 0), 2), Rect::new(88, 88, 600, 360));
}

#[test]
fn place_floating_wraps_the_cascade() {
    // 60% box on FWA leaves 216px of vertical slack; step 32 → wrap every 6.
    let base = place_floating_cascade(FWA, None, (0, 0), (0, 0), 0);
    assert_eq!(place_floating_cascade(FWA, None, (0, 0), (0, 0), 6), base);
}

#[test]
fn place_floating_uses_the_committed_size_when_it_fits() {
    let r = place_floating_cascade(FWA, Some((800, 400)), (0, 0), (0, 0), 0);
    assert_eq!(r, Rect::new(24, 24, 800, 400));
}

#[test]
fn place_floating_ignores_a_committed_size_that_does_not_fit() {
    let r = place_floating_cascade(FWA, Some((1200, 700)), (0, 0), (0, 0), 0);
    assert_eq!(r, Rect::new(24, 24, 600, 360)); // fell back to 60%
}

#[test]
fn place_floating_honours_min_constraints() {
    let r = place_floating_cascade(FWA, Some((100, 80)), (400, 300), (0, 0), 0);
    assert_eq!(r, Rect::new(24, 24, 400, 300));
}

#[test]
fn place_floating_keeps_the_rect_inside_the_work_area() {
    for idx in 0..40usize {
        let r = place_floating_cascade(FWA, None, (0, 0), (0, 0), idx);
        assert!(r.x >= FWA.x && r.y >= FWA.y);
        assert!(r.x + r.width <= FWA.x + FWA.width);
        assert!(r.y + r.height <= FWA.y + FWA.height);
    }
}

#[test]
fn place_floating_respects_a_non_zero_work_area_origin() {
    let wa = Rect::new(100, 50, 1000, 600);
    let r = place_floating_cascade(wa, None, (0, 0), (0, 0), 0);
    assert_eq!(r, Rect::new(124, 74, 600, 360));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p margo-layouts place_floating`
Expected: FAIL — `place_floating_cascade` not found.

- [ ] **Step 3: Implement `place_floating_cascade()` in `margo-layouts/src/algorithms.rs`**

Add directly after `floating()`:

```rust
/// Cascade placement for the `floating` layout. Pure geometry — the
/// compositor calls this once per newly-auto-floated client to seed
/// its `float_geom`.
///
/// * `preferred` — the client's committed toplevel size, used only
///   when both dimensions are positive and it fits `work_area`;
///   otherwise the window opens at 60% of the work area.
/// * `min` / `max` — the client's size constraints (`0` = unset).
/// * `cascade_index` — how many floating clients were already placed
///   on this tag; each step shifts the window 32px down-right, the
///   diagonal wrapping as a unit before it would leave `work_area`.
pub fn place_floating_cascade(
    work_area: Rect,
    preferred: Option<(i32, i32)>,
    min: (i32, i32),
    max: (i32, i32),
    cascade_index: usize,
) -> Rect {
    const INSET: i32 = 24;
    const STEP: i32 = 32;

    // 1. Size.
    let fallback = (
        (work_area.width as f32 * 0.6) as i32,
        (work_area.height as f32 * 0.6) as i32,
    );
    let (mut w, mut h) = match preferred {
        Some((pw, ph))
            if pw > 0 && ph > 0 && pw <= work_area.width && ph <= work_area.height =>
        {
            (pw, ph)
        }
        _ => fallback,
    };
    if min.0 > 0 {
        w = w.max(min.0);
    }
    if min.1 > 0 {
        h = h.max(min.1);
    }
    if max.0 > 0 {
        w = w.min(max.0);
    }
    if max.1 > 0 {
        h = h.min(max.1);
    }
    w = w.clamp(1, work_area.width.max(1));
    h = h.clamp(1, work_area.height.max(1));

    // 2. Diagonal cascade, wrapping as a unit.
    let slack_x = (work_area.width - w - INSET).max(0);
    let slack_y = (work_area.height - h - INSET).max(0);
    let cycle = (slack_x.min(slack_y) / STEP).max(1);
    let offset = (cascade_index as i32 % cycle) * STEP;
    let mut x = work_area.x + INSET + offset;
    let mut y = work_area.y + INSET + offset;

    // 3. Clamp fully inside.
    x = x.min(work_area.x + work_area.width - w).max(work_area.x);
    y = y.min(work_area.y + work_area.height - h).max(work_area.y);

    Rect::new(x, y, w, h)
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p margo-layouts place_floating`
Expected: PASS — all 8 tests.

- [ ] **Step 5: Commit**

```bash
git add margo-layouts/src/algorithms.rs margo-layouts/tests/layouts.rs
git commit -m "$(cat <<'EOF'
feat(layouts): add place_floating_cascade() pure helper

Deterministic cascade placement for the floating layout: 60% work-area
box (or the committed toplevel size when it fits), clamped to the
client's min/max, offset 32px per already-placed floating client, the
diagonal wrapping as a unit and the rect clamped fully inside the work
area.

Refs #1

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01QtWkq4eTWDHYM32zyD8vdZ
EOF
)"
```

---

## Task 3: `reconcile_floating_layout` — auto-float / re-tile

**Files:**
- Modify: `margo/src/state/data.rs` (`MargoClient` struct ~L184 after `group_active`; `MargoClient::new` ~L286 after `group_active: false,`)
- Modify: `margo/src/state/groups.rs` (add `dissolve_group` near `dissolve_if_degenerate` ~L98)
- Modify: `margo/src/state/arrange.rs` (new method before `arrange_monitor` ~L67; call site ~L106; `smartgaps` guard ~L206; `monly` guard ~L241)
- Modify: `margo/src/state/debug_dump.rs` (per-client `tracing::info!` ~L43)
- Modify: `margo/src/state/state_file.rs` (per-client `json!` ~L180)
- Create: `margo/src/tests/floating_layout.rs`
- Modify: `margo/src/tests/mod.rs` (add `mod floating_layout;`)

**Interfaces:**
- Consumes: `margo_layouts::place_floating_cascade` (Task 2); `LayoutId::Floating` (Task 1); `FullscreenMode` (already imported in `arrange.rs`); `MargoClient` (already imported in `arrange.rs`).
- Produces:
  - `MargoClient.floated_by_layout: bool` — `false` by default; `true` only while the `Floating` layout is auto-floating this client. A client with `is_floating && !floated_by_layout` was floated by the user (`togglefloating` / `isfloating:1` rule) and is never touched by the reconcile.
  - `MargoState::reconcile_floating_layout(&mut self, mon_idx: usize)` — private; called once at the top of `arrange_monitor`, after `maybe_apply_adaptive_layout`, before `let mon = &self.monitors[mon_idx]`. Idempotent.
  - `MargoState::dissolve_group(&mut self, gid: u32)` — `pub(crate)`; clears `group_id` / `group_active` on every member of `gid`.

- [ ] **Step 1: Write the failing integration tests**

Create `margo/src/tests/floating_layout.rs`:

```rust
//! `floating` layout — auto-float / re-tile reconciliation (issue #1).
//!
//! Drives real xdg_toplevels through the fixture so the windows live in
//! `state.clients`, are mapped, and past their deferred initial map —
//! then flips the current tag's layout and asserts what
//! `reconcile_floating_layout` (run inside `arrange_monitor`) does.

use super::client::ClientId;
use super::fixture::Fixture;
use crate::layout::LayoutId;

/// Map one focused toplevel and drive the deferred-map flow to
/// completion (so `is_initial_map_pending` clears).
fn map_window(fx: &mut Fixture) -> ClientId {
    let id = fx.add_client();
    let (_toplevel, surface) = fx.client(id).create_toplevel();
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    id
}

/// Three mapped windows on one 1080p output.
fn three_windows(fx: &mut Fixture) -> [ClientId; 3] {
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    [map_window(fx), map_window(fx), map_window(fx)]
}

fn pin_layout(fx: &mut Fixture, tag: usize, layout: LayoutId) {
    fx.server.state.monitors[0].pertag.ltidxs[tag] = layout;
    fx.server.state.monitors[0].pertag.user_picked_layout[tag] = true;
}

#[test]
fn floating_layout_auto_floats_every_tiled_client() {
    let mut fx = Fixture::new();
    let _ = three_windows(&mut fx);
    assert!(fx.server.state.clients.iter().all(|c| !c.is_floating));

    pin_layout(&mut fx, 1, LayoutId::Floating);
    fx.server.state.arrange_monitor(0);

    for c in &fx.server.state.clients {
        assert!(c.is_floating, "client not auto-floated under Floating");
        assert!(c.floated_by_layout, "floated_by_layout flag not set");
        assert!(c.float_geom.width > 0 && c.float_geom.height > 0);
        assert_eq!(c.geom, c.float_geom, "geom not applied from float_geom");
    }
}

#[test]
fn floating_layout_cascades_placement() {
    let mut fx = Fixture::new();
    let _ = three_windows(&mut fx);
    pin_layout(&mut fx, 1, LayoutId::Floating);
    fx.server.state.arrange_monitor(0);

    let mut origins: Vec<(i32, i32)> = fx
        .server
        .state
        .clients
        .iter()
        .map(|c| (c.float_geom.x, c.float_geom.y))
        .collect();
    origins.sort();
    origins.dedup();
    assert_eq!(origins.len(), 3, "cascade left windows stacked at one spot");
}

#[test]
fn switching_away_from_floating_re_tiles_auto_floated_clients() {
    let mut fx = Fixture::new();
    let _ = three_windows(&mut fx);
    pin_layout(&mut fx, 1, LayoutId::Floating);
    fx.server.state.arrange_monitor(0);
    assert!(fx.server.state.clients.iter().all(|c| c.is_floating));

    pin_layout(&mut fx, 1, LayoutId::Tile);
    fx.server.state.arrange_monitor(0);

    for c in &fx.server.state.clients {
        assert!(!c.is_floating, "auto-floated client not re-tiled");
        assert!(!c.floated_by_layout);
    }
}

#[test]
fn hand_floated_client_survives_switch_to_tiling() {
    let mut fx = Fixture::new();
    let _ = three_windows(&mut fx);

    // User floats window 0 by hand (togglefloating semantics).
    fx.server.state.clients[0].is_floating = true;
    fx.server.state.clients[0].floated_by_layout = false;

    pin_layout(&mut fx, 1, LayoutId::Floating);
    fx.server.state.arrange_monitor(0);
    pin_layout(&mut fx, 1, LayoutId::Tile);
    fx.server.state.arrange_monitor(0);

    assert!(fx.server.state.clients[0].is_floating, "hand-float lost");
    assert!(!fx.server.state.clients[0].floated_by_layout);
    assert!(!fx.server.state.clients[1].is_floating);
    assert!(!fx.server.state.clients[2].is_floating);
}

#[test]
fn reconcile_is_idempotent() {
    let mut fx = Fixture::new();
    let _ = three_windows(&mut fx);
    pin_layout(&mut fx, 1, LayoutId::Floating);
    fx.server.state.arrange_monitor(0);
    let first: Vec<_> = fx.server.state.clients.iter().map(|c| c.float_geom).collect();

    fx.server.state.arrange_monitor(0);
    fx.server.state.arrange_monitor(0);
    let after: Vec<_> = fx.server.state.clients.iter().map(|c| c.float_geom).collect();

    assert_eq!(first, after, "repeated arrange passes drifted the cascade");
}

#[test]
fn floating_layout_dissolves_tabbed_groups() {
    let mut fx = Fixture::new();
    let _ = three_windows(&mut fx);

    // Group windows 0 + 1.
    let w0 = fx.server.state.clients[0].window.clone();
    fx.server
        .state
        .focus_surface(Some(crate::state::FocusTarget::Window(w0)));
    fx.server.state.toggle_group();
    assert!(fx.server.state.clients[0].group_id.is_some());

    pin_layout(&mut fx, 1, LayoutId::Floating);
    fx.server.state.arrange_monitor(0);

    assert!(
        fx.server.state.clients.iter().all(|c| c.group_id.is_none()),
        "floating layout must dissolve tabbed groups"
    );
}
```

Add to `margo/src/tests/mod.rs` in the alphabetical block (after `mod focus_mon;`):

```rust
mod floating_layout;
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p margo --lib tests::floating_layout`
Expected: FAIL — `floated_by_layout` field does not exist; `reconcile_floating_layout` never runs.

- [ ] **Step 3: Add the `floated_by_layout` field in `margo/src/state/data.rs`**

Struct — after `pub group_active: bool,` (~L184):

```rust
    pub group_active: bool,
    /// `true` only while the `Floating` layout is auto-floating this
    /// client (set/cleared by `reconcile_floating_layout`). Lets a
    /// switch back to a tiling layout re-tile the windows the layout
    /// floated without disturbing windows the user floated by hand
    /// (`is_floating && !floated_by_layout`).
    pub floated_by_layout: bool,
```

`MargoClient::new` — after `group_active: false,` (~L286):

```rust
            group_active: false,
            floated_by_layout: false,
```

- [ ] **Step 4: Add `dissolve_group` in `margo/src/state/groups.rs`**

After `dissolve_if_degenerate` (~L98):

```rust
    /// Dissolve an entire group: every member becomes a plain
    /// ungrouped window. Used when a tag switches to the `Floating`
    /// layout, where a tabbed group (a tiling construct) has no
    /// meaning.
    pub(crate) fn dissolve_group(&mut self, gid: u32) {
        for c in self.clients.iter_mut() {
            if c.group_id == Some(gid) {
                c.group_id = None;
                c.group_active = false;
            }
        }
    }
```

- [ ] **Step 5: Add `reconcile_floating_layout` + call site + guards in `margo/src/state/arrange.rs`**

Add the method immediately before `pub fn arrange_monitor` (~L67), inside the same `impl MargoState` block:

```rust
    /// Auto-float / re-tile clients for the `Floating` layout. Runs at
    /// the top of every `arrange_monitor` pass (issue #1).
    ///
    /// On a tag whose layout is `Floating`, every governed tiled
    /// client is switched to floating (`floated_by_layout` marks it as
    /// ours) and given a cascaded `float_geom`; the existing
    /// float-apply pass then writes that to `geom`. On any other
    /// layout, clients we previously auto-floated are returned to the
    /// tiled set. Clients the user floated by hand are never touched.
    /// Idempotent: a second pass with no state change is a no-op.
    fn reconcile_floating_layout(&mut self, mon_idx: usize) {
        let Some(mon) = self.monitors.get(mon_idx) else {
            return;
        };
        if mon.is_overview {
            return;
        }
        let curtag = mon.pertag.curtag;
        let is_floating_layout = mon.pertag.ltidxs.get(curtag).copied()
            == Some(crate::layout::LayoutId::Floating);
        let tagset = mon.current_tagset();
        let work_area = mon.work_area;

        // Clients on this monitor, visible on the current tagset, that
        // the floating layout governs. Excludes fullscreen (the
        // fullscreen override wins above `is_floating`), scratchpad /
        // overlay (own visibility model), and not-yet-mapped / dying /
        // hidden-group-member clients.
        let governed: Vec<usize> = self
            .clients
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                c.monitor == mon_idx
                    && !c.is_initial_map_pending
                    && c.is_visible_on(mon_idx, tagset)
                    && c.fullscreen_mode == FullscreenMode::Off
                    && !c.is_in_scratchpad
                    && !c.is_named_scratchpad
                    && !c.is_overlay
                    && !c.is_minimized
                    && !c.is_killing
                    && !c.is_hidden_group_member()
            })
            .map(|(i, _)| i)
            .collect();

        if is_floating_layout {
            // A tabbed group is a tiling construct — dissolve any on
            // this tag before floating its members.
            let gids: std::collections::BTreeSet<u32> = governed
                .iter()
                .filter_map(|&i| self.clients[i].group_id)
                .collect();
            for gid in gids {
                self.dissolve_group(gid);
            }

            // Cascade slots already consumed by floating clients on
            // this tag (recomputed each pass → stable / idempotent).
            let mut cascade_index = governed
                .iter()
                .filter(|&&i| {
                    self.clients[i].is_floating && self.clients[i].float_geom.width != 0
                })
                .count();

            for &i in &governed {
                let needs_geom = self.clients[i].float_geom.width == 0;
                if !self.clients[i].is_floating {
                    self.clients[i].is_floating = true;
                    self.clients[i].floated_by_layout = true;
                }
                if needs_geom {
                    let c = &self.clients[i];
                    let preferred = if c.geom.width > 0 && c.geom.height > 0 {
                        Some((c.geom.width, c.geom.height))
                    } else {
                        None
                    };
                    let seed = crate::layout::place_floating_cascade(
                        work_area,
                        preferred,
                        (c.min_width, c.min_height),
                        (c.max_width, c.max_height),
                        cascade_index,
                    );
                    self.clients[i].float_geom = seed;
                    cascade_index += 1;
                }
                self.clients[i].geom = self.clients[i].float_geom;
            }
        } else {
            for &i in &governed {
                if self.clients[i].floated_by_layout {
                    self.clients[i].is_floating = false;
                    self.clients[i].floated_by_layout = false;
                    // `float_geom` is left intact: switching back to
                    // Floating restores each window to its last spot.
                }
            }
        }
    }
```

Call site — between the `maybe_apply_adaptive_layout` `if` block and `let mon = &self.monitors[mon_idx];` (~L106):

```rust
        if self.config.auto_layout && !self.monitors[mon_idx].is_overview {
            self.maybe_apply_adaptive_layout(mon_idx);
        }

        // Floating layout: auto-float / re-tile the current tag's
        // clients before `tiled` is built below (issue #1).
        self.reconcile_floating_layout(mon_idx);

        let mon = &self.monitors[mon_idx];
```

`smartgaps` guard (~L206) — add the layout check:

```rust
        if !is_overview
            && layout != crate::layout::LayoutId::Floating
            && self.config.smartgaps
            && tiled.len() <= 1
        {
```

`monly` guard (~L241):

```rust
        if !is_overview
            && layout != crate::layout::LayoutId::Floating
            && self.config.monly
            && tiled.len() == 1
        {
```

- [ ] **Step 6: Add `floated_by_layout` to `debug_dump` and the state snapshot**

`margo/src/state/debug_dump.rs` (~L43) — extend the per-client line:

```rust
            tracing::info!(
                "  client[{i}] mon={} tags={:#x} float={} fbl={} fs={} app_id={:?} title={:?} geom={}x{}+{}+{}",
                c.monitor,
                c.tags,
                c.is_floating,
                c.floated_by_layout,
                c.is_fullscreen,
                c.app_id,
                c.title,
                c.geom.width,
                c.geom.height,
                c.geom.x,
                c.geom.y,
            );
```

`margo/src/state/state_file.rs` per-client `json!` (~L180) — add after `"floating"`:

```rust
                    "floating": c.is_floating,
                    "floated_by_layout": c.floated_by_layout,
```

- [ ] **Step 7: Run to verify pass**

Run: `cargo test -p margo --lib tests::floating_layout`
Expected: PASS — all 6 tests.
Also run: `cargo test -p margo --lib tests::groups tests::arrange` (nothing regressed).

- [ ] **Step 8: Clippy the touched crate**

Run: `cargo clippy -p margo --all-targets` — expect no new warnings. (`reconcile_floating_layout` uses `get()` / `let else` / saturating math — no new `unwrap`/`expect`.)

- [ ] **Step 9: Commit**

```bash
git add margo/src/state/data.rs margo/src/state/groups.rs margo/src/state/arrange.rs \
        margo/src/state/debug_dump.rs margo/src/state/state_file.rs \
        margo/src/tests/floating_layout.rs margo/src/tests/mod.rs
git commit -m "$(cat <<'EOF'
feat(compositor): auto-float / re-tile for the Floating layout

reconcile_floating_layout() runs at the top of every arrange_monitor
pass: on a Floating tag it switches every governed tiled client to
floating (floated_by_layout marks it), dissolves tabbed groups, and
seeds a cascaded float_geom; on any other layout it re-tiles the
clients it previously floated. Windows the user floated by hand
(is_floating && !floated_by_layout) are untouched. The existing
float-apply pass writes float_geom -> geom unchanged.

Closes #1

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01QtWkq4eTWDHYM32zyD8vdZ
EOF
)"
```

---

## Task 4: Settings dropdown + mvisual thumbnail

**Files:**
- Modify: `mshell-crates/mshell-settings/src/tag_layout_settings.rs` (`LAYOUTS` ~L17)
- Modify: `mvisual/src/main.rs` (`render_layout` ~L140)

**Interfaces:**
- Consumes: `LayoutId::Floating` (Task 1).
- Produces: `tag_layout_settings::LAYOUTS` has `"floating"` as its 11th entry (matching `LayoutId` order); `mvisual`'s catalogue tile for `Floating` renders three cascade rects instead of an empty cell.

- [ ] **Step 1: Add `"floating"` to the Settings layout list**

`mshell-crates/mshell-settings/src/tag_layout_settings.rs` (~L17):

```rust
/// The 11 layouts offered per-tag, in `LayoutId` order. The last,
/// `floating`, is a stacking layout — the compositor bypasses the tiler
/// for it.
const LAYOUTS: &[&str] = &[
    "tile",
    "scroller",
    "grid",
    "monocle",
    "deck",
    "center_tile",
    "right_tile",
    "tgmix",
    "canvas",
    "dwindle",
    "floating",
];
```

(The DropDowns are `gtk::DropDown::from_strings(LAYOUTS)` and every index lookup is `LAYOUTS.get(i)` / `.position(...)`, so appending is safe.)

- [ ] **Step 2: Add the `mvisual` thumbnail special-case**

`mvisual/src/main.rs` in `render_layout`, right after the `if p.n_windows == 0 || w < 4 || h < 4 { return; }` guard (~L110), before the `ArrangeCtx` is built:

```rust
    // Floating: the real arrange() returns nothing (clients keep their
    // own float_geom). Draw a small cascade so the catalogue tile reads
    // as "stacking".
    if p.layout == LayoutId::Floating {
        let n = p.n_windows.min(4) as i32;
        let bw = (w as f64 * 0.55) as f64;
        let bh = (h as f64 * 0.55) as f64;
        let step_x = (w as f64 - bw) / (n.max(2) as f64);
        let step_y = (h as f64 - bh) / (n.max(2) as f64);
        for i in 0..n {
            let (r, g, b) = hsv_to_rgb(i as f64 / n.max(1) as f64, 0.42, 0.78);
            let alpha = if big { 0.92 } else { 0.85 };
            cr.set_source_rgba(r, g, b, alpha);
            cr.rectangle(i as f64 * step_x, i as f64 * step_y, bw, bh);
            let _ = cr.fill_preserve();
            cr.set_source_rgba(r * 0.5, g * 0.5, b * 0.5, 1.0);
            cr.set_line_width(if big { 1.5 } else { 1.0 });
            let _ = cr.stroke();
        }
        return;
    }
```

(`LayoutId` and `hsv_to_rgb` are already in scope in this file.)

- [ ] **Step 3: Build + test**

Run: `cargo test -p mvisual -p mshell-settings` and `cargo build -p mvisual -p mshell-settings`
Expected: PASS / clean build. The `mvisual` tests use `all_tileable().len()` via `%` so they adapt to 11 entries automatically.

- [ ] **Step 4: Commit**

```bash
git add mshell-crates/mshell-settings/src/tag_layout_settings.rs mvisual/src/main.rs
git commit -m "$(cat <<'EOF'
feat(settings,mvisual): surface the floating layout

Settings -> Tiling Layout gains a "floating" option; mvisual's catalogue
draws a cascade thumbnail for LayoutId::Floating (the real arrange()
returns nothing).

Refs #1

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01QtWkq4eTWDHYM32zyD8vdZ
EOF
)"
```

---

## Task 5: Layout-name list parity (`mctl` + snapshot `layouts[]`)

**Files:**
- Modify: `margo/src/dispatch/mod.rs` (`ALL_LAYOUTS` ~L54)
- Modify: `margo/src/state/state_file.rs` (`LAYOUT_NAMES` OnceLock ~L204)
- Modify: `mctl/src/actions.rs` (`LAYOUT_NAMES` ~L713; `setlayout` action `detail` ~L221)

**Interfaces:**
- Consumes: `LayoutId::Floating` (Task 1).
- Produces: `floating` is the 11th entry of the canonical layout list everywhere it is enumerated — `mctl status --json`'s `layouts[]`, `mctl layout 10` / `setlayoutindex 10`, and `mctl`'s `setlayout` help. `switch_layout` (cyclelayout) is unaffected — it uses the separate `config.circle_layouts` list, so `floating` stays out of the rotation as the spec requires.

- [ ] **Step 1: `ALL_LAYOUTS` in `margo/src/dispatch/mod.rs`**

Add `LayoutId::Floating,` after `LayoutId::Dwindle,` (~L64). Update the doc comment count if it names one.

- [ ] **Step 2: `LAYOUT_NAMES` OnceLock in `margo/src/state/state_file.rs`**

Add `crate::layout::LayoutId::Floating,` after `crate::layout::LayoutId::Dwindle,` (~L215). Update the "10-element" comment to "11-element".

- [ ] **Step 3: `mctl/src/actions.rs`**

`LAYOUT_NAMES` (~L713) — add `"floating",` after `"dwindle",`.

`setlayout` action `detail` (~L221) — change to:

```rust
        detail: "Names: tile, scroller, grid, monocle, deck, center_tile, \
                 right_tile, tgmix, canvas, dwindle, floating.",
```

- [ ] **Step 4: Test**

Run: `cargo test -p margo -p mctl`
Expected: PASS. (No test binds these list lengths; `arrange_dispatcher_matches_direct_call_all_layouts` was already handled in Task 1.)

- [ ] **Step 5: Commit**

```bash
git add margo/src/dispatch/mod.rs margo/src/state/state_file.rs mctl/src/actions.rs
git commit -m "$(cat <<'EOF'
feat(mctl): floating in the canonical layout-name list

setlayout floating / setlayoutindex 10 now resolve, and mctl status
--json's layouts[] lists it. cyclelayout still uses circle_layouts, so
floating stays out of the rotation.

Refs #1

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01QtWkq4eTWDHYM32zyD8vdZ
EOF
)"
```

---

## Task 6: Documentation + full CI gate

**Files:**
- Modify: `road_map.md` (~L61 layout list)
- Modify: `README.md` (~L111 layout list)
- Modify: `docs/config-conventions.md` (§6, ~L142)
- Check: `docs/protocol-comparison.md` (grep for a layout enumeration; edit only if one exists)

**Interfaces:** none (docs only).

- [ ] **Step 1: `road_map.md`**

At ~L61, change:

```
- 14 layout algorithms: tile, scroller, grid, monocle, deck, center / right / vertical variants, canvas, dwindle.
```

to:

```
- 14 layout algorithms: tile, scroller, grid, monocle, deck, center / right / vertical variants, canvas, dwindle — plus a **floating** (stacking) layout: no tiler, every window keeps its own geometry, cascaded on first show (issue #1).
```

- [ ] **Step 2: `README.md`**

At ~L111, change:

```
- **Layouts that remember.** Tile, scroller, grid, monocle, deck, dwindle, center / right mirrors and an overview. Each tag holds its own layout choice.
```

to:

```
- **Layouts that remember.** Tile, scroller, grid, monocle, deck, dwindle, center / right mirrors, an overview — and a **floating** layout for a classic stacking desktop. Each tag holds its own layout choice.
```

- [ ] **Step 3: `docs/config-conventions.md` §6**

In the "Per-tag tiling layout precedence" bullet (~L142), append a sentence:

```
  `floating` is a valid value for `taglayout` / `tagrule layout_name` /
  `default_layout` — it routes through `LayoutId::from_name` like any
  other, but bypasses the tiler entirely: the compositor auto-floats the
  tag's windows and cascades their placement (issue #1).
```

- [ ] **Step 4: Check `docs/protocol-comparison.md`**

Run: `grep -n "dwindle\|layout" docs/protocol-comparison.md`
If a row enumerates the layout catalogue or gives a layout count, add `floating` / bump it. If (as of writing) the only match is the protocol-globals count at ~L62, make no change.

- [ ] **Step 5: Full CI gate**

Run: `just check`
Expected: PASS — `cargo +1.95.0 fmt --all` clean, `cargo clippy --all-targets -D warnings` clean, `./scripts/panic-ratchet.sh` at baseline, `./scripts/design-lint.sh` clean, `cargo test` green.

If `panic-ratchet` reports the count rose: the new `unreachable!` in `snapshot_tests.rs` is inside a `#[cfg(test)] mod snapshot_tests;` gated file and must NOT count — re-check the file is on `gated_test_files`. `reconcile_floating_layout` adds no panic-prone calls. Do not raise the baseline.

- [ ] **Step 6: Commit**

```bash
git add road_map.md README.md docs/config-conventions.md docs/protocol-comparison.md
git commit -m "$(cat <<'EOF'
docs: floating layout

Refs #1

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01QtWkq4eTWDHYM32zyD8vdZ
EOF
)"
```

- [ ] **Step 7: Push**

```bash
git push origin main
```

---

## On-device verification (human, after push + `just margo && just shell && just cli` + re-login)

- `mctl dispatch setlayout floating` on a tag with 3 tiled windows → all three float, cascaded from the top-left; bar layout pill shows `F`.
- `mctl dispatch setlayout tile` → the same three re-tile.
- `togglefloating` one window by hand, `setlayout floating`, `setlayout tile` → the hand-floated one stays floating, the rest re-tile.
- `taglayout = 4, floating` in `config.conf` → `mctl reload` → tag 4 is a floating tag.
- Settings → Tiling Layout → pick "floating" for a tag → takes effect live.
- A fullscreen window on a floating tag stays fullscreen; a scratchpad stays a scratchpad.
- Open the overview on a floating tag → grid thumbnails as normal; close it → windows return to their cascaded float positions.

---

## Self-Review

**1. Spec coverage**

| Spec section | Task |
|---|---|
| §1 `LayoutId::Floating` (`name`/`symbol`/`from_name`/`from_symbol`/`all_tileable`; not in cyclelayout) | 1 (+ Task 5 notes cyclelayout untouched — uses `circle_layouts`) |
| §1 "add to `all()`" | N/A — `margo-layouts` has **no** `LayoutId::all()`; only `all_tileable()`. Corrected here. |
| §2 `floating()` + dispatch arm | 1 |
| §3 `floated_by_layout` field (+ default, + serialized in snapshot/`debug_dump`) | 3 (spec says `ClientData` — the struct is **`MargoClient`**; corrected) |
| §4 `reconcile_floating_layout` + `place_floating` cascade + call site | 2 (pure helper) + 3 (method + wiring) |
| §5 `smartgaps` / `monly` guards | 3 |
| §6 no `margo-config` change; `mvisual` special-case; `tag_layout_settings` `LAYOUTS += "floating"` | 4 (+ Task 5 for the `mctl` / snapshot lists, which the spec's "Files touched" omitted but its intent — "if they enumerate layout names" — covers) |
| §7 no dedicated keybind | honoured — no task adds one |
| Edge cases: fullscreen / scratchpad / overlay / minimized / killing / overview / XWayland-OR / tabbed group / multi-monitor / moved-to-tag / auto_layout / reload / preserved `float_geom` | 3 — the `governed` filter + `is_overview` early-return + group dissolve + "leave `float_geom` intact" all encode these; `hand_floated_client_survives_switch_to_tiling`, `floating_layout_dissolves_tabbed_groups`, `reconcile_is_idempotent` are the tests |
| Testing: `margo-layouts` `floating()` empty; snapshot scenario; `reconcile` unit test | 1 (`floating_arranges_nothing…` + `floating_three_windows_arranges_nothing` snapshot), 2 (cascade math), 3 (`floating_layout.rs` integration suite via the real fixture — richer than the spec's sketch, which predated confirming `margo/src/tests/fixture.rs` exists) |
| Docs: road_map / README / protocol-comparison / config-conventions §6 / mctl help | 6 (+ mctl help in Task 5) |
| Commit `Closes #1` | Task 3 (feature-complete commit); other commits `Refs #1` |

**2. Placeholder scan** — no `TBD` / `TODO` / "handle edge cases" / "similar to Task N". Every code step has literal code. The one conditional step (Task 6 Step 4, protocol-comparison) has an explicit grep + decision rule.

**3. Type consistency**
- `place_floating_cascade(work_area: Rect, preferred: Option<(i32,i32)>, min: (i32,i32), max: (i32,i32), cascade_index: usize) -> Rect` — defined identically in Task 2 Step 3 and called identically in Task 3 Step 5.
- `MargoClient.floated_by_layout: bool` — field name identical in data.rs, groups.rs (not referenced), arrange.rs, debug_dump.rs, state_file.rs, and all Task 3 tests.
- `dissolve_group(&mut self, gid: u32)` — defined in groups.rs, called in `reconcile_floating_layout`.
- `reconcile_floating_layout(&mut self, mon_idx: usize)` — defined + called once, both in arrange.rs.
- `LayoutId::Floating` — one variant, referenced by that exact path everywhere (`crate::layout::LayoutId::Floating` inside `margo`, `LayoutId::Floating` inside `margo-layouts` tests / `mvisual`).
- Cascade constants `INSET = 24`, `STEP = 32` live only in `place_floating_cascade`; the compositor never re-derives them.
