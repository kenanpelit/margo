# Wayland Protocol Surface — margo vs niri vs Hyprland

> **Last refreshed:** 2026-05-15
> **Method:** Walked smithay `delegate_*!` macros + hand-rolled `GlobalDispatch` impls in margo, niri's same set, and Hyprland's `src/protocols/` directory.

This document is the source-of-truth audit of which Wayland protocols
each Rust- or C++-backed daily-driver compositor advertises. It's
maintained alongside `road_map.md` §15.10 (the work plan); this file
is the **read-side reference** for "what does each compositor
actually advertise."

## Legend

| Mark | Meaning |
|---|---|
| ✅ | shipped / advertised |
| ⚠️ | partial (e.g. read-only, no write-side) |
| ❌ | not advertised |
| 🔧 | hand-rolled (not from smithay) |
| ↺ | smithay-native (delegate macro / built-in state) |

## Headline score

| Compositor | Protocols advertised (approx.) | Note |
|---|---|---|
| **Hyprland** | ~62 | Widest surface — includes Hyprland-specific protocols (`focus_grab`, `global_shortcuts`, `toplevel_export`, `toplevel_mapping`, `hyprland_surface`) |
| **margo** | **~54** | **Passed niri** on 2026-05-15; pursuing Hyprland on standard protocols |
| **niri** | ~41 | Tightest surface; deliberately minimal |

## Core baseline — present in all three

All daily-driver Wayland compositors must ship these. No comparison
table — assume ✅ everywhere.

- `wl_compositor` / `wl_shm` / `wl_seat` / `wl_output`
- `wp_viewporter`, `wp_presentation_time`, `wp_cursor_shape_v1`,
  `wp_fractional_scale_v1`
- `zwlr_layer_shell_v1`
- `xdg_shell`, `xdg_activation_v1`, `xdg_decoration_unstable_v1`
- `linux_dmabuf_v1`
- `ext_session_lock_v1`, `ext_idle_notifier_v1`,
  `zwp_idle_inhibit_manager_v1`
- `wl_data_device`, `zwlr_data_control_manager_v1`,
  `zwp_primary_selection_device_manager_v1`
- `zwp_pointer_constraints_v1`, `zwp_relative_pointer_manager_v1`
- `zwp_text_input_manager_v3`, `zwp_input_method_manager_v2`,
  `zwp_virtual_keyboard_manager_v1`

## Recently shipped — margo's catch-up batch (this session)

Sixteen protocols added in one pass on 2026-05-15; see commits
`dc44818`, `74a0edb`, `c146aac`. All are smithay-native delegations.

| Protocol | margo | niri | Hyprland | Use case |
|---|---|---|---|---|
| `zwp_keyboard_shortcuts_inhibit_v1` | ↺ ✅ | ↺ ✅ | ✅ | VNC / RDP / VM keyboard grab |
| `zwp_pointer_gestures_v1` | ↺ ✅ | ↺ ✅ | ✅ | Touchpad pinch / swipe |
| `xdg_foreign_v2` | ↺ ✅ | ↺ ✅ | ✅ | Firefox PiP, xdg-portal screencast |
| `wp_single_pixel_buffer_v1` | ↺ ✅ | ↺ ✅ | ✅ | Solid-color buffer optimization |
| `zwp_tablet_manager_v2` | ↺ ✅ | ↺ ✅ | ✅ | Wacom / Huion drawing tablets |
| `wp_security_context_v1` | ↺ ✅ | ↺ ✅ | ✅ | Flatpak / sandboxed clients |
| `org_kde_kwin_server_decoration` | ↺ ✅ | ↺ ✅ | ✅ | Legacy Qt5 / KDE decoration |
| `wp_content_type_v1` | ↺ ✅ | ❌ | ✅ | Game / video / photo hint |
| `wp_fifo_v1` | ↺ ✅ | ❌ | ✅ | FIFO commit ordering |
| `wp_commit_timing_v1` | ↺ ✅ | ❌ | ✅ | Explicit commit-time targets |
| `wp_alpha_modifier_v1` | ↺ ✅ | ❌ | ✅ | Per-surface alpha hint |
| `xdg_wm_dialog_v1` | ↺ ✅ | ❌ | ✅ | Modal dialog hint |
| `zwp_xwayland_keyboard_grab_v1` | ↺ ✅ | ❌ | ❌ | XWayland-side kb grab (**margo unique**) |
| `xdg_toplevel_icon_v1` | ↺ ✅ | ❌ | ❌ | Inline app icons on toplevels (**margo unique**) |
| `xdg_system_bell_v1` | ↺ ✅ | ❌ | ✅ | System bell |
| `wp_pointer_warp_v1` | ↺ ✅ | ❌ | ✅ | Programmatic cursor warp |
| `xdg_toplevel_tag_v1` | ↺ ✅ | ❌ | ❌ | Semantic toplevel tags (**margo unique**) |

## Remaining gaps in margo — vs niri / Hyprland

Six protocols margo doesn't advertise that at least one of niri /
Hyprland does. All require hand-rolled work (smithay either has no
impl, or the smithay impl needs backend hookup). Deferred to a
dedicated session per project policy.

| Protocol | margo | niri | Hyprland | Cost | Daily-driver value |
|---|---|---|---|---|---|
| `zwlr_foreign_toplevel_manager_v1` (write-side) | ⚠️ list-only | 🔧 ✅ | ✅ | ~669 LOC (niri ref) | mshell bar click-to-activate / close / minimize |
| `ext_workspace_v1` | ❌ | 🔧 ✅ | ✅ | ~715 LOC (niri ref) + tag ↔ workspace semantic design | Modern workspace protocol — needed for shells that don't speak dwl-ipc |
| `zwlr_virtual_pointer_manager_v1` | ❌ | 🔧 ✅ | ✅ | ~563 LOC (niri ref) | Companion to virtual-keyboard — remote-desktop / accessibility / `wtype --click` |
| `wp_drm_lease_device_v1` | ❌ | 🔧 ✅ | ✅ | smithay-native, udev backend hookup | VR headsets (Valve Index, Vive), DP gaming displays bypass compositor |
| `zwlr_output_power_management_v1` | ❌ | ❌ | ✅ | Hand-rolled (no smithay) | Wayland-side DPMS — `wlopm`, mshell DPMS control |
| `wp_tearing_control_v1` | ❌ | ❌ | ✅ | Hand-rolled (no smithay) | Variable-refresh / immediate-page-flip for games |

## Where margo is **ahead** of niri

Eight protocols margo advertises that niri doesn't. The first four
land via margo's already-shipped HDR / screencast / explicit-sync
work; the last four landed in this session's catch-up batch.

| Protocol | margo | niri | Why niri doesn't have it |
|---|---|---|---|
| `wp_color_management_v1` | 🔧 ✅ | ❌ | niri has no HDR work yet |
| `ext_image_capture_source_v1` family | ↺ ✅ | ❌ | niri uses screencopy-only |
| `linux_drm_syncobj_v1` (explicit sync) | ↺ ✅ | ❌ | niri pending |
| `xwayland_shell_v1` (HiDPI XWayland scaling) | ↺ ✅ | (different mechanism) | niri reaches it via a different code path |
| `wp_content_type_v1` | ↺ ✅ | ❌ | This session |
| `wp_fifo_v1` + `wp_commit_timing_v1` | ↺ ✅ | ❌ | This session |
| `wp_alpha_modifier_v1` | ↺ ✅ | ❌ | This session |
| `xdg_wm_dialog_v1` | ↺ ✅ | ❌ | This session |

## Where margo is **ahead** of both niri AND Hyprland

Three protocols margo advertises that **neither** of the other two
do. All three landed in this session's bonus batch (`c146aac`).

| Protocol | Reason it matters |
|---|---|
| `zwp_xwayland_keyboard_grab_v1` | X11-side keyboard grab. Complements `keyboard_shortcuts_inhibit_v1` — same VNC / VM / remote-desktop story via the XWayland mechanism. |
| `xdg_toplevel_icon_v1` | Toplevels ship their own inline PNG / SVG icon instead of the bar inferring from `.desktop`. mshell taskbar consumer is the natural next step. |
| `xdg_toplevel_tag_v1` | Semantic tags + description strings on toplevels — feeds window-rule matching once a UI consumer is wired up. |

## Hyprland-specific protocols (informational, not gaps)

Protocols Hyprland ships that are tied to its plugin / ecosystem
model and don't have an obvious margo use-case. Listed here so they
don't show up as "missing" in future audits.

| Protocol | What it does |
|---|---|
| `focus_grab` | Hyprland-internal grab semantics |
| `global_shortcuts` | xdg-desktop-portal global-shortcuts helper |
| `toplevel_export` / `toplevel_mapping` | Hyprland-flavored toplevel-export protocol |
| `hyprland_surface` | Hyprland-flavored surface extensions (rounding hints, opacity) |
| `xdg_output_v1` (legacy) | Pre-`wl_output v4` HiDPI workaround — most apps don't need it |
| `mesa_drm` | Compatibility shim Hyprland keeps for older mesa |
| `CTM_control` | Hyprland-specific color-transform-matrix control (different mechanism from `wp_color_management_v1`) |

## Niri-specific protocols (informational)

Protocols niri ships that are niri-internal. Not gaps for margo.

| Protocol | What it does |
|---|---|
| `background_effect` | niri's own surface effects extension |
| `mutter_x11_interop` | XWayland tweak niri inherits from mutter |

## Sequencing — when to take the remaining six

The deferred six are intentionally back-loaded. From most useful to
least, given margo's current trajectory:

1. **`zwlr_foreign_toplevel_manager_v1`** (write-side) — gates
   mshell bar click-to-activate. Highest UX leverage.
2. **`ext_workspace_v1`** — lets shells that don't speak dwl-ipc
   (sfwbar, ironbar) show margo workspaces. Needs a semantics design
   pass (tags ↔ workspaces).
3. **`zwlr_virtual_pointer_manager_v1`** — pairs with the already-
   shipped virtual-keyboard. Unlocks accessibility tooling.
4. **`zwlr_output_power_management_v1`** — Wayland-side DPMS so
   mshell can blank outputs without going through libdrm directly.
5. **`wp_tearing_control_v1`** — only matters for game-oriented users;
   parallels HDR Phase 2 work in tone (perf-tuning).
6. **`wp_drm_lease_device_v1`** — VR headsets / gaming displays.
   Smithay does the protocol; the cost is udev-backend connector
   exposure, which is a different code area.

See `road_map.md` §15.10 for the work plan and rationale.
