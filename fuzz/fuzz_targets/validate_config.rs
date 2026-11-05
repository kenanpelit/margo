//! Fuzz margo-config's validator (`mctl check-config`, the config-error
//! banner): whatever the bytes, it must report (Ok or Err) and never panic,
//! hang or blow the stack. It is a second line splitter next to the parser
//! and has drifted from it before, so it gets its own target.
//!
//! The validator reads from a path, so each input is written to a per-process
//! file under the temp dir (libFuzzer's `-jobs`/`-workers` run separate
//! processes). `source` / `include` lines are blanked first (they read other
//! files from disk): see `margo_fuzz::strip_file_directives`.
#![no_main]

use std::path::PathBuf;
use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;

fn scratch_file() -> &'static PathBuf {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        std::env::temp_dir().join(format!("margo-fuzz-validate-{}.conf", std::process::id()))
    })
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let text = margo_fuzz::strip_file_directives(text);
    let path = scratch_file();
    if std::fs::write(path, text).is_err() {
        return;
    }
    let _ = margo_config::validator::validate_config(Some(path.as_path()));
});
