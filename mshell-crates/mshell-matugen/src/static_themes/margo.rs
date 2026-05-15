use crate::json_struct::{Base16, Colors, MatugenTheme, MShell, Palettes, color};

/// Margo — the project's default colour scheme.
///
/// Warm-purple dark palette with a foreground tuned to match the
/// terminal foreground bar labels, menu text, and SVG icons read
/// against. The semantic slots (background, surface, surface
/// variants, primary / secondary / tertiary accents, error)
/// follow the project's house style.
///
/// Slot summary:
///
/// | Slot                | Hex      |
/// |---------------------|----------|
/// | background, surface | `#282A36` |
/// | surface variant     | `#44475A` |
/// | outline / muted     | `#6272A4` |
/// | foreground (text)   | `#CDD6F4` |
/// | primary             | `#BD93F9` |
/// | secondary           | `#FF79C6` |
/// | tertiary            | `#8BE9FD` |
/// | error               | `#FF5555` |
pub fn margo(mshell: MShell) -> MatugenTheme {
    // Foreground (text + icon). Shared across all on-surface /
    // on-background / inverse-surface / base05-07 slots so a
    // single tweak rolls through every text-bearing widget.
    const FG: &str = "#CDD6F4";

    MatugenTheme {
        mshell,
        image: String::new(),
        is_dark_mode: true,
        mode: "dark".to_string(),
        base16: Base16 {
            base00: color("#282A36"),
            base01: color("#44475A"),
            base02: color("#44475A"),
            base03: color("#6272A4"),
            base04: color("#6272A4"),
            // base05-07 = body / bold / bright foreground. Kitty
            // ties all three to one token, so we do too.
            base05: color(FG),
            base06: color(FG),
            base07: color(FG),
            base08: color("#FF5555"),
            base09: color("#FFB86C"),
            base0a: color("#F1FA8C"),
            base0b: color("#50FA7B"),
            base0c: color("#8BE9FD"),
            base0d: color("#BD93F9"),
            base0e: color("#FF79C6"),
            base0f: color("#FF5555"),
        },
        palettes: Palettes::default(),
        colors: Colors {
            surface: color("#282A36"),
            on_surface: color(FG),
            surface_variant: color("#44475A"),
            on_surface_variant: color(FG),
            surface_container_highest: color("#6272A4"),
            surface_container_high: color("#44475A"),
            surface_container: color("#44475A"),
            surface_container_low: color("#282A36"),
            surface_container_lowest: color("#282A36"),
            inverse_surface: color(FG),
            inverse_on_surface: color("#282A36"),
            surface_tint: color("#BD93F9"),
            primary: color("#BD93F9"),
            on_primary: color("#282A36"),
            primary_container: color("#44475A"),
            on_primary_container: color("#BD93F9"),
            secondary: color("#FF79C6"),
            on_secondary: color("#282A36"),
            secondary_container: color("#44475A"),
            on_secondary_container: color("#FF79C6"),
            tertiary: color("#8BE9FD"),
            on_tertiary: color("#282A36"),
            tertiary_container: color("#44475A"),
            on_tertiary_container: color("#8BE9FD"),
            error: color("#FF5555"),
            on_error: color("#282A36"),
            error_container: color("#44475A"),
            on_error_container: color("#FF5555"),
            outline: color("#6272A4"),
            outline_variant: color("#44475A"),
            background: color("#282A36"),
            on_background: color(FG),
            inverse_primary: color("#44475A"),
            primary_fixed: color("#BD93F9"),
            primary_fixed_dim: color("#BD93F9"),
            on_primary_fixed: color("#282A36"),
            on_primary_fixed_variant: color("#282A36"),
            secondary_fixed: color("#FF79C6"),
            secondary_fixed_dim: color("#FF79C6"),
            on_secondary_fixed: color("#282A36"),
            on_secondary_fixed_variant: color("#282A36"),
            tertiary_fixed: color("#8BE9FD"),
            tertiary_fixed_dim: color("#8BE9FD"),
            on_tertiary_fixed: color("#282A36"),
            on_tertiary_fixed_variant: color("#282A36"),
            scrim: color("#282A36"),
            shadow: color("#282A36"),
            source_color: color("#BD93F9"),
            surface_bright: color("#6272A4"),
            surface_dim: color("#282A36"),
        },
    }
}
