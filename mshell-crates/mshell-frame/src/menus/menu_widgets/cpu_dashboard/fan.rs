//! ThinkPad fan state + manual levels via `/proc/acpi/ibm/fan`
//! (thinkpad_acpi). Reading always works; writing needs the module's
//! `fan_control=1` parameter (off by default — and off on the owner's
//! machine), so the UI shows a hint instead of controls in that case and
//! never fails. When control is on but the file is root-only, a level
//! change goes through one `pkexec` call.

use std::path::Path;

pub(crate) const FAN_PATH: &str = "/proc/acpi/ibm/fan";
pub(crate) const FAN_CONTROL_PARAM: &str = "/sys/module/thinkpad_acpi/parameters/fan_control";

/// Shown when the kernel refuses manual control.
pub(crate) const FAN_CONTROL_HINT: &str = "Manual fan levels are off. Add \
`options thinkpad_acpi fan_control=1` to /etc/modprobe.d/thinkpad_acpi.conf \
and reboot to enable them.";

/// Parsed `/proc/acpi/ibm/fan`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct FanState {
    pub status: String,
    pub rpm: Option<u32>,
    /// `auto`, `disengaged`, `full-speed` or `0`–`7`.
    pub level: String,
    /// The file lists `commands:` lines only when fan_control=1.
    pub accepts_commands: bool,
}

pub(crate) fn parse_fan(s: &str) -> Option<FanState> {
    let mut st = FanState::default();
    let mut seen = false;
    for line in s.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let v = v.trim();
        match k.trim() {
            "status" => {
                st.status = v.to_string();
                seen = true;
            }
            "speed" => st.rpm = v.parse().ok(),
            "level" => st.level = v.to_string(),
            "commands" => st.accepts_commands = true,
            _ => {}
        }
    }
    seen.then_some(st)
}

/// A level the user can pick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FanLevel {
    Auto,
    Level(u8),
    FullSpeed,
}

impl FanLevel {
    /// Choices in display order.
    pub(crate) fn all() -> Vec<FanLevel> {
        let mut v = vec![FanLevel::Auto];
        v.extend((0..=7).map(FanLevel::Level));
        v.push(FanLevel::FullSpeed);
        v
    }

    pub(crate) fn label(self) -> String {
        match self {
            FanLevel::Auto => "Auto".to_string(),
            FanLevel::Level(n) => n.to_string(),
            FanLevel::FullSpeed => "Max".to_string(),
        }
    }

    /// The keyword the kernel takes after `level `.
    pub(crate) fn keyword(self) -> String {
        match self {
            FanLevel::Auto => "auto".to_string(),
            FanLevel::Level(n) => n.min(7).to_string(),
            FanLevel::FullSpeed => "full-speed".to_string(),
        }
    }

    /// The line written to `/proc/acpi/ibm/fan`.
    pub(crate) fn command(self) -> String {
        format!("level {}", self.keyword())
    }

    /// Whether `state_level` (the file's `level:` value) is this choice.
    /// `disengaged` reads as full speed (it's "unregulated, max").
    pub(crate) fn matches(self, state_level: &str) -> bool {
        match self {
            FanLevel::FullSpeed => state_level == "full-speed" || state_level == "disengaged",
            _ => state_level == self.keyword(),
        }
    }
}

/// How (and whether) the UI may change the fan level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FanControl {
    /// Not a ThinkPad / module not loaded: no fan row at all.
    Absent,
    /// Readable, but `fan_control` is off: show the hint.
    Disabled,
    /// fan_control on and the file is writable by us (udev group rule).
    Direct,
    /// fan_control on, file root-only: write through pkexec.
    Pkexec,
}

/// Decide from the module parameter (`Y`/`N`/`1`/`0`, `None` when the
/// module isn't loaded), whether the file exists, whether we can write it,
/// and whether it advertises commands.
pub(crate) fn control_mode(
    param: Option<&str>,
    exists: bool,
    writable: bool,
    accepts_commands: bool,
) -> FanControl {
    if !exists {
        return FanControl::Absent;
    }
    let on = matches!(param.map(str::trim), Some("Y" | "y" | "1")) || accepts_commands;
    if !on {
        FanControl::Disabled
    } else if writable {
        FanControl::Direct
    } else {
        FanControl::Pkexec
    }
}

/// Probe the live system. Blocking but tiny (two small reads + access(2)).
pub(crate) fn probe() -> (Option<FanState>, FanControl) {
    let text = std::fs::read_to_string(FAN_PATH).ok();
    let state = text.as_deref().and_then(parse_fan);
    let param = std::fs::read_to_string(FAN_CONTROL_PARAM).ok();
    let writable = Path::new(FAN_PATH).exists() && {
        let c = std::ffi::CString::new(FAN_PATH).unwrap_or_default();
        // SAFETY: `c` is a valid NUL-terminated path; access(2) reads it only.
        unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
    };
    let mode = control_mode(
        param.as_deref(),
        state.is_some(),
        writable,
        state.as_ref().is_some_and(|s| s.accepts_commands),
    );
    (state, mode)
}

/// pkexec argv writing `cmd` to the fan file. `cmd` comes from
/// `FanLevel::command` (a fixed vocabulary) and is passed as a positional
/// argument, never spliced into the script.
pub(crate) fn pkexec_argv(level: FanLevel) -> Vec<String> {
    vec![
        "/bin/sh".to_string(),
        "-c".to_string(),
        format!(r#"printf '%s\n' "$1" > {FAN_PATH}"#),
        "sh".to_string(),
        level.command(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUTO: &str = include_str!("testdata/ibm_fan_auto.txt");
    /// fan_control=1 layout per Documentation/admin-guide/laptops/thinkpad-acpi.rst.
    const CONTROLLED: &str = "status:\t\tenabled\nspeed:\t\t4100\nlevel:\t\t7\n\
        commands:\tlevel <level> (<level> is 0-7, auto, disengaged, full-speed)\n\
        commands:\tenable, disable\n\
        commands:\twatchdog <timeout> (<timeout> is 0 (off), 1-120 (seconds))\n";

    #[test]
    fn parses_real_fan_file() {
        assert_eq!(
            parse_fan(AUTO),
            Some(FanState {
                status: "enabled".into(),
                rpm: Some(2400),
                level: "auto".into(),
                accepts_commands: false,
            })
        );
        let c = parse_fan(CONTROLLED).expect("parses");
        assert!(c.accepts_commands);
        assert_eq!(c.level, "7");
        assert_eq!(parse_fan(""), None);
    }

    #[test]
    fn control_mode_decisions() {
        // The owner's machine today: fan_control=N, file root-only.
        assert_eq!(
            control_mode(Some("N\n"), true, false, false),
            FanControl::Disabled
        );
        assert_eq!(
            control_mode(Some("Y\n"), true, false, true),
            FanControl::Pkexec
        );
        assert_eq!(
            control_mode(Some("Y"), true, true, true),
            FanControl::Direct
        );
        assert_eq!(control_mode(None, false, false, false), FanControl::Absent);
        // Param unreadable but the file advertises commands → on.
        assert_eq!(control_mode(None, true, true, true), FanControl::Direct);
    }

    #[test]
    fn levels_and_commands() {
        let all = FanLevel::all();
        assert_eq!(all.len(), 10);
        assert_eq!(all[0].command(), "level auto");
        assert_eq!(FanLevel::Level(3).command(), "level 3");
        assert_eq!(FanLevel::Level(9).command(), "level 7", "clamped");
        assert_eq!(FanLevel::FullSpeed.command(), "level full-speed");
        assert!(FanLevel::Auto.matches("auto"));
        assert!(FanLevel::Level(7).matches("7"));
        assert!(FanLevel::FullSpeed.matches("disengaged"));
        assert!(!FanLevel::Level(1).matches("auto"));
        let argv = pkexec_argv(FanLevel::Level(2));
        assert_eq!(argv.last().map(String::as_str), Some("level 2"));
        assert!(
            !argv[2].contains("level 2"),
            "level is positional, not spliced"
        );
    }
}
