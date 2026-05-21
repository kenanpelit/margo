# Wayland Protocol Surface — margo vs niri vs Hyprland vs mango

> **Last refreshed:** 2026-05-21
> **Sources walked (all at that day's `HEAD`):**
> - **margo** — this repo (smithay `delegate_*!` macros + hand-rolled `GlobalDispatch`).
> - **niri** `26.4.0` (`4294948`) — same smithay method.
> - **Hyprland** `0.55.0` (`v0.55.0-55-g95d9ae2`) — `src/protocols/*.cpp` + `src/protocols/core/`.
> - **mango** `0.13.1` (`0.13.1-19-gda1e1ca`) — wlroots `wlr_*_create()` call sites + `protocols/*.xml`.

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
| **Hyprland** `0.55.0` | **~67** | C++ (hand-rolled) | Widest surface — 61 protocol modules + 6 core, plus Hyprland-only extensions |
| **margo** | **~54** | Rust / smithay | Modern surface; **passed niri**, **drew level with mango** on a different protocol mix |
| **mango** `0.13.1` | **~53** | C / wlroots | Broad-but-legacy: wlroots hands it everything, including protocols margo still lacks |
| **niri** `26.4.0` | **~41** | Rust / smithay | Tightest surface; deliberately minimal |

The headline story shifted this refresh: **margo (~54) has effectively
drawn level with its own C ancestor mango (~53) in raw count — but with
a deliberately different mix.** margo carries the *modern* protocol
surface (HDR colour-management, content-type, fifo/commit-timing,
security-context, pointer-warp, xdg-dialog, system-bell, toplevel-icon,
toplevel-tag, xwayland-keyboard-grab) that mango's wlroots base does not
expose, while mango still wins the *legacy wlroots freebies* (tearing,
drm-lease, virtual-pointer, output-power, ext-workspace,
foreign-toplevel write-side, export-dmabuf) that margo has not wired up
yet. The remaining six margo gaps are, almost exactly, that
wlroots-freebie set.

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
| `wp_fifo_v1` | ✅ | ❌ | ✅ | ❌ | FIFO commit ordering |
| `wp_commit_timing_v1` | ✅ | ❌ | ✅ | ❌ | Explicit commit-time targets |
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

## The remaining six margo gaps — = the wlroots freebies

Six protocols margo doesn't (fully) advertise. The pattern is now
clear: **these are exactly the protocols wlroots hands mango and
Hyprland for free, plus the two niri also hand-rolled.** smithay either
has no impl or needs backend hookup, so margo has deferred them to a
dedicated session.

| Protocol | margo | niri | Hyprland | mango | Daily-driver value |
|---|---|---|---|---|---|
| `zwlr_foreign_toplevel_manager_v1` (write-side) | ⚠️ list-only | ✅ | ✅ | ✅ | mshell bar click-to-activate / close / minimize |
| `ext_workspace_v1` | ❌ scaffold | ✅ | ✅ | ✅ | Workspace protocol for shells that don't speak dwl-ipc |
| `zwlr_virtual_pointer_manager_v1` | ❌ | ✅ | ✅ | ✅ | Remote-desktop / accessibility / `wtype --click` |
| `zwlr_output_power_management_v1` | ❌ | ❌ | ✅ | ✅ | Wayland-side DPMS — `wlopm`, mshell blank-output |
| `wp_tearing_control_v1` | ❌ | ❌ | ✅ | ✅ | Immediate-page-flip for games |
| `wp_drm_lease_device_v1` | ❌ | ✅ | ✅ | ✅ | VR headsets / DP gaming displays |

- `ext_workspace_v1` exists as an empty scaffold in margo
  (`protocols/ext_workspace.rs`) — state struct only, no global; tag
  state is exposed via `dwl-ipc-unstable-v2` instead.
- `zwlr_foreign_toplevel_manager_v1` is **read-side only** in margo: it
  advertises `ext-foreign-toplevel-list-v1` (the list), not the wlr
  write-side manager with `activate`/`close`/`set_minimized` requests.

## Compositor-specific protocols (informational, not gaps)

Tied to each project's own ecosystem; not counted as margo gaps.

| Protocol | Who | What it does |
|---|---|---|
| `focus_grab` | Hyprland | Internal grab semantics |
| `global_shortcuts` | Hyprland | xdg-desktop-portal global-shortcuts helper |
| `toplevel_export` / `toplevel_mapping` | Hyprland | Hyprland-flavored toplevel export |
| `hyprland_surface` | Hyprland | Rounding / opacity surface hints |
| `CTM_control` | Hyprland | Color-transform-matrix (pre-`wp_color_management`) |
| `mesa_drm` | Hyprland | Compat shim for older mesa |
| `background_effect` | niri **and** Hyprland | Surface blur / effect extension |
| `mutter_x11_interop` | niri | XWayland tweak inherited from mutter |
| `xdg_output_v1` (legacy) | Hyprland **and** mango | Pre-`wl_output v4` HiDPI workaround |
| `zwlr_export_dmabuf_manager_v1` | mango only | Legacy wlroots screencast (superseded by image-copy-capture) |

## Sequencing — when to take the remaining six

Back-loaded by design. Most-useful-first, given margo's trajectory:

1. **`zwlr_foreign_toplevel_manager_v1`** (write-side) — gates mshell
   bar click-to-activate. Highest UX leverage; niri's ~669-LOC impl is
   the reference.
2. **`ext_workspace_v1`** — lets non-dwl-ipc shells (sfwbar, ironbar)
   show margo workspaces. Needs a tags ↔ workspaces semantics pass.
3. **`zwlr_virtual_pointer_manager_v1`** — pairs with the already-
   shipped virtual-keyboard; unlocks accessibility tooling.
4. **`zwlr_output_power_management_v1`** — Wayland-side DPMS so mshell
   can blank outputs without libdrm.
5. **`wp_tearing_control_v1`** — game-oriented users only.
6. **`wp_drm_lease_device_v1`** — VR / gaming displays; smithay does the
   protocol, cost is udev-backend connector exposure.

See `road_map.md` §15.10 for the work plan and rationale.
