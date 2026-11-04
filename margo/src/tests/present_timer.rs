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

    // A real vblank with damage behind it: zero-delay present timer.
    let clock = state.per_output_clocks.entry("DP-1".into()).or_default();
    clock.dirty = true;
    clock.pending_vblank = false;
    clock.last_present = Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
    state.arm_present_timer("DP-1");
    assert!(state.per_output_clocks["DP-1"].timer_token.is_some());

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
