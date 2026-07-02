//! Per-element visibility toggles, read from `~/.config/margo/mlock.conf`.
//!
//! Hand-parsed key=value (same file + style as [`crate::background`]) so
//! the locker keeps zero config-crate dependencies. Every toggle defaults
//! to `true` — a fresh install shows the full set; Settings → Lock screen
//! writes the `show_*` keys.

#[derive(Clone, Copy, Debug)]
pub struct LockToggles {
    pub avatar: bool,
    pub greeting: bool,
    pub date: bool,
    pub battery: bool,
    pub layout: bool,
    pub notifications: bool,
    pub weather: bool,
    pub media: bool,
}

impl Default for LockToggles {
    fn default() -> Self {
        Self {
            avatar: true,
            greeting: true,
            date: true,
            battery: true,
            layout: true,
            notifications: true,
            weather: true,
            media: true,
        }
    }
}

fn conf_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config"))
        })?;
    Some(base.join("margo").join("mlock.conf"))
}

impl LockToggles {
    /// Read `show_*` keys; any missing key keeps its `true` default.
    pub fn load() -> Self {
        let Some(path) = conf_path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        Self::parse(&text)
    }

    /// Pure parse of the `mlock.conf` text — split out from [`Self::load`]
    /// so the key/truthy handling is unit-testable without the filesystem.
    /// Any missing key keeps its `true` default.
    pub(crate) fn parse(text: &str) -> Self {
        let mut t = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, val)) = line.split_once('=') else {
                continue;
            };
            let on = matches!(val.trim(), "true" | "1" | "yes" | "on");
            match key.trim() {
                "show_avatar" => t.avatar = on,
                "show_greeting" => t.greeting = on,
                "show_date" => t.date = on,
                "show_battery" => t.battery = on,
                "show_layout" => t.layout = on,
                "show_notifications" => t.notifications = on,
                "show_weather" => t.weather = on,
                "show_media" => t.media = on,
                _ => {}
            }
        }
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_keeps_every_toggle_on() {
        // A fresh install (no keys) must show the full set — every field
        // defaults to `true`.
        let t = LockToggles::parse("");
        assert!(t.avatar && t.greeting && t.date && t.battery);
        assert!(t.layout && t.notifications && t.weather && t.media);
    }

    #[test]
    fn known_keys_toggle_the_matching_field_only() {
        let t = LockToggles::parse("show_avatar = false\nshow_weather = false\n");
        assert!(!t.avatar, "show_avatar=false must turn avatar off");
        assert!(!t.weather, "show_weather=false must turn weather off");
        // Untouched keys keep their default.
        assert!(t.greeting && t.battery && t.media);
    }

    #[test]
    fn all_documented_truthy_spellings_parse_as_on() {
        for v in ["true", "1", "yes", "on"] {
            let t = LockToggles::parse(&format!("show_media = {v}"));
            assert!(t.media, "`{v}` must read as on");
        }
        // Anything else is off (the `false` path).
        for v in ["false", "0", "no", "off", "True", "YES", "maybe", ""] {
            let t = LockToggles::parse(&format!("show_media = {v}"));
            assert!(!t.media, "`{v}` must read as off");
        }
    }

    #[test]
    fn comments_blank_lines_and_unknown_keys_are_ignored() {
        // Comments / blanks / bogus keys must not disturb defaults, and a
        // line without `=` is skipped rather than panicking.
        let t = LockToggles::parse("# comment\n\n  \nunknown_key = false\nno equals here\n");
        assert!(t.avatar && t.greeting && t.notifications);
    }

    #[test]
    fn surrounding_whitespace_is_trimmed_on_key_and_value() {
        let t = LockToggles::parse("   show_battery   =   false   ");
        assert!(!t.battery, "whitespace around key/value must be trimmed");
    }
}
