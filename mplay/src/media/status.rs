//! Player status + transport command enums (pure).

/// Normalized playback status across all backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Playing,
    Paused,
    Stopped,
    Unknown,
}

impl Status {
    /// Map a backend's raw status word onto a `Status`.
    pub fn normalize(raw: &str) -> Status {
        match raw.trim().to_ascii_lowercase().as_str() {
            "playing" => Status::Playing,
            "paused" => Status::Paused,
            "stopped" => Status::Stopped,
            _ => Status::Unknown,
        }
    }

    /// Turkish label for notifications.
    pub fn label(self) -> &'static str {
        match self {
            Status::Playing => "Oynatılıyor",
            Status::Paused => "Duraklatıldı",
            Status::Stopped => "Durduruldu",
            Status::Unknown => "Hazır",
        }
    }
}

/// A transport command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Toggle,
    Play,
    Pause,
    Stop,
    Next,
    Prev,
    Status,
}

impl Command {
    pub fn parse(s: &str) -> Option<Command> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "toggle" => Command::Toggle,
            "play" => Command::Play,
            "pause" => Command::Pause,
            "stop" => Command::Stop,
            "next" => Command::Next,
            "prev" | "previous" => Command::Prev,
            "status" => Command::Status,
            _ => return None,
        })
    }

    /// The `playerctl` action name for this command.
    pub fn mpris_action(self) -> &'static str {
        match self {
            Command::Toggle => "play-pause",
            Command::Play => "play",
            Command::Pause => "pause",
            Command::Stop => "stop",
            Command::Next => "next",
            Command::Prev => "previous",
            Command::Status => "status",
        }
    }

    /// A query-only command (don't mutate the player).
    pub fn is_status(self) -> bool {
        matches!(self, Command::Status)
    }

    /// Turkish action label for notifications.
    pub fn label(self) -> &'static str {
        match self {
            Command::Toggle => "Play/Pause",
            Command::Play => "Oynat",
            Command::Pause => "Duraklat",
            Command::Stop => "Durdur",
            Command::Next => "Sonraki parça",
            Command::Prev => "Önceki parça",
            Command::Status => "Durum",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_normalize() {
        assert_eq!(Status::normalize("Playing"), Status::Playing);
        assert_eq!(Status::normalize("paused"), Status::Paused);
        assert_eq!(Status::normalize("  Stopped "), Status::Stopped);
        assert_eq!(Status::normalize("garbage"), Status::Unknown);
    }

    #[test]
    fn command_parse_and_actions() {
        assert_eq!(Command::parse("toggle"), Some(Command::Toggle));
        assert_eq!(Command::parse("PREV"), Some(Command::Prev));
        assert_eq!(Command::parse("previous"), Some(Command::Prev));
        assert_eq!(Command::parse("nope"), None);
        assert_eq!(Command::Toggle.mpris_action(), "play-pause");
        assert_eq!(Command::Prev.mpris_action(), "previous");
        assert!(Command::Status.is_status());
        assert!(!Command::Toggle.is_status());
    }
}
