use crate::json_struct::{MShell, MatugenTheme};
use crate::static_themes::ayu_dark::ayu_dark;
use crate::static_themes::catppuccin_mocha::catppuccin_mocha;
use crate::static_themes::dracula::dracula;
use crate::static_themes::everforest_dark_medium::everforest_dark_medium;
use crate::static_themes::flexoki::flexoki;
use crate::static_themes::github_dark::github_dark;
use crate::static_themes::gruvbox_dark_medium::gruvbox_dark_medium;
use crate::static_themes::gruvbox_material::gruvbox_material;
use crate::static_themes::horizon::horizon;
use crate::static_themes::kanagawa_wave::kanagawa_wave;
use crate::static_themes::kenp::kenp;
use crate::static_themes::margo::margo;
use crate::static_themes::monokai_classic::monokai_classic;
use crate::static_themes::nord_dark::nord_dark;
use crate::static_themes::one_dark::one_dark;
use crate::static_themes::oxocarbon::oxocarbon;
use crate::static_themes::rose_pine::rose_pine;
use crate::static_themes::solarized_dark::solarized_dark;
use crate::static_themes::tokyo_night::tokyo_night;
use crate::static_themes::vesper::vesper;
use mshell_config::schema::themes::Themes;

pub fn static_theme(theme: &Themes, mshell: Option<MShell>) -> Option<MatugenTheme> {
    let mshell = mshell.unwrap_or_default();
    match theme {
        // `Default` is the alias for "margo's default colour scheme",
        // which is now the house theme Kenp — so a profile carrying
        // `theme: Default` lands on Kenp, not on the old brand look.
        // `Margo` still resolves to the original brand palette so it
        // stays selectable. `Wallpaper` stays `None` because it *is*
        // the dynamic-from-wallpaper mode.
        Themes::Default => Some(kenp(mshell)),
        Themes::Margo => Some(margo(mshell)),
        Themes::Wallpaper => None,
        Themes::Kenp => Some(kenp(mshell)),
        Themes::AyuDark => Some(ayu_dark(mshell)),
        Themes::CatppuccinMocha => Some(catppuccin_mocha(mshell)),
        Themes::Dracula => Some(dracula(mshell)),
        Themes::EverforestDarkMedium => Some(everforest_dark_medium(mshell)),
        Themes::Flexoki => Some(flexoki(mshell)),
        Themes::GithubDark => Some(github_dark(mshell)),
        Themes::GruvboxDarkMedium => Some(gruvbox_dark_medium(mshell)),
        Themes::GruvboxMaterial => Some(gruvbox_material(mshell)),
        Themes::Horizon => Some(horizon(mshell)),
        Themes::KanagawaWave => Some(kanagawa_wave(mshell)),
        Themes::MonokaiClassic => Some(monokai_classic(mshell)),
        Themes::NordDark => Some(nord_dark(mshell)),
        Themes::OneDark => Some(one_dark(mshell)),
        Themes::Oxocarbon => Some(oxocarbon(mshell)),
        Themes::RosePine => Some(rose_pine(mshell)),
        Themes::SolarizedDark => Some(solarized_dark(mshell)),
        Themes::TokyoNight => Some(tokyo_night(mshell)),
        Themes::Vesper => Some(vesper(mshell)),
    }
}

#[cfg(test)]
mod theme_invariant_tests {
    use super::*;

    /// Parse a `#RRGGBB` string; `None` unless it's exactly `#` + 6 hex digits.
    fn parse_hex(s: &str) -> Option<(u8, u8, u8)> {
        let h = s.strip_prefix('#')?;
        if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        Some((
            u8::from_str_radix(&h[0..2], 16).ok()?,
            u8::from_str_radix(&h[2..4], 16).ok()?,
            u8::from_str_radix(&h[4..6], 16).ok()?,
        ))
    }

    /// WCAG relative luminance of an sRGB colour.
    fn luminance((r, g, b): (u8, u8, u8)) -> f64 {
        let lin = |c: u8| {
            let c = c as f64 / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    }

    /// WCAG contrast ratio between two colours (1.0 .. 21.0).
    fn contrast(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// Collect every `"color": "#..."` value in a serialized theme, so the
    /// check covers base16 + all M3 roles + palettes without listing fields.
    fn collect_colors(v: &serde_json::Value, out: &mut Vec<String>) {
        match v {
            serde_json::Value::Object(map) => {
                for (k, val) in map {
                    if k == "color" {
                        if let serde_json::Value::String(s) = val {
                            out.push(s.clone());
                        }
                    } else {
                        collect_colors(val, out);
                    }
                }
            }
            serde_json::Value::Array(arr) => arr.iter().for_each(|x| collect_colors(x, out)),
            _ => {}
        }
    }

    #[test]
    fn wallpaper_is_the_only_dynamic_theme() {
        // `Wallpaper` *is* the dynamic matugen mode, so it has no static palette.
        assert!(
            static_theme(&Themes::Wallpaper, None).is_none(),
            "Wallpaper must not map to a static palette",
        );
    }

    #[test]
    fn every_static_theme_produces_a_valid_dark_palette() {
        for theme in Themes::all() {
            if *theme == Themes::Wallpaper {
                continue;
            }
            let built = static_theme(theme, None)
                .unwrap_or_else(|| panic!("theme {:?} produced no MatugenTheme", theme.ident()));

            // Every colour (base16 + M3 roles + palettes) must be a well-formed
            // `#RRGGBB` — a truncated/malformed hex silently renders as black
            // through `ColorEntry::as_rgb`.
            let json = serde_json::to_value(&built).expect("theme serializes");
            let mut colors = Vec::new();
            collect_colors(&json, &mut colors);
            assert!(
                colors.len() >= 16,
                "theme {:?} produced only {} colours",
                theme.ident(),
                colors.len(),
            );
            for c in &colors {
                assert!(
                    parse_hex(c).is_some(),
                    "theme {:?} has a malformed colour {c:?}",
                    theme.ident(),
                );
            }

            // The catalogue is dark-only, and the default foreground must be
            // legible on the surface. 3.0:1 is a lenient legibility floor that
            // still catches a broken (e.g. near-black-on-black) palette.
            assert!(built.is_dark_mode, "theme {:?} must be dark", theme.ident());
            let bg = parse_hex(&built.colors.surface.default.color).unwrap();
            let fg = parse_hex(&built.colors.on_surface.default.color).unwrap();
            let ratio = contrast(bg, fg);
            assert!(
                ratio >= 3.0,
                "theme {:?} surface/on_surface contrast {ratio:.2} is below 3.0:1",
                theme.ident(),
            );
        }
    }
}
