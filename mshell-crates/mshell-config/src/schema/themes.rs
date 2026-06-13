use reactive_stores::{KeyMap, PatchField, Store, StorePath};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Store, JsonSchema)]
pub enum Themes {
    Default,
    Wallpaper,
    Bauhaus,
    BlackTurq,
    BloodRust,
    CatppuccinFrappe,
    CatppuccinLatte,
    CatppuccinMacchiato,
    CatppuccinMocha,
    Cyberpunk,
    DesertPower,
    Dracula,
    Eldritch,
    Ethereal,
    EverforestDarkHard,
    EverforestDarkMedium,
    EverforestDarkSoft,
    EverforestLightHard,
    EverforestLightMedium,
    EverforestLightSoft,
    GruvboxDarkHard,
    GruvboxDarkMedium,
    GruvboxDarkSoft,
    GruvboxLightHard,
    GruvboxLightMedium,
    GruvboxLightSoft,
    Hackerman,
    InkyPinky,
    KanagawaDragon,
    KanagawaLotus,
    KanagawaWave,
    Kenp,
    Margo,
    Miasma,
    MonokaiClassic,
    NordDark,
    NordLight,
    OceanicNext,
    OneDark,
    OsakaJade,
    Poimandres,
    Retro82,
    RosePine,
    RosePineDawn,
    RosePineMoon,
    Saga,
    Seoul,
    SolarizedDark,
    SolarizedLight,
    Solitude,
    Synthwave84,
    TokyoNight,
    TokyoNightStorm,
    TokyoNightLight,
    Varda,
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
            Self::Bauhaus => "Bauhaus",
            Self::BlackTurq => "Black Turq",
            Self::BloodRust => "Blood Rust",
            Self::CatppuccinFrappe => "Catppuccin Frappé",
            Self::CatppuccinLatte => "Catppuccin Latte",
            Self::CatppuccinMacchiato => "Catppuccin Macchiato",
            Self::CatppuccinMocha => "Catppuccin Mocha",
            Self::Cyberpunk => "Cyberpunk",
            Self::DesertPower => "Desert Power",
            Self::Dracula => "Dracula",
            Self::Eldritch => "Eldritch",
            Self::Ethereal => "Ethereal",
            Self::EverforestDarkHard => "Everforest Dark Hard",
            Self::EverforestDarkMedium => "Everforest Dark Medium",
            Self::EverforestDarkSoft => "Everforest Dark Soft",
            Self::EverforestLightHard => "Everforest Light Hard",
            Self::EverforestLightMedium => "Everforest Light Medium",
            Self::EverforestLightSoft => "Everforest Light Soft",
            Self::GruvboxDarkHard => "Gruvbox Dark Hard",
            Self::GruvboxDarkMedium => "Gruvbox Dark Medium",
            Self::GruvboxDarkSoft => "Gruvbox Dark Soft",
            Self::GruvboxLightHard => "Gruvbox Light Hard",
            Self::GruvboxLightMedium => "Gruvbox Light Medium",
            Self::GruvboxLightSoft => "Gruvbox Light Soft",
            Self::Hackerman => "Hackerman",
            Self::InkyPinky => "InkyPinky",
            Self::KanagawaDragon => "Kanagawa Dragon",
            Self::KanagawaLotus => "Kanagawa Lotus",
            Self::KanagawaWave => "Kanagawa Wave",
            Self::Kenp => "Kenp",
            Self::Margo => "Margo",
            Self::Miasma => "Miasma",
            Self::MonokaiClassic => "Monokai Classic",
            Self::NordDark => "Nord Dark",
            Self::NordLight => "Nord Light",
            Self::OneDark => "One Dark",
            Self::OceanicNext => "Oceanic Next",
            Self::OsakaJade => "Osaka Jade",
            Self::Poimandres => "Poimandres",
            Self::Retro82 => "Retro 82",
            Self::RosePine => "Rosé Pine",
            Self::RosePineDawn => "Rosé Pine Dawn",
            Self::RosePineMoon => "Rosé Pine Moon",
            Self::Saga => "Saga",
            Self::Seoul => "Seoul",
            Self::SolarizedDark => "Solarized Dark",
            Self::SolarizedLight => "Solarized Light",
            Self::Solitude => "Solitude",
            Self::Synthwave84 => "Synthwave 84",
            Self::TokyoNight => "Tokyo Night",
            Self::TokyoNightStorm => "Tokyo Night Storm",
            Self::TokyoNightLight => "Tokyo Night Light",
            Self::Varda => "Varda",
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
            Self::Bauhaus => "Bauhaus",
            Self::BlackTurq => "BlackTurq",
            Self::BloodRust => "BloodRust",
            Self::CatppuccinFrappe => "CatppuccinFrappe",
            Self::CatppuccinLatte => "CatppuccinLatte",
            Self::CatppuccinMacchiato => "CatppuccinMacchiato",
            Self::CatppuccinMocha => "CatppuccinMocha",
            Self::Cyberpunk => "Cyberpunk",
            Self::DesertPower => "DesertPower",
            Self::Dracula => "Dracula",
            Self::Eldritch => "Eldritch",
            Self::Ethereal => "Ethereal",
            Self::EverforestDarkHard => "EverforestDarkHard",
            Self::EverforestDarkMedium => "EverforestDarkMedium",
            Self::EverforestDarkSoft => "EverforestDarkSoft",
            Self::EverforestLightHard => "EverforestLightHard",
            Self::EverforestLightMedium => "EverforestLightMedium",
            Self::EverforestLightSoft => "EverforestLightSoft",
            Self::GruvboxDarkHard => "GruvboxDarkHard",
            Self::GruvboxDarkMedium => "GruvboxDarkMedium",
            Self::GruvboxDarkSoft => "GruvboxDarkSoft",
            Self::GruvboxLightHard => "GruvboxLightHard",
            Self::GruvboxLightMedium => "GruvboxLightMedium",
            Self::GruvboxLightSoft => "GruvboxLightSoft",
            Self::Hackerman => "Hackerman",
            Self::InkyPinky => "InkyPinky",
            Self::KanagawaDragon => "KanagawaDragon",
            Self::KanagawaLotus => "KanagawaLotus",
            Self::KanagawaWave => "KanagawaWave",
            Self::Kenp => "Kenp",
            Self::Margo => "Margo",
            Self::Miasma => "Miasma",
            Self::MonokaiClassic => "MonokaiClassic",
            Self::NordDark => "NordDark",
            Self::NordLight => "NordLight",
            Self::OceanicNext => "OceanicNext",
            Self::OneDark => "OneDark",
            Self::OsakaJade => "OsakaJade",
            Self::Poimandres => "Poimandres",
            Self::Retro82 => "Retro82",
            Self::RosePine => "RosePine",
            Self::RosePineDawn => "RosePineDawn",
            Self::RosePineMoon => "RosePineMoon",
            Self::Saga => "Saga",
            Self::Seoul => "Seoul",
            Self::SolarizedDark => "SolarizedDark",
            Self::SolarizedLight => "SolarizedLight",
            Self::Solitude => "Solitude",
            Self::Synthwave84 => "Synthwave84",
            Self::TokyoNight => "TokyoNight",
            Self::TokyoNightStorm => "TokyoNightStorm",
            Self::TokyoNightLight => "TokyoNightLight",
            Self::Varda => "Varda",
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
            Self::Bauhaus,
            Self::BlackTurq,
            Self::BloodRust,
            Self::CatppuccinFrappe,
            Self::CatppuccinLatte,
            Self::CatppuccinMacchiato,
            Self::CatppuccinMocha,
            Self::Cyberpunk,
            Self::DesertPower,
            Self::Dracula,
            Self::Eldritch,
            Self::Ethereal,
            Self::EverforestDarkHard,
            Self::EverforestDarkMedium,
            Self::EverforestDarkSoft,
            Self::EverforestLightHard,
            Self::EverforestLightMedium,
            Self::EverforestLightSoft,
            Self::GruvboxDarkHard,
            Self::GruvboxDarkMedium,
            Self::GruvboxDarkSoft,
            Self::GruvboxLightHard,
            Self::GruvboxLightMedium,
            Self::GruvboxLightSoft,
            Self::Hackerman,
            Self::InkyPinky,
            Self::KanagawaDragon,
            Self::KanagawaLotus,
            Self::KanagawaWave,
            Self::Kenp,
            Self::Margo,
            Self::Miasma,
            Self::MonokaiClassic,
            Self::NordDark,
            Self::NordLight,
            Self::OceanicNext,
            Self::OneDark,
            Self::OsakaJade,
            Self::Poimandres,
            Self::Retro82,
            Self::RosePine,
            Self::RosePineDawn,
            Self::RosePineMoon,
            Self::Saga,
            Self::Seoul,
            Self::SolarizedDark,
            Self::SolarizedLight,
            Self::Solitude,
            Self::Synthwave84,
            Self::TokyoNight,
            Self::TokyoNightStorm,
            Self::TokyoNightLight,
            Self::Varda,
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
        assert_eq!(
            Themes::from_cli("catppuccin frappe"),
            Some(Themes::CatppuccinFrappe),
        );
    }

    #[test]
    fn from_cli_rejects_unknown_and_empty() {
        assert_eq!(Themes::from_cli("not-a-real-theme"), None);
        assert_eq!(Themes::from_cli(""), None);
        assert_eq!(Themes::from_cli("   "), None);
    }
}
