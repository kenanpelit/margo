use dirs::home_dir;
use std::path::PathBuf;

pub const DEFAULT_PROFILE_NAME: &str = "default";

pub fn default_config_path() -> PathBuf {
    home_dir().expect("HOME not set").join(format!(
        ".config/margo/mshell/profiles/{}.yaml",
        DEFAULT_PROFILE_NAME
    ))
}

pub fn profiles_dir() -> PathBuf {
    home_dir()
        .expect("HOME not set")
        .join(".config/margo/mshell/profiles")
}

pub fn active_profile_cache_path() -> PathBuf {
    home_dir()
        .expect("HOME not set")
        .join(".cache/mshell/active_profile")
}

pub fn profile_path(name: &str) -> PathBuf {
    profiles_dir().join(format!("{name}.yaml"))
}
