//! Twilight (blue-light filter) integration methods on `MargoState`.
//!
//! Extracted from `state.rs` (roadmap Q1). The data side of twilight
//! lives in `crate::twilight` — schedule math, settings, the LUT
//! builder. This module is the thin glue between that pure engine
//! and the live compositor: scheduling the calloop tick timer, and
//! pushing computed ramps onto every monitor's `pending_gamma`
//! queue for the udev backend to consume on the next frame.

use smithay::output::Output;

use super::MargoState;

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
    /// Fix: insert a fresh single-shot Timer source from here that
    /// re-rearms itself the same way the kick-off timer in
    /// `main.rs` does. Multiple in-flight timers are fine — calloop
    /// keeps them separate, and on each fire `tick_twilight` is
    /// idempotent (it reads live config + override state).
    pub fn force_tick_twilight(&mut self) {
        let next = self.tick_twilight();
        let timer = smithay::reexports::calloop::timer::Timer::from_duration(next);
        let _ = self.loop_handle.insert_source(
            timer,
            move |_, _, state: &mut MargoState| {
                let next = state.tick_twilight();
                smithay::reexports::calloop::timer::TimeoutAction::ToDuration(next)
            },
        );
    }

    /// Advance twilight one tick + apply the resulting ramp to every
    /// connected output. Called from the calloop timer (steady-state
    /// path), from `reload_config` (force resample on config change),
    /// and from the `mctl twilight` dispatchers (force resample after
    /// preview / test / reset). Returns the desired sleep before the
    /// next automatic tick — the caller schedules a `calloop::timer`
    /// for that duration.
    pub fn tick_twilight(&mut self) -> std::time::Duration {
        let cfg = &self.config;
        let inputs = crate::twilight::TickInputs {
            enabled: cfg.twilight,
            schedule: crate::twilight::schedule_from_config(cfg),
            settings: crate::twilight::settings_from_config(cfg),
            is_static: matches!(cfg.twilight_mode, margo_config::TwilightMode::Static),
            idle_interval_s: cfg.twilight_update_interval,
            now: std::time::SystemTime::now(),
        };
        let out = self.twilight.tick(inputs);

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
    /// `pending_gamma`. The udev frame handler picks each one up
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
        // mutate `pending_gamma` without overlapping borrows.
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
                cached = crate::twilight::gamma_lut::build_ramp(
                    temp_k,
                    gamma_pct,
                    size as usize,
                );
                last_size = size;
            }
            // Drop any pending entry for this output first so we
            // never queue stale ramps behind the latest one.
            self.pending_gamma.retain(|(o, _)| o != &output);
            self.pending_gamma.push((output, Some(cached.clone())));
        }
        self.request_repaint();
    }

    /// Restore the kernel's default identity ramp on every output.
    /// Used when twilight is disabled mid-session.
    fn clear_twilight_ramp(&mut self) {
        for mon in &self.monitors {
            self.pending_gamma.retain(|(o, _)| o != &mon.output);
            self.pending_gamma.push((mon.output.clone(), None));
        }
        self.request_repaint();
    }
}
