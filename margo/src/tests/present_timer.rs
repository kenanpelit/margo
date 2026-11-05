//! A due present timer renders straight away through the backend's repaint
//! hook instead of pinging the repaint source for one more loop wake-up.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::fixture::Fixture;

#[test]
fn a_present_timer_runs_the_repaint_hook_without_a_ping() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let state = &mut fx.server.state;
    let tokens: Vec<_> = state
        .per_output_clocks
        .values_mut()
        .filter_map(|clock| clock.timer_token.take())
        .collect();
    for token in tokens {
        state.loop_handle.remove(token);
    }

    let pings = Arc::new(AtomicUsize::new(0));
    let seen = pings.clone();
    let (ping, source) = calloop::ping::make_ping().expect("test repaint ping");
    state
        .loop_handle
        .insert_source(source, move |(), _, _| {
            seen.fetch_add(1, Ordering::Relaxed);
        })
        .expect("insert test repaint source");
    state.set_repaint_ping(ping);

    let renders = Rc::new(Cell::new(0usize));
    let counted = renders.clone();
    state.set_repaint_hook(Rc::new(move |state: &mut crate::state::MargoState| {
        counted.set(counted.get() + 1);
        // What the udev hook does: take the due outputs and "render" them.
        let due = state.take_due_outputs();
        assert_eq!(due.len(), 1, "the timer fired for a due output");
    }));

    // A non-zero post-empty-render deadline still needs a timer.
    let clock = state.per_output_clocks.entry("DP-1".into()).or_default();
    clock.dirty = true;
    clock.note_estimated_present(std::time::Instant::now());
    state.arm_present_timer("DP-1");
    assert!(state.per_output_clocks["DP-1"].timer_token.is_some());
    std::thread::sleep(std::time::Duration::from_millis(20));

    for _ in 0..5 {
        fx.server.dispatch();
    }
    assert_eq!(renders.get(), 1, "the timer must render through the hook");
    assert_eq!(pings.load(Ordering::Relaxed), 0, "no extra ping wake-up");
}

#[test]
fn without_a_hook_the_present_timer_still_pings() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let state = &mut fx.server.state;
    let tokens: Vec<_> = state
        .per_output_clocks
        .values_mut()
        .filter_map(|clock| clock.timer_token.take())
        .collect();
    for token in tokens {
        state.loop_handle.remove(token);
    }
    let pings = Arc::new(AtomicUsize::new(0));
    let seen = pings.clone();
    let (ping, source) = calloop::ping::make_ping().expect("test repaint ping");
    state
        .loop_handle
        .insert_source(source, move |(), _, _| {
            seen.fetch_add(1, Ordering::Relaxed);
        })
        .expect("insert test repaint source");
    state.set_repaint_ping(ping);
    let clock = state.per_output_clocks.entry("DP-1".into()).or_default();
    clock.dirty = true;
    clock.pending_vblank = false;
    clock.last_present = Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
    state.arm_present_timer("DP-1");
    for _ in 0..5 {
        fx.server.dispatch();
    }
    assert_eq!(pings.load(Ordering::Relaxed), 1);
}

/// Continuous motion on the per-output clock: the vblank lands with damage
/// behind it. The end-of-iteration step renders the output right away
/// (after the animation tick) without allocating a zero-delay timer, so the
/// frame costs no extra loop iteration and the hook runs exactly once.
#[test]
fn a_due_output_renders_at_the_end_of_the_iteration_without_its_timer() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let state = &mut fx.server.state;
    let tokens: Vec<_> = state
        .per_output_clocks
        .values_mut()
        .filter_map(|clock| clock.timer_token.take())
        .collect();
    for token in tokens {
        state.loop_handle.remove(token);
    }
    let renders = Rc::new(Cell::new(0usize));
    let counted = renders.clone();
    state.set_repaint_hook(Rc::new(move |state: &mut crate::state::MargoState| {
        counted.set(counted.get() + 1);
        let due = state.take_due_outputs();
        assert_eq!(due.len(), 1);
    }));
    let output = state.monitors[0].output.clone();

    // A frame is in flight; damage arrives meanwhile.
    let clock = state.per_output_clocks.entry("DP-1".into()).or_default();
    clock.pending_vblank = true;
    clock.dirty = true;
    // Its flip lands: end-of-dispatch rendering needs no timer allocation.
    state.note_backend_vblank(&output, None);
    assert!(state.per_output_clocks["DP-1"].timer_token.is_none());

    state.render_due_after_dispatch();
    assert_eq!(renders.get(), 1, "rendered in the same iteration");
    assert!(
        state.per_output_clocks["DP-1"].timer_token.is_none(),
        "its timer is cancelled"
    );
    for _ in 0..5 {
        fx.server.dispatch();
    }
    assert_eq!(renders.get(), 1, "the cancelled timer never fires");
}

/// Nothing due (in flight, clean, or held back after an empty render): the
/// end-of-iteration step leaves the clocks and their timers alone.
#[test]
fn the_end_of_iteration_step_renders_nothing_that_is_not_due() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let state = &mut fx.server.state;
    let renders = Rc::new(Cell::new(0usize));
    let counted = renders.clone();
    state.set_repaint_hook(Rc::new(move |_: &mut crate::state::MargoState| {
        counted.set(counted.get() + 1);
    }));
    let clock = state.per_output_clocks.entry("DP-1".into()).or_default();
    clock.dirty = true;
    clock.pending_vblank = true;
    state.render_due_after_dispatch();
    let clock = state.per_output_clocks.entry("DP-1".into()).or_default();
    clock.pending_vblank = false;
    clock.dirty = false;
    state.render_due_after_dispatch();
    // After an empty render the next frame waits a refresh interval.
    let clock = state.per_output_clocks.entry("DP-1".into()).or_default();
    clock.dirty = true;
    clock.note_estimated_present(std::time::Instant::now());
    state.render_due_after_dispatch();
    assert_eq!(renders.get(), 0);
}
