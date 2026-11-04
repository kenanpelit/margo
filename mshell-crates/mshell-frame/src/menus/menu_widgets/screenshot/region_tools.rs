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
}

impl RegionTool {
    /// `mscreenshot` subcommand.
    pub(crate) fn mode(self) -> &'static str {
        match self {
            RegionTool::Ocr => "ocr",
            RegionTool::Qr => "qr",
        }
    }

    /// External binaries the pipeline needs besides mscreenshot itself.
    fn requires(self) -> &'static [&'static str] {
        match self {
            RegionTool::Ocr => &["tesseract"],
            RegionTool::Qr => &["zbarimg"],
        }
    }

    /// Tooltip for an enabled button.
    fn summary(self) -> &'static str {
        match self {
            RegionTool::Ocr => "Select an area and copy its text (T)",
            RegionTool::Qr => "Select an area and decode a QR code or barcode (Q)",
        }
    }

    /// Arch packages that provide [`Self::requires`].
    fn packages(self) -> &'static str {
        match self {
            RegionTool::Ocr => "tesseract tesseract-data-eng",
            RegionTool::Qr => "zbar",
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

pub(crate) fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(bin).is_file()))
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

    #[test]
    fn modes_are_mscreenshot_subcommands() {
        assert_eq!(RegionTool::Ocr.mode(), "ocr");
        assert_eq!(RegionTool::Qr.mode(), "qr");
        let (ok, tip) = availability(RegionTool::Qr, |b: &str| b != "zbarimg");
        assert!(!ok);
        assert_eq!(tip, "Needs zbarimg — install: zbar");
    }
}
