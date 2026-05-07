/// Re-exports IPC types for external crates that depend on `margo-ipc`.
/// The full client implementation lives in `bin/mctl.rs`.
pub use crate::{IpcError, IpcEvent, IpcRequest};
