# Changelog

All notable changes to **margo** are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
