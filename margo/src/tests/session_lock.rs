//! Integration tests for `SessionLockHandler` (W4.2 Phase 1
//! extracted impl at `state/handlers/session_lock.rs`).
//!
//! `ext-session-lock-v1` is what swaylock / gtklock / noctalia's
//! lock-screen bind. The handler:
//!
//! * `lock(confirmation)` flips `state.session_locked = true`,
//!   confirms the lock to the client, and re-arranges all monitors.
//! * `new_surface(...)` MUST configure with a non-zero size before
//!   the client will attach a buffer — without this, the lock
//!   surface stays unmapped and you get the "alt+l → black screen"
//!   symptom. Tests assert that lock_surfaces grows and that
//!   subsequent locks are tracked.
//! * `unlock()` flips back to false (covered indirectly via
//!   destroy → SessionLocker drop).

use super::fixture::Fixture;

#[test]
fn lock_request_flips_session_locked() {
    let mut fx = Fixture::new();
    let id = fx.add_client();
    assert!(
        !fx.server.state.session_locked,
        "fresh fixture starts unlocked",
    );

    let _lock = fx.client(id).create_session_lock();
    fx.roundtrip(id);

    assert!(
        fx.server.state.session_locked,
        "ext_session_lock.lock() must flip state.session_locked",
    );
}

#[test]
fn a_dead_lock_surface_is_pruned_instead_of_kept_forever() {
    // Reproduces the crash a real incident traced back here: a locking
    // client (mlock) gets SIGKILLed while genuinely locked (e.g. because
    // something ran it with an unexpected flag and then force-killed it
    // after a timeout). Per ext-session-lock-v1's own design, margo is
    // *supposed* to stay locked when that happens -- but it must not keep
    // rendering/touching the now-dead LockSurface forever, which is what
    // crashed the compositor.
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let id = fx.add_client();

    let lock = fx.client(id).create_session_lock();
    let (_compositor, surface) = fx.client(id).create_surface();
    let output = fx.client(id).bind_output();
    let _lock_surface = fx.client(id).create_lock_surface(&lock, &surface, &output);
    fx.roundtrip(id);

    assert!(fx.server.state.session_locked, "lock() must flip the flag");
    assert_eq!(
        fx.server.state.lock_surfaces.len(),
        1,
        "new_surface() must have recorded the lock surface"
    );

    // The client dies WITHOUT unlock_and_destroy -- exactly what
    // spawn_help's 2-second timeout did to mlock in the real incident.
    fx.kill_client(id);

    assert!(
        fx.server.state.session_locked,
        "staying locked after the locker dies is the correct, deliberate \
         security behavior -- do not regress this into an auto-unlock"
    );

    // The bug: without pruning, the dead LockSurface sits in
    // `lock_surfaces` forever, and every subsequent frame's
    // `send_frame_callbacks` (and the udev render path) touches its
    // now-destroyed wl_surface.
    fx.server.state.send_frame_callbacks(
        &fx.server.state.monitors[0].output.clone(),
        std::time::Duration::ZERO,
    );

    assert!(
        fx.server.state.lock_surfaces.is_empty(),
        "a dead LockSurface must be pruned once its client disconnects, \
         not kept around to be touched by later frames"
    );
}

#[test]
fn destroy_lock_object_unlocks() {
    // Per protocol: dropping the lock proxy without `unlock_and_destroy`
    // is an error, but the lock proxy's destroy invokes unlock first.
    // Margo's handler should observe the cleanup and clear
    // session_locked.
    let mut fx = Fixture::new();
    let id = fx.add_client();

    let lock = fx.client(id).create_session_lock();
    fx.roundtrip(id);
    assert!(fx.server.state.session_locked);

    lock.unlock_and_destroy();
    fx.client(id).flush();
    fx.roundtrip(id);

    assert!(
        !fx.server.state.session_locked,
        "unlock_and_destroy must flip session_locked back to false",
    );
}
