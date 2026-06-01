fn main() {
    // The wallpaper engine links libmpv's render API directly (see
    // src/paper/mpv_sys.rs). libmpv ships with mpv, which is already a
    // runtime requirement of every mplay command.
    println!("cargo:rustc-link-lib=mpv");
}
