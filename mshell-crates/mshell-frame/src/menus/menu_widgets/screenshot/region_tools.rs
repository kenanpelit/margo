//! Region tools in the screenshot menu (OCR, …).
//!
//! Each tool is one pipeline in the `mscreenshot` binary (capture via the
//! in-shell selector → external tool → clipboard + notification), the same
//! one the `screenshot-ocr` compositor action and `mshellctl screenshot
//! ocr` run — so the menu, keybinds and the CLI can't drift apart.

use mshell_process::SpawnReaped;
use std::process::{Command, Stdio};

/// A region tool the menu can launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RegionTool {
    Ocr,
    Qr,
    Palette,
}

impl RegionTool {
    /// `mscreenshot` subcommand.
    pub(crate) fn mode(self) -> &'static str {
        match self {
            RegionTool::Ocr => "ocr",
            RegionTool::Qr => "qr",
            RegionTool::Palette => "palette",
        }
    }

    /// External binaries the pipeline needs besides mscreenshot itself.
    fn requires(self) -> &'static [&'static str] {
        match self {
            RegionTool::Ocr => &["tesseract"],
            RegionTool::Qr => &["zbarimg"],
            RegionTool::Palette => &[],
        }
    }

    /// Tooltip for an enabled button.
    fn summary(self) -> &'static str {
        match self {
            RegionTool::Ocr => "Select an area and copy its text (T)",
            RegionTool::Qr => "Select an area and decode a QR code or barcode (Q)",
            RegionTool::Palette => "Select an area and copy its main colours (P)",
        }
    }

    /// Arch packages that provide [`Self::requires`].
    fn packages(self) -> &'static str {
        match self {
            RegionTool::Ocr => "tesseract tesseract-data-eng",
            RegionTool::Qr => "zbar",
            RegionTool::Palette => "margo (mscreenshot)",
        }
    }
}

/// `(sensitive, tooltip)` for a tool button given a PATH probe: a missing
/// dependency disables the button and the tooltip names the package to
/// install (DESIGN §17 "no service": informational, not an error).
pub(crate) fn availability(tool: RegionTool, on_path: impl Fn(&str) -> bool) -> (bool, String) {
    let missing: Vec<&str> = std::iter::once("mscreenshot")
        .chain(tool.requires().iter().copied())
        .filter(|b| !on_path(b))
        .collect();
    if missing.is_empty() {
        (true, tool.summary().to_string())
    } else {
        (
            false,
            format!(
                "Needs {} — install: {}",
                missing.join(", "),
                tool.packages()
            ),
        )
    }
}

/// `(sensitive, tooltip)` of every region tool. Re-probed each time the
/// menu opens (a few `stat`s), so installing tesseract / zbar later turns
/// the button and its T / Q key on without a shell restart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolStates {
    pub(crate) ocr: (bool, String),
    pub(crate) qr: (bool, String),
    pub(crate) palette: (bool, String),
}

impl ToolStates {
    pub(crate) fn probe(on_path: impl Fn(&str) -> bool) -> Self {
        Self {
            ocr: availability(RegionTool::Ocr, &on_path),
            qr: availability(RegionTool::Qr, &on_path),
            palette: availability(RegionTool::Palette, &on_path),
        }
    }

    pub(crate) fn enabled(&self, tool: RegionTool) -> bool {
        match tool {
            RegionTool::Ocr => self.ocr.0,
            RegionTool::Qr => self.qr.0,
            RegionTool::Palette => self.palette.0,
        }
    }
}

/// The region tool a key press launches (T / Q / P), if that key is a tool
/// key and the tool is available right now.
pub(crate) fn tool_for_key(key: char, tools: &ToolStates) -> Option<RegionTool> {
    let tool = match key.to_ascii_lowercase() {
        't' => RegionTool::Ocr,
        'q' => RegionTool::Qr,
        'p' => RegionTool::Palette,
        _ => return None,
    };
    tools.enabled(tool).then_some(tool)
}

/// Launch the tool detached (reaped). The menu has already closed, so the
/// in-shell area selector `mscreenshot` opens is the next thing on screen.
pub(crate) fn launch(tool: RegionTool) {
    if let Err(e) = Command::new("mscreenshot")
        .arg(tool.mode())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn_reaped()
    {
        tracing::warn!(error = %e, tool = tool.mode(), "screenshot menu: spawn mscreenshot failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ocr_is_enabled_only_with_tesseract_and_mscreenshot() {
        let all = |_: &str| true;
        let (ok, tip) = availability(RegionTool::Ocr, all);
        assert!(ok);
        assert!(tip.contains("copy its text"), "{tip}");

        let no_tess = |b: &str| b != "tesseract";
        let (ok, tip) = availability(RegionTool::Ocr, no_tess);
        assert!(!ok);
        assert_eq!(
            tip,
            "Needs tesseract — install: tesseract tesseract-data-eng"
        );

        let nothing = |_: &str| false;
        let (ok, tip) = availability(RegionTool::Ocr, nothing);
        assert!(!ok);
        assert!(tip.starts_with("Needs mscreenshot, tesseract"), "{tip}");
    }

    /// tesseract installed while the shell runs: the next probe (menu
    /// open) enables the OCR button and its T key. The old code measured
    /// once at build and copied the result into the key handler.
    #[test]
    fn a_tool_installed_later_is_picked_up_by_the_next_probe() {
        let before = ToolStates::probe(|b: &str| b != "tesseract");
        assert!(!before.enabled(RegionTool::Ocr));
        assert_eq!(tool_for_key('t', &before), None);
        assert!(before.ocr.1.starts_with("Needs tesseract"));

        let after = ToolStates::probe(|_: &str| true);
        assert_eq!(tool_for_key('t', &after), Some(RegionTool::Ocr));
        assert_eq!(tool_for_key('T', &after), Some(RegionTool::Ocr));
        assert!(after.ocr.1.contains("copy its text"));
    }

    #[test]
    fn tool_keys_map_to_their_tools_and_others_to_none() {
        let all = ToolStates::probe(|_: &str| true);
        assert_eq!(tool_for_key('q', &all), Some(RegionTool::Qr));
        assert_eq!(tool_for_key('P', &all), Some(RegionTool::Palette));
        assert_eq!(tool_for_key('a', &all), None);
        let none = ToolStates::probe(|_: &str| false);
        assert_eq!(tool_for_key('p', &none), None);
    }

    #[test]
    fn modes_are_mscreenshot_subcommands() {
        assert_eq!(RegionTool::Ocr.mode(), "ocr");
        assert_eq!(RegionTool::Qr.mode(), "qr");
        let (ok, tip) = availability(RegionTool::Qr, |b: &str| b != "zbarimg");
        assert!(!ok);
        assert_eq!(tip, "Needs zbarimg — install: zbar");
        assert_eq!(RegionTool::Palette.mode(), "palette");
        assert!(availability(RegionTool::Palette, |b: &str| b == "mscreenshot").0);
    }
}
