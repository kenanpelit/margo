use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn main() {
    // Re-run when ANY file under these trees changes. A bare
    // `rerun-if-changed=<dir>` reacts only to entries being added or
    // removed (the directory's own mtime) — NOT to edits of existing
    // nested files. So editing e.g. `scss/04-components/_foo.scss` would
    // otherwise bake a STALE stylesheet into the binary until an unrelated
    // file add forced a rebuild. Emit one directive per file instead.
    rerun_if_changed_recursive(Path::new("scss"));
    rerun_if_changed_recursive(Path::new("css_themes"));

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));

    // Compile main SCSS
    let entry = PathBuf::from("scss/main.scss");
    let css_out = out_dir.join("mshell-style.css");

    let scss = fs::read_to_string(&entry)
        .unwrap_or_else(|e| panic!("Failed to read {}: {e}", entry.display()));
    let mut opts = grass::Options::default();
    opts = opts.load_path("scss");
    let css =
        grass::from_string(scss, &opts).unwrap_or_else(|e| panic!("SCSS compile failed: {e}"));
    // grass prepends `@charset "UTF-8";` whenever the output contains
    // non-ASCII (our box-drawing comments, ×, Φ, …). GTK4's CSS parser
    // doesn't understand @-rules and logs "Unknown @ rule" on every
    // load — strip any leading @charset line (the data is already UTF-8).
    let css: String = css
        .lines()
        .filter(|l| !l.trim_start().starts_with("@charset"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&css_out, css)
        .unwrap_or_else(|e| panic!("Failed to write {}: {e}", css_out.display()));
}

/// Emit a `cargo:rerun-if-changed` for every file under `dir` (and the
/// directories themselves, to catch adds/removes), recursively.
fn rerun_if_changed_recursive(dir: &Path) {
    println!("cargo:rerun-if-changed={}", dir.display());
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rerun_if_changed_recursive(&path);
        } else {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}
