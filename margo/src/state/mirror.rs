//! Output mirroring on the compositor: carries out the
//! [`crate::mirror::MirrorState`] transitions (take the target out of the
//! layout, put it back) and feeds them from dispatch, hotplug and the
//! user's own enable/disable. Rendering lives in `backend::udev`.

use super::MargoState;
use crate::mirror::{MirrorChange, MirrorError, OutputView, default_pair};

impl MargoState {
    /// Start mirroring `source` onto `target`. Either may be `None`: a
    /// bare request mirrors the laptop panel (else the focused output)
    /// onto the first other enabled output; only a target given picks the
    /// laptop/focused output as source.
    pub fn start_mirror(
        &mut self,
        source: Option<&str>,
        target: Option<&str>,
    ) -> Result<(), MirrorError> {
        // The current target is out of the layout only because it shows
        // the mirror: it is a usable output here, for the default pair too
        // (`mirror_output eDP-1` again, or naming the target to swap).
        let names: Vec<(String, bool)> = self
            .monitors
            .iter()
            .map(|m| (m.name.clone(), m.enabled || self.mirror.is_target(&m.name)))
            .collect();
        let views: Vec<OutputView<'_>> = names
            .iter()
            .map(|(name, enabled)| OutputView {
                name,
                enabled: *enabled,
            })
            .collect();
        let focused = self
            .monitors
            .get(self.focused_monitor())
            .map(|m| m.name.clone());

        let (source, target) = match (source, target) {
            (Some(s), Some(t)) => (s.to_string(), t.to_string()),
            (s, t) => {
                let (ds, dt) =
                    default_pair(&views, focused.as_deref()).ok_or(MirrorError::NoPair)?;
                match (s, t) {
                    (Some(s), None) => {
                        // Source given: the default target, or the default
                        // source when that is the one asked to be shown.
                        let t = if dt == s { ds } else { dt };
                        (s.to_string(), t)
                    }
                    (None, Some(t)) => {
                        let s = if ds == t { dt } else { ds };
                        (s, t.to_string())
                    }
                    _ => (ds, dt),
                }
            }
        };
        let change = self.mirror.start(&source, &target, &views)?;
        self.apply_mirror_change(change);
        Ok(())
    }

    /// Stop mirroring (no-op when idle).
    pub fn stop_mirror(&mut self) {
        let change = self.mirror.stop();
        self.apply_mirror_change(change);
    }

    /// Stop when mirroring, else start like [`Self::start_mirror`].
    pub fn toggle_mirror(
        &mut self,
        source: Option<&str>,
        target: Option<&str>,
    ) -> Result<(), MirrorError> {
        if self.mirror.active().is_some() {
            self.stop_mirror();
            Ok(())
        } else {
            self.start_mirror(source, target)
        }
    }

    /// `name` was unplugged or switched on/off by hand: a mirror using it
    /// ends (auto-stop), and its target returns to the layout.
    pub fn mirror_output_gone(&mut self, name: &str) {
        let change = self.mirror.output_gone(name);
        self.apply_mirror_change(change);
    }

    /// Record a request's outcome for the `mirror.error` IPC field, which
    /// the shell shows. The requester itself gets the refusal in its
    /// dispatch reply: `mirror.error` is last-writer-wins across clients.
    pub fn note_mirror_result(&mut self, result: Result<(), MirrorError>) {
        let error = result.err().map(|e| {
            tracing::warn!(error = %e, "mirror request refused");
            e.to_string()
        });
        if self.mirror.last_error != error {
            self.mirror.last_error = error;
            self.mark_state_dirty();
        }
    }

    /// `name` was unplugged: a mirror using it ends. A lost target is not
    /// put back in the layout — it is about to be removed.
    pub fn mirror_output_unplugged(&mut self, name: &str) {
        if self.mirror.is_target(name) {
            if let MirrorChange::Stopped(session) = self.mirror.stop() {
                tracing::info!(
                    source = %session.source,
                    target = %session.target,
                    "mirror target unplugged: stopped mirroring"
                );
                self.mark_state_dirty();
            }
        } else {
            self.mirror_output_gone(name);
        }
    }

    fn apply_mirror_change(&mut self, change: MirrorChange) {
        match change {
            MirrorChange::Unchanged => return,
            MirrorChange::Started { session, replaced } => {
                if let Some(old) = replaced
                    && old.target != session.target
                    && let Some(idx) = self.monitors.iter().position(|m| m.name == old.target)
                {
                    self.put_monitor_back_in_layout(idx);
                }
                let Some(idx) = self.monitors.iter().position(|m| m.name == session.target) else {
                    return;
                };
                if self.monitors[idx].enabled && !self.take_monitor_out_of_layout(idx, false) {
                    // Refused (no other enabled output): nothing is mirrored.
                    self.mirror.stop();
                    return;
                }
                tracing::info!(
                    source = %session.source,
                    target = %session.target,
                    "mirroring output"
                );
            }
            MirrorChange::Stopped(session) => {
                if let Some(idx) = self.monitors.iter().position(|m| m.name == session.target) {
                    self.put_monitor_back_in_layout(idx);
                }
                tracing::info!(
                    source = %session.source,
                    target = %session.target,
                    "stopped mirroring output"
                );
            }
        }
        self.wake_repaint_backend();
        self.request_repaint();
        self.mark_state_dirty();
    }
}
