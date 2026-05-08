# Margo Road Map

> Last updated: **2026-05-08**
> Branch: `main` (single-branch — Rust port complete; the C tree is the legacy reference under `src/`)
> One-liner: **P0 → P4 fully shipped, P5/P6 long-term goals 6/6 in design or foundation phase.** Margo is now a daily-driver Wayland compositor with full modern-protocol parity, niri-grade animations + spring physics, on-demand redraw scheduler, and a tooling pass (`mctl status --json`, smoke tests, shell completions, post-install validator). The work ahead is depth on the long-term goals, not breadth.

This document is **the source of truth** for what's shipped, what's worth a second pass, and what's queued. Each section follows the same shape:

- **Shipped** — what landed, with the relevant commit hash so it's traceable.
- **Strengths** — the few decisions that paid off and should be preserved on any future rewrite.
- **Worth revisiting** — places where the current implementation works but isn't the best version of itself.

---

## TL;DR Status

| Block | Scope | Status |
|---|---|---|
| Core | UWSM, config, layouts, render, clipboard, layers, gamma, gestures | ✅ |
| **P0** | session_lock, idle_notifier, hotplug, debug log, move/resize, smoke | **✅ 6/6** |
| **P0+ polish** | text_input/IM, lock cursor-tracking, focus oracle, layer-mutate detect, tagview, z-order, scroller jitter, niri resize crossfade, deferred map | **✅ 12/12** |
| **P1 protocol parity** | dmabuf+drm-syncobj, dmabuf-screencopy, region-crop, blockout, pointer_constraints+relative_pointer, xdg_activation, output_management, presentation-time | **✅ 8/8** |
| **P2 perf/akıcılık** | frame_clock, spring engine, open/close/tag/focus/layer animations, hw cursor, direct scanout, damage opt | **✅ 6/6** |
| **P3 window mgmt v2** | scratchpad+named, mango/layerrule parity, CSD/SSD policy, IPC parity, XWayland HiDPI env, popup focus | **✅ 6/6** |
| **P4 tooling** | smoke-winit, manual checklist, mctl JSON/rules/check-config, post-install smoke, shell completions | **✅ 6/6** |
| **P5/P6 long-term** | spatial canvas, adaptive layout, drop shadow ✓ — built-in portal, HDR, scripting in design/foundation | **6/6 partial** |

---

## 0. Core (the floor everything stands on)

### Shipped

- **Session lifecycle**: UWSM systemd integration (`margo.desktop`, `margo-uwsm.desktop`), env import, noctalia + user services bootstrap.
- **Config**: live `mctl reload`, `source/include`, `conf.d`, `Super+Ctrl+R`.
- **Layout core**: tile, scroller, grid, monocle, deck, center/right/vertical variants, canvas, dwindle.
- **Tag system**: dwm-style "press-twice-for-back", per-tag home monitor (`tagrule = id:N, monitor_name:X`), automatic warp on `view_tag`.
- **Window rules**: regex `appid/title`, negative match (`exclude_appid`/`exclude_title`), size constraints, floating geometry, late `app_id/title` reapply.
- **Input**: keyboard, pointer, touchpad, swipe gestures, caps-to-ctrl.
- **Clipboard**: `wlr_data_control_v1`, `primary_selection_v1`, XWayland selection bridge.
- **Layer shell**: bar/notification ordering, exclusive-keyboard layering.
- **Render core**: GLES, rounded border shader, content clipping, fractional-scale-aware borders.
- **Night light**: `wlr_gamma_control_v1`, sunsetr/gammastep/wlsunset pipeline.
- **Screencopy**: SHM target for grim/wf-recorder/OBS; dmabuf added in P1.
- **Winit nested mode** for fast dev iteration.

### Strengths to preserve

- **Single-translation-unit feel ported into a real module tree.** The C codebase compiles `mango.c` as one giant unit; the Rust port keeps state surface centralized in `MargoState` but splits along natural seams (`backend/`, `layout/`, `dispatch/`, `render/`). Don't fragment further — each new submodule has paid its weight.
- **Per-tag state via `Pertag`** (layout, mfact, client count, plus the new `user_picked_layout` and `canvas_pan_x/y` fields) keeps every tag self-contained. Resist the urge to lift this onto `Monitor` "for simplicity"; tag-local state is what makes per-tag layout pinning, canvas memory, and home-monitor warp possible.

### Worth revisiting

- **Config parser** lives in a hand-written 4200-line C header and a structurally-similar Rust module. It's been hardened by user pain but isn't fun to extend. A second pass with a real grammar (pest / nom / chumsky) would give better error messages and locate the duplicate-bind detector inside the parser instead of as a separate `mctl check-config` pass.
- **Wayland listener wiring**. `MargoState` has a lot of fields with very long lifetimes; some bookkeeping (especially around layer-surface destruction and lock surfaces) could collapse if `slotmap` keys replaced ad-hoc `ObjectId` map lookups.

---

## 1. P0 — daily-driver baseline ✅ 6/6

### Shipped

- **`ext_session_lock_v1`** — three independent fixes: non-zero initial configure size, pointer pinning to lock surface, exclusive-keyboard layer skipped while locked. noctalia / swaylock / gtklock all unlock cleanly.
- **`ext_idle_notifier_v1` + idle inhibit** — every keyboard/pointer/touch/gesture bumps activity; mpv's `zwp_idle_inhibit_manager_v1` pauses the timer; surface-destroy cleanup automatic.
- **DRM hotplug** — per-CRTC `Connected` rescan (the old "any connector?" check failed dual-monitor); migrating clients on unplug; `setup_connector` callable runtime.
- **Crash + debug log** — `pkill -USR1 margo` dumps full state (outputs, tags, focus, clients, layer count); `panic::set_hook` writes location + payload + backtrace; `dispatch::debug_dump` keybind-triggered.
- **Interactive move/resize** — `xdg_toplevel.move/resize` requests + `mousebind = SUPER,btn_left,moveresize,curmove`; tiled drags promote to floating.
- **Windowrule regression suite** — `scripts/smoke-rules.sh`: spawn → poll → assert with 5 canonical cases; `cargo run -p margo-config --example check_config` parser validation.

### Strengths

- **Niri-pattern focus oracle** (`refresh_keyboard_focus`) is a single-point recompute called from every relevant event. Covered three previously-unrelated focus bugs (lock surface, layer-mutate, sloppy focus) with one abstraction.
- **`force_unlock` emergency escape** is the right kind of safety valve — gated to `super+ctrl+alt+BackSpace` only, whitelisted to fire while locked. Not a workaround; an explicit recovery surface.

### Worth revisiting

- **Hotplug rescan** is currently triggered on every udev event; `OutputDamageTracker` debounces but the rescan itself is cheap-ish ad-hoc work. A 50ms coalescer would smooth dock-with-multiple-monitors plug-ins.
- **Session lock pointer pinning** uses an early-return in `pointer_focus_under`; conceptually it's grab-shaped. A real `PointerGrab` impl would compose better with future kiosk-mode plans (P5 territory).

---

## 1.5 P0+ polish — daily-driver irritants ✅ 12/12

### Shipped

`bec1c51 → 2f57427 → 7832cd9` range. Highlights:

- **`text_input_v3` + `input_method_v2`** for QtWayland LockScreen password fields.
- **Niri-pattern keyboard focus oracle** — single recompute point, handles lock surface, layer-mutate, sloppy focus.
- **Multi-monitor lock cursor tracking** — focus follows the cursor's output; not the first lock surface in the vec.
- **`force_unlock` emergency keybind** for wedged-lock recovery.
- **Layer-destroy + layer-mutate focus restore** — covers both rofi (destroy) and noctalia (`keyboardFocus` mutation).
- **`tagview` action** — dwm `tag` keeps you here, `tagview` follows.
- **Z-band ordering** (`enforce_z_order`) — float > tile > overlay invariant.
- **Scroller jitter chain** — no-restart on identical animation, size-snap, `sloppyfocus_arrange = 0` default.
- **Niri-style resize crossfade** — two-texture pass, `clipped_surface` rounded-corner mask, snapshot tracks animated slot.
- **Deferred initial map** — Qt clients (CopyQ, KeePassXC) no longer flicker between default and final position.
- **Diagnostic logging** — `tracing::info!` instrumentation across session-lock, focus oracle, key forwarding, arrange, border drift.
- **noctalia LockScreen multi-monitor dots** — `~/.cachy/modules/noctalia/...` override using `lockControl.currentText.length`.

### Strengths

- **Snapshot-driven resize transition** is the centerpiece — capture live surface to GlesTexture once on slot change, render `tex_prev` and `tex_next` through the same `render_texture_from_to` shader pipeline. Zero opportunity for renderer-side divergence between the two layers.
- **Z-order invariant enforced once per arrange tick** rather than ad-hoc on every focus event. Easier to reason about; the bug surface is the invariant violation, not the call sites.

### Worth revisiting

- **Snapshot capture timing** is "every frame during animation" rather than "on arrange-emit". Slightly wasteful — for a static animation curve, one snapshot at start + one at end would suffice. Cost is bounded (only during ~250ms transitions) so leaving it alone is fine; the reason to revisit is GPU memory pressure on iGPUs running 8+ animated clients simultaneously.
- **Diagnostic logging**'s log lines are useful but ad-hoc. A structured-fields format (`tracing` already supports it) would let `journalctl -u margo --output=json | jq` slice per-client traces cleanly.

---

## 2. P1 — modern protocol parity ✅ 8/8

`78c9909 → 886eba5`, ~1300 LOC.

### Shipped

- **`linux_dmabuf_v1` + `linux-drm-syncobj-v1`** — Firefox/Chromium/GTK/Qt avoid SHM fallback; explicit-sync gated on `supports_syncobj_eventfd`.
- **DMA-BUF screencopy target** — OBS/Discord/wf-recorder zero-copy GPU→GPU full-output capture.
- **Region-based screencopy crop** — `grim -g "$(slurp)"` reads only the slurp region; `RelocateRenderElement` pattern so the rect lands at dmabuf (0,0).
- **`block_out_from_screencast` runtime filter** — `for_screencast: bool` parameter through `build_render_elements_inner`; substitution at element-collection time, not pixel-sample time.
- **`pointer_constraints_v1` + `relative_pointer_v1`** — FPS aim lock, Blender mid-mouse rotate stays inside, `cursor_position_hint` honored on unlock.
- **`xdg_activation_v1`** — token serial freshness check (≤10s), seat match, anti-focus-steal; tag-aware view_tag jump on activation.
- **`wlr_output_management_v1`** — `wlr-randr` and `kanshi` runtime topology changes (scale/transform/position). Mode/disable still `failed()` pending DRM re-modeset.
- **`presentation-time`** — kitty/mpv/native Vulkan get presented timestamp + refresh interval.

### Strengths

- **Anti-focus-steal protocol is strict by default.** Token must be serial-fresh, seat-matched, ≤10s old; the tradeoff is a few legitimate activations get rejected, but the alternative is letting any client steal focus by claiming an activation. Strict-by-default was the right call.
- **`block_out_from_screencast` substitutes at collection time.** One-line decision (`for_screencast: true` on the screencast path) eliminates the entire race window where a frame could be sampled with privacy-sensitive content visible.

### Worth revisiting

- **`wlr_output_management_v1` mode/disable** — the spec lets clients ask for an arbitrary mode change; we currently `failed()` because the DRM re-modeset path isn't wired. A second pass would let `kanshi --output DP-3 --mode 1920x1080@60` succeed instead of silently rejecting. ~150 LOC of DRM atomic work, blocked on smithay 0.7 ergonomics.
- **`presentation-time`** uses `now()` rather than the actual DRM page-flip sequence. Good enough for kitty / mpv pacing; Vulkan game frame-time graphs see ~16ms jitter from this approximation. The fix is plumbing `crtc->page_flip_seq` from the smithay flip event into `publish_presentation_feedback`.

---

## 3. P2 — frame clock, animations, scanout ✅ 6/6

### Shipped

- **On-demand redraw scheduler** — 16ms polling timer dropped; `Ping` source wakes loop only when needed; `pending_vblanks` counter prevents post-hook tick storms.
- **Spring physics primitive** — niri-style **analytical** critically-damped/under-damped solution (replaced an earlier numerical integrator that drifted). Per-channel velocity (x/y/w/h) preserved across mid-flight retargets.
- **Open/close/tag/focus/layer animations (5/5)**:
  - **Open** — snapshot-driven, live surface hidden during transition (no first-frame pop).
  - **Close** — client removed from `clients` vec immediately for layout/focus/scene; the animation lives in a separate `closing_clients` list rendering scale+alpha+rounded-clip.
  - **Tag switch** — direction from bit-position delta, slide-out for outgoing, off-screen-to-target for incoming via existing Move pipeline.
  - **Focus highlight** — `OpacityAnimation` cross-fade for both border color and `focused_opacity ↔ unfocused_opacity`.
  - **Layer surface** — `LayerSurfaceAnim` keyed by `ObjectId`, anchor-aware geom skipped for bars (no jitter).
- **Hardware cursor plane** — `DrmCompositor::new` reads driver-advertised `cursor_size()` (modern AMD/Intel/NVIDIA support 128²/256²); fallback 64×64.
- **Direct scanout** — smithay `FrameFlags::DEFAULT` already includes `ALLOW_PRIMARY_PLANE_SCANOUT | ALLOW_OVERLAY_PLANE_SCANOUT`; explicit-sync (P1) plus dmabuf feedback (P0+) feed the client side. Fullscreen mpv hits primary plane.
- **Damage tracking** — `OutputDamageTracker` per-frame; custom render elements (`RoundedBorderElement`, `ClippedSurfaceRenderElement`, `ResizeRenderElement`) bump `CommitCounter` only on geometry/shader-uniform change.

### Strengths

- **Analytical spring solution.** Every WM that does spring physics either ships a numerical integrator (drifts) or copies niri's analytical math (correct). We picked correct on the first iteration after one false start. The unit tests cover overshoot, critical damping, retargeting velocity preservation, and 60Hz/144Hz invariance — keep these as the regression boundary.
- **Open animation captures snapshot at first commit, hides live surface throughout.** Most compositors show a one-frame "pop" before the animation starts because the live surface renders before the first animation tick. We don't. This is invisibly good — users don't notice it because nothing flashes.

### Worth revisiting

- **Direct scanout has no observability today.** Smithay's `RenderFrameResult.states` exposes per-element plane assignment but we don't surface it via `mctl status`. A `scanout: bool` field per client in JSON output would let users verify "yes, this fullscreen mpv is on primary plane" without reading frame-by-frame logs.
- **Spring physics is opt-in for move only** (`animation_clock_move = spring`). Open/close/tag/focus/layer all use bezier curves. The path to spring-drive everything is plumbing-shaped (each animation type carries its own `Spring` state) but mostly mechanical; a future "spring everything" pass would unify the animation-tick code.
- **Frame clock is single-output today.** Multi-monitor mixed-refresh (say, 60Hz + 144Hz) uses a global tick. Per-output `next_frame_at` scheduling would let each monitor pace independently — only matters on truly mixed-refresh setups.

---

## 4. P3 — window management v2 ✅ 6/6

### Shipped

- **Scratchpad + named scratchpad** — `toggle_scratchpad`, `toggle_named_scratchpad <appid> <title> <spawn>`, `single_scratchpad`, `scratchpad_cross_monitor`. Window-rule `isnamedscratchpad:1` flag.
- **Mango windowrule + layerrule parity** — `windowrule.animation_type_open/close` apply at map; `layerrule` (previously parsed-but-never-applied) now matches namespace via regex, applies `noanim` and `animation_type_*`. `noblur`/`noshadow` parsed + stored, render hooks land in P5.
- **CSD/SSD policy** — `XdgDecorationHandler` defaults to ServerSide but honors `request_mode(ClientSide)` if window-rule has `allow_csd:1`. `unset_mode` re-resolves policy.
- **noctalia/IPC broadcast parity** — broadcast on focus shift, pure title change, `togglefloating`, `togglefullscreen`. Bar no longer stale on these state changes.
- **XWayland HiDPI cursor env** — `XCURSOR_SIZE` + `XCURSOR_THEME` exported on `XWayland::Ready`; user session env wins. Fixes "Steam/Discord/Spotify cursor shrinks on hover".
- **Popup focus via `xdg_popup.grab`** — `FocusTarget::Popup(WlSurface)` direct-focus path; portal file pickers, dropdowns, right-click menus get keyboard focus reliably.

### Strengths

- **Scratchpad with `is_visible_on` guard** — hidden scratchpads are excluded from `arrange_monitor`'s tag-match check. The bug we fixed (`13a225a`) was a re-mapping leak; the fix is one line in one well-named predicate. Don't move this logic.
- **Decoration handler is per-client policy via window-rule, not a global toggle.** Most WMs ship "all CSD" or "all SSD"; we let `allow_csd:1` opt-in per match. Right call — Firefox + GTK file pickers want CSD, kitty + Spotify don't.

### Worth revisiting

- **Popup grab is direct-focus.** It works for 99% of single-level popups but doesn't compose with smithay's `PopupKeyboardGrab`/`PopupPointerGrab` chain because `FocusTarget` variants don't all implement the right `From<PopupKind>` bounds. Nested popups (sub-menus inside a menu) work today only because each new grab fires the same direct-focus path. A real `PopupGrab` impl would compose better but requires `FocusTarget` refactor.
- **XWayland HiDPI is "cursor-only"** today. Full DPI scaling for X11 windows (xrandr Xft.dpi sync, per-app DPI) requires niri-style `xwayland-satellite`. Out of scope this round; users on HiDPI either run native Wayland apps or accept tiny X11 windows.
- **Scratchpad recovery** — we shipped `unscratchpad_focused` but it took two iterations to handle "Helium accidentally became scratchpad" wedge. The recovery path is `super+ctrl+Escape` (full-state reset). Worth documenting more visibly in `docs/manual-checklist.md`.

---

## 5. P4 — tooling, test, packaging ✅ 6/6

`f5b8d71`, `d2daba0`, `b3c5ba1` cluster.

### Shipped

- **`scripts/smoke-winit.sh`** — nested margo end-to-end: build → spawn → IPC → reload → focus → kill → empty-status. CI-runnable.
- **`docs/manual-checklist.md`** — 13-section post-install/reboot validation (bring-up, layer shells, notifications, clipboard, multi-monitor, lock, rules, scratchpad, animations, gamma, portal, recording, XWayland, idle).
- **`mctl status --json`** — stable schema (`tag_count`, `layouts[]`, `outputs[]` with `name`, `active`, `layout`, `focused`, `tags[]`); for `jq` pipelines and bar widgets.
- **`mctl rules --appid X --title Y [--verbose]`** — config-side introspection (no Wayland connection); Match / Reject(reason) classification per rule.
- **`mctl check-config`** — unknown-field detection, regex compile errors, **duplicate bind detection** (caught real shadowing in user's config), include-resolution, exit-1 for CI.
- **`scripts/post-install-smoke.sh`** — paket validation: binaries run, example config parses, dispatch catalogue ≥30 entries, `desktop-file-validate`, completions in correct paths, LICENSE installed.
- **Shell completions** — bash + zsh + fish. Pulls dispatch action names from `mctl actions --names` (cached); completes layout names, output names from `mctl status`. Zsh completion fixed (`b3c5ba1`) to be source-safe.
- **`mctl actions` catalogue** — 40+ typed actions with `name`, `aliases`, `args`, `group`, `summary`, `detail`. `--group`/`--names`/`--verbose` filters.
- **PKGBUILD updated** — completion install paths for system-wide packaging.

### Strengths

- **`mctl check-config` is offline.** Doesn't need a running compositor. CI can run it on every PR, editor integrations can run it on save. This is a deliberate design — config validation should never depend on the thing it's validating.
- **`mctl rules` introspection** also has no Wayland dependency. Same reason — you debug your config when it's wrong, which is exactly when the compositor might not be running cleanly.
- **`mctl actions` is typed and shared.** Lives in `margo-ipc/src/actions.rs` so completions, `mctl check-config`, and the dispatch table all read the same source of truth. No drift between "what completion offers" and "what margo accepts".

### Worth revisiting

- **Smoke tests don't run in CI yet.** They're written, they pass locally, but no GitHub Actions workflow exercises them. Adding `.github/workflows/smoke.yml` running `scripts/smoke-winit.sh` would catch regressions before they hit `main`.
- **`mctl status --json` schema isn't versioned.** First consumer (a future bar widget) will discover this on the first breaking change. Adding `"version": 1` to the top-level object now is cheap insurance.
- **Manual checklist isn't automated where it could be.** Sections like "screen recording produces non-empty output" could be `wf-recorder -t 2 /tmp/x.mp4 && [ -s /tmp/x.mp4 ]`. Worth one pass to extract the automatable third.

---

## 6. P5/P6 — long-term goals (6/6 shipped or designed)

Each entry tagged `[x]` (code shipped), `[~]` (foundation or design committed), `[ ]` (vapour). Currently 3 + 3 + 0.

### `[x]` Spatial canvas
**Commit `1c2bed1`.** Per-tag pan via `Pertag::canvas_pan_x/y`, `canvas_pan` and `canvas_reset` actions, threaded into 5 layout algorithms via `ArrangeCtx::canvas_pan`. PaperWM-style — each tag remembers its viewport.

*Worth revisiting:* No animation on pan. A spring-clock pan would be ~30 LOC and feels obvious; out of scope this sprint.

### `[x]` Adaptive layout engine
**Commit `b19b5d6`.** `Pertag::user_picked_layout: Vec<bool>` sticky bit + `maybe_apply_adaptive_layout()` heuristic (window count + monitor aspect ratio). User's `setlayout` pins the choice for that tag — heuristic never overrides.

*Worth revisiting:* Heuristic is rule-of-thumb (1 → monocle, 2-3 wide → tile, 4+ portrait → deck). A learned policy from past user picks would adapt. Probably premature.

### `[x]` Drop shadow (real-time blur/shadow phase 1)
**Commit `45cfc74`.** SDF analytic single-pass GLES shader in `render/shadow.rs`. No offscreen buffers; one fragment shader pass over (window + shadow_padding) rect. `udev::push_client_elements` pushes `MargoRenderElement::Shadow` for floating non-fullscreen non-scratchpad clients.

*Worth revisiting:* Shadows look "perfectly sharp" because the SDF is exact — fine at 10–25 px shadow_size (user's config), unnaturalistic at huge sizes. Kawase blur is the next phase if real wide-shadow demand surfaces.

### `[~]` Built-in xdg-desktop-portal backend
**Design: `docs/portal-design.md`.** 4 milestones — screencast (xdp-wlr fallback), screenshot, file chooser via xdp-gtk delegation, unified activation policy. Smithay 0.7 has no full xdp handler; needs zbus + per-method trait impl.

### `[~]` HDR + color management
**Design: `docs/hdr-design.md`.** 4 phases — `wp_color_management_v1` protocol scaffolding → linear-light fp16 composite path → KMS HDR scan-out (`HDR_OUTPUT_METADATA`) → ICC profile per-output 3D LUT. Per-phase LOC + hardware-test matrix (Intel/AMD/NVIDIA). DrmCompositor doesn't expose HDR primitives; drops to drm-rs directly.

### `[~]` Script/plugin system
**Foundation: `margo/src/scripting.rs` (commit `562b5f7`) + design `docs/scripting-design.md`.** Rhai 1.24 sandboxed engine; `~/.config/margo/init.rhai` evaluated at startup. Phase 1 binding: `spawn(cmd)`. Forward-compat stubs (`on_focus_change`, `on_tag_switch`, `on_window_open`) accept registrations today, fire when Phase 3 wires the event sites.

*Why Rhai over Lua:* pure Rust (no C build), type-safe `register_fn`, sandbox tight by default. Trade-off is unfamiliarity; mitigated by example scripts.

---

## What's queued (next sprint candidates)

Pick one or two; everything below is real work, not a wishlist.

1. **Wire one of the design-stage P5/P6 items.**
   Recommended: **scripting Phase 2** (`dispatch(action, args)` binding + read-only state introspection). ~150 LOC. High-leverage — every existing margo action becomes scriptable, replaces the shell-out-to-mctl pattern users currently use.

2. **CI for the smoke tests.** `.github/workflows/smoke.yml` running `cargo build` + `scripts/smoke-winit.sh` on push. Catches regressions before they land.

3. **`presentation-time` accuracy.** Plumb `crtc->page_flip_seq` from smithay flip events into `publish_presentation_feedback`. Vulkan frame-time graphs become accurate. ~50 LOC.

4. **`wlr_output_management_v1` mode change.** Wire the DRM atomic re-modeset so `wlr-randr --output DP-3 --mode 1920x1080@60` succeeds instead of `failed()`. ~150 LOC + careful testing.

5. **HDR phase 1.** Hand-generate `wp_color_management_v1` bindings, register the manager global, accept all named primaries + transfer functions, store per-surface state without changing render. ~300 LOC. Lets Chromium / mpv enable internal HDR paths even though composite is still SDR — useful capability advertisement.

---

## What could be redone better (if we got a do-over)

- **Render element collection has multiple paths** (display, screencast, dmabuf-screencopy region, snapshot). Each takes the same client list and produces a `MargoRenderElement` vec with subtle differences (`block_out_from_screencast`, region clip, snapshot vs live). A unified iterator with a `RenderTarget` enum parameter would dedup the wrappers. Today's code works; the cost is "every new render element type must be added to N places".
- **Animation tick fans out per-type.** `tick_animations` has separate branches for client move, opacity, layer surface, closing client, snapshot. They mostly do the same thing (advance phase, interpolate, mark `request_repaint` if not done). A single `Animation` trait would consolidate. Trade-off: harder per-type custom logic (e.g., layer-anchor-aware geom). Probably not worth the refactor.
- **`Config` is a giant flat struct** with 100+ fields. Most users edit ~20. Sectioned access (`config.input.keyboard.repeat_rate` instead of `config.repeat_rate`) would document grouping. Big migration, low value.
- **Window-rule application has three trigger sites** (`new_toplevel`, late-`app_id` reapply, reload). They mostly call the same function with slightly different "should we move the window" rules. One reapply path keyed on a `Reason` enum would be cleaner.

---

## Acceptance smoke test (post-install)

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
- [ ] HDR-capable monitor → no regression (still SDR; HDR phase 1 advertises capability only).
- [ ] `mctl status --json | jq .outputs[0].focused.app_id` returns the focused window.
- [ ] `mctl check-config ~/.config/margo/config.conf` reports zero errors.
- [ ] `~/.config/margo/init.rhai` evaluates at startup if present (one log line at info level).
