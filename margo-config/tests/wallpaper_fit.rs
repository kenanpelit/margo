//! `wallpaper_fit` names: every fit the compositor wallpaper can draw is
//! accepted by the parser and by the validator, under one name each. The
//! runtime wallpaper IPC (`dispatch wallpaper_set … fit=<name>`) uses the
//! same names, so mshell's four `content_fit` modes must all be here.

use margo_config::WallpaperFit;
use margo_config::parse_config_str;

fn fit_of(line: &str) -> WallpaperFit {
    parse_config_str(line, None)
        .unwrap_or_else(|e| panic!("`{line}` should parse: {e:#}"))
        .wallpaper_fit
}

#[test]
fn every_fit_name_parses_to_its_variant() {
    assert_eq!(fit_of("wallpaper_fit = cover"), WallpaperFit::Cover);
    assert_eq!(fit_of("wallpaper_fit = contain"), WallpaperFit::Contain);
    assert_eq!(fit_of("wallpaper_fit = fill"), WallpaperFit::Fill);
    assert_eq!(fit_of("wallpaper_fit = center"), WallpaperFit::Center);
    assert_eq!(
        fit_of("wallpaper_fit = scale-down"),
        WallpaperFit::ScaleDown
    );
    assert_eq!(
        fit_of("wallpaper_fit = Scale-Down"),
        WallpaperFit::ScaleDown
    );
}

#[test]
fn an_unknown_fit_keeps_the_default() {
    // A bad value is a per-line diagnostic, not a parse failure.
    assert_eq!(fit_of("wallpaper_fit = tile"), WallpaperFit::Cover);
}

#[test]
fn names_round_trip() {
    for &fit in WallpaperFit::ALL {
        assert_eq!(WallpaperFit::from_name(fit.name()), Some(fit));
    }
    let names: Vec<&str> = WallpaperFit::ALL.iter().map(|f| f.name()).collect();
    assert_eq!(names, WallpaperFit::NAMES, "the validator's list");
}

#[test]
fn the_validator_accepts_every_fit_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.conf");
    for &fit in WallpaperFit::ALL {
        std::fs::write(&path, format!("wallpaper_fit = {}\n", fit.name())).expect("write");
        let report = margo_config::validator::validate_config(Some(&path)).expect("validate");
        assert!(
            report.diagnostics.iter().all(|d| !d.is_error()),
            "{}: {:?}",
            fit.name(),
            report.diagnostics
        );
    }
}
