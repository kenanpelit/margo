//! Output mirroring ("presentation mode"): one output's picture shown on
//! another, scaled to fit and letterboxed.
//!
//! The *source* keeps working as a normal desktop. The *target* leaves the
//! layout the way a soft-disabled output does — its windows move to an
//! enabled output, the pointer can't enter it, nothing new opens there —
//! but its panel stays lit and the renderer draws the source's scene onto
//! it (see `backend::udev::frame`). Stopping re-enables the target.
//!
//! This module is the pure part: the session state machine and the
//! geometry. Everything that touches compositor state lives in
//! `state::mirror`.

/// One active mirror: `source`'s picture shown on `target` (connector
/// names, e.g. `eDP-1` → `DP-3`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorSession {
    pub source: String,
    pub target: String,
}

/// Why a mirror request was refused. The `Display` text is what the IPC
/// `dispatch` error and the log carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirrorError {
    /// Source and target are the same output.
    SameOutput,
    /// No connected output has this name.
    UnknownOutput(String),
    /// The source is switched off: there is no picture to show.
    SourceDisabled(String),
    /// The target is switched off by the user; turn it on first.
    TargetDisabled(String),
    /// No arguments and no second enabled output to mirror onto.
    NoPair,
}

impl std::fmt::Display for MirrorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MirrorError::SameOutput => write!(f, "source and target are the same output"),
            MirrorError::UnknownOutput(n) => write!(f, "no output named {n}"),
            MirrorError::SourceDisabled(n) => write!(f, "source output {n} is disabled"),
            MirrorError::TargetDisabled(n) => write!(f, "target output {n} is disabled"),
            MirrorError::NoPair => write!(f, "mirroring needs two enabled outputs"),
        }
    }
}

/// What a request changed, for the caller to carry out on the
/// compositor (take the new target out of the layout, put an old one
/// back).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirrorChange {
    /// A session started. `replaced` is the session it took over from,
    /// whose target must be restored unless it is part of the new one.
    Started {
        session: MirrorSession,
        replaced: Option<MirrorSession>,
    },
    /// The session ended; its target must be restored.
    Stopped(MirrorSession),
    /// Nothing to do (already mirroring exactly this, or nothing to stop).
    Unchanged,
}

/// A connected output as the state machine sees it.
#[derive(Debug, Clone, Copy)]
pub struct OutputView<'a> {
    pub name: &'a str,
    /// Enabled in the layout. A current mirror target reads as disabled
    /// here (it left the layout); the state machine accounts for that.
    pub enabled: bool,
}

/// The mirror state machine: idle, or one session.
#[derive(Debug, Default)]
pub struct MirrorState {
    session: Option<MirrorSession>,
    /// Why the last request was refused, cleared by the next accepted
    /// one. Published as `mirror.error` so `mctl` / the shell can say why.
    pub last_error: Option<String>,
}

impl MirrorState {
    pub fn active(&self) -> Option<&MirrorSession> {
        self.session.as_ref()
    }

    /// `name` is the output currently showing the mirror.
    pub fn is_target(&self, name: &str) -> bool {
        self.session.as_ref().is_some_and(|s| s.target == name)
    }

    /// `name` is the output currently being mirrored.
    pub fn is_source(&self, name: &str) -> bool {
        self.session.as_ref().is_some_and(|s| s.source == name)
    }

    /// Start mirroring `source` onto `target`. A running session is
    /// replaced (reported in `MirrorChange::Started::replaced`).
    pub fn start(
        &mut self,
        source: &str,
        target: &str,
        outputs: &[OutputView<'_>],
    ) -> Result<MirrorChange, MirrorError> {
        if source == target {
            return Err(MirrorError::SameOutput);
        }
        // Our own target is only "disabled" because it is mirroring.
        let usable = |name: &str| -> Result<bool, MirrorError> {
            outputs
                .iter()
                .find(|o| o.name == name)
                .map(|o| o.enabled || self.is_target(name))
                .ok_or_else(|| MirrorError::UnknownOutput(name.to_string()))
        };
        if !usable(source)? {
            return Err(MirrorError::SourceDisabled(source.to_string()));
        }
        if !usable(target)? {
            return Err(MirrorError::TargetDisabled(target.to_string()));
        }
        let session = MirrorSession {
            source: source.to_string(),
            target: target.to_string(),
        };
        if self.session.as_ref() == Some(&session) {
            return Ok(MirrorChange::Unchanged);
        }
        let replaced = self.session.replace(session.clone());
        Ok(MirrorChange::Started { session, replaced })
    }

    /// Stop the running session, if any.
    pub fn stop(&mut self) -> MirrorChange {
        match self.session.take() {
            Some(s) => MirrorChange::Stopped(s),
            None => MirrorChange::Unchanged,
        }
    }

    /// An output was unplugged, or the user switched it on/off by hand:
    /// a session that uses it ends.
    pub fn output_gone(&mut self, name: &str) -> MirrorChange {
        if self.is_source(name) || self.is_target(name) {
            self.stop()
        } else {
            MirrorChange::Unchanged
        }
    }
}

/// The laptop panel is the eDP / LVDS / DSI connector.
pub fn is_internal_connector(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l.starts_with("edp") || l.starts_with("lvds") || l.starts_with("dsi")
}

/// The `(source, target)` pair for a bare `mirror_output` /
/// `toggle_mirror`: the laptop panel (else the focused output, else the
/// first enabled one) onto the first other enabled output, an external
/// one by preference. `None` with fewer than two enabled outputs.
pub fn default_pair(outputs: &[OutputView<'_>], focused: Option<&str>) -> Option<(String, String)> {
    let enabled: Vec<&str> = outputs
        .iter()
        .filter(|o| o.enabled)
        .map(|o| o.name)
        .collect();
    let source = enabled
        .iter()
        .copied()
        .find(|n| is_internal_connector(n))
        .or_else(|| focused.filter(|f| enabled.contains(f)))
        .or_else(|| enabled.first().copied())?;
    let target = enabled
        .iter()
        .copied()
        .find(|n| *n != source && !is_internal_connector(n))
        .or_else(|| enabled.iter().copied().find(|n| *n != source))?;
    Some((source.to_string(), target.to_string()))
}

/// Where the source picture lands on the target, in target framebuffer
/// pixels: scaled uniformly by `scale`, `size` big, placed at `offset`,
/// with the rest (the letterbox / pillarbox bars) cleared to black.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Letterbox {
    pub scale: f64,
    pub offset: (i32, i32),
    pub size: (i32, i32),
}

/// Fit a `src`-sized picture into `dst`, keeping its aspect ratio and
/// centring it. Sizes are the outputs' transformed physical extents (the
/// space render elements live in). `None` for an empty size.
pub fn letterbox(src: (i32, i32), dst: (i32, i32)) -> Option<Letterbox> {
    if src.0 <= 0 || src.1 <= 0 || dst.0 <= 0 || dst.1 <= 0 {
        return None;
    }
    let sx = f64::from(dst.0) / f64::from(src.0);
    let sy = f64::from(dst.1) / f64::from(src.1);
    let scale = sx.min(sy);
    let w = ((f64::from(src.0) * scale).round() as i32).clamp(1, dst.0);
    let h = ((f64::from(src.1) * scale).round() as i32).clamp(1, dst.1);
    Some(Letterbox {
        scale,
        offset: ((dst.0 - w) / 2, (dst.1 - h) / 2),
        size: (w, h),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outs<'a>(list: &[(&'a str, bool)]) -> Vec<OutputView<'a>> {
        list.iter()
            .map(|&(name, enabled)| OutputView { name, enabled })
            .collect()
    }

    // ── geometry ─────────────────────────────────────────────────────

    #[test]
    fn a_16_10_laptop_on_a_16_9_monitor_is_pillarboxed() {
        // Owner's ThinkPad eDP-1 1920×1200 onto DP-3 2560×1440.
        let lb = letterbox((1920, 1200), (2560, 1440)).unwrap();
        assert!((lb.scale - 1.2).abs() < 1e-9);
        assert_eq!(lb.size, (2304, 1440));
        assert_eq!(lb.offset, (128, 0));
    }

    #[test]
    fn a_16_9_monitor_on_a_16_10_laptop_is_letterboxed() {
        let lb = letterbox((2560, 1440), (1920, 1200)).unwrap();
        assert!((lb.scale - 0.75).abs() < 1e-9);
        assert_eq!(lb.size, (1920, 1080));
        assert_eq!(lb.offset, (0, 60));
    }

    #[test]
    fn same_size_is_an_identity_fit() {
        let lb = letterbox((1920, 1080), (1920, 1080)).unwrap();
        assert_eq!(lb.scale, 1.0);
        assert_eq!((lb.offset, lb.size), ((0, 0), (1920, 1080)));
    }

    #[test]
    fn a_rotated_source_is_fitted_by_its_transformed_size() {
        // A portrait (90°) 1200×1920 panel onto a 1920×1080 projector.
        let lb = letterbox((1200, 1920), (1920, 1080)).unwrap();
        assert_eq!(lb.size.1, 1080);
        assert_eq!(lb.size.0, 675);
        assert_eq!(lb.offset, ((1920 - 675) / 2, 0));
    }

    #[test]
    fn the_picture_never_spills_past_the_target() {
        for (src, dst) in [
            ((1366, 768), (1024, 768)),
            ((3, 7), (1920, 1080)),
            ((7680, 4320), (800, 600)),
        ] {
            let lb = letterbox(src, dst).unwrap();
            assert!(lb.offset.0 >= 0 && lb.offset.1 >= 0, "{src:?}->{dst:?}");
            assert!(lb.offset.0 + lb.size.0 <= dst.0, "{src:?}->{dst:?}");
            assert!(lb.offset.1 + lb.size.1 <= dst.1, "{src:?}->{dst:?}");
        }
    }

    #[test]
    fn empty_sizes_have_no_fit() {
        assert!(letterbox((0, 1080), (1920, 1080)).is_none());
        assert!(letterbox((1920, 1080), (1920, 0)).is_none());
    }

    // ── state machine ────────────────────────────────────────────────

    #[test]
    fn start_then_stop_reports_the_target_to_restore() {
        let o = outs(&[("eDP-1", true), ("DP-3", true)]);
        let mut m = MirrorState::default();
        let s = MirrorSession {
            source: "eDP-1".into(),
            target: "DP-3".into(),
        };
        assert_eq!(
            m.start("eDP-1", "DP-3", &o),
            Ok(MirrorChange::Started {
                session: s.clone(),
                replaced: None
            })
        );
        assert!(m.is_target("DP-3") && m.is_source("eDP-1"));
        assert_eq!(m.stop(), MirrorChange::Stopped(s));
        assert_eq!(m.stop(), MirrorChange::Unchanged);
        assert!(m.active().is_none());
    }

    #[test]
    fn bad_requests_are_refused_and_leave_the_state_alone() {
        let o = outs(&[("eDP-1", true), ("DP-3", false), ("HDMI-A-1", true)]);
        let mut m = MirrorState::default();
        assert_eq!(m.start("eDP-1", "eDP-1", &o), Err(MirrorError::SameOutput));
        assert_eq!(
            m.start("eDP-1", "DP-9", &o),
            Err(MirrorError::UnknownOutput("DP-9".into()))
        );
        assert_eq!(
            m.start("DP-3", "eDP-1", &o),
            Err(MirrorError::SourceDisabled("DP-3".into()))
        );
        assert_eq!(
            m.start("eDP-1", "DP-3", &o),
            Err(MirrorError::TargetDisabled("DP-3".into()))
        );
        assert!(m.active().is_none());
    }

    #[test]
    fn repeating_the_running_request_changes_nothing() {
        let mut m = MirrorState::default();
        let o = outs(&[("eDP-1", true), ("DP-3", true)]);
        m.start("eDP-1", "DP-3", &o).unwrap();
        // The target now reads as disabled (it left the layout).
        let o = outs(&[("eDP-1", true), ("DP-3", false)]);
        assert_eq!(m.start("eDP-1", "DP-3", &o), Ok(MirrorChange::Unchanged));
    }

    #[test]
    fn swapping_direction_replaces_the_session() {
        let mut m = MirrorState::default();
        m.start("eDP-1", "DP-3", &outs(&[("eDP-1", true), ("DP-3", true)]))
            .unwrap();
        // DP-3 is out of the layout as the current target, yet may become
        // the new source.
        let o = outs(&[("eDP-1", true), ("DP-3", false)]);
        match m.start("DP-3", "eDP-1", &o) {
            Ok(MirrorChange::Started { session, replaced }) => {
                assert_eq!(session.source, "DP-3");
                assert_eq!(replaced.unwrap().target, "DP-3");
            }
            other => panic!("{other:?}"),
        }
        assert!(m.is_target("eDP-1"));
    }

    #[test]
    fn losing_either_output_stops_the_session_and_others_do_not() {
        let o = outs(&[("eDP-1", true), ("DP-3", true), ("HDMI-A-1", true)]);
        for gone in ["eDP-1", "DP-3"] {
            let mut m = MirrorState::default();
            m.start("eDP-1", "DP-3", &o).unwrap();
            assert_eq!(m.output_gone("HDMI-A-1"), MirrorChange::Unchanged);
            assert!(matches!(m.output_gone(gone), MirrorChange::Stopped(_)));
            assert!(m.active().is_none());
        }
    }

    #[test]
    fn default_pair_mirrors_the_laptop_onto_an_external_output() {
        let o = outs(&[("DP-3", true), ("eDP-1", true)]);
        assert_eq!(
            default_pair(&o, Some("DP-3")),
            Some(("eDP-1".into(), "DP-3".into()))
        );
    }

    #[test]
    fn default_pair_without_a_laptop_uses_the_focused_output() {
        let o = outs(&[("DP-1", true), ("DP-2", true), ("HDMI-A-1", false)]);
        assert_eq!(
            default_pair(&o, Some("DP-2")),
            Some(("DP-2".into(), "DP-1".into()))
        );
    }

    #[test]
    fn default_pair_needs_two_enabled_outputs() {
        assert_eq!(default_pair(&outs(&[("eDP-1", true)]), None), None);
        assert_eq!(
            default_pair(&outs(&[("eDP-1", true), ("DP-3", false)]), None),
            None
        );
    }
}
