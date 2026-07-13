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
        let names: Vec<String> = self
            .monitors
            .iter()
            .filter(|monitor| monitor.enabled)
            .map(|monitor| monitor.name.clone())
            .collect();
        self.per_output_clocks
            .retain(|name, _| names.contains(name));
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
        let enabled_count = self
            .monitors
            .iter()
            .filter(|monitor| monitor.enabled)
            .count();
        // Fast path: if every known output already has a dirty clock with
        // a timer armed (or a flip in flight), a repeated request_repaint
        // in the same frame — the common case during a commit burst — has
        // nothing to add. Skips the snapshot + per-output name()/arm work
        // below. A count mismatch (hotplug, missing clock) falls through
        // to the full loop, so this stays correct.
        if !self.per_output_clocks.is_empty()
            && self.per_output_clocks.len() == enabled_count
            && self.per_output_clocks.iter().all(|(name, clock)| {
                clock.dirty
                    && (clock.timer_token.is_some()
                        || clock.pending_vblank
                        || self.render_retry_pending.contains_key(name))
            })
        {
            return;
        }
        // Snapshot the live outputs once (name + refresh) and reuse it to
        // both dirty the clock and arm its timer. The old code did
        // `keys().cloned()` (a Vec<String>) then re-found each output by
        // name inside every `arm_present_timer` — O(n²) `Output::name()`
        // clones-under-mutex on a path that runs on every pointer motion
        // and surface commit.
        let outputs: Vec<(String, Output, Duration)> = self
            .monitors
            .iter()
            .filter(|monitor| monitor.enabled)
            .map(|m| {
                let out = m.output.clone();
                let interval = Self::output_refresh_interval(&out);
                (m.name.clone(), out, interval)
            })
            .collect();
        for (name, output, interval) in &outputs {
            let clock = self.per_output_clocks.entry(name.clone()).or_default();
            clock.dirty = true;
            if !self.render_retry_pending.contains_key(name) {
                self.arm_present_timer_for(name, output, *interval);
            }
        }
    }

    /// Mark exactly one output dirty, coalescing a commit burst behind the
    /// output's already-armed timer or in-flight vblank. The hot fast path does
    /// no `String`/`Output` clone: Chromium may commit several synchronized
    /// subsurfaces for one video frame, and only the root commit should reach
    /// here, but repeated root commits still need to be cheap.
    ///
    /// Returns `true` only when timer insertion failed and the caller must ping
    /// immediately because no timer/vblank will wake the scheduler.
    pub fn mark_output_clock_dirty(&mut self, output: &Output) -> bool {
        if !self.per_output_frame_clock_enabled() {
            return false;
        }
        let Some(mon_idx) = self
            .monitors
            .iter()
            .position(|monitor| monitor.enabled && monitor.output == *output)
        else {
            return false;
        };

        let name = self.monitors[mon_idx].name.as_str();
        if self.render_retry_pending.contains_key(name) {
            self.per_output_clocks
                .entry(name.to_string())
                .or_default()
                .dirty = true;
            return false;
        }
        if self.per_output_clocks.get(name).is_some_and(|clock| {
            clock.dirty && (clock.timer_token.is_some() || clock.pending_vblank)
        }) {
            return false;
        }

        let output = self.monitors[mon_idx].output.clone();
        let interval = Self::output_refresh_interval(&output);
        let name = self.monitors[mon_idx].name.clone();
        let clock = self.per_output_clocks.entry(name.clone()).or_default();
        clock.dirty = true;
        self.arm_present_timer_for(&name, &output, interval);
        !self
            .per_output_clocks
            .get(&name)
            .is_some_and(|clock| clock.timer_token.is_some() || clock.pending_vblank)
    }

    /// Put an output rendered by the forced all-outputs path under the same
    /// bookkeeping as an ordinary due-output render. This path is used for
    /// startup and deferred backend work such as DPMS. Without it,
    /// `render_all_outputs` increments the legacy global vblank counter while
    /// the DRM event is handled by [`Self::note_vblank_per_output`], leaking
    /// the global counter on every forced frame.
    pub fn begin_forced_render_per_output(&mut self, output: &Output) {
        if !self.per_output_frame_clock_enabled() {
            return;
        }
        let clock = self.per_output_clocks.entry(output.name()).or_default();
        clock.dirty = false;
        clock.pending_vblank = true;
    }

    /// Arm (or re-arm) the present timer for `name`, scheduled at the
    /// output's `last_present + refresh_interval`. If the output is
    /// already in-flight (`pending_vblank`) or no timer is needed yet,
    /// this is a no-op — the vblank handler re-arms once the flip lands.
    pub fn arm_present_timer(&mut self, name: &str) {
        if !self.per_output_frame_clock_enabled() {
            return;
        }
        // Find the live output for its refresh interval, then defer to the
        // shared arm. Hot callers that already hold the output
        // (`mark_all_clocks_dirty`) skip this lookup via
        // `arm_present_timer_for`.
        let Some((output, interval)) =
            self.monitors
                .iter()
                .find(|m| m.output.name() == name)
                .map(|m| {
                    let out = m.output.clone();
                    let interval = Self::output_refresh_interval(&out);
                    (out, interval)
                })
        else {
            return;
        };
        self.arm_present_timer_for(name, &output, interval);
    }

    /// Arm the present timer for `name` using an already-resolved output +
    /// refresh interval, skipping the by-name monitor lookup in
    /// [`Self::arm_present_timer`]. Same no-op guards.
    fn arm_present_timer_for(&mut self, name: &str, output: &Output, interval: Duration) {
        let Some(clock) = self.per_output_clocks.get_mut(name) else {
            return;
        };
        // Already waiting on a flip, or a timer is already pending —
        // don't stack a second source.
        if clock.pending_vblank || clock.timer_token.is_some() {
            return;
        }
        let delay = clock.next_tick_delay(Instant::now(), interval);
        self.next_present_timer_id = self.next_present_timer_id.wrapping_add(1).max(1);
        let timer_id = self.next_present_timer_id;
        clock.timer_id = timer_id;

        let timer = Timer::from_duration(delay);
        let cb_name = name.to_string();
        let cb_output = output.clone();
        let token = self.loop_handle.insert_source(timer, move |_, _, state| {
            // Connector names can be reused after a quick unplug/replug. An
            // old one-shot must not clear the replacement output's token or
            // wake its clock early.
            let owns_clock = state.monitors.iter().any(|monitor| {
                monitor.enabled && monitor.output == cb_output && monitor.name == cb_name
            }) && state
                .per_output_clocks
                .get(&cb_name)
                .is_some_and(|clock| clock.timer_id == timer_id);
            if !owns_clock {
                return TimeoutAction::Drop;
            }
            // Clear our token (the source is one-shot) before doing
            // work so `present_timer_fired` → `arm_present_timer` can
            // schedule the next one cleanly.
            if let Some(clock) = state.per_output_clocks.get_mut(&cb_name) {
                clock.timer_token = None;
                clock.timer_id = 0;
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
    pub fn present_timer_fired(&mut self, name: &str) {
        if !self.per_output_frame_clock_enabled() {
            return;
        }
        // Logical cancellation on hot-unplug removes the clock but leaves the
        // one-shot calloop source alive.  Its eventual event must be a no-op,
        // not an unrelated global wake.
        if !self.per_output_clocks.contains_key(name) {
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
            if self.render_retry_pending.contains_key(&name) {
                continue;
            }
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

    /// Complete a DRM vblank using the accounting mode that actually queued
    /// the frame, rather than the config value at completion time. The config
    /// can be reloaded while a flip is in flight; choosing by the new value
    /// would leak either the per-output gate or the legacy global counter.
    pub fn note_backend_vblank(&mut self, output: &Output) {
        let pending_on_output_clock = self
            .per_output_clocks
            .get(&output.name())
            .is_some_and(|clock| clock.pending_vblank);
        if pending_on_output_clock {
            self.note_vblank_per_output(output);
        } else {
            self.note_vblank(output);
        }
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

    /// Release this output's in-flight gate without stamping a present or
    /// arming an immediate timer. Render failures use
    /// [`Self::defer_output_render_retry`] to establish a bounded deadline;
    /// DPMS/disabled paths are woken by their next explicit repaint request.
    pub fn clear_pending_vblank_per_output(&mut self, output: &Output) {
        if !self.per_output_frame_clock_enabled() {
            return;
        }
        let name = output.name();
        if let Some(clock) = self.per_output_clocks.get_mut(&name) {
            clock.pending_vblank = false;
        }
    }

    /// Defer a failed render/queue submission behind a single one-shot timer.
    /// The delay starts at one refresh interval (never below 16 ms), doubles
    /// for consecutive failures, and caps at one second. This applies in both
    /// clock modes and, crucially, never manually removes a calloop source.
    pub fn defer_output_render_retry(&mut self, output: &Output, failure_streak: u32) {
        let name = output.name();
        if self.per_output_frame_clock_enabled() {
            let clock = self.per_output_clocks.entry(name.clone()).or_default();
            clock.pending_vblank = false;
            clock.dirty = true;
        }
        if self.render_retry_pending.contains_key(&name) {
            return;
        }
        self.next_render_retry_id = self.next_render_retry_id.wrapping_add(1).max(1);
        let retry_id = self.next_render_retry_id;
        self.render_retry_pending
            .insert(name.clone(), (output.clone(), retry_id));

        let base = Self::output_refresh_interval(output).max(Duration::from_millis(16));
        let shift = failure_streak.saturating_sub(1).min(6);
        let delay = base
            .saturating_mul(1_u32 << shift)
            .min(Duration::from_secs(1));
        let retry_output = output.clone();
        let cb_name = name.clone();
        let timer = Timer::from_duration(delay);
        if let Err(error) = self.loop_handle.insert_source(timer, move |_, _, state| {
            let owns_retry = state.render_retry_pending.get(&cb_name).is_some_and(
                |(pending_output, pending_id)| {
                    pending_output == &retry_output && *pending_id == retry_id
                },
            );
            if owns_retry {
                state.render_retry_pending.remove(&cb_name);
            }
            if owns_retry
                && state
                    .monitors
                    .iter()
                    .any(|monitor| monitor.enabled && monitor.output == retry_output)
            {
                state.request_repaint_output(&retry_output);
            }
            TimeoutAction::Drop
        }) {
            self.render_retry_pending.remove(&name);
            tracing::warn!(output = %name, ?error, "render retry timer insert failed");
        }
    }

    /// Clear a pending retry after an empty or successfully queued frame. Its
    /// one-shot source may still fire later, but sees no map entry and no-ops.
    pub fn clear_output_render_retry(&mut self, output: &Output) {
        self.render_retry_pending.remove(&output.name());
    }

    pub fn output_render_retry_pending(&self, output: &Output) -> bool {
        self.render_retry_pending
            .get(&output.name())
            .is_some_and(|(pending_output, _)| pending_output == output)
    }

    /// Drop the per-output clock for a removed output (hotplug-out), so
    /// a stale timer doesn't keep waking the loop for a gone display.
    pub fn drop_output_clock(&mut self, name: &str) {
        self.render_retry_pending.remove(name);
        // Present timers are one-shot and their callback first resolves this
        // map entry. Dropping only the logical clock avoids racing a queued
        // calloop event with `LoopHandle::remove` (the source will fire once,
        // observe the missing clock, and no-op).
        self.per_output_clocks.remove(name);
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
