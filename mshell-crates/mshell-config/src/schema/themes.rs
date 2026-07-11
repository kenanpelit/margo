use reactive_stores::{KeyMap, PatchField, Store, StorePath};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Store, JsonSchema)]
pub enum Themes {
    Default,
    Wallpaper,
    Kenp,
    Margo,
    AyuDark,
    CatppuccinMocha,
    Dracula,
    EverforestDarkMedium,
    Flexoki,
    GithubDark,
    GruvboxDarkMedium,
    GruvboxMaterial,
    Horizon,
    KanagawaWave,
    MonokaiClassic,
    NordDark,
    OneDark,
    Oxocarbon,
    RosePine,
    SolarizedDark,
    TokyoNight,
    Vesper,
}

impl PatchField for Themes {
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

impl Themes {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Wallpaper => "Wallpaper",
            Self::Kenp => "Kenp",
            Self::Margo => "Margo",
            Self::AyuDark => "Ayu Dark",
            Self::CatppuccinMocha => "Catppuccin Mocha",
            Self::Dracula => "Dracula",
            Self::EverforestDarkMedium => "Everforest Dark Medium",
            Self::Flexoki => "Flexoki",
            Self::GithubDark => "GitHub Dark",
            Self::GruvboxDarkMedium => "Gruvbox Dark Medium",
            Self::GruvboxMaterial => "Gruvbox Material",
            Self::Horizon => "Horizon",
            Self::KanagawaWave => "Kanagawa Wave",
            Self::MonokaiClassic => "Monokai Classic",
            Self::NordDark => "Nord Dark",
            Self::OneDark => "One Dark",
            Self::Oxocarbon => "Oxocarbon",
            Self::RosePine => "Rosé Pine",
            Self::SolarizedDark => "Solarized Dark",
            Self::TokyoNight => "Tokyo Night",
            Self::Vesper => "Vesper",
        }
    }

    /// The canonical, stable identifier — the PascalCase variant name,
    /// which is exactly what serde writes to the YAML profile. Use this
    /// for machine-facing output (`mshellctl theme get` / `list`) so the
    /// printed name round-trips back through [`Self::from_cli`].
    pub fn ident(&self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Wallpaper => "Wallpaper",
            Self::Kenp => "Kenp",
            Self::Margo => "Margo",
            Self::AyuDark => "AyuDark",
            Self::CatppuccinMocha => "CatppuccinMocha",
            Self::Dracula => "Dracula",
            Self::EverforestDarkMedium => "EverforestDarkMedium",
            Self::Flexoki => "Flexoki",
            Self::GithubDark => "GithubDark",
            Self::GruvboxDarkMedium => "GruvboxDarkMedium",
            Self::GruvboxMaterial => "GruvboxMaterial",
            Self::Horizon => "Horizon",
            Self::KanagawaWave => "KanagawaWave",
            Self::MonokaiClassic => "MonokaiClassic",
            Self::NordDark => "NordDark",
            Self::OneDark => "OneDark",
            Self::Oxocarbon => "Oxocarbon",
            Self::RosePine => "RosePine",
            Self::SolarizedDark => "SolarizedDark",
            Self::TokyoNight => "TokyoNight",
            Self::Vesper => "Vesper",
        }
    }

    /// Resolve a user-typed name to a theme, tolerant of case and of
    /// separators. We compare on the ASCII-alphanumeric skeleton of both
    /// the [`Self::ident`] and the [`Self::label`], so `kenp`, `Kenp`,
    /// `tokyo-night`, `tokyo_night`, `TokyoNight` and
    /// `"Tokyo Night"` all resolve. Matching the ident (which carries no
    /// accents) is what lets `catppuccin-frappe` / `rose-pine` work even
    /// though their labels are accented ("Frappé" / "Rosé").
    pub fn from_cli(input: &str) -> Option<Self> {
        let skeleton = |s: &str| -> String {
            s.chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .map(|c| c.to_ascii_lowercase())
                .collect()
        };
        let target = skeleton(input);
        if target.is_empty() {
            return None;
        }
        Self::all()
            .iter()
            .copied()
            .find(|t| skeleton(t.ident()) == target || skeleton(t.label()) == target)
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::Default,
            Self::Wallpaper,
            Self::Kenp,
            Self::Margo,
            Self::AyuDark,
            Self::CatppuccinMocha,
            Self::Dracula,
            Self::EverforestDarkMedium,
            Self::Flexoki,
            Self::GithubDark,
            Self::GruvboxDarkMedium,
            Self::GruvboxMaterial,
            Self::Horizon,
            Self::KanagawaWave,
            Self::MonokaiClassic,
            Self::NordDark,
            Self::OneDark,
            Self::Oxocarbon,
            Self::RosePine,
            Self::SolarizedDark,
            Self::TokyoNight,
            Self::Vesper,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Store, JsonSchema)]
pub enum MatugenPreference {
    Darkness,
    Lightness,
    Saturation,
    LessSaturation,
    Value,
}

impl PatchField for MatugenPreference {
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

impl fmt::Display for MatugenPreference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Darkness => "darkness",
            Self::Lightness => "lightness",
            Self::Saturation => "saturation",
            Self::LessSaturation => "less-saturation",
            Self::Value => "value",
        };
        f.write_str(s)
    }
}

impl MatugenPreference {
    pub fn all() -> &'static [Self] {
        &[
            Self::Darkness,
            Self::Lightness,
            Self::Saturation,
            Self::LessSaturation,
            Self::Value,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Darkness => "Darkness",
            Self::Lightness => "Lightness",
            Self::Saturation => "Saturation",
            Self::LessSaturation => "Less Saturation",
            Self::Value => "Value",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Store, JsonSchema)]
pub enum MatugenType {
    Content,
    Expressive,
    Fidelity,
    FruitSalad,
    Monochrome,
    Neutral,
    Rainbow,
    TonalSpot,
    Vibrant,
}

impl PatchField for MatugenType {
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

impl fmt::Display for MatugenType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Content => "scheme-content",
            Self::Expressive => "scheme-expressive",
            Self::Fidelity => "scheme-fidelity",
            Self::FruitSalad => "scheme-fruit-salad",
            Self::Monochrome => "scheme-monochrome",
            Self::Neutral => "scheme-neutral",
            Self::Rainbow => "scheme-rainbow",
            Self::TonalSpot => "scheme-tonal-spot",
            Self::Vibrant => "scheme-vibrant",
        };
        f.write_str(s)
    }
}

impl MatugenType {
    pub fn all() -> &'static [Self] {
        &[
            Self::Content,
            Self::Expressive,
            Self::Fidelity,
            Self::FruitSalad,
            Self::Monochrome,
            Self::Neutral,
            Self::Rainbow,
            Self::TonalSpot,
            Self::Vibrant,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Content => "Content",
            Self::Expressive => "Expressive",
            Self::Fidelity => "Fidelity",
            Self::FruitSalad => "Fruit Salad",
            Self::Monochrome => "Monochrome",
            Self::Neutral => "Neutral",
            Self::Rainbow => "Rainbow",
            Self::TonalSpot => "Tonal Spot",
            Self::Vibrant => "Vibrant",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Store, JsonSchema)]
pub enum MatugenMode {
    Light,
    Dark,
}

impl PatchField for MatugenMode {
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

impl fmt::Display for MatugenMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Light => "light",
            Self::Dark => "dark",
        };
        f.write_str(s)
    }
}

impl MatugenMode {
    pub fn all() -> &'static [Self] {
        &[Self::Light, Self::Dark]
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Store, JsonSchema)]
pub struct MatugenContrast(f64);

impl MatugenContrast {
    pub fn new(v: f64) -> Self {
        Self(v.clamp(-1.0, 1.0))
    }
    pub fn get(&self) -> f64 {
        self.0
    }
}

impl fmt::Display for MatugenContrast {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.2}", self.0)
    }
}

impl PatchField for MatugenContrast {
    fn patch_field(
        &mut self,
        new: Self,
        path: &StorePath,
        notify: &mut dyn FnMut(&StorePath),
        _keys: Option<&KeyMap>,
    ) {
        let clamped = MatugenContrast::new(new.0);
        if self.0 != clamped.0 {
            *self = clamped;
            notify(path);
        }
    }
}

impl PartialEq for MatugenContrast {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for MatugenContrast {}

#[derive(Clone, Debug, Serialize, Deserialize, Store, JsonSchema)]
pub struct WindowOpacity(f64);

impl WindowOpacity {
    pub fn new(v: f64) -> Self {
        Self(v.clamp(0.0, 1.0))
    }
    pub fn get(&self) -> f64 {
        self.0
    }
}

impl fmt::Display for WindowOpacity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.2}", self.0)
    }
}

impl PatchField for WindowOpacity {
    fn patch_field(
        &mut self,
        new: Self,
        path: &StorePath,
        notify: &mut dyn FnMut(&StorePath),
        _keys: Option<&KeyMap>,
    ) {
        let clamped = WindowOpacity::new(new.0);
        if self.0 != clamped.0 {
            *self = clamped;
            notify(path);
        }
    }
}

impl PartialEq for WindowOpacity {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for WindowOpacity {}

#[cfg(test)]
mod theme_ident_tests {
    use super::*;

    #[test]
    fn every_ident_round_trips_through_from_cli() {
        for theme in Themes::all() {
            let ident = theme.ident();
            assert_eq!(
                Themes::from_cli(ident),
                Some(*theme),
                "ident {ident:?} did not resolve back to its own theme",
            );
        }
    }

    #[test]
    fn idents_are_ascii_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for theme in Themes::all() {
            let ident = theme.ident();
            assert!(
                ident.is_ascii() && !ident.is_empty(),
                "ident {ident:?} must be non-empty ASCII (round-trips, no accents)",
            );
            assert!(seen.insert(ident), "duplicate ident {ident:?}");
        }
    }

    #[test]
    fn from_cli_ignores_case_and_separators() {
        for input in ["kenp", "Kenp", "KENP"] {
            assert_eq!(Themes::from_cli(input), Some(Themes::Kenp));
        }
        for input in ["tokyo-night", "tokyo_night", "TokyoNight", "Tokyo Night"] {
            assert_eq!(Themes::from_cli(input), Some(Themes::TokyoNight));
        }
        // Accented labels still resolve via the (accent-free) ident.
        assert_eq!(Themes::from_cli("rose-pine"), Some(Themes::RosePine));
        // Space-separated multi-word labels resolve too.
        assert_eq!(
            Themes::from_cli("catppuccin mocha"),
            Some(Themes::CatppuccinMocha),
        );
    }

    #[test]
    fn from_cli_rejects_unknown_and_empty() {
        assert_eq!(Themes::from_cli("not-a-real-theme"), None);
        assert_eq!(Themes::from_cli(""), None);
        assert_eq!(Themes::from_cli("   "), None);
    }
}
