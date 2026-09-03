// SPDX-License-Identifier: GPL-3.0-or-later

fn main() {
    glib_build_tools::compile_resources(&["src"], "src/mtune.gresource.xml", "mtune.gresource");
    println!("cargo:rerun-if-changed=src/mtune.gresource.xml");
    println!("cargo:rerun-if-changed=src/gtk");
    println!("cargo:rerun-if-changed=src/assets");
}
