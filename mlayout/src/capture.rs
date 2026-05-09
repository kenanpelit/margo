//! Capture the current monitor configuration into a layout file.
//!
//! Strategy: run `wlr-randr` (already a recommended optional
//! dependency of margo) and parse its plain-text output. We don't
//! talk Wayland directly — that would require pulling in a full
//! `wayland-client` stack to read `wl_output` / `zxdg_output_v1`
//! listeners, just to get geometry that wlr-randr already prints.
//!
//! Format reference (wlr-randr 0.5+):
//!
//!     DP-3 "Make - Model - Serial"
//!       Make: ...
//!       Model: ...
//!       Modes:
//!         2560x1440 px, 59.951000 Hz (preferred, current)
//!       Position: 0,0
//!       Transform: normal
//!       Scale: 1.000000
//!       Adaptive Sync: disabled
//!
//! We pick the `current` mode (or `preferred` as fallback) for
//! each connector, plus the position + scale + transform. That's
//! everything `monitorrule` needs to round-trip the live state
//! into a static layout file.

use anyhow::{anyhow, bail, Context, Result};
use std::process::Command;

/// One captured output as it currently looks on the running
/// session — what `mlayout new` writes into the layout file.
#[derive(Debug, Clone)]
pub struct CapturedOutput {
    pub connector: String,
    pub width: i32,
    pub height: i32,
    pub refresh: f32,
    pub x: i32,
    pub y: i32,
    pub scale: f32,
    pub transform: i32,
    pub vrr: bool,
    /// True when the connector is enabled. Disabled outputs are
    /// dropped on capture — there's no `monitorrule` field for
    /// "off" today, and capturing one would write a useless rule.
    pub enabled: bool,
}

/// Run `wlr-randr` and parse out one `CapturedOutput` per
/// connected output. Empty result means no outputs were active —
/// usually because `wlr-randr` couldn't connect to a Wayland
/// session (we then bail with a useful error).
pub fn capture_via_wlr_randr() -> Result<Vec<CapturedOutput>> {
    let out = Command::new("wlr-randr")
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow!(
                    "`wlr-randr` not found on PATH. Install it (Arch: \
                     `pacman -S wlr-randr`) — mlayout needs it to read \
                     the live monitor configuration."
                )
            } else {
                anyhow!("running wlr-randr: {e}")
            }
        })?;
    if !out.status.success() {
        bail!(
            "wlr-randr exited non-zero — are you running this from inside \
             a margo session? (Wayland socket required.)\n\nstderr:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let text = String::from_utf8(out.stdout)
        .context("wlr-randr produced non-UTF8 output")?;
    parse_wlr_randr(&text)
}

fn parse_wlr_randr(text: &str) -> Result<Vec<CapturedOutput>> {
    let mut outputs = Vec::new();
    let mut current: Option<CapturedOutput> = None;
    let mut in_modes = false;

    for raw in text.lines() {
        // Connector header: starts in column 0, has no leading
        // whitespace, and contains a connector-name token.
        if !raw.starts_with(' ') && !raw.starts_with('\t') && !raw.is_empty() {
            // Flush previous.
            if let Some(prev) = current.take() {
                outputs.push(prev);
            }
            let connector = raw
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_string();
            if connector.is_empty() {
                continue;
            }
            current = Some(CapturedOutput {
                connector,
                width: 0,
                height: 0,
                refresh: 0.0,
                x: 0,
                y: 0,
                scale: 1.0,
                transform: 0,
                vrr: false,
                enabled: true,
            });
            in_modes = false;
            continue;
        }

        let trimmed = raw.trim();
        let Some(c) = current.as_mut() else { continue };

        // Mode list section markers ------------------------------
        if trimmed == "Modes:" {
            in_modes = true;
            continue;
        }
        if trimmed.ends_with(':') && !trimmed.contains(' ') {
            // Some other section header (e.g. `Properties:`).
            in_modes = false;
            continue;
        }

        // Inside the Modes: section ------------------------------
        // wlr-randr's mode lines start with `WxH ` — once we see
        // anything else, we're back in key:value territory and
        // should drop out of modes mode so the Position/Scale/etc
        // handlers below fire.
        if in_modes && looks_like_mode_line(trimmed) {
            // Pick the mode marked `current` (live), else fall
            // back to `preferred` (best the EDID claims) — but
            // only if we haven't already captured a `current` for
            // this output.
            let is_current = trimmed.contains("current");
            let is_preferred = trimmed.contains("preferred");
            if !is_current && !is_preferred {
                continue;
            }
            if c.width != 0 && !is_current {
                continue;
            }
            if let Some((w, h, r)) = parse_mode_line(trimmed) {
                c.width = w;
                c.height = h;
                c.refresh = r;
            }
            continue;
        }
        // Non-mode line inside the Modes section → we've fallen
        // through to the next sibling field. Reset the flag and
        // fall through to the key:value matchers below.
        in_modes = false;

        // Single-line key:value fields ---------------------------
        if let Some(rest) = trimmed.strip_prefix("Position:") {
            // `Position: 0,0`
            let coord = rest.trim();
            if let Some((x, y)) = coord.split_once(',') {
                c.x = x.trim().parse().unwrap_or(0);
                c.y = y.trim().parse().unwrap_or(0);
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("Scale:") {
            c.scale = rest.trim().parse().unwrap_or(1.0);
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("Transform:") {
            c.transform = parse_transform(rest.trim());
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("Enabled:") {
            c.enabled = matches!(rest.trim(), "yes" | "true");
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("Adaptive Sync:") {
            c.vrr = matches!(rest.trim(), "enabled" | "on" | "true");
            continue;
        }
    }

    if let Some(last) = current {
        outputs.push(last);
    }

    // Drop disabled or geometry-less outputs — they wouldn't
    // produce a useful `monitorrule` line anyway.
    outputs.retain(|o| o.enabled && o.width > 0 && o.height > 0);

    if outputs.is_empty() {
        bail!(
            "wlr-randr returned no enabled outputs. Are you running this \
             outside a margo session?"
        );
    }

    Ok(outputs)
}

/// True if `line` has the wlr-randr mode-row shape:
/// `<W>x<H> px, ...`. Used to recognise where the `Modes:`
/// section ends.
fn looks_like_mode_line(line: &str) -> bool {
    let mut tokens = line.split_whitespace();
    let Some(geom) = tokens.next() else {
        return false;
    };
    let Some((w, h)) = geom.split_once('x') else {
        return false;
    };
    if w.parse::<i32>().is_err() || h.parse::<i32>().is_err() {
        return false;
    }
    matches!(tokens.next(), Some("px") | Some("px,"))
}

fn parse_mode_line(line: &str) -> Option<(i32, i32, f32)> {
    // Find `WxH` and the Hz field.
    let mut tokens = line.split_whitespace();
    let geom = tokens.next()?;
    let (w_str, h_str) = geom.split_once('x')?;
    let w: i32 = w_str.parse().ok()?;
    let h: i32 = h_str.parse().ok()?;
    // Walk the remaining tokens looking for the refresh value
    // (the token immediately before "Hz").
    let mut prev: Option<&str> = None;
    let mut refresh: f32 = 0.0;
    for tok in tokens {
        if tok == "Hz" {
            if let Some(p) = prev {
                refresh = p.parse().unwrap_or(0.0);
            }
            break;
        }
        prev = Some(tok);
    }
    Some((w, h, refresh))
}

fn parse_transform(s: &str) -> i32 {
    match s {
        "normal" => 0,
        "90" => 1,
        "180" => 2,
        "270" => 3,
        "flipped" => 4,
        "flipped-90" => 5,
        "flipped-180" => 6,
        "flipped-270" => 7,
        _ => 0,
    }
}

/// Convert one `CapturedOutput` to the corresponding margo
/// `monitorrule` value (everything after the `=` sign). Mirrors
/// the format users hand-write in `config.conf` so the generated
/// file is something they'd recognise as their own.
pub fn to_monitorrule_value(o: &CapturedOutput) -> String {
    let mut parts = vec![format!("name:{}", o.connector)];
    if o.width > 0 {
        parts.push(format!("width:{}", o.width));
    }
    if o.height > 0 {
        parts.push(format!("height:{}", o.height));
    }
    if o.refresh > 0.0 {
        parts.push(format!("refresh:{}", o.refresh.round() as i32));
    }
    parts.push(format!("x:{}", o.x));
    parts.push(format!("y:{}", o.y));
    if (o.scale - 1.0).abs() > 0.001 {
        // Match the trailing-zero style margo's own example file
        // uses (`scale:1.5` not `scale:1.500000`).
        parts.push(format!("scale:{}", trim_trailing_zeros(o.scale)));
    } else {
        parts.push("scale:1".to_string());
    }
    if o.transform != 0 {
        parts.push(format!("transform:{}", o.transform));
    }
    if o.vrr {
        parts.push("vrr:1".to_string());
    }
    parts.join(",")
}

fn trim_trailing_zeros(v: f32) -> String {
    let s = format!("{:.3}", v);
    let s = s.trim_end_matches('0');
    let s = s.trim_end_matches('.');
    s.to_string()
}

/// Auto-pick a colour-palette index for a connector based on its
/// name prefix. eDP/LVDS = laptop panel, DP/HDMI = external. The
/// goal is the freshly-generated layout file looks reasonable
/// without the user re-tuning every `#@ color` line.
pub fn auto_color_for_connector(name: &str) -> Option<u8> {
    let upper = name.to_uppercase();
    if upper.starts_with("EDP") || upper.starts_with("LVDS") {
        Some(11) // blue — laptop
    } else if upper.starts_with("DP") {
        Some(9) // cyan — DisplayPort
    } else if upper.starts_with("HDMI") {
        Some(7) // emerald — HDMI
    } else if upper.starts_with("DVI") || upper.starts_with("VGA") {
        Some(2) // orange — legacy
    } else {
        None
    }
}

/// Default short label for a connector — `eDP-1` → `laptop`,
/// `DP-3` → `external`, etc. Cosmetic only; the user can override
/// via `#@ output_name` after the file is generated.
pub fn auto_label_for_connector(name: &str) -> Option<&'static str> {
    let upper = name.to_uppercase();
    if upper.starts_with("EDP") || upper.starts_with("LVDS") {
        Some("laptop")
    } else if upper.starts_with("DP") {
        Some("external")
    } else if upper.starts_with("HDMI") {
        Some("hdmi")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "DP-3 \"Unknown - Unknown - DP-3\"
  Make: Unknown
  Model: Unknown
  Serial: Unknown
  Physical size: 600x340 mm
  Enabled: yes
  Modes:
    2560x1440 px, 59.951000 Hz (preferred, current)
    1920x1080 px, 60.000000 Hz
  Position: 0,0
  Transform: normal
  Scale: 1.000000
  Adaptive Sync: disabled
eDP-1 \"Unknown - Unknown - eDP-1\"
  Make: Unknown
  Enabled: yes
  Modes:
    1920x1200 px, 60.002998 Hz (preferred, current)
  Position: 320,1440
  Transform: normal
  Scale: 1.500000
  Adaptive Sync: disabled
";

    #[test]
    fn parses_two_outputs() {
        let outs = parse_wlr_randr(SAMPLE).unwrap();
        assert_eq!(outs.len(), 2);
        assert_eq!(outs[0].connector, "DP-3");
        assert_eq!(outs[0].width, 2560);
        assert_eq!(outs[0].height, 1440);
        assert!((outs[0].refresh - 59.951).abs() < 0.01);
        assert_eq!(outs[0].x, 0);
        assert_eq!(outs[0].y, 0);
        assert!((outs[0].scale - 1.0).abs() < 0.001);
        assert_eq!(outs[1].connector, "eDP-1");
        assert_eq!(outs[1].x, 320);
        assert_eq!(outs[1].y, 1440);
        assert!((outs[1].scale - 1.5).abs() < 0.001);
    }

    #[test]
    fn monitorrule_round_trip() {
        let outs = parse_wlr_randr(SAMPLE).unwrap();
        let dp3 = to_monitorrule_value(&outs[0]);
        assert!(dp3.starts_with("name:DP-3,"));
        assert!(dp3.contains("width:2560"));
        assert!(dp3.contains("height:1440"));
        assert!(dp3.contains("refresh:60"));
        assert!(dp3.contains("scale:1"));

        let edp = to_monitorrule_value(&outs[1]);
        assert!(edp.contains("scale:1.5"));
    }

    #[test]
    fn drops_disabled_outputs() {
        let txt = "DP-3 \"...\"
  Enabled: no
  Modes:
    2560x1440 px, 60.0 Hz (preferred, current)
  Position: 0,0
  Transform: normal
  Scale: 1.000000
";
        let outs = parse_wlr_randr(txt);
        assert!(outs.is_err()); // all-disabled → bail
    }

    #[test]
    fn auto_label_recognises_common_connectors() {
        assert_eq!(auto_label_for_connector("eDP-1"), Some("laptop"));
        assert_eq!(auto_label_for_connector("DP-3"), Some("external"));
        assert_eq!(auto_label_for_connector("HDMI-A-1"), Some("hdmi"));
        assert_eq!(auto_label_for_connector("Unknown-99"), None);
    }
}
