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

pub mod render_helpers;
