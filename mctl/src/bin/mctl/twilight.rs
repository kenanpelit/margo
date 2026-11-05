//! `mctl twilight`: status, presets and the schedule files.

use anyhow::Result;

use super::cli::{TwilightPresetCmd, TwilightScheduleCmd};
use super::query::read_state_file;
use super::send_dispatch;

pub(super) fn cmd_twilight_status(as_json: bool) -> Result<()> {
    let state = read_state_file().map_err(|e| anyhow::anyhow!("cannot query margo IPC: {e}"))?;
    let tw = state
        .get("twilight")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    if tw.is_null() {
        eprintln!("twilight: not advertised by this compositor (older margo?)");
        std::process::exit(1);
    }
    if as_json {
        println!("{}", serde_json::to_string_pretty(&tw)?);
        return Ok(());
    }
    let enabled = tw.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
    let mode = tw.get("mode").and_then(|v| v.as_str()).unwrap_or("?");
    let phase = tw.get("phase").and_then(|v| v.as_str()).unwrap_or("?");
    let source = tw.get("source").and_then(|v| v.as_str()).unwrap_or("?");
    let cur_k = tw.get("current_temp_k").and_then(|v| v.as_u64());
    let cur_g = tw.get("current_gamma_pct").and_then(|v| v.as_u64());
    let day_k = tw.get("day_temp_k").and_then(|v| v.as_u64()).unwrap_or(0);
    let nig_k = tw.get("night_temp_k").and_then(|v| v.as_u64()).unwrap_or(0);
    let day_g = tw
        .get("day_gamma_pct")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let nig_g = tw
        .get("night_gamma_pct")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let colored = std::io::IsTerminal::is_terminal(&std::io::stdout());
    let (b, dim, r) = if colored {
        ("\x1b[1m", "\x1b[2m", "\x1b[0m")
    } else {
        ("", "", "")
    };
    println!(
        "{b}twilight{r}: {} ({dim}{}{r})",
        if enabled { "ON" } else { "off" },
        mode
    );
    println!("  {dim}phase{r}   = {phase}    {dim}source{r} = {source}");
    match (cur_k, cur_g) {
        (Some(k), Some(g)) => {
            println!("  {dim}current{r} = {b}{k}K{r} @ {b}{g}%{r}");
        }
        _ => {
            println!("  {dim}current{r} = (none — gamma untouched)");
        }
    }
    println!("  {dim}day{r}     = {day_k}K @ {day_g}%");
    println!("  {dim}night{r}   = {nig_k}K @ {nig_g}%");
    Ok(())
}

// ── twilight preset management ────────────────────────────────────────────────
//
// The on-disk layout (`margo/src/twilight/preset.rs`) is:
//   <dir>/
//     schedule.conf          # `HH:MM <preset-name>` lines, # for comments
//     presets/<name>.toml    # `static_temp = K`, `static_gamma = %`
//
// All preset subcommands work on these files directly — no IPC dance
// for reads, just a best-effort `reload_config` dispatch at the end of
// writes so the compositor picks up the change without `mctl reload`.

fn twilight_dir() -> std::path::PathBuf {
    // Match the compositor's `default_dir()` from
    // `margo/src/twilight/preset.rs`. If the user has overridden
    // `twilight_schedule_dir` in their config, state snapshot doesn't
    // currently expose it — they'll need `mctl reload` after manual
    // edits in that custom location (uncommon enough to defer).
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return std::path::PathBuf::from(xdg).join("margo").join("twilight");
    }
    if let Ok(home) = std::env::var("HOME") {
        return std::path::PathBuf::from(home)
            .join(".config")
            .join("margo")
            .join("twilight");
    }
    std::path::PathBuf::from(".config/margo/twilight")
}

pub(super) fn cmd_twilight_preset(action: &TwilightPresetCmd) -> Result<()> {
    let dir = twilight_dir();
    match action {
        TwilightPresetCmd::List => cmd_twilight_preset_list(&dir),
        TwilightPresetCmd::Show { name } => cmd_twilight_preset_show(&dir, name),
        TwilightPresetCmd::Set { name, temp, gamma } => {
            cmd_twilight_preset_set(&dir, name, *temp, *gamma)
        }
        TwilightPresetCmd::Remove { name } => cmd_twilight_preset_remove(&dir, name),
        TwilightPresetCmd::Schedule { action } => match action {
            TwilightScheduleCmd::Set { time, name } => cmd_twilight_schedule_set(&dir, time, name),
            TwilightScheduleCmd::Remove { time } => cmd_twilight_schedule_remove(&dir, time),
        },
    }
}

fn cmd_twilight_preset_list(dir: &std::path::Path) -> Result<()> {
    let presets_dir = dir.join("presets");
    let schedule_path = dir.join("schedule.conf");
    let colored = std::io::IsTerminal::is_terminal(&std::io::stdout());
    let (b, dim, r) = if colored {
        ("\x1b[1m", "\x1b[2m", "\x1b[0m")
    } else {
        ("", "", "")
    };

    println!("{b}presets{r} ({dim}{}{r})", presets_dir.display());
    let mut names: Vec<(String, Option<(u32, u32)>)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&presets_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) != Some("toml") {
                continue;
            }
            let name = p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let parsed = read_preset_file(&p);
            names.push((name, parsed));
        }
    }
    names.sort_by(|a, b| a.0.cmp(&b.0));
    if names.is_empty() {
        println!("  {dim}(none — run `mctl twilight preset set <name> <K>` to create one){r}");
    } else {
        for (name, parsed) in &names {
            match parsed {
                Some((k, g)) => println!("  {b}{name:<16}{r}  {k}K @ {g}%"),
                None => println!("  {b}{name:<16}{r}  {dim}(unreadable){r}"),
            }
        }
    }

    println!();
    println!("{b}schedule{r} ({dim}{}{r})", schedule_path.display());
    match std::fs::read_to_string(&schedule_path) {
        Ok(text) => {
            let mut lines: Vec<(String, String)> = Vec::new();
            for line in text.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                let mut parts = trimmed.split_whitespace();
                let Some(time) = parts.next() else { continue };
                let Some(name) = parts.next() else { continue };
                lines.push((time.to_string(), name.to_string()));
            }
            if lines.is_empty() {
                println!("  {dim}(empty){r}");
            } else {
                for (time, name) in &lines {
                    println!("  {b}{time}{r}  {name}");
                }
            }
        }
        Err(_) => {
            println!("  {dim}(no schedule.conf){r}");
        }
    }
    Ok(())
}

fn cmd_twilight_preset_show(dir: &std::path::Path, name: &str) -> Result<()> {
    let path = dir.join("presets").join(format!("{name}.toml"));
    if !path.exists() {
        anyhow::bail!("preset {name:?} not found at {}", path.display());
    }
    let (temp, gamma) = read_preset_file(&path)
        .ok_or_else(|| anyhow::anyhow!("preset {name:?} unparseable at {}", path.display()))?;
    println!("{name}: {temp}K @ {gamma}%");
    println!("  path = {}", path.display());
    Ok(())
}

fn cmd_twilight_preset_set(dir: &std::path::Path, name: &str, temp: u32, gamma: u32) -> Result<()> {
    validate_preset_name(name)?;
    if !(1000..=25000).contains(&temp) {
        anyhow::bail!("temp must be 1000–25000, got {temp}");
    }
    if !(10..=200).contains(&gamma) {
        anyhow::bail!("gamma must be 10–200, got {gamma}");
    }
    let presets_dir = dir.join("presets");
    std::fs::create_dir_all(&presets_dir)
        .map_err(|e| anyhow::anyhow!("cannot create {}: {e}", presets_dir.display()))?;
    let path = presets_dir.join(format!("{name}.toml"));
    let body = format!(
        "# margo twilight preset\n\
         # static_temp: Kelvin (1000–25000)\n\
         # static_gamma: brightness % (10–200)\n\
         static_temp = {temp}\n\
         static_gamma = {gamma}\n"
    );
    std::fs::write(&path, body)
        .map_err(|e| anyhow::anyhow!("cannot write {}: {e}", path.display()))?;
    println!("wrote {} ({temp}K @ {gamma}%)", path.display());
    request_reload();
    Ok(())
}

fn cmd_twilight_preset_remove(dir: &std::path::Path, name: &str) -> Result<()> {
    validate_preset_name(name)?;
    let path = dir.join("presets").join(format!("{name}.toml"));
    if !path.exists() {
        anyhow::bail!("preset {name:?} not found at {}", path.display());
    }
    std::fs::remove_file(&path)
        .map_err(|e| anyhow::anyhow!("cannot delete {}: {e}", path.display()))?;
    println!("removed {}", path.display());

    // Strip any schedule lines that referenced this preset so the
    // schedule never points at a missing file (the compositor would
    // skip them with a warn, but a clean schedule file is friendlier).
    let schedule_path = dir.join("schedule.conf");
    if let Ok(text) = std::fs::read_to_string(&schedule_path) {
        let mut kept: Vec<String> = Vec::new();
        let mut dropped = 0;
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                kept.push(line.to_string());
                continue;
            }
            let mut parts = trimmed.split_whitespace();
            let _time = parts.next();
            let ref_name = parts.next().unwrap_or("");
            if ref_name == name {
                dropped += 1;
            } else {
                kept.push(line.to_string());
            }
        }
        if dropped > 0 {
            let mut out = kept.join("\n");
            if !out.ends_with('\n') {
                out.push('\n');
            }
            std::fs::write(&schedule_path, out)
                .map_err(|e| anyhow::anyhow!("cannot rewrite {}: {e}", schedule_path.display()))?;
            println!("removed {dropped} schedule line(s) referencing {name:?}");
        }
    }
    request_reload();
    Ok(())
}

fn cmd_twilight_schedule_set(dir: &std::path::Path, time: &str, name: &str) -> Result<()> {
    let normalized = normalize_hhmm(time)?;
    validate_preset_name(name)?;
    // Guard against pointing the schedule at a preset that doesn't
    // exist yet — the compositor would just warn-and-skip but it's
    // a friendlier error if mctl catches it.
    let preset_path = dir.join("presets").join(format!("{name}.toml"));
    if !preset_path.exists() {
        anyhow::bail!(
            "preset {name:?} not found at {}\n\
             create it first: mctl twilight preset set {name} <K>",
            preset_path.display()
        );
    }
    let schedule_path = dir.join("schedule.conf");
    std::fs::create_dir_all(dir)
        .map_err(|e| anyhow::anyhow!("cannot create {}: {e}", dir.display()))?;
    let existing = std::fs::read_to_string(&schedule_path).unwrap_or_default();
    let mut kept: Vec<String> = Vec::new();
    let mut replaced = false;
    for line in existing.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            kept.push(line.to_string());
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(t) = parts.next() else {
            kept.push(line.to_string());
            continue;
        };
        if normalize_hhmm(t).ok().as_deref() == Some(&normalized) {
            kept.push(format!("{normalized} {name}"));
            replaced = true;
        } else {
            kept.push(line.to_string());
        }
    }
    if !replaced {
        kept.push(format!("{normalized} {name}"));
    }
    let mut out = kept.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    std::fs::write(&schedule_path, out)
        .map_err(|e| anyhow::anyhow!("cannot write {}: {e}", schedule_path.display()))?;
    if replaced {
        println!("updated {normalized} → {name}");
    } else {
        println!("added {normalized} → {name}");
    }
    request_reload();
    Ok(())
}

fn cmd_twilight_schedule_remove(dir: &std::path::Path, time: &str) -> Result<()> {
    let normalized = normalize_hhmm(time)?;
    let schedule_path = dir.join("schedule.conf");
    let existing = std::fs::read_to_string(&schedule_path)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", schedule_path.display()))?;
    let mut kept: Vec<String> = Vec::new();
    let mut dropped = false;
    for line in existing.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            kept.push(line.to_string());
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(t) = parts.next() else {
            kept.push(line.to_string());
            continue;
        };
        if normalize_hhmm(t).ok().as_deref() == Some(&normalized) {
            dropped = true;
        } else {
            kept.push(line.to_string());
        }
    }
    if !dropped {
        anyhow::bail!("no schedule line found for {normalized}");
    }
    let mut out = kept.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    std::fs::write(&schedule_path, out)
        .map_err(|e| anyhow::anyhow!("cannot rewrite {}: {e}", schedule_path.display()))?;
    println!("removed schedule line at {normalized}");
    request_reload();
    Ok(())
}

fn read_preset_file(path: &std::path::Path) -> Option<(u32, u32)> {
    // Mirror the compositor-side parser in
    // `margo/src/twilight/preset.rs::load_preset_file` so the two
    // stay in sync — same two keys, same clamps.
    let text = std::fs::read_to_string(path).ok()?;
    let mut temp_k: Option<u32> = None;
    let mut gamma_pct: Option<u32> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let key = k.trim();
        let val = v.trim().trim_matches('"');
        match key {
            "static_temp" => {
                if let Ok(n) = val.parse::<u32>() {
                    temp_k = Some(n.clamp(1000, 25000));
                }
            }
            "static_gamma" => {
                if let Ok(n) = val.parse::<u32>() {
                    gamma_pct = Some(n.clamp(10, 200));
                }
            }
            _ => {}
        }
    }
    Some((temp_k?, gamma_pct?))
}

fn validate_preset_name(name: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("preset name cannot be empty");
    }
    if name.contains('/') || name.contains('\\') || name.starts_with('.') {
        anyhow::bail!("preset name {name:?} is invalid — no slashes, no leading dot");
    }
    Ok(())
}

fn normalize_hhmm(s: &str) -> Result<String> {
    let (h, m) = s
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("time must be HH:MM, got {s:?}"))?;
    let h: u32 = h
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("bad hour in {s:?}"))?;
    let m: u32 = m
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("bad minute in {s:?}"))?;
    if h >= 24 || m >= 60 {
        anyhow::bail!("HH must be 0–23 and MM 0–59, got {s:?}");
    }
    Ok(format!("{h:02}:{m:02}"))
}

/// Best-effort `reload_config` dispatch so a preset edit takes
/// effect without the user having to run `mctl reload`. Failures
/// are silent (compositor not running, no Wayland socket, etc.) —
/// the file write already succeeded, and `mctl reload` later will
/// finish the job.
fn request_reload() {
    // Best-effort fire-and-forget reload over the IPC socket. Any
    // failure (compositor not running, no socket) is ignored — the
    // file write already happened and `mctl reload` later closes the loop.
    let _ = send_dispatch("reload_config", &[]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hhmm_is_normalised_and_range_checked() {
        assert_eq!(normalize_hhmm("7:5").unwrap(), "07:05");
        assert_eq!(normalize_hhmm(" 23 : 59 ").unwrap(), "23:59");
        assert_eq!(normalize_hhmm("00:00").unwrap(), "00:00");
        for bad in ["24:00", "12:60", "1200", "", ":", "a:b", "-1:00", "12:5:00"] {
            assert!(normalize_hhmm(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn preset_names_cannot_escape_the_directory() {
        assert!(validate_preset_name("night").is_ok());
        assert!(validate_preset_name("warm-2").is_ok());
        for bad in ["", "../x", "a/b", "a\\b", ".hidden", "..", "."] {
            assert!(validate_preset_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn preset_file_needs_both_keys_and_clamps() {
        let dir = std::env::temp_dir().join(format!("mctl-preset-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, body: &str| {
            let p = dir.join(name);
            std::fs::write(&p, body).unwrap();
            p
        };
        let p = write(
            "ok.conf",
            "# night\n[preset]\nstatic_temp = 3400\nstatic_gamma = \"90\"\n",
        );
        assert_eq!(read_preset_file(&p), Some((3400, 90)));
        let p = write("clamp.conf", "static_temp=100\nstatic_gamma=999\n");
        assert_eq!(read_preset_file(&p), Some((1000, 200)));
        let p = write("half.conf", "static_temp=4000\n");
        assert_eq!(read_preset_file(&p), None);
        let p = write("bad.conf", "static_temp=warm\nstatic_gamma=90\n");
        assert_eq!(read_preset_file(&p), None);
        assert_eq!(read_preset_file(&dir.join("missing.conf")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
