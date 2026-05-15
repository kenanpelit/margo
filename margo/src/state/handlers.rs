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

mod alpha_modifier;
mod color_management;
mod compositor;
mod content_type;
mod dmabuf;
mod fifo;
mod gamma_control;
mod idle;
mod image_copy_capture;
mod input_method;
mod kde_decoration;
mod keyboard_shortcuts_inhibit;
mod layer_shell;
mod output_management;
mod pointer_constraints;
mod pointer_gestures;
mod pointer_warp;
mod screencopy;
mod security_context;
mod selection;
mod session_lock;
mod single_pixel_buffer;
mod tablet_manager;
mod x11;
mod xdg_activation;
mod xdg_decoration;
mod xdg_dialog;
mod xdg_foreign;
mod xdg_shell;
mod xdg_system_bell;
mod xdg_toplevel_icon;
mod xdg_toplevel_tag;
mod xwayland_keyboard_grab;
