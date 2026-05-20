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
