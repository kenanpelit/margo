//! `MargoState` glue for the per-output frame clock (on by default).
//!
//! The pure scheduling primitives live in [`crate::frame_clock`]; this
//! file is the `MargoState`-coupled side: creating/arming the per-output
//! present timers, marking clocks dirty, and the vblank re-arm. Every
//! method here is a no-op (or early-returns) when
//! `config.per_output_frame_clock` is false, so the global-tick path
//! remains available as an escape hatch.
//!
//! ## Flow (flag on)
//!
//! 1. Something dirties the scene → [`MargoState::request_repaint`]
//!    calls [`MargoState::mark_all_clocks_dirty`], which flags every
//!    output clock dirty and ensures each has a present timer armed.
//! 2. A clock's present `Timer` fires at `last_present + refresh` →
//!    [`MargoState::present_timer_fired`] re-pings the repaint source.
//! 3. The repaint ping callback (it owns the backend handle) renders
//!    only the outputs that are *due* (dirty + interval elapsed + not
//!    awaiting a vblank), via [`MargoState::take_due_outputs`].
//! 4. The DRM vblank lands → [`MargoState::note_vblank_per_output`]
//!    stamps `last_present`, clears the in-flight gate, and re-arms the
//!    timer for the next frame.

use std::time::{Duration, Instant};

use smithay::output::Output;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::frame_clock::refresh_interval_from_mhz;
use crate::state::MargoState;

impl MargoState {
    /// Whether the opt-in per-output frame clock is active.
    #[inline]
    pub fn per_output_frame_clock_enabled(&self) -> bool {
        self.config.per_output_frame_clock
    }

    /// Refresh interval for `output`, from its current mode (mHz),
    /// falling back to 60 Hz on a missing / zero-refresh mode.
    pub fn output_refresh_interval(output: &Output) -> Duration {
        output
            .current_mode()
            .map(|m| refresh_interval_from_mhz(m.refresh))
            .unwrap_or_else(|| refresh_interval_from_mhz(0))
    }

    /// Ensure a clock entry exists for every currently-known output.
    /// Cheap + idempotent; called before arming timers so freshly
    /// hotplugged outputs join the per-output schedule.
    pub fn ensure_output_clocks(&mut self) {
        if !self.per_output_frame_clock_enabled() {
            return;
        }
        let names: Vec<String> = self.monitors.iter().map(|m| m.output.name()).collect();
        for name in names {
            self.per_output_clocks.entry(name).or_default();
        }
    }

    /// Mark every output's clock dirty and make sure each has a present
    /// timer armed. Invoked from `request_repaint` on the opt-in path —
    /// a global repaint can affect any output, so all are flagged; the
    /// per-output timers then gate *when* each one actually renders.
    pub fn mark_all_clocks_dirty(&mut self) {
        if !self.per_output_frame_clock_enabled() {
            return;
        }
        self.ensure_output_clocks();
        let names: Vec<String> = self.per_output_clocks.keys().cloned().collect();
        for name in names {
            if let Some(clock) = self.per_output_clocks.get_mut(&name) {
                clock.dirty = true;
            }
            self.arm_present_timer(&name);
        }
    }

    /// Arm (or re-arm) the present timer for `name`, scheduled at the
    /// output's `last_present + refresh_interval`. If the output is
    /// already in-flight (`pending_vblank`) or no timer is needed yet,
    /// this is a no-op — the vblank handler re-arms once the flip lands.
    pub fn arm_present_timer(&mut self, name: &str) {
        if !self.per_output_frame_clock_enabled() {
            return;
        }
        // Find the live output for its refresh interval.
        let Some(output) = self
            .monitors
            .iter()
            .find(|m| m.output.name() == name)
            .map(|m| m.output.clone())
        else {
            return;
        };
        let interval = Self::output_refresh_interval(&output);

        let Some(clock) = self.per_output_clocks.get_mut(name) else {
            return;
        };
        // Already waiting on a flip, or a timer is already pending —
        // don't stack a second source.
        if clock.pending_vblank || clock.timer_token.is_some() {
            return;
        }
        let delay = clock.next_tick_delay(Instant::now(), interval);

        let timer = Timer::from_duration(delay);
        let cb_name = name.to_string();
        let token = self.loop_handle.insert_source(timer, move |_, _, state| {
            // Clear our token (the source is one-shot) before doing
            // work so `present_timer_fired` → `arm_present_timer` can
            // schedule the next one cleanly.
            if let Some(clock) = state.per_output_clocks.get_mut(&cb_name) {
                clock.timer_token = None;
            }
            state.present_timer_fired(&cb_name);
            TimeoutAction::Drop
        });

        match token {
            Ok(t) => {
                if let Some(clock) = self.per_output_clocks.get_mut(name) {
                    clock.timer_token = Some(t);
                }
            }
            Err(e) => tracing::warn!("arm_present_timer insert_source failed: {e}"),
        }
    }

    /// Present timer for `name` fired: the output may now be due. We
    /// don't render here (the timer callback has no backend handle) —
    /// instead wake the repaint source, whose callback owns the backend
    /// and renders exactly the due outputs via [`Self::take_due_outputs`].
    pub fn present_timer_fired(&mut self, _name: &str) {
        if !self.per_output_frame_clock_enabled() {
            return;
        }
        // Wake the redraw scheduler. Bypasses the global
        // `pending_vblanks` gate intentionally: per-output in-flight
        // state lives on each `OutputClock`, and `take_due_outputs`
        // re-checks it, so a fast output isn't held back by a slow
        // output's pending flip.
        if let Some(ping) = self.repaint_ping_handle() {
            ping.ping();
        }
    }

    /// Outputs that should render *now*: dirty, past their refresh
    /// interval, and not awaiting a vblank. Clears each returned
    /// output's dirty flag and marks it in-flight (`pending_vblank`)
    /// so a second tick can't double-submit before the flip lands.
    /// Returns the matching `Output`s for the render loop.
    pub fn take_due_outputs(&mut self) -> Vec<Output> {
        if !self.per_output_frame_clock_enabled() {
            return Vec::new();
        }
        self.ensure_output_clocks();
        let now = Instant::now();
        let mut due = Vec::new();
        // Snapshot (name, output, interval) to avoid borrowing
        // `monitors` and `per_output_clocks` simultaneously.
        let candidates: Vec<(String, Output, Duration)> = self
            .monitors
            .iter()
            .filter(|m| m.enabled)
            .map(|m| {
                let out = m.output.clone();
                let interval = Self::output_refresh_interval(&out);
                (out.name(), out, interval)
            })
            .collect();
        for (name, output, interval) in candidates {
            let Some(clock) = self.per_output_clocks.get_mut(&name) else {
                continue;
            };
            if clock.is_due(now, interval) {
                clock.dirty = false;
                clock.pending_vblank = true;
                due.push(output);
            }
        }
        due
    }

    /// Per-output vblank: the page-flip for `output` landed. Stamp
    /// `last_present`, drop the in-flight gate, and re-arm the present
    /// timer (re-dirtied if the scene is still animating, which a later
    /// `request_repaint` will flag). Mirrors the global path's
    /// `note_vblank` but scoped to one output.
    pub fn note_vblank_per_output(&mut self, output: &Output) {
        if !self.per_output_frame_clock_enabled() {
            return;
        }
        let name = output.name();
        if let Some(clock) = self.per_output_clocks.get_mut(&name) {
            clock.last_present = Some(Instant::now());
            clock.pending_vblank = false;
            // A timer may still be pending from before the flip; leave
            // it — `arm_present_timer` no-ops if a token exists. If
            // none is pending, arm the next one now.
        }
        self.arm_present_timer(&name);

        // Frame callbacks: same contract as the global `note_vblank`,
        // scoped to this output so its clients pace at its refresh.
        let now = self.clock.now();
        self.send_frame_callbacks(output, now);
    }

    /// Empty render on the per-output path: `render_frame` reported no
    /// damage, so no DRM page-flip and no vblank will come back to clear
    /// this output's in-flight gate. Treat it like a (zero-cost) present:
    /// stamp `last_present`, drop `pending_vblank`, re-arm the timer, and
    /// send frame callbacks so the output's clients still pace at its
    /// refresh rate (the per-output analogue of the global path's
    /// `queue_estimated_vblank_timer`).
    pub fn note_empty_render_per_output(&mut self, output: &Output) {
        if !self.per_output_frame_clock_enabled() {
            return;
        }
        let name = output.name();
        if let Some(clock) = self.per_output_clocks.get_mut(&name) {
            clock.last_present = Some(Instant::now());
            clock.pending_vblank = false;
        }
        self.arm_present_timer(&name);

        // Bump the per-output frame-callback sequence so dedup advances,
        // then send callbacks — mirrors `on_estimated_vblank_timer`.
        let entry = self
            .frame_callback_sequence
            .entry(name.clone())
            .or_insert(0);
        *entry = entry.wrapping_add(1);
        let now = self.clock.now();
        self.send_frame_callbacks(output, now);
    }

    /// Release this output's in-flight gate without stamping a present.
    /// Used when a `queue_frame` failed: no flip was scheduled, so no
    /// vblank is coming, and the gate would otherwise wedge the output.
    /// Re-arms the present timer so the next tick retries.
    pub fn clear_pending_vblank_per_output(&mut self, output: &Output) {
        if !self.per_output_frame_clock_enabled() {
            return;
        }
        let name = output.name();
        if let Some(clock) = self.per_output_clocks.get_mut(&name) {
            clock.pending_vblank = false;
        }
        self.arm_present_timer(&name);
    }

    /// Drop the per-output clock for a removed output (hotplug-out), so
    /// a stale timer doesn't keep waking the loop for a gone display.
    pub fn drop_output_clock(&mut self, name: &str) {
        if let Some(clock) = self.per_output_clocks.remove(name)
            && let Some(token) = clock.timer_token
        {
            self.loop_handle.remove(token);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `MargoState` is hard to build in a unit test (it owns a full
    // Wayland display + event loop), so the scheduler's *pure* logic is
    // covered in `crate::frame_clock::tests`. Here we only assert the
    // refresh-interval mapping that this module layers on top of the
    // pure helper, since it reads from an `Output` we can construct
    // cheaply.
    use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
    use smithay::utils::Size;

    fn make_output(refresh_mhz: i32) -> Output {
        let output = Output::new(
            "test-output".into(),
            PhysicalProperties {
                size: (300, 200).into(),
                subpixel: Subpixel::Unknown,
                make: "margo".into(),
                model: "test".into(),
                serial_number: "test".into(),
            },
        );
        let mode = Mode {
            size: Size::from((1920, 1080)),
            refresh: refresh_mhz,
        };
        output.change_current_state(Some(mode), None, None, None);
        output.set_preferred(mode);
        output
    }

    #[test]
    fn refresh_interval_reads_output_mode() {
        let out = make_output(144_000);
        let d = MargoState::output_refresh_interval(&out);
        assert!((d.as_secs_f64() - 1.0 / 144.0).abs() < 1e-6);
    }

    #[test]
    fn refresh_interval_zero_mode_falls_back() {
        let out = make_output(0);
        let d = MargoState::output_refresh_interval(&out);
        assert!((d.as_secs_f64() - 1.0 / 60.0).abs() < 1e-9);
    }
}
