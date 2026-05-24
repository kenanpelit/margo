//! Linux-VT colour bridge — make the bare-TTY greeter match `--preview`.
//!
//! mlogind's theme uses 24-bit hex colours (`#282A36`, `#BD93F9`, …) which
//! `config::get_color` turns into `ratatui::Color::Rgb`. A **terminal
//! emulator** (where `mlogind --preview` runs) renders those truecolor SGR
//! sequences exactly, so preview shows the real margo palette. The **bare
//! Linux VT** (kernel fbcon, where the real greeter runs) has *no* truecolor:
//! it owns a 16-entry palette and approximates every `38;2;r;g;b` down to the
//! nearest of those 16. That mismatch is why login looks nothing like
//! `--preview` — same config, different output colour depth.
//!
//! Fix: on the real VT we reprogram a handful of console palette entries to
//! the theme's *actual* RGB values via the Linux-console OSC
//! `ESC ] P n rrggbb` (see `console_codes(4)`), then hand back
//! `Color::Indexed(slot)` so the kernel uses that exact entry instead of an
//! approximation. In preview this whole module is a no-op — the emulator
//! keeps its truecolor.
//!
//! Slot policy: we assign first-come into [`USABLE_SLOTS`], which deliberately
//! **skips slots 1 (red) and 3 (yellow)** so `status_message`'s named
//! `Color::Red` / `Color::Yellow` keep working, and **lists the low slots
//! first** so the window background (drawn before anything else) lands on a
//! low slot — bright-slot *backgrounds* can blink on fbcon, low ones never do.

use std::collections::HashMap;
use std::io::Write;
use std::sync::{Mutex, OnceLock};

use ratatui::style::Color;

/// Console palette entries we may repurpose, in assignment order.
///
/// Skips 1 (red) and 3 (yellow) — reserved for `status_message`'s error /
/// warning text. Low slots (0,2,4–7) come first so the first colour seen each
/// frame (the background) gets a non-blinking entry; the bright half (8–15) is
/// overflow for richer themes (e.g. after `sync-theme` pulls a wallpaper
/// palette with more distinct colours).
const USABLE_SLOTS: [u8; 14] = [0, 2, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

struct PaletteState {
    /// True when rendering into a terminal emulator (`--preview`); then we
    /// leave truecolor alone and never emit Linux-VT escapes.
    preview: bool,
    /// Distinct RGB → assigned console slot.
    slots: HashMap<(u8, u8, u8), u8>,
    /// Index into `USABLE_SLOTS` of the next free slot.
    next: usize,
}

static STATE: OnceLock<Mutex<PaletteState>> = OnceLock::new();

fn state() -> &'static Mutex<PaletteState> {
    STATE.get_or_init(|| {
        Mutex::new(PaletteState {
            preview: false,
            slots: HashMap::new(),
            next: 0,
        })
    })
}

/// Record whether we're rendering in preview (terminal emulator) mode.
/// Call once at startup, before the first draw.
pub fn init(preview: bool) {
    state().lock().unwrap().preview = preview;
}

/// Map a resolved style colour to something the current output can show.
///
/// * preview, or a non-`Rgb` colour → returned unchanged (emulator does
///   truecolor; named colours already index the palette).
/// * real VT + `Rgb` → reprogram a console slot to that RGB and return
///   `Indexed(slot)` so the kernel renders it verbatim. Once every usable
///   slot is taken, fall back to `Rgb` and let the kernel approximate.
pub fn map_color(color: Color) -> Color {
    let Color::Rgb(r, g, b) = color else {
        return color;
    };

    let mut st = state().lock().unwrap();
    if st.preview {
        return color;
    }

    if let Some(&slot) = st.slots.get(&(r, g, b)) {
        return Color::Indexed(slot);
    }

    let Some(&slot) = USABLE_SLOTS.get(st.next) else {
        // Out of slots — let the kernel approximate this one to 16 colours.
        return color;
    };
    st.next += 1;
    st.slots.insert((r, g, b), slot);

    // Linux-console palette-set: ESC ] P <slot-nibble> <rrggbb>, 7 hex digits,
    // no string terminator. Written straight to stdout from inside the draw
    // closure — ratatui buffers the frame and flushes *after* the closure, so
    // this lands before the pixels that use it.
    let _ = write!(std::io::stdout(), "\x1b]P{slot:X}{r:02X}{g:02X}{b:02X}");
    let _ = std::io::stdout().flush();

    Color::Indexed(slot)
}

/// Restore the console's default palette and forget our slot assignments, so a
/// later redraw reprograms from scratch. No-op in preview.
pub fn reset() {
    let mut st = state().lock().unwrap();
    if st.preview {
        return;
    }
    st.slots.clear();
    st.next = 0;
    let _ = write!(std::io::stdout(), "\x1b]R");
    let _ = std::io::stdout().flush();
}
