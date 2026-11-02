pub mod brightness_osd;
pub mod mic_osd;
pub mod network_osd;
pub mod osd_geometry;
pub mod sound_alerts;
pub mod toast;
pub mod toast_producer;
pub mod volume_osd;

/// The numeric label next to an OSD slider: the percentage rounded to a whole
/// number (`42.5` → `"43%"`). Not clamped — PipeWire allows boosting volume
/// past 100 %, and the label should say so even though the slider saturates.
pub(crate) fn percent_label(percent: f64) -> String {
    format!("{}%", percent.round() as i32)
}

/// Counts `Show` requests and reports whether the OSD may actually appear.
///
/// The value watchers replay the current state when they subscribe, so the
/// first two `Show`s after startup are the initial state being observed, not a
/// user change, and must not pop the OSD. Every later `Show` is let through.
#[derive(Debug, Default)]
pub(crate) struct ShowGate {
    seen: u16,
}

impl ShowGate {
    /// Requests that arrive before the OSD is allowed to show.
    const WARMUP: u16 = 2;

    pub(crate) fn allow(&mut self) -> bool {
        if self.seen >= Self::WARMUP {
            true
        } else {
            self.seen += 1;
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_label_rounds_to_whole_percent() {
        assert_eq!(percent_label(0.0), "0%");
        assert_eq!(percent_label(42.4), "42%");
        assert_eq!(percent_label(42.5), "43%");
        assert_eq!(percent_label(99.6), "100%");
    }

    #[test]
    fn percent_label_keeps_boosted_and_tiny_values() {
        assert_eq!(percent_label(150.0), "150%");
        assert_eq!(percent_label(0.4), "0%");
        // A rounding wobble just below zero must not render as "-0%".
        assert_eq!(percent_label(-0.2), "0%");
    }

    #[test]
    fn show_gate_swallows_the_two_startup_emissions() {
        let mut gate = ShowGate::default();
        assert!(!gate.allow());
        assert!(!gate.allow());
        assert!(gate.allow());
        assert!(gate.allow());
        assert!(gate.allow());
    }
}
