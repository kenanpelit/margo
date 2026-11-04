//! XDG base directories, resolved one way for margo and mshell.
//!
//! The spec rule: a `$XDG_*` value is used only when it is set, non-empty
//! and absolute; otherwise the `$HOME`-relative default applies. Before this
//! module ~40 hand-rolled lookups disagreed (some took an empty or relative
//! value, some ignored XDG entirely, some fell back to a path relative to
//! the cwd), so Settings could edit a `config.conf` margo never read.
//!
//! Every function returns an absolute path. When `$HOME` is unset too (not a
//! real session), the fallback is the system temp dir rather than a path
//! relative to whatever the cwd happens to be.

use std::ffi::OsString;
use std::path::PathBuf;

/// Resolve one base dir from explicit values (the testable core).
/// `var` is the `$XDG_*` value, `home` is `$HOME`, `default` the
/// home-relative fallback (e.g. `.config`).
pub fn resolve(var: Option<OsString>, home: Option<OsString>, default: &str) -> Option<PathBuf> {
    if let Some(v) = var.map(PathBuf::from)
        && v.is_absolute()
    {
        return Some(v);
    }
    let home = home.map(PathBuf::from).filter(|h| h.is_absolute())?;
    Some(home.join(default))
}

fn base(var: &str, default: &str) -> PathBuf {
    resolve(std::env::var_os(var), std::env::var_os("HOME"), default)
        .unwrap_or_else(std::env::temp_dir)
}

/// `$HOME`, when set and absolute.
pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|h| h.is_absolute())
}

/// `$XDG_CONFIG_HOME`, default `~/.config`.
pub fn config_home() -> PathBuf {
    base("XDG_CONFIG_HOME", ".config")
}

/// `$XDG_DATA_HOME`, default `~/.local/share`.
pub fn data_home() -> PathBuf {
    base("XDG_DATA_HOME", ".local/share")
}

/// `$XDG_STATE_HOME`, default `~/.local/state`.
pub fn state_home() -> PathBuf {
    base("XDG_STATE_HOME", ".local/state")
}

/// `$XDG_CACHE_HOME`, default `~/.cache`.
pub fn cache_home() -> PathBuf {
    base("XDG_CACHE_HOME", ".cache")
}

/// `$XDG_RUNTIME_DIR` when set and absolute. No home fallback: the spec
/// gives none, and callers differ (temp dir, skip, warn).
pub fn runtime_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

/// `$XDG_RUNTIME_DIR`, or the system temp dir when it is unset.
pub fn runtime_dir_or_tmp() -> PathBuf {
    runtime_dir().unwrap_or_else(std::env::temp_dir)
}

/// margo's config dir: `<config_home>/margo` (`config.conf`, the mshell
/// profiles, tool configs).
pub fn margo_config_dir() -> PathBuf {
    config_home().join("margo")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(var: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
        resolve(var.map(Into::into), home.map(Into::into), ".config")
    }

    #[test]
    fn absolute_xdg_value_wins() {
        assert_eq!(
            r(Some("/x/cfg"), Some("/home/u")),
            Some(PathBuf::from("/x/cfg"))
        );
    }

    #[test]
    fn empty_or_relative_xdg_value_is_ignored() {
        let want = Some(PathBuf::from("/home/u/.config"));
        assert_eq!(r(Some(""), Some("/home/u")), want);
        assert_eq!(r(Some("rel/cfg"), Some("/home/u")), want);
        assert_eq!(r(None, Some("/home/u")), want);
    }

    #[test]
    fn no_home_gives_none_not_a_relative_path() {
        assert_eq!(r(None, None), None);
        assert_eq!(r(None, Some("")), None);
        assert_eq!(r(Some(""), Some("relative")), None);
    }
}
