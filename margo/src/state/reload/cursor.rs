//! Applying `cursor_theme` / `cursor_size` (see `crate::cursor::settings`)
//! to everything that shows a cursor, at startup and on every reload.

use super::*;
use crate::cursor::settings::{self, CursorSettings};

/// How the child-env overlay should change for `wanted`: `XCURSOR_SIZE`
/// always, `XCURSOR_THEME` set to the theme or removed when none is named
/// (children then see margo's own environment again).
pub(crate) fn cursor_child_env(wanted: &CursorSettings) -> [(&'static str, Option<String>); 2] {
    [
        ("XCURSOR_THEME", wanted.theme.clone()),
        ("XCURSOR_SIZE", Some(wanted.size.to_string())),
    ]
}

/// The XSETTINGS GTK reads its cursor from on X11 (live: running GTK
/// XWayland apps switch theme without a restart). `None` value: remove.
pub(crate) fn cursor_xsettings(
    wanted: &CursorSettings,
) -> [(
    &'static str,
    Option<smithay::xwayland::xwm::settings::Value>,
); 2] {
    use smithay::xwayland::xwm::settings::Value;
    [
        (
            "Gtk/CursorThemeName",
            wanted.theme.clone().map(Value::String),
        ),
        (
            "Gtk/CursorThemeSize",
            Some(Value::Integer(i32::try_from(wanted.size).unwrap_or(24))),
        ),
    ]
}

impl MargoState {
    /// Resolve the cursor settings from the current config and apply them:
    /// rebuild margo's cursor (new theme, empty sprite caches) when they
    /// changed, keep the children's `XCURSOR_*` in sync, and tell XWayland.
    /// Called at startup (after the XWayland WM starts) and after every
    /// reload; a change after startup is also pushed to the systemd / D-Bus
    /// activation environment, off the event loop.
    pub(crate) fn apply_cursor_settings(&mut self) {
        let wanted = settings::for_config(&self.config);
        let changed = self.cursor_manager.reconfigure(&wanted);
        self.sync_cursor_child_env();
        self.push_cursor_to_xwayland();
        if changed {
            tracing::info!(
                theme = wanted.theme_name(),
                size = wanted.size,
                "cursor theme applied"
            );
            self.request_repaint();
            // `systemctl --user import-environment` blocks; never on the
            // event loop. Never from tests: it would write the test's
            // cursor into the real user session's environment.
            #[cfg(not(test))]
            let spawned = std::thread::Builder::new()
                .name("margo-cursor-env".into())
                .spawn(|| {
                    crate::utils::import_session_environment(&["XCURSOR_THEME", "XCURSOR_SIZE"])
                });
            #[cfg(not(test))]
            if let Err(e) = spawned {
                tracing::warn!("cursor env import: thread spawn failed: {e}");
            }
        }
    }

    /// Write the cursor manager's settings into the child-env overlay.
    pub(crate) fn sync_cursor_child_env(&self) {
        for (key, value) in cursor_child_env(self.cursor_manager.settings()) {
            match value {
                Some(v) => crate::utils::set_child_env(key, v),
                None => crate::utils::remove_child_env(key),
            }
        }
    }

    /// XSETTINGS + the root-window cursor (what an X client without its own
    /// cursor shows). A no-op until the XWayland WM is up.
    pub(crate) fn push_cursor_to_xwayland(&mut self) {
        let Some(xwm) = self.xwm.as_mut() else {
            return;
        };
        let wanted = self.cursor_manager.settings().clone();
        let (set, unset): (Vec<_>, Vec<_>) = cursor_xsettings(&wanted)
            .into_iter()
            .partition(|(_, v)| v.is_some());
        if let Err(e) = xwm.set_xsettings(
            set.into_iter()
                .filter_map(|(k, v)| v.map(|v| (k.to_string(), v))),
        ) {
            tracing::warn!("XWayland cursor XSETTINGS: {e}");
        }
        if let Err(e) = xwm.remove_xsettings(unset.into_iter().map(|(k, _)| k.to_string())) {
            tracing::warn!("XWayland cursor XSETTINGS: {e}");
        }
        let image = self.cursor_manager.default_image();
        let (Ok(w), Ok(h), Ok(hx), Ok(hy)) = (
            u16::try_from(image.width),
            u16::try_from(image.height),
            u16::try_from(image.hotspot_x),
            u16::try_from(image.hotspot_y),
        ) else {
            return;
        };
        let pixels = image.pixels_rgba();
        if pixels.len() < usize::from(w) * usize::from(h) * 4 {
            return;
        }
        if let Err(e) = xwm.set_cursor(&pixels, (w, h).into(), (hx, hy).into()) {
            tracing::warn!("XWayland root cursor: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_env_follows_the_settings() {
        let s = CursorSettings {
            theme: Some("Bibata".into()),
            size: 32,
        };
        assert_eq!(
            cursor_child_env(&s),
            [
                ("XCURSOR_THEME", Some("Bibata".to_string())),
                ("XCURSOR_SIZE", Some("32".to_string())),
            ]
        );
        let none = CursorSettings {
            theme: None,
            size: 24,
        };
        assert_eq!(cursor_child_env(&none)[0], ("XCURSOR_THEME", None));
    }

    #[test]
    fn xsettings_name_and_size() {
        use smithay::xwayland::xwm::settings::Value;
        let s = CursorSettings {
            theme: Some("Bibata".into()),
            size: 32,
        };
        let [(n1, v1), (n2, v2)] = cursor_xsettings(&s);
        assert_eq!(n1, "Gtk/CursorThemeName");
        assert!(matches!(v1, Some(Value::String(t)) if t == "Bibata"));
        assert_eq!(n2, "Gtk/CursorThemeSize");
        assert!(matches!(v2, Some(Value::Integer(32))));
        let none = CursorSettings {
            theme: None,
            size: 24,
        };
        assert!(cursor_xsettings(&none)[0].1.is_none(), "no theme: removed");
    }
}
