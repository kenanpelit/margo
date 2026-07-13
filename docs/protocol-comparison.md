# Wayland Protocol Surface — margo vs niri vs Hyprland vs mango

> **Correctness update (2026-07-13):** margo temporarily withdrew
> `wp_fifo_v1` and `wp_commit_timing_v1`. The Smithay bindings remain compiled,
> but their managed commit barriers were never released by Margo's presentation
> scheduler; exposing them could stall Chromium after hiding and showing a
> window. The June source-count below is retained as an historical audit of
> `v1.0.7`; the current advertised surface is therefore **55** rather than 57
> until a real FIFO/deadline scheduler lands.

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
> - **mango** `0.14.4-7` (`892d127`) — wlroots `wlr_*_create()` globals + `protocols/*.xml`; **~53 carried from the 2026-05-28 walk, not re-counted this pass** (a clean count needs manual curation to exclude non-protocol `wlr_*_create` calls such as `wlr_scene_*` / `wlr_output_layout_*`).

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
| **margo** `main` | **55** | Rust / smithay | Modern surface; **ahead of niri and mango**; pursuing Hyprland. The June source count was 57; FIFO and commit-timing are currently withheld until their managed barriers are actually driven |
| **mango** `0.13.1` | **~53** | C / wlroots | Broad-but-legacy: wlroots hands it everything for free (carried from 2026-05-28; not re-counted) |
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

The current standing picture: **margo (~55) leads its own C ancestor mango
(~53)** and niri (~41), having hand-rolled the three wlroots-freebies
mango got for free — `zwlr_foreign_toplevel_manager_v1` (write-side, P2),
`ext_workspace_v1` (P5), `zwlr_virtual_pointer_manager_v1` (P7) — on top
of the *modern* surface (HDR colour-management, content-type,
security-context, pointer-warp, xdg-dialog,
system-bell, toplevel-icon, toplevel-tag, xwayland-keyboard-grab) that
mango's wlroots base doesn't expose. `output_power` now ships (external
DPMS control). Current main additionally withholds `fifo`/`commit-timing`
until their presentation barriers are driven; `tearing_control` and
`drm_lease` remain architecture/upstream-blocked — see "remaining gaps" below.

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
| `wp_color_management_v1` (HDR) | ✅ | ❌ | ✅ | ❌ | HDR / wide-gamut output |
| `linux_drm_syncobj_v1` (explicit sync) | ✅ | ❌ | ✅ | ✅ | Tear-free GPU sync, NVIDIA |
| `ext_image_copy_capture_v1` + capture-source | ✅ | ❌ | ✅ | ✅ | Modern screencast (replaces screencopy) |
| `xwayland_shell_v1` | ✅ | ⚠️ diff path | ✅ | ❌ | HiDPI XWayland scaling |
| `wp_content_type_v1` | ✅ | ❌ | ✅ | ❌ | Game / video / photo hint |
| `wp_fifo_v1` | ❌ | ❌ | ✅ | ❌ | FIFO commit ordering; Margo binding present but global withheld until barriers are driven |
| `wp_commit_timing_v1` | ❌ | ❌ | ✅ | ❌ | Explicit commit-time targets; needs Margo deadline scheduler |
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
