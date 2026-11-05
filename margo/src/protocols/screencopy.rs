//! `wlr-screencopy-unstable-v1` server implementation.
//!
//! Adapted from niri's `src/protocols/screencopy.rs`, simplified by
//! dropping the cast / portal tracking (margo doesn't yet ship a portal
//! integration). Covers the screenshot / `screen rec` use case: a client
//! asks the compositor for a CaptureOutput on a known [`Output`], the
//! compositor copies the most recent rendered frame into the client's
//! SHM/dmabuf buffer, and sends a `ready` event.
//!
//! Wire-up:
//! 1. `MargoState::screencopy_state` (an instance of
//!    [`ScreencopyManagerState`]) is created on startup.
//! 2. `delegate_screencopy!(MargoState)` connects the dispatch traits.
//! 3. `MargoState::frame()` (our [`ScreencopyHandler`] impl) pushes the
//!    [`Screencopy`] onto the manager queue. The udev backend's render
//!    pass drains pending screencopies after each successful frame and
//!    copies the rendered surface into the client buffer.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::{Buffer, Fourcc};
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::sync::SyncPoint;
use smithay::output::Output;
use smithay::reexports::calloop::{Interest, LoopHandle, Mode, PostAction, generic::Generic};
use smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::{
    zwlr_screencopy_frame_v1, zwlr_screencopy_manager_v1,
};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_shm::Format;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::utils::{Physical, Point, Rectangle, Size, Transform};
use smithay::wayland::{dmabuf, shm};
use tracing::{error, trace};
use zwlr_screencopy_frame_v1::{Flags, ZwlrScreencopyFrameV1};
use zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1;

const VERSION: u32 = 3;

/// Single source of truth for the SHM stride we advertise to clients
/// (`frame.buffer()`) and validate against on `Copy` — keeps the two
/// call sites from drifting apart (see the `Copy` handler's doc note
/// on why the old strict `shm_len` check was dropped).
fn screencopy_shm_buffer_stride(size: Size<i32, Physical>) -> i32 {
    size.w * 4
}

/// Per-manager-binding queue: tracks frames the compositor has handed out
/// but the client hasn't yet committed via `Copy`/`CopyWithDamage`, plus
/// the `Screencopy`s waiting to be served by the next render pass.
pub struct ScreencopyQueue {
    /// Per output: the damage tracker of this queue's captures of it,
    /// and the `(buffer size, scale bits)` it was built for. Per output,
    /// because a client's `copy_with_damage` damage is relative to its
    /// previous capture of the same output, not of whichever output this
    /// queue captured last.
    damage_trackers: HashMap<Output, (OutputDamageTracker, (Size<i32, Physical>, u64))>,
    pending_frames: HashSet<ZwlrScreencopyFrameV1>,
    screencopies: Vec<Screencopy>,
}

impl ScreencopyQueue {
    fn new() -> Self {
        Self {
            damage_trackers: HashMap::new(),
            pending_frames: HashSet::new(),
            screencopies: Vec::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.pending_frames.is_empty() && self.screencopies.is_empty()
    }

    pub fn push(&mut self, screencopy: Screencopy) {
        self.screencopies.push(screencopy);
    }

    /// Pending screencopies, oldest first.
    pub fn screencopies(&self) -> &[Screencopy] {
        &self.screencopies
    }

    /// Damage of `elements` since this queue's previous capture of
    /// `output`, in the capture buffer's physical coordinates. Empty when
    /// nothing changed. The tracker persists across captures (rebuilt
    /// only when the buffer size or scale changes), so a recorder polling
    /// with `copy_with_damage` gets the real changed region instead of a
    /// full frame every time.
    fn damage_since_last_capture<E: smithay::backend::renderer::element::Element>(
        &mut self,
        output: &Output,
        size: Size<i32, Physical>,
        scale: f64,
        elements: &[E],
    ) -> Vec<Rectangle<i32, Physical>> {
        let key = (size, scale.to_bits());
        let (tracker, built_for) =
            self.damage_trackers
                .entry(output.clone())
                .or_insert_with(|| {
                    (
                        OutputDamageTracker::new(size, scale, Transform::Normal),
                        key,
                    )
                });
        if *built_for != key {
            *tracker = OutputDamageTracker::new(size, scale, Transform::Normal);
            *built_for = key;
        }
        match tracker.damage_output(1, elements) {
            Ok((damage, _)) => damage.cloned().unwrap_or_default(),
            Err(_) => vec![Rectangle::from_size(size)],
        }
    }

    /// Drop any pending screencopies for the given output. Called by
    /// `ScreencopyManagerState::remove_output` when an output is unplugged.
    fn remove_output(&mut self, output: &Output) {
        self.screencopies
            .retain(|screencopy| screencopy.output() != output);
        self.damage_trackers.remove(output);
    }

    fn remove_frame(&mut self, frame: &ZwlrScreencopyFrameV1) {
        self.pending_frames.remove(frame);
        self.screencopies.retain(|s| s.frame != *frame);
    }
}

#[derive(Default)]
pub struct ScreencopyManagerState {
    queues: HashMap<ZwlrScreencopyManagerV1, ScreencopyQueue>,
    /// The session is paused (VT switch): nothing renders until the
    /// resume, so no frame can be served.
    session_paused: bool,
}

pub struct ScreencopyManagerGlobalData {
    filter: Box<dyn for<'c> Fn(&'c Client) -> bool + Send + Sync>,
}

impl ScreencopyManagerState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<ZwlrScreencopyManagerV1, ScreencopyManagerGlobalData>,
        D: Dispatch<ZwlrScreencopyManagerV1, ()>,
        D: Dispatch<ZwlrScreencopyFrameV1, ScreencopyFrameState>,
        D: ScreencopyHandler,
        D: 'static,
        F: for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static,
    {
        let global_data = ScreencopyManagerGlobalData {
            filter: Box::new(filter),
        };
        display.create_global::<D, ZwlrScreencopyManagerV1, _>(VERSION, global_data);

        Self {
            queues: HashMap::new(),
            session_paused: false,
        }
    }

    pub fn push(&mut self, manager: &ZwlrScreencopyManagerV1, screencopy: Screencopy) {
        if self.session_paused {
            trace!("screencopy: session paused, failing the frame");
            drop(screencopy); // → `failed`
            return;
        }
        let Some(queue) = self.queues.get_mut(manager) else {
            error!("screencopy queue must not be deleted while frames exist");
            return;
        };
        queue.push(screencopy);
    }

    /// The session paused (`true`) or resumed. Nothing is rendered
    /// while it is paused, so every queued frame is failed now, and new
    /// ones are failed as they come (see [`Self::push`]), instead of
    /// waiting for the switch back.
    pub fn set_session_paused(&mut self, paused: bool) {
        self.session_paused = paused;
        if paused {
            for queue in self.queues.values_mut() {
                queue.screencopies.clear(); // → `failed`
                queue.damage_trackers.clear();
            }
            self.cleanup_queues();
        }
    }

    /// Iterate every queue.
    pub fn queues(&self) -> impl Iterator<Item = &ScreencopyQueue> {
        self.queues.values()
    }

    /// Iterate every queue with mutable access. Used by the backend to
    /// drain pending screencopies and feed each one a freshly-rendered
    /// buffer.
    pub fn with_queues_mut(&mut self, mut f: impl FnMut(&mut ScreencopyQueue)) {
        for queue in self.queues.values_mut() {
            f(queue);
        }
        self.cleanup_queues();
    }

    /// Which capture variants are queued for `output`:
    /// `(with the cursor, without it)`.
    pub fn wanted_for(&self, output: &Output) -> (bool, bool) {
        let (mut cursor, mut plain) = (false, false);
        for queue in self.queues.values() {
            for sc in queue.screencopies.iter().filter(|sc| sc.output() == output) {
                if sc.overlay_cursor() {
                    cursor = true;
                } else {
                    plain = true;
                }
            }
        }
        (cursor, plain)
    }

    /// Take every queued frame for `output` that can be served from the
    /// frame just rendered for it, with its damage since that queue's
    /// previous capture of `output`. Frames for other outputs ahead of
    /// them in a queue don't hold them back (they wait for their own
    /// output's render). `elements_for` gives a frame's scene, already
    /// relocated into its capture region. A frame with a zero-sized
    /// buffer is failed; a `copy_with_damage` frame whose output did not
    /// change stays queued, with the later frames for that output, until
    /// it does.
    pub fn take_ready<E: smithay::backend::renderer::element::Element>(
        &mut self,
        output: &Output,
        scale: f64,
        mut elements_for: impl FnMut(&Screencopy) -> Vec<E>,
    ) -> Vec<(Screencopy, Vec<Rectangle<i32, Physical>>)> {
        let mut ready = Vec::new();
        self.with_queues_mut(|queue| {
            let mut i = 0;
            while let Some(sc) = queue.screencopies.get(i) {
                if sc.output() != output {
                    i += 1;
                    continue;
                }
                let size = sc.buffer_size();
                if size.w <= 0 || size.h <= 0 {
                    drop(queue.screencopies.remove(i)); // → `failed`
                    continue;
                }
                let with_damage = sc.with_damage();
                let elements = elements_for(sc);
                let damage = queue.damage_since_last_capture(output, size, scale, &elements);
                if with_damage && damage.is_empty() {
                    break;
                }
                ready.push((queue.screencopies.remove(i), damage));
            }
        });
        ready
    }

    /// Forget every queued screencopy targeting the given output; each
    /// one is answered `failed` (see `Screencopy`'s `Drop`). Called when
    /// the output is unplugged or goes dark (switched off, DPMS-off) —
    /// the render path no longer serves it.
    pub fn remove_output(&mut self, output: &Output) {
        for queue in self.queues.values_mut() {
            queue.remove_output(output);
        }
        self.cleanup_queues();
    }

    fn cleanup_queues(&mut self) {
        self.queues
            .retain(|manager, queue| manager.is_alive() || !queue.is_empty());
    }
}

impl<D> GlobalDispatch<ZwlrScreencopyManagerV1, ScreencopyManagerGlobalData, D>
    for ScreencopyManagerState
where
    D: GlobalDispatch<ZwlrScreencopyManagerV1, ScreencopyManagerGlobalData>,
    D: Dispatch<ZwlrScreencopyManagerV1, ()>,
    D: Dispatch<ZwlrScreencopyFrameV1, ScreencopyFrameState>,
    D: ScreencopyHandler,
    D: 'static,
{
    fn bind(
        state: &mut D,
        _dh: &DisplayHandle,
        _client: &Client,
        manager: New<ZwlrScreencopyManagerV1>,
        _data: &ScreencopyManagerGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        let manager = data_init.init(manager, ());
        state
            .screencopy_state()
            .queues
            .insert(manager, ScreencopyQueue::new());
    }

    fn can_view(client: Client, global_data: &ScreencopyManagerGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D> Dispatch<ZwlrScreencopyManagerV1, (), D> for ScreencopyManagerState
where
    D: GlobalDispatch<ZwlrScreencopyManagerV1, ScreencopyManagerGlobalData>,
    D: Dispatch<ZwlrScreencopyManagerV1, ()>,
    D: Dispatch<ZwlrScreencopyFrameV1, ScreencopyFrameState>,
    D: ScreencopyHandler,
    D: 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        manager: &ZwlrScreencopyManagerV1,
        request: zwlr_screencopy_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        let (frame, overlay_cursor, buffer_size, region_loc, output) = match request {
            zwlr_screencopy_manager_v1::Request::CaptureOutput {
                frame,
                overlay_cursor,
                output,
            } => {
                let Some(output) = Output::from_resource(&output) else {
                    let frame = data_init.init(frame, ScreencopyFrameState::Failed);
                    frame.failed();
                    return;
                };
                let Some(mode) = output.current_mode() else {
                    let frame = data_init.init(frame, ScreencopyFrameState::Failed);
                    frame.failed();
                    return;
                };
                (
                    frame,
                    overlay_cursor,
                    mode.size,
                    Point::from((0, 0)),
                    output,
                )
            }
            zwlr_screencopy_manager_v1::Request::CaptureOutputRegion {
                frame,
                overlay_cursor,
                x,
                y,
                width,
                height,
                output,
            } => {
                if width <= 0 || height <= 0 {
                    let frame = data_init.init(frame, ScreencopyFrameState::Failed);
                    frame.failed();
                    return;
                }
                let Some(output) = Output::from_resource(&output) else {
                    let frame = data_init.init(frame, ScreencopyFrameState::Failed);
                    frame.failed();
                    return;
                };
                let Some(mode) = output.current_mode() else {
                    let frame = data_init.init(frame, ScreencopyFrameState::Failed);
                    frame.failed();
                    return;
                };

                let output_transform = output.current_transform();
                let output_physical = output_transform.transform_size(mode.size);
                let output_rect = Rectangle::from_size(output_physical);

                let rect = Rectangle::new(Point::from((x, y)), Size::from((width, height)));
                let scale = output.current_scale().fractional_scale();
                let phys_rect = rect.to_physical_precise_round(scale);

                let Some(clamped) = phys_rect.intersection(output_rect) else {
                    let frame = data_init.init(frame, ScreencopyFrameState::Failed);
                    frame.failed();
                    return;
                };
                let untrans = output_transform
                    .invert()
                    .transform_rect_in(clamped, &output_physical);
                (frame, overlay_cursor, untrans.size, clamped.loc, output)
            }
            zwlr_screencopy_manager_v1::Request::Destroy => return,
            _ => unreachable!("zwlr_screencopy_manager_v1 request not in protocol XML"),
        };

        let info = ScreencopyFrameInfo {
            output,
            buffer_size,
            region_loc,
            overlay_cursor: overlay_cursor != 0,
        };
        let frame = data_init.init(
            frame,
            ScreencopyFrameState::Pending {
                manager: manager.clone(),
                info,
                copied: Arc::new(AtomicBool::new(false)),
            },
        );

        // Advertise SHM (Xrgb8888) buffer requirements.
        frame.buffer(
            Format::Xrgb8888,
            buffer_size.w as u32,
            buffer_size.h as u32,
            screencopy_shm_buffer_stride(buffer_size) as u32,
        );
        if frame.version() >= 3 {
            frame.linux_dmabuf(
                Fourcc::Xrgb8888 as u32,
                buffer_size.w as u32,
                buffer_size.h as u32,
            );
            frame.buffer_done();
        }

        let st = state.screencopy_state();
        // The queue is created when the manager binds, so this is normally
        // present — but a destroy/copy race shouldn't crash the compositor.
        let Some(queue) = st.queues.get_mut(manager) else {
            tracing::warn!("screencopy: no queue for manager; dropping frame");
            return;
        };
        queue.pending_frames.insert(frame);
    }

    fn destroyed(
        state: &mut D,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        manager: &ZwlrScreencopyManagerV1,
        _data: &(),
    ) {
        let st = state.screencopy_state();
        if let Some(queue) = st.queues.get_mut(manager) {
            if queue.is_empty() {
                st.queues.remove(manager);
            }
        }
    }
}

/// Compositor-side handler trait.
pub trait ScreencopyHandler {
    /// A client has supplied a buffer for a previously-handed-out frame.
    /// The handler should either fail the frame, copy synchronously, or
    /// push it onto the manager queue for the next render pass.
    fn frame(&mut self, manager: &ZwlrScreencopyManagerV1, screencopy: Screencopy);

    fn screencopy_state(&mut self) -> &mut ScreencopyManagerState;
}

#[macro_export]
macro_rules! delegate_screencopy {
    ($(@<$( $lt:tt $( : $clt:tt $(+ $dlt:tt )* )? ),+>)? $ty: ty) => {
        smithay::reexports::wayland_server::delegate_global_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1: $crate::protocols::screencopy::ScreencopyManagerGlobalData
        ] => $crate::protocols::screencopy::ScreencopyManagerState);

        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1: ()
        ] => $crate::protocols::screencopy::ScreencopyManagerState);

        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1: $crate::protocols::screencopy::ScreencopyFrameState
        ] => $crate::protocols::screencopy::ScreencopyManagerState);
    };
}

#[derive(Clone)]
pub struct ScreencopyFrameInfo {
    output: Output,
    buffer_size: Size<i32, Physical>,
    region_loc: Point<i32, Physical>,
    overlay_cursor: bool,
}

pub enum ScreencopyFrameState {
    Failed,
    Pending {
        manager: ZwlrScreencopyManagerV1,
        info: ScreencopyFrameInfo,
        copied: Arc<AtomicBool>,
    },
}

impl<D> Dispatch<ZwlrScreencopyFrameV1, ScreencopyFrameState, D> for ScreencopyManagerState
where
    D: Dispatch<ZwlrScreencopyFrameV1, ScreencopyFrameState>,
    D: ScreencopyHandler,
    D: 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        frame: &ZwlrScreencopyFrameV1,
        request: zwlr_screencopy_frame_v1::Request,
        data: &ScreencopyFrameState,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        if matches!(request, zwlr_screencopy_frame_v1::Request::Destroy) {
            return;
        }

        let ScreencopyFrameState::Pending {
            manager,
            info,
            copied,
        } = data
        else {
            return;
        };

        if copied.load(Ordering::SeqCst) {
            frame.post_error(
                zwlr_screencopy_frame_v1::Error::AlreadyUsed,
                "copy was already requested",
            );
            return;
        }

        let (buffer, with_damage) = match request {
            zwlr_screencopy_frame_v1::Request::Copy { buffer } => (buffer, false),
            zwlr_screencopy_frame_v1::Request::CopyWithDamage { buffer } => (buffer, true),
            _ => unreachable!("non-copy zwlr_screencopy_frame_v1 request reached copy_buffer"),
        };

        let size = info.buffer_size;

        let buffer = if let Ok(dmabuf) = dmabuf::get_dmabuf(&buffer) {
            if dmabuf.format().code == Fourcc::Xrgb8888
                && dmabuf.width() == size.w as u32
                && dmabuf.height() == size.h as u32
            {
                ScreencopyBuffer::Dmabuf(dmabuf.clone())
            } else {
                frame.post_error(
                    zwlr_screencopy_frame_v1::Error::InvalidBuffer,
                    "invalid dmabuf parameters",
                );
                return;
            }
        } else if shm::with_buffer_contents(&buffer, |_, _, buffer_data| {
            // No `shm_len == stride*height` check here (ported fix,
            // niri #4566): `shm_len` is the backing pool's size, which
            // is legitimately allowed to be larger than this one
            // buffer — a client sub-allocating several buffers from
            // one big pool was getting its perfectly valid buffer
            // rejected. The buffer's own bounds (offset + stride *
            // height within the pool) are what matter for safety, and
            // that's enforced independently by the SHM pool itself at
            // `wl_shm_pool.create_buffer` time, not by this check.
            buffer_data.format == Format::Xrgb8888
                && buffer_data.width == size.w
                && buffer_data.height == size.h
                && buffer_data.stride == screencopy_shm_buffer_stride(size)
        })
        .unwrap_or(false)
        {
            ScreencopyBuffer::Shm(buffer)
        } else {
            frame.post_error(
                zwlr_screencopy_frame_v1::Error::InvalidBuffer,
                "invalid buffer",
            );
            return;
        };

        copied.store(true, Ordering::SeqCst);

        state.frame(
            manager,
            Screencopy {
                buffer,
                frame: frame.clone(),
                info: info.clone(),
                with_damage,
                submitted: false,
            },
        );

        let st = state.screencopy_state();
        if let Some(queue) = st.queues.get_mut(manager) {
            queue.pending_frames.remove(frame);
            if queue.is_empty() && !manager.is_alive() {
                st.queues.remove(manager);
            }
        }
    }

    fn destroyed(
        state: &mut D,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        frame: &ZwlrScreencopyFrameV1,
        data: &ScreencopyFrameState,
    ) {
        let ScreencopyFrameState::Pending { manager, .. } = data else {
            return;
        };
        let st = state.screencopy_state();
        let Some(queue) = st.queues.get_mut(manager) else {
            return;
        };
        queue.remove_frame(frame);
        if queue.is_empty() && !manager.is_alive() {
            st.queues.remove(manager);
        }
    }
}

/// The buffer the client gave us — could be SHM or dmabuf.
#[derive(Clone)]
pub enum ScreencopyBuffer {
    /// Dmabuf target. Stored for the future zero-copy path; the udev
    /// backend currently falls through (`failed`) for this variant —
    /// see the dmabuf TODO in `serve_screencopies`.
    Dmabuf(Dmabuf),
    Shm(WlBuffer),
}

/// A pending screencopy: frame protocol object + target buffer + capture
/// info. Drop without `submit()` sends `failed` to the client.
pub struct Screencopy {
    info: ScreencopyFrameInfo,
    frame: ZwlrScreencopyFrameV1,
    buffer: ScreencopyBuffer,
    with_damage: bool,
    submitted: bool,
}

impl Drop for Screencopy {
    fn drop(&mut self) {
        if !self.submitted {
            self.frame.failed();
        }
    }
}

impl Screencopy {
    pub fn buffer(&self) -> &ScreencopyBuffer {
        &self.buffer
    }
    /// Top-left of the requested region within the source output, in
    /// physical pixels. `(0, 0)` for full-output captures.
    pub fn region_loc(&self) -> Point<i32, Physical> {
        self.info.region_loc
    }
    pub fn buffer_size(&self) -> Size<i32, Physical> {
        self.info.buffer_size
    }
    pub fn output(&self) -> &Output {
        &self.info.output
    }
    /// Whether the client wants the pointer cursor composited into the
    /// captured frame. Cursor-less captures (most screenshot tools default
    /// to false) get a hardware-cursor-only image of the desktop.
    pub fn overlay_cursor(&self) -> bool {
        self.info.overlay_cursor
    }
    pub fn with_damage(&self) -> bool {
        self.with_damage
    }

    pub fn damage(&self, damages: impl Iterator<Item = Rectangle<i32, smithay::utils::Buffer>>) {
        for Rectangle { loc, size } in damages {
            self.frame
                .damage(loc.x as u32, loc.y as u32, size.w as u32, size.h as u32);
        }
    }

    fn submit(mut self, y_invert: bool, timestamp: Duration) {
        self.frame.flags(if y_invert {
            Flags::YInvert
        } else {
            Flags::empty()
        });
        let tv_sec_hi = (timestamp.as_secs() >> 32) as u32;
        let tv_sec_lo = (timestamp.as_secs() & 0xFFFFFFFF) as u32;
        let tv_nsec = timestamp.subsec_nanos();
        self.frame.ready(tv_sec_hi, tv_sec_lo, tv_nsec);
        self.submitted = true;
    }

    /// Submit immediately, no GPU sync wait. Use for SHM/CPU-side copies.
    pub fn submit_now(self, y_invert: bool, timestamp: Duration) {
        self.submit(y_invert, timestamp);
    }

    /// Submit after the given `SyncPoint` is signalled. Use for dmabuf
    /// copies where the GPU may still be writing the destination. Will be
    /// the entry point once the dmabuf code path lands.
    #[allow(dead_code)]
    pub fn submit_after_sync<T>(
        self,
        y_invert: bool,
        sync_point: Option<SyncPoint>,
        event_loop: &LoopHandle<'_, T>,
    ) {
        let timestamp = monotonic_now();
        match sync_point.and_then(|s| s.export()) {
            None => self.submit(y_invert, timestamp),
            Some(sync_fd) => {
                let source = Generic::new(sync_fd, Interest::READ, Mode::OneShot);
                let mut screencopy = Some(self);
                if let Err(e) = event_loop.insert_source(source, move |_, _, _| {
                    screencopy.take().unwrap().submit(y_invert, timestamp);
                    Ok(PostAction::Remove)
                }) {
                    trace!("screencopy: failed to insert sync source: {e:?}");
                }
            }
        }
    }
}

/// Module-private monotonic clock for `submit_after_sync` timestamps.
/// Only needed when that fn is actually wired up (dmabuf path) — until
/// then the caller in udev.rs uses its own monotonic_now() directly.
/// CLOCK_MONOTONIC (not a process-start epoch) so screencopy `ready`
/// timestamps match the clock clients read.
#[allow(dead_code)]
fn monotonic_now() -> Duration {
    crate::utils::get_monotonic_time()
}
