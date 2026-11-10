// SPDX-License-Identifier: GPL-3.0-or-later
//! Remember the player volume across restarts.
//!
//! The level lives in the `volume` GSettings key (the same cubic 0..=1
//! scale the volume slider, MPRIS `Volume` and `org.margo.Tune.SetVolume`
//! use). Every volume change — slider, scroll, mute, tray, MPRIS, the
//! shell's Tune menu — lands in `PlayerState::volume`, so persistence
//! hangs off that one property. Writes are debounced: a slider drag or a
//! scroll burst fires dozens of changes and only the value it settles on
//! is worth a dconf write. Shutdown flushes whatever is still pending.

use std::{cell::RefCell, rc::Rc, time::Duration};

use gtk::{gio, glib, prelude::*};

/// GSettings key holding the remembered level.
pub const KEY: &str = "volume";

/// How long the volume must stay put before it is written.
pub const DEBOUNCE: Duration = Duration::from_millis(600);

/// Changes smaller than this are not worth a write (the state already
/// rounds its change notifications to two decimals).
const EPSILON: f64 = 0.005;

/// Bring any stored / requested level into `0.0..=1.0`. A non-finite
/// value (a hand-edited dconf entry) falls back to full volume, the
/// player's own default.
pub fn sanitize(volume: f64) -> f64 {
    if volume.is_finite() {
        volume.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

/// Pure debounce state: which value is waiting to be written, which timer
/// generation is allowed to write it, and what was last written.
#[derive(Debug, Clone, PartialEq)]
pub struct Debounce {
    pending: Option<f64>,
    generation: u64,
    saved: f64,
}

impl Debounce {
    /// `saved` is the value currently stored (read at startup).
    pub fn new(saved: f64) -> Self {
        Self {
            pending: None,
            generation: 0,
            saved: sanitize(saved),
        }
    }

    /// Record a new volume. Returns the generation the caller's timer must
    /// pass to [`Debounce::fire`]; any earlier timer becomes stale.
    pub fn note(&mut self, volume: f64) -> u64 {
        self.pending = Some(sanitize(volume));
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }

    /// A timer for `generation` elapsed. Returns the value to write, if
    /// this is still the latest change and it differs from what is stored.
    pub fn fire(&mut self, generation: u64) -> Option<f64> {
        if generation != self.generation {
            return None;
        }
        self.take()
    }

    /// Write whatever is pending now (shutdown), ignoring the timer.
    pub fn flush(&mut self) -> Option<f64> {
        self.take()
    }

    fn take(&mut self) -> Option<f64> {
        let value = self.pending.take()?;
        if (value - self.saved).abs() < EPSILON {
            return None;
        }
        self.saved = value;
        Some(value)
    }
}

/// The remembered level to apply at startup.
pub fn load(settings: &gio::Settings) -> f64 {
    sanitize(settings.double(KEY))
}

/// Debounced writer bound to a `gio::Settings`.
#[derive(Debug, Clone)]
pub struct VolumeMemory {
    settings: gio::Settings,
    state: Rc<RefCell<Debounce>>,
}

impl VolumeMemory {
    pub fn new(settings: gio::Settings) -> Self {
        let saved = load(&settings);
        Self {
            settings,
            state: Rc::new(RefCell::new(Debounce::new(saved))),
        }
    }

    /// The volume changed; write it once it has been stable for
    /// [`DEBOUNCE`]. Runs on the thread-default GLib main context.
    pub fn note(&self, volume: f64) {
        let generation = self.state.borrow_mut().note(volume);
        let this = self.clone();
        // The thread-default context: the GTK main loop in the app, the
        // test's own context under `with_thread_default`.
        glib::MainContext::ref_thread_default().spawn_local(async move {
            glib::timeout_future(DEBOUNCE).await;
            let value = this.state.borrow_mut().fire(generation);
            if let Some(value) = value {
                this.write(value);
            }
        });
    }

    /// Write a pending change immediately (application shutdown).
    pub fn flush(&self) {
        let value = self.state.borrow_mut().flush();
        if let Some(value) = value {
            self.write(value);
        }
    }

    fn write(&self, value: f64) {
        if let Err(e) = self.settings.set_double(KEY, value) {
            log::debug!("mtune: could not remember volume {value}: {e}");
        }
    }
}

#[cfg(test)]
#[path = "volume_memory_tests.rs"]
mod tests;
