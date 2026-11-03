//! `MargoMonitor::monitor_area` is the output's rect in layout (logical)
//! space: mode size → transform → fractional scale, the same rect as
//! `Space::output_geometry`. It used to be the DRM mode size (physical
//! pixels) with logical x/y, so on a scaled or rotated output exclusive
//! fullscreen, the pointer clamp, monitor hit-testing and `focus_mon`'s
//! pointer warp all used a rect that was too big or the wrong shape.
//!
//! The changes go through the real wlr-output-management apply handler
//! (`apply_output_pending`), the path wlr-randr / kanshi take.

use std::collections::HashMap;

use smithay::reexports::wayland_server::protocol::wl_output::Transform as WlTransform;

use super::fixture::Fixture;
use crate::layout::Rect;
use crate::protocols::output_management::{OutputManagementHandler, PendingHeadConfig};
use crate::state::FullscreenMode;

fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn apply<const N: usize>(fx: &mut Fixture, heads: [(&str, PendingHeadConfig); N]) {
    let pending: HashMap<String, PendingHeadConfig> = heads
        .into_iter()
        .map(|(name, head)| (name.to_string(), head))
        .collect();
    assert!(fx.server.state.apply_output_pending(pending));
}

fn head(
    scale: Option<f64>,
    transform: Option<WlTransform>,
    pos: Option<(i32, i32)>,
) -> PendingHeadConfig {
    PendingHeadConfig::for_test(scale, transform, pos)
}

/// Map one toplevel with a buffer; it lands on monitor 0.
fn map_window(fx: &mut Fixture) {
    let id = fx.add_client();
    let (_toplevel, surface) = fx.client(id).create_toplevel();
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    let buffer = fx.client(id).create_shm_buffer(64, 64);
    surface.attach(Some(&buffer), 0, 0);
    surface.damage_buffer(0, 0, 64, 64);
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
}

/// Exclusive fullscreen on monitor 0: the arranged rect and the size the
/// client is asked to allocate.
fn exclusive_fullscreen(fx: &mut Fixture) -> (Rect, Option<(i32, i32)>) {
    map_window(fx);
    let state = &mut fx.server.state;
    state.set_client_fullscreen_mode(0, FullscreenMode::Exclusive);
    let c = &state.clients[0];
    let size = c
        .window
        .toplevel()
        .and_then(|t| t.with_pending_state(|s| s.size))
        .map(|s| (s.w, s.h));
    (c.geom, size)
}

fn assert_logical(fx: &Fixture, expected: Rect) {
    let mon = &fx.server.state.monitors[0];
    assert_eq!(
        mon.monitor_area, expected,
        "monitor_area is the logical rect"
    );
    let geo = fx.server.state.space.output_geometry(&mon.output).unwrap();
    assert_eq!(
        (geo.loc.x, geo.loc.y, geo.size.w, geo.size.h),
        (expected.x, expected.y, expected.width, expected.height),
        "and matches the Space"
    );
    assert_eq!(
        mon.work_area, expected,
        "no layers: the work area is all of it"
    );
}

#[test]
fn exclusive_fullscreen_fills_the_logical_rect_of_a_2x_output() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (2560, 1600));
    apply(&mut fx, [("DP-1", head(Some(2.0), None, None))]);
    assert_logical(&fx, rect(0, 0, 1280, 800));

    let (geom, size) = exclusive_fullscreen(&mut fx);
    assert_eq!(geom, rect(0, 0, 1280, 800));
    assert_eq!(size, Some((1280, 800)));
}

#[test]
fn exclusive_fullscreen_fills_the_logical_rect_of_a_1_5x_output() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (2560, 1440));
    apply(&mut fx, [("DP-1", head(Some(1.5), None, None))]);
    // 2560 / 1.5 = 1706.67, rounded up like `Space::output_geometry`.
    assert_logical(&fx, rect(0, 0, 1707, 960));

    let (geom, size) = exclusive_fullscreen(&mut fx);
    assert_eq!(geom, rect(0, 0, 1707, 960));
    assert_eq!(size, Some((1707, 960)));
}

#[test]
fn exclusive_fullscreen_fills_the_logical_rect_of_a_rotated_output() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    apply(
        &mut fx,
        [("DP-1", head(None, Some(WlTransform::_90), None))],
    );
    assert_logical(&fx, rect(0, 0, 1080, 1920));

    let (geom, size) = exclusive_fullscreen(&mut fx);
    assert_eq!(geom, rect(0, 0, 1080, 1920));
    assert_eq!(size, Some((1080, 1920)));
}

#[test]
fn rotated_and_scaled_output_is_transformed_then_scaled() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (2560, 1600));
    apply(
        &mut fx,
        [("DP-1", head(Some(2.0), Some(WlTransform::_270), None))],
    );
    assert_logical(&fx, rect(0, 0, 800, 1280));
}

/// A 2× laptop panel (logical 1280×800) with a 1× monitor right of it.
fn scaled_desk() -> Fixture {
    let mut fx = Fixture::new();
    fx.add_output("eDP-1", (2560, 1600));
    fx.add_output("DP-1", (1920, 1080));
    apply(
        &mut fx,
        [
            ("eDP-1", head(Some(2.0), None, None)),
            ("DP-1", head(None, None, Some((1280, 0)))),
        ],
    );
    assert_eq!(
        fx.server.state.monitors[0].monitor_area,
        rect(0, 0, 1280, 800)
    );
    assert_eq!(
        fx.server.state.monitors[1].monitor_area,
        rect(1280, 0, 1920, 1080)
    );
    fx
}

#[test]
fn pointer_clamp_uses_the_logical_rect() {
    let mut fx = Fixture::new();
    fx.add_output("eDP-1", (2560, 1600));
    apply(&mut fx, [("eDP-1", head(Some(2.0), None, None))]);
    let state = &mut fx.server.state;
    state.input_pointer.x = 2000.0;
    state.input_pointer.y = 1500.0;
    state.clamp_pointer_to_outputs();
    assert_eq!(
        (state.input_pointer.x, state.input_pointer.y),
        (1279.0, 799.0)
    );
}

#[test]
fn hit_testing_and_focus_mon_use_the_logical_rect() {
    let mut fx = scaled_desk();
    let state = &mut fx.server.state;

    // Inside the 1× monitor, but inside eDP-1's physical width.
    state.input_pointer.x = 1500.0;
    state.input_pointer.y = 300.0;
    assert_eq!(state.pointer_monitor(), Some(1));

    // focus_mon warps to the target's centre: eDP-1's logical centre.
    state.focus_mon(-1);
    assert_eq!(
        (state.input_pointer.x, state.input_pointer.y),
        (640.0, 400.0)
    );
    assert_eq!(state.focused_monitor(), 0);

    state.focus_mon(1);
    assert_eq!(
        (state.input_pointer.x, state.input_pointer.y),
        (2240.0, 540.0)
    );
    assert_eq!(state.focused_monitor(), 1);
}

#[test]
fn a_mode_change_resizes_the_monitor_area() {
    // The udev mode-change path sets the new mode on the `Output` and then
    // refreshes the work area; the monitor rect has to follow.
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (2560, 1600));
    apply(&mut fx, [("DP-1", head(Some(2.0), None, None))]);
    let output = fx.server.state.monitors[0].output.clone();
    let mode = smithay::output::Mode {
        size: (1920, 1200).into(),
        refresh: 60_000,
    };
    output.change_current_state(Some(mode), None, None, None);
    fx.server.state.refresh_output_work_area(&output);
    assert_logical(&fx, rect(0, 0, 960, 600));
}

#[test]
fn state_json_keeps_the_mode_size_and_reports_the_logical_size() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (2560, 1600));
    apply(
        &mut fx,
        [("DP-1", head(Some(2.0), Some(WlTransform::_90), None))],
    );
    let snapshot = fx.server.state.build_state_snapshot();
    let o = &snapshot["outputs"][0];
    assert_eq!(
        (o["width"].clone(), o["height"].clone()),
        (2560.into(), 1600.into())
    );
    assert_eq!(
        (o["logical_width"].clone(), o["logical_height"].clone()),
        (800.into(), 1280.into())
    );
}

#[test]
fn state_json_reports_the_live_transform_and_scale() {
    let cases = [
        (WlTransform::Normal, 0),
        (WlTransform::_90, 1),
        (WlTransform::_180, 2),
        (WlTransform::_270, 3),
        (WlTransform::Flipped, 4),
        (WlTransform::Flipped90, 5),
        (WlTransform::Flipped180, 6),
        (WlTransform::Flipped270, 7),
    ];
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (2560, 1600));
    for (transform, wire) in cases {
        apply(&mut fx, [("DP-1", head(Some(2.0), Some(transform), None))]);
        let snapshot = fx.server.state.build_state_snapshot();
        let o = &snapshot["outputs"][0];
        assert_eq!(o["transform"], wire, "{transform:?}");
        assert_eq!(o["scale"], 2.0, "{transform:?}");
    }
}
