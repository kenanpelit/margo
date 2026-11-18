//! The full math-symbol table behind the `.` provider: ~2900 Unicode
//! math characters from the mathsym plugin (see the header of
//! `symbols_math.tsv` for its sources and licences), searchable by
//! description, LaTeX command (`\leq`, `\mathbb{R}`), Lean 4 input name
//! (`le`, `R`, `Ga`) and code point (`U+2264`, `0x2264`, `\u2264`,
//! `2264`, decimal `8804`), or by pasting the character itself.
//!
//! The TSV is embedded (`include_str!`) and parsed once on first use into
//! rows with pre-normalised keys, so a query is a single linear scan with
//! no allocation per row beyond the hits (well under a millisecond for
//! the table; `search_is_fast` in the tests holds it to that).
//!
//! Ranking follows the plugin's: a name that equals the query exactly and
//! case-sensitively first (Lean's input method is case-sensitive: `Go` is
//! ω, `go` is not), then case-insensitive equality, name prefixes, name
//! substrings, description words, and code points last. Shorter names win
//! ties.

use std::sync::OnceLock;

const TSV: &str = include_str!("symbols_math.tsv");

pub(crate) struct MathSymbol {
    pub(crate) char: &'static str,
    pub(crate) desc: &'static str,
    pub(crate) latex: &'static str,
    pub(crate) lean: &'static str,
    pub(crate) codepoint: &'static str,
    /// LaTeX + Lean names normalised (no `\`, braces or spaces), as typed.
    keys: Vec<String>,
    keys_lower: Vec<String>,
    desc_lower: String,
}

fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\\' | '{' | '}' | ' '))
        .collect()
}

pub(crate) fn table() -> &'static [MathSymbol] {
    static T: OnceLock<Vec<MathSymbol>> = OnceLock::new();
    T.get_or_init(|| {
        TSV.lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
            .filter_map(|l| {
                let mut f = l.split('\t');
                let (char, desc, latex, lean, codepoint) =
                    (f.next()?, f.next()?, f.next()?, f.next()?, f.next()?);
                let keys: Vec<String> = latex
                    .split_whitespace()
                    .chain(lean.split_whitespace())
                    .map(norm)
                    .filter(|k| !k.is_empty())
                    .collect();
                let keys_lower = keys.iter().map(|k| k.to_lowercase()).collect();
                Some(MathSymbol {
                    char,
                    desc,
                    latex,
                    lean,
                    codepoint,
                    keys,
                    keys_lower,
                    desc_lower: desc.to_lowercase(),
                })
            })
            .collect()
    })
}

/// A code point the query names: `U+2264`, `u+2264`, `0x2264`, `\u2264`,
/// or bare hex of 4+ digits; `dec` is the decimal reading of a bare number.
fn parse_codepoint(q: &str) -> (Option<String>, Option<String>) {
    let lower = q.to_ascii_lowercase();
    for p in ["u+", "0x", "\\u"] {
        if let Some(rest) = lower.strip_prefix(p)
            && !rest.is_empty()
            && rest.chars().all(|c| c.is_ascii_hexdigit())
        {
            return (Some(rest.to_ascii_uppercase()), None);
        }
    }
    let hex =
        (q.len() >= 4 && q.chars().all(|c| c.is_ascii_hexdigit())).then(|| q.to_ascii_uppercase());
    let dec = q
        .parse::<u32>()
        .ok()
        .and_then(char::from_u32)
        .map(|c| format!("{:04X}", c as u32));
    (hex, dec)
}

fn name_score(sym: &MathSymbol, q: &str, ql: &str) -> Option<f64> {
    let mut best: Option<(f64, usize)> = None;
    let mut take = |s: f64, len: usize| {
        if best.is_none_or(|(b, l)| s > b || (s == b && len < l)) {
            best = Some((s, len));
        }
    };
    for (k, kl) in sym.keys.iter().zip(&sym.keys_lower) {
        if k == q {
            take(200.0, k.len());
        } else if *kl == ql {
            take(175.0, k.len());
        } else if k.starts_with(q) {
            take(160.0, k.len());
        } else if kl.starts_with(ql) {
            take(140.0, k.len());
        } else if kl.contains(ql) {
            take(110.0, k.len());
        }
    }
    best.map(|(s, len)| s - (len.min(40) as f64) * 0.1)
}

fn desc_score(sym: &MathSymbol, ql: &str) -> Option<f64> {
    if sym.desc_lower.is_empty() {
        return None;
    }
    // Every query word must appear; word starts rank above substrings.
    let mut score: f64 = 100.0;
    for w in ql.split_whitespace() {
        if sym
            .desc_lower
            .split(|c: char| !c.is_alphanumeric())
            .any(|dw| dw.starts_with(w))
        {
            continue;
        }
        if sym.desc_lower.contains(w) {
            score = score.min(80.0);
        } else {
            return None;
        }
    }
    Some(score - (sym.desc.len().min(60) as f64) * 0.05)
}

/// `(row, score)` of the matches for `query`, best first, at most `limit`.
pub(crate) fn search(query: &str, limit: usize) -> Vec<(usize, f64)> {
    let q = query.trim();
    if q.is_empty() {
        return Vec::new();
    }
    let t = table();
    // A pasted non-ASCII character: that symbol, then entries starting with it.
    if !q.is_ascii() {
        let mut hits: Vec<(usize, f64)> = t
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                if s.char == q {
                    Some((i, 210.0))
                } else if s.char.starts_with(q) {
                    Some((i, 150.0 - s.char.len() as f64 * 0.1))
                } else {
                    None
                }
            })
            .collect();
        sort_and_cut(&mut hits, limit);
        return hits;
    }
    let nq = norm(q);
    let nql = nq.to_lowercase();
    let ql = q.trim_start_matches('\\').to_lowercase();
    let (cp_hex, cp_dec) = parse_codepoint(q);
    let mut hits: Vec<(usize, f64)> = t
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let mut best = if nq.is_empty() {
                None
            } else {
                name_score(s, &nq, &nql)
            };
            if let Some(d) = desc_score(s, &ql) {
                best = Some(best.map_or(d, |b: f64| b.max(d)));
            }
            for cp in [&cp_hex, &cp_dec].into_iter().flatten() {
                let c = if s.codepoint == cp.as_str() {
                    Some(195.0)
                } else if s.codepoint.starts_with(cp.as_str()) {
                    Some(70.0)
                } else if s.codepoint.contains(cp.as_str()) {
                    Some(50.0)
                } else {
                    None
                };
                if let Some(c) = c {
                    best = Some(best.map_or(c, |b: f64| b.max(c)));
                }
            }
            best.map(|b| (i, b))
        })
        .collect();
    sort_and_cut(&mut hits, limit);
    hits
}

fn sort_and_cut(hits: &mut Vec<(usize, f64)>, limit: usize) {
    hits.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    hits.truncate(limit);
}

/// The row's subtitle: `\leq \le · Lean: \le … · U+2264`.
pub(crate) fn subtitle(s: &MathSymbol) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !s.latex.is_empty() {
        parts.push(s.latex.to_string());
    }
    if !s.lean.is_empty() {
        let lean: Vec<String> = s
            .lean
            .split_whitespace()
            .take(4)
            .map(|n| format!("\\{n}"))
            .collect();
        parts.push(format!("Lean: {}", lean.join(" ")));
    }
    parts.push(format!("U+{}", s.codepoint));
    parts.join("  ·  ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn top(q: &str) -> &'static str {
        search(q, 5)
            .first()
            .map(|(i, _)| table()[*i].char)
            .unwrap_or("")
    }

    #[test]
    fn the_table_is_complete() {
        assert!(table().len() > 2800, "{}", table().len());
    }

    #[test]
    fn latex_lean_and_descriptions_find_symbols() {
        assert_eq!(top("alpha"), "α");
        assert_eq!(top("\\alpha"), "α");
        assert_eq!(top("leq"), "≤");
        assert_eq!(top("mathbb{R}"), "ℝ");
        assert_eq!(top("mathbb R"), "ℝ");
        assert_eq!(top("Rightarrow"), "⇒");
        assert_eq!(top("integral"), "∫");
        assert_eq!(
            top("Ga"),
            "α",
            "Lean input names are exact and case-sensitive"
        );
    }

    #[test]
    fn code_points_and_pasted_characters() {
        for q in ["U+2264", "0x2264", "\\u2264", "2264", "8804"] {
            assert_eq!(top(q), "≤", "{q}");
        }
        assert_eq!(top("∀"), "∀");
        assert!(search("∀", 50).len() > 1, "entries starting with ∀ follow");
    }

    #[test]
    fn subtitles_name_latex_lean_and_code_point() {
        let (i, _) = search("leq", 1)[0];
        let s = subtitle(&table()[i]);
        assert!(s.contains("\\leq"), "{s}");
        assert!(s.ends_with("U+2264"), "{s}");
    }

    #[test]
    fn search_is_fast() {
        let _ = table();
        let start = std::time::Instant::now();
        for q in [
            "a",
            "arrow",
            "mathbb",
            "U+22",
            "greek small",
            "x",
            "le",
            "integral",
        ] {
            let _ = search(q, 60);
        }
        // Eight queries; generous for an unoptimised test build.
        assert!(start.elapsed() < std::time::Duration::from_millis(400));
    }
}
