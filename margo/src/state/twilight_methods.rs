//! Twilight (blue-light filter) integration methods on `MargoState`.
//!
//! Extracted from `state.rs` (roadmap Q1). The data side of twilight
//! lives in `crate::twilight` — schedule math, settings, the LUT
//! builder. This module is the thin glue between that pure engine
//! and the live compositor: scheduling the calloop tick timer, and
//! pushing computed ramps onto every monitor's `output_pending.gamma`
//! queue for the udev backend to consume on the next frame.

use smithay::output::Output;

use super::MargoState;

/// Twilight's live state: the pure schedule engine plus the compositor-side
/// bits that drive it (tick timer, applied-ramp flag, preset cache).
pub struct Twilight {
    /// Twilight (blue-light filter) state. Always present; activity
    /// is gated by `Config::twilight`. `tick_twilight()` advances
    /// this on the calloop timer and pushes gamma ramps into
    /// `output_pending.gamma` for every connected output.
    pub engine: crate::twilight::TwilightState,
    /// Token of the single self-re-arming twilight tick timer. A
    /// force-tick (from `mctl twilight …` / `reload_config`) removes
    /// this one and re-inserts at the near-term interval instead of
    /// stacking a fresh permanent ticker per dispatch — the old
    /// `bool` guard was never consulted, so every toggle leaked a
    /// timer that kept waking the loop forever.
    pub timer_token: Option<smithay::reexports::calloop::RegistrationToken>,
    /// Whether a non-identity twilight ramp is currently applied. Starts
    /// `true` so the first disabled tick clears once (guaranteeing
    /// identity at startup); thereafter `clear_twilight_ramp` no-ops
    /// while already cleared, so a disabled-twilight session stops
    /// re-pushing gamma + repainting every 60 s.
    pub ramp_active: bool,
    /// Cached `(schedule_dir, ScheduleData)` for Schedule mode. The schedule
    /// presets are static between explicit changes, but `tick_twilight` runs
    /// as often as every 50 ms during a sunrise/sunset sweep — re-reading +
    /// parsing the seven preset files on every one of those ticks was pure
    /// waste. We load once and serve from here; the cache is cleared on
    /// config reload and on every `mctl twilight` command (both invalidation
    /// points users actually change presets through).
    pub schedule_cache: Option<(String, crate::twilight::preset::ScheduleData)>,
}

impl Twilight {
    pub fn new() -> Self {
        Self {
            engine: crate::twilight::TwilightState::default(),
            timer_token: None,
            ramp_active: true,
            schedule_cache: None,
        }
    }
}

impl Default for Twilight {
    fn default() -> Self {
        Self::new()
    }
}

impl MargoState {
    /// Force-tick twilight from a dispatch path and schedule the
    /// next tick at the duration the tick body just asked for.
    ///
    /// The plain `tick_twilight` returns a `Duration` so the calloop
    /// timer (the steady-state caller) can `ToDuration` itself. But
    /// the dispatchers behind `mctl twilight preview / test / reset`
    /// don't run from that timer — they run from the input-handler
    /// path. If we only force-ticked there without rearming, the
    /// next live update wouldn't happen until the calloop timer's
    /// already-scheduled fire (60 s away by default at steady state).
    /// That's the "test command is instant" symptom — sweep
    /// progress was sampled once at start, then nothing until the
    /// timer eventually fired half a minute later, by which time
    /// the sweep had already elapsed and we landed on Scheduled.
    ///
    /// Fix: RE-ARM the single steady-state timer rather than stacking a
    /// new one. calloop timers can't be rescheduled in place, so we
    /// remove the in-flight token and insert a fresh self-re-arming
    /// timer at the near-term interval, storing the new token. The old
    /// code inserted a permanent self-re-arming timer here on *every*
    /// call, so each `mctl twilight` toggle leaked a ticker that woke
    /// the loop (and forced a gamma repaint) for the rest of the
    /// session. `tick_twilight` is idempotent, so re-arming is safe.
    pub fn force_tick_twilight(&mut self) {
        // Every explicit twilight change (toggle / reset / set / preview /
        // test, and the `mctl twilight preset` writers) funnels through here,
        // so this is the right place to drop the schedule cache and pick up
        // edited preset files on the resample below.
        self.twilight.schedule_cache = None;
        let next = self.tick_twilight();
        // Persist the new twilight state to state snapshot. Every explicit
        // user action (`mctl twilight toggle/reset/set/preview/test`,
        // reload_config) comes through here, but the steady-state
        // calloop timer calls `tick_twilight` directly — so this writes
        // on the `enabled`/config changes that consumers (the shell's
        // night-light button polls `mctl twilight status`) must see,
        // without spamming a file write on every 50 ms transition tick.
        self.mark_state_dirty();
        // Drop the currently-scheduled tick and replace it with one at the
        // freshly-computed interval, so exactly one twilight timer is ever
        // in flight.
        if let Some(token) = self.twilight.timer_token.take() {
            self.loop_handle.remove(token);
        }
        let timer = smithay::reexports::calloop::timer::Timer::from_duration(next);
        self.twilight.timer_token = self
            .loop_handle
            .insert_source(timer, move |_, _, state: &mut MargoState| {
                let next = state.tick_twilight();
                smithay::reexports::calloop::timer::TimeoutAction::ToDuration(next)
            })
            .ok();
    }

    /// Schedule-mode presets, served from `twilight.schedule_cache`.
    ///
    /// Non-schedule modes get an empty table (the tick branches on
    /// `Mode::Schedule`, so a populated table only matters there). In schedule
    /// mode we read + parse the preset files at most once per
    /// reload / `mctl twilight` command — not on every 50 ms sweep tick — by
    /// keying the cache on the configured schedule dir. The cache is
    /// invalidated explicitly at those change points (see `force_tick_twilight`
    /// and the reload path), so a dir change here also forces a fresh load.
    fn twilight_presets_cached(&mut self) -> crate::twilight::preset::ScheduleData {
        if !matches!(
            self.config.twilight_mode,
            margo_config::TwilightMode::Schedule
        ) {
            return crate::twilight::preset::ScheduleData::default();
        }
        let dir = &self.config.twilight_schedule_dir;
        let fresh = matches!(&self.twilight.schedule_cache, Some((d, _)) if d == dir);
        if !fresh {
            let data = crate::twilight::preset::ScheduleData::load(dir);
            self.twilight.schedule_cache = Some((dir.clone(), data));
        }
        self.twilight
            .schedule_cache
            .as_ref()
            .map(|(_, d)| d.clone())
            .unwrap_or_default()
    }

    /// Advance twilight one tick + apply the resulting ramp to every
    /// connected output. Called from the calloop timer (steady-state
    /// path), from `reload_config` (force resample on config change),
    /// and from the `mctl twilight` dispatchers (force resample after
    /// preview / test / reset). Returns the desired sleep before the
    /// next automatic tick — the caller schedules a `calloop::timer`
    /// for that duration.
    pub fn tick_twilight(&mut self) -> std::time::Duration {
        // Resolve the schedule presets first (this needs `&mut self` for the
        // cache); the rest of the tick only borrows `&self.config`.
        let presets = self.twilight_presets_cached();
        let cfg = &self.config;
        let inputs = crate::twilight::TickInputs {
            enabled: cfg.twilight,
            schedule: crate::twilight::schedule_from_config(cfg),
            settings: crate::twilight::settings_from_config(cfg),
            is_static: matches!(cfg.twilight_mode, margo_config::TwilightMode::Static),
            idle_interval_s: cfg.twilight_update_interval,
            now: std::time::SystemTime::now(),
            presets,
        };
        let out = self.twilight.engine.tick(inputs);

        if let Some(target) = out.target {
            self.apply_twilight_ramp(target.temp_k, target.gamma_pct);
        } else if !cfg.twilight {
            // Twilight just got disabled — clear ramps back to
            // identity so the screen doesn't stay tinted.
            self.clear_twilight_ramp();
        }

        std::time::Duration::from_millis(out.next_tick_ms.max(50))
    }

    /// Build a ramp per output (each CRTC may declare a different
    /// `GAMMA_LUT_SIZE` — Intel Arc reports 1024, older AMD parts
    /// often 256, virtio sometimes 4096) and push them to
    /// `output_pending.gamma`. The udev frame handler picks each one up
    /// on the very next render and hands it to `g.set_gamma`,
    /// which validates the length against the kernel's declared
    /// size and rejects mismatches.
    fn apply_twilight_ramp(&mut self, temp_k: u32, gamma_pct: u32) {
        // Memo: most outputs share the same gamma_size, so cache
        // the last build so we don't pay the LUT compute again for
        // identical sizes within a single tick.
        let mut last_size: u32 = 0;
        let mut cached: Vec<u16> = Vec::new();

        // Clone the outputs we need to push to up-front so we can
        // mutate `output_pending.gamma` without overlapping borrows.
        let targets: Vec<(Output, u32)> = self
            .monitors
            .iter()
            .map(|m| (m.output.clone(), m.gamma_size))
            .collect();

        for (output, size) in targets {
            // gamma_size == 0 means the output's CRTC doesn't expose
            // GAMMA_LUT at all (winit backend, headless, certain
            // virtual connectors). Skip — sending a ramp would just
            // log a warning every tick.
            if size == 0 {
                continue;
            }
            if size != last_size {
                cached = crate::twilight::gamma_lut::build_ramp(temp_k, gamma_pct, size as usize);
                last_size = size;
            }
            // Drop any pending entry for this output first so we
            // never queue stale ramps behind the latest one.
            self.output_pending.gamma.retain(|(o, _)| o != &output);
            self.output_pending
                .gamma
                .push((output, Some(cached.clone())));
        }
        self.twilight.ramp_active = true;
        self.request_repaint();
    }

    /// Restore the kernel's default identity ramp on every output.
    /// Used when twilight is disabled mid-session. No-ops while already
    /// cleared so a disabled-twilight session doesn't re-push identity
    /// gamma + repaint every tick — the disabled branch of
    /// `tick_twilight` calls this once per minute otherwise.
    fn clear_twilight_ramp(&mut self) {
        if !self.twilight.ramp_active {
            return;
        }
        for mon in &self.monitors {
            self.output_pending.gamma.retain(|(o, _)| o != &mon.output);
            self.output_pending.gamma.push((mon.output.clone(), None));
        }
        self.twilight.ramp_active = false;
        self.request_repaint();
    }
}
