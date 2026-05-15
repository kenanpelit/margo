use crate::json_struct::{MatugenTheme, MShell};
use crate::static_themes::bauhaus::bauhaus;
use crate::static_themes::black_turq::black_turq;
use crate::static_themes::bloodrust::blood_rust;
use crate::static_themes::catppuccin_frappe::catppuccin_frappe;
use crate::static_themes::catppuccin_latte::catppuccin_latte;
use crate::static_themes::catppuccin_macchiato::catppuccin_macchiato;
use crate::static_themes::catppuccin_mocha::catppuccin_mocha;
use crate::static_themes::cyberpunk::cyberpunk;
use crate::static_themes::desert_power::desert_power;
use crate::static_themes::dracula::dracula;
use crate::static_themes::eldritch::eldritch;
use crate::static_themes::ethereal::ethereal;
use crate::static_themes::everforest_dark_hard::everforest_dark_hard;
use crate::static_themes::everforest_dark_medium::everforest_dark_medium;
use crate::static_themes::everforest_dark_soft::everforest_dark_soft;
use crate::static_themes::everforest_light_hard::everforest_light_hard;
use crate::static_themes::everforest_light_medium::everforest_light_medium;
use crate::static_themes::everforest_light_soft::everforest_light_soft;
use crate::static_themes::gruvbox_dark_hard::gruvbox_dark_hard;
use crate::static_themes::gruvbox_dark_medium::gruvbox_dark_medium;
use crate::static_themes::gruvbox_dark_soft::gruvbox_dark_soft;
use crate::static_themes::gruvbox_light_hard::gruvbox_light_hard;
use crate::static_themes::gruvbox_light_medium::gruvbox_light_medium;
use crate::static_themes::gruvbox_light_soft::gruvbox_light_soft;
use crate::static_themes::hackerman::hackerman;
use crate::static_themes::inky_pinky::inky_pinky;
use crate::static_themes::kanagawa_dragon::kanagawa_dragon;
use crate::static_themes::kanagawa_lotus::kanagawa_lotus;
use crate::static_themes::kanagawa_wave::kanagawa_wave;
use crate::static_themes::margo::margo;
use crate::static_themes::miasma::miasma;
use crate::static_themes::monokai_classic::monokai_classic;
use crate::static_themes::nord_dark::nord_dark;
use crate::static_themes::nord_light::nord_light;
use crate::static_themes::oceanic_next::oceanic_next;
use crate::static_themes::one_dark::one_dark;
use crate::static_themes::osaka_jade::osaka_jade;
use crate::static_themes::poimandres::poimandres;
use crate::static_themes::retro_82::retro_82;
use crate::static_themes::rose_pine::rose_pine;
use crate::static_themes::rose_pine_dawn::rose_pine_dawn;
use crate::static_themes::rose_pine_moon::rose_pine_moon;
use crate::static_themes::saga::saga;
use crate::static_themes::seoul::seoul;
use crate::static_themes::solarized_dark::solarized_dark;
use crate::static_themes::solarized_light::solarized_light;
use crate::static_themes::solitude::solitude;
use crate::static_themes::synthwave_84::synthwave84;
use crate::static_themes::tokyo_night::tokyo_night;
use crate::static_themes::tokyo_night_light::tokyo_night_light;
use crate::static_themes::tokyo_night_storm::tokyo_night_storm;
use crate::static_themes::varda::varda;
use mshell_config::schema::themes::Themes;

pub fn static_theme(theme: &Themes, mshell: Option<MShell>) -> Option<MatugenTheme> {
    let mshell = mshell.unwrap_or_default();
    match theme {
        Themes::Default | Themes::Wallpaper => None,
        Themes::Margo => Some(margo(mshell)),
        Themes::Bauhaus => Some(bauhaus(mshell)),
        Themes::BlackTurq => Some(black_turq(mshell)),
        Themes::BloodRust => Some(blood_rust(mshell)),
        Themes::CatppuccinFrappe => Some(catppuccin_frappe(mshell)),
        Themes::CatppuccinLatte => Some(catppuccin_latte(mshell)),
        Themes::CatppuccinMacchiato => Some(catppuccin_macchiato(mshell)),
        Themes::CatppuccinMocha => Some(catppuccin_mocha(mshell)),
        Themes::Cyberpunk => Some(cyberpunk(mshell)),
        Themes::DesertPower => Some(desert_power(mshell)),
        Themes::Dracula => Some(dracula(mshell)),
        Themes::Eldritch => Some(eldritch(mshell)),
        Themes::Ethereal => Some(ethereal(mshell)),
        Themes::EverforestDarkHard => Some(everforest_dark_hard(mshell)),
        Themes::EverforestDarkMedium => Some(everforest_dark_medium(mshell)),
        Themes::EverforestDarkSoft => Some(everforest_dark_soft(mshell)),
        Themes::EverforestLightHard => Some(everforest_light_hard(mshell)),
        Themes::EverforestLightMedium => Some(everforest_light_medium(mshell)),
        Themes::EverforestLightSoft => Some(everforest_light_soft(mshell)),
        Themes::GruvboxDarkHard => Some(gruvbox_dark_hard(mshell)),
        Themes::GruvboxDarkMedium => Some(gruvbox_dark_medium(mshell)),
        Themes::GruvboxDarkSoft => Some(gruvbox_dark_soft(mshell)),
        Themes::GruvboxLightHard => Some(gruvbox_light_hard(mshell)),
        Themes::GruvboxLightMedium => Some(gruvbox_light_medium(mshell)),
        Themes::GruvboxLightSoft => Some(gruvbox_light_soft(mshell)),
        Themes::Hackerman => Some(hackerman(mshell)),
        Themes::InkyPinky => Some(inky_pinky(mshell)),
        Themes::KanagawaDragon => Some(kanagawa_dragon(mshell)),
        Themes::KanagawaLotus => Some(kanagawa_lotus(mshell)),
        Themes::KanagawaWave => Some(kanagawa_wave(mshell)),
        Themes::Miasma => Some(miasma(mshell)),
        Themes::MonokaiClassic => Some(monokai_classic(mshell)),
        Themes::NordDark => Some(nord_dark(mshell)),
        Themes::NordLight => Some(nord_light(mshell)),
        Themes::OceanicNext => Some(oceanic_next(mshell)),
        Themes::OneDark => Some(one_dark(mshell)),
        Themes::OsakaJade => Some(osaka_jade(mshell)),
        Themes::Poimandres => Some(poimandres(mshell)),
        Themes::Retro82 => Some(retro_82(mshell)),
        Themes::RosePine => Some(rose_pine(mshell)),
        Themes::RosePineDawn => Some(rose_pine_dawn(mshell)),
        Themes::RosePineMoon => Some(rose_pine_moon(mshell)),
        Themes::Saga => Some(saga(mshell)),
        Themes::Seoul => Some(seoul(mshell)),
        Themes::SolarizedDark => Some(solarized_dark(mshell)),
        Themes::SolarizedLight => Some(solarized_light(mshell)),
        Themes::Solitude => Some(solitude(mshell)),
        Themes::Synthwave84 => Some(synthwave84(mshell)),
        Themes::TokyoNight => Some(tokyo_night(mshell)),
        Themes::TokyoNightStorm => Some(tokyo_night_storm(mshell)),
        Themes::TokyoNightLight => Some(tokyo_night_light(mshell)),
        Themes::Varda => Some(varda(mshell)),
    }
}
