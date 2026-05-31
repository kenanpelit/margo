# Protocol verification matrix — advertised vs implemented vs tested

> **Last refreshed:** 2026-06-01 (`main`)
> **Companion doc:** [`protocol-comparison.md`](protocol-comparison.md) answers
> *"what does each compositor advertise?"* (cross-project surface count).
> **This** doc answers a different, internal question: *"for every protocol
> **margo** advertises, how do we know it actually works?"* — the gap between
> "the global is in the registry" and "a client round-trips against it in CI."

This is a maturity ledger, not a marketing sheet. A protocol can be
`advertised ✅` and `implemented ✅` yet have **no automated coverage** — that
is exactly the row this table exists to surface, so the snapshot/integration
backlog (`road_map.md` §15.2, test coverage) is driven by evidence, not vibes.

## How the columns are derived (so they stay honest)

| Column | ✅ means | Source of truth |
|---|---|---|
| **Advertised** | global bound into the registry at startup | `State::new` in `margo/src/state.rs` (the `*State::new::<Self>(&dh)` calls) + the `delegate_*!` macro set |
| **Implemented** | real handler logic, not an advertise-only stub | the `*Handler` impls / `margo/src/protocols/*.rs` for hand-rolled ones; ⚠️ = partial (read-only, or a documented caveat) |
| **Runtime-tested** | a live Wayland client exercises it | 🟢 headless integration fixture in `margo/src/tests/` (real `MargoState` + nested client over a `UnixStream`, run in CI); 🟡 manual on-hardware only ([`manual-checklist.md`](manual-checklist.md)); ❌ neither |
| **Unit / snapshot-tested** | pure-unit or `insta` snapshot coverage | `#[test]` in the owning module / `insta::assert_snapshot!`; ❌ = none |
| **Known gaps** | the caveat a maintainer must know | — |

**Why two test columns.** They catch different bug classes, and margo runs
both halves (see `margo/src/tests/fixture.rs` for the rationale, paraphrasing
niri's "~5,280 snapshots *and* a fixture harness"):

- **Unit / snapshot** locks *pure logic* — layout arithmetic, LUT generation,
  schedule math. The layout engine alone has **38 `insta` text snapshots**
  (`margo/src/layout/snapshots/`) plus the colour/twilight unit suites
  (`render/icc_lut`, `render/hdr_metadata`, `twilight/*`). These don't touch
  the wire.
- **Runtime (integration fixture)** locks *protocol behaviour over a real
  connection* — `wl_registry.bind`, `wl_display.sync`, server dispatch order,
  configure-event sequencing. This is the only column that proves the
  advertised global is wired to a working handler.

🟢 (CI integration) is strictly stronger evidence than 🟡 (manual): 🟢 runs on
every push and can't silently rot; 🟡 depends on a human walking the checklist
on real DRM hardware that GitHub runners can't provide.

## Legend

| Mark | Meaning |
|---|---|
| ✅ | yes / shipped |
| ⚠️ | partial — see the gap note |
| 🟢 | covered by a CI headless-integration fixture test |
| 🟡 | covered only by the manual on-hardware checklist |
| ❌ | not covered |

---

## Core baseline

The registry globals every Wayland client expects. `globals.rs` (3 tests)
asserts the advertised set, so the *advertisement* of this whole block is
runtime-checked even where the per-protocol behaviour isn't.

| Protocol | Advertised | Implemented | Runtime-tested | Unit/snapshot | Known gaps |
|---|---|---|---|---|---|
| `wl_compositor` / `wl_subcompositor` | ✅ | ✅ | 🟢 `globals.rs` | ❌ | — |
| `wl_shm` | ✅ | ✅ | 🟢 `globals.rs` | ❌ | — |
| `wl_seat` | ✅ | ✅ | 🟢 `globals.rs` | ❌ | seat capability/focus paths exercised indirectly via `xdg_shell.rs` |
| `wl_output` | ✅ | ✅ | 🟢 `globals.rs`, `output_management.rs` | ❌ | — |
| `wp_viewporter` | ✅ | ✅ | 🟢 `globals.rs` | ❌ | advertisement only; crop/scale path not asserted |
| `wp_presentation_time` | ✅ | ✅ | ❌ | ❌ | no feedback-event test; relies on frame loop |
| `wp_cursor_shape_v1` | ✅ | ✅ | ❌ | ❌ | named-cursor mapping untested |
| `wp_fractional_scale_v1` | ✅ | ✅ | 🟡 (HiDPI in checklist) | ❌ | scale-factor event not asserted in fixture |

## xdg shell family

| Protocol | Advertised | Implemented | Runtime-tested | Unit/snapshot | Known gaps |
|---|---|---|---|---|---|
| `xdg_shell` (`xdg_wm_base`) | ✅ | ✅ | 🟢 `xdg_shell.rs` (6) | ❌ | strongest-covered shell path (map / configure / ack) |
| `xdg_decoration_unstable_v1` | ✅ | ✅ | 🟢 `xdg_decoration.rs` (2) | ❌ | — |
| `xdg_activation_v1` | ✅ | ✅ | 🟢 `xdg_activation.rs` (2) | ❌ | — |
| `xdg_foreign_v2` | ✅ | ✅ | ❌ | ❌ | export/import handle round-trip untested |
| `xdg_wm_dialog_v1` | ✅ | ✅ | ❌ | ❌ | modal hint honoured by window rules, not asserted |
| `xdg_system_bell_v1` | ✅ | ✅ | ❌ | ❌ | bell event reaches handler; no automated probe |
| `xdg_toplevel_icon_v1` | ✅ | ✅ | ❌ | ❌ | **margo near-unique** (see comparison doc); no test |
| `xdg_toplevel_tag_v1` | ✅ | ✅ | ❌ | ❌ | semantic tag plumbing; no test |
| `org_kde_kwin_server_decoration` (KDE) | ✅ | ✅ | ❌ | ❌ | legacy decoration negotiation; covered de-facto by xdg_decoration |

## Shells & surfaces (non-xdg)

| Protocol | Advertised | Implemented | Runtime-tested | Unit/snapshot | Known gaps |
|---|---|---|---|---|---|
| `zwlr_layer_shell_v1` | ✅ | ✅ | 🟢 `layer_shell.rs` (5) + 🟡 | ❌ | well-covered: anchor/exclusive-zone/popup paths |
| `ext_session_lock_v1` | ✅ | ✅ | 🟢 `session_lock.rs` (2) + 🟡 | ❌ | stuck-lock recovery is 🟡 manual only |
| `xwayland_shell_v1` | ✅ | ✅ | 🟢 `x11.rs` (2) | ❌ | HiDPI XWayland scale path is 🟡 manual |
| `wp_single_pixel_buffer_v1` | ✅ | ✅ | ❌ | ❌ | trivially correct; no probe |
| `wp_alpha_modifier_v1` | ✅ | ✅ | ❌ | ❌ | per-surface alpha applied in render; not asserted |
| `wp_content_type_v1` | ✅ | ✅ | ❌ | ❌ | hint stored; consumed by present path, untested |

## Input

| Protocol | Advertised | Implemented | Runtime-tested | Unit/snapshot | Known gaps |
|---|---|---|---|---|---|
| `zwp_pointer_constraints_v1` | ✅ | ✅ | 🟢 `pointer_constraints.rs` (2) | ❌ | lock/confine region |
| `zwp_relative_pointer_manager_v1` | ✅ | ✅ | ❌ | ❌ | relative-motion event untested (paired with constraints in games) |
| `zwp_pointer_gestures_v1` | ✅ | ✅ | ❌ | ❌ | swipe/pinch/hold forwarding untested |
| `wp_pointer_warp_v1` | ✅ | ✅ | ❌ | ❌ | programmatic warp; no probe |
| `zwp_text_input_manager_v3` | ✅ | ✅ | ❌ | ❌ | IME path needs an input-method peer to test |
| `zwp_input_method_manager_v2` | ✅ | ✅ | ❌ | ❌ | as above |
| `zwp_virtual_keyboard_manager_v1` | ✅ | ✅ | ❌ | ❌ | synthetic key injection untested |
| `zwlr_virtual_pointer_manager_v1` | ✅ | ✅ | ❌ | ❌ | hand-rolled (`protocols/virtual_pointer.rs`); feeds normal input path |
| `zwp_tablet_manager_v2` | ✅ | ✅ | ❌ | ❌ | no tablet in CI |
| `zwp_keyboard_shortcuts_inhibit_v1` | ✅ | ✅ | ❌ | ❌ | inhibit grant/revoke untested |
| `zwp_xwayland_keyboard_grab_v1` | ✅ | ✅ | ❌ | ❌ | **margo-unique**; X11-side grab, hard to fixture |

## Output / colour / capture

| Protocol | Advertised | Implemented | Runtime-tested | Unit/snapshot | Known gaps |
|---|---|---|---|---|---|
| `zwlr_output_manager_v1` | ✅ | ✅ | 🟢 `output_management.rs` (2) | ❌ | apply scale/transform/position; multi-output assignment is 🟡 manual |
| `wp_color_management_v1` (HDR) | ✅ | ✅ | 🟢 `color_management.rs` (2) | ✅ `render/icc_lut` (6), `render/hdr_metadata` (5), `render/linear_composite` (8) | best dual-covered protocol; see [`hdr-design.md`](hdr-design.md) |
| `zwlr_gamma_control_manager_v1` | ✅ | ✅ | 🟢 `gamma_control.rs` (3) + 🟡 | ✅ `twilight/*` (gamma LUT / schedule / interpolation, ~28) | day-night shift is 🟡 manual |
| `zwlr_screencopy_manager_v1` | ✅ | ✅ | 🟢 `screencopy.rs` (2) + 🟡 | ✅ `screenshot_region.rs` (14) | hand-rolled (`protocols/screencopy.rs`) |
| `ext_image_copy_capture_v1` (+ capture-source) | ✅ | ✅ | ❌ | ❌ | modern capture; output/toplevel source globals advertised, capture loop only manually exercised |
| `linux_dmabuf_v1` | ✅ | ✅ | 🟢 `dmabuf.rs` (3) | ❌ | format/modifier advertisement asserted |
| `linux_drm_syncobj_v1` | ✅ | ✅ | ❌ | ❌ | explicit-sync; needs real GPU timeline |
| `wp_fifo_v1` | ✅ | ✅ | ❌ | ❌ | FIFO commit ordering; no probe |
| `wp_commit_timing_v1` | ✅ | ✅ | ❌ | ❌ | commit-time targets; no probe |

## Idle / power / session

| Protocol | Advertised | Implemented | Runtime-tested | Unit/snapshot | Known gaps |
|---|---|---|---|---|---|
| `ext_idle_notifier_v1` | ✅ | ✅ | 🟢 `idle.rs` (3) | ❌ | — |
| `zwp_idle_inhibit_manager_v1` | ✅ | ✅ | 🟢 `idle.rs` (3) | ❌ | inhibitor create/destroy |
| `wp_security_context_v1` | ✅ | ✅ | ❌ | ❌ | sandbox/flatpak context; no probe |

## Selection / clipboard

| Protocol | Advertised | Implemented | Runtime-tested | Unit/snapshot | Known gaps |
|---|---|---|---|---|---|
| `wl_data_device` | ✅ | ✅ | 🟢 `selection.rs` (2) + 🟡 | ❌ | — |
| `zwlr_data_control_manager_v1` | ✅ | ✅ | 🟢 `selection.rs` + 🟡 | ❌ | clipboard-manager path (CopyQ/clipse) is 🟡 manual |
| `ext_data_control_manager_v1` | ✅ | ✅ | 🟢 `selection.rs` + 🟡 | ❌ | standardised successor to wlr data-control |
| `zwp_primary_selection_device_manager_v1` | ✅ | ✅ | 🟢 `selection.rs` | ❌ | middle-click paste |

## Workspace / toplevel management

| Protocol | Advertised | Implemented | Runtime-tested | Unit/snapshot | Known gaps |
|---|---|---|---|---|---|
| `ext_foreign_toplevel_list_v1` (read) | ✅ | ✅ | ❌ | ❌ | consumed by mshell active-window pill; covered de-facto at runtime, no fixture |
| `zwlr_foreign_toplevel_manager_v1` (write) | ✅ | ✅ | 🟡 (taskbar in checklist) | ❌ | hand-rolled (`protocols/wlr_foreign_toplevel.rs`); activate/close/fullscreen |
| `ext_workspace_v1` | ✅ | ✅ | ❌ | ❌ | hand-rolled (`protocols/ext_workspace.rs`); 9 fixed tag-workspaces per output |
| `dwl_ipc_unstable_v2` (custom) | ✅ | ✅ | 🟡 (bring-up §0 in checklist) | ❌ | margo↔mctl/mshell; `state.json` snapshot is the primary mshell bridge |

---

## Coverage summary

- **Advertised & implemented:** every protocol in this matrix (no advertise-only
  stubs — `⚠️` rows are *partial behaviour*, not missing handlers).
- **Runtime-tested in CI (🟢):** 16 protocol areas via `margo/src/tests/`
  (`globals`, `xdg_shell`, `xdg_decoration`, `xdg_activation`, `layer_shell`,
  `session_lock`, `xwayland_shell`, `pointer_constraints`, `output_management`,
  `color_management`, `gamma_control`, `screencopy`, `dmabuf`, `idle` ×2,
  `selection`).
- **Unit / snapshot:** concentrated in the **render/colour/twilight** stack and
  the **layout engine** (38 layout snapshots) — pure-logic surfaces. The wire
  protocols mostly rely on the integration fixture instead, by design.
- **🟡 manual-only / ❌ untested:** the long tail (input extensions, capture
  loop, sync/timing, security-context, foreign-toplevel/workspace). These are
  the **snapshot/integration backlog**.

### Next integration-fixture targets (the riskiest untested paths)

Ordered by blast radius, matching the agreed first wave:

1. **tag move across outputs** (`tagmon` / multi-monitor output assignment) —
   currently 🟡 manual only; highest-risk because it mutates per-output `Pertag`.
2. **layer-shell popup / menu** grab + dismiss — fixture has the surface, not the
   popup-grab path.
3. **focus restore after unmap / lock** — `session_lock.rs` covers lock, not the
   post-unlock focus stack.
4. **floating-over-tiled** stacking + focus.
5. **scroller offscreen focus** (focus a window scrolled out of view) — the
   overview has 6 tests; the offscreen-focus scroll-into-view path is separate.

See `road_map.md` §15.2 (test coverage) for the running backlog; this matrix is the
evidence table that feeds it.
