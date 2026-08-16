fn main() {
    // The wallpaper engine links libmpv's render API directly (see
    // src/mpv/paper/mpv_sys.rs). libmpv ships with mpv, which the mpv
    // companion commands already require at runtime.
    println!("cargo:rustc-link-lib=mpv");
}
