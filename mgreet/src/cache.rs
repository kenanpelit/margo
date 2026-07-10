//! Last-login cache, shared with mlogind's TUI greeter.
//!
//! mlogind caches the last environment + username at `config.cache_path`
//! (`mlogind/src/info_caching.rs`) as two lines — `ENVIRONMENT\nUSERNAME`.
//! mgreet reads the same file (path handed over as `MLOGIND_CACHE_PATH`) to
//! pre-fill the username and pre-select the session. It does NOT write it: the
//! session runner does, on a login that actually succeeded. A greeter has no
//! business writing `/var/cache` as root, and once it drops its privileges it
//! will not be able to.

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
    fn the_runners_own_format_parses_here() {
        // mlogind's `info_caching::set_cache` writes exactly this. Pinned so a
        // change on that side cannot silently stop pre-filling the form.
        assert_eq!(
            parse_cache("Sway\ncarol\n"),
            (Some("Sway".to_string()), Some("carol".to_string()))
        );
    }
}
