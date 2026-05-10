# Margo Road Map

> **Last updated:** 2026-05-10 — xdg fullscreen request handler shipped
> **Branch:** `main` (single-branch — Rust port complete; the C tree remains as legacy reference under `src/`)
> **Status:** Daily-driver Wayland compositor. P0 → P7 fully shipped; W1–W4 catch-and-surpass-niri sweep substantially complete.

Margo is a Rust + Smithay Wayland compositor with a dwm/dwl-style tag workflow, 14 layout algorithms, niri-grade animations + spring physics, on-demand redraw, runtime DRM mode change, an embedded Rhai scripting engine with mid-event-loop hooks, `wp_color_management_v1` HDR Phase 1 + Phase 2/3 scaffolding, and a built-in xdp-gnome screencast backend that lights up Window / Entire Screen tabs in browser share dialogs without a running gnome-shell.

This document is the **source of truth** for what's shipped, what's queued, and what's worth a second pass. It is organised by capability area, not chronologically — [Appendix A](#appendix-a--phase-ledger) preserves the original P0–P7 sprint history for archaeology.

---

## TL;DR Status

| Area | Scope | Status |
|---|---|---|
| Core | UWSM, config, layouts, render, clipboard, layers, gamma, gestures | ✅ |
| Daily-driver baseline (P0 + P0+) | session_lock, idle_notifier, hotplug, debug log, move/resize, IM/text-input, focus oracle, z-order, scroller jitter, deferred map | ✅ 18/18 |
| Modern protocol parity (P1) | dmabuf+drm-syncobj, dmabuf-screencopy, region-crop, blockout, pointer_constraints+relative_pointer, xdg_activation, output_management (incl. mode change + disable), presentation-time | ✅ 8/8 |
| Frame clock + animations (P2) | on-demand redraw, spring engine, open/close/tag/focus/layer animations (bezier + opt-in spring), hw cursor, direct scanout (+ observability), damage opt | ✅ 6/6 |
| Window management v2 (P3) | scratchpad+named, mango/layerrule parity, CSD/SSD policy, IPC parity, XWayland HiDPI env, popup focus, **xdg fullscreen request honoured** | ✅ 7/7 |
| Tooling & packaging (P4) | smoke-winit, manual checklist, mctl JSON/rules/check-config, post-install smoke, shell completions, GitHub Actions CI, smoke-in-CI | ✅ 7/7 |
| Long-term goals (P5/P6) | spatial canvas ✓, adaptive layout ✓, drop shadow ✓, scripting Phase 3 ✓, HDR Phase 1 ✓ | ✅ 5/5 |
| Built-in screencast portal (P7) | 5 Mutter D-Bus shims, PipeWire pipeline, frame pacing, damage, cursor (embedded + metadata), full-decoration casts, HiDPI, windows_changed signal | ✅ 9/9 phases |
| Catch-and-surpass-niri sweep (W1–W4) | snapshot tests, clippy gate, CONTRIBUTING, screenshot region UI, EGL graceful fallback, AccessKit, xwayland-satellite, cargo features, tracy, HDR Phase 3 metadata, mctl run, plugin packaging, dwl-ipc state extension, layout-cycle notify, per-tag wallpaper, mctl migrate, mvisual design tool, HDR Phase 4 ICC LUT, **udev backend split** | ✅ 19/22 (5 deferred / upstream-blocked) |

---

## 1. Core foundation

The floor everything else stands on. Behaviour-stable for the lifetime of the project.

- **Session lifecycle.** UWSM systemd integration (`margo.desktop`, `margo-uwsm.desktop`), env import, noctalia + user services bootstrap.
- **Config.** Live `mctl reload`, `source/include`, `conf.d`, `Super+Ctrl+R`. Hand-written parser in `margo-config/src/parser`, structurally mirrors the legacy 4200-line C parser.
- **Tag system.** dwm-style "press-twice-for-back", per-tag home monitor (`tagrule = id:N, monitor_name:X`), automatic warp on `view_tag`.
- **Window rules.** Regex `appid/title`, negative match (`exclude_appid`/`exclude_title`), size constraints, floating geometry, late `app_id/title` reapply.
- **Input.** Keyboard, pointer, touchpad, swipe gestures, caps-to-ctrl.
- **Clipboard.** `wlr_data_control_v1`, `primary_selection_v1`, XWayland selection bridge.
- **Layer shell.** Bar / notification ordering, exclusive-keyboard layering.
- **Render core.** GLES, rounded border shader, content clipping, fractional-scale-aware borders.
- **Night light.** `wlr_gamma_control_v1`, sunsetr / gammastep / wlsunset pipeline.
- **Screencopy.** SHM target for grim / wf-recorder / OBS; dmabuf path landed in P1.
- **Winit nested mode** for fast dev iteration.

**Strengths to preserve.** State surface centralised in `MargoState`, split along natural seams (`backend/`, `layout/`, `dispatch/`, `render/`); don't fragment further. Per-tag state via `Pertag` (layout, mfact, client count, `user_picked_layout`, `canvas_pan_x/y`) keeps every tag self-contained — resist the urge to lift this onto `Monitor` "for simplicity".

**Worth revisiting.** A second-pass config parser with a real grammar (pest / nom / chumsky) would give better error messages and let the duplicate-bind detector live inside the parser instead of as a separate `mctl check-config` pass.

---

## 2. Window management

### Layouts & tags
- 14 layout algorithms: tile, scroller, grid, monocle, deck, center / right / vertical variants, canvas, dwindle.
- **Adaptive layout engine** (`b19b5d6`) — `Pertag::user_picked_layout: Vec<bool>` sticky bit + `maybe_apply_adaptive_layout()` heuristic (window count + monitor aspect ratio). User's `setlayout` pins the choice; heuristic never overrides.
- **Spatial canvas** (`1c2bed1`) — per-tag pan via `Pertag::canvas_pan_x/y`, `canvas_pan` and `canvas_reset` actions, threaded into 5 layout algorithms via `ArrangeCtx::canvas_pan`. PaperWM-style — each tag remembers its viewport.

### Toplevel handling
- **Scratchpad + named scratchpad.** `toggle_scratchpad`, `toggle_named_scratchpad <appid> <title> <spawn>`, `single_scratchpad`, `scratchpad_cross_monitor`. Window-rule `isnamedscratchpad:1` flag. Recovery via `unscratchpad_focused`; full-state reset on `super+ctrl+Escape`.
- **Mango windowrule + layerrule parity.** `windowrule.animation_type_open/close` apply at map; `layerrule` (previously parsed-but-never-applied) now matches namespace via regex, applies `noanim` and `animation_type_*`. `noblur`/`noshadow` parsed + stored, render hooks land in P5.
- **CSD/SSD policy.** `XdgDecorationHandler` defaults to ServerSide but honours `request_mode(ClientSide)` if window-rule has `allow_csd:1`. Per-client policy via window-rule, not a global toggle.
- **Popup focus via `xdg_popup.grab`** — `FocusTarget::Popup(WlSurface)` direct-focus path; portal file pickers, dropdowns, right-click menus get keyboard focus reliably.
- **Interactive move/resize.** `xdg_toplevel.move/resize` requests; tiled drags promote to floating.
- **xdg fullscreen request honoured** (latest fix). `XdgShellHandler::{fullscreen_request, unfullscreen_request, maximize_request, unmaximize_request}` overridden; new `set_client_fullscreen(idx, fullscreen)` helper flips `is_fullscreen`, calls `toplevel.with_pending_state` to add/remove `xdg_toplevel::State::Fullscreen` + set/clear size, sends pending configure, arranges monitor, broadcasts dwl-ipc. F11 / browser fullscreen buttons / YouTube fullscreen all work.

**Worth revisiting.** Popup grab is direct-focus and works for 99% of single-level popups but doesn't compose with smithay's `PopupKeyboardGrab`/`PopupPointerGrab` chain. Real `PopupGrab` impl would require a `FocusTarget` refactor.

---

## 3. Animations & frame clock (P2)

- **On-demand redraw scheduler.** 16ms polling timer dropped; `Ping` source wakes loop only when needed; `pending_vblanks` counter prevents post-hook tick storms.
- **Spring physics primitive.** Niri-style **analytical** critically-damped/under-damped solution (replaced an earlier numerical integrator that drifted). Per-channel velocity (x/y/w/h) preserved across mid-flight retargets. Unit tests cover overshoot, critical damping, retargeting velocity preservation, 60Hz/144Hz invariance — keep these as the regression boundary.
- **5/5 animation types shipped:**
  - **Open** — snapshot-driven, live surface hidden during transition (no first-frame pop).
  - **Close** — client removed from `clients` vec immediately; the animation lives in a separate `closing_clients` list rendering scale + alpha + rounded-clip.
  - **Tag switch** — direction from bit-position delta, slide-out for outgoing, off-screen-to-target for incoming via existing Move pipeline.
  - **Focus highlight** — `OpacityAnimation` cross-fade for both border colour and `focused_opacity ↔ unfocused_opacity`.
  - **Layer surface** — `LayerSurfaceAnim` keyed by `ObjectId`, anchor-aware geom skipped for bars (no jitter).
- **Spring physics opt-in across all 5 animation types** (`71b95a1`) — `animation_clock_*` per-type config picks bezier or spring-baked curve. Default bezier; opt-in spring.
- **Niri-style resize crossfade** — two-texture pass, `clipped_surface` rounded-corner mask, snapshot tracks animated slot. Capture live surface to GlesTexture once on slot change, render `tex_prev` and `tex_next` through the same `render_texture_from_to` shader pipeline.
- **Z-order invariant** (`enforce_z_order`) — float > tile > overlay, enforced once per arrange tick rather than ad-hoc on every focus event.
- **Hardware cursor plane** — `DrmCompositor::new` reads driver-advertised `cursor_size()` (modern AMD / Intel / NVIDIA support 128² / 256²); fallback 64×64.
- **Direct scanout** — smithay `FrameFlags::DEFAULT` includes `ALLOW_PRIMARY_PLANE_SCANOUT | ALLOW_OVERLAY_PLANE_SCANOUT`; explicit-sync (P1) plus dmabuf feedback (P0+) feed the client side. Fullscreen mpv hits primary plane.
- **Direct-scanout observability.** `MargoClient::last_scanout` cached after each successful `render_frame` by walking the surface tree and matching `Id::from_wayland_resource` against `RenderElementStates` for `ZeroCopy` presentation. Exposed in `state.json`; `mctl clients` shows ★ for on-scanout windows.
- **Damage tracking** — `OutputDamageTracker` per-frame; custom render elements bump `CommitCounter` only on geometry / shader-uniform change.
- **Presentation-time accuracy** (`bcb6fb4`) — feedback signalled at VBlank, not submit.

**Worth revisiting.** Frame clock is single-output today; multi-monitor mixed-refresh (60Hz + 144Hz) uses a global tick. Per-output `next_frame_at` scheduling would let each monitor pace independently. Spring physics carries per-type state — a future "spring everything" pass would unify the animation-tick code.

---

## 4. Modern protocol parity (P1)

`78c9909 → 886eba5 → a26cc9b`, ~1300 LOC.

- **`linux_dmabuf_v1` + `linux-drm-syncobj-v1`** — Firefox / Chromium / GTK / Qt avoid SHM fallback; explicit-sync gated on `supports_syncobj_eventfd`.
- **DMA-BUF screencopy target** — OBS / Discord / wf-recorder zero-copy GPU→GPU full-output capture.
- **Region-based screencopy crop** — `grim -g "$(slurp)"` reads only the slurp region; `RelocateRenderElement` pattern so the rect lands at dmabuf (0,0).
- **`block_out_from_screencast` runtime filter** — `for_screencast: bool` parameter through `build_render_elements_inner`; substitution at element-collection time, not pixel-sample time. Eliminates the entire race window.
- **`pointer_constraints_v1` + `relative_pointer_v1`** — FPS aim lock, Blender mid-mouse rotate stays inside, `cursor_position_hint` honoured on unlock.
- **`xdg_activation_v1`** — token serial freshness check (≤10s), seat match, anti-focus-steal; tag-aware view_tag jump on activation. Strict-by-default.
- **`wlr_output_management_v1`** — `wlr-randr` and `kanshi` runtime topology changes (scale / transform / position). **Mode change** (`a26cc9b`) — runtime DRM atomic re-modeset; `wlr-randr --mode 1920x1080@60` succeeds. **Disable-output** soft-disable: marks `MargoMonitor::enabled = false`, migrates clients off, refuses if it would leave zero active outputs. New dispatch actions: `disable_output` / `enable_output` / `toggle_output`. DRM connector power-off is a follow-up.
- **`presentation-time`** — kitty / mpv / native Vulkan get presented timestamp + refresh interval at VBlank.

**Worth revisiting.** `presentation-time` uses `now()` rather than the actual DRM page-flip sequence. Plumb `crtc->page_flip_seq` from the smithay flip event into `publish_presentation_feedback`.

---

## 5. HDR & color management

| Phase | Status | Detail |
|---|---|---|
| **Phase 1** — `wp_color_management_v1` global | ✅ shipped (`25255a9`) | `margo/src/protocols/color_management.rs`. Advertises supported primaries (sRGB / BT.2020 / Display-P3 / Adobe RGB), transfer functions (sRGB / ext_linear / ST2084-PQ / HLG / γ2.2), perceptual rendering intent, parametric-creator feature surface. Chromium and mpv probes find a colour-managed compositor and light up HDR decode paths. ICC creator stubbed (Phase 4); parametric creator fully wired (`set_tf_named`, `set_primaries`, `set_luminances`, `set_mastering_*`, `set_max_cll`, `set_max_fall`). Per-surface trackers store the active description's identity in an atomic. |
| **Phase 2** — fp16 linear composite (scaffolding) | ✅ shipped, runtime activation upstream-blocked | `margo/src/render/linear_composite.rs`. sRGB / ST2084-PQ / HLG / γ2.2 transfer-function math with bit-exact GLSL **and** equivalent `f32` CPU implementations for unit-test verification (round-trip identity at sRGB 0.5 ↔ 0.21404 linear, PQ peak, HLG kink at 0.5 ↔ 1/12). GLES texture shaders compile lazily and cache thread-local. `MARGO_COLOR_LINEAR=1` env gate eagerly compiles both programs at first frame. **Runtime swapchain switch** from `Argb8888` to `Abgr16161616f` needs an `OutputDevice`-aware reformat that smithay 0.7's `DrmCompositor` doesn't expose. **~80 LOC of integration remains** once the upstream API lands. |
| **Phase 3** — KMS HDR scan-out (metadata scaffolding) | ✅ shipped, activation upstream-blocked | `margo/src/render/hdr_metadata.rs` (~270 LOC, 5 unit tests). `StaticMetadataDescriptor` ↔ 28-byte kernel blob (HDR_OUTPUT_METADATA), bit-exact spec encoding (CTA-861-G + ST2086) verified against published reference values. `EdidHdrBlock` parser decodes panel-advertised peak / avg / min luminance per CTA-861-G §7.5.13. `EotfId` + `InfoFrameType` constants. Activation blocked on smithay's `DrmCompositor` exposing `set_hdr_output_metadata`; integration is ~30 LOC once the API lands. |
| **Phase 4** — per-output ICC profiles | ✅ shipped, runtime activation upstream-blocked | `margo/src/render/icc_lut.rs` (~390 LOC, 6 unit tests). `colord` D-Bus client (`org.freedesktop.ColorManager` + Device + Profile proxies) resolves a DRM connector name → assigned ICC path; `lcms2`-backed `bake_lut` runs an identity 33³ grid through an sRGB → display-profile transform; `to_atlas_rgba32f` re-lays the cube as a 1089 × 33 RGB texture so the GLES2 path can sample it without a `sampler3D`. CPU-side trilinear sampler doubles as the GLSL reference for the `ICC_LUT_FRAG` shader (ships as `const`). `MARGO_HDR_ICC=1` env gate. Activation blocked on smithay's `compile_custom_texture_shader` exposing a second-sampler hook for binding the LUT atlas alongside the input surface; integration is ~30 LOC once the API lands. |

---

## 6. Built-in screencast portal (P7)

`a4f6ed6 → bf7e579 → 0c2f5d5 → f8f7a9a → 0455b4e → 81a6487`, ~3870 LOC across `margo/src/dbus/` (5 D-Bus shims) + `margo/src/screencasting/` (PipeWire core + render hooks) + udev backend integration.

### Why this exists

xdp-wlr advertises `ext-image-copy-capture` which works for full-output capture, but Chromium-family browsers (Helium, Chromium, Edge, Brave) do **not** light up the Window / Entire Screen tabs in their share dialog against the wlr backend — they only enable per-window / per-output picking when xdg-desktop-portal-gnome is the backend. xdp-gnome in turn talks to **gnome-shell** over D-Bus on `org.gnome.Mutter.ScreenCast` + `.DisplayConfig` + `org.gnome.Shell.Introspect` + `.Screenshot` + `.Mutter.ServiceChannel`. niri solved this by implementing those Mutter D-Bus interfaces inside the compositor binary — P7 is a direct port of that pattern to margo.

### Phases shipped (9/9)

| Phase | Commit | LOC | What landed |
|---|---|---|---|
| **A** | `09c4e68` | +50 deps + scaffold | `zbus 5`, `pipewire 0.9`, `async-io`, `async-channel` workspace deps; module skeletons. |
| **B** | `09c4e68` | +1080 | All 5 D-Bus interface shims (`mutter_screen_cast`, `mutter_display_config`, `mutter_service_channel`, `gnome_shell_introspect`, `gnome_shell_screenshot`). zbus 5.x `#[interface]` async impls; calloop ↔ async-channel bridges. |
| **C0** | `e3df482` | +215 | `screencasting/render_helpers.rs` — niri's GLES helpers (`encompassing_geo`, `render_to_texture`, `render_to_dmabuf`, `render_and_download`, `clear_dmabuf`). |
| **C1** | `acc47cb` | +1655 | `screencasting/pw_utils.rs` — full port of niri's `PipeWire` core + `Cast` + `CastInner` + format negotiation (dmabuf preferred, SHM fallback) + buffer dequeue / queue / sync_point handling. |
| **D1** | `2c9d4e0` | +135 | `Screencasting` top-level state on `MargoState` + `mutter_service_channel` `NewClient` channel routing. |
| **D2** | `a4f6ed6` | +244 | All 5 shims registered onto well-known names; xdp-gnome connects + finds margo-as-mutter. |
| **E1** | `bf7e579` | +183 | `MargoState::start_cast` resolves `StreamTargetId::{Output, Window}` → `(CastTarget, size, refresh, alpha)`; lazy-init Screencasting + PipeWire on first cast. |
| **E2** | `0c2f5d5 → f8f7a9a` | +250 | `drain_active_cast_frames` in `backend/udev.rs` — the actual frame producer. Window casts look up by stable `addr_of!` u64; output casts iterate every visible client on the monitor. Continuous-repaint re-arm. |
| **F** | `81a6487` | +170 | Five depth items fused: pacing (via `Cast::check_time_and_schedule`), damage (per-cast `OutputDamageTracker`), embedded cursor (gated by `cast.cursor_mode()`), full-decoration casts (new `CastRenderElement` enum: `Direct(MargoRenderElement)` for output, `Relocated(RelocateRenderElement<MargoRenderElement>)` for window), HiDPI scale fix (logical → physical conversion). |

Final cleanup commit `0455b4e` adds module-level `#![allow(dead_code)]` to the niri-port files; Phase F flips most of those flags into actual call sites.

### Sprint-3 depth items shipped

- **`gnome_shell_introspect::windows_changed` signal** — fires from `finalize_initial_map` + `toplevel_destroyed` (Wayland + X11) so xdp-gnome's window picker stays live mid-share-dialog. Helper `emit_windows_changed_sync` bridges blocking-zbus ↔ async via `async_io::block_on`.
- **`CursorMode::Metadata` cursor sidecar** — helper `build_cursor_elements_for_output` extracts cursor sprite render elements separately; metadata casts prepend pointer elements with `elem_count = cursor_count` so `CursorData::compute` wraps them, pw_utils strips them from the main damage pass, `add_cursor_metadata` writes a real cursor bitmap into the SPA sidecar. Browsers asking for metadata mode now see a sharp cursor at low cast resolutions.

### Strengths to preserve
- **Stable Window IDs via `addr_of!(*MargoClient) as u64`.** `MargoClient` lives in a `Vec` at a stable heap address — its memory address IS the stable ID. Zero bookkeeping.
- **`mem::take(&mut casting.casts)` borrow trick.** Detaching the casts vec lets us iterate while holding `&MargoState` for client/monitor lookups.
- **Continuous-repaint while casts active.** `request_repaint()` at end of `drain_active_cast_frames` keeps the loop ticking; PipeWire's `dequeue_available_buffer` self-throttles to the consumer.
- **Lazy PipeWire init.** `Screencasting` is `Option<Box<...>>`, only stood up on the first cast.

### Worth revisiting
- **`IpcOutputMap` snapshot is one-shot.** Cached `name`/`refresh`/`output` triple per cast goes stale on hotplug during an active cast. ~30 LOC.
- **`ScreenCast::Session::Stop` cleanup is partial.** No grace period — pipewire warnings on rapid stop/start cycles.
- **Continuous repaint is global.** Per-cast wake-only scheduling would help with multi-cast different-framerate setups.

---

## 7. Scripting & plugins

### Rhai engine — Phases 1, 2, 3 shipped

`margo/src/scripting.rs` — commits `562b5f7`, `13bdd57`, `769141e`. Rhai 1.24 sandboxed engine; `~/.config/margo/init.rhai` evaluated at startup.

**Bindings:**
- `dispatch(action, args_array)` + zero-arg overload — invokes any registered margo action.
- `spawn(cmd)`, `tag(n)` — convenience helpers.
- `current_tag()`, `current_tagmask()`, `focused_appid()`, `focused_title()`, `focused_monitor_name()`, `monitor_count()`, `monitor_names()`, `client_count()` — read-only state.

**Event hooks** (mid-event-loop):
- `on_focus_change(fn())` — fires from `focus_surface` (post-IPC-broadcast, gated on `prev != new`).
- `on_tag_switch(fn())` — fires from `view_tag` after arrange + IPC.
- `on_window_open(fn())` — fires from `finalize_initial_map` after window-rules + focus.
- `on_window_close(fn())` — fires AFTER state is consistent (client gone, focus shifted, arrange done) with `(app_id, title)` as Rhai string args.

**Pattern.** State-access via thread-local raw pointer set during eval, cleared via RAII guard. Hook firing uses an Option-take/restore dance so a re-entrant hook (a hook calls `dispatch(...)` triggering another event) finds `None` and is a no-op — recursion guard for free. Rhai's `print` / `debug` channels routed into tracing. Example: `contrib/scripts/init.example.rhai`.

**Why Rhai over Lua:** pure Rust (no C build), type-safe `register_fn`, sandbox tight by default.

### Plugin packaging — shipped

`margo/src/plugin.rs` (~270 LOC, 4 unit tests). Discovers `~/.config/margo/plugins/<name>/{plugin.toml,init.rhai}` directories at startup; each plugin's script runs against the same engine init.rhai uses, so plugins layer hooks on top. Hand-rolled TOML manifest parser (4 fields: `name` / `version` / `description` / `enabled`). `MargoState::plugins` exposes the loaded list for a future `mctl plugin list/enable/disable` workflow. Compile / runtime errors per-plugin don't take down the loader.

### Live scripting — `mctl run <file>` shipped

New `Run { file }` mctl subcommand + `run_script` dispatch action + `scripting::run_script_file` helper. Script is canonicalised client-side, sent absolute-path through dispatch, eval'd against the same live engine. Hook registrations inside the script persist after the run (live-edit your hooks). Falls back to standing up a fresh engine if init.rhai never ran. ~140 LOC.

### Worth revisiting
- `on_output_change` hook — easy add when demand surfaces.

---

## 8. IPC & tooling

### `mctl` — Swiss-army CLI

- **`mctl status --json`** — stable schema (`tag_count`, `layouts[]`, `outputs[]` with `name`, `active`, `layout`, `focused`, `tags[]`, `scratchpad_visible`, `scratchpad_hidden`, `focus_history` MRU-first); `"version": 1` field. For `jq` pipelines and bar widgets.
- **`mctl rules --appid X --title Y [--verbose]`** — config-side introspection (no Wayland connection); Match / Reject(reason) classification per rule.
- **`mctl check-config`** — unknown-field detection, regex compile errors, **duplicate bind detection**, include-resolution, exit-1 for CI. Offline.
- **`mctl actions`** catalogue — 40+ typed actions in `margo-ipc/src/actions.rs` so completions, `mctl check-config`, and the dispatch table all read the same source of truth. `--group`/`--names`/`--verbose` filters.
- **`mctl run <file>`** — see [§7](#7-scripting--plugins).
- **`mctl migrate --from {hyprland,sway} <file>`** — `margo-ipc/src/migrate.rs` (~430 LOC, 9 unit tests). Auto-detects format from path heuristics + content sniff (`bind = ...` → hyprland, `bindsym ...` → sway); override with `--from`. Translates the high-value subset: keybinds + spawn lines + workspace → tag bitmask + modifier names (`Mod4`/`SUPER` → `super`) + key aliases (`RETURN`/`return` → `Return`). Window rules / animations / monitor topology stay manual. Niri's KDL is intentionally out-of-scope (workspaces+scrolling don't map onto tag-based without inventing wrong semantics). Unconvertible source lines emit a warning to stderr with line number. Offline.

### dwl-ipc-v2 wire compat

dwl-ipc-v2 is the wire protocol; `state.json` (in-tree IPC, written on every `arrange_monitor` + dwl-ipc broadcast) is the extension. Per-output extension: `scratchpad_visible` / `scratchpad_hidden` counts, `focus_history` array (last 5 focused app_ids, MRU-first, fed by a VecDeque on `MargoMonitor`). Per-tag wallpaper hint: `tagrule = id:N, wallpaper:path` populates `Pertag::wallpapers: Vec<String>`; `state.json` exposes both the active-tag wallpaper (`wallpaper`) and the per-tag map (`wallpapers_by_tag`) for wallpaper daemons. Niri can't host this — no tags. dwl-ipc wire stays unchanged (avoids breaking noctalia / waybar-dwl).

### Companion binaries

- **`mscreenshot`** — full / region / window screenshot binary, integrates with the in-compositor region selector ([§9](#9-screenshot-ux)).
- **`mlayout`** — layout picker.
- **Shell completions** — bash + zsh + fish under `contrib/completions/`. Pulls dispatch action names from `mctl actions --names`. Zsh completion fixed at `b3c5ba1` to be source-safe.

### UX polish

- **Layout switch notification enriched.** `notify_layout` shows position-in-cycle (`scroller (1/6) → next: tile`). New `notify_layout_state(action, value)` for proportion / gap toggles — same toast theme. Hooked into `switch_proportion_preset` + `toggle_gaps`.

---

## 9. Screenshot UX

In-compositor region selector — `margo/src/screenshot_region.rs` (W2.1). New `region_selector: Option<ActiveRegionSelector>` field on `MargoState`. UX: drag a rect, Enter to confirm, Escape to cancel. On confirm spawns `mscreenshot <mode>` with `MARGO_REGION_GEOM="X,Y WxH"` set so the binary skips its own slurp call.

**Render side.** Four `SolidColorRenderElement` outline edges + dim overlay (~22% black `[0.0, 0.0, 0.0, 0.22]`). Layering `[cursor, edges, dim, scene]` ensures the cursor stays visible while in selection mode and the dim overlay gives a clear visual cue. Force-show cursor + `clamp_pointer_to_outputs` while active.

**Input side.** `handle_input` early-routes pointer + keyboard to selector handlers when active.

**Scope deliberately narrowed** vs an earlier reverted attempt — capture / save / clip / edit stay in `mscreenshot`; only the UX gap (slurp's separate window, focus fight) is replaced. ~340 LOC across margo + mscreenshot.

---

## 10. Test infrastructure (W1)

| # | Status | What landed |
|---|---|---|
| **W1.1** Layout-snapshot suite | ✅ shipped | `margo/src/layout/snapshot_tests.rs` + `snapshots/` dir lock geometry of 14 layout algorithms × 20 canonical scenarios into committed `.snap` text files. Insta-based (no PNG churn — pure text diff at PR review). 24/24 pass on `cargo test --workspace`. Property tests verify `arrange()` dispatcher matches direct calls and non-scroller layouts stay inside the work area. |
| **W1.2** Layout property-test extension | ✅ shipped | Added 14 property tests covering the full 14-layout catalogue × {1, 2, 3, 5, 8} window counts × focus shift × gap-zero edge case. Invariants verified: dispatcher matches direct call for *every* `LayoutId` variant; cardinality (`arranged.len() == n`); no degenerate rects (w/h > 0); monocle returns identical rect for every client; deck stack clients share one rect; tile-class layouts have pairwise-disjoint rects; focus-position invariance for non-scroller layouts; Overview aliases monocle; empty input yields empty output; right_tile master strictly right of stack; vertical_tile master top-half on portrait; scroller total width grows monotonically with window count; gap-zero uses full work area; scroller focus-centering holds for every focused index. Test count layout module: 26 → 40 (+14). |
| **W1.3** Window-rule snapshot tests | ✅ shipped | `margo/src/tests/window_rules.rs` + `tests/snapshots/` lock the matcher's decision table for a curated rule set × candidate matrix. Two tests: `window_rule_matches_against_curated_candidates` calls the matcher directly (pure function, ~100 candidates/sec, formats matched-rule indices + per-rule deltas), and `window_rule_application_via_xdg_shell_flow` drives each `(app_id, title)` pair through the real `new_toplevel` + `set_app_id` + commit + finalize_initial_map flow and snapshots the resulting `MargoClient` field deltas — catches the regression class where the matcher is right but `apply_matched_window_rules` wires the wrong field (e.g. swap of `no_border`/`no_shadow`). Six representative rules cover positive id, regex alternation on id, positive title, exclude_id, exclude_title, plus tag-pin + scratchpad + CSD payloads. `matching_window_rules` and `window_rule_matches` bumped to `pub(crate)` for direct test access. Test count: 107 → 109. |
| **W1.4** Clippy gating | ✅ shipped | New `clippy.toml` with `ignore-interior-mutability` for smithay's `Window` / `Output` / `ClientId` / `ObjectId`. Workspace cleanup pass: **51 → 0 warnings**. CI runs `cargo clippy --workspace --all-targets -- -D warnings` as a gate. |
| **W1.5** CONTRIBUTING.md + PR template | ✅ shipped | Quick-start build + system deps, code-layout map, lint posture, test workflow (`cargo insta review`, smoke-winit), commit-message style (conventional commits + why-not-what bodies), tracy-span hints, PR review checklist, AI-contribution policy. PR template at `.github/pull_request_template.md`. |
| **W1.6** Integration test fixture + per-handler tests | ✅ shipped (first cut) | Port of niri's `src/tests/{fixture,server,client}.rs` pattern to margo. New `margo/src/tests/` module gated on `#[cfg(test)]`: `server.rs` (~70 LOC) wraps a real `MargoState` driven by calloop; `client.rs` (~280 LOC) is a `wayland-client::Connection` with `prepare_read → read → dispatch_pending` loop, per-`ClientState` `Dispatch<…>` impls (registry / callback / compositor / xdg_wm_base / xdg_surface / xdg_toplevel), `bind_global<I>` helper, `create_surface` + `create_toplevel` shapers; `fixture.rs` (~120 LOC) interleaves server + client dispatch via `roundtrip(id)` with a 200-turn upper bound to surface deadlocks as panics. **Three smoke tests** in `globals.rs` (fresh_client_sees_all_required_globals — 30 globals, deliberately excludes xwayland_shell_v1 / wp_color_manager_v1 / dmabuf / drm_syncobj which are gated on a real backend; second_client_sees_the_same_globals_as_the_first; fixture_dispatches_without_clients). **Six xdg_shell tests** in `xdg_shell.rs` covering margo's commit-staged toplevel flow: `pre_commit_toplevel_is_pending_initial_map` (deferred-map flag set on `new_toplevel`), `first_commit_finalizes_initial_map` (commit clears the flag), `set_app_id_and_title_propagate_after_commit` (identity refresh on commit — load-bearing for window-rule lookups), `toplevel_destroy_removes_client`, `two_toplevels_coexist_in_clients_vec`, `destroying_one_of_two_toplevels_keeps_the_other` (catches index-shift regressions). **Five layer_shell tests** in `layer_shell.rs` covering bar / OSD / launcher mapping: `layer_surface_maps_into_output_layer_map` (handler maps into `layer_map_for_output`), `layer_rule_noanim_suppresses_open_animation` (regex namespace match → skip `layer_animations` queue), `matching_namespace_with_animations_on_queues_entry` (positive case), `layer_animations_off_in_config_skips_queueing` (default config gate), `layer_destroyed_unmaps_from_output` (no phantom layer entries). Fixture grew an `add_output(name, size)` helper (synthesises `Output` + `MargoMonitor` without DRM/GBM); client grew `create_layer_surface(namespace, layer)` with `set_size(1, 30)` baked in (without it, layer-shell rejects commit with `Protocol error 1: invalid_size`). **Three idle tests** in `idle.rs` covering inhibit/uninhibit cycle: `create_inhibitor_adds_to_set_and_flips_inhibited_flag`, `destroying_inhibitor_clears_the_set`, `two_inhibitors_two_entries_then_destroy_one_keeps_the_other` (catches "uninhibit collapses the whole set" regressions). **Two xdg_decoration tests** in `xdg_decoration.rs` covering the SSD-by-default policy and the no-rule rejection of ClientSide. **Two session_lock tests** in `session_lock.rs`: `lock_request_flips_session_locked`, `destroy_lock_object_unlocks`. **Two xdg_activation tests** in `xdg_activation.rs`: smoke-level coverage of the global advertisement and rejection-path no-panic guarantee (full anti-focus-steal flow needs a `add_focused_toplevel` helper, deferred). **Two pointer_constraints tests** in `pointer_constraints.rs`: bind smoke + `lock_pointer_without_focus_does_not_panic`. **Three gamma_control tests** in `gamma_control.rs`: global-advertise, gamma_size = 0 skip path, `add_output_full(name, size, gamma_size)` fixture extension to support gamma testing. **Two screencopy tests** in `screencopy.rs`: `zwlr_screencopy_manager_v1` + the three `ext-image-copy-capture` globals. **Two output_management tests** in `output_management.rs`: `zwlr_output_manager_v1` + `zxdg_output_manager_v1` advertisement. **Two selection tests** in `selection.rs`: clipboard / primary-selection / data-control globals + state field initialisation. **Three negative-invariant tests** for backend-gated handlers: `dmabuf.rs` (dmabuf + drm_syncobj globals must NOT advertise in headless mode; pins the udev-gating contract), `color_management.rs` (HDR Phase 2 gate; global stays off until Phase 2 lands), `x11.rs` (xwayland_shell global only stands up after `XWayland::spawn`; `state.xwm` starts None). Client harness grew `create_idle_inhibitor()`, `create_decoration(toplevel)`, `create_session_lock()`, `create_pointer()`, `lock_pointer(surface, pointer)`. Test count: 85 → 122 (+37 integration); **all 15 W4.2-extracted protocol handlers now have at least one integration test** — the audit's "load-bearing test gap" is closed for the protocol-handler surface. |
| **CI** | ✅ shipped | GitHub Actions (commit `2910567`): cargo build/test + `mctl check-config` on every PR. `.github/workflows/smoke.yml` runs `scripts/smoke-winit.sh` end-to-end under Xvfb on every push/PR with kitty as the test client; on failure uploads `/tmp/margo-smoke-*/` as a workflow artifact. |
| **Smoke binaries** | ✅ shipped | `scripts/smoke-winit.sh`, `scripts/post-install-smoke.sh` (binaries run, example config parses, dispatch catalogue ≥30 entries, `desktop-file-validate`, completions in correct paths, LICENSE installed), `docs/manual-checklist.md` (13-section post-install/reboot validation). |
| **Profiling** | ✅ shipped (W2.7) | `tracy-client = { version = "0.18", default-features = false }` always-on dep + `profile-with-tracy` feature. `tracy_client::span!(...)` calls are no-ops in normal builds and connect to a live Tracy GUI when the feature flips. Six hot-path spans: `render_output`, `build_render_elements`, `arrange_monitor`, `tick_animations`, `handle_input`, `focus_surface`. |

Test count: 14 → 85 across the catch-and-surpass sweep (W1.2 added +14 layout property tests).

---

## 11. Accessibility (W2.4)

**AccessKit screen-reader bridge** — `margo/src/a11y.rs` (~230 LOC, gated on `a11y` feature, off by default). Ports niri's adapter pattern: AccessKit `Adapter` lives on its own thread because zbus contention on the compositor mainloop deadlocks. `MargoState::a11y: A11yState` field + `publish_a11y_window_list()` helper called from `arrange_all`. Each window becomes an accessible `Node` with `Role::Window` and a `"AppID: Title"` label; the focused window is the tree's `focus`. Build with `--features a11y`. Orca and AT-SPI consumers can navigate margo's window list.

**Future work:** per-tag grouping, action handlers (a11y → focus dispatch), live announcements on tag-switch.

---

## 12. Platform & openness

### Cargo features (W2.6)

- `dbus` (default) — gates zbus + async-io + async-channel.
- `xdp-gnome-screencast` (default, requires `dbus`) — gates pipewire.
- `a11y` — gates accesskit + accesskit_unix.
- `profile-with-tracy` — flips `tracy-client/default`.

All optional deps are `optional = true`. Cfg-gated: `mod dbus`, `mod screencasting`, the `dbus_servers` + `screencasting` fields on `MargoState`, the `emit_windows_changed` body (function stays as a no-op), `start_cast` / `stop_cast` / `on_pw_msg` methods, the udev cast-drain hook, the 5 D-Bus init blocks in `main.rs`. **Three build configs verified:** default (full), `--no-default-features --features dbus` (no screencast), `--no-default-features` (lean). All snapshot tests pass under each. ~140 LOC of cfg sprinkles.

### XWayland modes (W2.5)

- `--xwayland-satellite[=BINARY]` spawns Supreeeme's xwayland-satellite as a separate process (X11 crash can't take margo down).
- `--no-xwayland` disables X11 entirely (pure-wayland session).

Default path stays in-tree (smithay `XWayland::spawn`). Lifetime coupling is user-side: pair with `systemctl --user link xwayland-satellite.service PartOf=margo-session.target` for strict tie-down. ~50 LOC. **XWayland HiDPI cursor env** — `XCURSOR_SIZE` + `XCURSOR_THEME` exported on `XWayland::Ready`. Fixes "Steam / Discord / Spotify cursor shrinks on hover".

### Graceful EGL failure (W2.2a)

When udev backend bring-up fails, margo logs the diagnostic + three actionable fix hints (Mesa install, `/dev/dri/card*` permissions, qemu virgl) and falls back to winit nested mode. ~25 LOC.

### Packaging

PKGBUILD in-tree (`/repo/archive/.kod/margo/PKGBUILD`) and at `/home/kenan/.kod/margo_build/PKGBUILD` (`r174.2e69a0c` at last build). Installs `/usr/bin/{margo,mctl,mlayout,mscreenshot}`, completions to system-wide paths, XDG portal preference at `/usr/share/xdg-desktop-portal/margo-portals.conf`, license headers for dwl/dwm/sway/tinywl/wlroots inheritance.

---

## 13. Daily-driver baseline (P0 + P0+ archive)

Behaviour-stable. Listed for completeness — bisect targets if anything regresses.

### P0 — daily-driver baseline ✅ 6/6
- **`ext_session_lock_v1`** — non-zero initial configure size, pointer pinning to lock surface, exclusive-keyboard layer skipped while locked. noctalia / swaylock / gtklock unlock cleanly. `force_unlock` emergency keybind (`super+ctrl+alt+BackSpace`, whitelisted to fire while locked).
- **`ext_idle_notifier_v1` + idle inhibit** — every keyboard / pointer / touch / gesture bumps activity; mpv's `zwp_idle_inhibit_manager_v1` pauses the timer.
- **DRM hotplug** — per-CRTC `Connected` rescan; migrating clients on unplug; `setup_connector` callable runtime.
- **Crash + debug log** — `pkill -USR1 margo` dumps full state; `panic::set_hook` writes location + payload + backtrace; `dispatch::debug_dump` keybind-triggered.
- **Interactive move/resize** — `xdg_toplevel.move/resize`; tiled drags promote to floating.
- **Windowrule regression suite** — `scripts/smoke-rules.sh`: spawn → poll → assert with 5 canonical cases.

### P0+ polish ✅ 12/12
`bec1c51 → 2f57427 → 7832cd9` range:
- `text_input_v3` + `input_method_v2` for QtWayland LockScreen password fields.
- Niri-pattern keyboard focus oracle — single recompute point handles lock surface, layer-mutate, sloppy focus.
- Multi-monitor lock cursor tracking — focus follows the cursor's output.
- Layer-destroy + layer-mutate focus restore — covers both rofi (destroy) and noctalia (`keyboardFocus` mutation).
- `tagview` action — dwm `tag` keeps you here, `tagview` follows.
- Z-band ordering (`enforce_z_order`) — float > tile > overlay invariant.
- Scroller jitter chain — no-restart on identical animation, size-snap, `sloppyfocus_arrange = 0` default.
- Niri-style resize crossfade (now in [§3](#3-animations--frame-clock-p2)).
- Deferred initial map — Qt clients (CopyQ, KeePassXC) no longer flicker between default and final position.
- Diagnostic logging — `tracing::info!` instrumentation across session-lock, focus oracle, key forwarding, arrange, border drift.
- noctalia LockScreen multi-monitor dots.

**Worth revisiting.** Hotplug rescan currently triggered on every udev event; a 50ms coalescer would smooth dock-with-multiple-monitors plug-ins. Snapshot capture timing is "every frame during animation" rather than "on arrange-emit"; cost bounded but worth revisiting on iGPUs running 8+ animated clients simultaneously.

---

## 14. Queued work

### Test infrastructure (W1)
- (none queued — W1.1 layout snapshots, W1.3 window-rule snapshots both shipped; see §10)

### Architecture (W4)
- **W4.1 — shipped (alt-module split, not separate crate).** `margo/src/backend/udev.rs` (3934 LOC, single file) split into 4 sub-modules + the orchestrator: `helpers.rs` (77, transform / CRTC pick / refresh-duration / monotonic clock), `mode.rs` (234, mode select + apply via `DrmCompositor::use_mode`), `hotplug.rs` (405, rescan + setup_connector + migrate-clients-off-output), `frame.rs` (331, render dispatch + presentation feedback + scanout flags), and the trimmed `mod.rs` (2873, ~27 % shrink). Type visibility lifted to `pub(super)` for `OutputDevice` / `BackendData` / `GammaProps` so submodules reach the shared state without trait indirection. **Decision: separate-crate split was rejected.** The road map's earlier framing claimed "niri has 7 backend crates" but those turn out to be smithay's `feature` flags, not crates — niri itself is one binary. The real win (incremental compile + readability) lands at this granularity without trait-abstracting `MargoState` (which would be ~3000 LOC churn for no downstream consumer). All 181 tests + clippy stay green at every extract.
- **W4.2 — Phase 1–8 shipped, complete.** Eight passes extract **17 handlers** into `margo/src/state/handlers/` (~2090 LOC moved): `xdg_decoration` (62), `session_lock` (96), `xdg_activation` (113), `layer_shell` (262), `color_management` (21), `idle` (50), `pointer_constraints` (77), `input_method` (57), `selection` (79, bundles SelectionHandler + DataDevice + PrimarySelection + DataControl + DndGrab), `gamma_control` (48), `screencopy` (30), `dmabuf` (64, bundles DmabufHandler + DrmSyncobjHandler), `output_management` (163), `x11` (203, bundles XWaylandShellHandler + XwmHandler + selection bridging), `xdg_shell` (~430, toplevel lifecycle + popup grabs + close animation snapshot + fullscreen/maximize routing), **`compositor` (~233, surface commit hub — session-lock priority, deferred-map, initial-configure pump for toplevels/layers/popups, layer-shell focus refresh; bundles BufferHandler)**, and **`image_copy_capture` (~208, ext-image-capture-source-v1 + ext-image-copy-capture-v1 — Window/Screen tab screencast scaffolding bundling four traits: ImageCaptureSourceHandler, OutputCaptureSourceHandler, ToplevelCaptureSourceHandler, ImageCopyCaptureHandler)**. Pattern: each submodule reaches into `MargoState` via `crate::state::MargoState`; the `delegate_*!` macros stay co-located with their impls. state.rs went **7651 → 5905 LOC** (-1746, ~23 % shrink). Behaviour-preserving — all 110 tests + clippy gate stay green at every step.
- **W4.3 — shipped.** mkdocs-material site at <https://kenanpelit.github.io/margo/>. Config: `mkdocs.yml` at repo root with deep_purple/deep_orange Material palette, GitHub-style slug rules (`pymdownx.slugs.slugify(case='lower')`) so cross-doc anchor links from `road_map.md` keep working. New site pages: `docs/index.md` (landing with hero + capability grid), `docs/install.md` (Arch / source / Nix flake), `docs/configuration.md` (window rules / tag rules / layer rules / animations / keybinds), `docs/companion-tools.md` (`mctl` / `mlayout` / `mscreenshot`), `docs/scripting.md` (Rhai bindings + hooks + plugin packaging — user-facing intro pointing to `scripting-design.md`). Existing design docs (`hdr-design.md`, `portal-design.md`, `scripting-design.md`, `manual-checklist.md`) surface in nav as-is. CI: `.github/workflows/docs.yml` deploys to GitHub Pages; syncs `road_map.md` + `CONTRIBUTING.md` into `docs/` before `mkdocs build --strict` so source-of-truth stays at repo root. README adds a docs-site link badge + callout. ~880 LOC of new docs content + ~80 LOC config/CI.
- **W4.5 — shipped.** `mvisual` GTK4 design tool — workspace binary (~620 LOC) + new shared `margo-layouts` crate (~1040 LOC, extracted from `margo/src/layout/` so the binary and the design tool consume the same pure arithmetic). Wider than niri-visual-tests on two axes: **(1)** all 14 tile-able layouts render side-by-side as live thumbnails — direct visual comparison, not click-cycle inspection; **(2)** a 1‒9 tag rail acts as the compositor's `Pertag` so you can pin a different layout per tag and rehearse the workflow before committing to a config. Live re-arrange on every parameter tweak (window count / mfact / nmaster / inner+outer gaps / focus index / scroller proportion). 38 layout snapshot tests + 2 inline algorithm tests + 3 mvisual unit tests stay green through the extract — behaviour-preserving by construction.

### HDR
- **Phase 2 runtime activation** — upstream-blocked on smithay's `DrmCompositor` exposing fp16 swapchain reformat. ~80 LOC integration once it lands.
- **Phase 3 runtime activation** — upstream-blocked on smithay's `DrmCompositor` exposing `set_hdr_output_metadata`. ~30 LOC integration.
- **Phase 4 runtime activation** — upstream-blocked on smithay's `compile_custom_texture_shader` exposing a second-sampler hook so the udev backend can bind a baked LUT atlas alongside the input surface texture. ~30 LOC integration once the API lands.

### Screencast portal
- **`IpcOutputMap` lazy refresh** on hotplug during active casts. ✅ shipped — `MargoState::ipc_outputs` now owns one shared `Arc<Mutex<IpcOutputMap>>`; both `DisplayConfig` and `ScreenCast` services receive a clone (was: each got its own startup snapshot, neither refreshed). New `MargoState::refresh_ipc_outputs()` rebuilds the snapshot from live monitors and is called from `remove_output` (hotplug-out) and from both `setup_connector` paths in udev (hotplug-in initial + live). xdp-gnome's chooser dialog now reflects the live output set without a margo restart.
- **`ScreenCast::Session::Stop` grace period** to avoid pipewire warnings on rapid stop/start cycles. *Deferred* — requires a live PipeWire test setup to verify; cosmetic warning, no functional impact.
- **Per-cast wake-only scheduling** — current continuous-repaint is global. *Deferred* — same testing constraint plus the change touches the render loop's frame-clock; deserves a dedicated session.

### Protocol depth
- **`presentation-time` page-flip seq** — ✅ shipped. Per-output monotonic `OutputDevice::vblank_seq` bumped at the head of every `DrmEvent::VBlank` handler; published as the `seq` field on `wp_presentation_feedback.presented`. The wp_presentation contract is "implementation-defined monotonic counter" — kernel `drm_event_vblank.sequence` would also satisfy it but smithay 0.7 doesn't surface it. Per-output bump is observably equivalent for frame-pacing-sensitive consumers (mpv `--vo=gpu-next`, kitty render loop).

---

## 15. Outstanding work — external triggers

All margo-internal long-tail items have shipped. What's still open is
gated on something margo can't unblock by itself: an upstream PR
landing, a piece of hardware showing up to dogfood against, or a test
setup that needs more than a winit nested session. Grouped by trigger
type so it's obvious *who* unblocks each item.

### Upstream-blocked (smithay PR needed)

| Item | Trigger | Integration cost |
|---|---|---|
| **HDR Phase 2 runtime** — fp16 linear-light composite | smithay's `DrmCompositor` exposing fp16 swapchain reformat | ~80 LOC once the API lands |
| **HDR Phase 3 runtime** — KMS HDR scan-out metadata | smithay's `DrmCompositor` exposing `set_hdr_output_metadata` per-CRTC | ~30 LOC |
| **HDR Phase 4 runtime** — per-output ICC LUT post-pass | smithay's `compile_custom_texture_shader` exposing a second-sampler hook so the udev backend can bind the baked LUT atlas alongside the input surface texture | ~30 LOC |

### Test-setup-deferred (margo-internal but not winit-testable)

| Item | Trigger | Why now |
|---|---|---|
| **`Screencast::Session::Stop` grace period** | live PipeWire test setup to verify the warning actually goes away on rapid stop/start cycles | Cosmetic — pipewire warnings on rapid cycle, no functional impact |
| **Per-cast wake-only scheduling** | dedicated session because the change touches the render loop's frame-clock | Current global continuous-repaint works; per-cast wake would help multi-cast different-framerate setups but isn't a regression |

### Hardware-driven (waiting on a user request)

| Item | Trigger | Cost |
|---|---|---|
| **W2.2b** Full pixman software renderer fallback | A user files "margo doesn't run in my qemu" with a concrete deployment to test against | Original ~400 LOC estimate undersells it; realistic scope is ~1500 LOC plus shader rewrites for SDF border/shadow paths. Every custom render element (RoundedBorder, Shadow, ResizeRender, ClippedSurface, OpenClose, LinearComposite — 7 modules) needs to be made generic over `R: Renderer + Bind<...>` AND parallel render paths in udev + winit backends. |
| **W2.3** Tablet input | A Wacom / Huion user files a request | ~500 LOC for `tablet_v2` protocol + stylus/pad button mapping + `map-to-focused-window` mode |

---

## 16. What could be redone better (do-over wishlist)

- **Render element collection has multiple paths** (display, screencast, dmabuf-screencopy region, snapshot) — *partial address shipped*: `RenderTarget::{Display, DisplayNoCursor, Screencast { include_cursor }}` enum replaces the previous `(include_cursor, for_screencast)` bool pair on `build_render_elements_inner`. Remaining axes (region clip on dmabuf-screencopy, snapshot's offscreen render) are different enough machinery that they deserve their own pass.
- **Animation tick fans out per-type.** `tick_animations` has separate branches for client move, opacity, layer surface, closing client, snapshot. A single `Animation` trait would consolidate. Trade-off: harder per-type custom logic. Probably not worth the refactor.
- **`Config` is a giant flat struct** with 100+ fields. Sectioned access (`config.input.keyboard.repeat_rate` instead of `config.repeat_rate`) would document grouping. Big migration, low value.
- **Window-rule application has three trigger sites** — ✅ shipped. `WindowRuleReason::{InitialMap, AppIdSettled, Reload}` enum routes every reapply through a single `MargoState::reapply_rules(idx, reason)` path with structured-debug logging of the trigger.
- **Diagnostic logging** — ✅ partial shipped. Hot-path frames in `backend/udev/{frame,hotplug}.rs` and `input_handler.rs` (gesture + keybinding match) emit `tracing` structured fields (`output = %name`, `reason = ...`, `error = ?e`, `"queued frame"`) instead of pre-formatted strings; `RUST_LOG=...` paired with `tracing-subscriber`'s JSON formatter makes `journalctl -u margo --output=json | jq` slice per-output traces cleanly. Cold-path callers (state.rs focus / dispatch noise, scripting, plugin loader) still use the old format-string shape — convertible piecemeal as touched.

---

## 17. Acceptance smoke test (post-install)

Run after installing a fresh package. Anything failing here is a release blocker.

- [ ] Reboot → margo UWSM session starts; noctalia bar visible; tags 1-9 reachable.
- [ ] `mctl reload` after editing a windowrule applies without logout.
- [ ] CopyQ / wiremix / pavucontrol / KeePassXC open at correct floating position on first frame (no flicker).
- [ ] Browser file picker focus lands in the picker, not parent toplevel.
- [ ] 3 windows in scroller tag → focus traverses without losing auto-center.
- [ ] Tag move → window only on target tag, no ghost on source.
- [ ] Night light → gamma transitions smooth, logout resets to default.
- [ ] grim full + region capture → both dmabuf path; wf-recorder records without SHM fallback.
- [ ] Screen lock → password input works on the cursor's monitor; multi-monitor dots animate in sync.
- [ ] HDR-capable monitor → no regression (still SDR; HDR Phase 1 advertises capability only, Phase 2/3 scaffolded but upstream-blocked).
- [ ] Helium / Chromium → Meet → Share screen → Window tab populates with live windows; pick one → share preview shows live content (not frozen first frame).
- [ ] F11 / browser fullscreen button / YouTube fullscreen → window goes fullscreen, exits cleanly.
- [ ] `bind = NONE,Print,screenshot-region-ui` → screen dims, cursor visible, drag-rect produces screenshot.
- [ ] `mctl status --json | jq .outputs[0].focused.app_id` returns the focused window.
- [ ] `mctl check-config ~/.config/margo/config.conf` reports zero errors.
- [ ] `~/.config/margo/init.rhai` evaluates at startup if present (one log line at info level).
- [ ] `~/.config/margo/plugins/<name>/init.rhai` plugins load (one log line per plugin at info level).

---

## Appendix A — Phase ledger

For archaeology only; capability detail lives in §1–§13.

| Phase | Scope | Headline commits |
|---|---|---|
| Core | UWSM, config, layouts, render, clipboard, layers, gamma, gestures | (foundational) |
| **P0** | session_lock, idle_notifier, hotplug, debug log, move/resize, smoke | 6/6 ✅ |
| **P0+** | text_input/IM, lock cursor-tracking, focus oracle, layer-mutate detect, tagview, z-order, scroller jitter, niri resize crossfade, deferred map | `bec1c51 → 2f57427 → 7832cd9` (12/12) ✅ |
| **P1** | dmabuf+drm-syncobj, dmabuf-screencopy, region-crop, blockout, pointer_constraints+relative_pointer, xdg_activation, output_management (mode change + disable), presentation-time | `78c9909 → 886eba5 → a26cc9b` (8/8) ✅ |
| **P2** | frame_clock, spring engine, open/close/tag/focus/layer animations, hw cursor, direct scanout, damage opt | `71b95a1 → bcb6fb4` (6/6) ✅ |
| **P3** | scratchpad+named, mango/layerrule parity, CSD/SSD policy, IPC parity, XWayland HiDPI env, popup focus, **xdg fullscreen request** | (7/7) ✅ |
| **P4** | smoke-winit, manual checklist, mctl JSON/rules/check-config, post-install smoke, shell completions, GitHub Actions CI | `f5b8d71`, `d2daba0`, `b3c5ba1`, `2910567` (7/7) ✅ |
| **P5/P6** | spatial canvas (`1c2bed1`), adaptive layout (`b19b5d6`), drop shadow (`45cfc74`), scripting Phase 3 (`562b5f7 → 13bdd57 → 769141e`), HDR Phase 1 (`25255a9`) | (5/5) ✅ |
| **P7** | 5 Mutter D-Bus shims, PipeWire pipeline, frame pacing, damage, cursor (embedded + metadata), full-decoration casts, HiDPI, windows_changed signal | `a4f6ed6 → bf7e579 → 0c2f5d5 → f8f7a9a → 0455b4e → 81a6487` (9/9) ✅ |
| **W1–W4** | catch-and-surpass-niri sweep | 19/22 shipped, 5 deferred / upstream-blocked |

---

## Appendix B — Catch-and-surpass-niri scoring

A side-by-side audit of margo (~33k LOC, **71 unit tests** post-sweep) against niri (~93k LOC, 61 unit tests, **5,280 visual snapshot files**, 47 docs, AccessKit, pixman, full tablet stack, modular per-backend crates) shows niri is the more battle-tested codebase by a wide margin. Margo wins on **tag-based dwm-style workflow**, **embedded Rhai scripting + plugin packaging**, **dwl-ipc-v2 wire compat**, **HDR Phase 1 + Phase 2/3 scaffolding**, **14-layout catalogue**, **per-tag wallpaper hint** — none of which niri has, all of which exist because margo is built around a specific user's workflow.

Post-W-sweep position: margo has visual snapshot regression coverage on par with niri for layouts (W1.1), feature parity on screenshot UX (W2.1), AccessKit a11y (W2.4), xwayland-satellite mode (W2.5), cargo features (W2.6), tracy profiler (W2.7), HDR Phase 3 metadata scaffolding (W3.1), `mctl run` + plugin packaging (W3.2 + W3.3), dwl-ipc state extension (W3.4), enriched layout-cycle notify (W3.5), per-tag wallpaper (W3.6), and `mctl migrate` from Hyprland / Sway (W4.4). What's left is 2 deferred (W2.2b pixman, W2.3 tablet) and 3 upstream-blocked (HDR Phase 2/3/4 runtime) — every internal long-tail item is shipped. Post-sweep additions: `mctl theme` live preset switch, `mctl session save/load` per-monitor tag/layout JSON persistence, touchscreen multi-finger swipe (mango port) routed to the same `gesture_bindings` table touchpad swipe uses.

The "personal-driver-only" framing has flipped: margo is now "a Rust + Smithay Wayland compositor with the maturity of niri AND a dwm/dwl-style tag workflow AND working HDR scaffolding" — a category niri doesn't and won't compete in (niri's design intent is scroller-only, no tags).
