//! `wl_surface.preferred_buffer_scale` / `preferred_buffer_transform`
//! (and `wp_fractional_scale_v1.preferred_scale`, sent by the same
//! helper) must reach toplevels when they map, and every window and layer
//! on an output must hear again when its scale or transform changes.
//! Before, only subsurfaces and new layer surfaces got them, so toplevels
//! on fractional outputs fell back to the integer wl_output scale.

use std::collections::HashMap;

use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
use smithay::reexports::wayland_server::protocol::wl_output::Transform as WlTransform;
use wayland_client::Proxy;
use wayland_client::WEnum;
use wayland_client::protocol::wl_output::Transform as ClientTransform;

use super::fixture::Fixture;
use crate::protocols::output_management::{OutputManagementHandler, PendingHeadConfig};

fn apply(fx: &mut Fixture, scale: Option<f64>, transform: Option<WlTransform>) {
    let pending: HashMap<String, PendingHeadConfig> = [(
        "DP-1".to_string(),
        PendingHeadConfig::for_test(scale, transform, None),
    )]
    .into_iter()
    .collect();
    let reply = crate::protocols::output_management::ModeReply::new(|_| {});
    assert!(fx.server.state.apply_output_pending(pending, &reply));
}

#[test]
fn windows_and_layers_follow_the_outputs_scale_and_transform() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (3840, 2160));
    apply(&mut fx, Some(2.0), None);

    let win = fx.add_client();
    let (_toplevel, surface) = fx.client(win).create_toplevel();
    let (child, _sub) = fx.client(win).create_sync_subsurface(&surface);
    surface.commit();
    fx.client(win).flush();
    fx.roundtrip(win);
    let buffer = fx.client(win).create_shm_buffer(64, 64);
    surface.attach(Some(&buffer), 0, 0);
    surface.commit();
    fx.client(win).flush();
    fx.roundtrip(win);
    fx.roundtrip(win);

    let win_id = surface.id().protocol_id();
    let child_id = child.id().protocol_id();
    let scale_of = |fx: &mut Fixture, client, id| {
        fx.client(client)
            .state
            .preferred_buffer_scale
            .get(&id)
            .copied()
    };
    assert_eq!(
        scale_of(&mut fx, win, win_id),
        Some(2),
        "a mapped toplevel hears its output's scale"
    );
    assert_eq!(scale_of(&mut fx, win, child_id), Some(2));

    let bar = fx.add_client();
    let (_layer, layer_surface) = fx.client(bar).create_layer_surface("bar", Layer::Top);
    layer_surface.commit();
    fx.client(bar).flush();
    fx.roundtrip(bar);
    let layer_id = layer_surface.id().protocol_id();
    assert_eq!(scale_of(&mut fx, bar, layer_id), Some(2));

    apply(&mut fx, Some(3.0), Some(WlTransform::_90));
    fx.roundtrip(win);
    fx.roundtrip(bar);

    assert_eq!(scale_of(&mut fx, win, win_id), Some(3));
    assert_eq!(scale_of(&mut fx, win, child_id), Some(3));
    assert_eq!(scale_of(&mut fx, bar, layer_id), Some(3));
    assert_eq!(
        fx.client(win)
            .state
            .preferred_buffer_transform
            .get(&win_id)
            .copied(),
        Some(WEnum::Value(ClientTransform::_90))
    );
    assert_eq!(
        fx.client(bar)
            .state
            .preferred_buffer_transform
            .get(&layer_id)
            .copied(),
        Some(WEnum::Value(ClientTransform::_90))
    );
}

#[test]
fn lock_surfaces_hear_the_outputs_scale() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (3840, 2160));
    apply(&mut fx, Some(2.0), None);
    let id = fx.add_client();
    let lock = fx.client(id).create_session_lock();
    let (_compositor, surface) = fx.client(id).create_surface();
    let output = fx.client(id).bind_output();
    let _lock_surface = fx.client(id).create_lock_surface(&lock, &surface, &output);
    fx.roundtrip(id);
    let sid = surface.id().protocol_id();
    assert_eq!(
        fx.client(id)
            .state
            .preferred_buffer_scale
            .get(&sid)
            .copied(),
        Some(2)
    );
}
