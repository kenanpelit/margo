//! Palette preview shared by the theme cards in Settings → Theme and the
//! frame's theme-picker menu (the drawing code used to be copied in both).

use mshell_config::schema::themes::Themes;
use mshell_matugen::static_theme_mapping::static_theme;
use relm4::gtk;
use relm4::gtk::gdk;
use relm4::gtk::prelude::DrawingAreaExtManual;

/// A static theme's preview colours: `[surface, on_surface, primary,
/// secondary, tertiary, error, outline]`. `None` for themes without a static
/// palette (default / wallpaper-generated).
pub fn theme_swatch_colors(theme: &Themes) -> Option<[String; 7]> {
    let t = static_theme(theme, None)?;
    Some([
        t.colors.surface.default.color.clone(),
        t.colors.on_surface.default.color.clone(),
        t.colors.primary.default.color.clone(),
        t.colors.secondary.default.color.clone(),
        t.colors.tertiary.default.color.clone(),
        t.colors.error.default.color.clone(),
        t.colors.outline.default.color.clone(),
    ])
}

fn set_source(cr: &gtk::cairo::Context, color: &str) -> bool {
    match gdk::RGBA::parse(color) {
        Ok(rgba) => {
            cr.set_source_rgba(
                f64::from(rgba.red()),
                f64::from(rgba.green()),
                f64::from(rgba.blue()),
                f64::from(rgba.alpha()),
            );
            true
        }
        Err(_) => false,
    }
}

/// Paint `colors` into a card: `bg` gets the surface colour as a rounded
/// rect, each `swatches[i]` a flat block of `colors[i + 1]`.
pub fn install_theme_swatches(
    colors: &[String; 7],
    bg: &gtk::DrawingArea,
    swatches: [&gtk::DrawingArea; 6],
) {
    let bg_color = colors[0].clone();
    bg.set_draw_func(move |_, cr, w, h| {
        if !set_source(cr, &bg_color) {
            return;
        }
        let r = 6.0_f64;
        let (w, h) = (f64::from(w), f64::from(h));
        use std::f64::consts::{FRAC_PI_2, PI};
        cr.new_sub_path();
        cr.arc(w - r, r, r, -FRAC_PI_2, 0.0);
        cr.arc(w - r, h - r, r, 0.0, FRAC_PI_2);
        cr.arc(r, h - r, r, FRAC_PI_2, PI);
        cr.arc(r, r, r, PI, 3.0 * FRAC_PI_2);
        cr.close_path();
        let _ = cr.fill();
    });
    for (swatch, color) in swatches.into_iter().zip(colors[1..].iter()) {
        let color = color.clone();
        swatch.set_draw_func(move |_, cr, w, h| {
            if set_source(cr, &color) {
                cr.rectangle(0.0, 0.0, f64::from(w), f64::from(h));
                let _ = cr.fill();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every static theme yields seven readable hex colours for its card.
    #[test]
    fn static_themes_have_seven_hex_swatches() {
        let mut seen = 0;
        for theme in Themes::all() {
            if let Some(colors) = theme_swatch_colors(theme) {
                seen += 1;
                for c in &colors {
                    assert!(
                        mshell_common::color::parse_hex_bytes(c).is_some(),
                        "{theme:?}: {c:?}"
                    );
                }
            }
        }
        assert!(seen > 0, "at least one static theme");
    }
}
