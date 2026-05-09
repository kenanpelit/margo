//! Wayland protocol handler trait impls for [`super::MargoState`].
//!
//! W4.2 from the road map's *catch-and-surpass-niri plan*: split
//! `state.rs` (~7000 LOC, every protocol handler in one translation
//! unit) into per-protocol handler files so individual handler edits
//! don't recompile everything. Each handler impl lives in its own
//! sibling file under `state/handlers/` so editing
//! `XdgShellHandler` doesn't touch the file holding
//! `WlrLayerShellHandler` and incremental rebuilds shrink.
//!
//! All submodules access `MargoState`'s internals through `super::super::*`
//! since child modules can see the parent's private items. The
//! `delegate_*!` macros stay co-located with their impls.

mod color_management;
mod dmabuf;
mod gamma_control;
mod idle;
mod input_method;
mod layer_shell;
mod output_management;
mod pointer_constraints;
mod screencopy;
mod selection;
mod session_lock;
mod xdg_activation;
mod xdg_decoration;
