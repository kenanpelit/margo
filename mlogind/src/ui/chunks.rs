//! Centred, height-adaptive greeter layout — mlock's vertical stack in
//! terminal cells.
//!
//! The previous version used a fixed-height centred block; on a short VT
//! (the bare console often has fewer rows than a terminal-emulator
//! `--preview`) the bottom of the block — the power-control chips — was
//! clipped away, so the F-keys vanished on the real login while preview
//! looked fine. This version instead:
//!   • **pins the power chips to the bottom row** (always visible),
//!   • **keeps the clock + credential card no matter what**, and
//!   • **drops the optional greeting / date / status lines** (in that
//!     order) when the terminal is too short to fit them.
//!
//! Top → bottom: greeting · big clock · date · rounded credential card
//! (session / user / password) · status · power-control chips.

use ratatui::{Frame, backend::Backend, layout::Rect};

/// Cells reserved on the left of each card row for its label ("Password").
const LABEL_W: u16 = 12;
/// Card width target; clamped to the terminal. Wide so session names and
/// the password have room to breathe.
const CARD_W_MAX: u16 = 72;
const CARD_W_MIN: u16 = 40;
/// Card height: 1 border + 1 pad + 3 rows + 1 pad + 1 border.
const CARD_H: u16 = 7;
const CLOCK_H: u16 = 5;

/// Vertical placement of the whole login block within the leftover space:
/// the free rows above/below the block are split with this top:bottom weight
/// instead of centring (which would be 50:50). A lighter top weight lifts the
/// block above the geometric centre, so it reads higher on a tall monitor.
const SPACE_ABOVE: u32 = 15;
const SPACE_BELOW: u32 = 100;

pub struct Chunks {
    pub greeting: Rect,
    pub clock: Rect,
    pub date: Rect,
    /// Outer rounded card (border drawn here).
    pub card: Rect,
    pub label_session: Rect,
    pub label_username: Rect,
    pub label_password: Rect,
    pub switcher: Rect,
    pub username_field: Rect,
    pub password_field: Rect,
    pub status_message: Rect,
    /// Centred power-control chip row, pinned near the bottom.
    pub key_menu: Rect,
}

fn centered_x(frame_w: u16, w: u16) -> u16 {
    frame_w.saturating_sub(w) / 2
}

/// Clip a rect to the frame so a short/narrow terminal can never push a
/// widget past the buffer (ratatui panics on out-of-bounds cells, and a
/// login manager must not crash). Fully-offscreen rects become zero-area
/// and simply render nothing.
fn clamp(r: Rect, bounds: Rect) -> Rect {
    let x = r.x.min(bounds.width);
    let y = r.y.min(bounds.height);
    let right = r.x.saturating_add(r.width).min(bounds.width);
    let bottom = r.y.saturating_add(r.height).min(bounds.height);
    Rect {
        x,
        y,
        width: right.saturating_sub(x),
        height: bottom.saturating_sub(y),
    }
}

const ZERO: Rect = Rect {
    x: 0,
    y: 0,
    width: 0,
    height: 0,
};

impl Chunks {
    pub fn new<B: Backend>(frame: &Frame<B>) -> Self {
        let size = frame.size();
        let (fw, fh) = (size.width, size.height);

        let card_w = fw.saturating_sub(6).clamp(CARD_W_MIN, CARD_W_MAX).min(fw);
        let content_w = card_w.max(24).min(fw);

        // Chips pinned one row up from the bottom (so they sit *inside* a
        // full-screen background border when one is configured; harmless
        // padding when not), with a blank row above them.
        let body_top = 1u16;
        // Usable rows: leave the top + bottom rows as a margin (a configured
        // full-screen background border lives there).
        let avail = fh.saturating_sub(2);

        // The card AND the power chips are mandatory and kept *together* — the
        // chips sit just below the card, both inside one centred block — so
        // wherever the card is visible the F-keys are too. (Pinning the chips
        // to the very bottom row dropped them on a cropped / overscanned
        // secondary monitor while the centred card still showed.) Everything
        // else is added only while it fits, most-important-first, so a short
        // console drops decorations rather than pushing anything off-screen.
        const GAP: u16 = 1;
        let mut used = CARD_H + GAP + 1; // card + gap + chips row
        let show_clock = used + CLOCK_H + GAP <= avail;
        if show_clock {
            used += CLOCK_H + GAP;
        }
        let show_status = used + 2 <= avail; // gap + status, between card & chips
        if show_status {
            used += 2;
        }
        let show_date = show_clock && used + 2 <= avail;
        if show_date {
            used += 2;
        }
        let show_greeting = show_clock && used + 2 <= avail;
        if show_greeting {
            used += 2;
        }

        let cx = centered_x(fw, content_w);
        let card_x = centered_x(fw, card_w);
        let line = |y: u16| Rect {
            x: cx,
            y,
            width: content_w,
            height: 1,
        };

        let free = avail.saturating_sub(used) as u32;
        let top_pad = (free * SPACE_ABOVE / (SPACE_ABOVE + SPACE_BELOW)) as u16;
        let mut y = body_top + top_pad;

        let greeting = if show_greeting {
            let r = line(y);
            y += 2;
            r
        } else {
            ZERO
        };

        let clock = if show_clock {
            let r = Rect {
                x: cx,
                y,
                width: content_w,
                height: CLOCK_H,
            };
            y += CLOCK_H + GAP;
            r
        } else {
            ZERO
        };

        let date = if show_date {
            let r = line(y);
            y += 2;
            r
        } else {
            ZERO
        };

        let card = Rect {
            x: card_x,
            y,
            width: card_w,
            height: CARD_H,
        };
        y += CARD_H;

        // Power chips sit directly below the card (one gap), so they stay
        // with it on every monitor; the transient status line goes below
        // them rather than wedging an always-reserved row in between.
        y += GAP;
        let key_menu = Rect {
            x: 0,
            y,
            width: fw,
            height: 1,
        };
        y += 1;

        let status_message = if show_status {
            y += GAP;
            line(y)
        } else {
            ZERO
        };

        // Card inner: skip the border (1) + one pad row, then 3 content rows.
        let inner_x = card.x + 2;
        let inner_w = card.width.saturating_sub(4);
        let value_x = inner_x + LABEL_W;
        let value_w = inner_w.saturating_sub(LABEL_W);
        let row = |ry: u16| {
            (
                Rect {
                    x: inner_x,
                    y: ry,
                    width: LABEL_W,
                    height: 1,
                },
                Rect {
                    x: value_x,
                    y: ry,
                    width: value_w,
                    height: 1,
                },
            )
        };
        // The session value is drawn inline by the greeter (truncated to fit),
        // so it just takes the full value area like the input rows.
        let (label_session, switcher) = row(card.y + 2);
        let (label_username, username_field) = row(card.y + 3);
        let (label_password, password_field) = row(card.y + 4);

        Self {
            greeting: clamp(greeting, size),
            clock: clamp(clock, size),
            date: clamp(date, size),
            card: clamp(card, size),
            label_session: clamp(label_session, size),
            label_username: clamp(label_username, size),
            label_password: clamp(label_password, size),
            switcher: clamp(switcher, size),
            username_field: clamp(username_field, size),
            password_field: clamp(password_field, size),
            status_message: clamp(status_message, size),
            key_menu: clamp(key_menu, size),
        }
    }
}
