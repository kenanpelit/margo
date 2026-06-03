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
        let mut t = Self::default();
        let Some(path) = conf_path() else {
            return t;
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return t;
        };
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
