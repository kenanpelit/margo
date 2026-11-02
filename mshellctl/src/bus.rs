//! The shell's D-Bus client: a typed [`mshell_ipc::ShellProxy`].
//!
//! Method names and argument types come from the shared proxy trait in
//! `mshell-ipc`, so a server-side rename or signature change fails to compile
//! here instead of silently erroring at runtime. A rejected argument comes
//! back as a D-Bus error; `?` propagates it so `mshellctl` exits non-zero.

pub use mshell_ipc::ShellProxy;

/// Connect to the running shell.
pub async fn shell() -> zbus::Result<ShellProxy<'static>> {
    mshell_ipc::connect().await
}
