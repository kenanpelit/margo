//! Read-only state queries: `status`, `clients`, `outputs`, `focused`,
//! `perf` and `mirror`.

use anyhow::{Context, Result};

use super::cli::MirrorCmd;
use super::dispatch_request;

pub(super) fn cmd_mirror(action: &MirrorCmd) -> Result<()> {
    let (dispatch, source, target) = match action {
        MirrorCmd::Status { json } => {
            let m = mctl::ipc_client::request_once("get mirror")
                .map_err(|e| anyhow::anyhow!("cannot query margo IPC: {e}"))?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&m)?);
            } else {
                println!("{}", mirror_status_line(&m));
            }
            return Ok(());
        }
        MirrorCmd::Stop => ("stop_mirror", None, None),
        MirrorCmd::Start { source, target } => {
            ("mirror_output", source.as_deref(), target.as_deref())
        }
        MirrorCmd::Toggle { source, target } => {
            ("toggle_mirror", source.as_deref(), target.as_deref())
        }
    };
    // One name is the source (`start DP-3`: mirror DP-3 somewhere).
    let args: Vec<&str> = [source, target].into_iter().flatten().collect();
    // A refusal comes back in this request's own reply. `mirror.error`
    // is last-writer-wins across clients: reading it could report
    // another client's refusal.
    let reply = mctl::ipc_client::request_once(&dispatch_request(dispatch, &args))?;
    if let Some(e) = mctl::ipc_client::dispatch_reply_error(&reply) {
        anyhow::bail!("mirror: {e}");
    }
    let m = mctl::ipc_client::request_once("get mirror")
        .map_err(|e| anyhow::anyhow!("cannot query margo IPC: {e}"))?;
    println!("{}", mirror_status_line(&m));
    Ok(())
}

/// One-line summary of the `mirror` IPC object.
fn mirror_status_line(m: &serde_json::Value) -> String {
    match (
        m["active"]["source"].as_str(),
        m["active"]["target"].as_str(),
    ) {
        (Some(s), Some(t)) => format!("mirroring {s} → {t}"),
        _ if m.get("active").is_none() => {
            "mirror: not advertised by this compositor (older margo?)".into()
        }
        _ => "not mirroring".into(),
    }
}

/// Render `mctl status` from the richer state snapshot snapshot —
/// per-output blocks with proper connector names + tag client
/// counts. Used when the file exists (post-r143 margo).
pub(super) fn print_status_rich(state: &serde_json::Value, output_filter: Option<&str>) {
    use std::io::IsTerminal;
    let tty = std::io::stdout().is_terminal();
    let bold = if tty { "\x1b[1m" } else { "" };
    let dim = if tty { "\x1b[2m" } else { "" };
    let cyan = if tty { "\x1b[36m" } else { "" };
    let yellow = if tty { "\x1b[33m" } else { "" };
    let green = if tty { "\x1b[32m" } else { "" };
    let red = if tty { "\x1b[31m" } else { "" };
    let reset = if tty { "\x1b[0m" } else { "" };

    let outputs = match state["outputs"].as_array() {
        Some(o) => o,
        None => return,
    };
    let clients = state["clients"]
        .as_array()
        .map(|v| v.as_slice())
        .unwrap_or(&[]);
    let layout_names: Vec<String> = state["layouts"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let tag_count = state["tag_count"].as_u64().unwrap_or(9) as usize;

    let mut printed_any = false;
    for out in outputs {
        let name = out["name"].as_str().unwrap_or("");
        if let Some(filter) = output_filter
            && name != filter
        {
            continue;
        }
        let active = out["active"].as_bool().unwrap_or(false);
        let layout_idx = out["layout_idx"].as_u64().unwrap_or(0) as usize;
        let layout = layout_names
            .get(layout_idx)
            .map(String::as_str)
            .unwrap_or("?");

        let active_marker = if active {
            format!("{green}●{reset} ")
        } else {
            "  ".to_string()
        };
        if printed_any {
            println!();
        }
        printed_any = true;
        println!("{active_marker}{bold}{name}{reset}  {dim}layout {reset}{cyan}{layout}{reset}");

        // Focused window on this output (find from clients array).
        let mon_idx = out["x"].as_i64(); // unused — we match by name
        let _ = mon_idx;
        let focused = clients
            .iter()
            .find(|c| c["focused"].as_bool() == Some(true) && c["monitor"].as_str() == Some(name));
        if let Some(c) = focused {
            let app = c["app_id"].as_str().unwrap_or("");
            let title = c["title"].as_str().unwrap_or("");
            let mut flags = Vec::new();
            if c["fullscreen"].as_bool().unwrap_or(false) {
                flags.push(format!("{red}FULLSCREEN{reset}"));
            }
            if c["floating"].as_bool().unwrap_or(false) {
                flags.push(format!("{yellow}FLOAT{reset}"));
            }
            let flags_str = if flags.is_empty() {
                String::new()
            } else {
                format!("  {}", flags.join(" "))
            };
            println!("    {dim}focused{reset}: {bold}{app}{reset} · {title}{flags_str}");
            let w = c["width"].as_i64().unwrap_or(0);
            let h = c["height"].as_i64().unwrap_or(0);
            if w > 0 && h > 0 {
                let x = c["x"].as_i64().unwrap_or(0);
                let y = c["y"].as_i64().unwrap_or(0);
                println!("    {dim}geometry{reset}: {w}×{h} @ {x},{y}");
            }
        } else {
            println!("    {dim}focused{reset}: (none)");
        }

        // Tag row: count clients per tag on this output.
        let active_tag = out["active_tag_mask"].as_u64().unwrap_or(0) as u32;
        let mut counts = vec![0u32; tag_count];
        for c in clients {
            if c["monitor"].as_str() != Some(name) {
                continue;
            }
            let tags = c["tags"].as_u64().unwrap_or(0) as u32;
            for (i, slot) in counts.iter_mut().enumerate() {
                if tags & (1 << i) != 0 {
                    *slot += 1;
                }
            }
        }
        let mut row = String::new();
        for (i, count) in counts.iter().enumerate() {
            let n = i + 1;
            let on = active_tag & (1 << i) != 0;
            let label = format!("{n}·{}", count);
            let cell = if on {
                let any_focused_here = clients.iter().any(|c| {
                    c["focused"].as_bool() == Some(true)
                        && c["monitor"].as_str() == Some(name)
                        && (c["tags"].as_u64().unwrap_or(0) as u32 & (1 << i)) != 0
                });
                if any_focused_here {
                    format!("{green}[{label}]●{reset}")
                } else {
                    format!("{cyan}[{label}]{reset}")
                }
            } else if counts[i] > 0 {
                label.clone()
            } else {
                format!("{dim}{label}{reset}")
            };
            if !row.is_empty() {
                row.push_str("  ");
            }
            row.push_str(&cell);
        }
        println!("    {dim}tags{reset}: {row}");
    }

    if printed_any {
        println!();
        println!(
            "{dim}layouts:{reset} {}",
            layout_names
                .iter()
                .enumerate()
                .map(|(i, l)| format!("{i}:{l}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    } else if let Some(f) = output_filter {
        eprintln!("(no output named `{f}`)");
    }
}

pub(super) fn read_state_file() -> Result<serde_json::Value> {
    // Query the compositor's IPC socket for the full state snapshot.
    // (Same document margo used to dump to state snapshot — the function
    // name is kept so every caller stays unchanged.)
    mctl::ipc_client::request_once("get state").with_context(|| {
        format!(
            "query margo IPC socket {}: is margo running?",
            mctl::ipc_client::socket_path().display()
        )
    })
}

pub(super) fn cmd_clients(
    json_out: bool,
    tag_filter: Option<u32>,
    monitor_filter: Option<&str>,
    appid_filter: Option<&str>,
    wide: bool,
) -> Result<()> {
    use std::io::IsTerminal;
    let state = read_state_file()?;
    let clients = state["clients"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("state file missing `clients` array"))?;

    let want_tag_mask = tag_filter.map(|n| 1u32 << (n.saturating_sub(1).min(31)));

    let filtered: Vec<&serde_json::Value> = clients
        .iter()
        .filter(|c| {
            if let Some(mask) = want_tag_mask {
                let tags = c["tags"].as_u64().unwrap_or(0) as u32;
                if tags & mask == 0 {
                    return false;
                }
            }
            if let Some(mon) = monitor_filter
                && c["monitor"].as_str().unwrap_or("") != mon
            {
                return false;
            }
            if let Some(needle) = appid_filter {
                let needle = needle.to_lowercase();
                let app = c["app_id"].as_str().unwrap_or("").to_lowercase();
                if !app.contains(&needle) {
                    return false;
                }
            }
            true
        })
        .collect();

    if json_out {
        let arr: Vec<_> = filtered.iter().cloned().cloned().collect();
        println!("{}", serde_json::to_string_pretty(&arr)?);
        return Ok(());
    }

    if filtered.is_empty() {
        println!("(no matching clients)");
        return Ok(());
    }

    let tty = std::io::stdout().is_terminal();
    let bold = if tty { "\x1b[1m" } else { "" };
    let dim = if tty { "\x1b[2m" } else { "" };
    let green = if tty { "\x1b[32m" } else { "" };
    let yellow = if tty { "\x1b[33m" } else { "" };
    let red = if tty { "\x1b[31m" } else { "" };
    let reset = if tty { "\x1b[0m" } else { "" };

    // Decode tag bitmask → comma list of tag numbers.
    let decode_tags = |mask: u32| -> String {
        let mut v = Vec::new();
        for i in 0..9 {
            if mask & (1 << i) != 0 {
                v.push(format!("{}", i + 1));
            }
        }
        if v.is_empty() {
            "—".to_string()
        } else {
            v.join(",")
        }
    };

    // Compute column widths.
    let max_tag = filtered
        .iter()
        .map(|c| decode_tags(c["tags"].as_u64().unwrap_or(0) as u32).len())
        .max()
        .unwrap_or(3)
        .max(3);
    let max_mon = filtered
        .iter()
        .map(|c| c["monitor"].as_str().unwrap_or("").len())
        .max()
        .unwrap_or(3)
        .max(3);
    let max_app = filtered
        .iter()
        .map(|c| c["app_id"].as_str().unwrap_or("").len())
        .max()
        .unwrap_or(6)
        .clamp(6, 28);

    // Header.
    if wide {
        println!(
            "{bold}{:<w_tag$}  {:<w_mon$}  {:<w_app$}  {:<22}  TITLE{reset}",
            "TAG",
            "MON",
            "APP-ID",
            "GEOMETRY",
            w_tag = max_tag,
            w_mon = max_mon,
            w_app = max_app,
        );
    } else {
        println!(
            "{bold}{:<w_tag$}  {:<w_mon$}  {:<w_app$}  TITLE{reset}",
            "TAG",
            "MON",
            "APP-ID",
            w_tag = max_tag,
            w_mon = max_mon,
            w_app = max_app,
        );
    }

    for c in &filtered {
        let tags = decode_tags(c["tags"].as_u64().unwrap_or(0) as u32);
        let mon = c["monitor"].as_str().unwrap_or("");
        let app = c["app_id"].as_str().unwrap_or("");
        let title = c["title"].as_str().unwrap_or("");
        let geom = format!(
            "{}×{}+{}+{}",
            c["width"].as_i64().unwrap_or(0),
            c["height"].as_i64().unwrap_or(0),
            c["x"].as_i64().unwrap_or(0),
            c["y"].as_i64().unwrap_or(0),
        );
        let mut markers = String::new();
        if c["focused"].as_bool().unwrap_or(false) {
            markers.push_str(&format!("{green}●{reset} "));
        }
        if c["fullscreen"].as_bool().unwrap_or(false) {
            markers.push_str(&format!("{red}⛶{reset} "));
        }
        if c["floating"].as_bool().unwrap_or(false) {
            markers.push_str(&format!("{yellow}⬚{reset} "));
        }
        if c["minimized"].as_bool().unwrap_or(false) {
            markers.push_str(&format!("{dim}↓{reset} "));
        }
        // ★ marker: this client is currently being scanned out
        // directly from a primary/overlay plane (zero-copy). Cheap
        // signal that compositor blending overhead is bypassed.
        if c["scanout"].as_bool().unwrap_or(false) {
            markers.push_str(&format!("{green}★{reset} "));
        }
        // Count/slice by chars, not bytes: a byte slice panics when
        // `max_app-1` lands mid-UTF-8 (app-ids can be non-ASCII), and
        // byte-length over-counts a short multibyte name as too wide.
        let app_disp = if app.chars().count() > max_app {
            let keep = max_app.saturating_sub(1);
            format!("{}…", app.chars().take(keep).collect::<String>())
        } else {
            app.to_string()
        };
        if wide {
            println!(
                "{:<w_tag$}  {:<w_mon$}  {:<w_app$}  {:<22}  {markers}{title}",
                tags,
                mon,
                app_disp,
                geom,
                w_tag = max_tag,
                w_mon = max_mon,
                w_app = max_app,
            );
        } else {
            println!(
                "{:<w_tag$}  {:<w_mon$}  {:<w_app$}  {markers}{title}",
                tags,
                mon,
                app_disp,
                w_tag = max_tag,
                w_mon = max_mon,
                w_app = max_app,
            );
        }
    }

    Ok(())
}

pub(super) fn cmd_outputs(json_out: bool) -> Result<()> {
    use std::io::IsTerminal;
    let state = read_state_file()?;
    let outputs = state["outputs"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("state file missing `outputs` array"))?;

    if json_out {
        println!("{}", serde_json::to_string_pretty(outputs)?);
        return Ok(());
    }

    let tty = std::io::stdout().is_terminal();
    let bold = if tty { "\x1b[1m" } else { "" };
    let dim = if tty { "\x1b[2m" } else { "" };
    let green = if tty { "\x1b[32m" } else { "" };
    let cyan = if tty { "\x1b[36m" } else { "" };
    let reset = if tty { "\x1b[0m" } else { "" };

    println!(
        "{bold}{:<10}  {:<11}  {:<6}  {:<10}  ACTIVE-TAGS{reset}",
        "NAME", "POSITION", "SCALE", "MODE",
    );
    for o in outputs {
        let name = o["name"].as_str().unwrap_or("");
        let x = o["x"].as_i64().unwrap_or(0);
        let y = o["y"].as_i64().unwrap_or(0);
        let w = o["width"].as_i64().unwrap_or(0);
        let h = o["height"].as_i64().unwrap_or(0);
        let scale = o["scale"].as_f64().unwrap_or(1.0);
        let mode = format!(
            "{}×{}",
            o["mode"]["physical_width"].as_i64().unwrap_or(0),
            o["mode"]["physical_height"].as_i64().unwrap_or(0),
        );
        let active = o["active"].as_bool().unwrap_or(false);
        let active_mark = if active {
            format!("{green}●{reset} ")
        } else {
            "  ".to_string()
        };
        let active_tag = o["active_tag_mask"].as_u64().unwrap_or(0) as u32;
        let mut tags = Vec::new();
        for i in 0..9 {
            if active_tag & (1 << i) != 0 {
                tags.push(format!("{}", i + 1));
            }
        }
        let tag_str = if tags.is_empty() {
            "—".to_string()
        } else {
            tags.join(",")
        };
        println!(
            "{active_mark}{bold}{name:<8}{reset}  {dim}{x:>4},{y:<4}{reset}  {dim}{scale:<6.2}{reset}  {cyan}{mode:<10}{reset}  {tag_str} {dim}({w}×{h} logical){reset}",
        );
    }

    Ok(())
}

pub(super) fn cmd_perf(json_out: bool, watch: bool) -> Result<()> {
    if watch && !json_out {
        // Live-refresh mode: clear + redraw once a second. The compositor keeps
        // a rolling 60 s sample window, so the windowed columns are meaningful
        // even on the very first frame. Loop exits (returns Err) if margo goes
        // away — request_once fails and the `?` propagates out.
        loop {
            // Clear screen + home the cursor, then draw one snapshot.
            print!("\x1b[2J\x1b[H");
            perf_print_once(false)?;
            use std::io::Write;
            std::io::stdout().flush().ok();
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
    perf_print_once(json_out)
}

/// Fetch the `perf` topic once and print it (table or `--json`).
fn perf_print_once(json_out: bool) -> Result<()> {
    use std::io::IsTerminal;
    let perf = mctl::ipc_client::request_once("get perf").with_context(|| {
        format!(
            "query margo IPC socket {}: is margo running?",
            mctl::ipc_client::socket_path().display()
        )
    })?;
    let outputs = perf["outputs"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("perf reply missing `outputs` array"))?;

    if json_out {
        println!("{}", serde_json::to_string_pretty(outputs)?);
        return Ok(());
    }

    if outputs.is_empty() {
        // The winit/nested backend keeps no per-output DRM counters.
        println!("no per-output counters (winit/nested backend, or nothing rendered yet)");
        return Ok(());
    }

    let tty = std::io::stdout().is_terminal();
    let bold = if tty { "\x1b[1m" } else { "" };
    let dim = if tty { "\x1b[2m" } else { "" };
    let cyan = if tty { "\x1b[36m" } else { "" };
    let yellow = if tty { "\x1b[33m" } else { "" };
    let reset = if tty { "\x1b[0m" } else { "" };

    // Live health first (windowed FPS + recent empty ratio + render-latency
    // tail + direct-scanout share + composited damage area), then lifetime
    // queue-error total. Cumulative renders/queued/empties stay available
    // via `--json`.
    println!(
        "{bold}{:<10}  {:>7}  {:>7}  {:>7}  {:>22}  {:>6}  {:>17}  {:>6}{reset}",
        "OUTPUT",
        "FPS 1s",
        "FPS 60s",
        "EMPTY%",
        "RENDER p50/p95/p99 ms",
        "SCAN%",
        "DMG p50/p95 Mpx",
        "Q-ERR",
    );
    for o in outputs {
        let name = o["name"].as_str().unwrap_or("");
        let q_err = o["queue_errors"].as_u64().unwrap_or(0);
        // Queue errors are the one counter that's bad when non-zero.
        let err_col = if q_err > 0 { yellow } else { dim };
        let n = o["window_samples"].as_u64().unwrap_or(0);
        if n == 0 {
            // Output present but no frames in the window yet — don't invent 0.0s.
            println!(
                "{bold}{name:<10}{reset}  {dim}{:>7}  {:>7}  {:>7}  {:>22}  {:>6}  {:>17}{reset}  {err_col}{q_err:>6}{reset}",
                "—", "—", "—", "—", "—", "—",
            );
            continue;
        }
        let fps1 = o["fps_1s"].as_f64().unwrap_or(0.0);
        let fps60 = o["fps_60s"].as_f64().unwrap_or(0.0);
        let empty_pct = o["empty_ratio_10s"].as_f64().unwrap_or(0.0) * 100.0;
        let p50 = o["render_us_p50"].as_u64().unwrap_or(0) as f64 / 1000.0;
        let p95 = o["render_us_p95"].as_u64().unwrap_or(0) as f64 / 1000.0;
        let p99 = o["render_us_p99"].as_u64().unwrap_or(0) as f64 / 1000.0;
        let lat = format!("{p50:.2} / {p95:.2} / {p99:.2}");
        let scan_pct = o["scanout_ratio_10s"].as_f64().unwrap_or(0.0) * 100.0;
        let dmg50 = o["damage_px_p50"].as_u64().unwrap_or(0) as f64 / 1_000_000.0;
        let dmg95 = o["damage_px_p95"].as_u64().unwrap_or(0) as f64 / 1_000_000.0;
        let dmg = format!("{dmg50:.2} / {dmg95:.2}");
        println!(
            "{bold}{name:<10}{reset}  {cyan}{fps1:>7.1}{reset}  {fps60:>7.1}  {cyan}{empty_pct:>6.1}%{reset}  {lat:>22}  {cyan}{scan_pct:>5.1}%{reset}  {dmg:>17}  {err_col}{q_err:>6}{reset}",
        );
    }

    Ok(())
}

pub(super) fn cmd_focused(json_out: bool) -> Result<()> {
    let state = read_state_file()?;
    let clients = state["clients"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("state file missing `clients` array"))?;
    let Some(focused) = clients
        .iter()
        .find(|c| c["focused"].as_bool() == Some(true))
    else {
        if json_out {
            println!("null");
        } else {
            println!("(no focused window)");
        }
        return Ok(());
    };
    if json_out {
        println!("{}", serde_json::to_string_pretty(focused)?);
        return Ok(());
    }
    let app = focused["app_id"].as_str().unwrap_or("");
    let title = focused["title"].as_str().unwrap_or("");
    println!("{app} · {title}");
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn mirror_status_reads_the_ipc_object() {
        use super::mirror_status_line;
        use serde_json::json;
        let on = json!({ "active": { "source": "eDP-1", "target": "DP-3" }, "error": null });
        assert_eq!(mirror_status_line(&on), "mirroring eDP-1 → DP-3");
        let refused = json!({ "active": null, "error": "no output named DP-9" });
        assert_eq!(mirror_status_line(&refused), "not mirroring");
        assert!(
            mirror_status_line(&json!({ "error": "unknown topic: mirror" }))
                .contains("older margo")
        );
    }
}
