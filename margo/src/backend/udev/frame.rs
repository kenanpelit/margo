//! Per-frame render dispatch + presentation-feedback bookkeeping.
//!
//! This module owns the `render_output` hot path: pull the snapshot
//! captures, build the element list, serve any pending screencopies,
//! call `DrmCompositor::render_frame` + `queue_frame`, and stash the
//! `wp_presentation_feedback` builder so the matching VBlank can
//! signal `presented(now, refresh, 0, Vsync)` with a real page-flip
//! timestamp instead of the submit-time approximation.
//!
//! Helpers live in `helpers.rs`; the actual element builders
//! (`build_render_elements`, `serve_screencopies`,
//! `build_cursor_elements_for_output`, `take_pending_*`) still live
//! in `udev/mod.rs` because they reach into `MargoState` quite
//! deeply — extracting them is a separate W-pass.

use std::collections::HashMap;

use smithay::{
    backend::{
        drm::{
            DrmDevice,
            compositor::{FrameFlags, PrimaryPlaneElement},
        },
        renderer::gles::GlesRenderer,
    },
    output::Output,
    reexports::drm::control::crtc,
    wayland::seat::WaylandFocus,
};
use tracing::{error, warn};

use super::{
    MargoRenderElement, OutputDevice,
    helpers::output_refresh_duration,
    render_elements::{build_mirror_frame, build_output_scene},
    serve_screencopies, take_pending_open_close_captures, take_pending_snapshots,
};
use crate::state::MargoState;

/// Refresh per-client `last_scanout` after a successful `render_frame`.
/// A client is on direct-scanout when *any* of its surfaces (toplevel
/// + subsurfaces) appears in `RenderElementStates::states` with
/// `ZeroCopy` presentation. ZeroCopy is smithay's signal that the
/// buffer went straight to a primary or overlay plane — composition
/// skipped, the client rendered nothing.
///
/// Surfaces are looked up by `Id::from_wayland_resource(&wl_surface)`,
/// the same id `WaylandSurfaceRenderElement` constructs at render
/// time. Clients on a different monitor than `output` are left alone —
/// their flag is updated when their own monitor renders.
/// Mirror niri's `update_primary_scanout_output` — for every visible
/// surface (windows, layer surfaces, popups), stamp its `data_map`
/// with the output it was just presented on. `send_frame_callbacks`
/// reads this stamp via `surface_primary_scanout_output` to decide
/// where to send the `wl_surface.frame` done event AND to skip
/// surfaces that didn't make it to the scanout. Without this update,
/// smithay's `surface_primary_scanout_output` ALWAYS returns `None`,
/// which is why margo had to bypass the primary-scanout filter
/// entirely — a workaround that also blocked `wp_presentation_feedback`
/// from going to clients, hurt damage tracking, and contributed to
/// the bar flicker. niri does the same pass in `niri.rs` lines
/// 4793-4922.
pub(super) fn update_primary_scanout_outputs(
    state: &mut MargoState,
    output: &Output,
    render_states: &smithay::backend::renderer::element::RenderElementStates,
) {
    use smithay::backend::renderer::element::default_primary_scanout_output_compare;
    use smithay::desktop::utils::update_surface_primary_scanout_output;
    use smithay::wayland::compositor::{TraversalAction, with_surface_tree_downward};

    for window in state.space.elements_for_output(output) {
        window.with_surfaces(|surface, states| {
            update_surface_primary_scanout_output(
                surface,
                output,
                states,
                None,
                render_states,
                default_primary_scanout_output_compare,
            );
        });
    }

    let map = smithay::desktop::layer_map_for_output(output);
    for layer in map.layers() {
        layer.with_surfaces(|surface, states| {
            update_surface_primary_scanout_output(
                surface,
                output,
                states,
                None,
                render_states,
                // Layer surfaces are output-pinned; always pick this output.
                |_, _, output, _| output,
            );
        });
        // Popups attached to the layer surface (rare: tooltips, menus).
        for (popup, _) in smithay::desktop::PopupManager::popups_for_surface(layer.wl_surface()) {
            let s = popup.wl_surface();
            with_surface_tree_downward(
                s,
                (),
                |_, _, _| TraversalAction::DoChildren(()),
                |surface, states, _| {
                    update_surface_primary_scanout_output(
                        surface,
                        output,
                        states,
                        None,
                        render_states,
                        |_, _, output, _| output,
                    );
                },
                |_, _, _| true,
            );
        }
    }

    // Lock surface, if any. Locked sessions have a dedicated lock
    // surface per output that we want to stamp so its presentation
    // feedback works correctly (mlock relies on this).
    if let Some((_, lock_surface)) = state.lock_surfaces.iter().find(|(o, _)| o == output) {
        with_surface_tree_downward(
            lock_surface.wl_surface(),
            (),
            |_, _, _| TraversalAction::DoChildren(()),
            |surface, states, _| {
                update_surface_primary_scanout_output(
                    surface,
                    output,
                    states,
                    None,
                    render_states,
                    default_primary_scanout_output_compare,
                );
            },
            |_, _, _| true,
        );
    }
}

pub(super) fn update_client_scanout_flags(
    state: &mut MargoState,
    output: &Output,
    render_states: &smithay::backend::renderer::element::RenderElementStates,
) {
    use smithay::backend::renderer::element::{Id, RenderElementPresentationState};
    use smithay::wayland::compositor::with_surface_tree_downward;

    let Some(out_idx) = state.monitors.iter().position(|m| &m.output == output) else {
        return;
    };

    let active_tagset = state.monitors[out_idx].current_tagset();

    for client in state.clients.iter_mut() {
        if client.monitor != out_idx {
            continue;
        }
        if !client.is_visible_on(out_idx, active_tagset) {
            client.last_scanout = false;
            continue;
        }
        let Some(root) = client.window.wl_surface().map(|s| s.into_owned()) else {
            // X11 or unmapped surface — direct scanout doesn't apply.
            client.last_scanout = false;
            continue;
        };
        let mut on_scanout = false;
        with_surface_tree_downward(
            &root,
            (),
            |_, _, _| smithay::wayland::compositor::TraversalAction::DoChildren(()),
            |surface, _, _| {
                if on_scanout {
                    return;
                }
                let id: Id = Id::from_wayland_resource(surface);
                if let Some(s) = render_states.element_render_state(id) {
                    if matches!(
                        s.presentation_state,
                        RenderElementPresentationState::ZeroCopy
                    ) {
                        on_scanout = true;
                    }
                }
            },
            |_, _, _| true,
        );
        client.last_scanout = on_scanout;
    }
}

/// Collect every surface's `wp_presentation_feedback` callback into a
/// single `OutputPresentationFeedback` builder. The builder is stashed
/// on the `OutputDevice` until the matching `DrmEvent::VBlank` fires,
/// then signalled with the real page-flip timestamp via
/// [`flush_presentation_feedback`].
pub(super) fn build_presentation_feedback(
    output: &Output,
    state: &mut MargoState,
    render_states: &smithay::backend::renderer::element::RenderElementStates,
) -> smithay::desktop::utils::OutputPresentationFeedback {
    use smithay::desktop::layer_map_for_output;
    use smithay::desktop::utils::{
        OutputPresentationFeedback, surface_presentation_feedback_flags_from_states,
        surface_primary_scanout_output,
    };

    let mut feedback = OutputPresentationFeedback::new(output);

    // Toplevels.
    for window in state.space.elements_for_output(output) {
        window.take_presentation_feedback(
            &mut feedback,
            surface_primary_scanout_output,
            |surface, _| {
                surface_presentation_feedback_flags_from_states(surface, None, render_states)
            },
        );
    }
    // Layer surfaces (bar, notifications, OSD).
    let map = layer_map_for_output(output);
    for layer_surface in map.layers() {
        layer_surface.take_presentation_feedback(
            &mut feedback,
            surface_primary_scanout_output,
            |surface, _| {
                surface_presentation_feedback_flags_from_states(surface, None, render_states)
            },
        );
    }

    feedback
}

/// Signal `presented(time, refresh, seq, Vsync)` on a feedback builder
/// previously stashed on the OutputDevice. Called from the
/// `DrmEvent::VBlank` handler with `time` = the kernel's page-flip
/// timestamp (CLOCK_MONOTONIC) from the event metadata, falling back to
/// the handler's own clock reading when the device can't provide one.
///
/// `seq` is `OutputDevice::vblank_seq`: the kernel's vblank counter
/// when the event carried one, kept monotonic by
/// [`crate::frame_clock::next_vblank_seq`].
pub(super) fn flush_presentation_feedback(
    output: &Output,
    feedback: smithay::desktop::utils::OutputPresentationFeedback,
    time: std::time::Duration,
    seq: u64,
) {
    use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback;

    let mut feedback = feedback;
    let refresh = output_refresh_duration(output);
    feedback.presented::<_, smithay::utils::Monotonic>(
        time,
        smithay::wayland::presentation::Refresh::fixed(refresh),
        seq,
        wp_presentation_feedback::Kind::Vsync,
    );
}

/// What to do with one queued gamma ramp on this device's render pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GammaStep {
    /// Commit it now.
    Apply,
    /// Keep it queued: the output lives on another device, or its page
    /// flip is still in flight (a GAMMA_LUT write is a blocking atomic
    /// commit that would first wait for that flip). The vblank handler wakes
    /// the repaint source once the flip lands.
    Defer,
    /// Drop it: the CRTC has no GAMMA_LUT, or the output is gone.
    Drop,
}

fn gamma_step(
    on_this_device: bool,
    has_gamma_lut: bool,
    flip_in_flight: bool,
    output_alive: bool,
) -> GammaStep {
    if !on_this_device {
        if output_alive {
            GammaStep::Defer
        } else {
            GammaStep::Drop
        }
    } else if !has_gamma_lut {
        GammaStep::Drop
    } else if flip_in_flight {
        GammaStep::Defer
    } else {
        GammaStep::Apply
    }
}

/// Apply any gamma ramp updates queued by wlr_gamma_control clients
/// (and the built-in twilight scheduler). Multi-GPU care: this runs
/// once per `BackendData` (= once per DRM device), and
/// `state.output_pending.gamma` is shared across all devices. Draining
/// unconditionally would consume entries destined for the *next*
/// device's render pass and silently drop them — so we only consume
/// entries whose output we can see in this device's `outputs` map, and
/// re-park the rest for the next render call.
///
/// A ramp for an output whose page flip is still in flight is re-parked
/// too: the GAMMA_LUT write is a blocking atomic commit that would sit on
/// the main thread until that flip (and then its own) completes. The
/// VBlank handler wakes the repaint source when the flip lands, and the
/// repaint source calls this before it renders anything.
pub(super) fn apply_pending_gamma(
    outputs: &mut HashMap<crtc::Handle, OutputDevice>,
    drm: &DrmDevice,
    state: &mut MargoState,
) {
    if state.output_pending.gamma.is_empty() {
        return;
    }
    let pending = std::mem::take(&mut state.output_pending.gamma);
    let mut deferred: Vec<(smithay::output::Output, Option<Vec<u16>>)> = Vec::new();
    for (output, ramp) in pending {
        let od = outputs.values_mut().find(|od| od.output == output);
        let alive = state.monitors.iter().any(|m| m.output == output);
        let step = match &od {
            Some(od) => gamma_step(true, od.gamma.is_some(), od.flip_in_flight, alive),
            None => gamma_step(false, false, false, alive),
        };
        match (step, od) {
            (GammaStep::Apply, Some(od)) => {
                let Some(g) = od.gamma.as_mut() else {
                    continue;
                };
                match g.set_gamma(drm, ramp.as_deref()) {
                    Ok(true) => tracing::debug!(
                        "gamma applied output={} ramp={}",
                        od.output.name(),
                        if ramp.is_some() { "client" } else { "default" }
                    ),
                    Ok(false) => {
                        tracing::trace!("gamma unchanged output={}", od.output.name());
                    }
                    Err(e) => {
                        // Let twilight queue its ramp again on the next tick
                        // instead of treating it as applied.
                        state.twilight.applied.forget(&output);
                        warn!(
                            output = %od.output.name(),
                            error = ?e,
                            "gamma set failed"
                        );
                    }
                }
            }
            (GammaStep::Defer, _) => deferred.push((output, ramp)),
            (GammaStep::Drop, Some(od)) if od.gamma.is_none() => {
                tracing::debug!("gamma: skip {} (no GAMMA_LUT)", od.output.name());
            }
            _ => {}
        }
    }
    if !deferred.is_empty() {
        state.output_pending.gamma = deferred;
    }
}

/// Drain `state.output_pending.dpms` — real panel power off/on. `off` calls
/// smithay's `DrmCompositor::clear()` (which sets DPMS off + disables the
/// planes); `on` just drops the flag, and the normal render below re-renders
/// + `queue_frame`s the output, which re-enables it. Entries for outputs on
/// another device are re-parked, like the gamma drain.
fn apply_pending_dpms(outputs: &mut HashMap<crtc::Handle, OutputDevice>, state: &mut MargoState) {
    if state.output_pending.dpms.is_empty() {
        return;
    }
    let pending = std::mem::take(&mut state.output_pending.dpms);
    let mut deferred: Vec<(Output, bool)> = Vec::new();
    for (output, on) in pending {
        let Some(od) = outputs.values_mut().find(|od| od.output == output) else {
            deferred.push((output, on));
            continue;
        };
        if on {
            if od.dpms_off {
                od.dpms_off = false;
                // `clear()` left the surface with no pending damage, so force
                // the next `render_frame` to produce a full, non-empty frame
                // — otherwise it could report `is_empty` and never queue a
                // page-flip, leaving the panel dark.
                let _ = od.compositor.reset_state();
                tracing::info!(output = %od.output.name(), "dpms: on");
            }
        } else if !od.dpms_off {
            match od.compositor.clear() {
                Ok(()) => {
                    od.dpms_off = true;
                    // `clear()` drops the queued frame; no vblank follows.
                    od.flip_in_flight = false;
                    tracing::info!(output = %od.output.name(), "dpms: off (panel cleared)");
                }
                Err(e) => warn!(output = %od.output.name(), error = ?e, "dpms: clear failed"),
            }
        }
    }
    if !deferred.is_empty() {
        state.output_pending.dpms = deferred;
        state.wake_repaint_backend();
    }
}

pub(super) fn render_all_outputs(
    renderer: &mut GlesRenderer,
    outputs: &mut HashMap<crtc::Handle, OutputDevice>,
    drm: &DrmDevice,
    state: &mut MargoState,
    reason: &'static str,
) {
    apply_pending_gamma(outputs, drm, state);
    apply_pending_dpms(outputs, state);

    let per_output = state.per_output_frame_clock_enabled();
    let mut mirror = build_mirror_frame(renderer, outputs, state);
    for od in outputs.values_mut() {
        if state.output_render_retry_pending(&od.output) {
            continue;
        }
        let mirrored = take_mirror_frame(&mut mirror, &od.output);
        // A mirror target has no frame clock of its own (it is out of
        // the layout): it renders on the global gate.
        let per_output = per_output && mirrored.is_none();
        if per_output {
            state.begin_forced_render_per_output(&od.output);
        }
        render_output(renderer, od, state, reason, per_output, mirrored);
    }
    finish_repaint_batch(state);
}

/// The mirror frame, if `output` is the target it was built for.
fn take_mirror_frame(
    mirror: &mut Option<(Output, Vec<MargoRenderElement>)>,
    output: &Output,
) -> Option<Vec<MargoRenderElement>> {
    if mirror.as_ref().is_some_and(|(target, _)| target == output) {
        mirror.take().map(|(_, elements)| elements)
    } else {
        None
    }
}

/// Opt-in per-output frame-clock render. Renders ONLY the outputs in
/// `due` (computed by `MargoState::take_due_outputs`). Each due output
/// was already flipped in-flight on its `OutputClock`; this function
/// passes `per_output = true` so `render_output` uses the per-output
/// vblank gate instead of the global `pending_vblanks` counter. Gamma
/// ramps are applied the same way as the global path.
pub(super) fn render_due_outputs(
    renderer: &mut GlesRenderer,
    outputs: &mut HashMap<crtc::Handle, OutputDevice>,
    drm: &DrmDevice,
    state: &mut MargoState,
    due: &[Output],
    reason: &'static str,
) {
    apply_pending_gamma(outputs, drm, state);
    apply_pending_dpms(outputs, state);
    // A mirror target has no frame clock (it is out of the layout): it
    // follows its source, re-rendering whenever the source is due and the
    // target's previous flip has landed.
    let source_due = state
        .mirror
        .active()
        .is_some_and(|m| due.iter().any(|o| o.name() == m.source));
    let mut mirror = if source_due {
        build_mirror_frame(renderer, outputs, state)
    } else {
        None
    };
    for od in outputs.values_mut() {
        if due.contains(&od.output) {
            render_output(renderer, od, state, reason, true, None);
        } else if !od.flip_in_flight
            && let Some(elements) = take_mirror_frame(&mut mirror, &od.output)
        {
            render_output(renderer, od, state, reason, false, Some(elements));
        }
    }
    finish_repaint_batch(state);
}

/// Run the once-per-batch post-render refresh
/// ([`MargoState::finish_repaint_batch`]) and flush the events it queued.
fn finish_repaint_batch(state: &mut MargoState) {
    if state.finish_repaint_batch() {
        state.display_handle.flush_clients().ok();
    }
}

/// Render and queue one output's frame. `mirror` is the prebuilt frame
/// when this output is the active mirror target ([`build_mirror_frame`]):
/// it then shows that instead of its own (out-of-layout) scene.
fn render_output(
    renderer: &mut GlesRenderer,
    od: &mut OutputDevice,
    state: &mut MargoState,
    reason: &'static str,
    per_output: bool,
    mirror: Option<Vec<MargoRenderElement>>,
) {
    let _span = tracy_client::span!("render_output");
    // Delineate frames for Tracy's frame/FPS view — one mark per render
    // cycle. Compiled out without `profile-with-tracy`.
    #[cfg(feature = "profile-with-tracy")]
    tracy_client::frame_mark();

    // DPMS-off output: the panel is powered down via `compositor.clear()`.
    // Rendering + queueing a frame would re-enable it, so skip entirely
    // until a `output_pending.dpms (output, true)` clears the flag.
    if od.dpms_off {
        if per_output {
            state.clear_pending_vblank_per_output(&od.output);
        }
        return;
    }
    // Soft-disabled output: skip entirely.
    if mirror.is_none()
        && let Some(mon) = state.monitors.iter().find(|m| m.output == od.output)
    {
        if !mon.enabled {
            if per_output {
                state.clear_pending_vblank_per_output(&od.output);
            }
            return;
        }
    }
    // HDR Phase 2 scaffolding hook — eagerly compile shaders if
    // MARGO_COLOR_LINEAR=1 so a driver-rejection regression surfaces
    // at startup. Actual render path stays the existing 8-bit
    // composite until the upstream fp16-swapchain hook lands.
    if crate::render::linear_composite::is_linear_composite_enabled() {
        let _ = crate::render::linear_composite::encoder_shader(renderer);
        let _ = crate::render::linear_composite::decoder_shader(renderer);
    }
    take_pending_snapshots(renderer, od, state);
    take_pending_open_close_captures(renderer, od, state);
    super::take_mru_thumbnails(renderer, od, state);

    let is_mirror = mirror.is_some();
    let elements = match mirror {
        Some(elements) => elements,
        None => build_output_scene(renderer, od, state),
    };

    serve_screencopies(renderer, od, state, &elements);
    // A mirror's letterbox bars are black, like a projector's.
    let clear_color = if state.session_locked || is_mirror {
        [0.0, 0.0, 0.0, 1.0]
    } else {
        [0.1, 0.1, 0.1, 1.0]
    };
    // niri-style frame flags: primary + cursor plane scanout, but
    // NO overlay plane. Overlay planes look like a free perf win on
    // paper (an extra DRM plane to scan a layer-shell surface from
    // directly) but they're a well-documented source of "weird flicker
    // / performance issues on Intel hardware" — niri disables them
    // by default for the same reason (see `niri/src/backend/tty.rs`
    // lines 1898-1928 and the comment "Overlay planes are disabled
    // by default as they cause weird performance issues on my
    // system"). On Intel Arc MTL specifically, enabling overlay
    // plane assignment for the gtk4-layer-shell bar is what tipped
    // mshell from "mostly OK" into the visible per-keystroke
    // flicker the user kept reporting. The bar still gets fast
    // primary-plane direct scanout (because we tag it with
    // `Kind::ScanoutCandidate` in `push_layer_elements`), just
    // through the primary plane, not a flaky overlay plane.
    let frame_flags =
        FrameFlags::ALLOW_PRIMARY_PLANE_SCANOUT_ANY | FrameFlags::ALLOW_CURSOR_PLANE_SCANOUT;
    // Blur needs no special handling here: `BlurRenderElement` is a
    // smithay framebuffer effect, so the damage tracker itself redraws the
    // whole backdrop under a blur whenever anything behind it changed and
    // has the element re-capture it (see `render::blur`). Age-based damage
    // tracking and the swapchain stay intact while blur is on. (This used
    // to call `reset_buffers()` on every frame with a blur element, which
    // dropped and re-allocated every GBM buffer, DRM framebuffer and
    // EGLImage of the output once per frame.)
    let render_start = std::time::Instant::now();
    match od
        .compositor
        .render_frame(renderer, &elements, clear_color, frame_flags)
    {
        Ok(result) => {
            // Time-based `perf` metrics: fold this frame's render latency,
            // empty flag, composited damage area, and direct-scanout state
            // into the per-output rolling window. All reads on `result` here
            // are immutable and disjoint from the `state` borrow below.
            let render_us = render_start.elapsed().as_micros().min(u32::MAX as u128) as u32;
            let is_empty = result.is_empty;
            // `DamageSnapshot::add` push_fronts per commit, so the first
            // `raw()` group is this frame's damage. Rects may overlap —
            // the sum is an upper bound, fine for a health signal.
            let (damage_px, scanout) = match &result.primary_element {
                PrimaryPlaneElement::Swapchain(el) => {
                    let px: u64 = el
                        .damage
                        .raw()
                        .next()
                        .map(|rects| rects.map(|r| r.size.w as u64 * r.size.h as u64).sum())
                        .unwrap_or(0);
                    (px.min(u32::MAX as u64) as u32, false)
                }
                PrimaryPlaneElement::Element(_) => (0, true),
            };
            let damage_px = if is_empty { 0 } else { damage_px };
            od.render_count += 1;
            state.record_frame_sample(&od.output_name, is_empty, render_us, damage_px, scanout);
            if is_empty {
                od.render_failure_streak = 0;
                state.clear_output_render_retry(&od.output);
                od.empty_count += 1;
                tracing::trace!(
                    output = %od.output.name(),
                    reason = reason,
                    renders = od.render_count,
                    empties = od.empty_count,
                    elements = elements.len(),
                    "render empty",
                );
                // No DRM page-flip = no real VBlank coming back. But
                // mshell / any gtk4-layer-shell client still expects
                // `wl_surface.frame` callbacks to fire so its frame
                // clock can pace at refresh rate. Schedule an
                // estimated-vblank Timer at the next refresh interval
                // — when it fires, `on_estimated_vblank_timer` bumps
                // the per-output `frame_callback_sequence` and runs
                // `send_frame_callbacks`. This matches niri's
                // `queue_estimated_vblank_timer` (tty.rs:2933) and is
                // the piece margo was missing: without it,
                // gtk4-layer-shell clients either stall waiting for a
                // callback (visible as bar flicker on margo while
                // Hyprland and niri stay smooth) or burn CPU if we
                // send callbacks unthrottled on every empty render.
                let refresh_interval = od
                    .output
                    .current_mode()
                    .map(|m| {
                        // `refresh` is millihertz — convert to per-frame
                        // duration. Fall back to 60 Hz (16.67 ms) on the
                        // pathological 0-refresh case some virtual
                        // outputs report.
                        if m.refresh > 0 {
                            std::time::Duration::from_nanos(1_000_000_000_000u64 / m.refresh as u64)
                        } else {
                            std::time::Duration::from_micros(16_667)
                        }
                    })
                    .unwrap_or(std::time::Duration::from_micros(16_667));
                if per_output {
                    // Per-output path: no page-flip means no DRM vblank
                    // to clear this output's in-flight gate or re-arm
                    // its present timer — do both here, then send frame
                    // callbacks so its clients still pace at refresh.
                    state.note_empty_render_per_output(&od.output);
                } else {
                    state.queue_estimated_vblank_timer(&od.output, refresh_interval);
                }
                state.display_handle.flush_clients().ok();
                // Empty frames return before the common tail below. Publish
                // here as well; otherwise `mctl perf` appears frozen and then
                // reports a large fake burst the next time a non-empty frame
                // happens to copy the accumulated counters.
                publish_output_perf(od, state);
                return;
            }

            match od.compositor.queue_frame(()) {
                Ok(()) => {
                    od.render_failure_streak = 0;
                    od.flip_in_flight = true;
                    state.clear_output_render_retry(&od.output);
                    od.queued_count += 1;
                    // Global gate only — the per-output path tracks its
                    // own in-flight state on the `OutputClock`
                    // (`pending_vblank`), set when the output became due.
                    if !per_output {
                        state.note_frame_queued();
                    }
                    tracing::trace!(
                        output = %od.output.name(),
                        reason = reason,
                        queued = od.queued_count,
                        renders = od.render_count,
                        elements = elements.len(),
                        "queued frame",
                    );
                    // Update per-surface primary-scanout state BEFORE
                    // building presentation feedback or sending frame
                    // callbacks — both downstream paths read
                    // `surface_primary_scanout_output` which returns
                    // None until this update lands. Without this,
                    // `wp_presentation_feedback` is silently dropped
                    // and `send_frame_callbacks` can't filter occluded
                    // surfaces.
                    // A mirror target shows surfaces that live on its
                    // source: they keep the source as their primary
                    // scanout output (their frame callbacks and
                    // presentation feedback pace on it), so the mirrored
                    // frame stamps none of that.
                    if !is_mirror {
                        update_primary_scanout_outputs(state, &od.output, &result.states);
                        let feedback =
                            build_presentation_feedback(&od.output, state, &result.states);
                        od.pending_presentation.push(feedback);
                        update_client_scanout_flags(state, &od.output, &result.states);
                    }
                    // Bump sequence at queue_frame Ok per niri's
                    // `tty.rs:1975-1979` comment ("buffers were
                    // latched, new commits will wait for VBlank").
                    // But do NOT send_frame_callbacks here yet:
                    // sending them now lets the client commit its
                    // next buffer BEFORE our VBlank lands, and on a
                    // fast GPU (Intel Arc MTL) queue_frame Ok fires
                    // well before VBlank — mshell ends up pacing at
                    // 80-90 Hz against a 60 Hz monitor (verified in
                    // the journal: 12-14 ms gaps between consecutive
                    // mshell-frame commits = ~80 Hz). That over-
                    // commit races the page-flip and the bar pixels
                    // visibly judder. Defer the callbacks to
                    // `note_vblank` so clients can only pace at the
                    // actual refresh rate. mshell-on-niri stays
                    // smooth because niri's redraw scheduling is
                    // gated by `frame_clock.next_presentation_time()`
                    // even after queue_frame Ok — margo doesn't have
                    // a frame_clock yet, so we use the
                    // simpler "callbacks at VBlank, not at submit"
                    // gate instead.
                    // Keyed off the cached `od.output_name` via `get_mut` so
                    // the steady-state hot path allocates nothing; only the
                    // first frame after a hotplug inserts (cloning the name
                    // once). `insert(name, 1)` matches the old
                    // `entry().or_insert(0)` + `wrapping_add(1)`.
                    if let Some(seq) = state
                        .frame_callback_sequence
                        .get_mut(od.output_name.as_str())
                    {
                        *seq = seq.wrapping_add(1);
                    } else {
                        state
                            .frame_callback_sequence
                            .insert(od.output_name.clone(), 1);
                    }
                    state.post_repaint(&od.output, state.clock.now());
                    state.display_handle.flush_clients().ok();
                }
                Err(e) => {
                    od.queue_error_count += 1;
                    od.render_failure_streak = od.render_failure_streak.saturating_add(1);
                    if per_output {
                        // No flip was scheduled, so no vblank will clear
                        // this output's in-flight gate. Release it without an
                        // immediate timer; the bounded retry below owns the
                        // next wake-up.
                        state.clear_pending_vblank_per_output(&od.output);
                    }
                    state.defer_output_render_retry(&od.output, od.render_failure_streak);
                    if od.queue_error_count <= 10 || od.queue_error_count.is_multiple_of(300) {
                        warn!(
                            output = %od.output.name(),
                            reason = reason,
                            errors = od.queue_error_count,
                            elements = elements.len(),
                            error = ?e,
                            "queue_frame failed",
                        );
                    }
                }
            }
        }
        Err(e) => {
            // A failed render never presents, so — exactly like the queue-error
            // path above — this output's per-output in-flight gate would
            // otherwise never be cleared by a vblank, wedging the clock until
            // some unrelated damage happens to kick it. Release + re-arm so the
            // next tick retries (paced by the frame clock, so no busy spin), and
            // rate-limit the log so a persistent GPU failure can't flood it.
            od.render_error_count += 1;
            od.render_failure_streak = od.render_failure_streak.saturating_add(1);
            if per_output {
                state.clear_pending_vblank_per_output(&od.output);
            }
            state.defer_output_render_retry(&od.output, od.render_failure_streak);
            if od.render_error_count <= 10 || od.render_error_count.is_multiple_of(300) {
                error!(
                    output = %od.output.name(),
                    reason = reason,
                    errors = od.render_error_count,
                    elements = elements.len(),
                    error = ?e,
                    "render_frame failed",
                );
            }
        }
    }

    publish_output_perf(od, state);
}

/// Mirror backend-owned lifetime counters into `MargoState` for `mctl perf`.
/// Called from both the normal tail and the early empty-frame return.
fn publish_output_perf(od: &OutputDevice, state: &mut MargoState) {
    let perf = match state.perf_counters.get_mut(od.output_name.as_str()) {
        Some(perf) => perf,
        None => state
            .perf_counters
            .entry(od.output_name.clone())
            .or_default(),
    };
    perf.renders = od.render_count;
    perf.queued = od.queued_count;
    perf.empties = od.empty_count;
    perf.queue_errors = od.queue_error_count;
    perf.render_errors = od.render_error_count;
}

#[cfg(test)]
mod tests {
    use super::{GammaStep, gamma_step};

    #[test]
    fn a_ramp_for_an_idle_output_is_applied() {
        assert_eq!(gamma_step(true, true, false, true), GammaStep::Apply);
    }

    #[test]
    fn a_ramp_waits_while_the_output_has_a_flip_in_flight() {
        assert_eq!(gamma_step(true, true, true, true), GammaStep::Defer);
    }

    #[test]
    fn a_ramp_for_an_output_without_gamma_lut_is_dropped() {
        assert_eq!(gamma_step(true, false, false, true), GammaStep::Drop);
        assert_eq!(gamma_step(true, false, true, true), GammaStep::Drop);
    }

    #[test]
    fn a_ramp_for_another_device_waits_unless_the_output_is_gone() {
        assert_eq!(gamma_step(false, false, false, true), GammaStep::Defer);
        // Re-parking it forever would keep re-queueing a dead entry.
        assert_eq!(gamma_step(false, false, false, false), GammaStep::Drop);
    }
}
