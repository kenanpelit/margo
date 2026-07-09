use std::fs;
use std::path::PathBuf;

use mshell_image::lut::{CLUT_THEMES, generate_clut};

// Materialise every theme's CLUT to `mshell-image/cluts/`. These files are not
// shipped or committed — the shell generates and caches CLUTs lazily at
// runtime (see `mshell_image::lut::load_clut`). This binary stays as a way to
// regenerate or inspect the full set, or to pre-seed a cache directory.
fn main() {
    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("mshell-clut-gen must live in the workspace")
        .join("mshell-image")
        .join("cluts");

    fs::create_dir_all(&out_dir).expect("failed to create cluts directory");

    let total = CLUT_THEMES.len();
    for (i, (name, theme)) in CLUT_THEMES.iter().enumerate() {
        let clut = generate_clut(theme).unwrap_or_else(|| panic!("no CLUT palette for {name}"));

        let path = out_dir.join(format!("{name}.bin"));
        fs::write(&path, &clut).expect("failed to write CLUT file");

        println!(
            "[{}/{}] Generated {} ({} bytes)",
            i + 1,
            total,
            name,
            clut.len()
        );
    }

    println!("\nDone! Generated {total} CLUTs in {}", out_dir.display());
}
