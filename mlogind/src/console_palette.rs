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

/// The 16 standard Linux-console palette RGBs, indexed 0..16, used purely to
/// find which named ANSI colour a themed RGB is closest to.
const ANSI16: [(u8, u8, u8); 16] = [
    (0, 0, 0),       // 0  black
    (170, 0, 0),     // 1  red
    (0, 170, 0),     // 2  green
    (170, 85, 0),    // 3  yellow/brown
    (0, 0, 170),     // 4  blue
    (170, 0, 170),   // 5  magenta
    (0, 170, 170),   // 6  cyan
    (170, 170, 170), // 7  white/gray
    (85, 85, 85),    // 8  bright black
    (255, 85, 85),   // 9  bright red
    (85, 255, 85),   // 10 bright green
    (255, 255, 85),  // 11 bright yellow
    (85, 85, 255),   // 12 bright blue
    (255, 85, 255),  // 13 bright magenta
    (85, 255, 255),  // 14 bright cyan
    (255, 255, 255), // 15 bright white
];

/// Record whether we're rendering in preview (terminal emulator) mode.
/// Call once at startup, before the first draw.
pub fn init(preview: bool) {
    PREVIEW.store(preview, Ordering::Relaxed);
}

/// Map a resolved style colour to something the current output can show.
///
/// * preview, or a non-`Rgb` colour → returned unchanged (emulator does
///   truecolor; named colours already index the palette correctly).
/// * real VT + `Rgb` → snapped to the nearest of the 16 standard ANSI colours,
///   returned as a *named* `Color` so the console renders it via its own
///   palette (no reprogramming, deterministic on every console).
pub fn map_color(color: Color) -> Color {
    let Color::Rgb(r, g, b) = color else {
        return color;
    };
    if PREVIEW.load(Ordering::Relaxed) {
        return color;
    }
    nearest_ansi16(r, g, b)
}

/// Nearest of the 16 standard ANSI colours, as a named ratatui `Color`.
fn nearest_ansi16(r: u8, g: u8, b: u8) -> Color {
    let (r, g, b) = (r as i32, g as i32, b as i32);
    let mut best = 0usize;
    let mut best_dist = i32::MAX;
    for (i, &(pr, pg, pb)) in ANSI16.iter().enumerate() {
        let (dr, dg, db) = (r - pr as i32, g - pg as i32, b - pb as i32);
        let dist = dr * dr + dg * dg + db * db;
        if dist < best_dist {
            best_dist = dist;
            best = i;
        }
    }
    match best {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        7 => Color::Gray,
        8 => Color::DarkGray,
        9 => Color::LightRed,
        10 => Color::LightGreen,
        11 => Color::LightYellow,
        12 => Color::LightBlue,
        13 => Color::LightMagenta,
        14 => Color::LightCyan,
        _ => Color::White,
    }
}

/// No-op kept for call-site compatibility — we no longer reprogram the console
/// palette, so there is nothing to restore.
pub fn reset() {}
