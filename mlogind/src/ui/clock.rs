//! A tiny 5-row block font for the greeter's centred clock.
//!
//! mlock renders a 110 pt pango clock; on a TTY we can't, so we draw the
//! same focal HH:MM with block glyphs — the closest the terminal gets to
//! mlock's "big clock" centrepiece. Digits are a fixed 4 cells wide, the
//! colon 1 cell, joined by a single-cell gap, so every time string lines
//! up regardless of the digits in it.

/// 5 rows for one glyph. Unknown chars render blank (4 wide).
fn glyph(c: char) -> [&'static str; 5] {
    match c {
        '0' => ["████", "█  █", "█  █", "█  █", "████"],
        '1' => ["  █ ", " ██ ", "  █ ", "  █ ", " ███"],
        '2' => ["████", "   █", "████", "█   ", "████"],
        '3' => ["████", "   █", " ███", "   █", "████"],
        '4' => ["█  █", "█  █", "████", "   █", "   █"],
        '5' => ["████", "█   ", "████", "   █", "████"],
        '6' => ["████", "█   ", "████", "█  █", "████"],
        '7' => ["████", "   █", "  █ ", " █  ", " █  "],
        '8' => ["████", "█  █", "████", "█  █", "████"],
        '9' => ["████", "█  █", "████", "   █", "████"],
        ':' => [" ", "█", " ", "█", " "],
        _ => ["    ", "    ", "    ", "    ", "    "],
    }
}

/// Render `s` (e.g. "21:04") to 5 rows of block art, glyphs separated by a
/// single space column.
pub fn big_time(s: &str) -> [String; 5] {
    let mut rows: [String; 5] = Default::default();
    for (i, ch) in s.chars().enumerate() {
        let g = glyph(ch);
        for (r, row) in rows.iter_mut().enumerate() {
            if i > 0 {
                row.push(' ');
            }
            row.push_str(g[r]);
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_has_equal_width() {
        let rows = big_time("21:04");
        let w = rows[0].chars().count();
        for row in &rows {
            assert_eq!(row.chars().count(), w, "row widths must match for centring");
        }
        // 4+1+4 +1+ 1 +1+ 4+1+4  = 5 glyphs (2,1,:,0,4) with gaps.
        assert_eq!(w, 4 + 1 + 4 + 1 + 1 + 1 + 4 + 1 + 4);
    }

    #[test]
    fn always_five_rows() {
        assert_eq!(big_time("00:00").len(), 5);
        assert!(big_time("").iter().all(|r| r.is_empty()));
    }
}
