# Compositor feature plans — blur · per-output frame clock · tabbed groups · output power-mgmt

> Status 2026-06-08: `mctl plugin list/enable/disable` shipped (small-wins #1).
> The four below are each a multi-session compositor feature — specced here so
> they can be executed carefully (fresh context, on-hardware where DRM is
> involved), not hacked in. Order = ascending blast radius.

---

## A. Output power management (`zwlr_output_power_management_v1`)

**Goal:** let external clients (wlr-randr, idle daemons) DPMS-off/on an output;
margo already has an internal `mctl dispatch dpms` path.

**Why it's last-touched / risky:** the DRM mode-off/on path is untested and a bug
black-screens the live session. Must be iterated **on hardware**, with a watchdog
(auto-restore after N seconds if no ack).

**Architecture:** smithay has no delegate for this protocol → hand-roll the global
+ `Dispatch` for `zwlr_output_power_manager_v1` / `_mode_v1`, mapping `set_mode`
to the existing DPMS routine on the `DrmDevice` (lives in udev `BackendData`,
reachable via the deferred-queue pattern, but the call is effectively async —
queue a `BackendCommand::SetDpms{output, on}`).

**Tasks:** (1) protocol handler + global (gated behind udev backend, like dmabuf).
(2) `BackendCommand::SetDpms` + handler that calls the DRM off/on. (3) re-emit
mode events on hotplug. (4) on-hardware test: off → black, on → restores, no
hang; multi-output; lock-screen interaction. (5) protocol-comparison.md flip.

---

## B. Per-output frame clock

**Goal:** pace each output by its own refresh (60 Hz + 144 Hz independent),
replacing the single global tick (road_map §7).

**Architecture:** today one calloop timer drives all outputs. Move to per-`Output`
`next_frame_at` scheduling: each output schedules its own present timer off its
last vblank; the render loop renders only the outputs that are due. Smithay's
`DrmCompositor` already gives per-surface vblank events — wire each output's
`frame_submitted` → schedule its next tick at `last_present + refresh_interval`.

**Tasks:** (1) move the frame-pacing state from global into `Output`/`MargoMonitor`.
(2) per-output present timer (calloop `Timer` per output, re-armed on vblank).
(3) animation tick: sample animations per-output at that output's cadence (the
spring/bezier clocks already carry per-instance state — feed them the per-output
dt). (4) damage/repaint only the due output. (5) tests: two synthetic outputs at
different refresh; assert independent tick counts. (6) on-hardware mixed-Hz check
(no global stutter, 144 Hz monitor smooth while 60 Hz idles).

**Risk:** medium-high — touches the render loop's heartbeat. Regression = stutter
or a stalled output. Keep the global tick behind a config/feature flag during
bring-up.

---

## C. Blur (Kawase, dual-filter)

**Goal:** real background blur behind translucent windows / layer-shells (the
config flags `blur` / `blur_layer` / `blur_params_*` and the `LYR_BLUR` layer
constant + `no_blur` window rule already exist; **the render pass does not**).

**Architecture (niri/Hyprland pattern):** dual-Kawase on the GLES renderer.
Per frame, for each blur region: (1) capture the framebuffer region behind the
window into an offscreen FBO; (2) downsample N passes (`blur_params_num_passes`),
each a Kawase down-filter at half-res; (3) upsample N passes; (4) composite the
blurred texture under the translucent surface, clipped to its rounded-rect.
`blur_params_{radius,brightness,contrast,saturation,noise}` feed the shader.
FBO ping-pong (the exact thing `shadow.rs` notes it deliberately avoids today).

**Tasks:** (1) `render/blur.rs`: two GLES programs (down/up Kawase) + an FBO pool.
(2) hook into the element render order: blur regions resolved from translucent
toplevels/layers minus `no_blur`. (3) damage: blur only invalidated regions
(naive = whole-output blur each frame; optimise later with
`blur_optimized`). (4) wire `blur` (windows) vs `blur_layer` (layer-shells) +
`layerrule noblur`. (5) perf budget on integrated GPU; (6) artifact check at
output edges / overlapping windows. (7) Settings → Effects toggle already exists
for shadows — extend for blur.

**Risk:** medium — contained to render, but perf + artifacts need iteration. No
DRM risk. Biggest visible payoff.

---

## D. Tabbed window groups

**Goal:** Hyprland-style `togglegroup` — merge windows into one tile with a tab
bar; cycle/move within the group. margo has `Deck` (stacked, no tabs).

**Architecture:** add group identity to the client model (`group_id: Option<u32>`
+ per-group `active` index + order). A grouped set occupies ONE layout slot
(reuse the Deck rect); only the active member renders full, the rest hidden. A
**tab strip** (compositor-drawn, like the border/shadow chrome, or a thin layer)
renders one chip per member at the tile's top, highlighting active. Dispatch:
`togglegroup` (group/ungroup focused with neighbour), `changegroupactive
next|prev`, `movewindowtogroup`, `lockgroups`. Input: click a tab → activate;
scroll on tab strip → cycle.

**Tasks:** (1) state: `group_id` + group registry + active tracking. (2) layout:
grouped clients collapse to one slot (Deck rect); arrange skips inactive members.
(3) render: tab-strip chrome (margo SDF chrome pattern) + active highlight (matugen
colours). (4) dispatch verbs + `mctl actions` docs. (5) input: tab click / scroll.
(6) config: `group_*` knobs (bar height, colours via matugen). (7) windowrule:
`group:1` to auto-group by class. (8) tests: group/ungroup invariants, single-slot
cardinality, active-cycle wrap. (9) config.example.conf + site docs.

**Risk:** high (surface area), but no DRM risk. Largest feature — do it last, its
own multi-step session via subagent-driven-development.

---

## Suggested execution order

1. **C. Blur** — most visible, render-contained, no DRM risk.
2. **B. Per-output frame clock** — correctness for mixed-Hz; render-loop care.
3. **D. Tabbed groups** — biggest; its own focused build.
4. **A. Output power-mgmt** — last, **on-hardware** with a black-screen watchdog.
