//! Shared client for margo's built-in **twilight** blue-light
//! filter. Both the bar pill and the Twilight menu drive it through
//! `mctl` — margo owns the output gamma ramps (geo / schedule /
//! phases), so going through `mctl` keeps a single source of truth
//! rather than a second gamma writer in the shell. State is read by
//! polling `mctl twilight status --json`.

use tracing::warn;

/// Snapshot of margo's state.json `twilight` object.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TwilightStatus {
    pub enabled: bool,
    /// `geo` | `manual` | `static` | `schedule`.
    pub mode: String,
    /// `day` | `night` | `transition_to_day` | `transition_to_night`
    /// | `idle`.
    pub phase: String,
    pub current_temp_k: Option<u32>,
    pub current_gamma_pct: Option<u32>,
}

impl TwilightStatus {
    /// Symbolic icon name: a warm night glyph while filtering, a
    /// neutral sun otherwise.
    pub fn icon(&self) -> &'static str {
        if self.enabled {
            "weather-clear-night-symbolic"
        } else {
            "weather-clear-symbolic"
        }
    }

    /// Human phase label (`""` when idle/unknown).
    pub fn phase_label(&self) -> &'static str {
        match self.phase.as_str() {
            "day" => "Day",
            "night" => "Night",
            "transition_to_day" => "→ Day",
            "transition_to_night" => "→ Night",
            _ => "",
        }
    }
}

/// Poll `mctl twilight status --json`. `None` when `mctl` is missing,
/// the compositor is unreachable, or the JSON doesn't parse.
pub(crate) async fn probe() -> Option<TwilightStatus> {
    let out = tokio::process::Command::new("mctl")
        .args(["twilight", "status", "--json"])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    Some(TwilightStatus {
        enabled: v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(false),
        mode: v
            .get("mode")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string(),
        phase: v
            .get("phase")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string(),
        current_temp_k: v.get("current_temp_k").and_then(|x| x.as_u64()).map(|n| n as u32),
        current_gamma_pct: v
            .get("current_gamma_pct")
            .and_then(|x| x.as_u64())
            .map(|n| n as u32),
    })
}

/// One schedule preset (name + temperature/brightness, optional
/// time-of-day from `schedule.conf`).
#[derive(Debug, Clone)]
pub(crate) struct Preset {
    pub name: String,
    pub temp_k: u32,
    pub gamma_pct: u32,
    pub time: Option<String>,
}

/// `$XDG_CONFIG_HOME/margo/twilight` (or `~/.config/margo/twilight`)
/// — margo's default `twilight_schedule_dir`. We don't read the
/// compositor config here; the override is rare and the menu only
/// needs the common case.
fn schedule_dir() -> std::path::PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return std::path::PathBuf::from(xdg).join("margo").join("twilight");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return std::path::PathBuf::from(home)
            .join(".config")
            .join("margo")
            .join("twilight");
    }
    std::path::PathBuf::from(".config/margo/twilight")
}

/// Parse one preset TOML's `static_temp` / `static_gamma`.
fn read_preset(path: &std::path::Path) -> Option<(u32, u32)> {
    let text = std::fs::read_to_string(path).ok()?;
    let (mut temp, mut gamma) = (None, None);
    for line in text.lines() {
        let line = line.trim();
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim().trim_matches('"');
        match k.trim() {
            "static_temp" => temp = v.parse().ok().map(|n: u32| n.clamp(1000, 25000)),
            "static_gamma" => gamma = v.parse().ok().map(|n: u32| n.clamp(10, 200)),
            _ => {}
        }
    }
    Some((temp?, gamma?))
}

/// Load the schedule presets in time order. Falls back to listing
/// `presets/*.toml` alphabetically when there's no `schedule.conf`.
pub(crate) fn load_presets() -> Vec<Preset> {
    let dir = schedule_dir();
    let presets_dir = dir.join("presets");
    let mut out: Vec<Preset> = Vec::new();

    if let Ok(schedule) = std::fs::read_to_string(dir.join("schedule.conf")) {
        for line in schedule.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            let mut parts = t.split_whitespace();
            let (Some(time), Some(name)) = (parts.next(), parts.next()) else {
                continue;
            };
            if let Some((temp_k, gamma_pct)) = read_preset(&presets_dir.join(format!("{name}.toml")))
            {
                out.push(Preset {
                    name: name.to_string(),
                    temp_k,
                    gamma_pct,
                    time: Some(time.to_string()),
                });
            }
        }
        out.sort_by(|a, b| a.time.cmp(&b.time));
        if !out.is_empty() {
            return out;
        }
    }

    // No schedule.conf (or it referenced nothing readable) — list files.
    if let Ok(entries) = std::fs::read_dir(&presets_dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) != Some("toml") {
                continue;
            }
            let name = p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
            if let Some((temp_k, gamma_pct)) = read_preset(&p) {
                out.push(Preset {
                    name,
                    temp_k,
                    gamma_pct,
                    time: None,
                });
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
    }
    out
}

/// Fire a `mctl …` invocation, fire-and-forget. Used for the menu's
/// toggle / mode / temperature / preset actions.
pub(crate) fn run(args: Vec<String>) {
    relm4::spawn(async move {
        match tokio::process::Command::new("mctl").args(&args).status().await {
            Ok(s) if s.success() => {}
            Ok(s) => warn!(?s, ?args, "mctl twilight returned non-zero"),
            Err(e) => warn!(error = %e, ?args, "mctl twilight spawn failed"),
        }
    });
}
