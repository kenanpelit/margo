//! Repaint/frame-pacing regressions modelled after Chromium's synchronized
//! subsurface tree and a two-output tag workflow.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use smithay::wayland::seat::WaylandFocus;

use super::client::ClientId;
use super::fixture::Fixture;

fn reset_clocks(fx: &mut Fixture) {
    let tokens: Vec<_> = fx
        .server
        .state
        .per_output_clocks
        .values_mut()
        .filter_map(|clock| clock.timer_token.take())
        .collect();
    for token in tokens {
        fx.server.state.loop_handle.remove(token);
    }
    for clock in fx.server.state.per_output_clocks.values_mut() {
        clock.dirty = false;
        clock.pending_vblank = false;
        clock.last_present = Some(std::time::Instant::now());
    }
    fx.server.state.take_repaint_request();
}

fn map_window(fx: &mut Fixture) -> (ClientId, wayland_client::protocol::wl_surface::WlSurface) {
    let id = fx.add_client();
    let (_toplevel, surface) = fx.client(id).create_toplevel();
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    (id, surface)
}

#[test]
fn exact_output_repaint_does_not_dirty_or_rearm_the_other_output() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));
    reset_clocks(&mut fx);

    let output = fx.server.state.monitors[0].output.clone();
    fx.server.state.request_repaint_output(&output);
    let first_token = fx.server.state.per_output_clocks["DP-1"].timer_token;
    assert!(fx.server.state.per_output_clocks["DP-1"].dirty);
    assert!(first_token.is_some());

    for _ in 0..10_000 {
        fx.server.state.request_repaint_output(&output);
    }

    let a = &fx.server.state.per_output_clocks["DP-1"];
    assert_eq!(
        a.timer_token, first_token,
        "commit burst must reuse one timer"
    );
    assert!(a.dirty);
    assert!(
        fx.server
            .state
            .per_output_clocks
            .get("DP-2")
            .is_none_or(|clock| !clock.dirty),
        "DP-1 damage must never dirty DP-2",
    );
}

#[test]
fn backend_wake_bypasses_a_stalled_output_clock() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    reset_clocks(&mut fx);

    // Model a DPMS-off clock wedged in the in-flight state: there is no real
    // vblank or timer left to wake it, even though `pending_vblank` ordinarily
    // means one is expected. A global request must still ping because the
    // repaint source is also what drains pending DPMS/mode/capture work.
    let clock = fx
        .server
        .state
        .per_output_clocks
        .entry("DP-1".into())
        .or_default();
    clock.dirty = true;
    clock.pending_vblank = true;
    clock.timer_token = None;

    let wakes = Arc::new(AtomicUsize::new(0));
    let seen = wakes.clone();
    let (ping, source) = calloop::ping::make_ping().expect("test repaint ping");
    fx.server
        .state
        .loop_handle
        .insert_source(source, move |(), _, _| {
            seen.fetch_add(1, Ordering::Relaxed);
        })
        .expect("insert test repaint source");
    fx.server.state.set_repaint_ping(ping);

    fx.server.state.wake_repaint_backend();
    fx.server.dispatch();

    assert_eq!(
        wakes.load(Ordering::Relaxed),
        1,
        "global repaint must wake even when a clock claims a vblank is pending",
    );
}

#[test]
fn global_scene_repaint_coalesces_behind_the_output_clock() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    reset_clocks(&mut fx);

    // Keep the deadline far enough away that this test proves an event-loop
    // wake came from the API itself, not from a racy 16.7 ms present timer.
    fx.server
        .state
        .per_output_clocks
        .entry("DP-1".into())
        .or_default()
        .last_present = Some(std::time::Instant::now() + std::time::Duration::from_secs(60));

    let wakes = Arc::new(AtomicUsize::new(0));
    let seen = wakes.clone();
    let (ping, source) = calloop::ping::make_ping().expect("test scene repaint ping");
    fx.server
        .state
        .loop_handle
        .insert_source(source, move |(), _, _| {
            seen.fetch_add(1, Ordering::Relaxed);
        })
        .expect("insert test scene repaint source");
    fx.server.state.set_repaint_ping(ping);

    fx.server.state.request_scene_repaint();
    let timer = fx.server.state.per_output_clocks["DP-1"].timer_token;
    assert!(timer.is_some(), "scene damage must arm one present timer");
    for _ in 0..10_000 {
        fx.server.state.request_scene_repaint();
    }
    assert_eq!(
        fx.server.state.per_output_clocks["DP-1"].timer_token, timer,
        "animation ticks must coalesce behind the existing timer",
    );
    for _ in 0..10 {
        fx.server.dispatch();
    }
    assert_eq!(
        wakes.load(Ordering::Relaxed),
        0,
        "scene animation must not self-ping before its frame-clock deadline",
    );

    // Backend work uses the intentionally immediate API and must still be
    // able to bypass a clock that cannot wake (DPMS/mode/capture queues).
    fx.server.state.wake_repaint_backend();
    fx.server.dispatch();
    assert_eq!(wakes.load(Ordering::Relaxed), 1);
}

#[test]
fn per_output_scene_repaint_respects_render_retry_backoff() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    reset_clocks(&mut fx);
    let output = fx.server.state.monitors[0].output.clone();

    let wakes = Arc::new(AtomicUsize::new(0));
    let seen = wakes.clone();
    let (ping, source) = calloop::ping::make_ping().expect("test retry scene ping");
    fx.server
        .state
        .loop_handle
        .insert_source(source, move |(), _, _| {
            seen.fetch_add(1, Ordering::Relaxed);
        })
        .expect("insert retry scene ping source");
    fx.server.state.set_repaint_ping(ping);

    // A large streak selects the one-second cap, avoiding a timing race with
    // the fixture while still exercising the exact production backoff path.
    fx.server.state.defer_output_render_retry(&output, 100);
    for _ in 0..10_000 {
        fx.server.state.request_scene_repaint();
    }
    for _ in 0..10 {
        fx.server.dispatch();
    }
    assert_eq!(
        wakes.load(Ordering::Relaxed),
        0,
        "animation damage must not bypass a failed output's retry deadline",
    );
    assert!(fx.server.state.output_render_retry_pending(&output));
}

#[test]
fn legacy_scene_repaint_waits_for_estimated_vblank_deadline() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    reset_clocks(&mut fx);
    fx.server.state.config.per_output_frame_clock = false;
    let output = fx.server.state.monitors[0].output.clone();

    let wakes = Arc::new(AtomicUsize::new(0));
    let seen = wakes.clone();
    let (ping, source) = calloop::ping::make_ping().expect("test legacy scene ping");
    fx.server
        .state
        .loop_handle
        .insert_source(source, move |(), _, _| {
            seen.fetch_add(1, Ordering::Relaxed);
        })
        .expect("insert legacy scene ping source");
    fx.server.state.set_repaint_ping(ping);

    // Model an empty frame whose estimated-vblank callback owns the next
    // deadline. Repeated animation ticks must not create an immediate ping.
    fx.server
        .state
        .queue_estimated_vblank_timer(&output, std::time::Duration::from_secs(60));
    for _ in 0..10_000 {
        fx.server.state.request_repaint_output(&output);
    }
    for _ in 0..10 {
        fx.server.dispatch();
    }
    assert_eq!(wakes.load(Ordering::Relaxed), 0);

    // At the real deadline the estimated-vblank callback transfers the
    // accumulated damage back to the repaint source exactly once.
    fx.server.state.on_estimated_vblank_timer(&output);
    fx.server.dispatch();
    assert_eq!(wakes.load(Ordering::Relaxed), 1);
}

#[test]
fn forced_all_output_render_uses_per_output_vblank_accounting() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    reset_clocks(&mut fx);
    let output = fx.server.state.monitors[0].output.clone();

    fx.server.state.request_repaint_output(&output);
    assert!(fx.server.state.per_output_clocks["DP-1"].dirty);

    fx.server.state.begin_forced_render_per_output(&output);
    let clock = &fx.server.state.per_output_clocks["DP-1"];
    assert!(!clock.dirty, "forced render consumes the pending damage");
    assert!(
        clock.pending_vblank,
        "forced render must wait on the output clock, not the legacy global counter",
    );

    // Empty/error paths have no DRM event and must release the same gate.
    fx.server.state.note_empty_render_per_output(&output);
    assert!(!fx.server.state.per_output_clocks["DP-1"].pending_vblank);
}

#[test]
fn vblank_completion_survives_frame_clock_mode_reload() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    reset_clocks(&mut fx);
    let output = fx.server.state.monitors[0].output.clone();

    // true -> false while a per-output frame is in flight must still clear
    // the per-output gate.
    fx.server.state.begin_forced_render_per_output(&output);
    fx.server.state.config.per_output_frame_clock = false;
    fx.server.state.note_backend_vblank(&output);
    assert!(!fx.server.state.per_output_clocks["DP-1"].pending_vblank);

    // false -> true while a legacy frame is in flight must still drain the
    // legacy counter and deliver the deferred global wake.
    let wakes = Arc::new(AtomicUsize::new(0));
    let seen = wakes.clone();
    let (ping, source) = calloop::ping::make_ping().expect("test repaint ping");
    fx.server
        .state
        .loop_handle
        .insert_source(source, move |(), _, _| {
            seen.fetch_add(1, Ordering::Relaxed);
        })
        .expect("insert test repaint source");
    fx.server.state.set_repaint_ping(ping);
    fx.server.state.note_frame_queued();
    fx.server.state.request_repaint();
    fx.server.state.config.per_output_frame_clock = true;
    fx.server.state.note_backend_vblank(&output);
    fx.server.dispatch();
    assert_eq!(wakes.load(Ordering::Relaxed), 1);
}

#[test]
fn synchronized_child_commit_waits_for_root_and_scopes_to_owner_output() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));
    let (id, parent) = map_window(&mut fx);
    let (child, _subsurface) = fx.client(id).create_sync_subsurface(&parent);
    fx.roundtrip(id);
    reset_clocks(&mut fx);

    child.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    assert!(
        fx.server
            .state
            .per_output_clocks
            .values()
            .all(|clock| !clock.dirty),
        "a synchronized child commit is not applied until its root commits",
    );

    parent.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    assert!(fx.server.state.per_output_clocks["DP-1"].dirty);
    assert!(
        fx.server
            .state
            .per_output_clocks
            .get("DP-2")
            .is_none_or(|clock| !clock.dirty),
    );
}

#[test]
fn spanning_floating_commit_repaints_every_overlapped_output() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));
    let (id, parent) = map_window(&mut fx);
    fx.server.state.clients[0].is_floating = true;

    // Make the two headless outputs overlap, then refresh Space so the test
    // window has a real two-output membership without needing a GPU buffer.
    let second = fx.server.state.monitors[1].output.clone();
    fx.server.state.space.map_output(&second, (0, 0));
    second.change_current_state(None, None, None, Some((0, 0).into()));
    fx.server.state.space.refresh();
    let window = fx.server.state.clients[0].window.clone();
    assert_eq!(
        fx.server.state.space.outputs_for_element(&window).len(),
        2,
        "fixture must model a window spanning both outputs",
    );
    reset_clocks(&mut fx);

    parent.commit();
    fx.client(id).flush();
    fx.roundtrip(id);

    assert!(fx.server.state.per_output_clocks["DP-1"].dirty);
    assert!(fx.server.state.per_output_clocks["DP-2"].dirty);
}

#[test]
fn commit_refreshes_stale_space_membership_before_the_render_batch() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));
    let (id, parent) = map_window(&mut fx);
    let window = fx.server.state.clients[0].window.clone();
    let first = fx.server.state.monitors[0].output.clone();
    let second = fx.server.state.monitors[1].output.clone();

    // Give the window a real surface-tree bbox; a bufferless xdg initial
    // commit has no stable footprint after `Window::on_commit`.
    let buffer = fx.client(id).create_shm_buffer(640, 480);
    parent.attach(Some(&buffer), 0, 0);
    parent.damage_buffer(0, 0, 640, 480);
    parent.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    fx.server.state.space.refresh();
    assert_eq!(
        fx.server.state.space.outputs_for_element(&window),
        vec![first.clone()]
    );

    // `map_element` deliberately retains Smithay's cached output set until
    // `Space::refresh`. This models a root commit whose fresh surface-tree
    // bbox has crossed onto DP-2 while the cached membership still says DP-1.
    // The commit handler must refresh before either due output is rendered.
    fx.server.state.clients[0].geom.x = 1920;
    fx.server
        .state
        .space
        .map_element(window.clone(), (1920, 0), false);
    assert_eq!(
        fx.server.state.space.outputs_for_element(&window),
        vec![first]
    );
    reset_clocks(&mut fx);

    parent.commit();
    fx.client(id).flush();
    fx.roundtrip(id);

    assert_eq!(
        fx.server.state.space.outputs_for_element(&window),
        vec![second],
        "the first render batch must observe the freshly committed footprint",
    );
    assert!(fx.server.state.per_output_clocks["DP-1"].dirty);
    assert!(fx.server.state.per_output_clocks["DP-2"].dirty);
}

#[test]
fn offscreen_scroller_tile_commit_does_not_dirty_kms() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));
    let (id, parent) = map_window(&mut fx);
    fx.server.state.config.warmup_hidden_ms = 0;

    // Scroller keeps same-tag columns mapped outside the visible strip. They
    // are logically on the active tag but have no physical output footprint.
    fx.server.state.clients[0].geom.x = -10_000;
    let window = fx.server.state.clients[0].window.clone();
    fx.server
        .state
        .space
        .map_element(window, (-10_000, 0), false);
    fx.server.state.space.refresh();
    assert!(
        fx.server
            .state
            .space
            .outputs_for_element(&fx.server.state.clients[0].window)
            .is_empty(),
    );
    reset_clocks(&mut fx);

    parent.commit();
    fx.client(id).flush();
    fx.roundtrip(id);

    assert!(
        fx.server
            .state
            .per_output_clocks
            .values()
            .all(|clock| !clock.dirty),
        "an offscreen same-tag column must not produce empty KMS renders",
    );
}

#[test]
fn arrange_move_repaints_old_and_new_output_footprints() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));
    let (_id, _parent) = map_window(&mut fx);
    let window = fx.server.state.clients[0].window.clone();
    assert!(
        fx.server
            .state
            .space
            .outputs_for_element(&window)
            .contains(&fx.server.state.monitors[0].output),
    );
    reset_clocks(&mut fx);

    // Reassign before arranging, matching tag_mon/output-rule flows. The
    // arrange pass must retain DP-1's old footprint and add DP-2's new one.
    fx.server.state.clients[0].monitor = 1;
    fx.server.state.arrange_monitor(1);

    assert!(fx.server.state.per_output_clocks["DP-1"].dirty);
    assert!(fx.server.state.per_output_clocks["DP-2"].dirty);
}

#[test]
fn hidden_chromium_tree_gets_callback_on_first_empty_present_after_remap() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));
    let (id, parent) = map_window(&mut fx);
    let (child, _subsurface) = fx.client(id).create_sync_subsurface(&parent);
    fx.roundtrip(id);
    fx.server.state.config.warmup_hidden_ms = 0;

    // Hide tag 1, then queue a Chromium-like synchronized child frame while
    // the toplevel is absent from Space.
    fx.server.state.monitors[0].tagset[0] = 0b10;
    fx.server.state.arrange_monitor(0);
    reset_clocks(&mut fx);
    let done = fx.client(id).request_frame(&child);
    child.commit();
    parent.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    assert!(!done.load(Ordering::Relaxed));
    assert!(
        fx.server
            .state
            .per_output_clocks
            .values()
            .all(|clock| !clock.dirty),
        "a hidden Chromium commit must not produce an empty KMS render loop",
    );

    // Remap tag 1. `arrange_monitor` must refresh Space membership before an
    // empty present tries to route callbacks to this output.
    fx.server.state.monitors[0].tagset[0] = 0b01;
    fx.server.state.arrange_monitor(0);
    let output = fx.server.state.monitors[0].output.clone();
    let window = fx.server.state.clients[0].window.clone();
    assert!(
        fx.server
            .state
            .space
            .outputs_for_element(&window)
            .contains(&output),
        "remapped window must own DP-1 before the first render result",
    );
    assert!(fx.server.state.per_output_clocks["DP-1"].dirty);
    assert!(
        fx.server
            .state
            .per_output_clocks
            .get("DP-2")
            .is_none_or(|clock| !clock.dirty),
        "tag return must dirty only the window's owner output",
    );

    fx.server.state.note_empty_render_per_output(&output);
    fx.server
        .state
        .display_handle
        .flush_clients()
        .expect("flush frame callback");
    for _ in 0..10 {
        if done.load(Ordering::Relaxed) {
            break;
        }
        fx.dispatch();
    }
    assert!(
        done.load(Ordering::Relaxed),
        "the first empty present after tag return must resume the child callback",
    );
}

#[test]
fn hidden_frame_callback_fallback_does_not_dirty_kms() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));
    let (id, parent) = map_window(&mut fx);
    fx.server.state.config.warmup_hidden_ms = 0;

    fx.server.state.monitors[0].tagset[0] = 0b10;
    fx.server.state.arrange_monitor(0);
    reset_clocks(&mut fx);
    let done = fx.client(id).request_frame(&parent);
    parent.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    assert!(
        fx.server
            .state
            .per_output_clocks
            .values()
            .all(|clock| !clock.dirty),
        "the hidden client's commit itself must stay off the KMS clock",
    );

    // The production fallback timer runs once per second and Smithay's
    // per-surface throttle is 995 ms. Model that deadline rather than calling
    // the fallback immediately after the request.
    std::thread::sleep(std::time::Duration::from_secs(1));
    fx.server.state.send_frame_callbacks_fallback();
    for _ in 0..10 {
        if done.load(Ordering::Relaxed) {
            break;
        }
        fx.dispatch();
    }

    assert!(done.load(Ordering::Relaxed));
    assert!(
        fx.server
            .state
            .per_output_clocks
            .values()
            .all(|clock| !clock.dirty),
        "fallback callbacks must not schedule a physical output render",
    );
}

#[test]
fn hidden_warmup_is_callback_only_and_never_dirties_kms() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let (id, parent) = map_window(&mut fx);
    assert!(fx.server.state.config.warmup_hidden_ms > 0);
    assert!(fx.server.state.clients[0].mapped_at.is_some());

    fx.server.state.monitors[0].tagset[0] = 0b10;
    fx.server.state.arrange_monitor(0);
    reset_clocks(&mut fx);

    parent.commit();
    fx.client(id).flush();
    fx.roundtrip(id);

    assert!(
        fx.server
            .state
            .per_output_clocks
            .values()
            .all(|clock| !clock.dirty),
        "warm-up must drive wl_surface callbacks, never invisible KMS frames",
    );
    assert_eq!(
        fx.server.state.frame_callback_fallback_interval(),
        std::time::Duration::from_millis(32),
    );
}

#[test]
fn visible_and_hidden_scratchpad_commits_follow_render_visibility() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let (id, parent) = map_window(&mut fx);
    fx.server.state.clients[0].is_in_scratchpad = true;
    fx.server.state.clients[0].is_scratchpad_show = true;
    reset_clocks(&mut fx);

    parent.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    assert!(
        fx.server.state.per_output_clocks["DP-1"].dirty,
        "a shown scratchpad is part of the live scene",
    );

    fx.server.state.clients[0].is_scratchpad_show = false;
    reset_clocks(&mut fx);
    parent.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    assert!(
        fx.server
            .state
            .per_output_clocks
            .values()
            .all(|clock| !clock.dirty),
        "a hidden scratchpad must stay off the physical frame clock",
    );
}

#[test]
fn fallback_does_not_bypass_vblank_for_a_visible_window() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let (id, parent) = map_window(&mut fx);
    let done = fx.client(id).request_frame(&parent);
    parent.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    reset_clocks(&mut fx);

    fx.server.state.send_frame_callbacks_fallback();
    for _ in 0..10 {
        fx.dispatch();
    }

    assert!(
        !done.load(Ordering::Relaxed),
        "visible surfaces must remain paced by their output's vblank",
    );
    assert!(
        fx.server
            .state
            .per_output_clocks
            .values()
            .all(|clock| !clock.dirty),
    );
}

#[test]
fn client_commit_behind_session_lock_does_not_dirty_kms() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));
    let (id, parent) = map_window(&mut fx);
    fx.server.state.session_locked = true;
    reset_clocks(&mut fx);

    parent.commit();
    fx.client(id).flush();
    fx.roundtrip(id);

    assert!(
        fx.server
            .state
            .per_output_clocks
            .values()
            .all(|clock| !clock.dirty),
        "a client hidden by the session lock must not produce an empty KMS frame",
    );
}

#[test]
fn window_activation_cannot_mutate_the_desktop_behind_session_lock() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let (_id, _surface) = map_window(&mut fx);
    fx.server.state.monitors[0].tagset[0] = 0b10;
    fx.server.state.clients[0].tags = 0b01;
    fx.server.state.session_locked = true;
    reset_clocks(&mut fx);

    fx.server.state.activate_window_idx(0);

    assert_eq!(fx.server.state.monitors[0].current_tagset(), 0b10);
    assert!(
        fx.server
            .state
            .per_output_clocks
            .values()
            .all(|clock| !clock.dirty),
        "foreign-toplevel/IPC activation must not raise or repaint hidden windows",
    );
}

#[test]
fn lock_vblank_does_not_drive_background_client_callbacks() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let (id, parent) = map_window(&mut fx);
    let done = fx.client(id).request_frame(&parent);
    parent.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    let output = fx.server.state.monitors[0].output.clone();
    fx.server.state.session_locked = true;

    fx.server
        .state
        .send_frame_callbacks(&output, fx.server.state.clock.now());
    fx.server
        .state
        .display_handle
        .flush_clients()
        .expect("flush lock callback test");
    for _ in 0..10 {
        fx.dispatch();
    }

    assert!(
        !done.load(Ordering::Relaxed),
        "lock refreshes must not pace hidden background applications",
    );
}

#[test]
fn layer_commit_behind_session_lock_does_not_dirty_kms() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));
    let id = fx.add_client();
    let (_layer, surface) = fx.client(id).create_layer_surface(
        "lock-hidden-test",
        smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer::Top,
    );
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    fx.server.state.session_locked = true;
    reset_clocks(&mut fx);

    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);

    assert!(
        fx.server
            .state
            .per_output_clocks
            .values()
            .all(|clock| !clock.dirty),
        "layer content hidden by the session lock must not repaint KMS",
    );
}

#[test]
fn suppressed_layers_do_not_drive_callbacks_or_kms() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let (_window_id, _window) = map_window(&mut fx);
    let layer_id = fx.add_client();
    let (_layer, surface) = fx.client(layer_id).create_layer_surface(
        "suppressed-layer-test",
        smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer::Top,
    );
    surface.commit();
    fx.client(layer_id).flush();
    fx.roundtrip(layer_id);
    let output = fx.server.state.monitors[0].output.clone();

    fx.server.state.clients[0].fullscreen_mode = crate::state::FullscreenMode::Exclusive;
    reset_clocks(&mut fx);
    let fullscreen_done = fx.client(layer_id).request_frame(&surface);
    surface.commit();
    fx.client(layer_id).flush();
    fx.roundtrip(layer_id);
    fx.server
        .state
        .send_frame_callbacks(&output, fx.server.state.clock.now());
    fx.server
        .state
        .display_handle
        .flush_clients()
        .expect("flush fullscreen layer callback test");
    for _ in 0..10 {
        fx.dispatch();
    }
    assert!(!fullscreen_done.load(Ordering::Relaxed));
    assert!(
        fx.server
            .state
            .per_output_clocks
            .values()
            .all(|clock| !clock.dirty),
        "exclusive fullscreen must suppress hidden layer KMS work",
    );

    fx.server.state.clients[0].fullscreen_mode = crate::state::FullscreenMode::Off;
    fx.server.state.open_scroller_overview();
    reset_clocks(&mut fx);
    let overview_done = fx.client(layer_id).request_frame(&surface);
    surface.commit();
    fx.client(layer_id).flush();
    fx.roundtrip(layer_id);
    fx.server
        .state
        .send_frame_callbacks(&output, fx.server.state.clock.now());
    fx.server
        .state
        .display_handle
        .flush_clients()
        .expect("flush overview layer callback test");
    for _ in 0..10 {
        fx.dispatch();
    }
    assert!(!overview_done.load(Ordering::Relaxed));
    assert!(
        fx.server
            .state
            .per_output_clocks
            .values()
            .all(|clock| !clock.dirty),
        "scroller overview must suppress hidden layer KMS work",
    );
}

#[test]
fn exclusive_fullscreen_keeps_normal_space_window_work_consistent() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let (exclusive_id, exclusive_surface) = map_window(&mut fx);
    let (background_id, background_surface) = map_window(&mut fx);
    fx.server.state.clients[0].fullscreen_mode = crate::state::FullscreenMode::Exclusive;
    let output = fx.server.state.monitors[0].output.clone();
    reset_clocks(&mut fx);

    let background_done = fx.client(background_id).request_frame(&background_surface);
    background_surface.commit();
    fx.client(background_id).flush();
    fx.roundtrip(background_id);
    assert!(
        fx.server
            .state
            .per_output_clocks
            .values()
            .any(|clock| clock.dirty),
        "exclusive mode suppresses layers, not normal Space windows",
    );

    let exclusive_done = fx.client(exclusive_id).request_frame(&exclusive_surface);
    exclusive_surface.commit();
    fx.client(exclusive_id).flush();
    fx.roundtrip(exclusive_id);
    fx.server
        .state
        .send_frame_callbacks(&output, fx.server.state.clock.now());
    fx.server
        .state
        .display_handle
        .flush_clients()
        .expect("flush exclusive fullscreen callback test");
    for _ in 0..10 {
        fx.dispatch();
    }

    assert!(
        background_done.load(Ordering::Relaxed),
        "rendered Space windows must keep receiving callbacks",
    );
    assert!(
        exclusive_done.load(Ordering::Relaxed),
        "the exclusive client must remain paced by its output",
    );
}

#[test]
fn scroller_overview_keeps_background_layers_live() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let layer_id = fx.add_client();
    let (_layer, surface) = fx.client(layer_id).create_layer_surface(
        "overview-wallpaper-test",
        smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer::Background,
    );
    surface.commit();
    fx.client(layer_id).flush();
    fx.roundtrip(layer_id);
    fx.server.state.open_scroller_overview();
    reset_clocks(&mut fx);
    let output = fx.server.state.monitors[0].output.clone();

    let done = fx.client(layer_id).request_frame(&surface);
    surface.commit();
    fx.client(layer_id).flush();
    fx.roundtrip(layer_id);
    assert!(fx.server.state.per_output_clocks["DP-1"].dirty);
    fx.server
        .state
        .send_frame_callbacks(&output, fx.server.state.clock.now());
    fx.server
        .state
        .display_handle
        .flush_clients()
        .expect("flush scroller background callback test");
    for _ in 0..10 {
        fx.dispatch();
    }
    assert!(done.load(Ordering::Relaxed));
}

#[test]
fn classic_overview_restores_layers_over_exclusive_clients() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let (_window_id, _window) = map_window(&mut fx);
    let layer_id = fx.add_client();
    let (_layer, surface) = fx.client(layer_id).create_layer_surface(
        "classic-overview-layer-test",
        smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer::Top,
    );
    surface.commit();
    fx.client(layer_id).flush();
    fx.roundtrip(layer_id);
    fx.server.state.clients[0].fullscreen_mode = crate::state::FullscreenMode::Exclusive;
    fx.server.state.monitors[0].is_overview = true;
    reset_clocks(&mut fx);
    let output = fx.server.state.monitors[0].output.clone();

    let done = fx.client(layer_id).request_frame(&surface);
    surface.commit();
    fx.client(layer_id).flush();
    fx.roundtrip(layer_id);
    assert!(fx.server.state.per_output_clocks["DP-1"].dirty);
    fx.server
        .state
        .send_frame_callbacks(&output, fx.server.state.clock.now());
    fx.server
        .state
        .display_handle
        .flush_clients()
        .expect("flush classic overview layer callback test");
    for _ in 0..10 {
        fx.dispatch();
    }
    assert!(done.load(Ordering::Relaxed));
}

#[test]
fn first_toplevel_commit_while_locked_stays_deferred_and_off_kms() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_keyboard();
    fx.server.state.session_locked = true;
    let id = fx.add_client();
    let (_toplevel, surface) = fx.client(id).create_toplevel();
    reset_clocks(&mut fx);

    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);

    assert_eq!(fx.server.state.clients.len(), 1);
    assert!(fx.server.state.clients[0].is_initial_map_pending);
    assert!(fx.server.state.focused_client_idx().is_none());
    assert!(
        fx.server
            .state
            .per_output_clocks
            .values()
            .all(|clock| !clock.dirty),
        "a toplevel created behind the lock must not map or repaint KMS",
    );
}

#[test]
fn legacy_render_retry_suppresses_immediate_global_wakes() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    reset_clocks(&mut fx);
    fx.server.state.config.per_output_frame_clock = false;
    let output = fx.server.state.monitors[0].output.clone();

    let wakes = Arc::new(AtomicUsize::new(0));
    let seen = wakes.clone();
    let (ping, source) = calloop::ping::make_ping().expect("test legacy retry ping");
    fx.server
        .state
        .loop_handle
        .insert_source(source, move |(), _, _| {
            seen.fetch_add(1, Ordering::Relaxed);
        })
        .expect("insert legacy retry ping source");
    fx.server.state.set_repaint_ping(ping);

    fx.server.state.defer_output_render_retry(&output, 1);
    for _ in 0..100 {
        fx.server.state.request_repaint_output(&output);
        fx.server.dispatch();
    }
    assert_eq!(
        wakes.load(Ordering::Relaxed),
        0,
        "legacy animation damage must wait behind the output retry gate",
    );

    std::thread::sleep(std::time::Duration::from_millis(25));
    for _ in 0..10 {
        fx.server.dispatch();
    }
    assert_eq!(wakes.load(Ordering::Relaxed), 1);
}

#[test]
fn render_retry_waits_for_backoff_and_coalesces_damage() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    reset_clocks(&mut fx);
    let output = fx.server.state.monitors[0].output.clone();

    let wakes = Arc::new(AtomicUsize::new(0));
    let seen = wakes.clone();
    let (ping, source) = calloop::ping::make_ping().expect("test retry ping");
    fx.server
        .state
        .loop_handle
        .insert_source(source, move |(), _, _| {
            seen.fetch_add(1, Ordering::Relaxed);
        })
        .expect("insert retry ping source");
    fx.server.state.set_repaint_ping(ping);

    fx.server.state.defer_output_render_retry(&output, 1);
    fx.server.state.request_repaint_output(&output);
    fx.server.state.defer_output_render_retry(&output, 2);
    for _ in 0..5 {
        fx.server.dispatch();
    }
    assert_eq!(
        wakes.load(Ordering::Relaxed),
        0,
        "damage during backoff must not start an immediate retry loop",
    );
    assert!(fx.server.state.output_render_retry_pending(&output));

    std::thread::sleep(std::time::Duration::from_millis(25));
    for _ in 0..10 {
        fx.server.dispatch();
    }
    assert_eq!(wakes.load(Ordering::Relaxed), 1);
    assert!(!fx.server.state.output_render_retry_pending(&output));
}

#[test]
fn cursor_commit_repaints_only_the_pointer_output() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));
    let (id, cursor) = map_window(&mut fx);
    let server_cursor = fx.server.state.clients[0]
        .window
        .wl_surface()
        .expect("test cursor server surface")
        .into_owned();
    fx.server.state.cursor_status =
        smithay::input::pointer::CursorImageStatus::Surface(server_cursor);
    fx.server.state.input_pointer.x = 960.0;
    fx.server.state.input_pointer.y = 540.0;
    reset_clocks(&mut fx);

    cursor.commit();
    fx.client(id).flush();
    fx.roundtrip(id);

    assert!(fx.server.state.per_output_clocks["DP-1"].dirty);
    assert!(
        fx.server
            .state
            .per_output_clocks
            .get("DP-2")
            .is_none_or(|clock| !clock.dirty),
    );
}
