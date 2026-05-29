//! GTK renderer for WASM-plugin UI trees (mplugins WASM tier — W2b).
//!
//! Turns a guest's [`UiNode`](mshell_plugin_host::UiNode) flat node list into
//! a live GTK widget tree and drives the event loop: a button click / entry
//! submit calls the guest's `update`, and the panel re-renders. Kept in its
//! own crate, behind the `wasm` feature, so the shell only pulls gtk4 + the
//! wasm host when WASM plugins are actually enabled.

#[cfg(feature = "wasm")]
mod panel;

#[cfg(feature = "wasm")]
pub use panel::{PluginPanel, PluginSettings};

// Re-exported so a host (the shell frame) needs only this crate to build a
// panel — the runtime is created once and passed to [`PluginPanel::new`].
#[cfg(feature = "wasm")]
pub use mshell_plugin_host::PluginRuntime;
