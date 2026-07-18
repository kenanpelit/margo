# Wayland Protocol Surface — margo vs niri vs Hyprland vs mango

> **Correctness update (2026-07-13, resolved in 1.1.9):** margo briefly
> withdrew `wp_fifo_v1` and `wp_commit_timing_v1` because their managed
> commit barriers were never released (a hidden-then-shown Chromium surface
> could stall behind them). Both globals are advertised again as of 1.1.9,
> now driven by a real barrier-release scheduler
> (`margo/src/state/pacing.rs`): FIFO barriers signal at every per-output
> present, hidden surfaces release from the frame-callback fallback tick,
> and commit-timing deadlines get an exact one-shot wake. The advertised
> surface is back at **57**.

> **margo refresh (2026-07-18):** two corrections from a fresh source
> audit. (1) **`wp_color_representation_v1` is now advertised** — a real
> hand-rolled implementation (`margo/src/protocols/color_representation.rs`)
> that only claims what the render path does: `premultiplied_electrical`
> alpha + `identity`/`full` coefficients; YCbCr matrices are deliberately
> not advertised while YUV conversion happens driver-side in EGL. Only
> margo and mango ship this protocol. (2) **`wp_color_management_v1`
> corrected ✅→⚠️**: the delegate + full Phase-1 plumbing are compiled, but
> the *global* has been withheld since `28ad09bf` ("do not register the
> global until Phase 2 lands" — the mpv/Chromium probe crashes). The
> delegate-macro count missed this; the *advertised* surface therefore
> stays **57** (−1 gated colour-management, +1 new colour-representation).
> When HDR Phase 2 flips the gate, both will be live and the count goes 58.

> **mango column re-walk (2026-07-17, mango `0.15.2-10` / `242ffe8`):**
> mango moved to **wlroots 0.20 + SceneFX** and its column below was stale
> (carried from 0.13/0.14). Re-counted from source (hand-curated
> `wlr_*_create` protocol globals): headline **~53 → ~55**. Cell flips:
> `wp_color_management_v1` ❌→✅ (created whenever the FX renderer reports
> colour-transform support — the per-monitor `hdr` knob gates HDR *output*,
> not the global; mango also ships `wp_color_representation_v1`, which
> margo answered with its own on 2026-07-18) and `xwayland_shell_v1` ❌→✅ (wlroots creates
> it in XWayland builds). Newly counted since the old walk:
> `ext_data_control_v1`, `ext_foreign_toplevel_list_v1` + per-window/output
> image-capture-source managers (window-capture path), `wl_fixes`. The rest
> of the modern set is still missing (content-type, fifo, commit-timing,
> xdg-dialog, system-bell, pointer-warp, security-context). Only the mango
> column was touched this pass.

> **Last refreshed:** 2026-06-17 — **margo column re-counted directly from
> source** at `v1.0.7` (`446f8597`): margo advertises **57 protocol-bearing
> smithay `delegate_*` macros** (excluding the two framework-internal
> `delegate_dispatch` / `delegate_global_dispatch`); all 8 hand-rolled
> `GlobalDispatch` globals overlap an existing delegate, so they add no
> extra. This **corrects the prior headline `~60`** (which over-counted —
> the standing-picture prose below already said `~57`) to the
> source-verified **57**. The protocol *set* is unchanged from the `v1.0.3`
> audit: `tearing_control` and `drm_lease` are still the only two not
> advertised (both blocked, see gaps below).
> **Sources (re-counted from each project's `HEAD` this pass):**
> - **margo** `v1.0.7` (`446f8597`) — this repo; **57** protocol `delegate_*` macros (+ overlapping hand-rolled `GlobalDispatch`).
> - **niri** `v26.04-34` (`fdb6d85`) — same smithay method; **41** protocol delegates (unchanged from the 2026-05-28 walk).
> - **Hyprland** `v0.55.0-189` (`2d190ba`) — `src/protocols/*.cpp` (**63**) + `src/protocols/core/` (**6**) = **69**.
> - **mango** `0.15.2-10` (`242ffe8`) — wlroots `wlr_*_create()` protocol globals, hand-curated to exclude non-protocol calls (`wlr_scene_*`, `wlr_output_layout_*`, device/backend objects); **~55, re-counted 2026-07-17** (wlroots 0.20 + SceneFX).

> **Companion:** [`protocol-matrix.md`](protocol-matrix.md) is the *internal*
> view — for each protocol margo advertises, whether it's implemented and how
> it's tested (CI fixture vs manual vs none). This doc is the *cross-project*
> view (who advertises what).

This document is the source-of-truth audit of which Wayland protocols
each daily-driver compositor advertises. **mango** — the C/wlroots
(dwl-derived) compositor that margo is the Rust rewrite of — is
included so the comparison shows what margo gained, kept, and dropped
relative to its own origin. Maintained alongside `road_map.md`
§15.10; this file is the **read-side reference** for "what does each
compositor actually advertise."

## Legend

| Mark | Meaning |
|---|---|
| ✅ | shipped / advertised |
| ⚠️ | partial (e.g. read-only, no write-side; or reached via a different mechanism) |
| ❌ | not advertised |

## Headline score

Counted with each project's native method (delegate macros / protocol
files / `wlr_*_create` globals), **core protocols included** so the
numbers are comparable.

| Compositor | Protocols (approx.) | Stack | Note |
|---|---|---|---|
| **Hyprland** `0.55.0` | **69** | C++ (hand-rolled) | Widest surface — 63 protocol modules + 6 core, plus Hyprland-only extensions (source-counted 2026-06-17) |
| **margo** `main` | **57** | Rust / smithay | Modern surface; **ahead of niri and mango**; pursuing Hyprland. FIFO and commit-timing rejoined in 1.1.9, now backed by the `state/pacing.rs` barrier-release scheduler |
| **mango** `0.15.2` | **~55** | C / wlroots | Broad and modernizing: wlroots 0.20 + SceneFX hand it the surface for free — now incl. HDR colour-management (re-counted 2026-07-17) |
| **niri** `26.04` | **41** | Rust / smithay | Tightest surface; deliberately minimal (source-counted 2026-06-17) |

**This refresh (2026-06-17) re-counts the surface from source; the
standings are unchanged — only the margo headline number was corrected**
(`~60` → source-verified **57**, matching the standing-picture prose that
already said `~57`). What was re-counted directly from each project's
checkout this pass: **margo = 57** protocol `delegate_*` macros, **niri =
41** (identical to the 2026-05-28 walk), **Hyprland = 69** (63
`src/protocols/*.cpp` + 6 `core/`; was 68 — consistent with its steady
module creep, e.g. the earlier `hyprland_lock_notify` addition).
**mango (~53) was *not* re-counted** this pass — its wlroots globals need
manual curation to separate true protocol globals from scene/output
`wlr_*_create` calls, so the 2026-05-28 figure is carried forward.

> **Scope of this pass:** only **margo's** column was re-verified against
> source (every margo ✅/❌ in the tables below was confirmed against the
> `delegate_*` list). The **niri / Hyprland / mango** per-protocol cells
> carry forward from the 2026-05-28 four-way walk and were **not**
> re-walked protocol-by-protocol here.

The current standing picture: **margo (~57) narrowly leads its own C
ancestor mango (~55)** and niri (~41), having hand-rolled the three
wlroots-freebies mango got for free — `zwlr_foreign_toplevel_manager_v1`
(write-side, P2), `ext_workspace_v1` (P5),
`zwlr_virtual_pointer_manager_v1` (P7) — on top of the *modern* surface
(content-type, fifo/commit-timing, security-context, pointer-warp,
xdg-dialog, system-bell, toplevel-icon, toplevel-tag,
xwayland-keyboard-grab) that mango's wlroots base doesn't expose. HDR
colour-management left that list in mango 0.15: wlroots 0.20 ships it and
mango wires it up, while margo's own manager global stays Phase-2-gated
(see the 2026-07-18 note). mango's `wp_color_representation_v1` lead
lasted a day — margo advertises its own since 2026-07-18. `output_power` now ships (external
DPMS control), and `fifo`/`commit-timing` are driven by a real
presentation scheduler since 1.1.9. The two margo still doesn't advertise
(`tearing_control`, `drm_lease`) are blocked, not just deferred — see
"remaining gaps" below.

## Core baseline — present in all four

All four advertise these. No comparison table — assume ✅ everywhere.

- `wl_compositor` / `wl_subcompositor` / `wl_shm` / `wl_seat` / `wl_output`
- `wp_viewporter`, `wp_presentation_time`, `wp_cursor_shape_v1`,
  `wp_fractional_scale_v1`
- `zwlr_layer_shell_v1`
- `xdg_shell`, `xdg_activation_v1`, `xdg_decoration_unstable_v1`,
  `xdg_foreign_v2`
- `linux_dmabuf_v1`
- `ext_session_lock_v1`, `ext_idle_notifier_v1`,
  `zwp_idle_inhibit_manager_v1`
- `wl_data_device`, `zwlr_data_control_manager_v1`,
  `ext_data_control_manager_v1`,
  `zwp_primary_selection_device_manager_v1`
- `zwp_pointer_constraints_v1`, `zwp_relative_pointer_manager_v1`,
  `zwp_pointer_gestures_v1`
- `zwp_text_input_manager_v3`, `zwp_input_method_manager_v2`,
  `zwp_virtual_keyboard_manager_v1`
- `zwp_tablet_manager_v2`
- `zwlr_gamma_control_manager_v1`, `zwlr_screencopy_manager_v1`
- `org_kde_kwin_server_decoration`, `wp_single_pixel_buffer_v1`
- `zwp_keyboard_shortcuts_inhibit_v1`
- `zwlr_output_manager_v1` (**output-management** — read topology + apply
  scale/transform/position; all four ship it, was previously missing
  from this audit)

## Modern protocol surface — where margo leads the pack

These are the protocols that separate a *current* compositor from a
*legacy wlroots* one. margo and Hyprland carry almost the full set;
niri and mango each miss large chunks.

| Protocol | margo | niri | Hyprland | mango | Use case |
|---|---|---|---|---|---|
| `wp_color_management_v1` (HDR) | ⚠️ gated | ❌ | ✅ | ✅ | HDR / wide-gamut output (margo: full Phase-1 plumbing compiled but the global is withheld until HDR Phase 2 — see 2026-07-18 note; mango since 0.15 via wlroots 0.20; renderer-gated, per-monitor `hdr` knob) |
| `wp_color_representation_v1` | ✅ | ❌ | ❌ | ✅ | Alpha-mode / YCbCr-matrix buffer metadata (margo since 2026-07-18: honestly advertises exactly what the render path does — premultiplied alpha, identity/full) |
| `linux_drm_syncobj_v1` (explicit sync) | ✅ | ❌ | ✅ | ✅ | Tear-free GPU sync, NVIDIA |
| `ext_image_copy_capture_v1` + capture-source | ✅ | ❌ | ✅ | ✅ | Modern screencast (replaces screencopy) |
| `xwayland_shell_v1` | ✅ | ⚠️ diff path | ✅ | ✅ | HiDPI XWayland scaling (mango: created by wlroots in XWayland builds) |
| `wp_content_type_v1` | ✅ | ❌ | ✅ | ❌ | Game / video / photo hint |
| `wp_fifo_v1` | ✅ | ❌ | ✅ | ❌ | FIFO commit ordering; barriers released by `state/pacing.rs` since 1.1.9 |
| `wp_commit_timing_v1` | ✅ | ❌ | ✅ | ❌ | Explicit commit-time targets; deadline wake + present-pass release since 1.1.9 |
| `wp_alpha_modifier_v1` | ✅ | ❌ | ✅ | ✅ | Per-surface alpha hint |
| `xdg_wm_dialog_v1` | ✅ | ❌ | ✅ | ❌ | Modal dialog hint |
| `xdg_system_bell_v1` | ✅ | ❌ | ✅ | ❌ | System bell |
| `wp_pointer_warp_v1` | ✅ | ❌ | ✅ | ❌ | Programmatic cursor warp |
| `wp_security_context_v1` | ✅ | ✅ | ✅ | ❌ | Flatpak / sandboxed clients |

## margo's unique / near-unique protocols

| Protocol | margo | niri | Hyprland | mango | Note |
|---|---|---|---|---|---|
| `zwp_xwayland_keyboard_grab_v1` | ✅ | ❌ | ❌ | ❌ | **margo unique** — X11-side kb grab for VNC/VM/remote |
| `xdg_toplevel_icon_v1` | ✅ | ❌ | ❌ | ❌ | **margo unique** — inline app icons on toplevels |
| `xdg_toplevel_tag_v1` | ✅ | ❌ | ✅ | ❌ | Semantic toplevel tags (Hyprland ships `XDGTag` too) |

> **Correction this refresh:** `xdg_toplevel_tag_v1` is **no longer
> margo-unique** — Hyprland 0.55 ships it as `XDGTag.cpp`. The previous
> "margo unique vs both niri and Hyprland" claim was stale. Only
> `xwayland_keyboard_grab` and `xdg_toplevel_icon` remain unique to
> margo across all four.

## Recently shipped — the wlroots-freebie catch-up (P2 / P5 / P7)

Three protocols mango/Hyprland get for free from wlroots, hand-rolled in
margo (ported from niri) so it no longer trails on them:

| Protocol | margo | niri | Hyprland | mango | What it unlocks |
|---|---|---|---|---|---|
| `zwlr_foreign_toplevel_manager_v1` (write-side) | ✅ | ✅ | ✅ | ✅ | Taskbar click-to-activate / close / (un)fullscreen (mshell active-window pill) |
| `ext_workspace_v1` | ✅ | ✅ | ✅ | ✅ | Workspace state for third-party bars (sfwbar, ironbar) — the standard bar-state path now that dwl-ipc is gone |
| `zwlr_virtual_pointer_manager_v1` | ✅ | ✅ | ✅ | ✅ | Synthetic pointer — `wtype --click`, remote desktop, accessibility |

margo's foreign-toplevel write-side runs *alongside* the existing
smithay `ext-foreign-toplevel-list-v1` (read side stays untouched).
`ext_workspace_v1` maps each output to a workspace group with 9 fixed
tag-workspaces (active = bitmask membership); it is the standard bar-state
protocol now that dwl-ipc has been removed (mctl + mshell use the Unix
control socket instead). `virtual_pointer` feeds margo's normal input path.

## The remaining gaps — blocked, not just deferred

These two are **not** a matter of effort — each hits a concrete
upstream or architectural wall (re-confirmed by source audit 2026-05-28).
(`output_power` was the third; it shipped in 1.0.2.)

| Protocol | margo | niri | Hyprland | mango | Why margo can't ship it cleanly today |
|---|---|---|---|---|---|
| `zwlr_output_power_management_v1` | ✅ | ❌ | ✅ | ✅ | **Shipped (1.0.2).** `set_mode` maps onto the recoverable `request_dpms` deferred-queue path (`DrmCompositor::clear()`); input-wake + VT-switch both guarantee recovery |
| `wp_tearing_control_v1` | ❌ | ❌ | ✅ | ✅ | **Upstream-blocked**: smithay's `DrmCompositor` exposes no tearing / async page-flip (`FrameFlags` has no tearing variant) and the `wp_tearing_control` bindings aren't in the pinned wayland-protocols. Advertising it would be a no-op lie to clients |
| `wp_drm_lease_device_v1` | ❌ | ✅ | ✅ | ✅ | **Architecture-blocked**: `lease_request` must synchronously build a `DrmLeaseBuilder` from the live `DrmDevice`, which lives in the udev `BackendData` that `MargoState` deliberately cannot reach (the deferred-queue pattern can't help — the return is synchronous) |

margo now ships `output_power` (niri still lacks it); both margo and niri
lack `tearing`; only `drm_lease` is something niri has that margo doesn't.
None blocks daily-driver use.

## Compositor-specific protocols (informational, not gaps)

Tied to each project's own ecosystem; not counted as margo gaps.

| Protocol | Who | What it does |
|---|---|---|
| `focus_grab` | Hyprland | Internal grab semantics |
| `hyprland_lock_notify` | Hyprland | **New 0.55.0-86** — notifies clients when the session locks/unlocks (Hyprland-flavoured; not the standard `ext_lock_notify_v1`) |
| `global_shortcuts` | Hyprland | xdg-desktop-portal global-shortcuts helper |
| `toplevel_export` / `toplevel_mapping` | Hyprland | Hyprland-flavored toplevel export |
| `hyprland_surface` | Hyprland | Rounding / opacity surface hints |
| `CTM_control` | Hyprland | Color-transform-matrix (pre-`wp_color_management`) |
| `mesa_drm` | Hyprland | Compat shim for older mesa |
| `background_effect` | niri **and** Hyprland | Surface blur / effect extension |
| `mutter_x11_interop` | niri | XWayland tweak inherited from mutter |
| `xdg_output_v1` (legacy) | Hyprland **and** mango | Pre-`wl_output v4` HiDPI workaround |
| `zwlr_export_dmabuf_manager_v1` | mango only | Legacy wlroots screencast (superseded by image-copy-capture) |

## Unblocking the remaining two

`zwlr_output_power_management_v1` shipped in 1.0.2 (DPMS via the recoverable
`request_dpms` deferred-queue path). The two that remain each need a
prerequisite, not just a coding session:

1. **`wp_tearing_control_v1`** — wait for smithay's `DrmCompositor` to
   expose tearing / async page-flip (FrameFlags variant). Until then any
   advertisement is a no-op. Track upstream smithay.
2. **`wp_drm_lease_device_v1`** — needs `MargoState` to reach the udev
   `DrmDevice` synchronously (a small backend-access change to the
   deliberate State/BackendData split). Lowest value (VR / leased
   connectors), so lowest priority.

See `road_map.md` §15.10 for the full work log and blocker analysis.
