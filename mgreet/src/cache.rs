//! Last-login cache, shared with mlogind's TUI greeter.
//!
//! mlogind caches the last environment + username at `config.cache_path`
//! (`mlogind/src/info_caching.rs`) as two lines — `ENVIRONMENT\nUSERNAME`.
//! mgreet reads the same file (path handed over as `MLOGIND_CACHE_PATH`) to
//! pre-fill the username and pre-select the session, and rewrites it on a
//! successful login, so both greeters remember the same last user.

use std::path::Path;

/// Read `(environment, username)` from the cache. Either may be `None` (missing
/// file, blank field). Never fails.
pub fn read(path: &Path) -> (Option<String>, Option<String>) {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_cache(&text),
        Err(_) => (None, None),
    }
}

/// Parse `(environment, username)` from the two-line cache text. Either may be
/// `None` (blank or absent line). Split from the read so it is testable from a
/// `&str`.
fn parse_cache(text: &str) -> (Option<String>, Option<String>) {
    let mut lines = text.lines();
    let field = |line: Option<&str>| {
        line.map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    (field(lines.next()), field(lines.next()))
}

/// Write `ENVIRONMENT\nUSERNAME\n`, matching mlogind's `info_caching` format so
/// the TUI greeter reads it back identically. Best-effort.
pub fn write(path: &Path, environment: &str, username: &str) {
    let _ = std::fs::write(path, format_cache(environment, username));
}

/// The exact on-disk cache text — `ENVIRONMENT\nUSERNAME\n`. Split from [`write`]
/// so the format is testable without a filesystem.
fn format_cache(environment: &str, username: &str) -> String {
    format!("{environment}\n{username}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_environment_and_username() {
        assert_eq!(
            parse_cache("GNOME\nalice\n"),
            (Some("GNOME".to_string()), Some("alice".to_string()))
        );
    }

    #[test]
    fn blank_fields_become_none() {
        assert_eq!(parse_cache("\n\n"), (None, None));
        assert_eq!(parse_cache(""), (None, None));
        assert_eq!(parse_cache("GNOME\n"), (Some("GNOME".to_string()), None));
    }

    #[test]
    fn fields_are_trimmed() {
        assert_eq!(
            parse_cache("  GNOME  \n  bob \n"),
            (Some("GNOME".to_string()), Some("bob".to_string()))
        );
    }

    #[test]
    fn format_round_trips_through_parse() {
        let text = format_cache("Sway", "carol");
        assert_eq!(text, "Sway\ncarol\n");
        assert_eq!(
            parse_cache(&text),
            (Some("Sway".to_string()), Some("carol".to_string()))
        );
    }
}
