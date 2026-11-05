//! Fuzz margo-config's text parser: whatever the bytes, `parse_config_str`
//! must return (Ok or Err) and never panic, hang or blow the stack.
//!
//! `source` / `include` lines are blanked first (they read other files from
//! disk): see `margo_fuzz::strip_file_directives`.
#![no_main]

use std::path::Path;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let text = margo_fuzz::strip_file_directives(text);
    // A path that never exists: relative includes would resolve under it,
    // and the parser never reads the origin itself for string input.
    let origin = Path::new("/nonexistent/margo-fuzz/config.conf");
    let _ = margo_config::parse_config_str(&text, Some(origin));
});
