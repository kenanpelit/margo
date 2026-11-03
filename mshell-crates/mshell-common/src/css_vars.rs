//! Reading the live theme's CSS custom properties (`--primary`, `--frame-bg`,
//! …) back out of GTK. GTK exposes matugen's colours as CSS variables, not
//! `@define-color`s, so `lookup_color` can't see them; the only way to get
//! one as a literal is to print a widget's resolved style and pick the
//! `--name: value;` lines out of it. Shared by the frame painter and the
//! Settings colour pickers that seed themselves from the theme.

use relm4::gtk;
use relm4::gtk::prelude::*;
use std::collections::HashMap;

/// How many `var(--x)` hops [`CssVars::resolve`] follows before giving up —
/// a guard against a self-referential definition (`--a: var(--a)`).
const MAX_VAR_DEPTH: usize = 16;

/// The custom properties visible on one widget, by name (`--surface`).
#[derive(Debug, Default, Clone)]
pub struct CssVars(HashMap<String, String>);

impl CssVars {
    /// Pick the `--name: value;` lines out of a printed style
    /// (`StyleContext::to_string(SHOW_STYLE)` output).
    pub fn parse(css: &str) -> Self {
        let mut vars = HashMap::new();
        for line in css.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("--")
                && let Some((name, value)) = rest.split_once(':')
            {
                let name = format!("--{}", name.trim());
                let value = value.trim().trim_end_matches(';').trim().to_string();
                vars.insert(name, value);
            }
        }
        Self(vars)
    }

    /// The custom properties in `widget`'s resolved style. The widget must be
    /// in a realized window for the theme's `:root` variables to reach it.
    pub fn of_widget(widget: &impl IsA<gtk::Widget>) -> Self {
        #[allow(deprecated)]
        let css = widget
            .style_context()
            .to_string(gtk::StyleContextPrintFlags::SHOW_STYLE);
        Self::parse(css.as_str())
    }

    /// The raw (unresolved) value of `name`.
    pub fn raw(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    /// `value` with `var(--x)` / `var(--x, fallback)` references followed to
    /// a literal. An undefined variable without a fallback, or a reference
    /// cycle, yields `None`.
    pub fn resolve<'a>(&'a self, value: &'a str) -> Option<&'a str> {
        let mut value = value.trim();
        for _ in 0..MAX_VAR_DEPTH {
            let Some(inner) = value.strip_prefix("var(").and_then(|s| s.strip_suffix(')')) else {
                return Some(value);
            };
            let (name, fallback) = match inner.split_once(',') {
                Some((name, fallback)) => (name.trim(), Some(fallback.trim())),
                None => (inner.trim(), None),
            };
            value = match (self.raw(name), fallback) {
                (Some(v), _) => v.trim(),
                (None, Some(f)) => f,
                (None, None) => return None,
            };
        }
        None
    }

    /// The resolved value of variable `name`.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.resolve(self.raw(name)?)
    }

    /// Variable `name` as a colour (`#hex`, `rgb()` or `rgba()`), in unit
    /// floats.
    pub fn color(&self, name: &str) -> Option<(f64, f64, f64, f64)> {
        parse_css_color(self.get(name)?)
    }
}

/// A CSS colour literal as unit floats: `#rgb[a]` / `#rrggbb[aa]`, or
/// `rgb(r, g, b)` / `rgba(r, g, b, a)` with channels as `0..=255` or
/// `0.0..=1.0`. `None` for anything else (named colours, `color-mix()`, …).
pub fn parse_css_color(s: &str) -> Option<(f64, f64, f64, f64)> {
    let s = s.trim();

    if s.starts_with('#') {
        return crate::color::parse_hex_rgba(s);
    }

    if let Some(inner) = s.strip_prefix("rgba(").and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() == 4 {
            let r = parse_channel(parts[0])?;
            let g = parse_channel(parts[1])?;
            let b = parse_channel(parts[2])?;
            let a = parse_channel(parts[3])?;
            return Some((r, g, b, a));
        }
    }

    if let Some(inner) = s.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() == 3 {
            let r = parse_channel(parts[0])?;
            let g = parse_channel(parts[1])?;
            let b = parse_channel(parts[2])?;
            return Some((r, g, b, 1.0));
        }
    }

    None
}

fn parse_channel(s: &str) -> Option<f64> {
    let v: f64 = s.trim().parse().ok()?;
    if v > 1.0 {
        Some((v / 255.0).clamp(0.0, 1.0))
    } else {
        Some(v.clamp(0.0, 1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STYLE: &str = "\
window.background:dir(ltr) {
  color: rgb(205,214,244);
  --surface: #282A36;
  --outline: #6272a4;
  --frame-bg: var(--surface);
  --frame-border: var(--outline);
  --alias: var(--frame-bg);
  --loop: var(--loop);
  --dangling: var(--nope);
  --fallback: var(--nope, var(--outline));
  --rgba: rgba(255, 0, 0, 0.5);
}";

    #[test]
    fn picks_custom_properties_out_of_a_printed_style() {
        let vars = CssVars::parse(STYLE);
        assert_eq!(vars.raw("--surface"), Some("#282A36"));
        assert_eq!(vars.raw("--frame-bg"), Some("var(--surface)"));
        assert_eq!(vars.raw("color"), None, "only custom properties");
    }

    #[test]
    fn follows_var_references_to_a_literal() {
        let vars = CssVars::parse(STYLE);
        assert_eq!(vars.get("--frame-bg"), Some("#282A36"));
        assert_eq!(vars.get("--alias"), Some("#282A36"));
        assert_eq!(vars.get("--fallback"), Some("#6272a4"));
    }

    #[test]
    fn unresolvable_references_are_none() {
        let vars = CssVars::parse(STYLE);
        assert_eq!(vars.get("--loop"), None, "cycle must not recurse forever");
        assert_eq!(vars.get("--dangling"), None);
        assert_eq!(vars.get("--missing"), None);
    }

    #[test]
    fn colours_resolve_through_references() {
        let vars = CssVars::parse(STYLE);
        let (r, g, b, a) = vars.color("--frame-border").expect("colour");
        assert_eq!(
            (r, g, b, a),
            (
                0x62 as f64 / 255.0,
                0x72 as f64 / 255.0,
                0xa4 as f64 / 255.0,
                1.0
            )
        );
        assert_eq!(vars.color("--rgba"), Some((1.0, 0.0, 0.0, 0.5)));
    }

    #[test]
    fn css_colour_forms() {
        assert_eq!(
            parse_css_color("rgb(255, 255, 0)"),
            Some((1.0, 1.0, 0.0, 1.0))
        );
        assert_eq!(parse_css_color(" #ff000080 ").map(|c| c.0), Some(1.0));
        assert_eq!(parse_css_color("red"), None);
        assert_eq!(parse_css_color("rgba(1, 2, 3)"), None);
    }
}
