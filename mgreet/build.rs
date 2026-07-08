// Compile the greeter stylesheet at build time (grass), the same way
// mshell-style does, and bake the CSS into the binary via include_str!
// (see src/style.rs). SCSS edits therefore need a rebuild to take effect.
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=src/style.scss");
    let scss = Path::new("src/style.scss");
    let css = grass::from_path(scss, &grass::Options::default())
        .expect("mgreet: src/style.scss must compile");
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR set by cargo");
    std::fs::write(Path::new(&out_dir).join("style.css"), css)
        .expect("mgreet: write compiled style.css");
}
