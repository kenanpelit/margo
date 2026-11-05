//! Integration tests for `ScreencopyHandler` (W4.2 Phase 4
//! extracted impl at `state/handlers/screencopy.rs`).
//!
//! `zwlr-screencopy-unstable-v1` is what `grim` / `wf-recorder` /
//! OBS bind for full-output / region capture. Margo's handler
//! defers the actual buffer copy to the backend's render path:
//! `frame()` pushes the screencopy onto `state.protocols.screencopy`,
//! which the udev backend drains at the next repaint.
//!
//! Without a render backend the drain side is unobservable, but
//! the bind / push surface IS — that's what these smoke tests
//! pin.

use super::fixture::Fixture;

#[test]
fn screencopy_global_advertised() {
    let mut fx = Fixture::new();
    let id = fx.add_client();
    fx.roundtrip(id);
    let names = fx.client(id).global_names();
    assert!(
        names.iter().any(|n| n == "zwlr_screencopy_manager_v1"),
        "screencopy global must be available; saw {:?}",
        names,
    );
}

#[test]
fn ext_image_copy_capture_globals_advertised() {
    // The newer ext-image-copy-capture stack (Phase 7 work) sits
    // alongside the wlr-screencopy global; both must bind so
    // xdp-wlr 0.8+ can pick the modern path while older clients
    // (grim) still hit the wlr one.
    let mut fx = Fixture::new();
    let id = fx.add_client();
    fx.roundtrip(id);
    let names = fx.client(id).global_names();
    for required in &[
        "ext_output_image_capture_source_manager_v1",
        "ext_foreign_toplevel_image_capture_source_manager_v1",
        "ext_image_copy_capture_manager_v1",
    ] {
        assert!(
            names.iter().any(|n| n == required),
            "{required} must be advertised; saw {:?}",
            names,
        );
    }
}

// ── Capturing an output that is dark ────────────────────────────────────────

use std::sync::{Arc, Mutex};

use smithay::reexports::wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::{Connection, Dispatch, QueueHandle};

use super::client::{ClientId, ClientState};

/// Events one frame received, in order (`"buffer"`, `"ready"`, `"failed"`).
type FrameLog = Arc<Mutex<Vec<&'static str>>>;

impl Dispatch<ZwlrScreencopyManagerV1, ()> for ClientState {
    fn event(
        _: &mut Self,
        _: &ZwlrScreencopyManagerV1,
        _: <ZwlrScreencopyManagerV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, FrameLog> for ClientState {
    fn event(
        _: &mut Self,
        _: &ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        log: &FrameLog,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let name = match event {
            zwlr_screencopy_frame_v1::Event::Buffer { .. } => "buffer",
            zwlr_screencopy_frame_v1::Event::Ready { .. } => "ready",
            zwlr_screencopy_frame_v1::Event::Failed => "failed",
            _ => return,
        };
        log.lock().unwrap().push(name);
    }
}

/// eDP-1 (wl_output #0) and DP-3 (#1), with a client bound to both and
/// to the screencopy manager — what `grim` does.
fn grim_desk() -> (Fixture, ClientId, ZwlrScreencopyManagerV1, Vec<WlOutput>) {
    desk((64, 48), (80, 40))
}

/// eDP-1 and DP-3 with the given mode sizes, bound as in [`grim_desk`].
fn desk(
    edp: (i32, i32),
    dp3: (i32, i32),
) -> (Fixture, ClientId, ZwlrScreencopyManagerV1, Vec<WlOutput>) {
    let mut fx = Fixture::new();
    fx.add_output("eDP-1", edp);
    fx.add_output("DP-3", dp3);
    let id = fx.add_client();
    fx.roundtrip(id);
    let c = fx.client(id);
    let outputs = vec![c.bind_output_nth(0), c.bind_output_nth(1)];
    let manager: ZwlrScreencopyManagerV1 = c.bind_global(3);
    c.flush();
    fx.roundtrip(id);
    (fx, id, manager, outputs)
}

/// `capture_output` + `copy` into a matching SHM buffer; the frame's log.
fn capture(
    fx: &mut Fixture,
    id: ClientId,
    manager: &ZwlrScreencopyManagerV1,
    output: &WlOutput,
    size: (i32, i32),
) -> FrameLog {
    capture_with(fx, id, manager, output, size, false)
}

/// [`capture`], with `copy_with_damage` when `with_damage`.
fn capture_with(
    fx: &mut Fixture,
    id: ClientId,
    manager: &ZwlrScreencopyManagerV1,
    output: &WlOutput,
    size: (i32, i32),
    with_damage: bool,
) -> FrameLog {
    let log = FrameLog::default();
    let c = fx.client(id);
    let frame = manager.capture_output(0, output, &c.qh, log.clone());
    c.flush();
    fx.roundtrip(id);
    let c = fx.client(id);
    let buffer = c.create_shm_buffer(size.0, size.1);
    if with_damage {
        frame.copy_with_damage(&buffer);
    } else {
        frame.copy(&buffer);
    }
    c.flush();
    fx.roundtrip(id);
    log
}

fn queued(fx: &Fixture) -> Vec<String> {
    fx.server
        .state
        .protocols
        .screencopy
        .queues()
        .flat_map(|q| q.screencopies().iter().map(|s| s.output().name()))
        .collect()
}

/// `grim` with DP-3 switched off queues DP-3's frame and eDP-1's on one
/// manager. DP-3 is never rendered, so its frame was never answered and
/// eDP-1's sat behind it (the backend serves a queue from its head): grim
/// — and the portal Screenshot, which runs it — hung.
#[test]
fn capturing_a_switched_off_output_fails_at_once_and_blocks_nothing() {
    let (mut fx, id, manager, outputs) = grim_desk();
    fx.server.state.disable_monitor(1);

    let dark = capture(&mut fx, id, &manager, &outputs[1], (80, 40));
    let lit = capture(&mut fx, id, &manager, &outputs[0], (64, 48));
    assert_eq!(*dark.lock().unwrap(), ["buffer", "failed"]);
    assert_eq!(*lit.lock().unwrap(), ["buffer"], "waits for its render");
    assert_eq!(queued(&fx), ["eDP-1"], "nothing dark ahead of eDP-1");
}

#[test]
fn capturing_a_dpms_off_output_fails_at_once() {
    let (mut fx, id, manager, outputs) = grim_desk();
    fx.server.state.request_dpms(Some(false), Some("DP-3"));
    let dark = capture(&mut fx, id, &manager, &outputs[1], (80, 40));
    assert_eq!(*dark.lock().unwrap(), ["buffer", "failed"]);
    assert!(queued(&fx).is_empty());
}

#[test]
fn a_queued_frame_fails_when_its_output_goes_dark() {
    for dpms in [false, true] {
        let (mut fx, id, manager, outputs) = grim_desk();
        let frame = capture(&mut fx, id, &manager, &outputs[1], (80, 40));
        assert_eq!(queued(&fx), ["DP-3"]);
        if dpms {
            fx.server.state.request_dpms(Some(false), None);
        } else {
            fx.server.state.disable_monitor(1);
        }
        fx.roundtrip(id);
        assert_eq!(*frame.lock().unwrap(), ["buffer", "failed"], "dpms={dpms}");
        assert!(queued(&fx).is_empty(), "dpms={dpms}");
    }
}

#[test]
fn a_mirror_target_can_still_be_captured() {
    // It is out of the layout but lit with the mirror picture.
    let (mut fx, id, manager, outputs) = grim_desk();
    fx.server
        .state
        .start_mirror(Some("eDP-1"), Some("DP-3"))
        .unwrap();
    let frame = capture(&mut fx, id, &manager, &outputs[1], (80, 40));
    assert_eq!(*frame.lock().unwrap(), ["buffer"]);
    assert_eq!(queued(&fx), ["DP-3"]);
}

// ── Serving the queue after a render ────────────────────────────────────────

use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::output::Output;
use smithay::utils::{Physical, Rectangle};

fn output_named(fx: &Fixture, name: &str) -> Output {
    fx.server
        .state
        .monitors
        .iter()
        .find(|m| m.name == name)
        .map(|m| m.output.clone())
        .unwrap()
}

/// Serve `output` as the backend does after rendering it, with
/// `scene` as its picture: the frames taken and their damage.
fn serve(
    fx: &mut Fixture,
    output: &str,
    scene: &[SolidColorRenderElement],
) -> Vec<(String, Vec<Rectangle<i32, Physical>>)> {
    let output = output_named(fx, output);
    fx.server
        .state
        .protocols
        .screencopy
        .take_ready(&output, 1.0, |_| scene.to_vec())
        .into_iter()
        .map(|(sc, damage)| (sc.output().name(), damage))
        .collect()
}

fn solid(id: &Id, commit: CommitCounter) -> SolidColorRenderElement {
    SolidColorRenderElement::new(
        id.clone(),
        Rectangle::new((4, 4).into(), (8, 8).into()),
        commit,
        [1.0, 0.0, 0.0, 1.0],
        Kind::Unspecified,
    )
}

/// `grim` queues DP-3's frame, then eDP-1's, on one manager. Rendering
/// eDP-1 must serve its frame at once: serving only from the queue's
/// head kept it waiting for DP-3's next render, so each lit output
/// delayed the other by a frame.
#[test]
fn a_rendered_output_is_served_even_behind_another_outputs_frame() {
    let (mut fx, id, manager, outputs) = grim_desk();
    capture(&mut fx, id, &manager, &outputs[1], (80, 40));
    capture(&mut fx, id, &manager, &outputs[0], (64, 48));
    assert_eq!(queued(&fx), ["DP-3", "eDP-1"]);

    let served = serve(&mut fx, "eDP-1", &[]);
    assert_eq!(served.len(), 1);
    assert_eq!(served[0].0, "eDP-1");
    assert_eq!(queued(&fx), ["DP-3"], "DP-3 waits for its own render");
    assert_eq!(
        fx.server
            .state
            .protocols
            .screencopy
            .wanted_for(&output_named(&fx, "eDP-1")),
        (false, false)
    );
}

/// The variant scan sees a frame queued behind another output's.
#[test]
fn a_frame_behind_another_outputs_frame_is_wanted() {
    let (mut fx, id, manager, outputs) = grim_desk();
    capture(&mut fx, id, &manager, &outputs[1], (80, 40));
    capture(&mut fx, id, &manager, &outputs[0], (64, 48));
    let edp = output_named(&fx, "eDP-1");
    assert_eq!(
        fx.server.state.protocols.screencopy.wanted_for(&edp),
        (false, true)
    );
}

/// A recorder's `copy_with_damage` frames are damaged against the same
/// output's previous capture. A queue-wide tracker compared eDP-1 with
/// whatever DP-3 last showed: a window spanning both, changed between
/// eDP-1's two captures but already changed in DP-3's, came out
/// undamaged, so the recorder's eDP-1 frame waited (or kept stale
/// pixels).
#[test]
fn copy_with_damage_compares_against_the_same_outputs_last_capture() {
    let (mut fx, id, manager, outputs) = desk((64, 48), (64, 48));
    let window = Id::new();
    let before = CommitCounter::default();
    let mut after = before;
    after.increment();

    capture_with(&mut fx, id, &manager, &outputs[0], (64, 48), true);
    assert_eq!(serve(&mut fx, "eDP-1", &[solid(&window, before)]).len(), 1);
    capture_with(&mut fx, id, &manager, &outputs[1], (64, 48), true);
    assert_eq!(serve(&mut fx, "DP-3", &[solid(&window, after)]).len(), 1);

    capture_with(&mut fx, id, &manager, &outputs[0], (64, 48), true);
    let served = serve(&mut fx, "eDP-1", &[solid(&window, after)]);
    assert_eq!(
        served.len(),
        1,
        "the window changed since eDP-1's last capture"
    );
    assert_eq!(
        served[0].1,
        [Rectangle::new((4, 4).into(), (8, 8).into())],
        "damage is the window's change"
    );

    // Nothing changed on eDP-1 since: the next damage frame waits.
    capture_with(&mut fx, id, &manager, &outputs[0], (64, 48), true);
    assert!(serve(&mut fx, "eDP-1", &[solid(&window, after)]).is_empty());
    assert_eq!(queued(&fx), ["eDP-1"]);
}

/// An output without a mode fails every frame it has queued, not only
/// those at a queue's head.
#[test]
fn failing_an_output_reaches_frames_behind_another_outputs() {
    let (mut fx, id, manager, outputs) = grim_desk();
    capture(&mut fx, id, &manager, &outputs[1], (80, 40));
    let edp_frame = capture(&mut fx, id, &manager, &outputs[0], (64, 48));
    let edp = output_named(&fx, "eDP-1");
    fx.server.state.protocols.screencopy.remove_output(&edp);
    fx.roundtrip(id);
    assert_eq!(*edp_frame.lock().unwrap(), ["buffer", "failed"]);
    assert_eq!(queued(&fx), ["DP-3"]);
}

/// While the session is paused (VT switch) nothing is rendered, so a
/// queued frame would only be answered after the switch back: grim and
/// the portal Screenshot hung for as long as the user was away. Queued
/// frames fail when the session pauses, new ones fail at once, and
/// capturing works again after the resume.
#[test]
fn screencopy_fails_promptly_while_the_session_is_paused() {
    let (mut fx, id, manager, outputs) = grim_desk();
    let queued_before = capture(&mut fx, id, &manager, &outputs[0], (64, 48));
    assert_eq!(queued(&fx), ["eDP-1"]);

    fx.server
        .state
        .protocols
        .screencopy
        .set_session_paused(true);
    fx.roundtrip(id);
    assert_eq!(*queued_before.lock().unwrap(), ["buffer", "failed"]);
    let during = capture(&mut fx, id, &manager, &outputs[1], (80, 40));
    assert_eq!(*during.lock().unwrap(), ["buffer", "failed"]);
    assert!(queued(&fx).is_empty());

    fx.server
        .state
        .protocols
        .screencopy
        .set_session_paused(false);
    let after = capture(&mut fx, id, &manager, &outputs[0], (64, 48));
    assert_eq!(*after.lock().unwrap(), ["buffer"]);
    assert_eq!(queued(&fx), ["eDP-1"]);
}
