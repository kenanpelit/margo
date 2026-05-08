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
//! preserved: GPL-3.0-or-later. Original niri provenance is annotated
//! at each function boundary.
//!
//! ## Why a separate thread loop
//!
//! pipewire-rs does its own event-loop dispatching off the calling
//! thread, and PipeWire's `pw_thread_loop` is a self-contained event
//! pump. Mixing it into smithay's calloop loop is possible via the
//! `pipewire-extra` integration crate but adds complexity; niri runs
//! a dedicated thread per cast session and we mirror that for now.

#![cfg(any())] // Phase A scaffold — pw_utils + cast lifecycle in Phase C.

pub mod pw_utils;
