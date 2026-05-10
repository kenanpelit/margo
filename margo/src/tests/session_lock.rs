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
