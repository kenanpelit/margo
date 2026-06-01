//! Player identity + candidate scoring (pure). Faithful port of
//! osc-media's `candidate_score`/auto-detect ranking.

use super::status::{Command, Status};

/// A controllable player and which backend drives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Mpris(String),
    Mpd,
    Mpv,
}

/// Browser-hosted MPRIS players (web media) score lower than real apps.
pub fn is_browser(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [
        "firefox", "chromium", "chrome", "brave", "zen", "vivaldi", "edge",
    ]
    .iter()
    .any(|b| n.contains(b))
}

impl Kind {
    /// The player's display/identity name.
    pub fn name(&self) -> &str {
        match self {
            Kind::Mpris(n) => n,
            Kind::Mpd => "mpd",
            Kind::Mpv => "mpv",
        }
    }

    /// Stable `<kind>:<name>` id for last-player memory + tie-breaking.
    pub fn id(&self) -> String {
        match self {
            Kind::Mpris(n) => format!("mpris:{n}"),
            Kind::Mpd => "mpd:mpd".to_string(),
            Kind::Mpv => "mpv:mpv".to_string(),
        }
    }
}

/// Rank a candidate player for auto-detect. Higher wins. Mirrors
/// osc-media: status base, kind bonus, name bonus, last-player bonus, and
/// command-context nudges.
pub fn candidate_score(kind: &Kind, status: Status, cmd: Command, last_id: &str) -> i32 {
    let name = kind.name();
    let lower = name.to_ascii_lowercase();

    let mut score = match status {
        Status::Playing => 300,
        Status::Paused => 180,
        Status::Stopped => 40,
        Status::Unknown => 20,
    };

    match kind {
        Kind::Mpv | Kind::Mpd => score += 40,
        Kind::Mpris(_) => score += if is_browser(name) { 8 } else { 35 },
    }

    if lower.starts_with("spotify") {
        score += 35;
    } else if lower.starts_with("vlc") {
        score += 28;
    } else if lower.starts_with("mpv") {
        score += 24;
    } else if is_browser(name) {
        score += 10;
    }

    if kind.id() == last_id {
        score += 90;
    }

    if matches!(
        cmd,
        Command::Toggle
            | Command::Play
            | Command::Pause
            | Command::Next
            | Command::Prev
            | Command::Status
    ) && status == Status::Playing
    {
        score += 18;
    }
    if cmd == Command::Play && status == Status::Paused {
        score += 15;
    }

    score
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playing_beats_paused_same_player() {
        let k = Kind::Mpv;
        assert!(
            candidate_score(&k, Status::Playing, Command::Toggle, "")
                > candidate_score(&k, Status::Paused, Command::Toggle, "")
        );
    }

    #[test]
    fn mpv_beats_browser_mpris_when_equal_status() {
        let mpv = candidate_score(&Kind::Mpv, Status::Paused, Command::Toggle, "");
        let fox = candidate_score(
            &Kind::Mpris("firefox".into()),
            Status::Paused,
            Command::Toggle,
            "",
        );
        assert!(mpv > fox);
    }

    #[test]
    fn spotify_outscores_a_browser() {
        let spot = candidate_score(
            &Kind::Mpris("spotify".into()),
            Status::Paused,
            Command::Toggle,
            "",
        );
        let brave = candidate_score(
            &Kind::Mpris("brave".into()),
            Status::Paused,
            Command::Toggle,
            "",
        );
        assert!(spot > brave);
    }

    #[test]
    fn last_player_bonus_breaks_a_tie() {
        let a = Kind::Mpris("firefox".into());
        let b = Kind::Mpris("chromium".into());
        // Equal otherwise (both browsers, paused); b is the last player.
        let sa = candidate_score(&a, Status::Paused, Command::Toggle, &b.id());
        let sb = candidate_score(&b, Status::Paused, Command::Toggle, &b.id());
        assert!(sb > sa);
    }

    #[test]
    fn is_browser_detects_known_engines() {
        assert!(is_browser("firefox"));
        assert!(is_browser("Brave"));
        assert!(!is_browser("spotify"));
        assert!(!is_browser("mpv"));
    }
}
