//! `wp_fifo_v1` + `wp_commit_timing_v1` barrier-release regressions
//! (road_map P15). The historical failure mode these guard against: a
//! barrier nobody signals wedges the client's commit queue — the
//! hidden-tag Chromium stall that forced the globals' withdrawal.

use std::time::Duration;

use smithay::reexports::rustix;

use super::client::ClientId;
use super::fixture::Fixture;
use wayland_client::protocol::wl_surface::WlSurface;

/// Map a toplevel and give it a real 100×50 buffer so later size changes
/// are observable through the server-side window geometry.
fn map_window_with_buffer(fx: &mut Fixture) -> (ClientId, WlSurface) {
    let id = fx.add_client();
    let (_toplevel, surface) = fx.client(id).create_toplevel();
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);

    let buffer = fx.client(id).create_shm_buffer(100, 50);
    surface.attach(Some(&buffer), 0, 0);
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    (id, surface)
}

fn window_size(fx: &Fixture) -> (i32, i32) {
    let geom = fx.server.state.clients[0].window.geometry();
    (geom.size.w, geom.size.h)
}

fn commit_sized_buffer(fx: &mut Fixture, id: ClientId, surface: &WlSurface, w: i32, h: i32) {
    let buffer = fx.client(id).create_shm_buffer(w, h);
    surface.attach(Some(&buffer), 0, 0);
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
}

#[test]
fn fifo_wait_blocks_until_the_present_pass_releases() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let (id, surface) = map_window_with_buffer(&mut fx);
    let fifo = fx.client(id).get_fifo(&surface);
    fx.roundtrip(id);

    // First pacing commit: sets the barrier the next commit waits on.
    let buffer = fx.client(id).create_shm_buffer(160, 90);
    surface.attach(Some(&buffer), 0, 0);
    fifo.set_barrier();
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    assert_eq!(window_size(&fx), (160, 90));

    // FIFO frame: must stay queued until the compositor presents.
    let buffer = fx.client(id).create_shm_buffer(200, 120);
    surface.attach(Some(&buffer), 0, 0);
    fifo.wait_barrier();
    fifo.set_barrier();
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    assert_eq!(
        window_size(&fx),
        (160, 90),
        "a wait_barrier commit must be held until the barrier is signaled",
    );

    let output = fx.server.state.monitors[0].output.clone();
    fx.server
        .state
        .send_frame_callbacks(&output, Duration::from_millis(16));
    fx.roundtrip(id);
    assert_eq!(
        window_size(&fx),
        (200, 120),
        "the per-output present pass must release the fifo barrier",
    );
}

#[test]
fn fifo_barrier_of_hidden_client_survives_via_fallback_and_tag_return() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let (id, surface) = map_window_with_buffer(&mut fx);
    let fifo = fx.client(id).get_fifo(&surface);
    fx.roundtrip(id);

    // Hide the window's tag, then queue a Chromium-like fifo frame chain.
    fx.server.state.monitors[0].tagset[0] = 0b10;
    fx.server.state.arrange_monitor(0);

    let buffer = fx.client(id).create_shm_buffer(160, 90);
    surface.attach(Some(&buffer), 0, 0);
    fifo.set_barrier();
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);

    let buffer = fx.client(id).create_shm_buffer(200, 120);
    surface.attach(Some(&buffer), 0, 0);
    fifo.wait_barrier();
    fifo.set_barrier();
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    assert_eq!(window_size(&fx), (160, 90));

    // A present on the output must NOT touch the hidden window's barrier —
    // it is not part of that output's scene.
    let output = fx.server.state.monitors[0].output.clone();
    fx.server
        .state
        .send_frame_callbacks(&output, Duration::from_millis(16));
    fx.roundtrip(id);
    assert_eq!(
        window_size(&fx),
        (160, 90),
        "a present pass must not release barriers of windows it does not show",
    );

    // The fallback tick is the hidden client's liveness guarantee.
    fx.server.state.send_frame_callbacks_fallback();
    fx.roundtrip(id);
    assert_eq!(
        window_size(&fx),
        (200, 120),
        "the fallback tick must keep a hidden fifo client's commit queue draining",
    );

    // Regression: the original Brave/YouTube stall — queue another fifo
    // frame while hidden, then return to the tag; the first present after
    // the return must release it.
    let buffer = fx.client(id).create_shm_buffer(240, 140);
    surface.attach(Some(&buffer), 0, 0);
    fifo.wait_barrier();
    fifo.set_barrier();
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    assert_eq!(window_size(&fx), (200, 120));

    fx.server.state.monitors[0].tagset[0] = 0b01;
    fx.server.state.arrange_monitor(0);
    fx.server.state.note_empty_render_per_output(&output);
    fx.roundtrip(id);
    assert_eq!(
        window_size(&fx),
        (240, 140),
        "the first present after a tag return must unblock the fifo client",
    );
}

#[test]
fn commit_timer_holds_future_deadlines_and_releases_when_due() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let (id, surface) = map_window_with_buffer(&mut fx);
    let timer = fx.client(id).get_commit_timer(&surface);
    fx.roundtrip(id);

    // Target a presentation time far in the future.
    let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    let target_sec = now.tv_sec as u64 + 600;
    timer.set_timestamp(
        (target_sec >> 32) as u32,
        target_sec as u32,
        now.tv_nsec as u32,
    );
    commit_sized_buffer(&mut fx, id, &surface, 160, 90);
    assert_eq!(
        window_size(&fx),
        (100, 50),
        "a commit with a future target time must stay queued",
    );
    assert!(
        fx.server.state.commit_timer_wake.is_some(),
        "the pre-commit hook must arm a deadline wake for the timed commit",
    );

    // A present releases only commits due before the *next* refresh.
    let output = fx.server.state.monitors[0].output.clone();
    fx.server
        .state
        .send_frame_callbacks(&output, Duration::from_millis(16));
    fx.roundtrip(id);
    assert_eq!(
        window_size(&fx),
        (100, 50),
        "a present must not release commits targeted beyond the next refresh",
    );

    // Once the deadline is reached the pass must apply the commit.
    let due = (fx.server.state.clock.now() + Duration::from_secs(3600)).into();
    fx.server.state.release_commit_timers_until(due);
    fx.roundtrip(id);
    assert_eq!(
        window_size(&fx),
        (160, 90),
        "release_commit_timers_until must apply commits whose deadline passed",
    );
}

#[test]
fn commit_timer_wake_releases_a_timed_commit_on_an_idle_output() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let (id, surface) = map_window_with_buffer(&mut fx);
    let timer = fx.client(id).get_commit_timer(&surface);
    fx.roundtrip(id);

    // Target ~50 ms out, then drive nothing but the event loop: no present,
    // no fallback. Only the armed deadline wake can release this commit.
    let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    let mut target_nsec = now.tv_nsec as u64 + 50_000_000;
    let mut target_sec = now.tv_sec as u64;
    if target_nsec >= 1_000_000_000 {
        target_sec += 1;
        target_nsec -= 1_000_000_000;
    }
    timer.set_timestamp(
        (target_sec >> 32) as u32,
        target_sec as u32,
        target_nsec as u32,
    );
    commit_sized_buffer(&mut fx, id, &surface, 160, 90);
    assert_eq!(window_size(&fx), (100, 50));

    for _ in 0..100 {
        if window_size(&fx) == (160, 90) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
        fx.dispatch();
        fx.roundtrip(id);
    }
    assert_eq!(
        window_size(&fx),
        (160, 90),
        "the deadline wake must release a timed commit without any present traffic",
    );
}
