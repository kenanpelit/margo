//! mplugins WASM host — the sandboxed in-shell plugin tier.
//!
//! See `docs/mplugins-wasm-design.md` for the full architecture. This is
//! **W1 — runtime foundation**: load a WebAssembly **component**, link a host
//! capability (`log`), and invoke a guest export (`run`). The UI tree, more
//! capabilities, and the event loop land in later milestones.
//!
//! The whole runtime is behind the `wasm` feature so default mshell builds
//! don't pull wasmtime. With the feature off this crate exposes nothing —
//! only the std-only path sandbox below is compiled (for its tests).

// The path sandbox is wasmtime-free and compiled unconditionally so its
// security-boundary tests run on every `cargo test --workspace` (the `wasm`
// feature only gates the heavy runtime).
mod sandbox;

#[cfg(feature = "wasm")]
mod runtime;

#[cfg(feature = "wasm")]
pub use runtime::{
    MediaInfo, MediaInfoSource, PluginCapabilities, PluginInstance, PluginRuntime, SystemInfo,
    SystemInfoSource, UiEvent, UiEventKind, UiKind, UiNode,
};
