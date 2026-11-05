//! The on-demand redraw scheduler pass and the blur-pool cleanup wake.

use std::{cell::RefCell, rc::Rc};

use tracing::warn;

#[cfg(feature = "xdp-gnome-screencast")]
use super::drain_active_cast_frames;
use super::{
    BackendData, apply_pending_gamma, apply_pending_mode_changes, drain_image_copy_frames,
    render_all_outputs, render_due_outputs, session,
};
use crate::state::MargoState;

/// One pass of the on-demand redraw scheduler: backend queues (mode changes,
/// gamma, DPMS), the due outputs (or every output on the global clock), then
/// image-copy and PipeWire captures. Run by the repaint-ping source and,
/// through [`MargoState::set_repaint_hook`], by a firing present timer.
pub(super) fn repaint_tick(backend_data: &Rc<RefCell<BackendData>>, state: &mut MargoState) {
    // Session paused (VT switch): the KMS work below waits for the resume,
    // which re-queues gamma, wakes DPMS and pings this source again. The
    // repaint request stays set until then. Offscreen captures still run.
    let kms_usable = session::kms_usable(&backend_data.borrow().drm);
    // Apply any wlr_output_management mode changes
    // before rendering. The handler that accepted them
    // ran on MargoState (no backend handle), so it
    // queued requests onto state.output_pending.mode_changes;
    // we drain here where DrmCompositor::use_mode is
    // reachable. A successful apply re-arranges and
    // re-pings repaint internally.
    if kms_usable && !state.output_pending.mode_changes.is_empty() {
        let mut bd = backend_data.borrow_mut();
        apply_pending_mode_changes(&mut bd, state);
    }
    // Gamma ramps (twilight, wlr-gamma-control) are CRTC
    // properties, not frames: their producers only wake this
    // source, so apply them whether or not anything renders.
    if kms_usable && !state.output_pending.gamma.is_empty() {
        let mut bd = backend_data.borrow_mut();
        let BackendData { outputs, drm, .. } = &mut *bd;
        apply_pending_gamma(outputs, drm, state);
    }
    // DPMS changes MUST go through the all-outputs path: a
    // DPMS-off output stops producing vblanks, so its per-output
    // clock stalls and it never becomes "due" — meaning a wake
    // queued in `output_pending.dpms` would never drain on the per-output
    // path and the panel would stay dark. Forcing the global path
    // whenever `output_pending.dpms` is non-empty guarantees the queue is
    // serviced (and is the recovery path the VT-switch relies on).
    let force_all_for_dpms = !state.output_pending.dpms.is_empty();
    if !kms_usable {
        // Leave the repaint request and the due clocks for the resume.
    } else if state.per_output_frame_clock_enabled() && !force_all_for_dpms {
        // Opt-in per-output path: render ONLY the outputs
        // whose present timer has come due (dirty + refresh
        // interval elapsed + not awaiting a vblank). Each
        // due output's clock is flipped in-flight by
        // `take_due_outputs`; its vblank re-arms the next
        // tick. The global dirty flag is still drained so it
        // doesn't leak into the global path on a later
        // config reload that turns the flag back off.
        state.take_repaint_request();
        let due = state.take_due_outputs();
        let mut bd = backend_data.borrow_mut();
        // A mirror target that owes a frame renders even when no output
        // with a clock is due (its own vblank woke us).
        if !due.is_empty() || bd.mirror_frame_pending {
            let BackendData {
                renderer,
                outputs,
                drm,
                mirror_frame_pending,
                ..
            } = &mut *bd;
            render_due_outputs(
                renderer,
                outputs,
                drm,
                state,
                &due,
                mirror_frame_pending,
                "repaint-per-output",
            );
        }
    } else if state.take_repaint_request() || force_all_for_dpms {
        let mut bd = backend_data.borrow_mut();
        let BackendData {
            renderer,
            outputs,
            drm,
            mirror_frame_pending,
            ..
        } = &mut *bd;
        render_all_outputs(
            renderer,
            outputs,
            drm,
            state,
            mirror_frame_pending,
            "repaint",
        );
    }
    // ext-image-copy-capture: drain pending frames
    // queued by `ImageCopyCaptureHandler::frame()` and
    // render each into its client buffer. Done after
    // the live render so the renderer is warm + the
    // scene state is the same one the user just saw.
    if !state.pending_image_copy_frames.is_empty() {
        let mut bd = backend_data.borrow_mut();
        let BackendData {
            renderer, outputs, ..
        } = &mut *bd;
        drain_image_copy_frames(renderer, outputs, state);
    }
    // PipeWire screencast: render every active cast on
    // every repaint. niri's output render path tags
    // casts with `check_time_and_schedule` for inter-
    // frame pacing; until that lands here, we lean on
    // PipeWire's own `dequeue_available_buffer` to drop
    // frames when the consumer hasn't returned a buffer
    // yet — buffer-bounded backpressure.
    #[cfg(feature = "xdp-gnome-screencast")]
    {
        let has_casts = state
            .screencasting
            .as_ref()
            .is_some_and(|s| !s.casts.is_empty());
        if has_casts {
            let mut bd = backend_data.borrow_mut();
            let BackendData {
                renderer, outputs, ..
            } = &mut *bd;
            drain_active_cast_frames(renderer, outputs, state);
        }
    }
    arm_blur_cleanup(backend_data, state);
}

/// A one-shot cleanup wake while blur scratch textures exist. It does
/// no rendering and never touches KMS; a static desktop still releases the
/// unused pool after the last blurred surface disappears.
fn arm_blur_cleanup(backend_data: &Rc<RefCell<BackendData>>, state: &mut MargoState) {
    let Some(delay) = crate::render::blur::next_cleanup_delay() else {
        return;
    };
    let backend = backend_data.clone();
    let timer = smithay::reexports::calloop::timer::Timer::from_duration(delay);
    if let Err(e) = state.loop_handle.insert_source(timer, move |_, _, state| {
        crate::render::blur::cleanup_timer_finished();
        crate::render::blur::release_idle_pool(&mut backend.borrow_mut().renderer);
        // Blur may have run since this timer was armed. Keep exactly one
        // timer for its updated deadline; otherwise the pool is now empty.
        arm_blur_cleanup(&backend, state);
        smithay::reexports::calloop::timer::TimeoutAction::Drop
    }) {
        crate::render::blur::cleanup_timer_finished();
        warn!("blur cleanup timer: {e}");
    }
}
