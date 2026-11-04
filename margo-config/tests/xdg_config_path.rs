//! margo must read the `config.conf` mshell's Settings pages write: both
//! resolve it through `margo_fs::xdg`, which honours `$XDG_CONFIG_HOME`.
//! (Own test binary: it sets process env.)

use std::path::PathBuf;

#[test]
fn default_config_path_honours_xdg_config_home() {
    // SAFETY: the only test in this binary; nothing else reads env here.
    unsafe {
        std::env::set_var("HOME", "/home/u");
        std::env::set_var("XDG_CONFIG_HOME", "/home/u/dotfiles/config");
    }
    assert_eq!(
        margo_config::resolve_config_path(None).unwrap(),
        PathBuf::from("/home/u/dotfiles/config/margo/config.conf")
    );
    assert_eq!(
        margo_fs::xdg::margo_config_dir().join("config.conf"),
        margo_config::resolve_config_path(None).unwrap(),
        "mshell (margo_fs) and margo agree"
    );

    // An empty value is unset per the spec.
    unsafe { std::env::set_var("XDG_CONFIG_HOME", "") };
    assert_eq!(
        margo_config::resolve_config_path(None).unwrap(),
        PathBuf::from("/home/u/.config/margo/config.conf")
    );
}
