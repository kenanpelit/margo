//! `mctl config` and `mctl check-config`: errors, generations, diff,
//! rollback and the offline config check.

use anyhow::{Context, Result, bail};

use super::query::read_state_file;
use super::send_dispatch;

pub(super) fn cmd_config_errors() -> Result<()> {
    use margo_config::diagnostics::{ConfigDiagnostic, Severity};
    let state = read_state_file().map_err(|e| anyhow::anyhow!("cannot query margo IPC: {e}"))?;
    let entries = state
        .get("config_errors")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if entries.is_empty() {
        println!("no config errors");
        return Ok(());
    }
    let colored = std::io::IsTerminal::is_terminal(&std::io::stderr());
    let mut errs = 0usize;
    let mut warns = 0usize;
    for entry in &entries {
        let severity = match entry.get("severity").and_then(|v| v.as_str()) {
            Some("error") => {
                errs += 1;
                Severity::Error
            }
            _ => {
                warns += 1;
                Severity::Warning
            }
        };
        let d = ConfigDiagnostic {
            path: entry
                .get("path")
                .and_then(|v| v.as_str())
                .map(std::path::PathBuf::from)
                .unwrap_or_default(),
            line: entry.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
            col: entry.get("col").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
            end_col: entry.get("end_col").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
            severity,
            code: entry
                .get("code")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            message: entry
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            line_text: entry
                .get("line_text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        };
        eprint!("{}", d.render(colored));
        eprintln!();
    }
    println!(
        "result: {} error{}, {} warning{}",
        errs,
        if errs == 1 { "" } else { "s" },
        warns,
        if warns == 1 { "" } else { "s" },
    );
    if errs > 0 {
        std::process::exit(1);
    }
    Ok(())
}

pub(super) fn cmd_config_list(config_override: Option<&std::path::Path>) -> Result<()> {
    let gens = margo_config::generations::list()?;
    if gens.is_empty() {
        println!(
            "no saved generations yet — one is created on the next successful boot or `mctl reload`"
        );
        return Ok(());
    }
    // Best-effort: an unreadable live file just means the "vs current"
    // column is blank for this run, not a hard error for the whole
    // command.
    let live_content = std::fs::read_to_string(resolve_live_config_path(config_override)).ok();
    for (idx, r#gen) in gens.iter().enumerate() {
        let when: chrono::DateTime<chrono::Local> = r#gen.timestamp.into();
        let vs_current = live_content
            .as_deref()
            .and_then(|live| {
                margo_config::generations::read(&r#gen.id)
                    .ok()
                    .map(|gen_content| diff_stat_vs_current(&gen_content, live))
            })
            .unwrap_or_default();
        println!(
            "{idx:>3}  {}  {}  {vs_current}",
            when.format("%Y-%m-%d %H:%M:%S"),
            r#gen.id
        );
    }
    Ok(())
}

/// Short "vs current file" marker for one `mctl config list` row:
/// `(current)` on an exact match, else a `+added/-removed` line-count
/// diff-stat (relative to `gen_content`, matching `cmd_config_diff`'s
/// old=generation / new=live direction).
fn diff_stat_vs_current(gen_content: &str, live_content: &str) -> String {
    if gen_content == live_content {
        return "(current)".to_string();
    }
    let diff = similar::TextDiff::from_lines(gen_content, live_content);
    let mut added = 0usize;
    let mut removed = 0usize;
    for change in diff.iter_all_changes() {
        match change.tag() {
            similar::ChangeTag::Insert => added += 1,
            similar::ChangeTag::Delete => removed += 1,
            similar::ChangeTag::Equal => {}
        }
    }
    format!("+{added}/-{removed}")
}

fn resolve_live_config_path(config_override: Option<&std::path::Path>) -> std::path::PathBuf {
    config_override.map(|p| p.to_path_buf()).unwrap_or_else(|| {
        std::path::PathBuf::from(format!(
            "{}/.config/margo/config.conf",
            std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
        ))
    })
}

pub(super) fn cmd_config_diff(
    n: Option<usize>,
    config_override: Option<&std::path::Path>,
) -> Result<()> {
    let gens = margo_config::generations::list()?;
    let idx = n.unwrap_or(0);
    let r#gen = gens
        .get(idx)
        .ok_or_else(|| anyhow::anyhow!("no generation at index {idx} (have {})", gens.len()))?;
    let old = margo_config::generations::read(&r#gen.id)?;
    let live_path = resolve_live_config_path(config_override);
    let new = std::fs::read_to_string(&live_path)
        .with_context(|| format!("reading {}", live_path.display()))?;
    let diff = similar::TextDiff::from_lines(&old, &new);
    print!(
        "{}",
        diff.unified_diff().header(
            &format!("generation {}", r#gen.id),
            &live_path.display().to_string()
        )
    );
    Ok(())
}

/// Default rollback index when the user didn't pass `N` explicitly.
///
/// `0` (the last known-good generation) if the live file's content
/// differs from the newest saved generation's — i.e. the live file
/// carries an edit that was never itself successfully saved as a
/// generation (a failed reload, or a boot-time parse failure): that
/// newest generation IS the last-known-good state, so the default
/// must not skip past it. `1` (one generation before the newest) if
/// they match — the classic "reloaded fine, now I dislike it" case.
/// Falls back to `1` (today's pre-fix behavior) whenever there isn't
/// enough information to compare — `get(1)` errors clearly if that's
/// out of range, same as before this fix.
fn default_rollback_index(
    live_content: Option<&str>,
    newest_generation_content: Option<&str>,
) -> usize {
    match (live_content, newest_generation_content) {
        (Some(live), Some(newest)) if live != newest => 0,
        _ => 1,
    }
}

pub(super) fn cmd_config_rollback(
    n: Option<usize>,
    yes: bool,
    config_override: Option<&std::path::Path>,
) -> Result<()> {
    let gens = margo_config::generations::list()?;
    let live_path = resolve_live_config_path(config_override);

    let idx = match n {
        Some(n) => n,
        None => {
            let live_content = std::fs::read_to_string(&live_path).ok();
            let newest_content = gens
                .first()
                .and_then(|g| margo_config::generations::read(&g.id).ok());
            default_rollback_index(live_content.as_deref(), newest_content.as_deref())
        }
    };
    let r#gen = gens
        .get(idx)
        .ok_or_else(|| anyhow::anyhow!("no generation at index {idx} (have {})", gens.len()))?;
    let content = margo_config::generations::read(&r#gen.id)?;

    // Safety check: run the real validator (not the permissive parser)
    // against the candidate content before writing it — a generation
    // can be parseable-but-validator-rejected (the boot-path save
    // never runs the validator, only the parser). Written to a temp
    // file *in the same directory* as the live path so relative
    // `source = ...` lines resolve exactly as they would for the live
    // file itself.
    let tmp_path = live_path
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join(format!(".mctl-rollback-check-{}.conf", std::process::id()));
    std::fs::write(&tmp_path, &content)
        .with_context(|| format!("writing temp file {}", tmp_path.display()))?;
    let validated = margo_config::validator::validate_config(Some(&tmp_path));
    let _ = std::fs::remove_file(&tmp_path);
    let report =
        validated.with_context(|| format!("validating generation {} before rollback", r#gen.id))?;
    if report.has_errors() {
        let err_count = report.errors().count();
        bail!(
            "generation {} fails validation ({err_count} error{}); refusing to roll back to it.",
            r#gen.id,
            if err_count == 1 { "" } else { "s" },
        );
    }

    let is_symlink = std::fs::symlink_metadata(&live_path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    if is_symlink && !yes {
        let target = std::fs::read_link(&live_path).unwrap_or_default();
        eprintln!(
            "{} is a symlink to {} — rollback writes through it, changing that file.",
            live_path.display(),
            target.display()
        );
        eprint!("Continue? [y/N] ");
        use std::io::Write;
        std::io::stdout().flush().ok();
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !answer.trim().eq_ignore_ascii_case("y") {
            println!("rollback cancelled");
            return Ok(());
        }
    }

    // Back up whatever's currently on disk before overwriting it. This
    // is the one write path that can destroy content that was never
    // itself saved as a generation (a failed reload, or a boot-time
    // fallback edit) — deliberately NOT via `generations::save()`:
    // that store is the boot-fallback's trusted "known-good" source,
    // and this content is exactly what's about to be replaced
    // (possibly for being broken); seeding it here would poison the
    // next fallback.
    if let Ok(current) = std::fs::read_to_string(&live_path) {
        let mut backup_name = live_path.as_os_str().to_owned();
        backup_name.push(".bak");
        let backup_path = std::path::PathBuf::from(backup_name);
        std::fs::write(&backup_path, &current)
            .with_context(|| format!("writing backup {}", backup_path.display()))?;
        println!("backed up current config to {}", backup_path.display());
    }

    std::fs::write(&live_path, &content)
        .with_context(|| format!("writing {}", live_path.display()))?;
    println!(
        "restored generation {} to {}",
        r#gen.id,
        live_path.display()
    );
    send_dispatch("reload", &[])?;
    println!("reloaded");
    Ok(())
}

pub(super) fn cmd_check_config(
    config_override: Option<&std::path::Path>,
    strict: bool,
) -> Result<()> {
    use margo_config::diagnostics::Severity;

    let cfg_path = config_override.map(|p| p.to_path_buf()).unwrap_or_else(|| {
        std::path::PathBuf::from(format!(
            "{}/.config/margo/config.conf",
            std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
        ))
    });

    if !cfg_path.exists() {
        // A missing config is NOT an error — margo falls back to built-in
        // defaults. Treat it as a clean pass so the start-margo preflight
        // doesn't refuse to launch on a first login (before any config or
        // dotfiles exist). Only a config that *exists but is broken* fails.
        println!(
            "config OK: no file at {} — margo will use built-in defaults",
            cfg_path.display()
        );
        return Ok(());
    }

    // Pass 1 — the structured validator over the whole `source` tree:
    // unknown keys and rule options, bad values, stray commas, unresolved
    // includes, rule regexes, duplicate binds (keymode-aware, first wins).
    let mut report =
        margo_config::validator::validate_config(Some(&cfg_path)).unwrap_or_else(|e| {
            eprintln!("error: validator could not read config: {e}");
            std::process::exit(2);
        });

    // Pass 2 — try to fully parse for the summary line. If the parser
    // itself bails (rare; it's permissive) record an E000 diagnostic
    // and keep going so the rest of the diagnostic stream still prints.
    let cfg = margo_config::parse_config(Some(&cfg_path)).ok();

    // Stable render order: by file, line, then severity (errors first).
    report.diagnostics.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(a.line.cmp(&b.line))
            .then(a.col.cmp(&b.col))
            .then_with(|| match (a.severity, b.severity) {
                (Severity::Error, Severity::Warning) => std::cmp::Ordering::Less,
                (Severity::Warning, Severity::Error) => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            })
    });

    let colored = std::io::IsTerminal::is_terminal(&std::io::stderr());
    let err_count = report.errors().count();
    let warn_count = report.warnings().count();

    for d in &report.diagnostics {
        eprint!("{}", d.render(colored));
        eprintln!();
    }

    println!("config: {}", cfg_path.display());
    if let Some(cfg) = cfg.as_ref() {
        println!(
            "summary: binds={} windowrules={} layerrules={} monitorrules={} tagrules={}",
            cfg.key_bindings.len(),
            cfg.window_rules.len(),
            cfg.layer_rules.len(),
            cfg.monitor_rules.len(),
            cfg.tag_rules.len(),
        );
    }

    if err_count == 0 && warn_count == 0 {
        println!("✓ no problems detected");
        return Ok(());
    }
    println!(
        "result: {} error{}, {} warning{}",
        err_count,
        if err_count == 1 { "" } else { "s" },
        warn_count,
        if warn_count == 1 { "" } else { "s" },
    );
    if err_count > 0 || (strict && warn_count > 0) {
        if err_count == 0 {
            println!("--strict: warnings count as failures");
        }
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── default_rollback_index (Fix 1) ──────────────────────────────

    #[test]
    fn default_rollback_index_is_zero_when_live_differs_from_newest() {
        // The live file carries an unsaved edit (failed reload, or
        // boot fallback) — the newest generation IS the last
        // known-good state, so the default must not skip past it.
        assert_eq!(default_rollback_index(Some("a"), Some("b")), 0);
    }

    #[test]
    fn default_rollback_index_is_one_when_live_matches_newest() {
        // Nothing's changed since the last successful save — go back
        // one further, past the state the user says they dislike.
        assert_eq!(default_rollback_index(Some("a"), Some("a")), 1);
    }

    #[test]
    fn default_rollback_index_falls_back_to_one_when_live_unreadable() {
        assert_eq!(default_rollback_index(None, Some("a")), 1);
    }

    #[test]
    fn default_rollback_index_falls_back_to_one_when_no_generations() {
        assert_eq!(default_rollback_index(Some("a"), None), 1);
    }

    // ── diff_stat_vs_current (Fix 6) ────────────────────────────────

    #[test]
    fn diff_stat_vs_current_marks_exact_match() {
        assert_eq!(diff_stat_vs_current("a\nb\n", "a\nb\n"), "(current)");
    }

    #[test]
    fn diff_stat_vs_current_counts_added_and_removed_lines() {
        let stat = diff_stat_vs_current("a\nb\n", "a\nc\n");
        assert_eq!(stat, "+1/-1");
    }
}
