use crate::json_struct::{Base16, Colors, MShell, MatugenTheme, Palettes, color};

/// Kenp — the author's house theme and margo's default. A dusk palette:
/// deep twilight blue-violet surfaces, a bioluminescent teal primary, and
/// twilight-violet / sunset-amber / rose accents — calm and professional,
/// with a jewel-like glow. (Formerly shipped as "Eventide"; promoted to the
/// house theme and renamed.)
///
/// The five `surface_container_*` tiers form a monotonic elevation ladder
/// (deepest well → brightest lift) so dashboard cards, menu card-stacks and
/// control-center tiles separate visually; `outline` is bright enough to
/// read as a window border (it feeds the compositor's unfocused
/// `bordercolor`) and as an input edge.
pub fn kenp(mshell: MShell) -> MatugenTheme {
    MatugenTheme {
        mshell,
        image: String::new(),
        is_dark_mode: true,
        mode: "dark".to_string(),
        base16: Base16 {
            base00: color("#15171f"),
            base01: color("#1f2230"),
            base02: color("#252939"),
            base03: color("#5b6184"),
            base04: color("#a6acc9"),
            base05: color("#d9def0"),
            base06: color("#e6eafa"),
            base07: color("#f2f4ff"),
            base08: color("#f7768e"),
            base09: color("#f5b97a"),
            base0a: color("#e0af68"),
            base0b: color("#9ece6a"),
            base0c: color("#5ec8c5"),
            base0d: color("#8aa9f0"),
            base0e: color("#c9a0f5"),
            base0f: color("#f7768e"),
        },
        palettes: Palettes::default(),
        colors: Colors {
            surface: color("#1b1e2b"),
            on_surface: color("#d9def0"),
            surface_variant: color("#2f3346"),
            on_surface_variant: color("#a6acc9"),
            surface_container_highest: color("#3b4058"),
            surface_container_high: color("#2f3346"),
            surface_container: color("#252939"),
            surface_container_low: color("#1f2230"),
            surface_container_lowest: color("#15171f"),
            inverse_surface: color("#d9def0"),
            inverse_on_surface: color("#1b1e2b"),
            surface_tint: color("#5ec8c5"),
            primary: color("#5ec8c5"),
            on_primary: color("#10212a"),
            primary_container: color("#244a4d"),
            on_primary_container: color("#8fe3e0"),
            secondary: color("#c9a0f5"),
            on_secondary: color("#1b1e2b"),
            secondary_container: color("#3a335a"),
            on_secondary_container: color("#ddc8fb"),
            tertiary: color("#f5b97a"),
            on_tertiary: color("#2a1c0e"),
            tertiary_container: color("#4d3a24"),
            on_tertiary_container: color("#ffd9b0"),
            error: color("#f7768e"),
            on_error: color("#2a0f16"),
            error_container: color("#4d2530"),
            on_error_container: color("#ffb3c0"),
            outline: color("#5b6184"),
            outline_variant: color("#3a3f57"),
            background: color("#1b1e2b"),
            on_background: color("#d9def0"),
            inverse_primary: color("#244a4d"),
            primary_fixed: color("#5ec8c5"),
            primary_fixed_dim: color("#5ec8c5"),
            on_primary_fixed: color("#10212a"),
            on_primary_fixed_variant: color("#10212a"),
            secondary_fixed: color("#c9a0f5"),
            secondary_fixed_dim: color("#c9a0f5"),
            on_secondary_fixed: color("#1b1e2b"),
            on_secondary_fixed_variant: color("#1b1e2b"),
            tertiary_fixed: color("#f5b97a"),
            tertiary_fixed_dim: color("#f5b97a"),
            on_tertiary_fixed: color("#2a1c0e"),
            on_tertiary_fixed_variant: color("#2a1c0e"),
            scrim: color("#10121b"),
            shadow: color("#10121b"),
            source_color: color("#5ec8c5"),
            surface_bright: color("#3b4058"),
            surface_dim: color("#15171f"),
        },
    }
}
