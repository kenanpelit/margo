//! Theme config: matugen, fonts, sizing, icons, app themes (Helium).

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Theme {
    pub icons: Icons,
    pub theme: Themes,
    pub matugen: Matugen,
    pub apps: ThemeApps,
    pub css_file: String,
    pub attributes: ThemeAttributes,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            icons: Icons::default(),
            theme: Themes::Kenp,
            matugen: Matugen::default(),
            apps: ThemeApps::default(),
            css_file: String::new(),
            attributes: ThemeAttributes::default(),
        }
    }
}

#[derive(
    Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize, Store, Patch, JsonSchema,
)]
#[serde(default)]
pub struct ThemeApps {
    pub helium: HeliumTheme,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, Store, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HeliumTargetMode {
    /// Apply to every real Chromium profile found below `isolated_root`.
    All,
    /// Apply to the `profile.last_used` profile of each isolated Helium root.
    #[default]
    LastUsed,
    /// Apply only to the explicit instance/profile rows enabled below.
    Selected,
}

impl PatchField for HeliumTargetMode {
    fn patch_field(
        &mut self,
        new: Self,
        path: &StorePath,
        notify: &mut dyn FnMut(&StorePath),
        _keys: Option<&KeyMap>,
    ) {
        if *self != new {
            *self = new;
            notify(path);
        }
    }
}

impl HeliumTargetMode {
    pub fn all() -> &'static [Self] {
        &[Self::LastUsed, Self::Selected, Self::All]
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::LastUsed => "Last used profiles",
            Self::Selected => "Selected profiles",
            Self::All => "All profiles",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct HeliumProfileTarget {
    pub instance: String,
    pub profile: String,
    pub enabled: bool,
}

impl Default for HeliumProfileTarget {
    fn default() -> Self {
        Self {
            instance: String::new(),
            profile: String::new(),
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct HeliumTheme {
    /// Follow margo/matugen by writing Helium's Chromium user-colour theme.
    pub enabled: bool,
    /// Root containing Helium isolated browser instances. Each child with a
    /// `Local State` file is treated as one isolated Chromium user-data-dir.
    pub isolated_root: String,
    /// Which discovered profiles receive the generated colour seed.
    pub target_mode: HeliumTargetMode,
    /// Apply automatically after every successful matugen/theme regeneration.
    pub apply_on_theme_change: bool,
    /// Explicit rows used when `target_mode = selected`.
    pub targets: Vec<HeliumProfileTarget>,
}

impl Default for HeliumTheme {
    fn default() -> Self {
        Self {
            enabled: false,
            isolated_root: "~/.helium/isolated".to_string(),
            target_mode: HeliumTargetMode::default(),
            apply_on_theme_change: true,
            targets: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Icons {
    pub shell_icon_theme: String,
    pub app_icon_theme: String,
    pub apply_theme_filter: bool,
    pub filter_strength: ThemeFilterStrength,
    pub monochrome_strength: ThemeFilterStrength,
    pub contrast_strength: ContrastFilterStrength,
}

impl Default for Icons {
    fn default() -> Self {
        Self {
            shell_icon_theme: "MargoMaterial".to_string(),
            app_icon_theme: "MargoMaterial".to_string(),
            apply_theme_filter: false,
            filter_strength: ThemeFilterStrength::new(1.0),
            monochrome_strength: ThemeFilterStrength::new(0.0),
            contrast_strength: ContrastFilterStrength::new(1.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct ThemeAttributes {
    pub font: Font,
    pub sizing: Sizing,
    pub window_opacity: WindowOpacity,
}

impl Default for ThemeAttributes {
    fn default() -> Self {
        Self {
            window_opacity: WindowOpacity::new(1.0),
            font: Font::default(),
            sizing: Sizing::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
#[derive(Default)]
pub struct Font {
    pub primary: String,
    pub secondary: String,
    pub tertiary: String,
    /// Monospace family — drives `--font-family-monospace` (clipboard
    /// previews, the 2FA code chip, other tabular/code-ish bits). Empty
    /// = the CSS `monospace` generic.
    pub monospace: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Sizing {
    pub radius_widget: i32,
    pub radius_window: i32,
    pub border_width: i32,
    /// Hover tint strength (%) shared by every bar pill — the matugen
    /// `--primary` mixed over the bar at this opacity. One value drives
    /// all bar widgets so their hovers read identically (Settings →
    /// Bar). Sane range ~0–40.
    pub bar_hover_strength: i32,
    /// Multiplier applied to every font-size inside the Settings
    /// panel. `1.0` keeps the +1pt-bumped defaults; bigger values
    /// scale further (useful on hi-DPI displays where 15-16 px
    /// reads small), smaller values shrink. Range is enforced
    /// loosely — values outside `0.5..=2.0` will warp the layout.
    pub settings_font_scale: f64,
    /// Global UI font scale — multiplies every `--font-*` token across
    /// the whole shell (bar + menus + dashboard), unlike
    /// `settings_font_scale` which only touches the Settings panel.
    /// `1.0` = unscaled; clamped to `0.5..=2.0`.
    pub font_scale: f64,
    /// Bar-pill font scale — multiplies `--font-bar` (clock / battery /
    /// media / network labels) on top of the global `font_scale`.
    /// `1.0` = unscaled; clamped to `0.5..=2.0`.
    pub bar_font_scale: f64,
    /// Painted shell-surface opacity, as a percentage (`100` = opaque). Lower
    /// values frost the bar + menu/panel backgrounds so the wallpaper shows
    /// through (ashell-style). Clamped to `60..=100` so a surface never
    /// disappears. Drives `--surface-opacity` (the frame-draw fill alpha +
    /// the frameless panel backgrounds).
    pub surface_opacity: i32,
    /// Manual frame FILL colour override as a CSS hex string
    /// (`#rrggbbaa`). Empty = follow the matugen `--surface` role. Set from
    /// Settings → Bar → Frame; drives `--frame-bg` (the painted bar frame).
    pub frame_color: String,
    /// Manual frame BORDER colour override (CSS hex `#rrggbbaa`). Empty =
    /// matugen `--outline`. Drives `--frame-border`.
    pub frame_border_color: String,
    /// Manual bar SEPARATOR colour override (CSS hex `#rrggbbaa`). Empty =
    /// matugen `--outline`. Drives `--bar-separator-color`. Settings → Bar.
    pub separator_color: String,
}

impl Default for Sizing {
    fn default() -> Self {
        Self {
            // Soft, GNOME/ashell-style defaults: 16 px for the widget (bar
            // pill / frame chrome) and 18 px for the window (menu / dialog /
            // settings panel) corner radius — matches the semantic shape
            // scale's gentle rounding. The value Settings → Theme → Sizing
            // "Reset to defaults" restores.
            radius_widget: 16,
            radius_window: 18,
            border_width: 2,
            bar_hover_strength: 14,
            settings_font_scale: 1.0,
            font_scale: 1.0,
            bar_font_scale: 1.0,
            surface_opacity: 100,
            frame_color: String::new(),
            frame_border_color: String::new(),
            separator_color: String::new(),
        }
    }
}

// Manually implemented because the `f64` field rules out a
// derived `Eq` (NaN ≠ NaN). PartialEq still works for the
// reactive store's change detection — float comparison is exact
// here because slider widgets snap to user-typed values.
impl Eq for Sizing {}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Matugen {
    pub preference: MatugenPreference,
    pub scheme_type: MatugenType,
    pub mode: MatugenMode,
    pub contrast: MatugenContrast,
    /// When true, the light/dark polarity is auto-derived from the
    /// wallpaper's average luminance (bright → Light, dark → Dark) on each
    /// wallpaper change, overriding `mode`. Only applies to the
    /// wallpaper-driven (`Themes::Wallpaper`) theme.
    pub auto_polarity: bool,
}

impl Default for Matugen {
    fn default() -> Self {
        Self {
            preference: MatugenPreference::Darkness,
            scheme_type: MatugenType::TonalSpot,
            mode: MatugenMode::Dark,
            contrast: MatugenContrast::new(0.0),
            auto_polarity: false,
        }
    }
}
