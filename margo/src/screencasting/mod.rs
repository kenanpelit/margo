#![allow(dead_code)]
//! PipeWire screencast streams + cast session lifecycle.
//!
//! Companion module to [`crate::dbus::mutter_screen_cast`]. Where the
//! D-Bus side handles the protocol (session/stream object creation,
//! `Start` / `Stop` method dispatch), this module owns the actual
//! pixel pipeline:
//!
//!   1. Per-session PipeWire core + thread loop.
//!   2. Per-stream `pipewire::stream::Stream` with format negotiation
//!      (dmabuf preferred, SHM fallback) — driven by the spa params
//!      enumerated from the source's render output.
//!   3. Frame routing: when margo's render loop produces a frame for
//!      an output (or per-toplevel buffer for a window source), it
//!      copies/imports into the stream's queued buffer and signals
//!      the consumer.
//!
//! Reference port: niri/src/screencasting/{mod,pw_utils}.rs. License
//! preserved: GPL-3.0-or-later.
//!
//! ## Phase status
//!
//! * **Phase C foundation (this commit)** — [`render_helpers`]:
//!   the GLES helpers ported from niri/src/render_helpers/ that
//!   the rest of the cast pipeline calls into
//!   (`render_to_dmabuf`, `render_and_download`, `clear_dmabuf`,
//!   `encompassing_geo`).
//! * **Phase C1 (next)** — `pw_utils` submodule with the
//!   `PipeWire` core, `Cast` struct, format negotiation, stream
//!   lifecycle. Direct port of niri/src/screencasting/pw_utils.rs
//!   (~1500 LOC adapted to margo's render path).
//! * **Phase C2 (after)** — `Screencasting` top-level state
//!   (cast list, dynamic-target tracking) and the `redraw_cast`
//!   entry point the udev backend's repaint loop calls.
//! * **Phase D (final)** — wire `Screencasting` into
//!   `MargoState`, hook the D-Bus `ScreenCastToCompositor`
//!   channel onto a calloop receiver, flip portals.conf to
//!   `gnome` so xdp-gnome routes through the new shim.
//!
//! Built incrementally so each commit lands compile-clean — large
//! ports tend to drift when you stage everything at once.

pub mod pw_utils;
pub mod render_helpers;

use smithay::output::WeakOutput;

/// What a screencast stream is targeting. Direct port of niri's
/// `crate::niri::CastTarget` enum — same shape so the imported
/// pw_utils.rs flow doesn't change. Output uses `WeakOutput` so
/// hot-unplug between session creation and frame production
/// surfaces as `upgrade()` returning None.
#[derive(Clone, PartialEq, Eq)]
pub enum CastTarget {
    /// Dynamic cast before the user has picked a target.
    Nothing,
    Output {
        output: WeakOutput,
        /// Cached output name so we can match against the
        /// session's stashed handle even if the WeakOutput
        /// has gone stale.
        name: String,
    },
    Window {
        id: u64,
    },
}

/// Top-level screencast state. Created lazily when xdp-gnome
/// opens its first ScreenCast session via the Mutter D-Bus shim;
/// holds the PipeWire core + list of active casts.
///
/// Direct port of niri's `Screencasting` (niri/src/screencasting/
/// mod.rs). Margo doesn't use the dynamic-cast / mapped_cast_output
/// fields niri carries for its window-id IPC — we route directly
/// off the foreign-toplevel handle margo's D-Bus shim stashes on
/// the source.
pub struct Screencasting {
    pub casts: Vec<pw_utils::Cast>,

    /// Channel from the PipeWire side back to the compositor:
    /// `StopCast`, `Redraw`, `FatalError`. Receiver lives in
    /// `MargoState`'s event loop and dispatches into the cast
    /// pipeline.
    pub pw_to_compositor: calloop::channel::Sender<pw_utils::PwToNiri>,

    /// Drop PipeWire last (after the casts) to avoid a use-after-
    /// free; the casts hold StreamRc handles tied to the core.
    pub pipewire: Option<pw_utils::PipeWire>,
}

impl Screencasting {
    /// Stand up the screencasting state + register the calloop
    /// receiver that drains `PwToNiri` messages from PipeWire
    /// callbacks back into the compositor event loop. Mirrors
    /// niri's `Screencasting::new`.
    pub fn new(
        event_loop: &calloop::LoopHandle<'static, crate::state::MargoState>,
    ) -> Self {
        let pw_to_compositor = {
            let (tx, rx) = calloop::channel::channel();
            event_loop
                .insert_source(rx, move |event, _, state| match event {
                    calloop::channel::Event::Msg(msg) => state.on_pw_msg(msg),
                    calloop::channel::Event::Closed => (),
                })
                .unwrap();
            tx
        };

        Self {
            casts: Vec::new(),
            pw_to_compositor,
            pipewire: None,
        }
    }
}

/// The render-element type cast streams render.
///
/// Two variants:
///
///   * `Direct` — a `MargoRenderElement` straight from the live
///     output render path. Used for `CastTarget::Output` where the
///     cast buffer is the same size + scale as the output, and
///     element coords map 1:1 onto the cast buffer.
///   * `Relocated` — a `MargoRenderElement` shifted by some offset
///     so it lands at a different origin in the cast buffer. Used
///     for `CastTarget::Window` where the cast buffer is the size
///     of *one window* and we need to re-center the entire output
///     element list so the target window's top-left lands at (0,0)
///     of the cast buffer.
///
/// Margo's full `MargoRenderElement` set (border, shadow, clipped
/// surface, open / close / resize animation, solid block-out) flows
/// through both variants — the cast view matches the live display
/// pixel-for-pixel modulo the relocate offset for window casts.
mod cast_render_element {
    use smithay::backend::renderer::{
        element::{render_elements, utils::RelocateRenderElement},
        gles::GlesRenderer,
    };

    use crate::backend::udev::MargoRenderElement;

    render_elements! {
        pub CastRenderElement<=GlesRenderer>;
        Direct=MargoRenderElement,
        Relocated=RelocateRenderElement<MargoRenderElement>,
    }
}

pub use cast_render_element::CastRenderElement;
