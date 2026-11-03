//! Fuzz margo-config's text parser: whatever the bytes, `parse_config_str`
//! must return (Ok or Err) and never panic, hang or blow the stack.
//!
//! `include` / `source` lines are skipped: they read other files from disk
//! (an absolute path like `/dev/zero` would just read forever), which makes
//! runs depend on the machine instead of the input. The include path is
//! covered by the parser's own unit tests.
#![no_main]

use std::path::Path;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if text.contains("include") || text.contains("source") {
        return;
    }
    // A path that never exists: relative includes would resolve under it,
    // and the parser never reads the origin itself for string input.
    let origin = Path::new("/nonexistent/margo-fuzz/config.conf");
    let _ = margo_config::parse_config_str(text, Some(origin));
});
