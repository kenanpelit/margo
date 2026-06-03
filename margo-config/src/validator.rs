//! Config validator — re-walks the user's config file and collects
//! structured diagnostics WITHOUT mutating the parser's existing
//! silent-default behaviour.
//!
//! Design choice: the parser proper (`parser.rs`) intentionally
//! keeps the compositor up under a broken config — every malformed
//! primitive falls through `unwrap_or(default)`. That's the right
//! call for the live process. The validator is a separate pass
//! aimed at the user: it tells them exactly what's wrong and where.
//!
//! The validator does NOT try to be a full re-implementation of the
//! parser. It focuses on the failure modes a user can actually hit:
//!
//!   * E001 — trailing or doubled comma in CSV-shaped values
//!     (`bind`, `gesturebind`, `monitorrule`, `windowrule`, …).
//!     These are silently absorbed by `split_csv` and the bind
//!     ends up with an empty arg slot, which is what the user hit
//!     this session.
//!   * E002 — malformed line that the parser would log via
//!     `error!()` and skip. We hoist the same condition up to the
//!     diagnostic stream so it's visible without scraping logs.
//!   * E003 — include/source path that doesn't resolve.
//!   * W001 — unknown top-level key (the parser currently warns
//!     into tracing; we surface it structured).
//!
//! New rules slot into `validate_text` as more conditions show up.

use std::path::{Path, PathBuf};

use crate::diagnostics::{ConfigDiagnostic, DiagnosticReport, Severity};

/// Resolve the config path the same way `parse_config` does, then
/// validate it (plus any `include`/`source`-referenced files). Returns
/// a report whose `has_errors()` flag drives mctl's exit code.
pub fn validate_config(path: Option<&Path>) -> std::io::Result<DiagnosticReport> {
    let resolved = resolve_config_path(path)?;
    let mut report = DiagnosticReport::default();
    let mut visited = Vec::new();
    validate_file(&resolved, &mut report, &mut visited)?;
    Ok(report)
}

fn resolve_config_path(explicit: Option<&Path>) -> std::io::Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p.to_path_buf());
    }
    let home = std::env::var("HOME")
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::NotFound, "HOME env var not set"))?;
    Ok(PathBuf::from(home).join(".config/margo/config.conf"))
}

fn validate_file(
    path: &Path,
    report: &mut DiagnosticReport,
    visited: &mut Vec<PathBuf>,
) -> std::io::Result<()> {
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if visited.contains(&canon) {
        return Ok(());
    }
    visited.push(canon);

    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            report.push(ConfigDiagnostic {
                path: path.to_path_buf(),
                line: 0,
                col: 0,
                end_col: 0,
                severity: Severity::Error,
                code: "E000".into(),
                message: format!("cannot read config file: {e}"),
                line_text: String::new(),
            });
            return Ok(());
        }
    };
    validate_text(path, &text, report, visited)
}

fn validate_text(
    path: &Path,
    text: &str,
    report: &mut DiagnosticReport,
    visited: &mut Vec<PathBuf>,
) -> std::io::Result<()> {
    for (idx, raw) in text.lines().enumerate() {
        let lineno = idx + 1;
        let line_trim = raw.trim_start();
        if line_trim.is_empty() || line_trim.starts_with('#') {
            continue;
        }
        // Split key=value the same way the parser does.
        let Some(eq_pos) = raw.find('=') else {
            report.push(ConfigDiagnostic {
                path: path.to_path_buf(),
                line: lineno,
                col: 1,
                end_col: raw.len().max(1) + 1,
                severity: Severity::Error,
                code: "E002".into(),
                message: "missing `=` separator".to_string(),
                line_text: raw.to_string(),
            });
            continue;
        };
        let raw_key = &raw[..eq_pos];
        let raw_val = &raw[eq_pos + 1..];
        let key = raw_key.trim();
        let val_trim_offset = raw_val.len() - raw_val.trim_start().len();
        let val = strip_inline_comment(raw_val).trim().to_string();

        // include/source path resolution check.
        if key == "include" || key == "source" {
            let resolved = resolve_include_path(&val, path);
            if !resolved.exists() {
                let val_start = eq_pos + 1 + val_trim_offset + 1; // 1-indexed
                report.push(ConfigDiagnostic {
                    path: path.to_path_buf(),
                    line: lineno,
                    col: val_start,
                    end_col: val_start + val.len(),
                    severity: Severity::Error,
                    code: "E003".into(),
                    message: format!(
                        "source/include `{}` does not exist (resolved to `{}`)",
                        val,
                        resolved.display()
                    ),
                    line_text: raw.to_string(),
                });
            } else {
                let _ = validate_file(&resolved, report, visited);
            }
            continue;
        }

        // CSV-shaped values get the trailing/doubled comma check.
        if is_csv_shaped_key(key) {
            check_csv_commas(path, lineno, raw, eq_pos, &val, val_trim_offset, report);
        }

        // `bind` arity: needs at least MODS,KEY,ACTION. A 1- or 2-field
        // bind is silently dropped by the parser (no key/action), so
        // surface it. (Trailing/leading/doubled commas are E001 above,
        // so a value like `alt,Tab,zoom,` won't double-report here.)
        if is_bind_key(key) && val.split(',').count() < 3 {
            let val_start = eq_pos + 1 + val_trim_offset + 1;
            report.push(ConfigDiagnostic {
                path: path.to_path_buf(),
                line: lineno,
                col: val_start,
                end_col: val_start + val.len().max(1),
                severity: Severity::Error,
                code: "E004".into(),
                message: "incomplete `bind` — expected MODS,KEY,ACTION \
                          (e.g. `super,Return,spawn,kitty`)"
                    .to_string(),
                line_text: raw.to_string(),
            });
        }

        // Unknown top-level key (best-effort; allowlist).
        if !is_csv_shaped_key(key) && !is_bind_key(key) && !is_known_scalar_key(key) {
            let key_col = raw.find(key.chars().next().unwrap_or('?')).unwrap_or(0) + 1;
            let message = match suggest_key(key) {
                Some(s) => format!("unknown config key `{key}` — did you mean `{s}`?"),
                None => {
                    format!("unknown config key `{key}` — typo? (compositor will use the default)")
                }
            };
            report.push(ConfigDiagnostic {
                path: path.to_path_buf(),
                line: lineno,
                col: key_col,
                end_col: key_col + key.len(),
                severity: Severity::Warning,
                code: "W001".into(),
                message,
                line_text: raw.to_string(),
            });
        }
    }
    Ok(())
}

fn check_csv_commas(
    path: &Path,
    lineno: usize,
    raw: &str,
    eq_pos: usize,
    val: &str,
    val_trim_offset: usize,
    report: &mut DiagnosticReport,
) {
    // Compute 1-indexed start column of the value within the raw line.
    let val_start = eq_pos + 1 + val_trim_offset + 1;

    // Leading comma.
    if val.starts_with(',') {
        report.push(ConfigDiagnostic {
            path: path.to_path_buf(),
            line: lineno,
            col: val_start,
            end_col: val_start + 1,
            severity: Severity::Error,
            code: "E001".into(),
            message: "leading comma in CSV value — remove the `,`".to_string(),
            line_text: raw.to_string(),
        });
    }

    // Trailing comma.
    if val.ends_with(',') {
        let caret_col = val_start + val.len() - 1;
        report.push(ConfigDiagnostic {
            path: path.to_path_buf(),
            line: lineno,
            col: caret_col,
            end_col: caret_col + 1,
            severity: Severity::Error,
            code: "E001".into(),
            message: "trailing comma in CSV value — remove the `,`".to_string(),
            line_text: raw.to_string(),
        });
    }

    // Doubled / empty interior slot. Scan once.
    let bytes = val.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b',' && bytes[i + 1] == b',' {
            let caret_col = val_start + i;
            report.push(ConfigDiagnostic {
                path: path.to_path_buf(),
                line: lineno,
                col: caret_col,
                end_col: caret_col + 2,
                severity: Severity::Error,
                code: "E001".into(),
                message: "doubled comma in CSV value — empty slot between two `,`".to_string(),
                line_text: raw.to_string(),
            });
            // Skip to after the run of commas to avoid N N-1 ... 1
            // overlapping reports for `,,,,`.
            while i < bytes.len() && bytes[i] == b',' {
                i += 1;
            }
            continue;
        }
        i += 1;
    }
}

fn strip_inline_comment(s: &str) -> &str {
    // Match parser.rs strip_inline_comment: only strip ` #` at a
    // whitespace boundary so hex colours and regex anchors survive.
    let mut last_was_ws = true;
    for (i, c) in s.char_indices() {
        if last_was_ws && c == '#' {
            return &s[..i];
        }
        last_was_ws = c.is_whitespace();
    }
    s
}

fn resolve_include_path(include: &str, relative_to: &Path) -> PathBuf {
    let path = if let Some(rest) = include.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home).join(rest)
    } else if let Some(rest) = include.strip_prefix("./") {
        let dir = relative_to.parent().unwrap_or(Path::new("."));
        dir.join(rest)
    } else {
        PathBuf::from(include)
    };
    if path.is_absolute() {
        return path;
    }
    let dir = relative_to.parent().unwrap_or(Path::new("."));
    dir.join(path)
}

fn is_bind_key(k: &str) -> bool {
    if !k.starts_with("bind") {
        return false;
    }
    k[4..].chars().all(|c| matches!(c, 's' | 'l' | 'r' | 'p'))
}

/// CSV-shaped value keys (comma-separated fields). Kept as a slice so
/// it doubles as a suggestion source for `suggest_key`.
const CSV_SHAPED_KEYS: &[&str] = &[
    "mousebind",
    "axisbind",
    "switchbind",
    "gesturebind",
    "touchgesturebind",
    "windowrule",
    "monitorrule",
    "tagrule",
    "taglayout",
    "layerrule",
    "monitor",
    "env",
    "circle_layout",
];

fn is_csv_shaped_key(k: &str) -> bool {
    is_bind_key(k) || CSV_SHAPED_KEYS.contains(&k)
}

/// Levenshtein edit distance. Inputs are config keys (short), so the
/// straightforward two-row DP is plenty fast.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// The closest known config key to `unknown`, for "did you mean …?".
/// Returns `None` when nothing is within a small edit distance, so a
/// genuinely novel key doesn't get a misleading suggestion.
fn suggest_key(unknown: &str) -> Option<&'static str> {
    const EXTRAS: &[&str] = &["exec", "exec-once", "include", "source", "bind"];
    let candidates = crate::parser::OPTION_KEYS
        .iter()
        .copied()
        .chain(CSV_SHAPED_KEYS.iter().copied())
        .chain(EXTRAS.iter().copied());
    let mut best: Option<(&'static str, usize)> = None;
    for c in candidates {
        let d = levenshtein(unknown, c);
        if best.is_none_or(|(_, bd)| d < bd) {
            best = Some((c, d));
        }
    }
    // Suggest only a genuinely close match: within 2 edits and shorter
    // than the candidate (so a 3-char typo doesn't map to a 20-char key).
    best.filter(|&(c, d)| d > 0 && d <= 2 && d < c.len())
        .map(|(c, _)| c)
}

/// Recognised top-level scalar option keys (`exec`, `exec-once`,
/// `include`, `source` plus everything `parse_option` dispatches on).
/// The big list lives in `parser::OPTION_KEYS` so the validator and
/// the parser stay byte-for-byte aligned — adding a new option there
/// flips this `bool` `true` for the new key without touching the
/// validator.
fn is_known_scalar_key(k: &str) -> bool {
    matches!(k, "exec" | "exec-once" | "include" | "source")
        || crate::parser::OPTION_KEYS.contains(&k)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validate_str(text: &str) -> DiagnosticReport {
        let path = PathBuf::from("/tmp/test.conf");
        let mut report = DiagnosticReport::default();
        let mut visited = Vec::new();
        validate_text(&path, text, &mut report, &mut visited).unwrap();
        report
    }

    #[test]
    fn recently_added_keys_are_not_flagged_unknown() {
        // Guards parser↔OPTION_KEYS drift: a key handled by the parser
        // but missing from OPTION_KEYS would wrongly warn W001 here
        // (the exact regression these knobs hit during development).
        for key in ["tag_carousel", "edge_scroller_focus_allow_speed"] {
            let r = validate_str(&format!("{key} = 1\n"));
            assert!(
                !r.warnings().any(|w| w.code == "W001"),
                "`{key}` should be a known config key, got W001"
            );
        }
    }

    #[test]
    fn trailing_comma_in_bind_is_an_error() {
        let r = validate_str("bind = alt,Tab,overview_focus_next,\n");
        assert!(r.has_errors(), "trailing comma must surface as E001");
        let e = r.errors().next().unwrap();
        assert_eq!(e.code, "E001");
        assert_eq!(e.line, 1);
    }

    #[test]
    fn leading_comma_in_bind_is_an_error() {
        let r = validate_str("bind = ,alt,Tab,overview_focus_next\n");
        assert!(r.has_errors());
        assert_eq!(r.errors().next().unwrap().code, "E001");
    }

    #[test]
    fn doubled_comma_in_bind_is_an_error() {
        let r = validate_str("bind = alt,,Tab,overview_focus_next\n");
        assert!(r.has_errors());
        assert_eq!(r.errors().next().unwrap().code, "E001");
    }

    #[test]
    fn incomplete_bind_is_an_error() {
        // `bind` needs at least MODS,KEY,ACTION — this is missing the action.
        let r = validate_str("bind = alt,Tab\n");
        assert!(r.has_errors(), "a 2-field bind must surface an error");
        let e = r.errors().next().unwrap();
        assert_eq!(e.code, "E004");
        assert_eq!(e.line, 1);
    }

    #[test]
    fn complete_bind_with_three_fields_is_clean() {
        let r = validate_str("bind = alt,Tab,zoom\n");
        assert!(
            !r.has_errors() && !r.has_warnings(),
            "MODS,KEY,ACTION is a valid bind"
        );
    }

    #[test]
    fn missing_equals_is_an_error() {
        let r = validate_str("borderpx 4\n");
        assert!(r.has_errors());
        assert_eq!(r.errors().next().unwrap().code, "E002");
    }

    #[test]
    fn unknown_key_is_warning() {
        let r = validate_str("frobulator_intensity = 11\n");
        assert!(r.has_warnings());
        let w = r.warnings().next().unwrap();
        assert_eq!(w.code, "W001");
        assert_eq!(w.line, 1);
    }

    #[test]
    fn unknown_key_suggests_closest_match() {
        // `borderpix` is a one-char typo of the real key `borderpx`.
        let r = validate_str("borderpix = 4\n");
        let w = r.warnings().next().expect("typo should warn");
        assert_eq!(w.code, "W001");
        assert!(
            w.message.contains("did you mean") && w.message.contains("borderpx"),
            "expected a `did you mean borderpx` suggestion, got: {}",
            w.message
        );
    }

    #[test]
    fn wildly_unknown_key_has_no_suggestion() {
        // No close key — don't invent a misleading suggestion.
        let r = validate_str("frobulator_intensity = 11\n");
        let w = r.warnings().next().unwrap();
        assert!(
            !w.message.contains("did you mean"),
            "no near match should mean no suggestion, got: {}",
            w.message
        );
    }

    #[test]
    fn comment_lines_are_skipped() {
        let r = validate_str("# this is a comment\n# bind = ,bad,,,\n");
        assert!(!r.has_errors() && !r.has_warnings());
    }

    #[test]
    fn known_key_passes_clean() {
        let r = validate_str("borderpx = 4\n");
        assert!(!r.has_errors() && !r.has_warnings());
    }

    #[test]
    fn known_csv_key_passes_clean() {
        let r = validate_str("bind = alt,Tab,overview_focus_next\n");
        assert!(!r.has_errors() && !r.has_warnings());
    }
}
