//! mshell's own paths. All resolve through `margo_fs::xdg` (the one XDG
//! rule margo and mshell share): profiles live in
//! `<config_home>/margo/mshell/profiles`, next to margo's `config.conf`.

use margo_fs::xdg;
use std::path::PathBuf;

pub const DEFAULT_PROFILE_NAME: &str = "default";

pub fn default_config_path() -> PathBuf {
    profile_path(DEFAULT_PROFILE_NAME)
}

pub fn profiles_dir() -> PathBuf {
    xdg::margo_config_dir().join("mshell").join("profiles")
}

pub fn active_profile_cache_path() -> PathBuf {
    xdg::cache_home().join("mshell").join("active_profile")
}

pub fn profile_path(name: &str) -> PathBuf {
    profiles_dir().join(format!("{name}.yaml"))
}

/// `true` when `name` is safe to use as a single filesystem path
/// component — i.e. it has no separators, no `.`/`..`, and isn't an
/// absolute-path root. `name` reaches [`profile_path`] straight from a
/// free-text Settings dialog (new-profile / snapshot-as-profile) with no
/// other validation, so this is what stands between a stray `..` (typo
/// or otherwise) and a write/delete outside `profiles_dir()`.
pub fn is_safe_profile_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    matches!(
        std::path::Path::new(name)
            .components()
            .collect::<Vec<_>>()
            .as_slice(),
        [std::path::Component::Normal(_)]
    )
}

/// Marker written once the setup wizard has applied, so first-launch
/// auto-open stops nagging. Re-opening from Settings / `mshellctl
/// wizard` always works regardless of this file.
pub fn wizard_sentinel_path() -> PathBuf {
    xdg::margo_config_dir().join(".wizard-done")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_name_rejects_path_traversal() {
        assert!(is_safe_profile_name("Nova"));
        assert!(is_safe_profile_name("work-laptop_2"));
        assert!(!is_safe_profile_name(""));
        assert!(!is_safe_profile_name(".."));
        assert!(!is_safe_profile_name("."));
        assert!(!is_safe_profile_name("../../etc/passwd"));
        assert!(!is_safe_profile_name("a/b"));
        assert!(!is_safe_profile_name("/etc"));
    }
}
