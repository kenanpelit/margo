//! The ThinkPad logo LED tile: the red dot in the lid's logo
//! (`/sys/class/leds/tpacpi::lid_logo_dot`), on or off.
//!
//! Reading the LED needs nothing (the class files are world-readable).
//! Writing it goes straight to sysfs when a udev rule made `brightness`
//! writable for the user, otherwise through the root helper's fixed
//! `led-set` verb (one polkit prompt, `tpacpi::*` LEDs only). The tile
//! only shows on machines that have the LED.

use super::{ControlCenterTilesCommandOutput, ControlCenterTilesModel};
use relm4::ComponentSender;
use std::path::Path;

/// The lid logo LED's class name.
pub(crate) const LOGO_LED: &str = "tpacpi::lid_logo_dot";
/// The LED class directory.
const LEDS: &str = "/sys/class/leds";

/// What the tile shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LedState {
    /// The LED exists on this machine.
    pub(crate) available: bool,
    pub(crate) on: bool,
    /// `max_brightness` (the "on" value written).
    pub(crate) max: u32,
    /// `brightness` is writable without root.
    pub(crate) writable: bool,
    /// A write (or polkit prompt) is in flight.
    pub(crate) busy: bool,
    /// The last toggle failed: shown as the subtitle until the next probe.
    pub(crate) error: Option<String>,
}

impl LedState {
    pub(crate) fn subtitle(&self) -> String {
        if self.busy {
            return "…".to_string();
        }
        if let Some(e) = &self.error {
            return e.clone();
        }
        if self.on { "On" } else { "Off" }.to_string()
    }

    /// The brightness a click writes: off when on, else full.
    pub(crate) fn target(&self) -> u32 {
        if self.on { 0 } else { self.max.max(1) }
    }
}

/// Read `<leds>/<name>` (`leds` = `/sys/class/leds`).
pub(crate) fn probe_in(leds: &Path, name: &str) -> LedState {
    let dir = leds.join(name);
    let read = |f: &str| -> Option<u32> {
        std::fs::read_to_string(dir.join(f))
            .ok()?
            .trim()
            .parse()
            .ok()
    };
    let (Some(brightness), Some(max)) = (read("brightness"), read("max_brightness")) else {
        return LedState::default();
    };
    LedState {
        available: true,
        on: brightness > 0,
        max,
        writable: writable(&dir.join("brightness")),
        busy: false,
        error: None,
    }
}

pub(crate) fn probe() -> LedState {
    probe_in(Path::new(LEDS), LOGO_LED)
}

fn writable(p: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(p.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `c` is a valid NUL-terminated path; access(2) only reads it.
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

/// The pkexec argv writing `value` to `name`.
pub(crate) fn pkexec_argv(name: &str, value: u32) -> Result<Vec<String>, String> {
    let value =
        u8::try_from(value).map_err(|_| format!("{name}: brightness {value} is out of range"))?;
    let req = mshell_privileged::Request::LedSet {
        name: name.to_string(),
        value,
    }
    .validated()
    .map_err(|e| e.message().to_string())?;
    Ok(mshell_privileged::lossy(&req.argv()))
}

async fn write(state: LedState) -> Result<(), String> {
    let value = state.target();
    if state.writable {
        let path = Path::new(LEDS).join(LOGO_LED).join("brightness");
        return std::fs::write(&path, format!("{value}\n"))
            .map_err(|e| format!("Couldn't switch the LED: {e}"));
    }
    let argv = pkexec_argv(LOGO_LED, value)?;
    if !mshell_privileged::installed() {
        return Err(mshell_privileged::MISSING.to_string());
    }
    let out = tokio::process::Command::new("pkexec")
        .args(&argv)
        .output()
        .await
        .map_err(|e| format!("pkexec unavailable: {e}"))?;
    mshell_privileged::outcome(out.status.code(), &String::from_utf8_lossy(&out.stderr))
}

/// Tile click: flip the LED, then re-read it (the hardware decides).
pub(super) fn toggle(
    model: &mut ControlCenterTilesModel,
    sender: &ComponentSender<ControlCenterTilesModel>,
) {
    if model.led.busy || !model.led.available {
        return;
    }
    model.led.busy = true;
    model.led.error = None;
    let state = model.led.clone();
    sender.oneshot_command(async move {
        let res = write(state).await;
        let mut now = probe();
        now.error = res.err();
        ControlCenterTilesCommandOutput::LedRefreshed(now)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probes_the_led_class_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let led = dir.path().join(LOGO_LED);
        std::fs::create_dir_all(&led).expect("mkdir");
        std::fs::write(led.join("brightness"), "0\n").expect("w");
        std::fs::write(led.join("max_brightness"), "255\n").expect("w");
        let s = probe_in(dir.path(), LOGO_LED);
        assert!(s.available);
        assert!(!s.on);
        assert_eq!(s.max, 255);
        assert_eq!(s.target(), 255);
        assert_eq!(s.subtitle(), "Off");
        std::fs::write(led.join("brightness"), "255\n").expect("w");
        let s = probe_in(dir.path(), LOGO_LED);
        assert!(s.on);
        assert_eq!(s.target(), 0);
        assert_eq!(s.subtitle(), "On");
        assert_eq!(probe_in(dir.path(), "tpacpi::nothing"), LedState::default());
    }

    #[test]
    fn subtitle_shows_progress_and_errors() {
        let busy = LedState {
            available: true,
            busy: true,
            ..LedState::default()
        };
        assert_eq!(busy.subtitle(), "…");
        let failed = LedState {
            available: true,
            error: Some("Authorization was cancelled".into()),
            ..LedState::default()
        };
        assert_eq!(failed.subtitle(), "Authorization was cancelled");
    }

    #[test]
    fn the_privileged_write_is_the_fixed_verb() {
        assert_eq!(
            pkexec_argv(LOGO_LED, 255).unwrap(),
            [
                mshell_privileged::HELPER,
                "led-set",
                "tpacpi::lid_logo_dot",
                "255"
            ]
        );
        assert!(pkexec_argv(LOGO_LED, 256).is_err());
        assert!(pkexec_argv("input3::capslock", 1).is_err());
    }
}
