//! `ext-image-capture-source-v1` + `ext-image-copy-capture-v1`
//! — the per-window/per-output screencast stack that backs the
//! Window / Screen tabs in browser meeting share dialogs.
//!
//! Two protocol layers, four traits, all wired up together:
//!
//! * `ext-image-capture-source-v1` — the source factory. Three
//!   trait impls (`ImageCaptureSourceHandler`,
//!   `OutputCaptureSourceHandler`, `ToplevelCaptureSourceHandler`)
//!   register output / toplevel sources and stash a backref on the
//!   resource's user_data so `frame()` can resolve back to the
//!   right pixel source.
//! * `ext-image-copy-capture-v1` — the buffer pump.
//!   `capture_constraints` reports the size + format the client
//!   should allocate; `frame()` queues a [`PendingImageCopyFrame`]
//!   that the udev backend drains during the next render.
//!
//! Compositor-side state (`output_capture_source_state`,
//!   `toplevel_capture_source_state`, `image_copy_capture_state`)
//! lives on `MargoState`; their only callers were the impl blocks
//! below, so the extraction is mechanical.

use smithay::wayland::image_capture_source::{
    ImageCaptureSource, ImageCaptureSourceHandler, OutputCaptureSourceHandler,
    OutputCaptureSourceState, ToplevelCaptureSourceHandler, ToplevelCaptureSourceState,
};
use smithay::wayland::image_copy_capture::{
    BufferConstraints, CursorSession, Frame, ImageCopyCaptureHandler, ImageCopyCaptureState,
    Session, SessionRef,
};

use crate::state::MargoState;

impl ImageCaptureSourceHandler for MargoState {
    fn source_destroyed(&mut self, source: ImageCaptureSource) {
        // Nothing to clean up on the compositor side yet — sources
        // are stateless until rendering wires up a per-source
        // tracker.
        let _ = source;
    }
}

impl OutputCaptureSourceHandler for MargoState {
    fn output_capture_source_state(&mut self) -> &mut OutputCaptureSourceState {
        &mut self.output_capture_source_state
    }

    fn output_source_created(
        &mut self,
        source: ImageCaptureSource,
        output: &smithay::output::Output,
    ) {
        // Stash the output's downgrade handle on the source so
        // Step 2's frame() can find which output the client wants.
        source.user_data().insert_if_missing(|| output.downgrade());
    }
}

impl ToplevelCaptureSourceHandler for MargoState {
    fn toplevel_capture_source_state(&mut self) -> &mut ToplevelCaptureSourceState {
        &mut self.toplevel_capture_source_state
    }

    fn toplevel_source_created(
        &mut self,
        source: ImageCaptureSource,
        toplevel: smithay::wayland::foreign_toplevel_list::ForeignToplevelHandle,
    ) {
        // Stash the toplevel handle so Step 2's frame() can map
        // the source back to a `MargoClient` index. Cloning a
        // ForeignToplevelHandle is cheap (Arc-backed).
        source.user_data().insert_if_missing(|| toplevel);
    }
}

impl ImageCopyCaptureHandler for MargoState {
    fn image_copy_capture_state(&mut self) -> &mut ImageCopyCaptureState {
        &mut self.image_copy_capture_state
    }

    fn capture_constraints(&mut self, source: &ImageCaptureSource) -> Option<BufferConstraints> {
        // Two source kinds: output (display capture) and toplevel
        // (per-window capture). The user_data carries a different
        // handle for each — we picked the one that matches.

        // Output source (Screen tab in meeting clients)
        if let Some(weak_output) = source.user_data().get::<smithay::output::WeakOutput>() {
            let output = weak_output.upgrade()?;
            let mode = output.current_mode()?;
            let size = smithay::utils::Size::<i32, smithay::utils::Buffer>::from((
                mode.size.w,
                mode.size.h,
            ));
            return Some(BufferConstraints {
                size,
                shm: vec![
                    smithay::reexports::wayland_server::protocol::wl_shm::Format::Argb8888,
                    smithay::reexports::wayland_server::protocol::wl_shm::Format::Xrgb8888,
                ],
                dma: None,
            });
        }

        // Toplevel source (Window tab) — find the MargoClient
        // backing this ForeignToplevelHandle and report its
        // current geometry. Bbox-with-popups would clip popups;
        // the live capture only catches the toplevel proper.
        let handle = source
            .user_data()
            .get::<smithay::wayland::foreign_toplevel_list::ForeignToplevelHandle>()?;
        let client = self.clients.iter().find(|c| {
            c.foreign_toplevel_handle
                .as_ref()
                .is_some_and(|h| h.matches(handle))
        })?;

        // Window size — geometry().size is the surface-side
        // logical size; for screencast we want pixels in the
        // buffer-coord domain. They're numerically the same when
        // scale=1 and identical for arrange-tracked geometry.
        let size = smithay::utils::Size::<i32, smithay::utils::Buffer>::from((
            client.geom.width.max(1),
            client.geom.height.max(1),
        ));
        Some(BufferConstraints {
            size,
            shm: vec![
                smithay::reexports::wayland_server::protocol::wl_shm::Format::Argb8888,
                smithay::reexports::wayland_server::protocol::wl_shm::Format::Xrgb8888,
            ],
            dma: None,
        })
    }

    fn new_session(&mut self, session: Session) {
        // Hold the session so it doesn't drop (drop sends
        // `stopped` to the client). Sessions live until the
        // client tears them down or the source becomes invalid.
        self.image_copy_capture_sessions.push(session);
    }

    fn new_cursor_session(&mut self, session: CursorSession) {
        // Cursor capture is a separate sub-protocol; we don't
        // route the cursor through ext-image-copy-capture yet
        // (margo's cursor lives on a hardware plane when
        // possible). Drop = `stopped` to the client.
        let _ = session;
    }

    fn frame(&mut self, session: &SessionRef, frame: Frame) {
        // Route the frame to either an output or a toplevel
        // render path based on which user_data the source
        // carries.
        let source = session.source();

        // Output source — match by name so the udev side can
        // resolve back to an OutputDevice.
        if let Some(weak_output) = source.user_data().get::<smithay::output::WeakOutput>() {
            if let Some(output) = weak_output.upgrade() {
                self.pending_image_copy_frames
                    .push(crate::PendingImageCopyFrame {
                        source: crate::PendingImageCopySource::Output(output.name()),
                        frame: Some(frame),
                    });
                self.request_repaint();
                return;
            }
        }

        // Toplevel source — clone the matching client's Window
        // (Arc-backed, cheap) so the udev side can render it
        // directly. Index into self.clients can shift between
        // request and drain, so don't store an index.
        if let Some(handle) = source
            .user_data()
            .get::<smithay::wayland::foreign_toplevel_list::ForeignToplevelHandle>()
        {
            if let Some(client) = self.clients.iter().find(|c| {
                c.foreign_toplevel_handle
                    .as_ref()
                    .is_some_and(|h| h.matches(handle))
            }) {
                let window = client.window.clone();
                self.pending_image_copy_frames
                    .push(crate::PendingImageCopyFrame {
                        source: crate::PendingImageCopySource::Toplevel(window),
                        frame: Some(frame),
                    });
                self.request_repaint();
                return;
            }
            // Toplevel went away (closed) between session
            // creation and frame request.
            frame.fail(smithay::wayland::image_copy_capture::CaptureFailureReason::Stopped);
            return;
        }

        // Source carries neither user_data tag — shouldn't
        // happen unless someone wires a custom source we don't
        // recognise.
        frame.fail(smithay::wayland::image_copy_capture::CaptureFailureReason::Unknown);
    }
}

smithay::delegate_image_capture_source!(MargoState);
smithay::delegate_output_capture_source!(MargoState);
smithay::delegate_toplevel_capture_source!(MargoState);
smithay::delegate_image_copy_capture!(MargoState);
