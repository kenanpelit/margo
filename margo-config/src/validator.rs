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
//!   * E000 — the config file (or a sourced one) can't be read.
//!   * E001 — an empty KEY or ACTION field in a `bind` (a stray comma):
//!     margo drops the bind. A stray comma margo just ignores (a
//!     trailing one, an empty rule slot) is the warning W010.
//!   * E002 — malformed line that the parser would log via
//!     `error!()` and skip. We hoist the same condition up to the
//!     diagnostic stream so it's visible without scraping logs.
//!   * E003 — include/source path that doesn't resolve or can't be read.
//!   * E004 — incomplete `bind` (fewer than MODS,KEY,ACTION fields).
//!   * E005 — unknown modifier in a `bind` (with a "did you mean").
//!   * E006 — a malformed `var = name, value` line: the variable is not
//!     defined, so later `$name` references stay literal.
//!   * E007 — a `bind` KEY margo can't resolve to a keysym: the bind is
//!     dropped.
//!   * W001 — unknown top-level key (parser warns into tracing; we
//!     surface it structured, with a closest-key suggestion).
//!   * W002 — out-of-set value for an enum key (lists the allowed set
//!     + a "did you mean").
//!   * W003 — a typed scalar value that doesn't parse as the key's
//!     declared primitive (bool / int / uint / float). Tables are
//!     derived from `parser::parse_option`; a drift-guard test keeps
//!     them inside `OPTION_KEYS`.
//!   * W004 — the same MODS+KEY combo bound more than once in the same
//!     `keymode` across the config file tree (`config.conf` + every
//!     recursively `source`d/`include`d file, so `binds.d/*.conf` and
//!     `binds.conf` are both covered). margo's keypress lookup is
//!     `.iter().find()` — first match wins (`margo/src/input/mod.rs`) — so
//!     every bind after the first in a matching group is silently dead; only
//!     those are flagged, not the live one. A `common` bind fires in every
//!     mode, so an earlier `common` bind shadows the combo in all of them.
//!     Suppressed when BOTH the earlier and later bind use the `c` suffix
//!     (`bindc`) — an explicit "I know these share a combo" opt-in.
//!   * W005 — a bind ACTION that names no dispatch action (margo no-ops it).
//!   * W006 — a key margo accepts but doesn't act on.
//!   * W007 — `monitorrule` fields that only work together, set alone.
//!   * W008 — an option a `windowrule` / `layerrule` / `tagrule` /
//!     `monitorrule` doesn't know (the parser only logs it), with a
//!     "did you mean".
//!   * W009 — a window/layer rule with no recognised selector that also
//!     carries an unknown option: almost always a misspelled selector
//!     (`class:`, `namespace:`). margo drops such a rule instead of
//!     applying it to every window or layer surface.
//!   * W010 — a leading, trailing or doubled comma margo ignores.
//!   * W011 — a window/layer rule pattern that isn't a valid regex. margo
//!     falls back to a plain substring match, so the rule still loads.
//!
//! Values are `$var`-expanded and `source` wildcards resolved exactly as in
//! the parser (shared code), so both read the same text and the same files.
//! Every error marks a line the parser also drops or misreads;
//! `tests/parity.rs` holds both to that.
//!
//! New rules slot into `validate_text` as more conditions show up.

use std::path::{Path, PathBuf};

use crate::diagnostics::{ConfigDiagnostic, DiagnosticReport, Severity};

/// A single parsed `bind*` line, collected while walking the file tree
/// so W004 can compare across files after the walk finishes. Push
/// order matches parse order (files are visited depth-first at their
/// `source`/`include` point, same as `parser.rs`), which is exactly
/// margo's first-match-wins order.
struct BindEntry {
    path: PathBuf,
    line: usize,
    col: usize,
    end_col: usize,
    line_text: String,
    combo: String,
    /// The `keymode` the bind belongs to (`common` = every mode).
    mode: String,
    /// `bindc` — see the W004 note above.
    allow_conflict: bool,
}

/// Resolve the config path the same way `parse_config` does, then
/// validate it (plus any `include`/`source`-referenced files). Returns
/// a report whose `has_errors()` flag drives mctl's exit code.
pub fn validate_config(path: Option<&Path>) -> std::io::Result<DiagnosticReport> {
    let resolved = resolve_config_path(path)?;
    let mut report = DiagnosticReport::default();
    let mut walk = Walk::default();
    validate_file(&resolved, &mut report, &mut walk)?;
    check_bind_conflicts(&walk.binds, &mut report);
    Ok(report)
}

/// The validator's report on the config margo just booted with, for
/// `mctl config-errors` and the banner — so the diagnostics are visible
/// right after login, not only after the first reload. Empty when the file
/// doesn't exist or can't be read: margo then runs on defaults or a saved
/// generation and reports that on its own (`E-BOOT`).
pub fn boot_diagnostics(path: Option<&Path>) -> Vec<ConfigDiagnostic> {
    let Ok(resolved) = resolve_config_path(path) else {
        return Vec::new();
    };
    if std::fs::File::open(&resolved).is_err() {
        return Vec::new();
    }
    validate_config(Some(&resolved))
        .map(|r| r.diagnostics)
        .unwrap_or_default()
}

/// W004: flag every bind entry that is shadowed by an earlier one with
/// the same normalised MODS+KEY combo.
fn check_bind_conflicts(binds: &[BindEntry], report: &mut DiagnosticReport) {
    for (i, entry) in binds.iter().enumerate() {
        // The earliest bind on this combo that fires in every mode `entry`
        // fires in: same mode, or a `common` one. A later `common` bind is
        // never fully dead (it still fires in modes without that combo).
        let winner = binds[..i].iter().find(|w| {
            w.combo == entry.combo
                && (w.mode == entry.mode || w.mode == "common")
                && !(entry.allow_conflict && w.allow_conflict)
        });
        let Some(winner) = winner else { continue };
        let mode = if entry.mode == "default" {
            String::new()
        } else {
            format!(" in keymode `{}`", entry.mode)
        };
        report.push(ConfigDiagnostic {
            path: entry.path.clone(),
            line: entry.line,
            col: entry.col,
            end_col: entry.end_col,
            severity: Severity::Warning,
            code: "W004".into(),
            message: format!(
                "keybind `{}`{mode} is shadowed by an earlier bind at {}:{} — margo's first match wins, this one never fires",
                entry.combo,
                winner.path.display(),
                winner.line
            ),
            line_text: entry.line_text.clone(),
        });
    }
}

fn resolve_config_path(explicit: Option<&Path>) -> std::io::Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p.to_path_buf());
    }
    let home = std::env::var("HOME")
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::NotFound, "HOME env var not set"))?;
    Ok(PathBuf::from(home).join(".config/margo/config.conf"))
}

/// State carried across the whole `source`/`include` tree, in parse order —
/// the same things the parser threads through its recursion.
#[derive(Default)]
struct Walk {
    visited: Vec<PathBuf>,
    binds: Vec<BindEntry>,
    /// `var = name, value` definitions so far; a sourced file sees the
    /// variables defined before its `source` line, as in the parser.
    vars: crate::parser::Vars,
    /// The `keymode` binds are currently added to (`None` = margo's
    /// default, `"default"`).
    keymode: Option<String>,
}

impl Walk {
    fn keymode(&self) -> &str {
        self.keymode.as_deref().unwrap_or("default")
    }
}

fn validate_file(
    path: &Path,
    report: &mut DiagnosticReport,
    walk: &mut Walk,
) -> std::io::Result<()> {
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if walk.visited.contains(&canon) {
        return Ok(());
    }
    walk.visited.push(canon);

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
    validate_text(path, &text, report, walk)
}

/// When `$var` expansion changed a value, the per-check carets (computed on
/// the expanded text) don't line up with the raw line: widen them to the
/// whole raw value instead.
fn widen_carets(report: &mut DiagnosticReport, from: usize, span: Option<(usize, usize)>) {
    if let Some((start, end)) = span {
        for d in &mut report.diagnostics[from..] {
            if d.col >= start {
                d.col = start;
                d.end_col = end;
            }
        }
    }
}

fn validate_text(
    path: &Path,
    text: &str,
    report: &mut DiagnosticReport,
    walk: &mut Walk,
) -> std::io::Result<()> {
    // (index of the line's first diagnostic, raw value span) of the
    // previous line when its value was `$var`-expanded.
    let mut pending: (usize, Option<(usize, usize)>) = (0, None);
    for (idx, raw) in text.lines().enumerate() {
        widen_carets(report, pending.0, pending.1.take());
        pending.0 = report.diagnostics.len();
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
        let mut val_trim_offset = raw_val.len() - raw_val.trim_start().len();
        // Reuse the parser's stripper so the validator sees the exact
        // same value the parser will — critically, its hex-colour guard
        // keeps `focuscolor = #c66b25` from being mistaken for a comment.
        // Enclosing quotes come off too, exactly as in the parser.
        let commentless = crate::parser::strip_inline_comment(raw_val).trim();
        let unquoted = crate::parser::strip_enclosing_quotes(commentless);
        if unquoted.len() != commentless.len() {
            // Keep carets on the value: skip the opening quote and any
            // whitespace that followed it.
            val_trim_offset += commentless[1..].len() - commentless[1..].trim_start().len() + 1;
        }
        // `$name` / `${name}` expand exactly as in the parser, so a bind's
        // `$mod` or a `source = $dir/x.conf` is checked as margo reads it.
        let val = crate::parser::expand_vars(unquoted, &walk.vars);
        if val != unquoted {
            let start = eq_pos + 1 + val_trim_offset + 1;
            pending.1 = Some((start, start + unquoted.len().max(1)));
        }

        if key == "var" {
            if let Err(e) = crate::parser::parse_var(&val, &mut walk.vars) {
                let val_start = eq_pos + 1 + val_trim_offset + 1;
                report.push(ConfigDiagnostic {
                    path: path.to_path_buf(),
                    line: lineno,
                    col: val_start,
                    end_col: val_start + val.len().max(1),
                    severity: Severity::Error,
                    code: "E006".into(),
                    message: format!("{e} — the variable is not defined"),
                    line_text: raw.to_string(),
                });
            }
            continue;
        }

        if key == "keymode" {
            // Same value the parser stores (it keeps the first 27 chars).
            walk.keymode = Some(val.chars().take(27).collect());
        }

        // include/source path resolution check.
        if key == "include" || key == "source" {
            match crate::parser::include_targets(&val, path) {
                crate::parser::IncludeTarget::Missing(resolved) => {
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
                }
                crate::parser::IncludeTarget::Files(files) => {
                    for f in files {
                        // Unreadable (a directory, no permission): report it
                        // on this line, where the parser logs it too.
                        let readable = std::fs::File::open(&f)
                            .and_then(|h| h.metadata())
                            .map_err(|e| e.to_string())
                            .and_then(|m| {
                                if m.is_dir() {
                                    Err("it is a directory".to_string())
                                } else {
                                    Ok(())
                                }
                            });
                        if let Err(e) = readable {
                            let val_start = eq_pos + 1 + val_trim_offset + 1;
                            report.push(ConfigDiagnostic {
                                path: path.to_path_buf(),
                                line: lineno,
                                col: val_start,
                                end_col: val_start + val.len().max(1),
                                severity: Severity::Error,
                                code: "E003".into(),
                                message: format!(
                                    "source/include `{}` cannot be read ({e}; resolved to `{}`)",
                                    val,
                                    f.display()
                                ),
                                line_text: raw.to_string(),
                            });
                            continue;
                        }
                        let _ = validate_file(&f, report, walk);
                    }
                }
            }
            continue;
        }

        // CSV-shaped values get the stray-comma check (W010). `env`'s value
        // is free text after the name, commas included.
        if is_csv_shaped_key(key) && key != "env" {
            check_csv_commas(path, lineno, raw, eq_pos, &val, val_trim_offset, report);
        }

        // W007: monitorrule fields that only work together (mango
        // 4d0fb96a). margo keeps the rule — `width`+`height` without
        // `refresh` still picks the highest rate — but says what's
        // ignored instead of leaving the user guessing.
        if (key == "monitorrule" || key == "monitor")
            && let Some(msg) = monitorrule_problem(&val)
        {
            let val_start = eq_pos + 1 + val_trim_offset + 1;
            report.push(ConfigDiagnostic {
                path: path.to_path_buf(),
                line: lineno,
                col: val_start,
                end_col: val_start + val.len().max(1),
                severity: Severity::Warning,
                code: "W007".into(),
                message: msg.to_string(),
                line_text: raw.to_string(),
            });
        }

        // W008 / W009: rule options the parser doesn't know.
        if let Some(table) = rule_table(key) {
            let val_start = eq_pos + 1 + val_trim_offset + 1;
            check_rule_options(path, lineno, raw, val_start, key, &val, &table, report);
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
            } else {
                // E001: an empty KEY or ACTION field (a stray comma) — the
                // parser drops the bind. E007: a KEY margo can't resolve —
                // also dropped.
                let fields: Vec<&str> = val.splitn(4, ',').collect();
                let key_col = val_start + fields[0].len() + 1;
                let action_col = key_col + fields[1].len() + 1;
                for (field, col, what) in [
                    (fields[1], key_col, "KEY"),
                    (fields[2], action_col, "ACTION"),
                ] {
                    if field.trim().is_empty() {
                        report.push(ConfigDiagnostic {
                            path: path.to_path_buf(),
                            line: lineno,
                            col: col.saturating_sub(1).max(1),
                            end_col: col + field.len().max(1),
                            severity: Severity::Error,
                            code: "E001".into(),
                            message: format!(
                                "empty {what} field in `{key}` (stray comma?) — margo drops this bind"
                            ),
                            line_text: raw.to_string(),
                        });
                    }
                }
                let key_field = fields[1].trim();
                if !key_field.is_empty()
                    && crate::parser::parse_key(key_field, key[4..].contains('s')).is_err()
                {
                    let lead = fields[1].len() - fields[1].trim_start().len();
                    report.push(ConfigDiagnostic {
                        path: path.to_path_buf(),
                        line: lineno,
                        col: key_col + lead,
                        end_col: key_col + lead + key_field.len(),
                        severity: Severity::Error,
                        code: "E007".into(),
                        message: format!(
                            "unknown key `{key_field}` in `{key}` — margo drops this bind \
                             (an xkb keysym name such as `Return`, or `code:NN`)"
                        ),
                        line_text: raw.to_string(),
                    });
                }
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

            // W005: the ACTION field (3rd) names no dispatch action.
            // margo's dispatcher silently no-ops an unknown action
            // (`_ => "unhandled action"`), so a typo'd bind just does
            // nothing with no feedback. Checked against the `actions`
            // catalogue every other tool renders; a test in `margo`
            // keeps that catalogue in lock-step with the dispatcher.
            if val.split(',').count() >= 3 {
                let mut fields = val.splitn(4, ',');
                let mods_raw = fields.next().unwrap_or("");
                let key_raw = fields.next().unwrap_or("");
                let action_raw = fields.next().unwrap_or("");
                let action = action_raw.trim();
                if !action.is_empty() && !crate::actions::is_known(action) {
                    let lead = action_raw.len() - action_raw.trim_start().len();
                    let col = val_start + mods_raw.len() + 1 + key_raw.len() + 1 + lead;
                    let hint = match closest(action, crate::actions::all_names().into_iter()) {
                        Some(s) => format!(" — did you mean `{s}`?"),
                        None => String::new(),
                    };
                    report.push(ConfigDiagnostic {
                        path: path.to_path_buf(),
                        line: lineno,
                        col,
                        end_col: col + action.len(),
                        severity: Severity::Warning,
                        code: "W005".into(),
                        message: format!("unknown bind action `{action}`{hint}"),
                        line_text: raw.to_string(),
                    });
                }
            }

            // W004 input: only collect combos from arity-clean binds —
            // an E004/E001 line's MODS/KEY fields aren't trustworthy.
            if val.split(',').count() >= 3 {
                let mut parts = val.splitn(3, ',');
                let mods_part = parts.next().unwrap_or("").trim();
                let key_part = parts.next().unwrap_or("").trim();
                if !key_part.is_empty() {
                    let mut mods: Vec<String> = mods_part
                        .split('+')
                        .map(|t| t.trim().to_ascii_lowercase())
                        .filter(|t| !t.is_empty())
                        .collect();
                    mods.sort();
                    let key_lower = key_part.to_ascii_lowercase();
                    let combo = if mods.is_empty() {
                        key_lower
                    } else {
                        format!("{}+{key_lower}", mods.join("+"))
                    };
                    walk.binds.push(BindEntry {
                        path: path.to_path_buf(),
                        line: lineno,
                        col: val_start,
                        end_col: val_start + mods_part.len() + 1 + key_part.len(),
                        line_text: raw.to_string(),
                        combo,
                        mode: walk.keymode().to_string(),
                        allow_conflict: key[4..].contains('c'),
                    });
                }
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

        // W006: accepted, but the compositor doesn't act on it.
        if NO_EFFECT_KEYS.contains(&key) {
            let key_col = raw.len() - raw.trim_start().len() + 1;
            report.push(ConfigDiagnostic {
                path: path.to_path_buf(),
                line: lineno,
                col: key_col,
                end_col: key_col + key.len(),
                severity: Severity::Warning,
                code: "W006".into(),
                message: format!("`{key}` is accepted but currently has no effect in margo"),
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
    widen_carets(report, pending.0, pending.1);
    Ok(())
}

/// The option table (and selector subset) for a rule key.
struct RuleTable {
    kind: &'static str,
    keys: &'static [&'static str],
    matchers: &'static [&'static str],
}

fn rule_table(key: &str) -> Option<RuleTable> {
    use crate::parser::{
        LAYERRULE_KEYS, LAYERRULE_MATCHERS, MONITORRULE_KEYS, TAGRULE_KEYS, WINDOWRULE_KEYS,
        WINDOWRULE_MATCHERS,
    };
    let (kind, keys, matchers): (_, &'static [&'static str], &'static [&'static str]) = match key {
        "windowrule" | "windowrule-once" => ("windowrule", WINDOWRULE_KEYS, WINDOWRULE_MATCHERS),
        "layerrule" => ("layerrule", LAYERRULE_KEYS, LAYERRULE_MATCHERS),
        "tagrule" => ("tagrule", TAGRULE_KEYS, &[]),
        "monitorrule" => ("monitorrule", MONITORRULE_KEYS, &[]),
        _ => return None,
    };
    Some(RuleTable {
        kind,
        keys,
        matchers,
    })
}

/// Spellings from other compositors (and margo's own docs, once) that
/// edit distance can't map to the margo option.
const RULE_KEY_HINTS: &[(&str, &str, &str)] = &[
    ("windowrule", "class", "appid"),
    ("windowrule", "initialclass", "appid"),
    ("windowrule", "initial_class", "appid"),
    ("windowrule", "initialtitle", "title"),
    ("windowrule", "initial_title", "title"),
    (
        "windowrule",
        "opacity",
        "focused_opacity` / `unfocused_opacity",
    ),
    ("windowrule", "workspace", "tags"),
    ("windowrule", "float", "isfloating"),
    ("layerrule", "namespace", "layer_name"),
    ("layerrule", "layer", "layer_name"),
];

/// W008 for every option a rule doesn't know; W009 when the rule also has
/// no selector — margo drops such a rule rather than apply it everywhere
/// (see `parser::check_rule_selector`).
#[allow(clippy::too_many_arguments)]
fn check_rule_options(
    path: &Path,
    lineno: usize,
    raw: &str,
    val_start: usize,
    key: &str,
    val: &str,
    table: &RuleTable,
    report: &mut DiagnosticReport,
) {
    let mut has_matcher = false;
    let mut unknown = Vec::new();
    let mut off = 0usize;
    for part in val.split(',') {
        let k_raw = part.split(':').next().unwrap_or("");
        let k = k_raw.trim();
        let lead = k_raw.len() - k_raw.trim_start().len();
        let col = val_start + off + lead;
        off += part.len() + 1;
        if k.is_empty() {
            // An empty slot is E001's business.
            continue;
        }
        if table.matchers.contains(&k) {
            has_matcher = true;
            // W011: margo compiles selector patterns with the `regex`
            // crate (`matches_rule_text`) and falls back to a substring
            // match when that fails.
            if let Some((_, pat_raw)) = part.split_once(':') {
                let pat = pat_raw.trim();
                if !pat.is_empty()
                    && let Err(e) = regex::Regex::new(pat)
                {
                    let pcol = val_start
                        + (off - part.len() - 1)
                        + k_raw.len()
                        + 1
                        + (pat_raw.len() - pat_raw.trim_start().len());
                    let first = e.to_string();
                    let first = first.lines().last().unwrap_or("").trim();
                    report.push(ConfigDiagnostic {
                        path: path.to_path_buf(),
                        line: lineno,
                        col: pcol,
                        end_col: pcol + pat.len(),
                        severity: Severity::Warning,
                        code: "W011".into(),
                        message: format!(
                            "`{k}:` is not a valid regex ({first}) — margo falls back to a plain \
                             substring match"
                        ),
                        line_text: raw.to_string(),
                    });
                }
            }
        }
        if table.keys.contains(&k) {
            continue;
        }
        let hint = RULE_KEY_HINTS
            .iter()
            .find(|(kind, wrong, _)| *kind == table.kind && wrong.eq_ignore_ascii_case(k))
            .map(|(_, _, right)| *right)
            .or_else(|| closest(&k.to_ascii_lowercase(), table.keys.iter().copied()));
        let hint = match hint {
            Some(s) => format!(" — did you mean `{s}`?"),
            None => String::new(),
        };
        report.push(ConfigDiagnostic {
            path: path.to_path_buf(),
            line: lineno,
            col,
            end_col: col + k.len(),
            severity: Severity::Warning,
            code: "W008".into(),
            message: format!("unknown {} option `{k}`{hint} (ignored)", table.kind),
            line_text: raw.to_string(),
        });
        unknown.push(k);
    }
    if !table.matchers.is_empty() && !has_matcher && !unknown.is_empty() {
        report.push(ConfigDiagnostic {
            path: path.to_path_buf(),
            line: lineno,
            col: val_start,
            end_col: val_start + val.len().max(1),
            severity: Severity::Warning,
            code: "W009".into(),
            message: format!(
                "`{key}` has no recognised selector ({}) — margo ignores this rule \
                 instead of applying it to everything",
                table.matchers.join(", ")
            ),
            line_text: raw.to_string(),
        });
    }
}

/// The first monitorrule field combination the compositor can't honour.
fn monitorrule_problem(val: &str) -> Option<&'static str> {
    let mut has = std::collections::HashSet::new();
    let mut custom = false;
    for field in val.split(',') {
        let Some((k, v)) = field.split_once(':') else {
            continue;
        };
        let k = k.trim().to_ascii_lowercase();
        if k == "custom" {
            custom = matches!(v.trim(), "1" | "true" | "yes" | "on");
        }
        has.insert(k);
    }
    let (w, h, r) = (
        has.contains("width"),
        has.contains("height"),
        has.contains("refresh"),
    );
    if w != h {
        Some(
            "monitorrule sets only one of `width` / `height` — set both or the mode part is ignored",
        )
    } else if custom && !(w && r) {
        Some("monitorrule `custom:1` needs `width`, `height` and `refresh` — it is ignored")
    } else if r && !w {
        Some("monitorrule `refresh` has no effect without `width` and `height`")
    } else if has.contains("x") != has.contains("y") {
        Some("monitorrule sets only one of `x` / `y` — set both or the position is automatic")
    } else {
        None
    }
}

/// W010: a leading, trailing or doubled comma. margo reads the empty
/// field as unset, which is harmless in most places (a bind's trailing
/// argument, a rule's option list) — so a warning. Where an empty field
/// makes margo drop the line, that check reports an error of its own
/// (E001 for a bind's KEY/ACTION, E004, E007).
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
            severity: Severity::Warning,
            code: "W010".into(),
            message: "leading comma — the first field is empty; remove the `,`".to_string(),
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
            severity: Severity::Warning,
            code: "W010".into(),
            message: "trailing comma — remove the `,` (margo ignores the empty field)".to_string(),
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
                severity: Severity::Warning,
                code: "W010".into(),
                message: "doubled comma — an empty field between two `,`".to_string(),
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

fn is_bind_key(k: &str) -> bool {
    if !k.starts_with("bind") {
        return false;
    }
    k[4..]
        .chars()
        .all(|c| matches!(c, 's' | 'l' | 'r' | 'p' | 'c'))
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
    "windowrule-once",
    "monitorrule",
    "tagrule",
    "taglayout",
    "layerrule",
    "monitor",
    "env",
    "var",
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

/// (`blur_optimized` is a deliberate, documented compatibility no-op —
/// its wallpaper-only blur cache was hardware-tested and reverted — so
/// it is not listed.)
///
/// Keys the parser accepts (mango compatibility, or settings that were
/// planned) but the compositor never reads — setting them changes
/// nothing. Reported as W006 so a user tuning one isn't left guessing.
/// `margo`'s `no_effect_keys_stay_unread` test fails when the compositor
/// starts reading one of these, so an implemented key can't keep warning.
pub const NO_EFFECT_KEYS: &[&str] = &[
    "allow_tearing",
    "tablet_map_to_mon",
    "touch_degrees_leniency",
    "touch_distance_threshold",
    "touch_edge_size_bottom",
    "touch_edge_size_left",
    "touch_edge_size_right",
    "touch_edge_size_top",
    "touch_timeoutms",
    "touchgesturebind",
];

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
    "auto_reload_config",
    "blur",
    "blur_layer",
    "blur_optimized",
    "capslock",
    "center_master_overspread",
    "center_when_single_stack",
    "color_management",
    "dim_enable",
    "disable_middle_paste",
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
    "hot_corner_disable_on_fullscreen",
    "idle_inhibit_when_fullscreen",
    "idleinhibit_ignore_visible",
    "layer_animations",
    "layer_shadows",
    "left_handed",
    "log_to_file",
    "middle_button_emulation",
    "monly",
    "mosaic_auto_overflow_tag",
    "mosaic_overflow_stack",
    "mouse_natural_scrolling",
    "mru_accent_selection",
    "mru_show_labels",
    "new_is_master",
    "no_border_when_single",
    "no_radius_when_single",
    "numlockon",
    "per_output_frame_clock",
    "scratchpad_cross_monitor",
    "scroller_focus_center",
    "scroller_ignore_proportion_single",
    "scroller_overview_loop",
    "scroller_prefer_center",
    "scroller_prefer_overspread",
    "shadow_only_floating",
    "shadows",
    "single_scratchpad",
    "sloppyfocus",
    "sloppyfocus_arrange",
    "smartgaps",
    "syncobj_enable",
    "tag_carousel",
    "tag_gather",
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
    "allow_shortcuts_inhibit",
    "allow_tearing",
    "blur_params_num_passes",
    "blur_params_radius",
    "border_radius",
    "drag_corner",
    "hotarea_corner",
    "log_level",
    "overviewgappi",
    "overviewgappo",
    "repeat_delay",
    "repeat_rate",
    "scroller_overview_gap",
    "scroller_structs",
    "shadows_position_x",
    "shadows_position_y",
    "snap_distance",
];

const UINT_KEYS: &[&str] = &[
    "accel_profile",
    "animation_duration_close",
    "animation_duration_focus",
    "animation_duration_move",
    "animation_duration_open",
    "animation_duration_tag",
    "axis_bind_apply_timeout",
    "borderpx",
    "button_map",
    "click_method",
    "cursor_hide_timeout",
    "cursor_size",
    "default_nmaster",
    "gap",
    "gappi",
    "gappih",
    "gappiv",
    "gappo",
    "gappoh",
    "gappov",
    "gaps",
    "gaps_in",
    "gaps_out",
    "gapsin",
    "gapsout",
    "group_bar_gap",
    "group_bar_height",
    "hot_corner_dwell_ms",
    "session_end_countdown_ms",
    "session_end_grace_ms",
    "hotarea_size",
    "inner_gaps",
    "log_keep_sessions",
    "monitor_gap",
    "mouse_accel_profile",
    "mru_max",
    "mru_panel_padding",
    "mru_thumb_gap",
    "mru_thumb_height",
    "outer_gaps",
    "ov_tab_mode",
    "overview_transition_ms",
    "scroll_button",
    "scroll_method",
    "send_events_mode",
    "shadows_size",
    "swipe_min_threshold",
    "touch_timeoutms",
    "trackpad_accel_profile",
    "twilight_day_gamma",
    "twilight_day_temp",
    "twilight_night_gamma",
    "twilight_night_temp",
    "twilight_static_gamma",
    "twilight_static_temp",
    "twilight_transition_s",
    "twilight_update_interval",
    "warmup_hidden_ms",
    "window_gap",
];

const FLOAT_KEYS: &[&str] = &[
    "accel_speed",
    "animation_spring_damping_ratio",
    "animation_spring_mass",
    "animation_spring_stiffness",
    "axis_scroll_factor",
    "blur_params_brightness",
    "blur_params_contrast",
    "blur_params_noise",
    "blur_params_saturation",
    "default_mfact",
    "drag_floating_refresh_interval",
    "drag_tile_refresh_interval",
    "edge_scroller_focus_allow_speed",
    "fadein_begin_opacity",
    "fadeout_begin_opacity",
    "focused_opacity",
    "mouse_accel_speed",
    "mru_dim_alpha",
    "overview_dim_alpha",
    "overview_selected_border_multiplier",
    "overview_zoom",
    "scratchpad_height_ratio",
    "scratchpad_width_ratio",
    "scroller_default_proportion",
    "scroller_default_proportion_single",
    "scroller_overview_zoom",
    "shadows_blur",
    "touch_degrees_leniency",
    "touch_distance_threshold",
    "touch_edge_size_bottom",
    "touch_edge_size_left",
    "touch_edge_size_right",
    "touch_edge_size_top",
    "trackpad_accel_speed",
    "trackpad_scroll_factor",
    "twilight_latitude",
    "twilight_longitude",
    "unfocused_opacity",
    "zoom_end_ratio",
    "zoom_initial_ratio",
];

const ANIMATION_CLOCKS: &[&str] = &["bezier", "spring"];

/// Scalar keys whose value is one of a fixed set. Mirrors the `match`
/// arms in `parser::parse_option`; keep in sync when a new enum knob lands.
const ENUM_KEYS: &[(&str, &[&str])] = &[
    ("animation_clock_close", ANIMATION_CLOCKS),
    ("animation_clock_focus", ANIMATION_CLOCKS),
    ("animation_clock_layer", ANIMATION_CLOCKS),
    ("animation_clock_move", ANIMATION_CLOCKS),
    ("animation_clock_open", ANIMATION_CLOCKS),
    ("animation_clock_tag", ANIMATION_CLOCKS),
    (
        "log_file_level",
        &["error", "warn", "info", "debug", "trace"],
    ),
    ("mru_filter", &["all", "appid"]),
    ("mru_scope", &["all", "output", "workspace"]),
    ("overview_cycle_order", &["mru", "tag", "mixed"]),
    ("overview_style", &["grid", "scroller"]),
    ("twilight_mode", &["geo", "manual", "static", "schedule"]),
    ("wallpaper_fit", crate::types::WallpaperFit::NAMES),
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
        let mut walk = Walk::default();
        validate_text(&path, text, &mut report, &mut walk).unwrap();
        check_bind_conflicts(&walk.binds, &mut report);
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
    fn trailing_comma_in_bind_is_a_warning() {
        // margo loads this bind (the empty trailing argument is unset), so
        // it must not block a reload.
        let r = validate_str("bind = alt,Tab,overview_focus_next,\n");
        assert!(!r.has_errors(), "{:?}", r.diagnostics);
        let w = r.warnings().next().unwrap();
        assert_eq!(w.code, "W010");
        assert_eq!(w.line, 1);
    }

    #[test]
    fn leading_comma_in_bind_shifts_fields_into_an_error() {
        // MODS empty, KEY `alt` (no such keysym) — margo drops the bind.
        let r = validate_str("bind = ,alt,Tab,overview_focus_next\n");
        assert!(r.has_errors());
        assert!(r.errors().any(|e| e.code == "E007"), "{:?}", r.diagnostics);
    }

    #[test]
    fn empty_action_and_unknown_key_are_errors() {
        let r = validate_str("bind = super,q,,1\nbind = super,Retrun,spawn,kitty\n");
        assert_eq!(
            r.errors()
                .map(|e| (e.code.as_str(), e.line))
                .collect::<Vec<_>>(),
            [("E001", 1), ("E007", 2)]
        );
    }

    #[test]
    fn empty_bind_arguments_are_a_documented_form() {
        // `focusapp,app,,cmd` — an empty middle argument stays unset.
        let r = validate_str("bind = super,f,focusapp,firefox,,firefox\n");
        assert!(!r.has_errors(), "{:?}", r.diagnostics);
    }

    #[test]
    fn env_value_may_contain_commas() {
        let r = validate_str("env = LIST,a,,b\n");
        assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
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
    fn unknown_bind_action_is_a_warning() {
        let r = validate_str("bind = super+ctrl,Tab,focuslsat\n");
        let w = r
            .warnings()
            .find(|w| w.code == "W005")
            .expect("W005 expected for a typo'd action");
        assert!(
            w.message.contains("focuslsat") && w.message.contains("focuslast"),
            "should flag `focuslsat` and suggest `focuslast`, got: {}",
            w.message
        );
        assert!(!r.has_errors());
    }

    #[test]
    fn known_bind_actions_are_clean() {
        // Canonical names, aliases, and the once-uncatalogued verbs
        // (`dpms`, `mru_next`, `twilight_toggle`) must all pass.
        for line in [
            "bind = super,z,zoom\n",
            "bind = super+ctrl,Tab,focuslast\n",
            "bind = super+ctrl,Tab,focus-last\n", // alias
            "bind = super,Return,spawn,kitty -e htop\n",
            "bind = super,F10,dpms,off\n",
            "bind = super,Tab,mru_next,workspace,appid\n",
            "bind = super,n,twilight_toggle\n",
        ] {
            let r = validate_str(line);
            assert!(
                r.warnings().all(|w| w.code != "W005"),
                "`{line}` names a real action, no W005 expected"
            );
        }
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
    fn quoted_values_validate_like_the_parser_reads_them() {
        let r = validate_str("overview_style = \"scroller\"\nborderpx = '4'\n");
        assert!(!r.has_errors() && !r.has_warnings());
        // A bad value inside quotes still warns, caret on the value.
        let r = validate_str("overview_style = \"blah\"\n");
        let w = r.warnings().find(|w| w.code == "W002").expect("W002");
        assert_eq!(w.col, 19, "caret should skip the opening quote");
    }

    #[test]
    fn monitorrule_field_pairs_warn() {
        let w7 = |line: &str| validate_str(line).warnings().any(|w| w.code == "W007");
        assert!(!w7(
            "monitorrule = name:DP-3, width:2560, height:1440, refresh:60, x:0, y:0\n"
        ));
        assert!(!w7("monitorrule = name:DP-3, width:2560, height:1440\n"));
        assert!(w7("monitorrule = name:DP-3, width:2560\n"));
        assert!(w7("monitorrule = name:HDMI-A-1, refresh:144\n"));
        assert!(w7("monitorrule = name:DP-3, x:0\n"));
        assert!(w7(
            "monitorrule = name:DP-3, width:2560, height:1440, custom:1\n"
        ));
        assert!(!w7(
            "monitorrule = name:DP-3, width:2560, height:1440, refresh:75, custom:1\n"
        ));
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
    fn typed_tables_are_mutually_disjoint() {
        // A key has exactly one primitive type. If it appeared in two
        // tables, the validator's if/else type check (BOOL → INT → UINT →
        // FLOAT, then ENUM) would silently pick the first match and
        // mis-validate every value the user writes for it.
        use std::collections::HashMap;
        let mut seen: HashMap<&str, &str> = HashMap::new();
        for (label, keys) in [
            ("BOOL", BOOL_KEYS),
            ("INT", INT_KEYS),
            ("UINT", UINT_KEYS),
            ("FLOAT", FLOAT_KEYS),
        ] {
            for &k in keys {
                assert!(
                    seen.insert(k, label).is_none(),
                    "`{k}` is classified as two primitive types (second: {label})"
                );
            }
        }
        for (k, _) in ENUM_KEYS {
            assert!(
                !seen.contains_key(k),
                "`{k}` is in ENUM_KEYS and also a scalar type table"
            );
            // ENUM keys are real options too (the sibling ⊆ test doesn't
            // cover them).
            assert!(
                crate::parser::OPTION_KEYS.contains(k),
                "`{k}` is in ENUM_KEYS but not in parser::OPTION_KEYS"
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

    #[test]
    fn duplicate_bind_combo_is_a_warning() {
        let r = validate_str("bind = super+alt,k,spawn,foo\nbind = super+alt,k,spawn,bar\n");
        let w = r
            .warnings()
            .find(|w| w.code == "W004")
            .expect("W004 expected");
        // The second (shadowed) occurrence is flagged, on its own line.
        assert_eq!(w.line, 2);
        assert!(
            w.message.contains("1"),
            "should point back at line 1: {}",
            w.message
        );
    }

    #[test]
    fn modifier_order_is_normalised_for_conflict_detection() {
        // alt+super and super+alt are the same combo.
        let r = validate_str("bind = alt+super,k,spawn,foo\nbind = super+alt,k,spawn,bar\n");
        assert!(r.warnings().any(|w| w.code == "W004"));
    }

    #[test]
    fn distinct_combos_do_not_conflict() {
        let r = validate_str("bind = super,j,spawn,foo\nbind = super,k,spawn,bar\n");
        assert!(!r.warnings().any(|w| w.code == "W004"));
    }

    #[test]
    fn arity_broken_bind_is_not_fed_into_conflict_check() {
        // Only 2 fields (E004) — must not spuriously conflict with anything.
        let r = validate_str("bind = super,k\nbind = super,j,spawn,foo\n");
        assert!(!r.warnings().any(|w| w.code == "W004"));
    }

    #[test]
    fn bindc_on_both_sides_suppresses_the_warning() {
        let r = validate_str("bindc = super+alt,k,spawn,foo\nbindc = super+alt,k,spawn,bar\n");
        assert!(!r.warnings().any(|w| w.code == "W004"));
    }

    #[test]
    fn bindc_on_only_one_side_still_warns() {
        let r = validate_str("bindc = super+alt,k,spawn,foo\nbind = super+alt,k,spawn,bar\n");
        assert!(r.warnings().any(|w| w.code == "W004"));
    }

    #[test]
    fn plain_bind_after_a_bindc_pair_still_warns() {
        // Third bind on the same combo didn't opt in — still flagged,
        // and still points back at the original (first) winner.
        let r = validate_str(
            "bindc = super+alt,k,spawn,foo\nbindc = super+alt,k,spawn,bar\nbind = super+alt,k,spawn,baz\n",
        );
        let w = r
            .warnings()
            .find(|w| w.code == "W004")
            .expect("W004 expected");
        assert_eq!(w.line, 3);
        assert!(
            w.message.contains("1"),
            "should point at line 1: {}",
            w.message
        );
    }

    /// Every `parse_option` arm that parses through a typed helper must
    /// be in the matching type table, or a malformed value (e.g. a
    /// decimal comma, `mouse_accel_speed = 0,5`) silently becomes 0
    /// with no W003.
    #[test]
    fn typed_parser_arms_are_in_type_tables() {
        let src = include_str!("parser/options.rs");
        let lines: Vec<&str> = src.lines().collect();
        let start = lines
            .iter()
            .position(|l| l.trim_start().starts_with("pub(super) fn parse_option"))
            .expect("parse_option fn");
        let end = (start..lines.len())
            .find(|&i| {
                lines[i].trim_start().starts_with("other =>")
                    && lines
                        .get(i + 1)
                        .is_some_and(|n| n.contains("unknown config key"))
            })
            .expect("parse_option catch-all");

        let mut arms: Vec<(Vec<String>, String)> = Vec::new();
        for &l in &lines[start..end] {
            let is_arm = l.starts_with("        \"") || l.starts_with("        | \"");
            if is_arm {
                let pat = l.split("=>").next().unwrap_or(l);
                let keys: Vec<String> = pat
                    .split('"')
                    .skip(1)
                    .step_by(2)
                    .map(String::from)
                    .collect();
                let continues = l.starts_with("        | \"")
                    && arms.last().is_some_and(|(_, body)| body.is_empty());
                if !continues {
                    arms.push((Vec::new(), String::new()));
                }
                let arm = arms.last_mut().expect("arm");
                arm.0.extend(keys);
                if let Some((_, rest)) = l.split_once("=>") {
                    arm.1.push_str(rest);
                }
            } else if let Some(arm) = arms.last_mut() {
                arm.1.push_str(l);
            }
        }

        let typed = [
            ("parse_bool(", BOOL_KEYS),
            ("parse_i32(", INT_KEYS),
            ("parse_u32(", UINT_KEYS),
            ("parse_f32(", FLOAT_KEYS),
            ("parse_f64(", FLOAT_KEYS),
        ];
        let mut missing = Vec::new();
        for (keys, body) in &arms {
            let first = typed
                .iter()
                .filter_map(|(f, table)| body.find(f).map(|pos| (pos, *f, *table)))
                .min_by_key(|(pos, _, _)| *pos);
            if let Some((_, f, table)) = first {
                for k in keys {
                    if !table.contains(&k.as_str()) {
                        missing.push(format!("{k} ({f}..)"));
                    }
                }
            }
        }
        assert!(
            missing.is_empty(),
            "typed keys missing from validator tables: {missing:?}"
        );
    }

    #[test]
    fn decimal_comma_on_a_float_key_warns() {
        let r = validate_str("mouse_accel_speed = 0,5\n");
        assert!(r.diagnostics.iter().any(|d| d.code == "W003"));
    }

    #[test]
    fn no_effect_keys_warn_and_are_real_keys() {
        for key in NO_EFFECT_KEYS {
            assert!(
                is_known_scalar_key(key)
                    || is_csv_shaped_key(key)
                    || is_bind_key(key)
                    || *key == "touchgesturebind",
                "{key} is not a key the parser accepts"
            );
        }
        let r = validate_str("tablet_map_to_mon = DP-1\n");
        assert!(r.diagnostics.iter().any(|d| d.code == "W006"));
        let r = validate_str("borderpx = 2\n");
        assert!(!r.diagnostics.iter().any(|d| d.code == "W006"));
    }

    #[test]
    fn implemented_binds_are_known_and_not_no_effect() {
        let r = validate_str("axisbind = super,UP,focusstack,-1\nswitchbind = fold,spawn,true\n");
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == "W001" || d.code == "W006"),
            "{:?}",
            r.diagnostics
        );
    }

    // ── Rule options (W008 / W009) ───────────────────────────────────────

    fn codes(r: &DiagnosticReport) -> Vec<&str> {
        r.diagnostics.iter().map(|d| d.code.as_str()).collect()
    }

    #[test]
    fn unknown_windowrule_matcher_warns_with_hint_and_no_matcher() {
        let r = validate_str("windowrule = isfloating:1, class:^pavucontrol$\n");
        let w = r
            .diagnostics
            .iter()
            .find(|d| d.code == "W008")
            .expect("W008");
        assert!(w.message.contains("`class`"), "{}", w.message);
        assert!(w.message.contains("appid"), "hint missing: {}", w.message);
        // Caret on the bad key, not the whole line.
        assert_eq!(&w.line_text[w.col - 1..w.end_col - 1], "class");
        assert!(codes(&r).contains(&"W009"), "{:?}", codes(&r));
        assert!(!r.has_errors());
    }

    #[test]
    fn layerrule_namespace_suggests_layer_name() {
        let r = validate_str("layerrule = noanim:1, namespace:^waybar$\n");
        let w = r
            .diagnostics
            .iter()
            .find(|d| d.code == "W008")
            .expect("W008");
        assert!(w.message.contains("layer_name"), "{}", w.message);
        assert!(codes(&r).contains(&"W009"), "{:?}", codes(&r));
    }

    #[test]
    fn owners_opacity_rule_is_reported() {
        let r = validate_str("windowrule = opacity:1.0\n");
        let w = r
            .diagnostics
            .iter()
            .find(|d| d.code == "W008")
            .expect("W008");
        assert!(w.message.contains("focused_opacity"), "{}", w.message);
        assert!(codes(&r).contains(&"W009"), "{:?}", codes(&r));
    }

    #[test]
    fn matcherless_rule_without_unknown_keys_is_a_deliberate_catch_all() {
        let r = validate_str("windowrule = isfloating:1\nlayerrule = noblur:1\n");
        assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
    }

    #[test]
    fn every_rule_key_in_the_parser_tables_is_accepted() {
        use crate::parser::{LAYERRULE_KEYS, MONITORRULE_KEYS, TAGRULE_KEYS, WINDOWRULE_KEYS};
        for (rule, keys, base) in [
            ("windowrule", WINDOWRULE_KEYS, "appid:^x$"),
            ("windowrule-once", WINDOWRULE_KEYS, "appid:^x$"),
            ("layerrule", LAYERRULE_KEYS, "layer_name:^x$"),
            ("tagrule", TAGRULE_KEYS, "id:1"),
            ("monitorrule", MONITORRULE_KEYS, "name:DP-1"),
        ] {
            for k in keys {
                let r = validate_str(&format!("{rule} = {base}, {k}:1\n"));
                assert!(
                    !r.diagnostics
                        .iter()
                        .any(|d| d.code == "W008" || d.code == "W009"),
                    "{rule} `{k}` flagged: {:?}",
                    r.diagnostics
                );
            }
        }
    }

    #[test]
    fn unknown_tagrule_and_monitorrule_options_warn() {
        let r = validate_str("tagrule = id:1, layout:tile\nmonitorrule = name:DP-1, scal:2\n");
        let w008: Vec<_> = r.diagnostics.iter().filter(|d| d.code == "W008").collect();
        assert_eq!(w008.len(), 2, "{:?}", r.diagnostics);
        assert!(w008[1].message.contains("scale"), "{}", w008[1].message);
    }

    // ── keymode-aware W004, regex W011 ──────────────────────────────────

    #[test]
    fn same_key_in_two_keymodes_is_not_a_conflict() {
        let r = validate_str(
            "keymode = resize\nbind = none,h,resizewin,-10,0\nkeymode = default\nbind = none,h,focusdir,left\n",
        );
        assert!(
            !r.diagnostics.iter().any(|d| d.code == "W004"),
            "{:?}",
            r.diagnostics
        );
    }

    #[test]
    fn same_key_twice_in_one_keymode_conflicts() {
        let r = validate_str(
            "keymode = resize\nbind = none,h,resizewin,-10,0\nbind = none,h,resizewin,10,0\n",
        );
        let w: Vec<_> = r.diagnostics.iter().filter(|d| d.code == "W004").collect();
        assert_eq!(w.len(), 1, "{:?}", r.diagnostics);
        assert_eq!(w[0].line, 3);
        assert!(w[0].message.contains("resize"), "{}", w[0].message);
    }

    #[test]
    fn an_earlier_common_bind_shadows_every_keymode() {
        // `common` binds fire in every mode, and the first match wins.
        let r = validate_str(
            "keymode = common\nbind = super,q,killclient\nkeymode = resize\nbind = super,q,quit\n",
        );
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.code == "W004" && d.line == 4),
            "{:?}",
            r.diagnostics
        );
        // …but a later common bind is still live in the other modes.
        let r = validate_str(
            "keymode = resize\nbind = super,q,quit\nkeymode = common\nbind = super,q,killclient\n",
        );
        assert!(
            !r.diagnostics.iter().any(|d| d.code == "W004"),
            "{:?}",
            r.diagnostics
        );
    }

    #[test]
    fn invalid_rule_regex_is_a_warning_with_its_own_code() {
        let r = validate_str(
            "windowrule = isfloating:1, appid:^($[\nlayerrule = noanim:1, layer_name:(\n",
        );
        let w: Vec<_> = r.diagnostics.iter().filter(|d| d.code == "W011").collect();
        assert_eq!(w.len(), 2, "{:?}", r.diagnostics);
        assert!(
            !r.has_errors(),
            "margo falls back to a substring match: {:?}",
            r.diagnostics
        );
        assert_eq!(&w[0].line_text[w[0].col - 1..w[0].end_col - 1], "^($[");
    }

    #[test]
    fn valid_rule_regex_is_clean() {
        let r = validate_str("windowrule = isfloating:1, appid:^(foo|bar)$, title:(?i)x\n");
        assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
    }

    #[test]
    fn diagnostic_codes_are_unique_per_meaning() {
        // Every code the validator can emit, with one meaning each — the
        // list in the module docs. A new check must add a new code.
        let src = include_str!("validator.rs");
        let mut codes = std::collections::BTreeSet::new();
        for (i, _) in src.match_indices("code: \"") {
            let rest = &src[i + 7..];
            let c = &rest[..rest.find('"').unwrap()];
            codes.insert(c.to_string());
        }
        let doc: String = src.lines().take_while(|l| l.starts_with("//!")).collect();
        for c in &codes {
            assert!(
                doc.contains(&format!("{c} —")),
                "code {c} is emitted but not documented in the module docs"
            );
        }
    }

    #[test]
    fn boot_diagnostics_reports_the_booted_config() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("config.conf");
        std::fs::write(&main, "borderpx = 2\nwindowrule = opacity:1.0\n").unwrap();
        let d = boot_diagnostics(Some(&main));
        assert!(d.iter().any(|d| d.code == "W008"), "{d:?}");
        assert!(boot_diagnostics(Some(&dir.path().join("missing.conf"))).is_empty());
    }
}
