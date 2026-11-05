//! Small helpers shared by providers that wrap optional external tools.
//!
//! Optional tools (`fd`, `plocate`, `qalc`, `pass-otp`, a Chromium
//! browser …) are detected at runtime so a missing package degrades to a
//! hint row instead of a broken action.

use std::path::Path;

/// Single-quote `s` for `sh -c`, escaping embedded single quotes.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Replace a leading `$HOME` with `~` for display.
pub fn tildify(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// The user's terminal: `$TERMINAL`, else the first of kitty / alacritty /
/// foot / wezterm on PATH, else kitty (same order as the ssh/tmux
/// providers).
pub fn default_terminal() -> String {
    std::env::var("TERMINAL")
        .ok()
        .filter(|t| !t.trim().is_empty())
        .or_else(|| {
            ["kitty", "alacritty", "foot", "wezterm"]
                .iter()
                .find(|t| margo_fs::path::on_path(t))
                .map(|t| t.to_string())
        })
        .unwrap_or_else(|| "kitty".into())
}

/// argv running `cmd` inside `terminal`. wezterm wants `start --`, the
/// others accept `-e`.
pub fn terminal_argv(terminal: &str, cmd: &[&str]) -> Vec<String> {
    let mut argv = vec![terminal.to_string()];
    if Path::new(terminal).file_name().and_then(|n| n.to_str()) == Some("wezterm") {
        argv.push("start".into());
        argv.push("--".into());
    } else {
        argv.push("-e".into());
    }
    argv.extend(cmd.iter().map(|s| s.to_string()));
    argv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_argv_handles_wezterm() {
        assert_eq!(
            terminal_argv("kitty", &["htop"]),
            vec!["kitty", "-e", "htop"]
        );
        assert_eq!(
            terminal_argv("/usr/bin/wezterm", &["sh", "-c", "x"]),
            vec!["/usr/bin/wezterm", "start", "--", "sh", "-c", "x"]
        );
    }

    #[test]
    fn quoting_and_tilde() {
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        let home = Path::new("/home/u");
        assert_eq!(tildify(Path::new("/home/u/a/b"), home), "~/a/b");
        assert_eq!(tildify(Path::new("/home/u"), home), "~");
        assert_eq!(tildify(Path::new("/etc/x"), home), "/etc/x");
    }
}
