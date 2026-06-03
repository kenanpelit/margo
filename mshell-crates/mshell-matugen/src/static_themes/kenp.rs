use crate::json_struct::{Base16, Colors, MShell, MatugenTheme, Palettes, color};

/// Kenp — the author's house theme. A high-contrast electric palette:
/// warm butter-yellow primary, periwinkle secondary and mint tertiary
/// over a near-black indigo surface, with a hot-pink/red error. The
/// light variant inverts to a soft lavender paper with an indigo primary
/// and a dark-indigo "ink" tertiary chip.
///
/// Designed to sit correctly across the *whole* margo surface area, not
/// just the bar: the five `surface_container_*` tiers form a monotonic
/// elevation ladder (deepest well → brightest lift) so dashboard cards,
/// menu card-stacks and control-center tiles separate visually; `outline`
/// is bright enough to read as a window border (it feeds the compositor's
/// unfocused `bordercolor`) and as an input edge; every `on-*` accent
/// rides on one shared deep `INK`; and the base16 row keeps eight distinct
/// terminal hues (red/amber/yellow/green/cyan/blue/magenta).
///
/// | Slot        | Dark      | Light     |
/// |-------------|-----------|-----------|
/// | surface     | `#070722` | `#eef0ff` |
/// | on-surface  | `#f3edf7` | `#101046` |
/// | outline     | `#3a3f7a` | `#7a80c8` |
/// | primary     | `#fff59b` | `#4a52e0` |
/// | secondary   | `#a9aefe` | `#6b71c4` |
/// | tertiary    | `#9bfece` | `#101046` |
/// | error       | `#fd4663` | `#d92846` |
pub fn kenp(mshell: MShell) -> MatugenTheme {
    // --- Indigo-black elevation ladder (deepest → brightest) ---------
    const SC_LOWEST: &str = "#050518"; // recessed wells (below surface)
    const SURFACE: &str = "#070722"; // base canvas
    const SC_LOW: &str = "#0d0d2e";
    const SC: &str = "#14143a";
    const SC_HIGH: &str = "#1e1e4a";
    const SC_HIGHEST: &str = "#2a2a64"; // brightest lift (tiles, headers)
    const SURF_VAR: &str = "#1c1c46"; // mid surface — chips / inputs
    // --- Text -------------------------------------------------------
    const FG: &str = "#f3edf7"; // body / icon foreground
    const MUTED: &str = "#8b90c8"; // subtext / on-surface-variant
    const OUTLINE: &str = "#3a3f7a"; // visible border / divider
    const OUTLINE_V: &str = "#1e1e4a"; // faint hairline divider
    const INK: &str = "#0a0a26"; // text drawn ON the bright accents
    // --- Signature accents ------------------------------------------
    const PRIMARY: &str = "#fff59b"; // butter-yellow
    const SECONDARY: &str = "#a9aefe"; // periwinkle
    const TERTIARY: &str = "#9bfece"; // mint
    const ERROR: &str = "#fd4663"; // hot-pink red

    MatugenTheme {
        mshell,
        image: String::new(),
        is_dark_mode: true,
        mode: "dark".to_string(),
        base16: Base16 {
            base00: color(SURFACE),
            base01: color(SC_LOW),
            base02: color(SC_HIGH), // selection
            base03: color(MUTED),
            base04: color(MUTED),
            base05: color(FG),
            base06: color(FG),
            base07: color("#ffffff"),
            base08: color(ERROR),     // red
            base09: color("#ffd479"), // amber / orange
            base0a: color(PRIMARY),   // yellow
            base0b: color(TERTIARY),  // green / mint
            base0c: color("#7ce0ff"), // cyan
            base0d: color(SECONDARY), // blue / periwinkle
            base0e: color("#ff7eb6"), // magenta
            base0f: color(ERROR),
        },
        palettes: Palettes::default(),
        colors: Colors {
            surface: color(SURFACE),
            on_surface: color(FG),
            surface_variant: color(SURF_VAR),
            on_surface_variant: color(MUTED),
            surface_container_highest: color(SC_HIGHEST),
            surface_container_high: color(SC_HIGH),
            surface_container: color(SC),
            surface_container_low: color(SC_LOW),
            surface_container_lowest: color(SC_LOWEST),
            inverse_surface: color(FG),
            inverse_on_surface: color(SURFACE),
            surface_tint: color(PRIMARY),
            primary: color(PRIMARY),
            on_primary: color(INK),
            primary_container: color(SURF_VAR),
            on_primary_container: color(PRIMARY),
            secondary: color(SECONDARY),
            on_secondary: color(INK),
            secondary_container: color(SURF_VAR),
            on_secondary_container: color(SECONDARY),
            tertiary: color(TERTIARY),
            on_tertiary: color(INK),
            tertiary_container: color(SURF_VAR),
            on_tertiary_container: color(TERTIARY),
            error: color(ERROR),
            on_error: color(INK),
            error_container: color(SURF_VAR),
            on_error_container: color(ERROR),
            outline: color(OUTLINE),
            outline_variant: color(OUTLINE_V),
            background: color(SURFACE),
            on_background: color(FG),
            inverse_primary: color("#5d65f5"),
            primary_fixed: color(PRIMARY),
            primary_fixed_dim: color(PRIMARY),
            on_primary_fixed: color(INK),
            on_primary_fixed_variant: color(INK),
            secondary_fixed: color(SECONDARY),
            secondary_fixed_dim: color(SECONDARY),
            on_secondary_fixed: color(INK),
            on_secondary_fixed_variant: color(INK),
            tertiary_fixed: color(TERTIARY),
            tertiary_fixed_dim: color(TERTIARY),
            on_tertiary_fixed: color(INK),
            on_tertiary_fixed_variant: color(INK),
            scrim: color(SC_LOWEST),
            shadow: color("#000010"),
            source_color: color(PRIMARY),
            surface_bright: color(SC_HIGHEST),
            surface_dim: color(SURFACE),
        },
    }
}

/// Kenp Light — the light counterpart of [`kenp`]. Soft lavender paper
/// with an indigo primary; the elevation ladder runs the other way
/// (brightest paper → most-tinted lift) so cards still separate. Tertiary
/// is a dark-indigo "ink" chip carrying the butter-yellow as its text.
pub fn kenp_light(mshell: MShell) -> MatugenTheme {
    // --- Lavender paper ladder (brightest → most tinted) ------------
    const SC_LOWEST: &str = "#ffffff"; // brightest (raised cards)
    const SURFACE: &str = "#eef0ff"; // base paper
    const SC_LOW: &str = "#e7eafb";
    const SC: &str = "#e0e3f7";
    const SC_HIGH: &str = "#d7dbf3";
    const SC_HIGHEST: &str = "#ccd1ef"; // most-tinted lift
    const SURF_VAR: &str = "#dde0f6"; // chips / inputs
    // --- Text -------------------------------------------------------
    const FG: &str = "#101046"; // deep indigo ink
    const MUTED: &str = "#4b51a0"; // subtext
    const OUTLINE: &str = "#7a80c8"; // visible border / divider
    const OUTLINE_V: &str = "#c3c8ea"; // faint hairline
    const ON_LIGHT: &str = "#ffffff"; // text ON the saturated accents
    // --- Accents ----------------------------------------------------
    const PRIMARY: &str = "#4a52e0"; // indigo
    const SECONDARY: &str = "#6b71c4"; // muted periwinkle
    const TERTIARY: &str = "#101046"; // dark ink chip
    const ERROR: &str = "#d92846";

    MatugenTheme {
        mshell,
        image: String::new(),
        is_dark_mode: false,
        mode: "light".to_string(),
        base16: Base16 {
            base00: color(SURFACE),
            base01: color(SC_LOW),
            base02: color("#c3c8ea"), // selection
            base03: color(MUTED),
            base04: color(MUTED),
            base05: color(FG),
            base06: color(FG),
            base07: color("#000018"),
            base08: color(ERROR),     // red
            base09: color("#9a5b00"), // amber / orange
            base0a: color("#7d6a00"), // dark yellow
            base0b: color("#1f7a4f"), // green / mint
            base0c: color("#1f6a8a"), // cyan
            base0d: color("#3a40c0"), // blue
            base0e: color("#b03a8a"), // magenta
            base0f: color(ERROR),
        },
        palettes: Palettes::default(),
        colors: Colors {
            surface: color(SURFACE),
            on_surface: color(FG),
            surface_variant: color(SURF_VAR),
            on_surface_variant: color(MUTED),
            surface_container_highest: color(SC_HIGHEST),
            surface_container_high: color(SC_HIGH),
            surface_container: color(SC),
            surface_container_low: color(SC_LOW),
            surface_container_lowest: color(SC_LOWEST),
            inverse_surface: color(FG),
            inverse_on_surface: color(SURFACE),
            surface_tint: color(PRIMARY),
            primary: color(PRIMARY),
            on_primary: color(ON_LIGHT),
            primary_container: color("#dce0fa"),
            on_primary_container: color("#2a2f9e"),
            secondary: color(SECONDARY),
            on_secondary: color(ON_LIGHT),
            secondary_container: color(SURF_VAR),
            on_secondary_container: color("#3a3f8e"),
            tertiary: color(TERTIARY),
            on_tertiary: color("#fff59b"),
            tertiary_container: color(SURF_VAR),
            on_tertiary_container: color(FG),
            error: color(ERROR),
            on_error: color("#fff0f2"),
            error_container: color("#f7d8de"),
            on_error_container: color("#b01430"),
            outline: color(OUTLINE),
            outline_variant: color(OUTLINE_V),
            background: color(SURFACE),
            on_background: color(FG),
            inverse_primary: color("#fff59b"),
            primary_fixed: color(PRIMARY),
            primary_fixed_dim: color(PRIMARY),
            on_primary_fixed: color(ON_LIGHT),
            on_primary_fixed_variant: color(ON_LIGHT),
            secondary_fixed: color(SECONDARY),
            secondary_fixed_dim: color(SECONDARY),
            on_secondary_fixed: color(ON_LIGHT),
            on_secondary_fixed_variant: color(ON_LIGHT),
            tertiary_fixed: color(TERTIARY),
            tertiary_fixed_dim: color(TERTIARY),
            on_tertiary_fixed: color("#fff59b"),
            on_tertiary_fixed_variant: color("#fff59b"),
            scrim: color(FG),
            shadow: color("#c3c8ea"),
            source_color: color(PRIMARY),
            surface_bright: color(SC_LOWEST),
            surface_dim: color(SC_HIGHEST),
        },
    }
}
