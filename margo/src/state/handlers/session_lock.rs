//! `ext-session-lock-v1` handler — locking + unlocking + per-output
//! lock surfaces.
//!
//! The protocol requires the compositor to send a configure with a
//! non-zero size before the client attaches a buffer; without it the
//! lock surface stays unmapped and the screen renders solid black
//! ("alt+l → black screen" symptom).

use smithay::{
    delegate_session_lock,
    output::Output,
    reexports::wayland_server::protocol::wl_output::WlOutput,
    utils::Size,
    wayland::session_lock::{LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker},
};

use crate::state::MargoState;

impl SessionLockHandler for MargoState {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.session_lock_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        tracing::info!(
            "session_lock: lock() called (was locked={}, lock_surfaces={})",
            self.session_locked,
            self.lock_surfaces.len()
        );
        confirmation.lock();
        self.session_locked = true;
        self.arrange_all();
    }

    fn unlock(&mut self) {
        tracing::info!("session_lock: unlock() called");
        self.session_locked = false;
        self.lock_surfaces.clear();
        self.arrange_all();
        // After unlock, push focus back to a real window — by default
        // current_focus is still pointing at the (now-dead) lock surface
        // and the user has to nudge the mouse before any keys reach the
        // toplevel underneath.
        self.refresh_keyboard_focus();
    }

    fn new_surface(&mut self, surface: LockSurface, output: WlOutput) {
        let Some(output) = Output::from_resource(&output) else {
            tracing::warn!("session_lock: new_surface for unknown output");
            return;
        };

        // CRITICAL: ext-session-lock-v1 requires the compositor to send a
        // configure WITH a non-zero size before the client will attach a
        // buffer. Without this, the lock surface stays unmapped and we
        // render solid black with just the cursor on top.
        let size = output
            .current_mode()
            .map(|m| {
                // Apply the output's transform so portrait outputs get the
                // logical (post-transform) size.
                let transform = output.current_transform();
                let physical = transform.transform_size(m.size);
                let scale = output.current_scale().fractional_scale();
                Size::<u32, smithay::utils::Logical>::from((
                    (physical.w as f64 / scale).round().max(1.0) as u32,
                    (physical.h as f64 / scale).round().max(1.0) as u32,
                ))
            })
            .unwrap_or_else(|| Size::<u32, smithay::utils::Logical>::from((1280, 720)));

        surface.with_pending_state(|state| {
            state.size = Some(size);
        });
        surface.send_configure();

        tracing::info!(
            "session_lock: new lock surface on {} size {}x{}",
            output.name(),
            size.w,
            size.h
        );

        self.lock_surfaces.push((output, surface));
        // Don't try to set focus here: the wl_surface exists but has no
        // buffer yet, so `wl_keyboard.enter` arrives before Qt's
        // QQuickWindow is paint-ready and the password TextInput's
        // `forceActiveFocus()` no-ops. The commit handler runs the
        // refresh once the surface attaches its first buffer, which
        // both fixes that timing AND picks the lock surface on the
        // user's monitor instead of the first one in `lock_surfaces`.
        self.refresh_keyboard_focus();
        self.request_repaint();
    }
}
delegate_session_lock!(MargoState);
