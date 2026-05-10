# Infinite Spatial Overview — design

> **Status:** Foundation in flight (Phase 3 commit 1/3).
> **Goal:** Replace the legacy single-Grid overview with a 2D
> pan-zoomable canvas that holds every tag's clients in their own
> world-space cluster — Hyprland/Mango cinematic continuity, niri
> keyboard-first navigation, paperwm/aerospace spatial memory.
> Hot corner, 4-finger swipe, alt+Tab MRU cycle, `mctl` triggers all
> share the same back-end; Grid overview becomes the opt-in
> `overview_mode = grid` fallback for users who prefer it.

## Why this exists

Margo's port-zero overview was *"throw every client into one Grid
over the work area"*. Quick to ship, but it lost the spatial-memory
cue that makes overview useful in the first place — a window's
overview position had no relation to where it normally lived. The
0.1.8 per-tag-thumbnail attempt fixed that on small scales but
burned pixel budget on empty tags, hard-coded a 3×3 grid, and
introduced three drift-prone hit-test helpers.

Phase 3's mandate is to put margo on a **third axis** the existing
compositors don't compete on:

| Project | Overview model | Strength | Weakness |
|---|---|---|---|
| Hyprland | Cinematic zoom-out of all workspaces | Visually rich | Heavy on GPU, fragile plugin ABI |
| niri | Scrolling column overview | Linear, deterministic | Tied to scrolling-only layout, no spatial freedom |
| Mango/mango-ext | Single Grid zoom-out | Spatial continuity within one workspace | No multi-tag awareness |
| Aerospace/paperwm | Tile-graph navigation | Keyboard-first, predictable | macOS-only / X11-only respectively |

The combination margo can claim — and no other compositor in the
audit has — is **infinite 2D spatial canvas + tag-based dwm
workflow + Rust/smithay/rest-of-margo's existing animation engine**:

* **Infinite** because every tag occupies a fixed slot in world
  coordinates; pan + zoom let the user navigate the whole world,
  not just the active tag. The "9 tags" upper bound is a config
  semantic, not a layout constraint.
* **Spatial** because each tag's clients keep their *configured*
  layout (Tile / Scroller / Grid / Canvas) inside their slot,
  zoomed but otherwise unchanged. A scroller tag still looks like
  a scroller in overview; a grid tag still looks like a grid.
* **Inertial** because pan and zoom carry momentum (paperwm/Aerospace
  pattern), decayed with the same spring engine margo already uses
  for window animations — no new physics primitive.

This document fixes the architecture decisions before the
implementation lands; the actual code arrives across three commits
(this one is design + foundation; the next two are state +
arrange-side and input + tick respectively).

## Mental model

```
            world coordinates (logical pixels)
            ┌──────────────────────────────────┐
            │  ┌────────┬────────┬────────┐    │
            │  │ tag 1  │ tag 2  │ tag 3  │    │   each tag area
            │  │ Tile   │ Scrol  │ Grid   │    │   = monitor size
            │  ├────────┼────────┼────────┤    │
            │  │ tag 4  │ tag 5  │ tag 6  │    │   + outer padding
            │  │ Mono   │ Canvas │ Deck   │    │
            │  ├────────┼────────┼────────┤    │
            │  │ tag 7  │ tag 8  │ tag 9  │    │
            │  │ TgMix  │ VTile  │ Dwind  │    │
            │  └────────┴────────┴────────┘    │
            │              ▲                   │
            │              │ camera viewport   │
            │           ┌─────┐                │
            │           │     │  ← what the    │
            │           │     │    user sees   │
            │           └─────┘                │
            │                                  │
            └──────────────────────────────────┘
```

Three coordinate spaces:

1. **World space** — every tag has a fixed `(tag_world_x, tag_world_y)`
   anchor; each client's `geom` within a tag is the same layout it
   would have if the tag were active full-screen.
2. **Camera viewport** — `(cam_x, cam_y, zoom)` defines what subset
   of world space is currently rendered into the monitor's work area.
3. **Screen space** — physical pixels post-DRM transform; smithay
   handles this last leg the same way it does for any element.

`screen = (world - cam) * zoom + work_area_origin`
`world  = (screen - work_area_origin) / zoom + cam`

These two formulas are the entire spatial pipeline. Camera is
the only thing that changes per frame; everything else is
recomputed cheaply from it.

## Camera

```rust
pub struct SpatialCamera {
    /// World-space coordinates of the camera's centre (logical
    /// pixels). Mouse drag and keyboard pan move this directly.
    pub x: f64,
    pub y: f64,
    /// Zoom factor — 1.0 means "one logical pixel of the work area
    /// shows one logical pixel of the world". Smaller = more world
    /// visible. Clamped at `[overview_zoom_min, overview_zoom_max]`.
    pub zoom: f64,
    /// Target the camera is interpolating toward. The spring engine
    /// drives `x/y/zoom` toward these every frame; setting all three
    /// equal disables motion (steady state).
    pub target_x: f64,
    pub target_y: f64,
    pub target_zoom: f64,
    /// Momentum components — set on pan/zoom *release*, decayed by
    /// friction every tick. Non-zero values keep `target_*` moving
    /// even after the user lifts the mouse / keyboard, producing
    /// the inertial slide niri's gesture path gives for free but
    /// margo previously didn't have for keyboard-driven motion.
    pub vx: f64,
    pub vy: f64,
    pub vzoom: f64,
}
```

Spring constants: damping ratio 1.0 (critically damped — no
overshoot), stiffness 220 (`overview_transition_ms` × 1.2). Same
spring math margo already uses for window move animations; one
new instance per-camera-axis, decaying separately.

Friction (free-flight after release): `0.92` per frame at 60 Hz
(`vx *= 0.92` etc.) — matches paperwm's feel. At 144 Hz the same
constant gives a slightly snappier decay, which is fine.

Zoom bounds: `overview_zoom_min = 0.1` (entire world visible on a
1080p monitor — every tag fits in a 360 px square), `overview_zoom_max
= 1.5` (zoomed-in beyond 1:1 for accessibility). Default open zoom:
`overview_zoom` config (existing knob, currently 0.5 default,
re-interpreted as "what zoom should `open_overview_spatial` land at").

## World layout

Each tag occupies a `(monitor_width + 2 × tag_padding) × (monitor_height
+ 2 × tag_padding)` slot. Slots arrange in a 3×3 grid by default:

```
tag 1 anchor = (0, 0)
tag 2 anchor = (slot_w, 0)
tag 3 anchor = (2 × slot_w, 0)
tag 4 anchor = (0, slot_h)
...
tag 9 anchor = (2 × slot_w, 2 × slot_h)
```

`tag_padding` defaults to 64 logical px — large enough to *visually*
separate tags at zoom 0.5, small enough not to dwarf the content at
zoom 1.0. Configurable as `spatial_tag_padding`.

Inside its slot, a tag runs its **configured layout** (`Pertag::
ltidxs[tag]`) at full size; the slot rect *is* the layout's work
area. Spatial mode doesn't replace the layout engine — it pans /
zooms the camera over its output.

## Triggers (unchanged from Faz A)

* `bind = alt,Tab,overview_focus_next` — open + cycle to next
  MRU client (auto-pans the camera to centre that client).
* `bind = alt+shift,Tab,overview_focus_prev` — open + cycle previous.
* `bind = alt,Return,overview_activate` — close, activate selection.
* `hot_corner_top_left = toggle_overview` — open / close.
* `gesture = swipe, 4, up, toggle_overview` — same.
* `bind = super,grave,toggle_overview` — keybind alias.

Spatial mode reuses every trigger; the difference is what
`toggle_overview` and `overview_focus_*` do internally.

## Pan & zoom

| Input | Action | Notes |
|---|---|---|
| Mouse left-drag empty space | Pan camera | `vx/vy` track drag velocity; release → momentum |
| Mouse scroll | Zoom toward cursor | `zoom *= 1.1` per click, biased so the world point under the cursor stays put |
| Touchpad 2-finger scroll | Pan camera | Same delta budget as drag, same momentum |
| Touchpad pinch | Zoom toward gesture centroid | Future — gesture handler exists, math same as scroll-zoom |
| Keyboard `super+arrow` in overview | Pan one screen | Existing dispatch surface; binds to new `overview_pan_left/right/up/down` actions |
| Keyboard `super+= / super+-` in overview | Zoom in/out | Same — `overview_zoom_in/out` |
| Click on client thumbnail | Activate + close | Reuses existing overview-click path |

## Activation (`overview_focus_next` in spatial mode)

* If overview is closed, open at default zoom centred on the
  active tag's centre.
* Walk the visible-client list (MRU-sorted) one step in `dir`.
* `is_overview_hovered` flag flips to the new client (the border
  pipeline catches up via `border::refresh`).
* **Camera animates** toward the new client's centre, target zoom
  unchanged (so cycling within a tag doesn't yank zoom). Spring
  engine drives the motion; default 220 ms to settle.
* No `focus_surface` (the focus-crossfade bug from 0.1.8 stays
  fixed); `overview_activate` runs the focus path once on commit.

## Phase 3 implementation slices

This document covers the foundation only:

* **Commit 1 (this design doc + module skeleton):** `OverviewMode`
  enum, `SpatialCamera` struct, math helpers (`world_to_screen`,
  `screen_to_world`, `tag_anchor`, `client_world_rect`), config
  field, no behaviour change yet.
* **Commit 2:** `MargoState::spatial: SpatialCamera` field,
  `arrange_monitor` branches into the spatial path when
  `is_overview && overview_mode == Spatial`, render output uses
  the transformed client geoms unchanged (camera math lives
  entirely in arrange).
* **Commit 3:** Input handlers (mouse pan, scroll zoom, keyboard
  pan/zoom dispatches), frame-tick momentum decay, `overview_focus_*`
  spatial-aware camera animations.

After commit 3 the spatial overview is fully usable; polish
(semantic grouping by app-id, minimap, app-cluster heatmap) lives
in Phase 3.5+.

## What's out of scope

* **Multi-monitor world topology.** v1 keeps one world per-monitor
  (tags-on-this-monitor laid out in 3×3 inside that monitor's
  world space). Cross-monitor canvas is a real feature but
  belongs in Phase 3.5 once the single-monitor case is solid.
* **Persistent camera state across overview open/close.** Each
  open re-centres on the active tag at default zoom. Persistent
  camera ("resume where I left off") is an easy follow-up but not
  required for the core paradigm to feel right.
* **Live updates of client thumbnails while spatial is open.**
  Clients in spatial overview are arranged once on `open` and
  re-arranged when the layout actually changes (a real config
  reload, a mode switch, etc.) — not on every frame. The 60 Hz
  recompute would burn CPU for no visible benefit at low zoom.

## Risk + mitigation

| Risk | Mitigation |
|---|---|
| Pan/zoom math drift between input handler and arrange | All three coordinate transforms live in one module (`spatial_overview.rs`); arrange + render + input all call the same `world_to_screen` / `screen_to_world` helpers. |
| Inertial momentum runs forever (forgot to clamp velocity) | Velocity threshold (`|vx| + |vy| < 0.5 px/frame`) sets velocity to 0; same for zoom. |
| Existing animation tick gets re-entered from camera tick | Camera ticks in the same frame-clock callback that ticks window animations — one source of truth (the `tick_animations` site) so re-entry doesn't happen. |
| User flips `overview_mode` mid-cycle | Mode is read at the top of `arrange_monitor`; mid-cycle flip just takes effect on the next arrange. The cycle's hovered client follows. |

## Testable invariants

For Commit 1 (this PR):

* `world_to_screen(screen_to_world(p, cam), cam) == p` (round-trip
  identity inside camera math).
* `tag_anchor(N)` returns the same `(x, y)` for every monitor
  (world layout is monitor-independent on the inside; per-monitor
  scaling comes from the slot size, not the anchor).
* `SpatialCamera::default()` is positioned at the centre of the
  3×3 world with zoom 0.5 — opens overview at "everything visible"
  without any further setup.

Three unit tests in `margo/src/spatial_overview.rs`. Commit 2 adds
arrange-side invariants, commit 3 adds input/tick invariants.
