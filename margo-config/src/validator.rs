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
//!   * E004 — incomplete `bind` (fewer than MODS,KEY,ACTION fields).
//!   * E005 — unknown modifier in a `bind` (with a "did you mean").
//!   * W001 — unknown top-level key (parser warns into tracing; we
//!     surface it structured, with a closest-key suggestion).
//!   * W002 — out-of-set value for an enum key (lists the allowed set
//!     + a "did you mean").
//!   * W003 — a typed scalar value that doesn't parse as the key's
//!     declared primitive (bool / int / uint / float). Tables are
//!     derived from `parser::parse_option`; a drift-guard test keeps
//!     them inside `OPTION_KEYS`.
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
        // Reuse the parser's stripper so the validator sees the exact
        // same value the parser will — critically, its hex-colour guard
        // keeps `focuscolor = #c66b25` from being mistaken for a comment.
        let val = crate::parser::strip_inline_comment(raw_val)
            .trim()
            .to_string();

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
        if is_bind_key(key) {
            let val_start = eq_pos + 1 + val_trim_offset + 1;
            if val.split(',').count() < 3 {
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
            // E005: unknown modifier in the first (MODS) field. Walk the
            // `+`-joined tokens, tracking byte offset for a precise caret.
            let mods_field = val.split(',').next().unwrap_or("");
            let mut off = 0usize;
            for token in mods_field.split('+') {
                let tok = token.trim();
                if !tok.is_empty() && !is_valid_modifier_token(tok) {
                    let lead = token.len() - token.trim_start().len();
                    let col = val_start + off + lead;
                    let hint = match closest(
                        &tok.to_ascii_lowercase(),
                        MODIFIER_SUGGESTIONS.iter().copied(),
                    ) {
                        Some(s) => format!(" — did you mean `{s}`?"),
                        None => String::new(),
                    };
                    report.push(ConfigDiagnostic {
                        path: path.to_path_buf(),
                        line: lineno,
                        col,
                        end_col: col + tok.len(),
                        severity: Severity::Error,
                        code: "E005".into(),
                        message: format!("unknown modifier `{tok}` in bind{hint}"),
                        line_text: raw.to_string(),
                    });
                }
                off += token.len() + 1; // +1 for the consumed '+'
            }
        }

        // W002: a scalar enum key with a value outside its fixed set.
        // The parser keeps the default (or bails) on these; surface the
        // allowed set + a "did you mean" so the user doesn't scrape logs.
        if let Some((_, allowed)) = ENUM_KEYS.iter().find(|(k, _)| *k == key) {
            let v = val.to_ascii_lowercase();
            if !v.is_empty() && !allowed.contains(&v.as_str()) {
                let val_start = eq_pos + 1 + val_trim_offset + 1;
                let hint = match closest(&v, allowed.iter().copied()) {
                    Some(s) => format!(" — did you mean `{s}`?"),
                    None => String::new(),
                };
                report.push(ConfigDiagnostic {
                    path: path.to_path_buf(),
                    line: lineno,
                    col: val_start,
                    end_col: val_start + val.len().max(1),
                    severity: Severity::Warning,
                    code: "W002".into(),
                    message: format!(
                        "invalid value `{val}` for `{key}` (allowed: {}){hint}",
                        allowed.join(", ")
                    ),
                    line_text: raw.to_string(),
                });
            }
        }

        // W003: a typed scalar value that doesn't parse as the key's
        // declared primitive. The parser keeps the default silently, so
        // surface it. (Colours are excluded — see the type-table note.)
        if !val.is_empty() {
            let expected: Option<&str> = if BOOL_KEYS.contains(&key) {
                let v = val.to_ascii_lowercase();
                match v.as_str() {
                    "0" | "1" | "true" | "false" | "yes" | "no" | "on" | "off" => None,
                    _ => Some("a boolean (true/false)"),
                }
            } else if UINT_KEYS.contains(&key) {
                if val.parse::<u32>().is_ok() {
                    None
                } else {
                    Some("a non-negative integer")
                }
            } else if INT_KEYS.contains(&key) {
                if val.parse::<i32>().is_ok() {
                    None
                } else {
                    Some("an integer")
                }
            } else if FLOAT_KEYS.contains(&key) {
                if val.parse::<f64>().is_ok() {
                    None
                } else {
                    Some("a number")
                }
            } else {
                None
            };
            if let Some(expected) = expected {
                let val_start = eq_pos + 1 + val_trim_offset + 1;
                report.push(ConfigDiagnostic {
                    path: path.to_path_buf(),
                    line: lineno,
                    col: val_start,
                    end_col: val_start + val.len().max(1),
                    severity: Severity::Warning,
                    code: "W003".into(),
                    message: format!("expected {expected} for `{key}`, found `{val}`"),
                    line_text: raw.to_string(),
                });
            }
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

/// Closest candidate to `unknown` within a small edit distance, for
/// "did you mean …?". Returns `None` when nothing is close enough, so a
/// genuinely novel token doesn't get a misleading suggestion (within 2
/// edits and shorter than the candidate).
fn closest<'a>(unknown: &str, candidates: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let mut best: Option<(&'a str, usize)> = None;
    for c in candidates {
        let d = levenshtein(unknown, c);
        if best.is_none_or(|(_, bd)| d < bd) {
            best = Some((c, d));
        }
    }
    best.filter(|&(c, d)| d > 0 && d <= 2 && d < c.len())
        .map(|(c, _)| c)
}

/// The closest known config key to `unknown`.
fn suggest_key(unknown: &str) -> Option<&'static str> {
    const EXTRAS: &[&str] = &["exec", "exec-once", "include", "source", "bind"];
    closest(
        unknown,
        crate::parser::OPTION_KEYS
            .iter()
            .copied()
            .chain(CSV_SHAPED_KEYS.iter().copied())
            .chain(EXTRAS.iter().copied()),
    )
}

/// Valid bind modifier tokens (lowercased), matching `parser::parse_modifiers`.
const VALID_MODIFIERS: &[&str] = &[
    "super", "super_l", "super_r", "ctrl", "ctrl_l", "ctrl_r", "shift", "shift_l", "shift_r",
    "alt", "alt_l", "alt_r", "hyper", "hyper_l", "hyper_r", "none",
];
/// Canonical names offered as "did you mean" for a bad modifier (no _l/_r noise).
const MODIFIER_SUGGESTIONS: &[&str] = &["super", "ctrl", "shift", "alt", "hyper", "none"];

fn is_valid_modifier_token(t: &str) -> bool {
    let t = t.trim().to_ascii_lowercase();
    t.is_empty() || t.starts_with("code:") || VALID_MODIFIERS.contains(&t.as_str())
}

// ── Typed scalar keys ───────────────────────────────────────────────────────
// Derived from the `parse_bool` / `parse_i32` / `parse_u32` / `parse_f32`
// call sites in `parser::parse_option`. The parser silently keeps the default
// on a value that doesn't parse, so the validator surfaces it (W003). Colours
// are intentionally excluded (parser already errors on a bad colour, and
// mirroring its accepted formats here risks false positives). A drift-guard
// test asserts every entry is a real `OPTION_KEYS` member.

const BOOL_KEYS: &[&str] = &[
    "allow_lock_transparent",
    "animation_fade_in",
    "animation_fade_out",
    "animations",
    "auto_layout",
    "blur",
    "blur_layer",
    "blur_optimized",
    "canvas_anchor_animate",
    "canvas_pan_on_kill",
    "canvas_tiling",
    "capslock",
    "center_master_overspread",
    "center_when_single_stack",
    "disable_trackpad",
    "disable_while_typing",
    "drag_lock",
    "drag_tile_small",
    "drag_tile_to_tile",
    "drag_warp_cursor",
    "edge_scroller_pointer_focus",
    "enable_floating_snap",
    "enable_gaps",
    "enable_hotarea",
    "exchange_cross_monitor",
    "focus_cross_monitor",
    "focus_cross_tag",
    "focus_on_activate",
    "gaps_enabled",
    "idleinhibit_ignore_visible",
    "layer_animations",
    "layer_shadows",
    "left_handed",
    "middle_button_emulation",
    "mouse_natural_scrolling",
    "mru_show_labels",
    "mru_accent_selection",
    "new_is_master",
    "no_border_when_single",
    "no_radius_when_single",
    "numlockon",
    "per_output_frame_clock",
    "scratchpad_cross_monitor",
    "scroller_focus_center",
    "scroller_overview_loop",
    "scroller_prefer_center",
    "scroller_prefer_overspread",
    "shadow_only_floating",
    "shadows",
    "single_scratchpad",
    "sloppyfocus",
    "sloppyfocus_arrange",
    "smartgaps",
    "monly",
    "syncobj_enable",
    "tag_carousel",
    "taglayout_force",
    "tap_and_drag",
    "tap_to_click",
    "trackpad_natural_scrolling",
    "twilight",
    "view_current_to_back",
    "warpcursor",
    "xwayland_persistence",
];

const INT_KEYS: &[&str] = &[
    "blur_params_num_passes",
    "blur_params_radius",
    "border_radius",
    "canvas_tiling_gap",
    "drag_corner",
    "log_level",
    "overviewgappi",
    "overviewgappo",
    "repeat_delay",
    "mru_thumb_height",
    "repeat_rate",
    "scroller_overview_gap",
    "scroller_structs",
    "shadows_position_x",
    "shadows_position_y",
    "snap_distance",
];

const UINT_KEYS: &[&str] = &[
    "animation_duration_canvas_pan",
    "animation_duration_canvas_zoom",
    "animation_duration_close",
    "animation_duration_focus",
    "animation_duration_move",
    "animation_duration_open",
    "animation_duration_tag",
    "axis_bind_apply_timeout",
    "borderpx",
    "button_map",
    "cursor_hide_timeout",
    "cursor_size",
    "default_nmaster",
    "gappih",
    "gappiv",
    "gappoh",
    "gappov",
    "group_bar_gap",
    "group_bar_height",
    "hot_corner_dwell_ms",
    "hotarea_size",
    "mru_max",
    "mru_thumb_gap",
    "mru_panel_padding",
    "ov_tab_mode",
    "overview_transition_ms",
    "warmup_hidden_ms",
    "scroll_button",
    "send_events_mode",
    "shadows_size",
    "swipe_min_threshold",
    "touch_timeoutms",
    "twilight_day_gamma",
    "twilight_day_temp",
    "twilight_night_gamma",
    "twilight_night_temp",
    "twilight_static_gamma",
    "twilight_static_temp",
    "twilight_transition_s",
    "twilight_update_interval",
];

const FLOAT_KEYS: &[&str] = &[
    "blur_params_brightness",
    "blur_params_contrast",
    "blur_params_noise",
    "blur_params_saturation",
    "default_mfact",
    "drag_floating_refresh_interval",
    "drag_tile_refresh_interval",
    "fadein_begin_opacity",
    "fadeout_begin_opacity",
    "focused_opacity",
    "overview_dim_alpha",
    "mru_dim_alpha",
    "overview_zoom",
    "scratchpad_height_ratio",
    "scratchpad_width_ratio",
    "scroller_default_proportion",
    "scroller_overview_zoom",
    "shadows_blur",
    "twilight_latitude",
    "twilight_longitude",
    "unfocused_opacity",
    "zoom_end_ratio",
    "zoom_initial_ratio",
];

/// Scalar keys whose value is one of a fixed set. Mirrors the `match`
/// arms in `parser::parse_option`; keep in sync when a new enum knob lands.
const ENUM_KEYS: &[(&str, &[&str])] = &[
    ("mru_filter", &["all", "appid"]),
    ("mru_scope", &["all", "output", "workspace"]),
    ("overview_cycle_order", &["mru", "tag", "mixed"]),
    ("overview_style", &["grid", "scroller"]),
    ("twilight_mode", &["geo", "manual", "static", "schedule"]),
    ("wallpaper_fit", &["cover", "contain", "fill", "center"]),
];

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
    fn unknown_bind_modifier_is_an_error() {
        let r = validate_str("bind = altt,Tab,zoom\n");
        assert!(r.has_errors(), "`altt` is not a valid modifier");
        let e = r
            .errors()
            .find(|e| e.code == "E005")
            .expect("E005 expected");
        assert!(
            e.message.contains("altt") && e.message.contains("alt"),
            "should flag `altt` and suggest `alt`, got: {}",
            e.message
        );
    }

    #[test]
    fn valid_compound_modifiers_are_clean() {
        let r = validate_str("bind = super+shift,Return,spawn,kitty\n");
        assert!(!r.has_errors() && !r.has_warnings());
    }

    #[test]
    fn none_and_code_modifiers_are_clean() {
        let r = validate_str("bind = NONE,Print,screenshot\nbind = code:133,d,spawn,fuzzel\n");
        assert!(
            !r.has_errors() && !r.has_warnings(),
            "NONE and code: are valid"
        );
    }

    #[test]
    fn unknown_enum_value_warns_with_allowed_list() {
        let r = validate_str("overview_style = blah\n");
        let w = r
            .warnings()
            .find(|w| w.code == "W002")
            .expect("W002 expected for bad enum value");
        assert!(
            w.message.contains("grid") && w.message.contains("scroller"),
            "should list allowed values, got: {}",
            w.message
        );
    }

    #[test]
    fn valid_enum_value_is_clean() {
        let r = validate_str("overview_style = scroller\ntwilight_mode = manual\n");
        assert!(!r.has_errors() && !r.has_warnings());
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
    fn non_integer_value_warns() {
        let r = validate_str("borderpx = abc\n");
        let w = r.warnings().find(|w| w.code == "W003").expect("W003");
        assert!(
            w.message.contains("integer"),
            "expected integer hint, got: {}",
            w.message
        );
    }

    #[test]
    fn negative_for_unsigned_key_warns() {
        let r = validate_str("borderpx = -4\n");
        assert!(r.warnings().any(|w| w.code == "W003"));
    }

    #[test]
    fn non_bool_value_warns() {
        let r = validate_str("animations = maybe\n");
        let w = r.warnings().find(|w| w.code == "W003").expect("W003");
        assert!(w.message.contains("bool"), "got: {}", w.message);
    }

    #[test]
    fn non_number_for_float_key_warns() {
        let r = validate_str("default_mfact = half\n");
        assert!(r.warnings().any(|w| w.code == "W003"));
    }

    #[test]
    fn valid_typed_values_are_clean() {
        let r = validate_str(
            "borderpx = 4\nanimations = true\nsmartgaps = 0\nblur = off\ndefault_mfact = 0.55\n",
        );
        assert!(
            !r.has_errors() && !r.has_warnings(),
            "valid typed values must be clean, got: {:?}",
            r.diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn typed_tables_only_list_real_option_keys() {
        // Drift guard: every key I classified must be a real parser
        // option key, so a typo / removed key in the tables is caught.
        for &k in BOOL_KEYS
            .iter()
            .chain(INT_KEYS)
            .chain(UINT_KEYS)
            .chain(FLOAT_KEYS)
        {
            assert!(
                crate::parser::OPTION_KEYS.contains(&k),
                "`{k}` is in a type table but not in parser::OPTION_KEYS"
            );
        }
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
