//! `zwlr_virtual_pointer_v1` input sanity.

use super::fixture::Fixture;

#[test]
fn absolute_motion_with_a_zero_extent_is_ignored() {
    // Coordinates are fractions of the extents: x / 0 put the pointer at
    // NaN / infinity, which then poisoned every hit test and clamp.
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let id = fx.add_client();
    let pointer = fx.client(id).create_virtual_pointer();
    let before = (
        fx.server.state.input_pointer.x,
        fx.server.state.input_pointer.y,
    );

    pointer.motion_absolute(0, 10, 10, 0, 0);
    pointer.frame();
    fx.client(id).flush();
    fx.roundtrip(id);

    let (x, y) = (
        fx.server.state.input_pointer.x,
        fx.server.state.input_pointer.y,
    );
    assert!(x.is_finite() && y.is_finite(), "pointer at ({x}, {y})");
    assert_eq!((x, y), before);

    // A valid absolute motion still moves the pointer.
    pointer.motion_absolute(0, 1, 1, 2, 2);
    pointer.frame();
    fx.client(id).flush();
    fx.roundtrip(id);
    assert_ne!(
        (
            fx.server.state.input_pointer.x,
            fx.server.state.input_pointer.y
        ),
        before
    );
}
