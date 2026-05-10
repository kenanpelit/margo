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
        drm::{compositor::FrameFlags, DrmDevice},
        renderer::gles::GlesRenderer,
    },
    output::Output,
    reexports::drm::control::crtc,
    wayland::seat::WaylandFocus,
};
use tracing::{error, info, warn};

use super::{
    build_cursor_elements_for_output, build_render_elements,
    helpers::{monotonic_now, output_refresh_duration},
    serve_screencopies, take_pending_open_close_captures, take_pending_snapshots,
    MargoRenderElement, OutputDevice,
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
        surface_presentation_feedback_flags_from_states, surface_primary_scanout_output,
        OutputPresentationFeedback,
    };

    let mut feedback = OutputPresentationFeedback::new(output);

    // Toplevels.
    for window in state.space.elements() {
        if state.space.outputs_for_element(window).contains(output) {
            window.take_presentation_feedback(
                &mut feedback,
                surface_primary_scanout_output,
                |surface, _| {
                    surface_presentation_feedback_flags_from_states(surface, None, render_states)
                },
            );
        }
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

/// Signal `presented(now, refresh, seq, Vsync)` on a feedback builder
/// previously stashed on the OutputDevice. Called from the
/// `DrmEvent::VBlank` handler, so `now` reflects the actual page-flip
/// moment — not the submit time, which is the cheap approximation we
/// used to do.
///
/// `seq` is the per-output monotonic VBlank counter from
/// `OutputDevice::vblank_seq`. Smithay 0.7's `DrmEvent::VBlank(crtc)`
/// doesn't surface the kernel's `drm_event_vblank.sequence`, but the
/// `wp_presentation` protocol contract is "implementation-defined
/// monotonic counter" — a per-output increment satisfies that and
/// is observably equivalent for frame-pacing-sensitive consumers
/// (mpv `--vo=gpu-next`, kitty render loop).
pub(super) fn flush_presentation_feedback(
    output: &Output,
    feedback: smithay::desktop::utils::OutputPresentationFeedback,
    seq: u64,
) {
    use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback;

    let mut feedback = feedback;
    let now = monotonic_now();
    let refresh = output_refresh_duration(output);
    feedback.presented::<_, smithay::utils::Monotonic>(
        now,
        smithay::wayland::presentation::Refresh::fixed(refresh),
        seq,
        wp_presentation_feedback::Kind::Vsync,
    );
}

pub(super) fn render_all_outputs(
    renderer: &mut GlesRenderer,
    outputs: &mut HashMap<crtc::Handle, OutputDevice>,
    drm: &DrmDevice,
    state: &mut MargoState,
    reason: &'static str,
) {
    // Apply any gamma ramp updates queued by wlr_gamma_control clients.
    if !state.pending_gamma.is_empty() {
        let pending = std::mem::take(&mut state.pending_gamma);
        for (output, ramp) in pending {
            let target = outputs.values_mut().find(|od| od.output == output);
            let Some(od) = target else { continue };
            let Some(g) = od.gamma.as_mut() else {
                tracing::debug!("gamma: skip {} (no GAMMA_LUT)", od.output.name());
                continue;
            };
            match g.set_gamma(drm, ramp.as_deref()) {
                Ok(()) => tracing::debug!(
                    "gamma applied output={} ramp={}",
                    od.output.name(),
                    if ramp.is_some() { "client" } else { "default" }
                ),
                Err(e) => warn!(
                    output = %od.output.name(),
                    error = ?e,
                    "gamma set failed"
                ),
            }
        }
    }

    for od in outputs.values_mut() {
        render_output(renderer, od, state, reason);
    }
}

fn render_output(
    renderer: &mut GlesRenderer,
    od: &mut OutputDevice,
    state: &mut MargoState,
    reason: &'static str,
) {
    let _span = tracy_client::span!("render_output");

    // Soft-disabled output: skip entirely.
    if let Some(mon) = state.monitors.iter().find(|m| m.output == od.output) {
        if !mon.enabled {
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

    let mut elements = build_render_elements(renderer, od, state);
    // W2.1 region selector overlay — when active, layer:
    //   [cursor (top), outline edges, dim, live scene]
    if state.region_selector.is_some() {
        let (mon_origin, mon_size) = state
            .monitors
            .iter()
            .find(|m| m.output == od.output)
            .map(|m| (
                (m.monitor_area.x, m.monitor_area.y),
                (m.monitor_area.width, m.monitor_area.height),
            ))
            .unwrap_or(((0, 0), (1920, 1080)));
        let scale = od.output.current_scale().fractional_scale();
        let (cursor_elements, _cursor_loc) =
            build_cursor_elements_for_output(renderer, od, state);
        let cursor_count = cursor_elements.len();

        if let Some(sel) = state.region_selector.as_mut() {
            let overlay = sel.render_elements(mon_origin, mon_size, scale);
            let mut head: Vec<MargoRenderElement> = Vec::new();
            for c in cursor_elements.into_iter().take(cursor_count) {
                head.push(c);
            }
            let scene_tail: Vec<MargoRenderElement> =
                elements.drain(cursor_count..).collect();
            for o in overlay {
                head.push(MargoRenderElement::Solid(o));
            }
            for s in scene_tail {
                head.push(s);
            }
            elements = head;
        }
    }
    serve_screencopies(renderer, od, state, &elements);
    let clear_color = if state.session_locked {
        [0.0, 0.0, 0.0, 1.0]
    } else {
        [0.1, 0.1, 0.1, 1.0]
    };
    match od
        .compositor
        .render_frame(renderer, &elements, clear_color, FrameFlags::DEFAULT)
    {
        Ok(result) => {
            od.render_count += 1;
            if result.is_empty {
                od.empty_count += 1;
                if od.empty_count <= 5 || od.empty_count.is_multiple_of(120) {
                    info!(
                        output = %od.output.name(),
                        reason = reason,
                        renders = od.render_count,
                        elements = elements.len(),
                        "render empty",
                    );
                }
                return;
            }

            match od.compositor.queue_frame(()) {
                Ok(()) => {
                    od.queued_count += 1;
                    state.note_frame_queued();
                    if od.queued_count <= 10 || od.queued_count.is_multiple_of(300) {
                        info!(
                            output = %od.output.name(),
                            reason = reason,
                            queued = od.queued_count,
                            renders = od.render_count,
                            elements = elements.len(),
                            "queued frame",
                        );
                    }
                    let feedback = build_presentation_feedback(&od.output, state, &result.states);
                    od.pending_presentation.push(feedback);
                    update_client_scanout_flags(state, &od.output, &result.states);
                    state.post_repaint(&od.output, state.clock.now());
                    state.display_handle.flush_clients().ok();
                }
                Err(e) => {
                    od.queue_error_count += 1;
                    state.request_repaint();
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
        Err(e) => error!(
            output = %od.output.name(),
            reason = reason,
            elements = elements.len(),
            error = ?e,
            "render_frame failed",
        ),
    }
}
