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

/// Lock the session with a live client holding one mapped lock surface on
/// `DP-1`; returns the fixture, the locker's id, and the protocol objects the
/// caller must keep alive.
fn locked_by_live_client() -> (
    Fixture,
    super::client::ClientId,
    impl Sized, // keeps the lock + surface proxies alive
) {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let id = fx.add_client();
    let lock = fx.client(id).create_session_lock();
    let (_compositor, surface) = fx.client(id).create_surface();
    let output = fx.client(id).bind_output();
    let lock_surface = fx.client(id).create_lock_surface(&lock, &surface, &output);
    fx.roundtrip(id);
    assert!(fx.server.state.session_locked);
    (fx, id, (lock, surface, lock_surface, _compositor, output))
}

#[test]
fn force_unlock_is_refused_while_a_live_locker_holds_the_lock() {
    // `force_unlock` is reachable from a keybind (shipped in the default
    // binds for a long time) and from `mctl dispatch`. While the locker is
    // alive and showing its surface, honouring it would unlock the screen
    // without a password.
    let (mut fx, _id, _keep) = locked_by_live_client();

    fx.server.state.force_unlock();

    assert!(
        fx.server.state.session_locked,
        "force_unlock must not tear down a lock held by a live locker"
    );
    assert_eq!(fx.server.state.lock_surfaces.len(), 1);
}

#[test]
fn force_unlock_is_refused_through_dispatch_too() {
    let (mut fx, _id, _keep) = locked_by_live_client();

    let _ =
        crate::dispatch::dispatch_action(&mut fx.server.state, "force_unlock", &Default::default());

    assert!(fx.server.state.session_locked);
}

#[test]
fn force_unlock_recovers_once_the_locker_is_dead() {
    // The escape hatch still exists for its real use: the locker crashed and
    // left the session locked with nothing to type a password into.
    let (mut fx, id, keep) = locked_by_live_client();
    drop(keep);
    fx.kill_client(id);
    assert!(
        fx.server.state.session_locked,
        "a dead locker keeps the lock"
    );

    fx.server.state.force_unlock();

    assert!(!fx.server.state.session_locked);
    assert!(fx.server.state.lock_surfaces.is_empty());
}

#[test]
fn force_unlock_recovers_a_live_locker_that_never_shows_a_surface() {
    // A locker that locked but never created a lock surface leaves a black
    // screen with no password prompt. After a grace period that counts as
    // stuck too.
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let id = fx.add_client();
    let _lock = fx.client(id).create_session_lock();
    fx.roundtrip(id);
    assert!(fx.server.state.session_locked);

    fx.server.state.force_unlock();
    assert!(
        fx.server.state.session_locked,
        "right after lock() the locker may still be creating its surfaces"
    );

    fx.server.state.session_locked_at =
        Some(std::time::Instant::now() - crate::state::FORCE_UNLOCK_NO_SURFACE_GRACE);
    fx.server.state.force_unlock();
    assert!(!fx.server.state.session_locked);
}

#[test]
fn a_second_client_cannot_take_over_a_live_lock() {
    // Before the fix margo confirmed every lock request, which made the
    // newest lock object the owner. Any client could then lock and
    // immediately unlock_and_destroy, unlocking the session that mlock held.
    let (mut fx, locker, _keep) = locked_by_live_client();
    let intruder = fx.add_client();

    let lock = fx.client(intruder).create_session_lock();
    fx.roundtrip(intruder);
    lock.unlock_and_destroy();
    fx.client(intruder).flush();
    for _ in 0..5 {
        fx.dispatch();
    }
    fx.roundtrip(locker);

    assert!(
        fx.server.state.session_locked,
        "a second client must not be able to unlock a live lock"
    );
    assert_eq!(fx.server.state.lock_surfaces.len(), 1);
}

#[test]
fn a_new_locker_can_take_over_after_the_old_one_died() {
    let (mut fx, id, keep) = locked_by_live_client();
    drop(keep);
    fx.kill_client(id);
    assert!(fx.server.state.session_locked);

    let relocker = fx.add_client();
    let lock = fx.client(relocker).create_session_lock();
    fx.roundtrip(relocker);
    assert!(fx.server.state.session_locked);

    lock.unlock_and_destroy();
    fx.client(relocker).flush();
    fx.roundtrip(relocker);
    assert!(
        !fx.server.state.session_locked,
        "a replacement locker owns the lock and can release it"
    );
}

#[test]
fn force_unlock_allowed_policy() {
    use crate::state::{FORCE_UNLOCK_NO_SURFACE_GRACE as GRACE, force_unlock_allowed};
    use std::time::Duration;
    // Dead owner: always recoverable.
    assert!(force_unlock_allowed(false, 0, None));
    assert!(force_unlock_allowed(false, 2, Some(Duration::ZERO)));
    // Live owner showing a surface: never.
    assert!(!force_unlock_allowed(true, 1, Some(GRACE * 100)));
    // Live owner, no surface: only after the grace period.
    assert!(!force_unlock_allowed(true, 0, None));
    assert!(!force_unlock_allowed(
        true,
        0,
        Some(GRACE - Duration::from_millis(1))
    ));
    assert!(force_unlock_allowed(true, 0, Some(GRACE)));
}

#[test]
fn removing_the_focused_monitor_moves_focus_to_a_remaining_lock_surface() {
    // Lid closed with the external monitor docked while locked: the lock
    // surface holding keyboard focus goes away with its output. Keyboard
    // focus has to move to the lock surface that is left, or the password
    // prompt there never sees a key until the pointer happens to move.
    use crate::state::FocusTarget;

    let mut fx = Fixture::new();
    fx.add_keyboard();
    fx.add_output("eDP-1", (1920, 1080));
    fx.add_output("DP-1", (1920, 1080));
    let id = fx.add_client();

    let lock = fx.client(id).create_session_lock();
    let mut kept = Vec::new();
    for n in 0..2 {
        let (_compositor, surface) = fx.client(id).create_surface();
        let output = fx.client(id).bind_output_nth(n);
        let lock_surface = fx.client(id).create_lock_surface(&lock, &surface, &output);
        fx.roundtrip(id);
        let buffer = fx.client(id).create_shm_buffer(1920, 1080);
        surface.attach(Some(&buffer), 0, 0);
        surface.commit();
        fx.client(id).flush();
        fx.roundtrip(id);
        kept.push((surface, output, lock_surface, buffer));
    }
    assert_eq!(fx.server.state.lock_surfaces.len(), 2);

    let focused_output = |fx: &mut Fixture| {
        let focus = fx
            .server
            .state
            .seat
            .get_keyboard()
            .and_then(|k| k.current_focus());
        let Some(FocusTarget::SessionLock(focused)) = focus else {
            return None;
        };
        fx.server
            .state
            .lock_surfaces
            .iter()
            .find(|(_, s)| s == &focused)
            .map(|(o, _)| o.clone())
    };

    let before = focused_output(&mut fx).expect("a lock surface holds keyboard focus");
    fx.server.state.remove_output(&before);

    let after = focused_output(&mut fx)
        .expect("focus must move to the lock surface on the remaining monitor");
    assert_ne!(after, before);
}
