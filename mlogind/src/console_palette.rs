//! Linux-VT colour bridge — make the bare-TTY greeter render the margo theme
//! reliably on any console.
//!
//! mlogind's theme uses 24-bit hex colours (`#282A36`, `#BD93F9`, …) which
//! `config::get_color` turns into `ratatui::Color::Rgb`. A **terminal
//! emulator** (where `mlogind --preview` runs) renders those truecolor SGR
//! sequences exactly, so preview shows the real margo palette. The **bare
//! Linux VT** (kernel fbcon / DRM-KMS console, where the real greeter runs)
//! has *no* truecolor: it owns a 16-entry palette and approximates every
//! `38;2;r;g;b` down to the nearest of those 16.
//!
//! An earlier version tried to *reprogram* console palette slots via the
//! Linux-console OSC `ESC ] P n rrggbb` and hand back `Color::Indexed(slot)`.
//! On modern DRM/KMS consoles that escape is silently ignored, so the indexed
//! colours rendered with the console's *default* palette instead — e.g. the
//! accent landed on slot 2 (default green), which is why login came out
//! "greenish" no matter what the theme said.
//!
//! Fix: don't depend on reprogramming. On the real VT we map each themed
//! `Rgb` to the **nearest of the 16 standard ANSI colours** and return it as a
//! *named* `Color` (`Black`, `LightMagenta`, …). Named colours render through
//! the console's own (boot-time) palette, so the result is deterministic and
//! correct on every console — a clean 16-colour Dracula. In preview we leave
//! truecolor untouched so the emulator still shows the exact palette.

use std::sync::atomic::{AtomicBool, Ordering};

use ratatui::style::Color;

/// True when rendering into a terminal emulator (`--preview`): keep truecolor
/// and never down-map to the 16-colour palette.
static PREVIEW: AtomicBool = AtomicBool::new(false);

/// A standard Linux-console palette entry: its RGB (for nearest-match) and the
/// named ratatui `Color` to render it as.
struct Ansi(u8, u8, u8, Color);

/// The grayscale ramp — used for genuinely achromatic theme colours so a dark
/// near-neutral background doesn't get tinted.
const ACHROMATIC: [Ansi; 4] = [
    Ansi(0, 0, 0, Color::Black),
    Ansi(85, 85, 85, Color::DarkGray),
    Ansi(170, 170, 170, Color::Gray),
    Ansi(255, 255, 255, Color::White),
];

/// The 12 chromatic console colours (normal + bright) — used for any theme
/// colour with real hue, so Dracula's blues/purples/cyans stay *coloured*
/// instead of collapsing onto the nearest gray.
const CHROMATIC: [Ansi; 12] = [
    Ansi(170, 0, 0, Color::Red),
    Ansi(0, 170, 0, Color::Green),
    Ansi(170, 85, 0, Color::Yellow),
    Ansi(0, 0, 170, Color::Blue),
    Ansi(170, 0, 170, Color::Magenta),
    Ansi(0, 170, 170, Color::Cyan),
    Ansi(255, 85, 85, Color::LightRed),
    Ansi(85, 255, 85, Color::LightGreen),
    Ansi(255, 255, 85, Color::LightYellow),
    Ansi(85, 85, 255, Color::LightBlue),
    Ansi(255, 85, 255, Color::LightMagenta),
    Ansi(85, 255, 255, Color::LightCyan),
];

/// Chroma (max − min channel) at or above which a colour is treated as having
/// real hue. `#CDD6F4` (text, chroma 39) stays achromatic → white; `#6272A4`
/// (field border, chroma 66) and the accents read as colour.
const CHROMA_THRESHOLD: i32 = 48;

/// Record whether we're rendering in preview (terminal emulator) mode.
/// Call once at startup, before the first draw.
pub fn init(preview: bool) {
    PREVIEW.store(preview, Ordering::Relaxed);
}

/// Map a resolved style colour to something the current output can show.
///
/// * preview, or a non-`Rgb` colour → returned unchanged (emulator does
///   truecolor; named colours already index the palette correctly).
/// * real VT + `Rgb` → snapped to a named ANSI colour so the console renders it
///   via its own palette (no reprogramming, deterministic on every console).
///   Achromatic colours snap to the gray ramp; anything with real hue snaps to
///   a chromatic bucket, so the dark theme keeps its blues/purples/cyans and
///   doesn't read as plain black-and-white.
pub fn map_color(color: Color) -> Color {
    let Color::Rgb(r, g, b) = color else {
        return color;
    };
    if PREVIEW.load(Ordering::Relaxed) {
        return color;
    }
    nearest_ansi(r, g, b)
}

/// Nearest named ANSI colour, gated by chroma so low-saturation colours map to
/// gray and hued colours map to a chromatic bucket.
fn nearest_ansi(r: u8, g: u8, b: u8) -> Color {
    let chroma = r.max(g).max(b) as i32 - r.min(g).min(b) as i32;
    let palette: &[Ansi] = if chroma >= CHROMA_THRESHOLD {
        &CHROMATIC
    } else {
        &ACHROMATIC
    };

    let (r, g, b) = (r as i32, g as i32, b as i32);
    let mut best = &palette[0];
    let mut best_dist = i32::MAX;
    for entry in palette {
        let (dr, dg, db) = (r - entry.0 as i32, g - entry.1 as i32, b - entry.2 as i32);
        let dist = dr * dr + dg * dg + db * db;
        if dist < best_dist {
            best_dist = dist;
            best = entry;
        }
    }
    best.3
}

/// No-op kept for call-site compatibility — we no longer reprogram the console
/// palette, so there is nothing to restore.
pub fn reset() {}
