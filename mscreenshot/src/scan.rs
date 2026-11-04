//! `mscreenshot qr` — the pure half of the QR / barcode scan.
//!
//! `zbarimg -q` (without `--raw`) prints one `SYMBOLOGY:data` record per
//! decoded symbol. A symbol's data may itself contain newlines, so a line
//! that does not start with a known symbology prefix continues the
//! previous record — `--raw` would lose that boundary entirely (two codes
//! and one two-line code print the same).

/// zbar symbology names as `zbar_get_symbol_name` prints them.
const SYMBOLOGIES: &[&str] = &[
    "QR-Code",
    "SQ-Code",
    "EAN-2",
    "EAN-5",
    "EAN-8",
    "EAN-13",
    "UPC-A",
    "UPC-E",
    "ISBN-10",
    "ISBN-13",
    "COMPOSITE",
    "I2/5",
    "DataBar",
    "DataBar-Exp",
    "Codabar",
    "CODE-39",
    "CODE-93",
    "CODE-128",
    "PDF417",
];

/// zbarimg's exit status when it scanned fine but found nothing.
pub const ZBAR_NOTHING_FOUND: i32 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub kind: String,
    pub data: String,
}

fn split_record(line: &str) -> Option<(&str, &str)> {
    let (kind, data) = line.split_once(':')?;
    SYMBOLOGIES.contains(&kind).then_some((kind, data))
}

/// Decoded symbols from `zbarimg -q` stdout, in output order, with exact
/// duplicates (the same code seen twice) dropped.
pub fn parse_zbarimg(out: &str) -> Vec<Symbol> {
    let mut symbols: Vec<Symbol> = Vec::new();
    for line in out.split('\n') {
        if let Some((kind, data)) = split_record(line) {
            symbols.push(Symbol {
                kind: kind.to_string(),
                data: data.to_string(),
            });
        } else if let Some(last) = symbols.last_mut() {
            last.data.push('\n');
            last.data.push_str(line);
        }
    }
    for s in &mut symbols {
        // The record terminator, not part of the data.
        let trimmed = s.data.trim_end_matches(['\n', '\r']).len();
        s.data.truncate(trimmed);
    }
    let mut out: Vec<Symbol> = Vec::new();
    for s in symbols {
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// What goes on the clipboard: every decoded payload, one per line.
pub fn clipboard_text(symbols: &[Symbol]) -> String {
    symbols
        .iter()
        .map(|s| s.data.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The URL to offer an "Open" action for: only when exactly one symbol
/// was decoded and it is a plain `http(s)://` URL. Other schemes found in
/// codes (`file:`, `javascript:`, custom handlers, `WIFI:` configs) are
/// copied but never handed to `xdg-open` — a QR code is untrusted input.
pub fn openable_url(symbols: &[Symbol]) -> Option<&str> {
    let [only] = symbols else {
        return None;
    };
    let url = only.data.trim();
    let lower = url.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))?;
    let host_ok = rest
        .split(['/', '?', '#'])
        .next()
        .is_some_and(|h| !h.is_empty());
    (host_ok && !url.chars().any(|c| c.is_whitespace() || c.is_control())).then_some(url)
}

/// Notification title, e.g. `QR-Code` or `2 codes`.
pub fn title(symbols: &[Symbol]) -> String {
    match symbols {
        [one] => format!("{} copied", one.kind),
        many => format!("{} codes copied", many.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from `zbarimg -q` 0.23.95 on qrencode-generated images.
    const ONE_URL: &str = "QR-Code:https://example.org/a?b=1\n";
    const MULTILINE: &str = "QR-Code:line one\nline two\n";
    const TWO_CODES: &str = "QR-Code:line one\nline two\nQR-Code:https://example.org/a?b=1\n";

    fn sym(kind: &str, data: &str) -> Symbol {
        Symbol {
            kind: kind.into(),
            data: data.into(),
        }
    }

    #[test]
    fn single_code() {
        assert_eq!(
            parse_zbarimg(ONE_URL),
            [sym("QR-Code", "https://example.org/a?b=1")]
        );
    }

    #[test]
    fn multiline_payload_stays_one_symbol() {
        assert_eq!(
            parse_zbarimg(MULTILINE),
            [sym("QR-Code", "line one\nline two")]
        );
    }

    #[test]
    fn two_codes_keep_their_boundary() {
        let s = parse_zbarimg(TWO_CODES);
        assert_eq!(
            s,
            [
                sym("QR-Code", "line one\nline two"),
                sym("QR-Code", "https://example.org/a?b=1")
            ]
        );
        assert_eq!(
            clipboard_text(&s),
            "line one\nline two\nhttps://example.org/a?b=1"
        );
        assert_eq!(title(&s), "2 codes copied");
        assert_eq!(openable_url(&s), None, "ambiguous with two codes");
    }

    #[test]
    fn barcodes_and_colons_in_data() {
        let s = parse_zbarimg("EAN-13:5901234123457\nQR-Code:WIFI:S:home;T:WPA;P:x;;\n");
        assert_eq!(
            s,
            [
                sym("EAN-13", "5901234123457"),
                sym("QR-Code", "WIFI:S:home;T:WPA;P:x;;")
            ]
        );
        // A data line that merely contains a colon is a continuation.
        let s = parse_zbarimg("QR-Code:a\nnote: b\n");
        assert_eq!(s, [sym("QR-Code", "a\nnote: b")]);
    }

    #[test]
    fn empty_and_duplicate_output() {
        assert!(parse_zbarimg("").is_empty());
        assert!(parse_zbarimg("stray\n").is_empty());
        assert_eq!(
            parse_zbarimg("QR-Code:x\nQR-Code:x\n"),
            [sym("QR-Code", "x")]
        );
    }

    #[test]
    fn only_plain_http_urls_are_openable() {
        let one = |d: &str| vec![sym("QR-Code", d)];
        assert_eq!(
            openable_url(&one("https://example.org/a?b=1")),
            Some("https://example.org/a?b=1")
        );
        assert_eq!(openable_url(&one("HTTP://x.y")), Some("HTTP://x.y"));
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "https://",
            "https:///path",
            "https://a b",
            "see https://x.y",
            "WIFI:S:x;;",
        ] {
            assert_eq!(openable_url(&one(bad)), None, "{bad}");
        }
        assert_eq!(title(&one("x")), "QR-Code copied");
    }
}
