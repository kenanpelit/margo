# Changelog

All notable changes to **margo** are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- **Alt-release auto-commit now actually fires on the Alt-release
  event.** Previous attempt read `modifiers` from the release-event
  filter callback and checked whether the snapshot still overlapped.
  Problem: xkbcommon updates its modifier state *after* the filter
  runs, so on the `Alt_L` release event the callback still sees
  `modifiers.alt = true`. The intersection check never went empty and
  overview stayed open until a second alt+Tab press happened. New
  approach reads the *released keysym* (`handle.raw_syms()`) and maps
  it to its `margo_config::Modifiers` bit directly, subtracts that
  bit from the pending-cycle snapshot, and commits when the snapshot
  empties. Works regardless of which order the user releases
  modifiers — Alt+Shift+Tab still needs both keys released, but in
  either order.

- **Alt+Tab opening overview now auto-commits on Alt release.** When
  the user pressed Alt+Tab with overview closed, `overview_focus_step`
  called `open_overview()` first — and `open_overview` reset
  `overview_cycle_pending` + `overview_cycle_modifier_mask` to default
  "fresh open" values. That clobbered the snapshot the input handler
  had just set milliseconds earlier in the keybind-match path. So
  the Alt-release branch read `cycle_pending = false`, did nothing,
  and overview stayed open after the user let go of Alt. Fix: drop
  the defensive reset from `open_overview`. `close_overview` and
  `overview_activate` already handle the flag's lifetime on the way
  out; opens reached through `overview_focus_step` carry the
  freshly-set snapshot through to the release branch.

- **Alt+Tab first press now jumps to the *previously*-used window,
  not back to the focused one.** The cycle anchor was
  `is_overview_hovered.position()` only — which is `None` on the
  very first press while overview is freshly open. The `None`
  fallback landed at index 0, and in MRU mode index 0 is the
  currently-focused window (most-recent entry in `focus_history`).
  So the first Tab tap looked like a no-op: highlight didn't
  move, then the *second* press moved one step. Standard alt+Tab
  on every other DE (i3 / sway / Hypr / niri / GNOME) is "one tap
  = jump to the other window."
  Fix: when there's no in-progress hover, anchor on the focused
  client's *position in the list*. `dir = +1` then moves to
  index 1, which in MRU is the previously-used window. Same fix
  benefits `tag` / `mixed` modes: the user's first cycle step
  moves away from where they already are, not onto it.

### Added

- **`overview_cycle_order` config — let the user pick the alt+Tab
  walk order.** New three-valued config key on top of the existing
  MRU-only behaviour, all wired through one match in
  `overview_visible_clients`:
  * `mru` (default, preserves 0.1.9 behaviour) — `focus_history`
    first (most-recent first), then any remaining visible clients
    in clients-vec order. The Win/GNOME/Hypr muscle memory.
  * `tag` — strict tag-1-to-9 order, clients-vec inside each tag.
    Spatial-memory model: tag 1's windows always come first.
  * `mixed` — current tag's clients in MRU order, remaining tags
    in strict tag order. The "MRU where you live, tag elsewhere"
    hybrid.

  Implementation reuses two helpers (`push_mru` with optional tag
  filter, `push_tag_order` with optional skip mask) — adding any
  future mode is now one `match` arm. Unknown / typo'd values fall
  back to `mru` with a `tracing::warn!`.

## [0.1.9] – 2026-05-10

Overview reborn. The whole release is one focused theme: nail the
zoom-out-grid UX so it beats Hyprland, niri, and the upstream
mango-ext on the metric the user actually feels — keyboard latency,
spatial continuity, modifier muscle memory. Three iterations to get
there (Phase 3 spatial reverted, fixed 3×3 thumbnails reverted,
mango-ext `overview(m){grid(m);}` shipped); then cinematic dim +
thicker selection border + MRU cycle + alt-release auto-commit on
top of the same single-arrange path. End state is one of the
shortest overview implementations in any Wayland compositor and the
most responsive.



### Added

- **Alt+Tab muscle-memory commit — release modifier to confirm.**
  Holding Alt and tapping Tab to walk thumbnails was already
  smooth, but the user still had to press Enter (`alt+Return →
  overview_activate`) to commit the pick. Now, releasing Alt (or
  whichever modifier the binding uses) is enough — overview
  closes onto the highlighted thumbnail and focus moves there.
  Matches the Win/GNOME/Hypr "hold modifier, tap to cycle, let
  go to confirm" muscle memory the user expects from alt+Tab
  outside this compositor.
  * Implemented as a modifier snapshot taken when an
    `overview_focus_next/prev` keybind fires, plus a release-
    branch in the keyboard handler that watches for the snapshot
    set going to zero (every snapshotted modifier released).
  * Works for any modifier — `super,Tab,overview_focus_next`
    binding would commit on Super release.
  * `alt+shift+Tab` walks backwards: releasing Shift alone won't
    commit (Alt is still held); releasing both Alt and Shift
    will.
  * Two new `MargoState` fields: `overview_cycle_pending` and
    `overview_cycle_modifier_mask`. Cleared by `open_overview`,
    `close_overview`, and `overview_activate`. `alt+Return` still
    works as the explicit commit path.

- **Overview cinematic selection — dim + thicker border on the
  pick.** Two new config keys, both clamped, both default-on:
  * `overview_selected_border_multiplier` (default `1.6`, range
    `[1.0, 4.0]`) — multiplies the normal border width on the
    keyboard / hover-selected thumbnail. Border already paints
    `focuscolor` on selection; the multiplier makes the pick read
    even at small thumbnail sizes without a separate render path.
  * `overview_dim_alpha` (default `0.6`, range `[0.1, 1.0]`) —
    alpha multiplier applied to **non-selected** thumbnails while
    overview is open. The selected thumbnail stays at full
    opacity. Result: a spotlight on the focuscolor-bordered
    selection, the cinematic feel niri/Hypr ship by default. The
    multiplier folds into the existing alpha parameter on
    `render_elements_from_surface_tree` (Wayland live surface) and
    the X11 `AsRenderElements` path, so no new render element
    type is needed — one f32 per window per frame.
  Set either to `1.0` to opt out individually.

- **Overview alt+Tab now MRU-ordered.** `overview_visible_clients`
  walks the per-monitor `focus_history` first (most-recent first),
  then appends any remaining visible clients in clients-vec order
  for completeness. Result: `alt+Tab` steps through windows in the
  order the user last touched them — matches every other alt+Tab
  in existence (i3, sway, Hypr, niri, GNOME). Previous behaviour
  cycled in map-then-rearrange order, which felt random when the
  user switched between long-running windows.

### Fixed

- **Overview alt+Tab border lit up instantly.** The cycle path
  (`overview_focus_step`) was running a snap-no-slide
  `arrange_monitor` after every Tab press to push the new
  selection through the layout pipeline. Even at 1 ms duration,
  the arrange-time `border::refresh` ran against per-client move
  state in flux and the focuscolor border landed one frame after
  the user expected. Removed the arrange call entirely — Mango-ext
  overview is a Grid layout, every cell stays put across a cycle,
  and only the *selected* state changes. The cycle now flips
  `is_overview_hovered`, calls `border::refresh`, requests a
  repaint — single render to focuscolor, no animation gate, no
  recompute. ("border anında diğer pencerede değil" → fixed.)

### Changed

- **Overview switched from fixed 3×3 per-tag thumbnails to mango-ext
  `overview(m) { grid(m); }` semantics.** The per-tag thumbnail grid
  always carved the work area into 9 cells regardless of window
  count, so a tag with 1-2 windows ended up at ~⅓ × ⅓ of the screen
  — "küçük gözüküyor, natif değil." Mango-ext's overview is just a
  Grid layout over all visible clients (`tagset = !0` + Grid +
  floating-included filter), so cell count = window count. Net
  effect: 1 window ≈ 90% × 90% of the screen, 2 → side-by-side
  halves, 4 → 2 × 2 quarters, 9 → 3 × 3 evenly. Cells shrink as
  window count grows, matching the native MangoWM feel.
  * Removed `MargoState::arrange_overview_per_tag_grid` helper
    (~95 LOC including doc) and its `is_overview` branch in
    `arrange_monitor`.
  * The `is_overview` setup at the top of `arrange_monitor`
    (`layout = Grid` + `tagset = !0` + `is_tiled` filter relaxed)
    is now sufficient — a single `layout::arrange(layout, &ctx)`
    call produces the dynamic grid.
  * hot-corner / alt+Tab cycle / alt+Return commit / 4-finger
    swipe / snap-no-slide cycle animation all unchanged.

- **Overview reverted from "Infinite Spatial" back to Mango-style
  per-tag thumbnail grid.** Five commits of camera-pan canvas
  (foundation + state + nav + auto-fit + window-centred cycle) were
  reverted in one pass after live UX feedback: the live camera
  felt fiddly compared to a fixed-grid that the user's spatial
  memory could rely on. Final shape:
  * Fixed 3×3 grid (tag 1 top-left → tag 9 bottom-right). Same cell
    index every time, spatial memory carries.
  * Each thumbnail runs that tag's configured layout (Tile /
    Scroller / Grid / Canvas / …). Scroller tag stays
    scroller-shaped, grid tag stays grid-shaped.
  * alt+Tab MRU cycle keeps the snap-no-slide arrange from the
    spatial attempt — each Tab press lights `focuscolor` border
    on the new selection instantly, no animation kaos.
  * `spatial_overview` module + design doc + 7 dispatch actions +
    `OverviewMode`/`overview_mode` config + `MargoState::spatial`/
    `spatial_panning` fields + `SpatialCamera` + frame-tick
    momentum + scroll-zoom intercept + LMB-drag pan handler all
    removed. ~600 LOC out, simpler render path, no spatial state
    to debug.

### Added (replaces previous Phase 3 entries)

- **Phase 3 — Spatial Overview live navigation (3 / 3, final).**
  Mouse + scroll + keyboard navigation all wired through the
  spatial camera; momentum decays every frame on the animation
  tick. Phase 3 is now fully usable.
  * **Mouse left-drag on empty overview space** pans the camera —
    every motion event streams its delta through
    `pan_by_screen_delta` so velocity feeds momentum on release.
  * **Scroll wheel** zooms around the cursor (world point under
    the cursor stays fixed, niri/paperwm/Aerospace default).
  * **Keyboard:** seven new dispatch actions —
    `overview_pan_left/right/up/down` step ¼ of the panel each,
    `overview_zoom_in/out` × 1.2 / × 1/1.2, `overview_zoom_reset`
    snaps to active tag at config zoom. Bind any of them inside
    overview for accessibility / no-mouse flows.
  * **Frame tick:** the same `tick_animations` hop that drives
    window animations now also ticks `MargoState::spatial`. While
    momentum is non-zero or the camera is interpolating toward a
    target, `arrange_all` runs and the next frame schedules — so
    the camera keeps coasting until friction settles it
    (`FRICTION = 0.92` per frame, `VELOCITY_FLOOR = 0.5 px/frame`
    snap-to-rest).
  * **mctl actions** catalogue grew seven entries documenting the
    pan/zoom/reset surface.

  Phase 3 is now feature-complete. Bind freely:

  ```ini
  overview_mode        = spatial      # default
  overview_zoom        = 0.5
  overview_transition_ms = 180
  hot_corner_top_left  = toggle_overview
  bind = alt,Tab,overview_focus_next
  bind = alt+shift,Tab,overview_focus_prev
  bind = alt,Return,overview_activate
  bind = super,Left,overview_pan_left
  bind = super,Right,overview_pan_right
  bind = super,Up,overview_pan_up
  bind = super,Down,overview_pan_down
  bind = super,equal,overview_zoom_in
  bind = super,minus,overview_zoom_out
  bind = super,0,overview_zoom_reset
  ```

- **Phase 3 — Spatial Overview wired into arrange + state (2 / 3).**
  Spatial mode is now the default at config level (`overview_mode =
  spatial` — opt out with `overview_mode = grid`). On open,
  `arrange_monitor` branches into the new
  `arrange_spatial_overview_geometries` helper:
  * Every tag's clients arrange in **that tag's** configured layout
    (Tile / Scroller / Grid / Canvas / …) inside a monitor-sized
    world slot — no override to a single Grid.
  * Each client's world rect (tag anchor + local layout output) is
    transformed through `SpatialCamera::world_to_screen` to land
    its `geom` on screen. Render, border, hit-test paths all read
    `client.geom` unchanged — they don't know spatial mode is on.
  * `open_overview` snaps `MargoState::spatial` to the active tag's
    world centre at `overview_zoom` so the open transition reads
    as "stay where I was, zoom out".

  Camera is loaded at default centred-zero state from
  `MargoState::new`; pan/zoom input handlers + frame-tick momentum
  arrive in commit 3 (final slice). Until commit 3 ships, spatial
  overview displays correctly but is static — exactly the visual
  the design doc calls for, just without live navigation.

- **Phase 3 — Infinite Spatial Overview, foundation (1 / 3).** New
  module `margo/src/spatial_overview.rs` (~450 LOC, 12 unit tests)
  carrying the foundation for the spatial canvas overview that
  replaces the legacy single-Grid overview as the default in
  commit 3 of this slice. Design doc at
  `docs/design/spatial-overview.md` covers the whole arc.

  This commit is foundation-only — no behaviour change:
  * `OverviewMode { Grid, Spatial }` enum + `from_config_str`
    parser (Grid alias: `grid` / `legacy` / `flat`; Spatial alias:
    `spatial` / `infinite` / `canvas`)
  * `SpatialCamera` struct — current + target position, momentum
    velocities, zoom clamps (`ZOOM_MIN = 0.1`, `ZOOM_MAX = 1.5`),
    friction (0.92 per frame), velocity floor (0.5 px/frame for
    snap-to-zero)
  * Methods: `snap_to` (hard re-centre), `pan_to` / `zoom_to_target`
    (set targets without snapping), `pan_by_screen_delta` (mouse
    drag), `zoom_around_screen_point` (scroll-zoom keeps the
    cursor's world point fixed), `tick` (per-frame integration:
    momentum → target, friction, smooth-step current → target)
  * Coordinate transforms `world_to_screen` / `screen_to_world` —
    the single transform every consumer goes through, so arrange,
    render, and input can't drift out of step
  * World layout: `tag_anchor` (3×3 grid, tag 1 top-left, tag 9
    bottom-right), `client_world_rect` (tag anchor + local layout
    rect), `world_bounds`
  * `TAG_PADDING` const (64 logical px between tag slots)

  Commit 2 (next) wires `MargoState::spatial`, `arrange_monitor`
  spatial branch, render path passthrough. Commit 3 adds input
  handlers (mouse pan, scroll zoom, keyboard dispatches),
  frame-tick momentum decay, and spatial-aware
  `overview_focus_next/_prev`.

### Fixed

- **Hot corner no longer leaks through to the lock screen.**
  `update_hot_corner` now early-exits when `session_locked` is true,
  when the screenshot region selector is active, or when smithay
  holds a pointer / keyboard grab (xdg_popup grabs, drag-and-drop).
  Symptom was: pointer in the top-left corner while the lock surface
  owned focus → `dispatch_action("toggle_overview")` fired → Tab /
  Return reached greetd's authentication form and the user landed
  in the login screen. Three guards added; armed_at stays None so
  re-entry restarts the timer cleanly after the guard lifts.

- **`overview_focus_next/_prev` border highlight tracks the
  selection.** The previous attempt called `focus_surface` on every
  Tab press, which fired margo's focus-crossfade opacity animation
  for each step. The crossfade re-painted all borders mid-cycle
  (interpolating between focuscolor and bordercolor), so the
  visible cue was "cursor warps but borders all look smudged".
  Now the cycle relies on the `is_overview_hovered` path that
  `border::refresh` already paints with `focuscolor`
  (`border.rs:64`), with no crossfade kick. Border, cursor, and
  hover flag move together on every Tab; commit goes through
  `overview_activate` (Enter), which runs the focus path once.

### Changed

- **Overview rewritten — Mango/Hypr geometric continuity + niri
  alt+Tab MRU cycle.** The Round 2b/3/4 per-tag thumbnail grid is
  reverted in favour of the previous "single Grid arrangement of
  every visible client over the zoomed work area" — windows keep a
  deterministic spot in the thumbnail, overview reads as a
  zoom-out of the desktop, the user's spatial memory survives.
  `arrange_overview_per_tag_grid`, `compute_overview_grid_layout`,
  `overview_cell_rect`, `overview_cell_at_cursor`,
  `overview_client_at_cursor`, and `OverviewDrag` all removed —
  ~600 LOC out, much simpler render path, no drift between three
  grid-math implementations. Round 1 (hot corner + zoom config +
  4-finger swipe wiring) and Round 2a (geometric zoom +
  transition_ms wiring) stay.

  `overview_focus_step` now opens the overview on its first press
  if it's closed, and every step calls
  `focus_surface(FocusTarget::Window(...))` so border + smithay
  keyboard focus track the cycle. Bind to alt+Tab and the gesture
  feels like a real alt+Tab on every other DE: focus moves with
  the selection, overview stays open between presses, commit via
  Enter (`overview_activate`).

  Try it:
  ```
  bind = alt,Tab,overview_focus_next
  bind = alt+shift,Tab,overview_focus_prev
  bind = alt,Return,overview_activate
  hot_corner_top_left = toggle_overview
  gesture = swipe, 4, up, toggle_overview
  overview_zoom = 0.5
  ```

  `mctl actions` catalogue now documents the three nav actions
  (`overview_focus_next`, `_prev`, `_activate`) explicitly with
  the new auto-open / focus-follows behaviour.

  **Phase 3 mandate (separate sprint):** "Infinite Spatial Overview"
  — workspace → space, 2D pan-zoomable canvas, semantic grouping,
  inertial camera, minimap. Design doc + opt-in `overview_mode =
  spatial` config. Not in this sprint; this overview ships now,
  spatial mode lands as an alternative later.

### Added

- **Niri-overview port — Round 4 (dynamic grid).** The overview no
  longer hard-codes a 3×3 grid of all 9 tags; instead, only tags
  with visible clients on the monitor are shown, and the grid
  shape is picked to fit: 1 occupied → 1×1 (full-screen
  thumbnail), 2 → 2×1, 3 → 3×1, 4 → 2×2, 5–6 → 3×2, 7–9 → 3×3.
  Even at `overview_zoom = 1.0` thumbnails were too small on a
  1080p monitor because we were always burning 6 cells of pixel
  budget on empty tags; now a single-tag day uses the whole
  screen. While a drag is past the 5 px threshold every tag is
  shown so empty tags become valid drop targets — drag UX
  unchanged. New `MargoState::compute_overview_grid_layout`
  helper is the single source of truth for the cell list;
  `arrange_overview_per_tag_grid`, `overview_cell_rect`, and
  `overview_cell_at_cursor` all consume it. Three-way drift gone.

- **Niri-overview port — Round 3 (mouse drag-and-drop windows across
  tags).** Inside the overview, left-press on a window thumbnail
  starts a drag; cursor motion past 5 px arms drag mode and
  highlights the target tag's cell with a `focuscolor` border;
  release on a cell rect retags the dragged window to that tag and
  re-arranges (overview stays open so the user can keep moving
  things). Release below the 5 px threshold, or outside any cell,
  falls back to the legacy click-to-activate-and-close behaviour —
  so a quick click on a thumbnail still opens that window like
  before. New `MargoState::overview_drag: Option<OverviewDrag>`
  state, plus `overview_cell_at_cursor` / `overview_cell_rect` /
  `overview_client_at_cursor` hit-test helpers (kept in math
  lock-step with `arrange_overview_per_tag_grid`). Visual feedback
  is a 4 px accent outline around the target cell — drawn after
  cursor so the cursor stays on top, before `upper_layers` so the
  bar still wins z-order.

  niri's "drag a window across workspaces" feature, adapted: niri
  inserts new workspaces between drop columns; margo doesn't
  (tags are abstract, no spatial "between"), so the drop simply
  retags onto the cell-tag.

### Changed

- **`toggle_overview` is the single dispatch name.** The
  `toggleoverview` / `toggle-overview` / `overview` aliases that
  briefly landed in 0.1.8 have been removed in favour of one
  canonical name. Update any keybinds / hot-corner config strings
  that used the underscore-less spelling. The `mctl actions`
  catalogue entry now reads `toggle_overview`.

- **Niri-overview port — Round 2b (per-tag thumbnails).** Overview
  no longer dumps every visible window into one Grid; instead, each
  tag (1-9) gets its own thumbnail cell in a 3×3 layout over the
  zoomed work area, and *each cell runs that tag's configured
  layout* — a scroller tag stays scroller-shaped at thumbnail size,
  a grid tag stays grid-shaped, etc. Per-tag `mfact` / `nmaster` /
  layout from `Pertag::ltidxs` flow through unchanged. Empty tags
  get an empty cell. Tag → cell mapping: tag 1 top-left, tag 9
  bottom-right (matches the 1-9 keypad mental model). New
  `MargoState::arrange_overview_per_tag_grid` helper drives the
  cell-by-cell arrange; `arrange_monitor` branches into it when
  `is_overview` is set. Round 3's drag-and-drop will hit-test
  against these cell rects to drop windows onto target tags.

- **Niri-overview port — Round 2a (geometric zoom + transition wiring).**
  `overview_zoom` (added in 0.1.8) is now consumed by
  `arrange_monitor`: while overview is open, the work area shrinks
  to `zoom × work_area` centered inside the monitor's logical work
  rect, so windows arrange inside a smaller, centered region —
  niri's "zoom 0.5" feel without a true scene-tree transform.
  Layer-shell positioning is unchanged on purpose: top + overlay
  layers (the bar) stay anchored to the panel edges, matching
  niri's "background + bottom would zoom in lockstep, top + overlay
  stay at 1.0" pattern. `overview_transition_ms` config is now
  honoured via a new `overview_transition_ms()` helper (fallback
  180 ms when config value is 0).

  Round 2b (per-tag thumbnails — every tag gets its own mini-layout
  area, not the current "every window in one Grid") and Round 3
  (mouse drag-and-drop windows across tags inside overview) are
  the next two slices.

## [0.1.8] – 2026-05-10

Niri-overview port — Round 1 (trigger mechanics). The next two rounds
(zoom-out / layer-shell handling, and mouse drag-and-drop windows
across tags) ship as follow-up releases.

### Added

- **Hot corner trigger.** Pointer dwelling in a 1×1-logical-pixel
  rectangle at any of the four output corners fires a configured
  dispatch action — niri pattern with a dwell threshold so a quick
  flick past the corner doesn't trigger. Per-corner config; default
  is "off" until the user opts in.

  ```ini
  hot_corner_top_left      = toggle_overview
  hot_corner_top_right     =
  hot_corner_bottom_left   =
  hot_corner_bottom_right  =
  hot_corner_dwell_ms      = 100
  ```

  Cleared on pointer-leave so out-and-back-in restarts the timer
  (matches niri). Action string accepts every known dispatch name
  (`toggleoverview` / `toggle_overview` / `toggle-overview` /
  `overview` all alias to the same handler).
- **Overview config knobs.** Two new fields:
  - `overview_zoom` (default `0.5`, clamped `[0.1, 1.0]`) — wired in
    config + state today; the Round-2 layer-shell + zoom-out render
    pass consumes it.
  - `overview_transition_ms` (default `180`) — replaces the
    previously-hardcoded transition duration.
- **`toggle_overview` dispatch aliases.** The handler used to only
  accept the no-underscore `toggleoverview` string; now also takes
  `toggle_overview`, `toggle-overview`, and bare `overview` so
  config strings written to the new hot-corner fields don't have to
  guess the spelling. The same handler underpins the existing
  keybind path and the (already-supported) 4-finger swipe-up
  gesture binding:

  ```ini
  bind = super,grave,toggle_overview
  gesture = swipe, 4, up, toggle_overview
  ```

### Changed

- `MargoState` gains `hot_corner_dwelling: Option<HotCorner>` +
  `hot_corner_armed_at: Option<Instant>` to drive the dwell timer.
  `update_hot_corner()` runs at the tail of every `pointer_motion`
  handler — cheap (4 corner checks per output, no allocation).

### What's coming in Round 2 / 3

- **Round 2 (next release):** real zoom-out rendering (overview
  thumbnails respect `overview_zoom`), layer-shell handling
  (background + bottom layers zoom along, overlay + top stay at
  1.0 — niri pattern).
- **Round 3:** mouse drag-and-drop windows across tags inside the
  overview, with target-tag visual highlight.

## [0.1.7] – 2026-05-10

First Phase 2 release. Single user-facing feature: a real fix for
fullscreen — the prior `togglefullscreen` looked full-screen but the
bar (noctalia / wlr-bar) kept rendering on top, covering the
window's top portion. Now there are two distinct fullscreen modes,
each on its own keybind.

### Added

- **`togglefullscreen_exclusive` dispatch action.** True fullscreen:
  window resizes to `monitor_area` (entire output) and the render
  path suppresses every layer-shell surface on that monitor — the
  bar literally disappears while exclusive fullscreen is active.
  Right behaviour for mpv / browser fullscreen movie / fullscreen
  games. Aliases: `togglefullscreen-exclusive`,
  `togglefullscreenexclusive`.

  ```ini
  bind = super,f,togglefullscreen
  bind = super+shift,f,togglefullscreen_exclusive
  ```

### Changed

- **`togglefullscreen` now respects `work_area`.** The default
  fullscreen action used to size the window to the full
  `monitor_area` even though the layer-shell bar kept rendering on
  top — the window's top region was permanently covered. Now the
  window resizes to `monitors[].work_area` (after layer-shell
  exclusion zones), so the bar stays visible and the window covers
  every other pixel below it. Standard `F11` feel.
- **`MargoClient` gains a `fullscreen_mode: FullscreenMode { Off,
  WorkArea, Exclusive }` field** alongside the existing
  `is_fullscreen: bool`. The bool is kept in lock-step
  (`is_fullscreen == fullscreen_mode != Off`) for backward-compat
  with 20+ callsites in render / IPC / window-rule paths;
  `set_client_fullscreen_mode(idx, mode)` is the new source of
  truth and `set_client_fullscreen(idx, bool)` shims to
  `WorkArea`. `xdg_toplevel` size hint matches the active mode so
  client first-frame buffer allocations land correctly.

## [0.1.6] – 2026-05-10

A `mvisual` UX hot-fix. `cargo run -p mvisual` flashed a window for a
single frame and exited — the design tool was unusable.

### Fixed

- **`mvisual` window no longer flashes-and-quits.** GApplication
  registers itself on the session bus by default; if a stale
  `dev.margo.visual` name was still claimed (most commonly: a previous
  `cargo run` session whose dbus name hadn't been released), the
  second start registered as *remote*, forwarded the `activate` signal
  to the (now-dead) primary, and exited immediately. Symptom was a
  window appearing on screen for one frame then disappearing,
  with no error output. Fixed by passing
  `gio::ApplicationFlags::NON_UNIQUE` on the Application builder —
  mvisual is a developer / design tool, multiple parallel instances
  are intentional.

## [0.1.5] – 2026-05-10

A 0.1.4 hot-fix. The `theme` / `session-save` / `session-load`
subcommands wired in 0.1.4 ran without error but had no visible
effect — every preset switch silently fell through to `default`.
`mctl run <file>` was carrying the same latent bug. One commit, one
slot-fix; everything user-facing actually works now.

### Fixed

- **`mctl theme <preset>` payload now reaches the dispatch handler.**
  dwl-ipc-v2's `dispatch` request takes 5 string slots; margo maps
  them as `arg1 → arg.i` (numeric parse), `arg2 → arg.i2`,
  `arg3 → arg.f`, `arg4 → arg.v` (string), `arg5 → arg.v2`. The
  0.1.4 `Theme { preset }` clap variant was stuffing the preset
  into slot 1 — the i32 parse silently failed, `arg.v` stayed
  `None`, and `theme gaudy` quietly resolved to the `default`
  preset. Now lands in slot 4 alongside the convention every other
  string-payload dispatch follows. `session-save` / `session-load`
  don't take args so they were already correct; the latent
  `mctl run <file>` bug (path stuffed into slot 1, `run_script`
  handler reads `arg.v`) is fixed in the same pass.

## [0.1.4] – 2026-05-10

A "0.1.3 follow-up" release. The 0.1.3 commit added the `theme` /
`session_save` / `session_load` dispatch actions on the compositor
side but didn't wire them as `mctl` clap subcommands — running
`mctl theme gaudy` died with "unrecognized subcommand". This fixes
that, plus a hot-path structured-logging migration and a road-map
reorganisation that were already pending in `[Unreleased]`.

### Fixed

- **`mctl theme` / `mctl session-save` / `mctl session-load`
  subcommands.** Three new `Command` variants in the `mctl` clap
  parser route through the existing dispatch path. No
  compositor-side change — the dispatch handlers landed in 0.1.3,
  only the CLI surface was missing. `mctl --help` now lists all
  three; `session-save`/`session-load` accept the underscore alias
  too for symmetry with the action name.

### Changed

- **Hot-path logging migrated to `tracing` structured fields.**
  `backend/udev/frame.rs`, `backend/udev/hotplug.rs`, and the gesture +
  keybinding-match log lines in `input_handler.rs` now emit per-event
  fields (`output = %name`, `reason = …`, `queued = …`, `error = ?e`)
  instead of pre-formatted strings. Run with `tracing-subscriber`'s
  JSON formatter and `journalctl -u margo --output=json | jq` slices
  per-output traces cleanly. Cold-path callsites (state.rs focus /
  dispatch chatter, scripting, plugin loader) still use the old
  format-string shape and convert piecemeal as touched. Roadmap §16
  do-over wishlist item.

### Docs

- **Roadmap §15 reorganised into "Outstanding work — external
  triggers"** with three sub-tables: upstream-blocked (smithay PRs),
  test-setup-deferred (live PipeWire), and hardware-driven (W2.2b
  pixman, W2.3 tablet). All margo-internal long-tail items are
  shipped — what's left is gated on something margo can't unblock by
  itself. §16 do-over wishlist marks the WindowRuleReason and
  RenderTarget refactors as shipped/partial; structured logging note
  added.

## [0.1.3] – 2026-05-10

A "post-W-sweep capability + cleanup" pass. Four features and three
refactors land between the 0.1.2 release and now; together they close
out every internal long-tail item the road map flagged.

### Added

- **`mctl theme <preset>` — live visual theme switch.** Three built-in
  presets (`default` / `minimal` / `gaudy`) toggle border thickness,
  shadow depth, blur, and corner radius without touching the config
  file. First switch captures a `theme_baseline` snapshot so
  `default` always reverts to "what the config said"; `mctl reload`
  invalidates the baseline so the next `default` lands the freshly-
  parsed values. (`feat(theme)`)
- **`mctl session save` / `mctl session load`.** JSON snapshot of
  every monitor's tag selection, per-tag layout / mfact / nmaster /
  canvas-pan to `$XDG_STATE_HOME/margo/session.json`. Atomic write
  via temp + rename so a crash mid-write can't shadow a good file.
  Open windows aren't captured (clients are bound to processes — the
  spawn line lives in user-space). Snapshot entries for absent
  monitors are logged + skipped on load. Versioned format with
  rejection on mismatch. (`feat(session)`)
- **Touchscreen multi-finger swipe → `gesture_bindings` dispatch.**
  True touch events (TouchDown/Motion/Up) are now distilled into
  the same `(fingers, motion, mods) → action` lookup the touchpad
  swipe path uses. A binding written as `gesture = swipe, 3,
  right, view_tag` fires regardless of input surface. (`feat(input)`)
- **`presentation-time` real per-output VBlank seq.** The `seq` field
  in `wp_presentation_feedback.presented` was hardcoded to 0; it's
  now a monotonic `OutputDevice::vblank_seq` bumped at the head of
  every `DrmEvent::VBlank` handler. Frame-pacing-sensitive consumers
  (mpv `--vo=gpu-next`, kitty render loop, gnome-shell's
  `getRefreshRate` polling) now see the contract the protocol
  promises. (`feat(presentation-time)`)

### Changed

- **Window-rule reapply unified via `WindowRuleReason` enum.** Three
  trigger sites (`finalize_initial_map`, late `app_id` settle,
  `mctl reload`) previously called `apply_window_rules_to_client`
  with no shared signal of *why* a rule was firing. New
  `WindowRuleReason::{InitialMap, AppIdSettled, Reload}` is passed
  to a single `reapply_rules(idx, reason)` path; the debug log
  records the trigger so a `RUST_LOG=margo::state::windowrule=debug`
  trace tells you which call site landed. Roadmap §16 #4 do-over
  wishlist item. (`refactor(state)`)
- **`RenderTarget` enum replaces `(include_cursor, for_screencast)`
  bool pair.** `build_render_elements_inner` callsites now read
  `RenderTarget::Display` / `DisplayNoCursor` / `Screencast { .. }`
  instead of two anonymous booleans the reader had to remember the
  meaning of. Internal `flags()` helper unpacks back into the same
  two bools the function body still uses, so the hot path is
  unchanged. Partial address of roadmap §16 #1. (`refactor(udev)`)

## [0.1.2] – 2026-05-10

A "catch-and-surpass-niri sweep" tail-end release. Three commits land
the last three queued W-items: a GTK4 design tool, HDR Phase 4 ICC
scaffolding, and the udev backend split into focused sub-modules. No
behaviour changes for existing daily-driver flows — the W-sweep is
about coverage and architecture, and the test suite (181 passing) +
clippy gate stay green at every step.

### Added

- **`mvisual` design tool (W4.5).** New workspace binary
  (`cargo run -p mvisual`) renders all 14 tile-able layouts side-by-side
  as live thumbnails plus a 1‒9 tag rail that mirrors the compositor's
  `Pertag` so users can rehearse per-tag layout pinning before
  committing to a config. GTK4-rs UI; live re-arrange on every
  parameter tweak (window count / mfact / nmaster / inner+outer gaps /
  focus index / scroller proportion). Wider than `niri-visual-tests`
  on two axes: every layout visible at once (no click-cycle), plus
  the per-tag pinning preview niri can't host since it has no tags.
- **`margo-layouts` workspace crate.** Pure layout arithmetic
  (~1040 LOC, no smithay/wlroots deps) extracted from
  `margo/src/layout/{mod,algorithms}.rs` so the compositor binary
  and `mvisual` consume the exact same `arrange()`. The 38-snapshot
  layout regression suite stays in place, just retargeted at the new
  crate.
- **HDR Phase 4 — per-output ICC profiles (scaffolding).**
  `margo/src/render/icc_lut.rs` (~390 LOC, 6 unit tests). `colord`
  D-Bus client (`org.freedesktop.ColorManager` + Device + Profile
  proxies) resolves a DRM connector name → assigned ICC path;
  `lcms2`-backed `bake_lut` runs an identity 33³ grid through
  sRGB → display-profile transform; `to_atlas_rgba32f` re-lays the
  cube as a 1089 × 33 RGB texture so the GLES2 path can sample it
  without a `sampler3D`. CPU-side trilinear sampler doubles as the
  GLSL reference for the `ICC_LUT_FRAG` shader (ships as `const`).
  `MARGO_HDR_ICC=1` env gate. Runtime activation upstream-blocked
  on smithay's `compile_custom_texture_shader` exposing a
  second-sampler hook.

### Changed

- **`backend/udev.rs` (3934 LOC) split into 4 sub-modules (W4.1).**
  `backend/udev/` is now a directory: `mod.rs` (2873, ~27 % shrink,
  the orchestrator), `helpers.rs` (77, transform / CRTC pick /
  refresh-duration / monotonic clock), `mode.rs` (234, mode select +
  apply via `DrmCompositor::use_mode`), `hotplug.rs` (405, rescan +
  setup_connector + migrate-clients-off-output), `frame.rs` (331,
  render dispatch + presentation feedback + scanout flags). Type
  visibility for `OutputDevice` / `BackendData` / `GammaProps` lifted
  to `pub(super)` so submodules reach shared state without trait
  indirection. Behaviour-preserving — all 181 tests green at every
  extract step. The road map's earlier "split into separate crates"
  framing was rejected: niri's "7 backend crates" turn out to be
  smithay's *feature flags*, not crates, and the real wins
  (incremental compile + readability) land at sub-module granularity
  without trait-abstracting `MargoState` (~3000 LOC churn for no
  downstream consumer).

## [0.1.1] – 2026-05-10

A focused popup-handling bug-fix release. Three commits, one
chain of root causes — GTK and Chromium menus (Helium 3-dot,
Nemo right-click, file-picker dropdowns) were unusable because
xdg_popup wasn't being driven through the full xdg-shell
handshake. After this release, popups, right-click context
menus, and double-click navigation work as expected on every
xdg-shell client we've tested.

### Fixed

- **Initial configure for xdg_popups.** Margo's commit handler
  was pumping the initial `xdg_surface.configure` for toplevels
  and layer surfaces but never for popups. Without it, GTK and
  Chromium would create the popup, send a bufferless commit, and
  sit forever waiting for an ack — the popup was tracked
  internally but never mapped, and clients gave up silently.
  Visible symptom: Helium's 3-dot menu, Nemo's right-click
  context menu, and any GTK chevron dropdown did absolutely
  nothing on click; `GDK_BACKEND=x11` worked because XWayland
  takes a different protocol path. The commit handler now mirrors
  smithay anvil's pattern: find the popup via `PopupManager`, and
  if `is_initial_configure_sent()` is false, call `send_configure()`
  on the first commit. Also restores the original double-click
  navigation in Nemo, which was failing as a side effect of the
  same broken popup state.
- **Pointer/keyboard input no longer steals focus during an active
  grab.** Even after wiring up `PopupPointerGrab`/`PopupKeyboardGrab`,
  GTK and Chromium menus would still flicker open and close because
  `handle_pointer_button` and `apply_sloppy_focus` called
  `state.focus_surface(...)` *before* forwarding the click. The
  toplevel-level `focus_under()` lookup can't see popups (popups
  aren't in `state.space.elements()`), so it returned whichever
  toplevel the popup happened to overlap geometrically — and our
  side effects (`selected`, dwl-ipc broadcast, scripting hooks,
  border crossfade, sloppy-focus arrange) ran against the wrong
  window while the popup was still up. The visible symptoms were
  "menu opens for one frame, then closes", right-click producing a
  brief flash, and Nemo double-clicks getting routed as window
  focus swaps. Both call sites now skip our focus logic when
  `pointer.is_grabbed()` or `keyboard.is_grabbed()` — smithay's
  active grab owns focus routing for the duration, and dismissal
  re-establishes focus through the normal motion path.
- **`xdg_popup.grab` now sets up a real popup grab.** Browser
  context menus (Helium / Chromium right-click), Helium's 3-dot
  toolbar menu, Nemo's right-click context menu, GTK file-picker
  dropdowns, and any other popup that requests `xdg_popup.grab`
  could open and instantly dismiss because margo was only
  flipping keyboard focus to the popup wl_surface — pointer
  events kept being delivered to the parent toplevel, so the
  toplevel saw a click "outside" the popup it had just opened
  and tore the popup down. The visible symptom was "menu doesn't
  open" / "right-click doesn't work" / "double-click does
  nothing". Margo now goes through the standard smithay path:
  `PopupManager::grab_popup` validates the serial, ensures the
  popup is the topmost in its chain, and returns a `PopupGrab`;
  margo then installs that grab on both the keyboard
  (`PopupKeyboardGrab`) and pointer (`PopupPointerGrab`) so
  events drill through the popup hierarchy and clicks outside
  dismiss the chain. Implementing this required two trivial
  `From` impls — `From<PopupKind> for FocusTarget` and
  `From<FocusTarget> for WlSurface` — that the previous
  workaround had explicitly side-stepped.

## [0.1.0] – 2026-05-10

First public release. margo crosses from "in-progress Rust port of mango"
into "daily-driver Wayland compositor with full modern-protocol parity,
the dwm/dwl-style 9-tag workflow, 14-layout catalogue, niri-grade
animations, embedded scripting, an in-compositor screencast portal,
and HDR scaffolding." Every line in the workspace is original to this
project except for the deliberately-attributed portions of dwl, dwm,
sway, tinywl, and wlroots — see `LICENSE.*`.

### Compositor

- **Tag-based workflow** — nine multi-select tags per session,
  `view N` / `tag N`, dwm-style press-twice-for-back, per-tag
  home monitor (`tagrule = id:N, monitor_name:X`) with automatic
  warp on view, per-tag layout / mfact / nmaster pinning via
  `Pertag`, per-tag wallpaper hint surfaced through `state.json`
  for wallpaper daemons.
- **Layout catalogue** — `tile`, `right_tile`, `monocle`, `grid`,
  `deck`, `center_tile`, `scroller`, `vertical_tile`,
  `vertical_grid`, `vertical_scroller`, `vertical_deck`,
  `tgmix`, `canvas`, `dwindle`, plus a global overview mode.
  Each layout is a pure function of `ArrangeCtx → Vec<(idx, Rect)>`
  so every algorithm gets snapshot-tested against a committed
  text fixture.
- **Adaptive layout engine** — per-tag `user_picked_layout`
  sticky bit + window-count / aspect-ratio heuristic; user
  `setlayout` pins the choice, heuristic never overrides.
- **Spatial canvas** — PaperWM-style per-tag pan via
  `canvas_pan` / `canvas_reset` actions, threaded into 5 layout
  algorithms.
- **Animations** — niri-style analytical spring physics with
  mid-flight retarget for window movement, carefully-tuned
  bezier curves for open / close / tag / focus / layer
  transitions. All five animation types support both clocks
  via `animation_clock_*` per-domain config. Snapshot-driven
  open / close so there's no first-frame "pop" before the
  transition starts.
- **Drop shadows + rounded corners** — single-pass SDF GLES
  shader, no offscreen buffers; clipped-surface rounded-corner
  mask shared across windowed / fullscreen / animated paths.
- **Modern protocol stack** — `linux-dmabuf-v1` +
  `linux-drm-syncobj-v1` (Firefox / Chromium / GTK / Qt avoid
  SHM fallback), DMA-BUF screencopy (zero-copy GPU→GPU full-
  output capture), region-based screencopy crop, runtime
  `wlr-output-management-v1` (mode + scale + position changes
  apply live, kanshi compatible), `pointer_constraints_v1` +
  `relative_pointer_v1` (FPS games / Blender), `xdg_activation_v1`
  with strict-by-default anti-focus-steal policy,
  VBlank-accurate `presentation-time`, `wp_color_management_v1`
  (HDR Phase 1 protocol surface), `ext_idle_notifier_v1` +
  `idle-inhibit`, `text-input-v3` + `input-method-v2`,
  `ext-session-lock-v1`, `wlr-gamma-control-v1`,
  `ext-foreign-toplevel-list-v1`,
  `wp_single_pixel_buffer_v1`, `ext-image-capture-source-v1` +
  `ext-image-copy-capture-v1`.
- **Built-in xdg-desktop-portal-gnome backend** — five Mutter
  D-Bus interface shims (`org.gnome.Mutter.ScreenCast`,
  `.DisplayConfig`, `.Shell.Introspect`, `.Shell.Screenshot`,
  `.Mutter.ServiceChannel`) + a PipeWire pipeline that lights
  up the Window / Entire Screen tabs in browser meeting clients
  (Helium, Chromium, Edge, Brave) without a running gnome-shell.
  Includes paced rendering, per-cast damage tracking, embedded
  cursor + metadata cursor sidecar, full-decoration casts
  (borders / shadows / popups / animations / block-out come
  through to the share view), HiDPI scale handling, and live
  `windows_changed` updates so xdp-gnome's window picker stays
  fresh mid-share-dialog.
- **Window rules with PCRE2** — regex match by `app_id` /
  `title` / `exclude_*`, size constraints, floating geometry,
  per-rule animation overrides, `block_out_from_screencast`,
  scratchpad / named-scratchpad opt-in, CSD-allow whitelist.
  Late `app_id` / `title` reapply so Qt clients don't flicker.
- **Scratchpad system** — anonymous + named scratchpads,
  cross-monitor support, `single_scratchpad` mode, recovery
  via `unscratchpad_focused` and `super+ctrl+Escape` reset.
- **Embedded scripting** — Rhai 1.24 sandboxed engine with
  `dispatch(action, args)` plus state-introspection bindings
  (`current_tag`, `focused_appid`, `monitor_count`, …) and
  event hooks (`on_focus_change`, `on_tag_switch`,
  `on_window_open`, `on_window_close`) that fire from the
  compositor mainloop with a re-entrancy guard. Plugin
  packaging via `~/.config/margo/plugins/<name>/{plugin.toml,
  init.rhai}` discovers and loads multiple scripts; per-plugin
  errors don't take down the loader. `mctl run <file>` evaluates
  a script against the live engine for hot-edit workflows.
- **Hot reload** — `mctl reload` (and the bundled
  `Super+Ctrl+R` keybind) re-applies window rules, key binds,
  monitor topology, animation curves, and gestures without a
  logout. `mctl check-config` is the offline validator —
  exit 1 on regex compile errors, unknown fields, duplicate
  binds, or include-resolution loops.
- **HDR scaffolding (Phases 1 + 2 + 3)** —
  `wp_color_management_v1` global advertising primaries / TFs
  / parametric creator (Phase 1, shipped); fp16 linear-light
  composite math + GLSL shaders + spec-value verification
  (Phase 2, gated on smithay's swapchain reformat API);
  `HDR_OUTPUT_METADATA` blob writer + `EdidHdrBlock` parser
  (Phase 3, gated on smithay's `set_hdr_output_metadata`).
  Phase 4 (per-output ICC profiles) is queued.
- **dwl-ipc-v2 wire compat** — drop-in for noctalia,
  waybar-dwl, fnott, and any other dwl/mango widget. Rich
  state.json sidecar exposes `scratchpad_visible`,
  `scratchpad_hidden`, MRU `focus_history`, per-tag wallpaper.

### Companion tools

- **`mctl`** — IPC + dispatch CLI. Subcommands:
  `status` / `clients` / `outputs` / `focused` / `watch`
  (live JSON / table inspection), `dispatch` (40+ typed
  actions; mirrors `bind = …` argument shape),
  `actions [--names | --verbose]` (the dispatch catalogue),
  `rules --appid X --title Y --verbose` (offline rule
  introspection), `check-config` (offline validation),
  `reload`, `run <file>` (live Rhai eval), `spawn`,
  `migrate --from {hyprland, sway} <file>` (offline config
  translator). Stable JSON schema with `version: 1`.
- **`mlayout`** — named monitor-topology profiles for
  laptops with frequent dock changes. `mlayout suggest /
  list / set / save / edit`. Wraps `wlr-randr` against
  margo's `wlr-output-management-v1` handler so changes
  apply live without logout.
- **`mscreenshot`** — region / window / output capture.
  Wraps `grim` + `slurp` + `wl-copy` + an optional editor
  (`swappy` / `satty` if installed). Modes: `rec`, `area`,
  `screen`, `window`, `open`, `dir`. The in-compositor
  region selector (Print key default) replaces slurp's
  separate window with a dim-overlay + drag-rect UI on the
  margo render path itself.

### Architecture

- **State management** — `MargoState` lives in
  `margo/src/state.rs` (~6,100 LOC after the W4.2 split,
  down from 7,651). 15 protocol-handler impls extracted into
  `state/handlers/` files for incremental-compile wins
  (`xdg_decoration`, `session_lock`, `xdg_activation`,
  `layer_shell`, `color_management`, `idle`,
  `pointer_constraints`, `input_method`, `selection`,
  `gamma_control`, `screencopy`, `dmabuf`,
  `output_management`, `x11`, `xdg_shell`).
- **Workspace layout** — `margo` (compositor binary),
  `margo-config` (parser + types), `margo-ipc` (mctl + the
  dispatch action catalogue + Hyprland/Sway migrate),
  `mlayout`, `mscreenshot`. Pinned smithay revision
  `ff5fa7df`; Rust 1.85+.
- **Cargo features** — `dbus` (default; gates D-Bus +
  async-io), `xdp-gnome-screencast` (default; requires
  `dbus`; gates pipewire), `a11y` (off by default; gates
  AccessKit), `profile-with-tracy` (off by default; flips
  `tracy-client` to its full backend so a live Tracy GUI
  can connect). Three build configurations verified.
- **AccessKit a11y** — `accesskit_unix` adapter on a
  dedicated thread (zbus-on-mainloop deadlock avoidance),
  publishes the window list as accessible nodes. Orca and
  AT-SPI consumers can navigate margo's window state.
- **xwayland-satellite mode** — `--xwayland-satellite[=BIN]`
  spawns Supreeeme's xwayland-satellite as a separate
  process so X11 crashes can't take margo down.
  `--no-xwayland` disables X11 entirely. Default path stays
  in-tree (smithay `XWayland::spawn`).
- **Tracy profiler hooks** — six hot-path spans
  (`render_output`, `build_render_elements`,
  `arrange_monitor`, `tick_animations`, `handle_input`,
  `focus_surface`) compile to no-ops in normal builds.

### Test infrastructure

- **Layout snapshot suite** — 20 committed `.snap` text
  fixtures locking the geometry of all 14 layouts × multiple
  scenarios. Insta-based; pure text diff at PR review time.
- **Layout property tests** — 14 invariants verified across
  the full catalogue × {1, 2, 3, 5, 8} window counts × focus
  shift × gap-zero edge cases (cardinality, no-degenerate-rects,
  monocle / deck identity, tile-class disjointness, focus
  invariance for non-scroller layouts, scroller monotonic
  width growth, gap-zero work-area coverage, focus-centring
  invariant for every focused index).
- **Integration test fixture** — calloop-driven
  `Server` + `wayland-client` `Client` + `Fixture` harness
  (port of niri's `src/tests/{fixture,server,client}.rs`).
  All 15 W4.2-extracted protocol handlers have at least one
  integration test; **41 integration tests** across
  `xdg_shell`, `layer_shell`, `idle`, `xdg_decoration`,
  `session_lock`, `xdg_activation`, `pointer_constraints`,
  `gamma_control`, `screencopy`, `output_management`,
  `selection`, `globals`, plus negative-invariant pinning
  for `dmabuf` / `color_management` / `x11/xwm` (gated on
  backend prerequisites that the headless harness can't
  drive). Total in-tree workspace test count: **126**
  (compositor: 102 layout + property + integration; config
  parser: 9; mctl + ipc + migrate: 15).
- **Smoke testing** — `scripts/smoke-winit.sh` (build →
  spawn → IPC → reload → focus → kill → empty-status, runs
  in CI under Xvfb), `scripts/post-install-smoke.sh` (binary
  presence, example config parses, dispatch catalogue
  ≥30 entries, completion paths, license install).
- **Clippy gating** — workspace + all targets run under
  `-D warnings`; `clippy.toml` documents the
  smithay-handle interior-mutability allowlist.

### Documentation

- **Published site** at <https://kenanpelit.github.io/margo/>
  (mkdocs-material; deploy automated via
  `.github/workflows/docs.yml`). Pages: Overview, Install
  (Arch / source / Nix flake), Configuration overview,
  **Full configuration reference** (the entire annotated
  `config.example.conf` rendered inline via `pymdownx.snippets`,
  syntax-highlighted), Companion tools, Scripting, Manual
  checklist, three design notes (HDR, Built-in portal,
  Scripting engine), Roadmap, Contributing.
- **Annotated example config** — 1,028 lines at
  `margo/src/config.example.conf`; every option documented
  inline.
- **CONTRIBUTING.md + PR template** — quick-start build,
  code-layout map, lint posture, test workflow, conventional
  commit style, AI-contribution policy.

### Compatibility

- **Display managers** — ships `margo.desktop` (direct
  session) and `margo-uwsm.desktop` (UWSM-driven for
  systemd graphical-session.target plumbing).
- **Existing widgets / bars** — drop-in for noctalia,
  waybar-dwl, fnott via dwl-ipc-v2.
- **Migration** — `mctl migrate --from {hyprland, sway}`
  translates the high-value config subset (keybinds, spawn
  lines, workspace → tag bitmask conversions, modifier names,
  key aliases). Window rules / animations / monitor topology
  stay manual to avoid inventing wrong semantics.

### Packaging

- **Arch / makepkg** — PKGBUILD at the repo root installs
  `margo`, `mctl`, `mlayout`, `mscreenshot`, the wayland-
  session entries, the example layouts, the XDG portal
  preference at `/usr/share/xdg-desktop-portal/`, shell
  completions for bash / zsh / fish, and license headers
  for the dwl/dwm/sway/tinywl/wlroots inheritance chain.
- **Nix flake** — `flake.nix` exposes `packages.default`,
  `devShells.default` with `rust-analyzer` + `clippy`, plus
  `nixosModules.margo` and `hmModules.margo`.
- **GitHub Actions** — three workflows: `ci.yml`
  (build/test/clippy/check-config on every PR), `smoke.yml`
  (end-to-end nested-mode smoke under Xvfb), `docs.yml`
  (Pages deployment).

[Unreleased]: https://github.com/kenanpelit/margo/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/kenanpelit/margo/releases/tag/v0.1.0
