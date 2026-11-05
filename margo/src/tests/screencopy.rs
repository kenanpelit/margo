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
    let mut fx = Fixture::new();
    fx.add_output("eDP-1", (64, 48));
    fx.add_output("DP-3", (80, 40));
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
    let log = FrameLog::default();
    let c = fx.client(id);
    let frame = manager.capture_output(0, output, &c.qh, log.clone());
    c.flush();
    fx.roundtrip(id);
    let c = fx.client(id);
    let buffer = c.create_shm_buffer(size.0, size.1);
    frame.copy(&buffer);
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
