//! Panel theming from margo's matugen palette cache.
//!
//! `~/.cache/margo/mshell-colors.toml` is regenerated on every wallpaper
//! change, so reading it at launch keeps the panel in step with the shell —
//! with no compile-time coupling to mshell. Missing/old file → a calm dark
//! fallback so the panel always renders.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Palette {
    pub bg: String,
    pub surface: String,
    pub surface_hi: String,
    pub text: String,
    pub dim: String,
    pub primary: String,
    pub on_primary: String,
    pub success: String,
    pub danger: String,
    /// Menu surface opacity (DESIGN.md §5 — menus are translucent).
    pub menu_opacity: f64,
}

impl Default for Palette {
    fn default() -> Self {
        // Material-ish dark fallback (close to margo's default scheme).
        Self {
            bg: "#191112".into(),
            surface: "#22191a".into(),
            surface_hi: "#312829".into(),
            text: "#f0dee0".into(),
            dim: "#b3a3a5".into(),
            primary: "#ffb2be".into(),
            on_primary: "#561d2a".into(),
            success: "#a7d18b".into(),
            danger: "#ffb4ab".into(),
            menu_opacity: 0.96,
        }
    }
}

fn cache_path() -> PathBuf {
    let base = std::env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".cache")
        });
    base.join("margo/mshell-colors.toml")
}

pub fn load() -> Palette {
    let mut p = Palette::default();
    let Ok(body) = std::fs::read_to_string(cache_path()) else {
        return p;
    };
    let Ok(v) = body.parse::<toml::Value>() else {
        return p;
    };
    let app = v.get("appearance");
    let get = |tbl: Option<&toml::Value>, key: &str, sub: Option<&str>| -> Option<String> {
        let t = tbl?.get(key)?;
        match sub {
            Some(s) => t.get(s)?.as_str().map(str::to_string),
            None => t.as_str().map(str::to_string),
        }
    };
    if let Some(s) = get(app, "background_color", Some("base")) {
        p.bg = s;
    }
    if let Some(s) = get(app, "background_color", Some("weak")) {
        p.surface = s;
    }
    if let Some(s) = get(app, "background_color", Some("neutral")) {
        p.surface_hi = s;
    }
    if let Some(s) = get(app, "background_color", Some("text")) {
        p.text = s.clone();
        p.dim = s; // dimmed via opacity in CSS
    }
    if let Some(s) = get(app, "primary_color", Some("base")) {
        p.primary = s;
    }
    if let Some(s) = get(app, "primary_color", Some("text")) {
        p.on_primary = s;
    }
    if let Some(s) = get(app, "success_color", None) {
        p.success = s;
    }
    if let Some(s) = get(app, "danger_color", None) {
        p.danger = s;
    }
    if let Some(o) = app
        .and_then(|a| a.get("menu"))
        .and_then(|m| m.get("opacity"))
        .and_then(toml::Value::as_float)
    {
        p.menu_opacity = o.clamp(0.5, 1.0);
    }
    p
}

/// Build the full panel stylesheet from the palette.
pub fn css(p: &Palette) -> String {
    format!(
        r#"
window.mvpn {{ background: transparent; }}
.mvpn-root {{
    background-color: alpha({bg}, {opacity});
    color: {text};
    border-radius: 18px;
    padding: 16px;
    border: 1px solid alpha({surface_hi}, 0.8);
}}
.mvpn-title {{ font-size: 15px; font-weight: 700; }}
.mvpn-dim {{ color: {text}; opacity: 0.6; font-size: 12px; }}
.mvpn-hero {{
    background-color: {surface};
    border-radius: 14px;
    padding: 14px;
}}
.mvpn-hero-on {{ border-left: 3px solid {primary}; }}
.mvpn-relay {{ font-size: 16px; font-weight: 700; }}
.mvpn-badge {{
    background-color: {surface_hi};
    border-radius: 999px;
    padding: 2px 10px;
    font-size: 11px;
    font-weight: 600;
}}
.mvpn-badge.ok {{ background-color: alpha({success}, 0.25); color: {success}; }}
.mvpn-card {{ background-color: {surface}; border-radius: 12px; padding: 10px; }}
button.mvpn-action {{
    border-radius: 10px;
    padding: 8px 12px;
    background-color: {surface_hi};
    color: {text};
    border: none;
}}
button.mvpn-action:hover {{ background-color: alpha({primary}, 0.18); }}
button.mvpn-primary {{ background-color: {primary}; color: {on_primary}; font-weight: 700; }}
button.mvpn-danger {{ background-color: alpha({danger}, 0.22); color: {danger}; }}
/* Segmented mode/action buttons (Mullvad / Blocky / Default, Random / … ). */
button.mvpn-mode {{ padding: 8px 6px; font-weight: 600; }}
button.mvpn-mode.selected {{
    background-color: {primary};
    color: {on_primary};
    font-weight: 700;
}}
button.mvpn-mode.selected:hover {{ background-color: {primary}; }}
.mvpn-chip {{ border-radius: 999px; padding: 6px 12px; }}
row.mvpn-row {{ border-radius: 10px; padding: 2px 4px; }}
row.mvpn-row:hover {{ background-color: alpha({primary}, 0.12); }}
.mvpn-key {{
    background-color: {surface_hi};
    border-radius: 6px;
    padding: 1px 6px;
    font-family: monospace;
    font-size: 11px;
}}
list.mvpn-list {{ background: transparent; }}
.mvpn-ping {{ color: {success}; font-family: monospace; font-size: 12px; }}
entry.mvpn-search {{ border-radius: 10px; }}
"#,
        bg = p.bg,
        text = p.text,
        surface = p.surface,
        surface_hi = p.surface_hi,
        primary = p.primary,
        on_primary = p.on_primary,
        success = p.success,
        danger = p.danger,
        opacity = p.menu_opacity,
    )
}
