# Margo Road Map

> Last updated: **2026-05-09** (post-screencast-portal Phase F — pacing + damage + cursor + full-decoration casts)
> Branch: `main` (single-branch — Rust port complete; the C tree is the legacy reference under `src/`)
> One-liner: **P0 → P4 fully shipped; P5/P6 long-term goals all moved from design to code; P7 (built-in screencast portal) lit up the Window/Entire Screen tabs in browser meeting clients via a niri-pattern Mutter D-Bus shim + PipeWire pipeline.** Margo is now a daily-driver Wayland compositor with full modern-protocol parity, niri-grade animations + spring physics across every transition type, on-demand redraw scheduler, runtime DRM mode change via `wlr-randr`/`kanshi`, GitHub Actions CI gate, an embedded Rhai scripting engine that fires event hooks mid-event-loop, `wp_color_management_v1` standing up for HDR-capable client probes, and a built-in xdp-gnome backend that delivers live window/output frames into Helium / Chromium / Firefox screen-share dialogs without a running gnome-shell. What's left is depth on each, not new feature areas.

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
| **P1 protocol parity** | dmabuf+drm-syncobj, dmabuf-screencopy, region-crop, blockout, pointer_constraints+relative_pointer, xdg_activation, output_management (incl. mode change), presentation-time (VBlank-accurate) | **✅ 8/8** |
| **P2 perf/akıcılık** | frame_clock, spring engine, open/close/tag/focus/layer animations (bezier + opt-in spring across all 5), hw cursor, direct scanout, damage opt | **✅ 6/6** |
| **P3 window mgmt v2** | scratchpad+named, mango/layerrule parity, CSD/SSD policy, IPC parity, XWayland HiDPI env, popup focus | **✅ 6/6** |
| **P4 tooling** | smoke-winit, manual checklist, mctl JSON/rules/check-config, post-install smoke, shell completions, GitHub Actions CI | **✅ 7/7** |
| **P5/P6 long-term** | spatial canvas ✓, adaptive layout ✓, drop shadow ✓, scripting Phase 3 ✓, HDR Phase 1 ✓, screencast portal moved to **P7** | **5/5 code-shipped** |
| **P7 screencast portal** | 5 Mutter D-Bus shims, PipeWire pipeline, frame pacing, damage, cursor, full-decoration casts, HiDPI | **✅ 9/9 phases** |

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

### `[x]` Built-in xdg-desktop-portal backend → moved to **P7** below
The "design only" entry in earlier revisions of this roadmap shipped as a full implementation in May 2026. See P7 for the eight-phase port and what's worth revisiting.

### `[~]` HDR + color management *(Phase 1 shipped)*
**Phase 1 code: `margo/src/protocols/color_management.rs` (commit `25255a9`) + design `docs/hdr-design.md`.**

`wp_color_management_v1` global stands up; on bind it advertises supported primaries (sRGB / BT.2020 / Display-P3 / Adobe RGB), transfer functions (sRGB / ext_linear / ST2084-PQ / HLG / gamma 2.2), the perceptual rendering intent, and the parametric-creator feature surface. Chromium and mpv probes find a colour-managed compositor and light up their HDR decode paths even though composite still tone-maps to sRGB. ICC creator stubbed (Phase 4); parametric creator fully wired with all setters (`set_tf_named`, `set_primaries`, `set_luminances`, `set_mastering_*`, `set_max_cll`, `set_max_fall`).

Per-surface trackers store the active description's identity in an atomic; Phase 2 (linear-light fp16 composite) reads it from the render path. Phase 3 is KMS HDR scan-out; Phase 4 is per-output ICC.

### `[x]` Script/plugin system — Phases 1, 2, **3** shipped
**`margo/src/scripting.rs` (commits `562b5f7`, `13bdd57`, `769141e`) + design `docs/scripting-design.md`.** Rhai 1.24 sandboxed engine; `~/.config/margo/init.rhai` evaluated at startup with full action invocation, read-only state introspection, AND event hooks that fire mid-event-loop.

Bindings shipped:
- `dispatch(action, args_array)` + zero-arg overload — invokes any registered margo action.
- `spawn(cmd)`, `tag(n)` — convenience helpers.
- `current_tag()`, `current_tagmask()`, `focused_appid()`, `focused_title()`, `focused_monitor_name()`, `monitor_count()`, `monitor_names()`, `client_count()` — read-only state.
- **`on_focus_change(fn())`** — fires from `focus_surface` (post-IPC-broadcast, gated on `prev != new`).
- **`on_tag_switch(fn())`** — fires from `view_tag` after arrange + IPC.
- **`on_window_open(fn())`** — fires from `finalize_initial_map` after window-rules + focus.

State-access pattern: thread-local raw pointer set during eval, cleared via RAII guard. Hook firing uses an Option-take/restore dance so a re-entrant hook (a hook calls `dispatch(...)` triggering another event) finds `None` and is a no-op — recursion guard for free. Rhai's `print` / `debug` channels routed into tracing so script output lands in `journalctl`. Example: `contrib/scripts/init.example.rhai`.

*Why Rhai over Lua:* pure Rust (no C build), type-safe `register_fn`, sandbox tight by default. Trade-off is unfamiliarity; mitigated by example scripts.

What's still missing: `on_window_close` (needs stable identity for closing windows), `on_output_change` (easy add when demand surfaces), `mctl run <script>` for one-shot scripts (Phase 4), plugin packaging (Phase 5).

---

## 7. P7 — built-in screencast portal ✅ 9/9 phases

`a4f6ed6 → bf7e579 → 0c2f5d5 → f8f7a9a → 0455b4e → 81a6487`, ~3870 LOC across `margo/src/dbus/` (5 D-Bus shims) + `margo/src/screencasting/` (PipeWire core + render hooks) + udev backend integration + `margo/Cargo.toml` deps.

### Why this exists

xdp-wlr advertises `ext-image-copy-capture` which works for full-output capture, but Chromium-family browsers (Helium, regular Chromium, Edge, Brave) do **not** light up the Window / Entire Screen tabs in their share dialog against the wlr backend — they only enable per-window / per-output picking when xdg-desktop-portal-gnome is the backend. xdp-gnome in turn talks to **gnome-shell** over D-Bus on `org.gnome.Mutter.ScreenCast` + `.DisplayConfig` + `org.gnome.Shell.Introspect` + `.Screenshot` + `.Mutter.ServiceChannel`. On gnome-shell-less compositors those interfaces don't exist and xdp-gnome silently fails.

niri solved this by **implementing those Mutter D-Bus interfaces inside the compositor binary** so xdp-gnome can't tell it's not talking to real gnome-shell. P7 is a direct port of that pattern to margo.

### Shipped — eight phases

| Phase | Commit | LOC | What landed |
|---|---|---|---|
| **A** | `09c4e68` | +50 deps + scaffold | `zbus 5`, `pipewire 0.9`, `async-io`, `async-channel` workspace deps; module skeletons. |
| **B** | `09c4e68` | +1080 | All 5 D-Bus interface shims (`mutter_screen_cast`, `mutter_display_config`, `mutter_service_channel`, `gnome_shell_introspect`, `gnome_shell_screenshot`). zbus 5.x `#[interface]` async impls; calloop ↔ async-channel bridges so D-Bus threads can talk to the compositor event loop. |
| **C0** | `e3df482` | +215 | `screencasting/render_helpers.rs` — niri's GLES helpers (`encompassing_geo`, `render_to_texture`, `render_to_dmabuf`, `render_and_download`, `clear_dmabuf`). |
| **C1** | `acc47cb` | +1655 | `screencasting/pw_utils.rs` — full port of niri's `PipeWire` core + `Cast` struct + `CastInner` state machine + format negotiation (dmabuf preferred, SHM fallback) + buffer dequeue/queue/sync_point handling. |
| **D1** | `2c9d4e0` | +135 | `Screencasting` top-level state on `MargoState` + `mutter_service_channel` `NewClient` channel routing. |
| **D2** | `a4f6ed6` | +244 | All 5 shims registered onto their well-known names (`org.gnome.Mutter.ScreenCast`, `.Mutter.DisplayConfig`, `.Shell.Introspect`, `.Shell.Screenshot`, `.Mutter.ServiceChannel`); xdp-gnome connects + finds margo-as-mutter. |
| **E1** | `bf7e579` | +183 | `MargoState::start_cast` resolves `StreamTargetId::{Output, Window}` → `(CastTarget, size, refresh, alpha)`; lazy-init Screencasting + PipeWire on first cast; calls `pw.start_cast(...)`; pushes the resulting `Cast` onto `casting.casts`. xdp-gnome receives the PipeWire node ID and starts the WebRTC pipeline. |
| **E2** | `0c2f5d5 → f8f7a9a` | +250 | `drain_active_cast_frames` in `backend/udev.rs` — the actual frame producer. Iterates every active cast each repaint; for `Window { id }` looks up the matching `MargoClient` by stable `addr_of!` u64 and renders the surface tree at (0,0); for `Output { name }` iterates every visible client on that monitor and renders each at its monitor-local position; calls `Cast::dequeue_buffer_and_render` against the queued PipeWire buffer. Plus continuous-repaint re-arm so the chain doesn't go idle while sharing. |
| **F** | `81a6487` | +170 | Five depth items fused: (1) pacing via `Cast::check_time_and_schedule` — niri-port scaffolding from Phase C1 actually wired now; static scenes skip element-build at the gate. (2) damage via the per-cast `OutputDamageTracker` already inside `dequeue_buffer_and_render` — no buffer queued when nothing changed; encoder drops to keyframe-only. (3) cursor embedded as a `MargoRenderElement::Cursor` element via `include_cursor=true`; gated by `cast.cursor_mode()` so Hidden/Metadata don't leak. (4) full-decoration casts via a new `CastRenderElement` enum (`Direct(MargoRenderElement)` for output, `Relocated(RelocateRenderElement<MargoRenderElement>)` for window) — the share view now matches the live display with borders, shadows, popups, animations, block-out. (5) HiDPI scale fix: window casts now read the monitor's fractional scale and convert `client.geom` (logical) to physical pixels for the cast buffer. |

Final cleanup commit `0455b4e` adds module-level `#![allow(dead_code)]` to the niri-port files so the build is warning-free; Phase F flips most of those flags into actual call sites.

### Strengths

- **Stable Window IDs via `addr_of!(*MargoClient) as u64`.** xdp-gnome's window picker needs a u64 handle that's stable for the duration of the client's life. Most compositors invent a side-channel ID map; we lean on the fact that `MargoClient` lives in a Vec at a stable heap address — its memory address IS the stable ID. Zero bookkeeping. `gnome_shell_introspect::GetWindows` returns these; `mutter_screen_cast::StreamTargetId::Window { id }` echoes one back; `MargoState::start_cast` matches against `state.clients` by linear scan. O(N) lookup is fine — N is single-digit-windows in practice.

- **`mem::take(&mut casting.casts)` borrow trick.** `Cast::dequeue_buffer_and_render` needs `&mut Cast`; the surrounding render code needs `&MargoState` to look up clients/monitors. Both live on `MargoState`, so straight nested borrows fail. Detaching the casts vec lets us iterate freely; re-attaching is a single move. Direct port of niri's pattern in `redraw_cast`.

- **Continuous-repaint while casts active.** Margo's repaint scheduler is dirty-flag-gated — without input or animation the loop goes idle. PipeWire fires `Redraw` exactly twice per stream lifetime (initial Streaming + first dmabuf), then never again, so pacing-by-PipeWire-callback would freeze on the first frame. Solution: at end of `drain_active_cast_frames`, if any cast is active, call `request_repaint()`. The VBlank handler re-pings the loop and we get continuous ~refresh-rate cast frames. PipeWire's `dequeue_available_buffer` returns None when the consumer hasn't returned a buffer yet, so frame production self-throttles to whatever the WebRTC consumer can chew through.

- **Lazy PipeWire init.** `Screencasting` and the PipeWire core are an `Option<Box<...>>` on `MargoState`, only stood up on the first cast. Normal sessions (no screen sharing) pay zero PipeWire cost — no thread, no socket, no main_loop iteration. Cleanly mirrors the design intent: screencast is a feature, not a baseline.

- **Buffer-bounded backpressure.** No frame-pacing logic in margo; we render every repaint into PipeWire's pool. PipeWire's pool has a fixed buffer count negotiated with the consumer. If the consumer hasn't drained a buffer yet, dequeue returns None and we drop the frame. Cleaner than a margo-side timer: the actual consumer determines the rate.

### Worth revisiting

- **`gnome_shell_introspect::windows_changed` signal never fired.** xdp-gnome listens for it to refresh the window picker live. Today the picker shows the snapshot from when the dialog opened; new windows that appear afterwards aren't visible until the user re-opens the share dialog. ~30 LOC to fire from `finalize_initial_map` and `toplevel_destroyed`.

- **`IpcOutputMap` snapshot is one-shot.** `mutter_display_config::GetCurrentState` builds a fresh map per call (small N — fine) but the cached `name`/`refresh`/`output` triple stashed on each cast at `start_cast` time goes stale on hotplug. Disconnect/reconnect during an active cast won't update the snapshot. Niri tracks this via `mapped_cast_output`; margo can rebuild lazily via `WeakOutput::upgrade`.

- **`ScreenCast::Session::Stop` D-Bus method exists but the cleanup chain is partial.** Stop messages route through `ScreenCastToCompositor::StopCast` and `MargoState::stop_cast`, which retains the cast-vec by `session_id`. PipeWire stream and Cast struct are dropped in order (Cast carries the listener which is dropped before the Stream — verified). What's NOT done: the WebRTC consumer can hang briefly if it's mid-frame when we drop. niri has `cleanup_with_grace_period`; we just drop. Acceptable; logs may show pipewire warnings on rapid stop/start cycles.

- **Continuous repaint while casts active is global.** When any cast is active, `drain_active_cast_frames` re-arms `request_repaint()` at end-of-drain so the loop ticks at refresh rate. With pacing in place this is mostly cheap — paced-skip casts bail before the expensive element build — but a paced-skip frame still costs ~5µs per cast for the borrow + duration compare. niri schedules a per-cast timer that wakes the loop only at the right moment; margo would benefit when running multiple casts at different framerates simultaneously. Optimization, not a correctness gate.

- **`CursorMode::Metadata` falls back to "no cursor at all".** Embedded mode embeds the cursor in the frame; Hidden mode omits the cursor; Metadata mode is supposed to send cursor as a sidecar so the consumer can composite it natively (sharper at low cast resolutions). Phase F gates `include_cursor` on `cast.cursor_mode() == Embedded` only — Metadata mode produces frames with no cursor and no metadata. Most browsers ask for Embedded so this is rarely visible; ~80 LOC to populate `CursorData` properly when xdp-gnome asks for Metadata.

---

## What's queued (next sprint candidates)

Recently shipped (this two-sprint depth-pass):

**Sprint 1 — tooling + observability:**
- ✓ Scripting Phase 2 — `dispatch(...)` + state introspection (commit `13bdd57`).
- ✓ GitHub Actions CI workflow — cargo build/test + `mctl check-config` on every PR (commit `2910567`).
- ✓ JSON schema `"version": 1` field on `mctl status --json` (commit `2910567`).
- ✓ `presentation-time` accuracy — feedback signalled at VBlank, not submit (commit `bcb6fb4`).

**Sprint 2 — depth on long-term goals:**
- ✓ **`wlr_output_management_v1` mode change** — runtime DRM atomic re-modeset; `wlr-randr --mode 1920x1080@60` succeeds, kanshi profile flips actually change scan-out resolution (commit `a26cc9b`).
- ✓ **HDR Phase 1** — `wp_color_management_v1` global stands up, advertises primaries + TFs + parametric creator. Chromium / mpv detect colour-managed compositor and light up HDR paths (commit `25255a9`).
- ✓ **Scripting Phase 3** — `on_focus_change` / `on_tag_switch` / `on_window_open` registrations fire from compositor event sites with recursion guard (commit `769141e`).
- ✓ **Spring physics for open/close/tag/focus/layer** — `animation_clock_*` per-type config picks bezier or spring-baked curve. Default bezier; opt-in spring (commit `71b95a1`).

**Sprint 3 — quick wins from the depth pass:**
- ✓ **Screencast `windows_changed` D-Bus signal** — fires from `finalize_initial_map` + `toplevel_destroyed` (Wayland + X11) so xdp-gnome's window picker stays live mid-share-dialog. New helper `emit_windows_changed_sync` bridges blocking-zbus ↔ async via `async_io::block_on`, mirroring the existing `pipe_wire_stream_added` pattern.
- ✓ **`on_window_close` Rhai hook** — fires AFTER state is consistent (client gone, focus shifted, arrange done) with `(app_id, title)` as Rhai string args (focused_*() can't reach a dead window). Same recursion-guard discipline as the other hooks. Example script extended.
- ✓ **Direct scanout observability** — `MargoClient::last_scanout` cached after each successful `render_frame` by walking the surface tree and matching `Id::from_wayland_resource` against `RenderElementStates` for `ZeroCopy` presentation. Surfaces in ZeroCopy → primary or overlay plane (composition skipped). Exposed in state.json + `mctl clients` shows ★ marker for on-scanout windows.
- ✓ **Screencast `CursorMode::Metadata` cursor sidecar** — new helper `build_cursor_elements_for_output` extracts cursor sprite render elements separately from the main scene; metadata casts now prepend pointer elements to the cast slice with `elem_count = cursor_count` so `CursorData::compute` wraps them, pw_utils strips the pointer elements from the main damage pass, and `add_cursor_metadata` writes a real cursor bitmap into the SPA sidecar. xdp-gnome consumers (browsers requesting metadata mode) see a sharp cursor at low cast resolutions instead of "no cursor at all".

**Sprint 4 — protocol depth + foundation work:**
- ✓ **Smoke test in CI** — `.github/workflows/smoke.yml` runs `scripts/smoke-winit.sh` end-to-end under Xvfb on every push/PR with kitty as the test client. Lives separately from the lightweight `ci.yml` so the build/test/check-config path stays fast; on failure uploads `/tmp/margo-smoke-*/` as a workflow artifact.
- ✓ **`wlr_output_management_v1` disable-output** — soft-disable path: marks `MargoMonitor::enabled = false`, migrates every client on it to the first remaining enabled monitor, skips arrange + render for it, and refuses any disable that would leave zero active outputs. The smithay `Output` and udev `OutputDevice` stay alive so re-enable resumes without a hotplug; pertag/gamma/scale survive. New keybind dispatch actions: `disable_output` / `enable_output` / `toggle_output` (catalogued in `mctl actions`). DRM connector power-off remains a follow-up.
- ✓ **HDR Phase 2 — scaffolding shipped, swapchain integration upstream-gated.** New module `render/linear_composite.rs` ships the linear-light side of HDR: sRGB / ST2084-PQ / HLG / γ2.2 transfer-function math with bit-exact GLSL **and** equivalent `f32` CPU implementations for unit-test verification (round-trip identity at spec-value sample points: sRGB 0.5↔0.21404 linear, PQ peak, HLG kink at 0.5↔1/12). GLES texture shaders (encoder + decoder) compile lazily and cache thread-local. `MARGO_COLOR_LINEAR=1` env gate eagerly compiles both programs on the live renderer at first frame so driver-side rejection surfaces at startup, not first cast. The runtime swapchain switch from `Argb8888` to `Abgr16161616f` needs an `OutputDevice`-aware reformat that smithay 0.7's `DrmCompositor` doesn't expose at runtime — so `is_linear_composite_active()` returns `false` today; flip the body to `is_linear_composite_enabled()` once the upstream API lands and queue a `TextureRenderElement` wrapping the encoder onto the frame. ~80 LOC of integration remains, the math + shaders are validated and ready.

Still queued:

1. **Upstream smithay fp16 swapchain knob** — once landed, finishes Phase 2's runtime activation in ~80 LOC.

2. **HDR Phase 3 — KMS HDR scan-out.** Negotiate output's preferred HDR format via DRM `EDR_PROPERTIES` blob; on HDR-capable surface visible, set `HDR_OUTPUT_METADATA` and flip to 10-bit PQ scan-out. ~400 LOC + heavy hardware-test matrix.

3. **HDR Phase 4 — per-output ICC profiles.** Read `colord` ICC via D-Bus, bake into per-output 3D LUT, sample after composition. ~250 LOC.

---

## Catch-and-surpass-niri plan

**Premise.** A fair side-by-side audit of margo (~33k LOC, 29 unit tests, 5 docs files) against niri (~93k LOC, 61 unit tests, **5,280 visual snapshot files**, 47 docs, AccessKit a11y, pixman software renderer, full tablet stack, modular per-backend crates) shows niri is the more battle-tested codebase by a wide margin. Margo wins on **tag-based dwm-style workflow**, **embedded Rhai scripting**, **dwl-ipc-v2 wire compat**, **HDR Phase 1 + Phase 2 scaffolding**, **14-layout catalogue** — none of which niri has, all of which exist because margo is built around a specific user's workflow. To make margo a *better* product than niri (not just a personal driver), four work-streams need to land. Prioritised by "biggest gap to close" first.

### W1 — Test infrastructure depth (the single biggest niri lead)

niri ships 5,280 visual snapshot files (`src/tests/snapshots/`) covering window-opening, fullscreen, floating, layer-shell, and remove-output state matrices. A single regression in any of those state machines is caught at PR time, not at a user's reload. Margo's 29 unit tests cover the parser, IPC catalogue, mlayout pickers — none of the *visual* output.

| # | Item | Estimate | Notes |
|---|---|---|---|
| W1.1 | ✅ **Layout-snapshot test suite shipped.** `margo/src/layout/snapshot_tests.rs` + `snapshots/` dir lock the geometry of 14 layout algorithms × 20 canonical scenarios into committed `.snap` text files. Insta-based (no PNG churn — pure text diff at PR review). 24/24 pass on `cargo test --workspace`. Property tests verify `arrange()` dispatcher matches direct calls and non-scroller layouts stay inside the work area. |
| W1.2 | Layout-algorithm property tests (already partially shipped) | ~200 LOC | Extend `layout/algorithms.rs`'s 2 existing fixtures to cover all 14 layouts × 1/2/3/n-window cases × overview-vs-normal. Today only scroller has even a partial test. |
| W1.3 | Window-rule snapshot tests | ~150 LOC | `mctl rules` already prints rule-match outcomes. Build a fixture-driven test that loads N candidate (appid, title) pairs against the example config and snapshots the rule-match decisions. Catches windowrule regressions like "Electron leaked from tag 5" before users see them. |
| W1.4 | clippy gating + `clippy.toml` | ~30 LOC | niri ships `clippy.toml` with project lints + clippy is a CI gate. Margo's CI runs clippy non-gating because of organic drift; do an opt-in cleanup pass (one PR), then flip to gated. |
| W1.5 | `CONTRIBUTING.md` + PR template | ~150 LOC docs | niri has 114-line CONTRIBUTING.md; margo has none. Required if we want external contributions. |

### W2 — Capability parity (niri has, margo doesn't, users notice)

| # | Item | Estimate | Why it matters |
|---|---|---|---|
| W2.1 | ✅ **In-compositor region selector shipped** (lean re-do of the reverted Phase 3). New module `screenshot_region.rs` plus a `region_selector: Option<ActiveRegionSelector>` field on `MargoState`. Drag a rect with the mouse / press Enter to confirm / Escape to cancel; on confirm spawns `mscreenshot <mode>` with `MARGO_REGION_GEOM="X,Y WxH"` set so the binary skips its own slurp call. Render side: four `SolidColorRenderElement` outline edges prepended to the scene each frame (no custom shader needed; smithay supports SolidColor as a first-class element). Input side: `handle_input` early-routes pointer + keyboard to selector handlers when active. **Scope deliberately narrowed** vs the reverted attempt — capture / save / clip / edit stay in `mscreenshot` (where shell tools already work well); only the UX gap (slurp's separate window, focus fight) is replaced. ~340 LOC across margo + mscreenshot. |
| W2.2 | **Pixman software renderer fallback** | ~400 LOC | niri ships `renderer_pixman` as a separate crate so margo runs in `qemu`-without-virgl, in containers, on systems with no GPU/EGL. Margo today panics on bring-up if EGL fails. The hook is already there in smithay (`Renderer` trait); 400 LOC is the wrapper layer + dispatch. |
| W2.3 | ⏸ **Tablet input** *(deferred)* — bigger lift than the surrounding items, no immediate hardware to dogfood against. `tablet_v2` protocol, stylus + pad button mapping, `map-to-focused-window` mode. Re-prioritize when a Wacom/Huion user files a request. ~500 LOC. |
| W2.4 | **AccessKit a11y tree** | ~350 LOC | `accesskit` + `accesskit_unix` + a `Niri::a11y_tree()` analogue. Screen readers (Orca) get a window list. Without this, margo isn't usable for blind users — niri is. The dbus surface is `org.freedesktop.a11y`; smithay-side it's a tree-emit per-arrange. |
| W2.5 | **xwayland-satellite mode** | ~200 LOC | Run XWayland as a separate process per niri's pattern instead of in-tree. Resilience win: an X11 client crash can't take the compositor down. Today margo runs the in-tree XWayland (smithay `XWayland::start`); satellite mode is gated behind a feature flag. |
| W2.6 | ✅ **Cargo feature flags shipped.** Two features added to `margo/Cargo.toml`: `dbus` (default, gates zbus + async-io + async-channel) and `xdp-gnome-screencast` (default, requires `dbus`, gates pipewire). All four deps moved to `optional = true`. Cfg-gated: `mod dbus`, `mod screencasting`, the `dbus_servers` + `screencasting` fields on `MargoState`, the `emit_windows_changed` body (function stays as a no-op), `start_cast` / `stop_cast` / `on_pw_msg` methods, the udev cast-drain hook, the 5 D-Bus init blocks in `main.rs`. Three build configs verified: default (full), `--no-default-features --features dbus` (no screencast), `--no-default-features` (lean). All snapshot tests pass under each. ~140 LOC of cfg sprinkles. |
| W2.7 | ✅ **Tracy profiler hooks shipped.** `tracy-client = { version = "0.18", default-features = false }` always-on dep + `profile-with-tracy = ["tracy-client/default"]` feature. `let _span = tracy_client::span!(...)` calls are no-ops in normal builds (zero cost) and connect to a live Tracy GUI when the feature flips. Six hot-path spans wired: `render_output`, `build_render_elements`, `arrange_monitor`, `tick_animations`, `handle_input`, `focus_surface`. Build + run with `--features profile-with-tracy` and connect Tracy to debug per-frame timing without log spelunking. |

### W3 — Capability extension (push margo's existing lead further)

These are areas where margo is *already ahead* and could widen the gap.

| # | Item | Estimate | Notes |
|---|---|---|---|
| W3.1 | HDR Phase 3 — KMS HDR scan-out | ~400 LOC | Already in roadmap. Margo's HDR Phase 1 + 2 scaffolding give it a head start vs niri (zero HDR work). Land Phase 3 first, then Phase 4 ICC. |
| W3.2 | Rhai scripting Phase 4 — `mctl run <script>` | ~200 LOC | Today scripting is init.rhai-only. One-shot script execution against the live state would let users script complex window-management actions on the fly. |
| W3.3 | Rhai scripting Phase 5 — plugin packaging | ~400 LOC | A `~/.config/margo/plugins/<name>.rhai` directory + manifest + sandboxing per-plugin. Lets users share plugins (e.g. "smart-tag-routing", "ai-window-grouping") without modifying init.rhai. |
| W3.4 | dwl-ipc-v2 extensions | ~150 LOC | We're the reference dwl-ipc-v2 implementation outside dwl/mango. Adding events for occupied counts, focus history, scratchpad state would make margo + noctalia / waybar-dwl combos richer than niri+swaybar. |
| W3.5 | Layout-cycle keybinds with previews | ~200 LOC | `switch_layout` cycles through `circle_layout`. Adding a preview overlay (mini icons + hint) before commit would make the 14-layout catalogue actually navigable instead of just configurable. |
| W3.6 | Per-tag wallpaper | ~150 LOC | Each tag carries its own wallpaper hint via `tagrule = id:N, wallpaper:path`; the swaybg/noctalia wallpaper component reads the active tag's hint via dwl-ipc. niri can't even contemplate this — it has no tags. |

### W4 — Architecture / openness (slow-bake, but compounds)

| # | Item | Estimate | Notes |
|---|---|---|---|
| W4.1 | Split backends into separate crates | ~600 LOC churn | niri has `backend_drm`, `backend_egl`, `backend_gbm`, `backend_libinput`, `backend_session_libseat`, `backend_winit`, `backend_udev` as 7 crates. Margo's `backend/` is in-tree. Splitting eases incremental compilation (touching the input loop doesn't recompile the renderer) AND lets downstream Wayland projects depend on margo's backend crates without pulling the whole compositor. |
| W4.2 | Move `state.rs` (~7000 LOC) into modules | ~100 LOC churn | The single-translation-unit feel was a deliberate port from C, but at 7000 LOC compile times suffer. Split into `state/{ipc, focus, scratchpad, output, x11, wayland_handlers}`. Behaviour-preserving refactor. |
| W4.3 | mkdocs site + GitHub Pages | ~200 LOC config + content | niri has a full `docs/` mkdocs site with wiki + rendered hooks docs. Margo has 5 markdown files. A published site (`kenanpelit.github.io/margo`) doubles the discoverability. |
| W4.4 | Configuration migration tooling | ~250 LOC | A `mctl migrate` that reads a hyprland.conf / niri-config.kdl / sway/config and emits margo-equivalent keybinds + windowrules. Onboards users from those compositors with one command. niri has nothing like this. |
| W4.5 | `niri-visual-tests`-equivalent design tool | ~500 LOC | niri ships an interactive "play with animations + colours live" GTK app. We'd want the equivalent for margo's 14 layouts × per-tag layout pinning preview. Useful for both regression catches and config authoring. |

### Why this beats niri (sequenced execution)

If we land **W1.1 (visual snapshots) + W2.1 (in-compositor screenshot) + W2.3 (tablet) + W3.1 (HDR Phase 3)** in that order — that's the four highest-leverage moves. After those four:

- Margo has the *only* HDR-capable Wayland compositor with a Rust + Smithay base on the desktop side.
- Margo has visual snapshot regression coverage on par with niri.
- Margo has feature parity on screenshot UX + tablet + a11y (after W2.4).
- Margo retains its unique advantages: tag-based workflow, 14 layouts, dwl-ipc-v2, Rhai.

The "personal-driver-only" framing flips: margo becomes "a Rust+Smithay Wayland compositor with the maturity of niri AND a dwm/dwl-style tag workflow AND working HDR" — a category niri doesn't and won't compete in (niri's design intent is scroller-only, no tags).

**Order of operations**: do W1.1 first regardless of what comes next — every capability item lands faster when a regression test catches the geometry bug before bisect.

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
- [ ] Helium / Chromium → Meet → Share screen → Window tab populates with live windows; pick one → share preview shows live content (not frozen first frame).
- [ ] `mctl status --json | jq .outputs[0].focused.app_id` returns the focused window.
- [ ] `mctl check-config ~/.config/margo/config.conf` reports zero errors.
- [ ] `~/.config/margo/init.rhai` evaluates at startup if present (one log line at info level).
