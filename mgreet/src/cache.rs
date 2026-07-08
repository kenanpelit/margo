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
    let Ok(text) = std::fs::read_to_string(path) else {
        return (None, None);
    };
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
    let _ = std::fs::write(path, format!("{environment}\n{username}\n"));
}
