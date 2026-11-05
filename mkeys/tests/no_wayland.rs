//! mkeys started outside a Wayland session (a tty, an early systemd unit)
//! must exit with a clear error instead of panicking.

use std::process::Command;

#[test]
fn without_a_wayland_compositor_it_fails_cleanly() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let out = Command::new(env!("CARGO_BIN_EXE_mkeys"))
        .arg("show")
        .env_clear()
        .env("HOME", tmp.path())
        .env("XDG_RUNTIME_DIR", tmp.path())
        .env("XDG_CONFIG_HOME", tmp.path().join("config"))
        // An absolute path that does not exist: the connection must fail.
        .env("WAYLAND_DISPLAY", tmp.path().join("no-such-wayland-socket"))
        .env("MARGO_SOCKET", tmp.path().join("no-such-margo-socket"))
        .output()?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("panicked"), "mkeys panicked:\n{stderr}");
    assert_eq!(out.status.code(), Some(1), "stderr:\n{stderr}");
    assert!(
        stderr.contains("needs a Wayland session"),
        "no clear error:\n{stderr}"
    );
    Ok(())
}
