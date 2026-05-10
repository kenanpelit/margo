# Changelog

All notable changes to **margo** are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
