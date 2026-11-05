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
use std::rc::Rc;

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
    mirror_element::SharedScene,
    render_elements::{build_mirror_frame, build_output_scene, mirror_source_of},
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

/// Minimum spacing between two GAMMA_LUT commits on one output. Atomic
/// updates no longer block the event loop, but can still contend with the
/// next page flip. Twilight transitions (4 steps/s), a `mctl twilight test`
/// sweep (20/s) and fading wlr-gamma-control clients are held to two colour
/// steps a second; the newest ramp always lands.
pub(super) const GAMMA_MIN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

/// What to do with one queued gamma ramp on this device's render pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GammaStep {
    /// Commit it now.
    Apply,
    /// Keep it queued: the output lives on another device, or its page
    /// flip is still in flight (an atomic LUT update would race that
    /// flip). The vblank handler wakes the repaint source once it lands.
    Defer,
    /// Keep it queued until [`GAMMA_MIN_INTERVAL`] has passed since this
    /// output's last commit; a timer wakes the repaint source then.
    Wait(std::time::Duration),
    /// Drop it: the CRTC has no GAMMA_LUT, or the output is gone.
    Drop,
}

fn gamma_step(
    on_this_device: bool,
    has_gamma_lut: bool,
    flip_in_flight: bool,
    output_alive: bool,
    since_last_commit: Option<std::time::Duration>,
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
    } else if let Some(since) = since_last_commit
        && since < GAMMA_MIN_INTERVAL
    {
        GammaStep::Wait(GAMMA_MIN_INTERVAL - since)
    } else {
        GammaStep::Apply
    }
}

/// The queued ramps with only the newest one per output kept (in the order
/// of those newest entries): an older ramp for the same output would only
/// be overwritten, at the price of one more atomic commit.
fn latest_ramp_per_output<K: PartialEq, R>(pending: Vec<(K, R)>) -> Vec<(K, R)> {
    let mut latest: Vec<(K, R)> = Vec::with_capacity(pending.len());
    for (key, ramp) in pending {
        latest.retain(|(k, _)| *k != key);
        latest.push((key, ramp));
    }
    latest
}

/// Whether a gamma commit raced another atomic update. Keep the ramp
/// queued and retry after the rate-limit interval (no busy loop, and an
/// external wlr-gamma-control client's one-shot update is never lost).
fn gamma_retryable(error: &anyhow::Error) -> bool {
    error.downcast_ref::<std::io::Error>().is_some_and(|e| {
        e.raw_os_error() == Some(16) // Linux EBUSY
            || e.kind() == std::io::ErrorKind::WouldBlock
    })
}

/// Every rate-limited output whose deadline we marked as armed must have
/// a wake-up at that deadline, not just the earliest one. Equal deadlines
/// can share one timer because it drains the shared queue.
fn gamma_wake_delays(mut delays: Vec<std::time::Duration>) -> Vec<std::time::Duration> {
    delays.sort_unstable();
    delays.dedup();
    delays
}

/// Whether a wake-up timer must be armed for a rate-limited ramp: none is
/// pending (`wake_at` unset, or its deadline already passed).
fn gamma_wake_needed(wake_at: Option<std::time::Instant>, now: std::time::Instant) -> bool {
    wake_at.is_none_or(|t| t <= now)
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
/// too, so its NONBLOCK atomic LUT update does not race the page flip.
/// The VBlank handler wakes the repaint source when the flip lands.
/// A following queue_frame racing the LUT update uses the normal bounded
/// render-retry path; a busy LUT update itself is re-parked as well.
pub(super) fn apply_pending_gamma(
    outputs: &mut HashMap<crtc::Handle, OutputDevice>,
    drm: &DrmDevice,
    state: &mut MargoState,
) {
    if state.output_pending.gamma.is_empty() {
        return;
    }
    let pending = latest_ramp_per_output(std::mem::take(&mut state.output_pending.gamma));
    let now = std::time::Instant::now();
    let mut deferred: Vec<(smithay::output::Output, Option<Vec<u16>>)> = Vec::new();
    let mut wake_delays = Vec::new();
    for (output, ramp) in pending {
        let od = outputs.values_mut().find(|od| od.output == output);
        let alive = state.monitors.iter().any(|m| m.output == output);
        let step = match &od {
            Some(od) => gamma_step(
                true,
                od.gamma.is_some(),
                od.flip_in_flight,
                alive,
                od.gamma
                    .as_ref()
                    .and_then(|g| g.last_commit)
                    .map(|t| now.saturating_duration_since(t)),
            ),
            None => gamma_step(false, false, false, alive, None),
        };
        match (step, od) {
            (GammaStep::Apply, Some(od)) => {
                let Some(g) = od.gamma.as_mut() else {
                    continue;
                };
                g.wake_at = None;
                match g.set_gamma(drm, ramp.as_deref()) {
                    Ok(true) => tracing::debug!(
                        "gamma applied output={} ramp={}",
                        od.output.name(),
                        if ramp.is_some() { "client" } else { "default" }
                    ),
                    Ok(false) => {
                        tracing::trace!("gamma unchanged output={}", od.output.name());
                    }
                    Err(e) if gamma_retryable(&e) => {
                        g.wake_at = Some(now + GAMMA_MIN_INTERVAL);
                        wake_delays.push(GAMMA_MIN_INTERVAL);
                        deferred.push((output, ramp));
                        tracing::debug!(output = %od.output_name, "gamma busy: retry queued");
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
            (GammaStep::Wait(left), Some(od)) => {
                if let Some(g) = od.gamma.as_mut()
                    && gamma_wake_needed(g.wake_at, now)
                {
                    g.wake_at = Some(now + left);
                    wake_delays.push(left);
                }
                tracing::trace!(output = %od.output_name, ?left, "gamma rate-limited");
                deferred.push((output, ramp));
            }
            (GammaStep::Defer, _) => deferred.push((output, ramp)),
            (GammaStep::Drop, Some(od)) if od.gamma.is_none() => {
                tracing::debug!("gamma: skip {} (no GAMMA_LUT)", od.output.name());
            }
            _ => {}
        }
    }
    if !deferred.is_empty() {
        // Ramps queued meanwhile go after these (newer wins on coalescing).
        deferred.append(&mut state.output_pending.gamma);
        state.output_pending.gamma = deferred;
    }
    for delay in gamma_wake_delays(wake_delays) {
        let timer = smithay::reexports::calloop::timer::Timer::from_duration(delay);
        if let Err(e) = state.loop_handle.insert_source(timer, |_, _, state| {
            state.wake_repaint_backend();
            smithay::reexports::calloop::timer::TimeoutAction::Drop
        }) {
            warn!("gamma wake timer: {e}");
        }
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
    mirror_pending: &mut bool,
    reason: &'static str,
) {
    apply_pending_gamma(outputs, drm, state);
    apply_pending_dpms(outputs, state);
    // Every output renders here, the mirror target included.
    *mirror_pending = false;

    let per_output = state.per_output_frame_clock_enabled();
    // The mirror target renders last, from the scene its source rendered
    // from in this loop (built once per batch).
    let mut source_scene = None;
    for od in outputs.values_mut() {
        if state.output_render_retry_pending(&od.output) || state.mirror.is_target(&od.output_name)
        {
            continue;
        }
        if per_output {
            state.begin_forced_render_per_output(&od.output);
        }
        let scene = render_output(renderer, od, state, reason, per_output, None);
        if state.mirror.is_source(&od.output_name) {
            source_scene = scene;
        }
    }
    // A mirror target has no frame clock of its own (it is out of the
    // layout): it renders on the global gate.
    render_mirror_target(renderer, outputs, state, reason, source_scene);
    finish_repaint_batch(state);
}

/// Render the active mirror's target from `source_scene` (the scene the
/// source rendered from in this batch, if it did). Skipped while the
/// target waits out a render-retry backoff, before anything is built.
fn render_mirror_target(
    renderer: &mut GlesRenderer,
    outputs: &mut HashMap<crtc::Handle, OutputDevice>,
    state: &mut MargoState,
    reason: &'static str,
    source_scene: Option<SharedScene>,
) {
    let retrying = outputs
        .values()
        .find(|od| state.mirror.is_target(&od.output_name))
        .is_some_and(|od| state.output_render_retry_pending(&od.output));
    if retrying {
        return;
    }
    let Some((target, elements)) = build_mirror_frame(renderer, outputs, state, source_scene)
    else {
        return;
    };
    // Captures of the target record the source's capture scene (the
    // target's own is empty: it is out of the layout).
    if let Some(od) = outputs.values().find(|od| od.output == target) {
        let source = mirror_source_of(outputs, od, state);
        serve_screencopies(renderer, od, source, state, &elements);
    }
    if let Some(od) = outputs.values_mut().find(|od| od.output == target) {
        render_output(renderer, od, state, reason, false, Some(elements));
    }
}

/// What the per-output-clock path does with the active mirror's target in
/// one repaint batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MirrorStep {
    /// Nothing new to show.
    Idle,
    /// Build the mirror frame and render the target now.
    Render,
    /// The target's previous flip is still in flight: remember that it owes
    /// a frame and draw it when that flip lands.
    Defer,
}

/// `source_due`: the source renders in this batch. `pending`: an earlier
/// batch deferred the target. `target_in_flight`: the target's last flip
/// hasn't landed yet.
fn mirror_target_step(source_due: bool, pending: bool, target_in_flight: bool) -> MirrorStep {
    if !source_due && !pending {
        MirrorStep::Idle
    } else if target_in_flight {
        MirrorStep::Defer
    } else {
        MirrorStep::Render
    }
}

/// Opt-in per-output frame-clock render. Renders ONLY the outputs in
/// `due` (computed by `MargoState::take_due_outputs`). Each due output
/// was already flipped in-flight on its `OutputClock`; this function
/// passes `per_output = true` so `render_output` uses the per-output
/// vblank gate instead of the global `pending_vblanks` counter. Gamma
/// ramps are applied the same way as the global path.
///
/// A mirror target has no frame clock (it is out of the layout): it
/// follows its source, rendering in every batch where the source is due.
/// When its previous flip is still in flight then, `mirror_pending` records
/// that it owes a frame; the target's vblank wakes a batch (possibly with
/// nothing due) that draws it from a fresh mirror frame.
pub(super) fn render_due_outputs(
    renderer: &mut GlesRenderer,
    outputs: &mut HashMap<crtc::Handle, OutputDevice>,
    drm: &DrmDevice,
    state: &mut MargoState,
    due: &[Output],
    mirror_pending: &mut bool,
    reason: &'static str,
) {
    apply_pending_gamma(outputs, drm, state);
    apply_pending_dpms(outputs, state);
    let step = match state.mirror.active() {
        Some(m) => {
            let source_due = due.iter().any(|o| o.name() == m.source);
            let target_in_flight = outputs
                .values()
                .any(|od| od.output_name == m.target && od.flip_in_flight);
            mirror_target_step(source_due, *mirror_pending, target_in_flight)
        }
        None => MirrorStep::Idle,
    };
    // `Idle` means nothing was owed either, so this also drops a stale
    // flag once mirroring stops.
    match step {
        MirrorStep::Defer if !*mirror_pending => {
            tracing::debug!(target: "margo::mirror", "target flip in flight: frame deferred");
        }
        MirrorStep::Render if *mirror_pending => {
            tracing::debug!(target: "margo::mirror", "drawing the deferred mirror frame");
        }
        _ => {}
    }
    *mirror_pending = step == MirrorStep::Defer;
    let mut source_scene = None;
    for od in outputs.values_mut() {
        if due.contains(&od.output) {
            let scene = render_output(renderer, od, state, reason, true, None);
            if step == MirrorStep::Render && state.mirror.is_source(&od.output_name) {
                source_scene = scene;
            }
        }
    }
    // Built only when the target can take it (a frame built for an
    // in-flight target would be thrown away), from the source's scene
    // when the source rendered above.
    if step == MirrorStep::Render {
        render_mirror_target(renderer, outputs, state, reason, source_scene);
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
/// it then shows that instead of its own (out-of-layout) scene. Returns the
/// scene it rendered from, so a mirror target can show the same one
/// without building it again; `None` when the output was skipped.
fn render_output(
    renderer: &mut GlesRenderer,
    od: &mut OutputDevice,
    state: &mut MargoState,
    reason: &'static str,
    per_output: bool,
    mirror: Option<Vec<MargoRenderElement>>,
) -> Option<SharedScene> {
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
        return None;
    }
    // Soft-disabled output: skip entirely.
    if mirror.is_none()
        && let Some(mon) = state.monitors.iter().find(|m| m.output == od.output)
    {
        if !mon.enabled {
            if per_output {
                state.clear_pending_vblank_per_output(&od.output);
            }
            return None;
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
    // Give the blur scratch textures back once blur has gone unused for
    // a while (no-op otherwise; outside any frame, so GL is free to use).
    crate::render::blur::release_idle_pool(renderer);
    take_pending_snapshots(renderer, od, state);
    take_pending_open_close_captures(renderer, od, state);
    super::take_mru_thumbnails(renderer, od, state);

    let is_mirror = mirror.is_some();
    let elements = match mirror {
        Some(elements) => elements,
        None => build_output_scene(renderer, od, state),
    };
    present_output(
        renderer, od, state, reason, per_output, &elements, is_mirror,
    );
    // Only the mirror source shares its scene. Ordinary outputs need no
    // Rc allocation at all (the vec is consumed when this call returns).
    state
        .mirror
        .is_source(&od.output_name)
        .then(|| Rc::new(elements))
}

/// The rest of [`render_output`] once the scene is built: serve
/// screencopies, render, queue the flip and do the bookkeeping.
fn present_output(
    renderer: &mut GlesRenderer,
    od: &mut OutputDevice,
    state: &mut MargoState,
    reason: &'static str,
    per_output: bool,
    elements: &[MargoRenderElement],
    is_mirror: bool,
) {
    // A mirror target's screencopies were served by `render_mirror_target`,
    // which can reach the source's device.
    if !is_mirror {
        serve_screencopies(renderer, od, None, state, elements);
    }
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
        .render_frame(renderer, elements, clear_color, frame_flags)
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
    use super::{
        GAMMA_MIN_INTERVAL, GammaStep, MirrorStep, gamma_step, gamma_wake_needed,
        latest_ramp_per_output, mirror_target_step,
    };
    use std::time::{Duration, Instant};

    /// A 60 Hz source mirrored to a 50 Hz projector (20 ms flips): the
    /// source renders at t = 0 and again at t = 5 ms (the last frame of a
    /// slide transition), while the target's first flip is still in flight.
    /// When that flip lands the target must show the 5 ms frame, not keep
    /// the 0 ms one until the source changes again.
    #[test]
    fn a_mirror_frame_skipped_mid_flip_is_drawn_when_the_flip_lands() {
        #[derive(Clone, Copy)]
        enum Ev {
            SourceRenders(u32),
            TargetVblank,
        }
        let timeline = [
            (0, Ev::SourceRenders(0)),
            (5, Ev::SourceRenders(1)),
            (20, Ev::TargetVblank),
            (40, Ev::TargetVblank),
        ];
        let mut source_version = 0;
        let mut pending = false;
        let mut in_flight = false;
        let mut shown = None;
        let batch = |source_due: bool,
                     source_version: u32,
                     pending: &mut bool,
                     in_flight: &mut bool,
                     shown: &mut Option<u32>| {
            match mirror_target_step(source_due, *pending, *in_flight) {
                MirrorStep::Render => {
                    *shown = Some(source_version);
                    *in_flight = true;
                    *pending = false;
                }
                MirrorStep::Defer => *pending = true,
                MirrorStep::Idle => {}
            }
        };
        for (_t, ev) in timeline {
            match ev {
                Ev::SourceRenders(v) => {
                    source_version = v;
                    batch(
                        true,
                        source_version,
                        &mut pending,
                        &mut in_flight,
                        &mut shown,
                    );
                }
                Ev::TargetVblank => {
                    in_flight = false;
                    // The vblank handler wakes a batch only when the target
                    // owes a frame; the source isn't due in it.
                    if pending {
                        batch(
                            false,
                            source_version,
                            &mut pending,
                            &mut in_flight,
                            &mut shown,
                        );
                    }
                }
            }
        }
        assert_eq!(shown, Some(1));
        assert!(!pending);
    }

    #[test]
    fn mirror_target_steps() {
        // Source due, target free: render.
        assert_eq!(mirror_target_step(true, false, false), MirrorStep::Render);
        // Target busy: never build a frame that would be thrown away.
        assert_eq!(mirror_target_step(true, false, true), MirrorStep::Defer);
        assert_eq!(mirror_target_step(false, true, true), MirrorStep::Defer);
        // Owed frame, target free (its flip just landed): render alone.
        assert_eq!(mirror_target_step(false, true, false), MirrorStep::Render);
        // Nothing new.
        assert_eq!(mirror_target_step(false, false, false), MirrorStep::Idle);
        assert_eq!(mirror_target_step(false, false, true), MirrorStep::Idle);
    }

    #[test]
    fn a_ramp_for_an_idle_output_is_applied() {
        assert_eq!(gamma_step(true, true, false, true, None), GammaStep::Apply);
        assert_eq!(
            gamma_step(true, true, false, true, Some(GAMMA_MIN_INTERVAL)),
            GammaStep::Apply
        );
    }

    #[test]
    fn a_ramp_waits_while_the_output_has_a_flip_in_flight() {
        assert_eq!(gamma_step(true, true, true, true, None), GammaStep::Defer);
    }

    #[test]
    fn a_ramp_for_an_output_without_gamma_lut_is_dropped() {
        assert_eq!(gamma_step(true, false, false, true, None), GammaStep::Drop);
        assert_eq!(gamma_step(true, false, true, true, None), GammaStep::Drop);
    }

    #[test]
    fn a_ramp_for_another_device_waits_unless_the_output_is_gone() {
        assert_eq!(
            gamma_step(false, false, false, true, None),
            GammaStep::Defer
        );
        // Re-parking it forever would keep re-queueing a dead entry.
        assert_eq!(
            gamma_step(false, false, false, false, None),
            GammaStep::Drop
        );
    }

    #[test]
    fn ramps_are_held_to_two_commits_a_second() {
        let ms = Duration::from_millis;
        // 50 ms after the last commit (a `mctl twilight test` sweep step):
        // wait out the rest of the interval.
        assert_eq!(
            gamma_step(true, true, false, true, Some(ms(50))),
            GammaStep::Wait(GAMMA_MIN_INTERVAL - ms(50))
        );
        // A flip in flight still defers to the vblank first.
        assert_eq!(
            gamma_step(true, true, true, true, Some(ms(50))),
            GammaStep::Defer
        );
        // Twilight's 250 ms transition steps: every other one is folded.
        assert_eq!(
            gamma_step(true, true, false, true, Some(ms(250))),
            GammaStep::Wait(ms(250))
        );
    }

    #[test]
    fn only_the_newest_ramp_per_output_is_committed() {
        let pending = vec![
            ("eDP-1", 1),
            ("DP-3", 10),
            ("eDP-1", 2),
            ("eDP-1", 3),
            ("DP-3", 11),
        ];
        assert_eq!(
            latest_ramp_per_output(pending),
            vec![("eDP-1", 3), ("DP-3", 11)]
        );
        assert!(latest_ramp_per_output(Vec::<(&str, u8)>::new()).is_empty());
    }

    #[test]
    fn a_busy_gamma_commit_is_retried_but_invalid_updates_are_not() {
        let busy = anyhow::Error::new(std::io::Error::from_raw_os_error(16))
            .context("set GAMMA_LUT property");
        assert!(super::gamma_retryable(&busy));
        let invalid = anyhow::Error::new(std::io::Error::from_raw_os_error(22));
        assert!(!super::gamma_retryable(&invalid));
    }

    #[test]
    fn outputs_with_different_gamma_deadlines_each_get_a_wake() {
        let ms = Duration::from_millis;
        assert_eq!(
            super::gamma_wake_delays(vec![ms(200), ms(50), ms(200)]),
            vec![ms(50), ms(200)],
        );
        assert!(super::gamma_wake_delays(vec![]).is_empty());
    }

    #[test]
    fn one_wake_timer_per_wait() {
        let now = Instant::now();
        assert!(gamma_wake_needed(None, now));
        assert!(!gamma_wake_needed(
            Some(now + Duration::from_millis(100)),
            now
        ));
        // The armed deadline passed (its timer fired): arm again.
        assert!(gamma_wake_needed(Some(now), now));
    }
}
