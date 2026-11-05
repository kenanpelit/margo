//! mpicker started without any display must exit non-zero, not panic.

use std::process::Command;

#[test]
fn without_a_display_it_exits_without_panicking() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("mpicker-no-display-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let out = Command::new(env!("CARGO_BIN_EXE_mpicker"))
        .env_clear()
        .env("HOME", &dir)
        .env("XDG_RUNTIME_DIR", &dir)
        .env("WAYLAND_DISPLAY", dir.join("no-such-wayland-socket"))
        .output();
    let _ = std::fs::remove_dir_all(&dir);
    let out = out?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("panicked"), "mpicker panicked:\n{stderr}");
    assert!(!out.status.success(), "stderr:\n{stderr}");
    Ok(())
}
