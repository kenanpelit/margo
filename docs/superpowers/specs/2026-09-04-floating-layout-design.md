# Floating Layout — Design

**Goal:** Add a `floating` layout to margo alongside the existing tiling
layouts, giving a traditional stacking/floating desktop (GNOME / Plasma /
XFCE style) selectable per-tag or globally. Closes
[#1](https://github.com/kenanpelit/margo/issues/1).

**Non-goals:** No new window-management primitives. Floating windows,
`float_geom`, interactive move/resize, the float>tile>overlay z-band, and
per-tag layout state all already exist — this feature composes them.

## Background — what already exists

- `ClientData.is_floating: bool`, `float_geom: Rect`. `is_tiled()` is
  `!is_floating`. `arrange_monitor` builds `tiled` from
  `visible_in_pass(c) && c.is_tiled()` and passes it to the layout
  algorithm; floating clients keep `float_geom` and are applied
  separately (`arrange.rs` ~L536–561).
- `LayoutId` (`margo-layouts/src/lib.rs`): `Tile Scroller Grid Monocle
  Deck CenterTile RightTile TgMix Canvas Dwindle Overview`. Each has
  `name()` / `symbol()` / `from_name()` / `from_symbol()` / `all()` /
  `all_tileable()`.
- `Canvas` is the precedent for "layout that produces no tiled geometry":
  `canvas(_ctx) -> vec![]` (`algorithms.rs` ~L476) — clients keep
  `canvas_geom`, positioned by pan/zoom, not the arrange path.
- Per-tag layout: `mon.pertag.ltidxs[curtag]`, sticky
  `user_picked_layout[curtag]`. `set_layout(name)` →
  `LayoutId::from_name(name)`. Config `default_layout`, `taglayout = <tag>,
  <name>`, `tagrule … layout_name = …`; precedence `taglayout > tagrule
  layout_name > default_layout` (config-conventions §6), re-applied in
  `reload_config`.
- `maybe_apply_adaptive_layout` (auto-layout heuristic) only fires when
  `!user_picked_layout[curtag]`; picks Tile/Scroller/Grid/Monocle by
  window count + aspect — never an explicit-only layout.
- Layout snapshot tests: `margo/src/layout/snapshot_tests.rs` (insta text
  snapshots, `INSTA_UPDATE=always` writes baselines) +
  `margo-layouts/tests/layouts.rs`.
- `mshell-settings/src/tag_layout_settings.rs` — `LAYOUTS: &[&str]` (the
  10 tile-able names, `LayoutId` order), index-based DropDowns for
  `default_layout` + per-tag. `mvisual` renders the catalogue from
  `LayoutId::all_tileable()`.

## Design

### 1. `LayoutId::Floating`

Add the variant. `name() = "floating"`, `symbol() = "F"` (unused, free).
Add to `from_name` / `from_symbol` arrays, `all()`, and
`all_tileable()` (it *is* user-selectable; the name is historical — it
means "offered in catalogues/pickers", not "produces a tiling"). Do
**not** add to any `cyclelayout` rotation (per the approved decision —
explicit selection only, like `Overview`).

`from_name("floating")` returning `Some` is all that's needed for
`mctl dispatch setlayout floating`, `taglayout = 3, floating`, and
`default_layout = floating` to work — those paths already route through
`from_name`.

### 2. `floating()` algorithm

```rust
// margo-layouts/src/algorithms.rs
/// Floating layout — the tiler produces nothing; every client keeps its
/// own `float_geom` (the compositor auto-floats tiled clients and
/// cascades placement in `reconcile_floating_layout`).
pub fn floating(_ctx: &ArrangeCtx) -> ArrangeResult {
    vec![]
}
```

Dispatch arm in `arrange()`: `LayoutId::Floating => floating(ctx)`.

### 3. `ClientData.floated_by_layout: bool`

New field, default `false`. Distinguishes a client the floating layout
auto-floated from one the user explicitly floated with `togglefloating`
or an `isfloating:1` rule.

| how it became floating | `is_floating` | `floated_by_layout` | on switch to a tiling layout |
|---|---|---|---|
| floating layout auto-floated it | `true` | `true` | re-tiled (`is_floating=false`, flag cleared) |
| user `togglefloating` / rule | `true` | `false` | stays floating (unchanged behaviour) |

Serialized in the state snapshot / `debug_dump` next to `is_floating`
(so `mctl get` / debugging reflect it). No IPC changes.

### 4. `reconcile_floating_layout(mon_idx)`

New `MargoState` method, called from `arrange_monitor` immediately after
`layout` is resolved (`arrange.rs` ~L119, after
`maybe_apply_adaptive_layout`), before `tiled` is built. `arrange_monitor`
already re-runs on every relevant change (layout switch, tag switch,
window map/unmap, monitor move, `reload_config`), so this is the single
reconcile point — no edits to `set_layout`, `seed_taglayouts`, the map
path, or tag-move.

```
fn reconcile_floating_layout(&mut self, mon_idx: usize):
    curtag       = monitors[mon_idx].pertag.curtag
    is_floating_layout = monitors[mon_idx].pertag.ltidxs[curtag] == Floating
    work_area    = <monitor work area, same as arrange_monitor computes>

    for each client c on this monitor visible on the current tagset,
        excluding: fullscreen, scratchpad, overlay, minimized, killing,
        overview-only, group non-active members:

        if is_floating_layout:
            if !c.is_floating:
                c.is_floating        = true
                c.floated_by_layout  = true
                if c.float_geom.width == 0:
                    c.float_geom = place_floating(c, work_area, &mut cascade_cursor)
                c.geom = c.float_geom            // apply immediately
        else:
            if c.floated_by_layout:
                c.is_floating       = false
                c.floated_by_layout = false
                // c re-enters `tiled`; the active tiling layout sizes it
```

`place_floating(c, wa, cursor)`:
1. `size` — the client's requested/committed toplevel size if it has one
   and it fits `wa`; else `(0.6*wa.w, 0.6*wa.h)`, clamped to
   `[c.min_*, c.max_*]`.
2. `pos` — `cursor` (starts at `wa` top-left + a fixed inset). Advance
   `cursor` by `(+32, +32)` after each placement; when
   `cursor.x + size.w > wa.right` **or** `cursor.y + size.h > wa.bottom`,
   reset `cursor` to the inset (wrap the cascade).
3. Clamp the final rect fully inside `wa`.

`cascade_cursor` is a local, recomputed each reconcile from the count of
already-placed floating clients on the tag — no new persistent state, and
stable across arrange passes (an already-placed client keeps its
`float_geom`; only newly-auto-floated ones consume cursor slots).

### 5. `arrange_monitor` — no special-casing needed

Once `reconcile_floating_layout` has run, on a floating tag every client
is `is_floating`, so `c.is_tiled()` is false, `tiled` comes out empty,
`floating(ctx)` returns `vec![]`, and the existing float-apply pass
(`arrange.rs` ~L536) writes each `float_geom` to `geom`. Guard the
`smartgaps` / `monly` blocks with `layout != Floating` (they're
meaningless with an empty `tiled` but skipping the work is tidy).

### 6. Config surface

Nothing in `margo-config` — `default_layout` / `taglayout` are free-form
strings routed through `LayoutId::from_name`, an unknown name is silently
ignored, and `validator.rs` only marks `taglayout` as csv-shaped (no
value whitelist). Precedence is unchanged and inherited.

`mvisual` builds an `ArrangeCtx` and calls the real `arrange()` — for
`Floating` that returns `vec![]`, so the thumbnail renders empty. Give
`mvisual` a per-layout special-case: when `layout == Floating`, draw
three cascade rects (60% of the cell, +offset each) so the catalogue
tile reads as "stacking". `mshell-settings/tag_layout_settings.rs` —
append `"floating"` to `LAYOUTS` (11th entry; `LayoutId` order → after
`dwindle`).

### 7. Keybind

No dedicated `togglefloatinglayout` dispatch action. `setlayout floating`
covers it; a user binds `Super+Shift+F -> setlayout floating` in
`binds.conf` if they want a key. (Revisit only if requested.)

## Edge cases

| case | behaviour |
|---|---|
| fullscreen window on a floating tag | untouched — the fullscreen override in `arrange.rs` sits above `is_floating` and already wins |
| scratchpad / named-scratchpad / overlay | excluded from the reconcile; unchanged |
| minimized / killing / tag-switching clients | excluded (not visible in pass) |
| overview open | overview forces its own Grid path (`arrange.rs` L116) — reconcile is a no-op there; on close, arrange re-runs and reconciles |
| XWayland override-redirect | already `is_floating`; reconcile leaves it (`floated_by_layout` stays false) |
| tabbed group on a floating tag | dissolve the group on entering floating (call the existing `state::groups` ungroup helper for members on that tag) — a group is a tiling construct |
| multi-monitor, per-tag | `pertag.ltidxs` is already per-monitor-per-tag; reconcile is per-monitor |
| window moved *to* a floating tag | next `arrange_monitor` for that monitor reconciles it |
| `auto_layout` on | heuristic never picks `Floating`; a tag left on `Floating` by the user has `user_picked_layout=true` so the heuristic won't override it |
| `mctl reload` with `taglayout = N, floating` | `reload_config` re-seeds taglayouts (existing path), next arrange reconciles — no new reload code |
| switch floating → tiling with a window the user *also* resized | `float_geom` is preserved (not cleared); if they switch back to floating it returns to that geom |

## Testing

- `margo-layouts/tests/layouts.rs` — `floating()` returns `vec![]` for
  empty / 1 / N clients.
- `margo/src/layout/snapshot_tests.rs` — a `floating` scenario:
  3 clients, assert `format_arranged` shows the empty arrange result;
  a second snapshot after a simulated reconcile (helper builds clients
  with `float_geom` at cascade offsets) asserting the cascade math
  (0,0-inset / +32,+32 / +64,+64), and a wrap case.
- New unit test for `reconcile_floating_layout`: build a `MargoState`
  fixture with 3 tiled clients on tag 1, set `ltidxs[1] = Floating`,
  call reconcile → all `is_floating && floated_by_layout`, cascade
  positions correct; set `ltidxs[1] = Tile`, reconcile → the 3 re-tile,
  a 4th client the test marked `is_floating` + `floated_by_layout=false`
  stays floating.
- `cargo test -p margo -p margo-layouts` + `just check` (the full CI
  gate). `INSTA_UPDATE=always` once to write the new `.snap` baselines,
  review under `git diff -- margo/src/layout/snapshots/`.

## Docs

- `road_map.md` — add `floating` to the layouts list / compositor
  highlights.
- `docs/protocol-comparison.md` — layout count / capability row if it
  enumerates layouts.
- `README.md` — the "Tile, Deck, …" layout list.
- `docs/config-conventions.md` §6 — note `floating` is a valid
  `taglayout` / `default_layout` value that bypasses the tiler.
- `mctl` help / completions if they enumerate layout names.

## Files touched

| file | change |
|---|---|
| `margo-layouts/src/lib.rs` | `LayoutId::Floating` + `name`/`symbol`/`from_name`/`from_symbol`/`all`/`all_tileable` |
| `margo-layouts/src/algorithms.rs` | `floating()` fn + `arrange()` dispatch arm |
| `margo-layouts/tests/layouts.rs` | `floating()` returns empty |
| `margo/src/state/data.rs` | `ClientData.floated_by_layout: bool` (+ default, + `debug_dump`) |
| `margo/src/state/arrange.rs` | `reconcile_floating_layout` method; call it in `arrange_monitor`; guard `smartgaps`/`monly` |
| `margo/src/state/groups.rs` | dissolve groups on a tag entering `Floating` (reuse existing ungroup helper) |
| `margo/src/layout/snapshot_tests.rs` + `snapshots/` | floating scenarios + baselines |
| `mshell-crates/mshell-settings/src/tag_layout_settings.rs` | `"floating"` in `LAYOUTS` |
| `mvisual/src/main.rs` | `Floating` thumbnail special-case (3 cascade rects) |
| `road_map.md`, `README.md`, `docs/protocol-comparison.md`, `docs/config-conventions.md` | layout list / note |

## Open risks

- **`place_floating` size source.** The client's committed toplevel size
  may not be known at the first reconcile (before the initial commit).
  Fallback to 60% work-area is safe; a later arrange (post-commit) will
  *not* re-place (float_geom already set), so a client that wanted a
  specific size but reconciled early keeps the 60% box. Acceptable for
  v1; a follow-up could re-place once on the first real size commit if
  `floated_by_layout && float_geom was the 60% fallback`.
- **Cascade determinism across monitors / tag re-entry.** Recomputing
  `cascade_cursor` from the placed-count each pass keeps it stable, but
  verify with the snapshot test that re-running arrange twice is a
  no-op (idempotent reconcile).
