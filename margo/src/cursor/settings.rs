//! Which cursor theme and size margo uses, and where they come from.
//!
//! One answer feeds everything that shows a cursor: margo's own pointer
//! ([`super::CursorManager`]), the `XCURSOR_THEME` / `XCURSOR_SIZE` that
//! spawned children see, the systemd / D-Bus activation environment, and
//! XWayland (XSETTINGS `Gtk/CursorThemeName` / `Gtk/CursorThemeSize` and the
//! root-window cursor). It is resolved at startup and again on every config
//! reload, so a cursor picker that writes `cursor_theme` / `cursor_size` and
//! runs `mctl reload` changes the cursor live.
//!
//! Precedence, per value (first one set wins):
//! 1. the `cursor_theme` / `cursor_size` config keys;
//! 2. an `env = XCURSOR_THEME,…` / `env = XCURSOR_SIZE,…` line in the
//!    config (re-read on reload; margo's own process env can't be);
//! 3. the `XCURSOR_*` margo was launched with (captured before the config's
//!    `env` lines are applied, see [`capture_inherited_env`]);
//! 4. no theme (libXcursor's `default`) and 24 px.

use std::sync::OnceLock;

/// Smallest / largest cursor size margo draws (px, logical).
pub const MIN_SIZE: u32 = 8;
pub const MAX_SIZE: u32 = 256;
/// Size when nothing sets one (the GTK / Adwaita default).
pub const DEFAULT_SIZE: u32 = 24;

/// The effective cursor theme and size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorSettings {
    /// `None`: nothing names a theme; margo loads libXcursor's `default`
    /// and leaves `XCURSOR_THEME` alone for children.
    pub theme: Option<String>,
    /// Nominal size in logical px, clamped to [`MIN_SIZE`]..=[`MAX_SIZE`].
    pub size: u32,
}

impl CursorSettings {
    /// The theme name to load (`default` when none is set).
    pub fn theme_name(&self) -> &str {
        self.theme.as_deref().unwrap_or("default")
    }
}

/// `XCURSOR_THEME` / `XCURSOR_SIZE` from margo's launch environment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InheritedCursorEnv {
    pub theme: Option<String>,
    pub size: Option<String>,
}

static INHERITED: OnceLock<InheritedCursorEnv> = OnceLock::new();

/// Remember the launch environment's `XCURSOR_*`. Call before the config's
/// `env` lines are written into the process env, so a later reload that
/// drops such a line falls back to what margo was started with rather than
/// the stale line. Later calls are no-ops.
pub fn capture_inherited_env() {
    INHERITED.get_or_init(|| InheritedCursorEnv {
        theme: std::env::var("XCURSOR_THEME").ok(),
        size: std::env::var("XCURSOR_SIZE").ok(),
    });
}

/// The captured launch environment (read now if never captured, e.g. in
/// tests).
pub fn inherited_env() -> &'static InheritedCursorEnv {
    capture_inherited_env();
    INHERITED.get_or_init(InheritedCursorEnv::default)
}

fn non_empty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

fn parse_size(s: Option<&str>) -> Option<u32> {
    non_empty(s)?.parse::<u32>().ok().filter(|n| *n > 0)
}

/// Resolve the effective settings (see the module docs for precedence).
/// `envs` are the config's `env = NAME,value` lines in file order; the last
/// one for a name wins, as when they are applied to the process env.
pub fn resolve(
    key_theme: Option<&str>,
    key_size: Option<u32>,
    envs: &[(String, String)],
    inherited: &InheritedCursorEnv,
) -> CursorSettings {
    let env_line = |name: &str| {
        envs.iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    };
    let theme = non_empty(key_theme)
        .or_else(|| non_empty(env_line("XCURSOR_THEME")))
        .or_else(|| non_empty(inherited.theme.as_deref()))
        .map(str::to_owned);
    let size = key_size
        .filter(|n| *n > 0)
        .or_else(|| parse_size(env_line("XCURSOR_SIZE")))
        .or_else(|| parse_size(inherited.size.as_deref()))
        .unwrap_or(DEFAULT_SIZE)
        .clamp(MIN_SIZE, MAX_SIZE);
    CursorSettings { theme, size }
}

/// [`resolve`] for a parsed config and margo's launch environment.
pub fn for_config(config: &margo_config::Config) -> CursorSettings {
    resolve(
        config.cursor_theme.as_deref(),
        config.cursor_size_set.then_some(config.cursor_size),
        &config.envs,
        inherited_env(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn inherited(theme: Option<&str>, size: Option<&str>) -> InheritedCursorEnv {
        InheritedCursorEnv {
            theme: theme.map(str::to_owned),
            size: size.map(str::to_owned),
        }
    }

    #[test]
    fn nothing_set_is_the_default_theme_at_24px() {
        let s = resolve(None, None, &[], &InheritedCursorEnv::default());
        assert_eq!(
            s,
            CursorSettings {
                theme: None,
                size: 24
            }
        );
        assert_eq!(s.theme_name(), "default");
    }

    #[test]
    fn the_config_keys_beat_the_launch_environment() {
        // The owner's session: XCURSOR_THEME=capitaine-cursors inherited
        // from environment.d. A picker writing `cursor_theme` must win, or
        // it could never change margo's cursor.
        let inh = inherited(Some("capitaine-cursors"), Some("24"));
        let s = resolve(Some("Bibata-Modern-Ice"), Some(32), &[], &inh);
        assert_eq!(s.theme.as_deref(), Some("Bibata-Modern-Ice"));
        assert_eq!(s.size, 32);
    }

    #[test]
    fn the_launch_environment_is_used_when_the_keys_are_unset() {
        let inh = inherited(Some("capitaine-cursors"), Some("36"));
        let s = resolve(None, None, &[], &inh);
        assert_eq!(s.theme.as_deref(), Some("capitaine-cursors"));
        assert_eq!(s.size, 36);
    }

    #[test]
    fn a_config_env_line_sits_between_the_key_and_the_launch_env() {
        let inh = inherited(Some("capitaine-cursors"), Some("24"));
        let lines = env(&[("XCURSOR_THEME", "Adwaita"), ("XCURSOR_SIZE", "48")]);
        let s = resolve(None, None, &lines, &inh);
        assert_eq!(s.theme.as_deref(), Some("Adwaita"));
        assert_eq!(s.size, 48);
        // The keys still win over the lines.
        let s = resolve(Some("Bibata"), Some(30), &lines, &inh);
        assert_eq!((s.theme.as_deref(), s.size), (Some("Bibata"), 30));
        // Last line for a name wins, as when applied to the process env.
        let lines = env(&[("XCURSOR_SIZE", "48"), ("XCURSOR_SIZE", "40")]);
        assert_eq!(resolve(None, None, &lines, &inh).size, 40);
    }

    #[test]
    fn empty_and_invalid_values_fall_through() {
        let inh = inherited(Some("capitaine-cursors"), Some("big"));
        let lines = env(&[("XCURSOR_THEME", " "), ("XCURSOR_SIZE", "0")]);
        let s = resolve(Some(""), Some(0), &lines, &inh);
        assert_eq!(s.theme.as_deref(), Some("capitaine-cursors"));
        assert_eq!(s.size, DEFAULT_SIZE);
    }

    #[test]
    fn sizes_are_clamped() {
        let none = InheritedCursorEnv::default();
        assert_eq!(resolve(None, Some(2), &[], &none).size, MIN_SIZE);
        assert_eq!(resolve(None, Some(4096), &[], &none).size, MAX_SIZE);
    }
}
