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

/// Read margo's twilight state (what `mctl twilight status --json` prints)
/// over the IPC socket, off the async workers. `None` when the compositor
/// is unreachable or doesn't advertise twilight.
pub(crate) async fn probe() -> Option<TwilightStatus> {
    let v = tokio::task::spawn_blocking(mshell_margo_client::ctl::twilight_status)
        .await
        .ok()?
        .ok()
        .filter(|v| !v.is_null())?;
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
        current_temp_k: v
            .get("current_temp_k")
            .and_then(|x| x.as_u64())
            .map(|n| n as u32),
        current_gamma_pct: v
            .get("current_gamma_pct")
            .and_then(|x| x.as_u64())
            .map(|n| n as u32),
    })
}

/// One schedule preset (name + temperature/brightness, optional
/// time-of-day from `schedule.conf`) — margo-config's shared parser.
pub(crate) use margo_config::twilight::Preset;

/// margo's `twilight_schedule_dir` (empty = `<config_home>/margo/twilight`),
/// read from the compositor config so a moved schedule shows up here too.
fn schedule_dir() -> std::path::PathBuf {
    let configured = margo_config::parse_config(None)
        .map(|c| c.twilight_schedule_dir)
        .unwrap_or_default();
    margo_config::twilight::resolve_schedule_dir(&configured)
}

/// Load the schedule presets in time order. Falls back to listing
/// `presets/*.toml` alphabetically when there's no `schedule.conf` (or it
/// references nothing readable).
pub(crate) fn load_presets() -> Vec<Preset> {
    let dir = schedule_dir();
    let scheduled = margo_config::twilight::load_schedule(&dir);
    if !scheduled.is_empty() {
        return scheduled;
    }
    margo_config::twilight::list_presets(&dir)
}

/// Run an `mctl twilight …` command, fire-and-forget. Used for the menu's
/// toggle / mode / temperature / preset actions; these are all plain
/// dispatches, so they go over margo's socket
/// ([`mshell_margo_client::ctl::run_mctl`]).
pub(crate) fn run(args: Vec<String>) {
    relm4::spawn(async move {
        if let Err(e) = mshell_margo_client::ctl::run_mctl(args).await {
            warn!(error = %e, "twilight request failed");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::TwilightStatus;

    #[test]
    fn icon_reflects_enabled() {
        let on = TwilightStatus {
            enabled: true,
            ..Default::default()
        };
        let off = TwilightStatus {
            enabled: false,
            ..Default::default()
        };
        assert_eq!(on.icon(), "weather-clear-night-symbolic");
        assert_eq!(off.icon(), "weather-clear-symbolic");
    }

    #[test]
    fn phase_label_maps_known_phases() {
        let label = |p: &str| {
            TwilightStatus {
                phase: p.to_string(),
                ..Default::default()
            }
            .phase_label()
        };
        assert_eq!(label("day"), "Day");
        assert_eq!(label("night"), "Night");
        assert_eq!(label("transition_to_day"), "\u{2192} Day");
        assert_eq!(label("transition_to_night"), "\u{2192} Night");
        assert_eq!(label("idle"), "");
        assert_eq!(label(""), "");
    }
}
