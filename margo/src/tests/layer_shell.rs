//! Integration tests for `WlrLayerShellHandler` (W4.2 Phase 1
//! extracted impl at `state/handlers/layer_shell.rs`).
//!
//! Layer-shell is what bars (waybar, noctalia, fnott) and OSDs
//! (notifications, launchers, screenshot overlays) bind. Margo's
//! handler does three things on `new_layer_surface` that the
//! tests below pin down:
//!
//! 1. Map the surface into `layer_map_for_output(output)` so the
//!    render path picks it up.
//! 2. Apply layer-rules — regex match against the
//!    client-supplied `namespace`. `noanim:1` skips the open
//!    animation; `animation_type_open` overrides the global
//!    default for that namespace.
//! 3. Queue an entry in `MargoState::layer_animations` keyed on
//!    the wl_surface, gated on
//!    `config.animations && config.layer_animations &&
//!    animation_duration_open > 0`. The render path then drives a
//!    slide / fade transition.
//!
//! Tests use `Fixture::add_output` to give the handler somewhere
//! to map the layer; without an output, `new_layer_surface`
//! silently early-returns and the handler doesn't run.

use margo_config::{Config, LayerRule};
use smithay::desktop::layer_map_for_output;
use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;

use super::fixture::Fixture;

#[test]
fn layer_surface_maps_into_output_layer_map() {
    let mut fx = Fixture::new();
    fx.add_output("HEADLESS-1", (1920, 1080));
    let id = fx.add_client();

    let (_layer_surface, surface) =
        fx.client(id).create_layer_surface("noctalia-bar", Layer::Top);
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);

    let output = fx.server.state.monitors[0].output.clone();
    let count = layer_map_for_output(&output).layers().count();
    assert_eq!(
        count, 1,
        "layer-shell handler should have mapped the surface into the output's layer map",
    );
}

#[test]
fn layer_rule_noanim_suppresses_open_animation() {
    // Namespace-rule rule: `layerrule = noanim:1, namespace:^bar$`.
    // Even with both global animation toggles ON and a non-zero
    // open-duration, the noanim flag must skip queueing the
    // animation — that's what users add to bars to kill the
    // slide-in jitter every reload.
    let config = Config {
        animations: true,
        layer_animations: true,
        animation_duration_open: 200,
        layer_rules: vec![LayerRule {
            no_anim: true,
            layer_name: Some("^bar$".to_string()),
            ..LayerRule::default()
        }],
        ..Config::default()
    };

    let mut fx = Fixture::with_config(config);
    fx.add_output("HEADLESS-1", (1920, 1080));
    let id = fx.add_client();

    let (_layer_surface, surface) =
        fx.client(id).create_layer_surface("bar", Layer::Top);
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);

    assert!(
        fx.server.state.layer_animations.is_empty(),
        "noanim:1 rule must skip queueing into layer_animations",
    );
}

#[test]
fn matching_namespace_with_animations_on_queues_entry() {
    // Mirror of the previous test without the noanim rule: with
    // `config.layer_animations = true` and a non-zero duration,
    // the open animation entry IS queued. Catches "the animation
    // gate flipped condition order" regressions.
    let config = Config {
        animations: true,
        layer_animations: true,
        animation_duration_open: 200,
        ..Config::default()
    };

    let mut fx = Fixture::with_config(config);
    fx.add_output("HEADLESS-1", (1920, 1080));
    let id = fx.add_client();

    let (_layer_surface, surface) =
        fx.client(id).create_layer_surface("noctalia-launcher", Layer::Top);
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);

    assert_eq!(
        fx.server.state.layer_animations.len(),
        1,
        "layer animation entry should be queued (config.layer_animations = true, no rule rejects)",
    );
}

#[test]
fn layer_animations_off_in_config_skips_queueing() {
    // Default Config has `layer_animations = false`. Even though
    // a layer surface gets created and mapped, no entry should
    // land in layer_animations. This pins the condition order so
    // a future "always animate layer surfaces" change doesn't
    // sneak past review.
    let mut fx = Fixture::new();
    fx.add_output("HEADLESS-1", (1920, 1080));
    let id = fx.add_client();

    let (_layer_surface, surface) =
        fx.client(id).create_layer_surface("waybar", Layer::Top);
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);

    assert!(
        fx.server.state.layer_animations.is_empty(),
        "default config has layer_animations = false; no entry should queue",
    );
    // But the layer DID still map — animation toggle is orthogonal.
    let output = fx.server.state.monitors[0].output.clone();
    assert_eq!(
        layer_map_for_output(&output).layers().count(),
        1,
        "animation off doesn't mean don't map the layer",
    );
}

#[test]
fn layer_destroyed_unmaps_from_output() {
    // Sequence: create + commit + map → destroy → roundtrip.
    // Server-side `layer_destroyed` should unmap from
    // layer_map_for_output. Without this, every layer-shell
    // client that died unexpectedly would leave a phantom entry
    // breaking the renderer.
    let mut fx = Fixture::new();
    fx.add_output("HEADLESS-1", (1920, 1080));
    let id = fx.add_client();

    let (layer_surface, surface) =
        fx.client(id).create_layer_surface("rofi", Layer::Top);
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    let output = fx.server.state.monitors[0].output.clone();
    assert_eq!(layer_map_for_output(&output).layers().count(), 1);

    layer_surface.destroy();
    surface.destroy();
    fx.client(id).flush();
    fx.roundtrip(id);

    assert_eq!(
        layer_map_for_output(&output).layers().count(),
        0,
        "destroying the layer surface must clear the output's layer_map slot",
    );
}
