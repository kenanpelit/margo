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
