//! Centred greeter layout — mlock's vertical stack, in terminal cells.
//!
//! Top → bottom, vertically centred as one block:
//!   • battery (absolute top-right, laptops only)
//!   • greeting line ("Good evening")
//!   • big block clock (5 rows)
//!   • date line
//!   • a rounded card holding session / username / password rows
//!   • status line
//!   • a centred row of power-control chips
//!
//! Everything is measured up-front so the block stays centred no matter the
//! terminal size; on a very short terminal it falls back to top-aligned.

use ratatui::{backend::Backend, layout::Rect, Frame};

/// Cells reserved on the left of each card row for its label ("Password").
const LABEL_W: u16 = 11;
/// Card width target; clamped to the terminal.
const CARD_W: u16 = 56;
/// Card height: 1 border + 1 pad + 3 rows + 1 pad + 1 border.
const CARD_H: u16 = 7;
const CLOCK_H: u16 = 5;

pub struct Chunks {
    pub battery: Rect,
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
    /// Centred power-control chip row.
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

impl Chunks {
    pub fn new<B: Backend>(frame: &Frame<B>) -> Self {
        let size = frame.size();
        let (fw, fh) = (size.width, size.height);

        let card_w = CARD_W.min(fw.saturating_sub(2));
        // content width spans the widest element (the clock can be ~21).
        let content_w = card_w.max(24).min(fw);

        // Heights with a single blank-row rhythm between sections.
        // greeting(1) gap(1) clock(5) gap(1) date(1) gap(1) card(7)
        // gap(1) status(1) gap(1) chips(1) = 21
        let total: u16 = 1 + 1 + CLOCK_H + 1 + 1 + 1 + CARD_H + 1 + 1 + 1 + 1;
        // Leave the top row free for the battery; top-align if too short.
        let mut y = if fh > total + 1 {
            (fh - total) / 2
        } else {
            1
        };

        let cx = centered_x(fw, content_w);
        let card_x = centered_x(fw, card_w);
        let line = |y: u16| Rect {
            x: cx,
            y,
            width: content_w,
            height: 1,
        };

        let battery = Rect {
            x: fw.saturating_sub(13),
            y: 0,
            width: 12,
            height: 1,
        };

        let greeting = line(y);
        y += 2; // greeting + gap
        let clock = Rect {
            x: cx,
            y,
            width: content_w,
            height: CLOCK_H,
        };
        y += CLOCK_H + 1;
        let date = line(y);
        y += 2; // date + gap

        let card = Rect {
            x: card_x,
            y,
            width: card_w,
            height: CARD_H,
        };
        // Card inner: skip the border (1) + one pad row, then 3 content rows.
        let inner_x = card_x + 2;
        let inner_w = card_w.saturating_sub(4);
        let value_x = inner_x + LABEL_W;
        let value_w = inner_w.saturating_sub(LABEL_W);
        let row = |ry: u16| {
            (
                Rect { x: inner_x, y: ry, width: LABEL_W, height: 1 },
                Rect { x: value_x, y: ry, width: value_w, height: 1 },
            )
        };
        let (label_session, switcher) = row(card.y + 2);
        let (label_username, username_field) = row(card.y + 3);
        let (label_password, password_field) = row(card.y + 4);

        y += CARD_H + 1;
        let status_message = line(y);
        y += 2; // status + gap
        let key_menu = Rect {
            x: 0,
            y,
            width: fw,
            height: 1,
        };

        Self {
            battery: clamp(battery, size),
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
