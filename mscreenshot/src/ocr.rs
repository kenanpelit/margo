//! `mscreenshot ocr` — the pure half of the OCR pipeline.
//!
//! The capture (grim + region selector) and the process plumbing live in
//! `main.rs`; everything here is a decision over strings so it can be
//! tested against real `tesseract` output without a session.
//!
//! Language precedence: `--lang` flag, then `MSCREENSHOT_OCR_LANG`, then
//! the built-in default ([`PREFERRED_DEFAULT`], `eng+tur`) narrowed to the
//! tessdata packs that are actually installed. An explicit request naming
//! a missing pack is an error rather than a silent fallback, because
//! tesseract itself only warns on stderr and then recognises with `eng`
//! anyway (exit 0) — the user would get worse text with no hint why.

/// Languages tried when the user set nothing, in `-l` order. Each one is
/// used only when its tessdata pack is installed; if none are, `eng`.
pub const PREFERRED_DEFAULT: &[&str] = &["eng", "tur"];

/// tesseract `--psm` default: "assume a single uniform block of text",
/// the right shape for a dragged region (`3`, fully automatic, splits a
/// small selection into bogus columns).
pub const DEFAULT_PSM: u8 = 6;

/// Notification preview length, in characters (not bytes — a Turkish
/// `ş` or `ğ` straddling a byte cut would otherwise be split).
pub const PREVIEW_CHARS: usize = 160;

/// Installed tessdata languages from `tesseract --list-langs` stdout.
/// tesseract 5 prints a `List of available languages in "…" (N):` header
/// followed by one code per line; `osd` (orientation detection) is not a
/// recognition language and is dropped.
pub fn parse_list_langs(out: &str) -> Vec<String> {
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("List of available languages"))
        .filter(|l| l.chars().all(is_lang_char))
        .filter(|l| *l != "osd")
        .map(str::to_string)
        .collect()
}

fn is_lang_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Split a `-l` style spec (`eng+tur`, `eng, tur`, `eng tur`) into codes,
/// dropping anything that is not a plausible tessdata name.
pub fn split_langs(spec: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tok in spec.split(|c: char| c == '+' || c == ',' || c.is_whitespace()) {
        if !tok.is_empty() && tok.chars().all(is_lang_char) && !out.iter().any(|t| t == tok) {
            out.push(tok.to_string());
        }
    }
    out
}

/// The `-l` value to run with, or the list of requested codes whose
/// tessdata is missing.
///
/// `installed` empty means "could not list" (old tesseract, odd
/// TESSDATA_PREFIX): an explicit request is then trusted as-is and the
/// default degrades to plain `eng`.
pub fn resolve_langs(requested: Option<&str>, installed: &[String]) -> Result<String, Vec<String>> {
    let requested = requested.map(split_langs).unwrap_or_default();
    if !requested.is_empty() {
        if installed.is_empty() {
            return Ok(requested.join("+"));
        }
        let missing: Vec<String> = requested
            .iter()
            .filter(|l| !installed.contains(l))
            .cloned()
            .collect();
        return if missing.is_empty() {
            Ok(requested.join("+"))
        } else {
            Err(missing)
        };
    }
    let picked: Vec<&str> = PREFERRED_DEFAULT
        .iter()
        .copied()
        .filter(|l| installed.iter().any(|i| i == l))
        .collect();
    if picked.is_empty() {
        Ok("eng".to_string())
    } else {
        Ok(picked.join("+"))
    }
}

/// Arch package that provides the tessdata for `lang`.
pub fn tessdata_package(lang: &str) -> String {
    format!("tesseract-data-{lang}")
}

/// `--psm` from a flag / env string; out-of-range or junk → default.
pub fn resolve_psm(value: Option<&str>) -> u8 {
    value
        .and_then(|v| v.trim().parse::<u8>().ok())
        .filter(|p| *p <= 13)
        .unwrap_or(DEFAULT_PSM)
}

/// tesseract argv (program name excluded): read `image`, write the text
/// to stdout.
pub fn tesseract_argv(image: &str, langs: &str, psm: u8) -> Vec<String> {
    vec![
        image.to_string(),
        "stdout".to_string(),
        "-l".to_string(),
        langs.to_string(),
        "--psm".to_string(),
        psm.to_string(),
    ]
}

/// Tidy raw tesseract output for the clipboard: drop the trailing form
/// feed tesseract appends per page, normalise CRLF, strip trailing
/// whitespace on every line, collapse blank-line runs to one, trim the
/// whole thing.
pub fn clean_text(raw: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut blank_run = false;
    for line in raw.split('\n') {
        let line = line.trim_end_matches(['\r', '\u{c}', ' ', '\t']);
        let line = line.trim_start_matches('\u{c}');
        if line.trim().is_empty() {
            if !blank_run && !out.is_empty() {
                out.push("");
            }
            blank_run = true;
        } else {
            out.push(line);
            blank_run = false;
        }
    }
    while out.last() == Some(&"") {
        out.pop();
    }
    out.join("\n")
}

/// First `max` characters of `text`, with `…` when cut.
pub fn preview(text: &str, max: usize) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        format!("{}…", head.trim_end())
    } else {
        head
    }
}

/// Escape the three characters the notification spec's body markup
/// interprets (mshell's daemon advertises `body-markup`), so recognised
/// text like `a < b` or `R&D` shows literally instead of vanishing.
pub fn escape_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured verbatim from `tesseract --list-langs` (tesseract 5) on
    /// the owner's machine after `pacman -S tesseract-data-tur`.
    const LIST_LANGS_ENG_TUR: &str =
        "List of available languages in \"/usr/share/tessdata/\" (3):\neng\nosd\ntur\n";
    /// Same, before tesseract-data-tur was installed.
    const LIST_LANGS_ENG: &str =
        "List of available languages in \"/usr/share/tessdata/\" (2):\neng\nosd\n";

    fn installed(out: &str) -> Vec<String> {
        parse_list_langs(out)
    }

    #[test]
    fn list_langs_drops_header_and_osd() {
        assert_eq!(installed(LIST_LANGS_ENG_TUR), ["eng", "tur"]);
        assert_eq!(installed(LIST_LANGS_ENG), ["eng"]);
        assert!(installed("").is_empty());
        assert_eq!(installed("chi_sim\nscript/Latin\n"), ["chi_sim"]);
    }

    #[test]
    fn default_is_eng_plus_tur_when_both_are_installed() {
        assert_eq!(
            resolve_langs(None, &installed(LIST_LANGS_ENG_TUR)),
            Ok("eng+tur".into())
        );
    }

    #[test]
    fn default_narrows_to_what_is_installed() {
        assert_eq!(
            resolve_langs(None, &installed(LIST_LANGS_ENG)),
            Ok("eng".into())
        );
        let only_tur = vec!["tur".to_string()];
        assert_eq!(resolve_langs(None, &only_tur), Ok("tur".into()));
        assert_eq!(resolve_langs(None, &[]), Ok("eng".into()));
        assert_eq!(resolve_langs(Some("  "), &[]), Ok("eng".into()));
    }

    #[test]
    fn explicit_request_is_honoured_and_normalised() {
        let inst = installed(LIST_LANGS_ENG_TUR);
        assert_eq!(resolve_langs(Some("tur"), &inst), Ok("tur".into()));
        assert_eq!(resolve_langs(Some("tur, eng"), &inst), Ok("tur+eng".into()));
        assert_eq!(resolve_langs(Some("eng+eng"), &inst), Ok("eng".into()));
    }

    #[test]
    fn explicit_request_for_a_missing_pack_is_an_error() {
        let inst = installed(LIST_LANGS_ENG);
        assert_eq!(
            resolve_langs(Some("eng+tur"), &inst),
            Err(vec!["tur".to_string()])
        );
        assert_eq!(tessdata_package("tur"), "tesseract-data-tur");
        // Can't list → trust the user.
        assert_eq!(resolve_langs(Some("deu"), &[]), Ok("deu".into()));
    }

    #[test]
    fn junk_in_the_language_spec_is_dropped() {
        assert_eq!(split_langs("eng+../../x;rm"), ["eng"]);
        assert_eq!(split_langs("eng tur\tdeu"), ["eng", "tur", "deu"]);
    }

    #[test]
    fn psm_falls_back_on_junk() {
        assert_eq!(resolve_psm(None), 6);
        assert_eq!(resolve_psm(Some("7")), 7);
        assert_eq!(resolve_psm(Some(" 3 ")), 3);
        assert_eq!(resolve_psm(Some("14")), 6);
        assert_eq!(resolve_psm(Some("block")), 6);
    }

    #[test]
    fn argv_shape() {
        assert_eq!(
            tesseract_argv("/run/x.png", "eng+tur", 6),
            ["/run/x.png", "stdout", "-l", "eng+tur", "--psm", "6"]
        );
    }

    #[test]
    fn clean_text_strips_form_feed_and_blank_runs() {
        // Real tesseract stdout ends with "\n\u{c}" per page.
        let raw = "Merhaba dünya  \r\n\n\n\nikinci satır\t\n\u{c}";
        assert_eq!(clean_text(raw), "Merhaba dünya\n\nikinci satır");
        assert_eq!(clean_text("\n \n\u{c}"), "");
        assert_eq!(clean_text("\n\nx"), "x");
    }

    #[test]
    fn preview_cuts_on_characters_not_bytes() {
        let text = "ş".repeat(200);
        let p = preview(&text, 160);
        assert_eq!(p.chars().count(), 161);
        assert!(p.ends_with('…'));
        assert_eq!(preview("kısa", 160), "kısa");
        assert_eq!(preview("abc", 3), "abc");
    }

    #[test]
    fn markup_is_escaped() {
        assert_eq!(
            escape_markup("R&D <b>x</b>"),
            "R&amp;D &lt;b&gt;x&lt;/b&gt;"
        );
    }
}
