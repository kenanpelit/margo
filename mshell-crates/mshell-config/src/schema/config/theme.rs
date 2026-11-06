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
    /// Saved theme bundles (Settings → Theme → Bundles): a colour scheme,
    /// matugen settings and a wallpaper set applied together in one click.
    pub bundles: Vec<ThemeBundle>,
    /// Name of the bundle applied last, or empty. While it names a bundle
    /// that has wallpapers on disk, wallpaper next/previous/random and the
    /// automatic rotation step through that bundle's wallpapers instead of
    /// the whole wallpaper folder.
    pub active_bundle: String,
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
            bundles: Vec::new(),
            active_bundle: String::new(),
        }
    }
}

/// A named look: colour scheme + matugen settings + wallpapers + a preview.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct ThemeBundle {
    /// Shown on the card; unique among bundles (the apply key).
    pub name: String,
    /// The colour scheme the bundle switches to.
    pub theme: Themes,
    /// Matugen scheme type (used by the wallpaper-driven scheme).
    pub scheme_type: MatugenType,
    /// Light or dark (used by the wallpaper-driven scheme).
    pub mode: MatugenMode,
    /// Wallpaper image paths; the first one that exists is set on apply.
    pub wallpapers: Vec<String>,
    /// Preview image path (a desktop capture or any picture); empty = the
    /// first wallpaper.
    pub preview: String,
}

impl Default for ThemeBundle {
    fn default() -> Self {
        Self {
            name: String::new(),
            theme: Themes::Wallpaper,
            scheme_type: MatugenType::TonalSpot,
            mode: MatugenMode::Dark,
            wallpapers: Vec::new(),
            preview: String::new(),
        }
    }
}

impl ThemeBundle {
    /// A bundle of the look in use now: `theme`'s scheme and matugen
    /// settings, with `wallpaper` (if any) as its only wallpaper.
    pub fn from_current(name: &str, theme: &Theme, wallpaper: Option<&str>) -> Self {
        Self {
            name: name.to_string(),
            theme: theme.theme,
            scheme_type: theme.matugen.scheme_type,
            mode: theme.matugen.mode,
            wallpapers: wallpaper.map(str::to_string).into_iter().collect(),
            preview: String::new(),
        }
    }

    /// Write the bundle's scheme and matugen settings into `theme` and mark
    /// it active. The wallpaper is the caller's job (it is file IO).
    pub fn apply_to(&self, theme: &mut Theme) {
        theme.theme = self.theme;
        theme.matugen.scheme_type = self.scheme_type;
        theme.matugen.mode = self.mode;
        theme.active_bundle = self.name.clone();
    }

    /// File-name-safe slug of the name (`Rose Pine!` → `rose-pine`), for
    /// the captured preview's file name.
    pub fn slug(&self) -> String {
        let mut out = String::new();
        for c in self.name.chars() {
            if c.is_alphanumeric() {
                out.extend(c.to_lowercase());
            } else if !out.ends_with('-') && !out.is_empty() {
                out.push('-');
            }
        }
        let out = out.trim_end_matches('-').to_string();
        if out.is_empty() {
            "bundle".to_string()
        } else {
            out
        }
    }
}

impl Theme {
    /// The bundle named `name` (exact, then case-insensitive).
    pub fn bundle(&self, name: &str) -> Option<&ThemeBundle> {
        let name = name.trim();
        self.bundles.iter().find(|b| b.name == name).or_else(|| {
            self.bundles
                .iter()
                .find(|b| b.name.eq_ignore_ascii_case(name))
        })
    }

    /// The active bundle, if `active_bundle` names one.
    pub fn active_bundle(&self) -> Option<&ThemeBundle> {
        if self.active_bundle.is_empty() {
            return None;
        }
        self.bundle(&self.active_bundle)
    }

    /// `base` if no bundle uses it, else `base 2`, `base 3`, … — new bundle
    /// names stay unique so a name always picks one bundle.
    pub fn unique_bundle_name(&self, base: &str) -> String {
        let base = base.trim();
        let taken = |n: &str| self.bundles.iter().any(|b| b.name.eq_ignore_ascii_case(n));
        if !taken(base) {
            return base.to_string();
        }
        (2..)
            .map(|i| format!("{base} {i}"))
            .find(|n| !taken(n))
            .unwrap_or_else(|| base.to_string())
    }
}

/// Why a bundle could not be saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleError {
    EmptyName,
    /// Another bundle already has this name (names are the apply key).
    DuplicateName(String),
}

impl std::fmt::Display for BundleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyName => f.write_str("Give the bundle a name."),
            Self::DuplicateName(n) => write!(f, "A bundle named “{n}” already exists."),
        }
    }
}

impl Theme {
    /// Save `bundle`: replace the bundle named `original` (an edit), or
    /// append it (`None`, a new bundle). The name is trimmed and must be
    /// unique (case-insensitive) among the other bundles. Renaming the
    /// active bundle keeps it active.
    pub fn save_bundle(
        &mut self,
        original: Option<&str>,
        mut bundle: ThemeBundle,
    ) -> Result<(), BundleError> {
        bundle.name = bundle.name.trim().to_string();
        if bundle.name.is_empty() {
            return Err(BundleError::EmptyName);
        }
        bundle.wallpapers.retain(|w| !w.trim().is_empty());
        let slot = original.and_then(|o| self.bundles.iter().position(|b| b.name == o));
        let clash = self
            .bundles
            .iter()
            .enumerate()
            .any(|(i, b)| Some(i) != slot && b.name.eq_ignore_ascii_case(&bundle.name));
        if clash {
            return Err(BundleError::DuplicateName(bundle.name));
        }
        match slot {
            Some(i) => {
                if self.active_bundle == self.bundles[i].name {
                    self.active_bundle = bundle.name.clone();
                }
                self.bundles[i] = bundle;
            }
            None => self.bundles.push(bundle),
        }
        Ok(())
    }

    /// Remove the bundle named `name`; deleting the active bundle ends the
    /// wallpaper scoping.
    pub fn delete_bundle(&mut self, name: &str) {
        self.bundles.retain(|b| b.name != name);
        if self.active_bundle == name {
            self.active_bundle.clear();
        }
    }
}

#[cfg(test)]
mod bundle_tests {
    use super::*;

    fn bundle(name: &str) -> ThemeBundle {
        ThemeBundle {
            name: name.to_string(),
            ..ThemeBundle::default()
        }
    }

    #[test]
    fn apply_sets_scheme_matugen_and_active_name_only() {
        let mut theme = Theme::default();
        let before_contrast = theme.matugen.contrast.clone();
        let b = ThemeBundle {
            name: "Rose".into(),
            theme: Themes::RosePine,
            scheme_type: MatugenType::Vibrant,
            mode: MatugenMode::Light,
            wallpapers: vec!["/w/a.png".into()],
            preview: String::new(),
        };
        b.apply_to(&mut theme);
        assert_eq!(theme.theme, Themes::RosePine);
        assert_eq!(theme.matugen.scheme_type, MatugenType::Vibrant);
        assert_eq!(theme.matugen.mode, MatugenMode::Light);
        assert_eq!(theme.active_bundle, "Rose");
        assert_eq!(theme.matugen.contrast, before_contrast);
    }

    #[test]
    fn from_current_captures_the_look_in_use() {
        let mut theme = Theme::default();
        theme.theme = Themes::TokyoNight;
        theme.matugen.mode = MatugenMode::Light;
        let b = ThemeBundle::from_current("Now", &theme, Some("/w/x.jpg"));
        assert_eq!(b.theme, Themes::TokyoNight);
        assert_eq!(b.mode, MatugenMode::Light);
        assert_eq!(b.wallpapers, ["/w/x.jpg"]);
        assert!(
            ThemeBundle::from_current("Now", &theme, None)
                .wallpapers
                .is_empty()
        );
    }

    #[test]
    fn lookup_is_exact_first_then_case_insensitive() {
        let mut theme = Theme::default();
        theme.bundles = vec![bundle("night"), bundle("Night")];
        assert_eq!(
            theme.bundle("Night").map(|b| b.name.as_str()),
            Some("Night")
        );
        assert_eq!(
            theme.bundle(" NIGHT ").map(|b| b.name.as_str()),
            Some("night")
        );
        assert!(theme.bundle("day").is_none());
        assert!(theme.active_bundle().is_none(), "empty active name");
        theme.active_bundle = "gone".into();
        assert!(theme.active_bundle().is_none(), "stale active name");
        theme.active_bundle = "night".into();
        assert_eq!(
            theme.active_bundle().map(|b| b.name.as_str()),
            Some("night")
        );
    }

    #[test]
    fn unique_names_count_up() {
        let mut theme = Theme::default();
        assert_eq!(theme.unique_bundle_name("Bundle"), "Bundle");
        theme.bundles = vec![bundle("Bundle"), bundle("bundle 2")];
        assert_eq!(theme.unique_bundle_name("Bundle"), "Bundle 3");
    }

    #[test]
    fn save_appends_replaces_and_follows_a_rename() {
        let mut theme = Theme::default();
        theme.save_bundle(None, bundle(" Night ")).expect("new");
        assert_eq!(theme.bundles[0].name, "Night", "trimmed");
        theme.active_bundle = "Night".into();
        let renamed = ThemeBundle {
            wallpapers: vec!["/w/a.png".into(), "  ".into()],
            ..bundle("Midnight")
        };
        theme.save_bundle(Some("Night"), renamed).expect("edit");
        assert_eq!(theme.bundles.len(), 1);
        assert_eq!(theme.bundles[0].name, "Midnight");
        assert_eq!(
            theme.bundles[0].wallpapers,
            ["/w/a.png"],
            "blank paths dropped"
        );
        assert_eq!(theme.active_bundle, "Midnight", "active follows the rename");
        // Keeping its own name (in another case) is not a clash.
        theme
            .save_bundle(Some("Midnight"), bundle("MIDNIGHT"))
            .expect("recase");
    }

    #[test]
    fn save_rejects_empty_and_duplicate_names() {
        let mut theme = Theme::default();
        theme.bundles = vec![bundle("Day"), bundle("Night")];
        assert_eq!(
            theme.save_bundle(None, bundle("  ")),
            Err(BundleError::EmptyName)
        );
        assert_eq!(
            theme.save_bundle(None, bundle("day")),
            Err(BundleError::DuplicateName("day".into()))
        );
        assert_eq!(
            theme.save_bundle(Some("Night"), bundle("Day")),
            Err(BundleError::DuplicateName("Day".into()))
        );
        assert_eq!(theme.bundles.len(), 2, "nothing changed");
    }

    #[test]
    fn deleting_the_active_bundle_ends_scoping() {
        let mut theme = Theme::default();
        theme.bundles = vec![bundle("Day"), bundle("Night")];
        theme.active_bundle = "Night".into();
        theme.delete_bundle("Day");
        assert_eq!(theme.active_bundle, "Night");
        theme.delete_bundle("Night");
        assert!(theme.bundles.is_empty());
        assert!(theme.active_bundle.is_empty());
    }

    #[test]
    fn slugs_are_file_name_safe() {
        assert_eq!(bundle("Rose Pine!").slug(), "rose-pine");
        assert_eq!(bundle("  Gruvbox -- Dark ").slug(), "gruvbox-dark");
        assert_eq!(bundle("../..").slug(), "bundle");
        assert_eq!(bundle("Çay Saati").slug(), "çay-saati");
    }

    #[test]
    fn old_configs_without_bundles_still_parse() {
        let theme: Theme = serde_json::from_str("{}").expect("parse");
        assert!(theme.bundles.is_empty());
        assert!(theme.active_bundle.is_empty());
        let b: ThemeBundle = serde_json::from_str(r#"{"name":"x"}"#).expect("parse");
        assert_eq!(b.theme, Themes::Wallpaper);
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
