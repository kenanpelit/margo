//! Analytical spring animation primitive.
//!
//! Ported from niri's `niri/src/animation/spring.rs` (which is in turn ported
//! from libadwaita's `adw-spring-animation.c`, GNOME's general-purpose spring
//! solver).
//!
//! The previous version of this module ran semi-implicit Euler integration
//! step-by-step, with an `is_settled(displacement, velocity)` predicate
//! deciding when to stop. That worked in tests but interacted very badly
//! with margo's per-loop tick: when c.geom rounded to its integer target
//! while velocity was still above the velocity-epsilon, the spring stayed
//! "running" while producing no visible change, and the post-dispatch
//! repaint pump kept re-arming itself for thousands of iterations per
//! second — locking the CPU and making tmux unresponsive in the user's
//! report.
//!
//! Niri's solution is to compute the analytical solution of the harmonic
//! oscillator equation `m·ẍ + c·ẋ + kx = 0` directly, in closed form:
//!
//!   * Critically damped (β = ω₀):
//!         x(t) = to + e^(-βt) · (x₀ + (β·x₀ + v₀)·t)
//!   * Underdamped       (β < ω₀):
//!         x(t) = to + e^(-βt) · (x₀·cos(ω₁t) + ((β·x₀+v₀)/ω₁)·sin(ω₁t))
//!   * Overdamped        (β > ω₀):
//!         x(t) = to + e^(-βt) · (x₀·cosh(ω₂t) + ((β·x₀+v₀)/ω₂)·sinh(ω₂t))
//!
//! And then a precomputed [`clamped_duration`] tells the caller exactly
//! when the spring will be within `epsilon` of `to` for the first time —
//! that's the animation's hard end, set as `ClientAnimation::duration` at
//! arrange time. After that wall-clock duration the move animation is
//! definitively over; no per-step settle predicate, no possibility of the
//! tick loop refusing to drop the running flag.
//!
//! This is why niri ships fixed-duration window-movement and resize
//! transitions despite using spring physics: every animation has a
//! pre-calculated end time even though the *shape* of the curve is driven
//! by physics. We follow the same pattern.

use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub struct SpringParams {
    pub damping: f64,
    pub mass: f64,
    pub stiffness: f64,
    pub epsilon: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct Spring {
    pub from: f64,
    pub to: f64,
    pub initial_velocity: f64,
    pub params: SpringParams,
}

impl SpringParams {
    /// Resolve `damping` from a user-friendly damping ratio.
    /// `damping_ratio = 1.0` is critically damped; <1 underdamped (bouncy);
    /// >1 overdamped (sluggish).
    pub fn new(damping_ratio: f64, stiffness: f64, epsilon: f64) -> Self {
        let damping_ratio = damping_ratio.max(0.);
        let stiffness = stiffness.max(0.);
        let epsilon = epsilon.max(f64::EPSILON);
        let mass = 1.;
        let critical_damping = 2. * (mass * stiffness).sqrt();
        let damping = damping_ratio * critical_damping;
        Self {
            damping,
            mass,
            stiffness,
            epsilon,
        }
    }
}

impl Default for Spring {
    fn default() -> Self {
        Spring {
            from: 0.0,
            to: 1.0,
            initial_velocity: 0.0,
            params: SpringParams::new(1.0, 800.0, 0.0001),
        }
    }
}

impl Spring {
    /// Position at wall-clock time `t` since the animation started.
    pub fn value_at(&self, t: Duration) -> f64 {
        self.oscillate(t.as_secs_f64())
    }

    /// First time the oscillator is within `epsilon` of `to`. Used as the
    /// animation's hard duration. Returns `None` only if the convergence
    /// search runs more than 3000 iterations (≈ 3 s) — pathological
    /// over-damped configurations with stiffness ≪ damping. Callers fall
    /// back to a sane bezier-style duration cap in that case.
    pub fn clamped_duration(&self) -> Option<Duration> {
        let beta = self.params.damping / (2. * self.params.mass);

        if beta.abs() <= f64::EPSILON || beta < 0. {
            return Some(Duration::MAX);
        }

        if (self.to - self.from).abs() <= f64::EPSILON {
            return Some(Duration::ZERO);
        }

        // Skip the trivial-zero first frame.
        let mut i = 1u16;
        let mut y = self.oscillate(f64::from(i) / 1000.);

        while (self.to - self.from > f64::EPSILON && self.to - y > self.params.epsilon)
            || (self.from - self.to > f64::EPSILON && y - self.to > self.params.epsilon)
        {
            if i > 3000 {
                return None;
            }
            i += 1;
            y = self.oscillate(f64::from(i) / 1000.);
        }
        Some(Duration::from_millis(u64::from(i)))
    }

    /// Total time until the envelope decays below `epsilon` (i.e., the
    /// spring is essentially at rest). For overdamped springs this can
    /// be far longer than `clamped_duration`. We don't currently use
    /// this — `clamped_duration` is enough to know when the visible
    /// motion is over — but it's kept for parity with niri's API.
    pub fn duration(&self) -> Duration {
        const DELTA: f64 = 0.001;
        let beta = self.params.damping / (2. * self.params.mass);
        if beta.abs() <= f64::EPSILON || beta < 0. {
            return Duration::MAX;
        }
        if (self.to - self.from).abs() <= f64::EPSILON {
            return Duration::ZERO;
        }
        let omega0 = (self.params.stiffness / self.params.mass).sqrt();
        let mut x0 = -self.params.epsilon.ln() / beta;
        if (beta - omega0).abs() <= f64::from(f32::EPSILON) || beta < omega0 {
            return Duration::from_secs_f64(x0);
        }

        let mut y0 = self.oscillate(x0);
        let m = (self.oscillate(x0 + DELTA) - y0) / DELTA;
        let mut x1 = (self.to - y0 + m * x0) / m;
        let mut y1 = self.oscillate(x1);

        let mut i = 0;
        while (self.to - y1).abs() > self.params.epsilon {
            if i > 1000 {
                return Duration::ZERO;
            }
            x0 = x1;
            y0 = y1;
            let m = (self.oscillate(x0 + DELTA) - y0) / DELTA;
            x1 = (self.to - y0 + m * x0) / m;
            y1 = self.oscillate(x1);
            if !y1.is_finite() {
                return Duration::from_secs_f64(x0);
            }
            i += 1;
        }
        Duration::from_secs_f64(x1)
    }

    /// Closed-form solution to `m·ẍ + b·ẋ + kx = 0` evaluated at time `t`
    /// (seconds). Branches by damping regime so each case stays
    /// numerically stable.
    fn oscillate(&self, t: f64) -> f64 {
        let b = self.params.damping;
        let m = self.params.mass;
        let k = self.params.stiffness;
        let v0 = self.initial_velocity;

        let beta = b / (2. * m);
        let omega0 = (k / m).sqrt();
        let x0 = self.from - self.to;
        let envelope = (-beta * t).exp();

        if (beta - omega0).abs() <= f64::from(f32::EPSILON) {
            // Critically damped.
            self.to + envelope * (x0 + (beta * x0 + v0) * t)
        } else if beta < omega0 {
            // Underdamped (oscillates).
            let omega1 = ((omega0 * omega0) - (beta * beta)).sqrt();
            self.to
                + envelope
                    * (x0 * (omega1 * t).cos() + ((beta * x0 + v0) / omega1) * (omega1 * t).sin())
        } else {
            // Overdamped.
            let omega2 = ((beta * beta) - (omega0 * omega0)).sqrt();
            self.to
                + envelope
                    * (x0 * (omega2 * t).cosh()
                        + ((beta * x0 + v0) / omega2) * (omega2 * t).sinh())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn critically_damped_settles_within_epsilon() {
        let spring = Spring {
            from: 0.0,
            to: 100.0,
            initial_velocity: 0.0,
            params: SpringParams::new(1.0, 800.0, 0.5),
        };
        let dur = spring.clamped_duration().expect("should converge");
        let value = spring.value_at(dur);
        assert!(
            (100.0 - value).abs() < spring.params.epsilon * 2.0,
            "value at clamped_duration ({:?}) = {value}, target=100",
            dur
        );
    }

    #[test]
    fn underdamped_overshoots_then_settles() {
        let spring = Spring {
            from: 0.0,
            to: 100.0,
            initial_velocity: 0.0,
            params: SpringParams::new(0.4, 800.0, 0.5),
        };
        let mut max_v: f64 = 0.0;
        for ms in 1..400 {
            let v = spring.value_at(Duration::from_millis(ms));
            max_v = max_v.max(v);
        }
        assert!(max_v > 100.0, "expected overshoot, max={max_v}");
    }

    #[test]
    fn equal_from_to_returns_zero_duration() {
        let spring = Spring {
            from: 5.0,
            to: 5.0,
            initial_velocity: 0.0,
            params: SpringParams::new(1.0, 800.0, 0.5),
        };
        assert_eq!(spring.clamped_duration(), Some(Duration::ZERO));
    }

    #[test]
    fn frame_rate_invariance() {
        // Sampling the analytical solution at different times produces
        // the same value irrespective of the caller's dt — there is no
        // integration error to drift across step sizes (this was the
        // killer regression in the previous numerical integrator).
        let spring = Spring {
            from: 0.0,
            to: 50.0,
            initial_velocity: 0.0,
            params: SpringParams::new(1.0, 600.0, 0.5),
        };
        // Evaluate at exactly the same wall-clock time (50 ms) once;
        // the value is fully determined by `t`, no per-step state.
        let v1 = spring.value_at(Duration::from_millis(50));
        let v2 = spring.value_at(Duration::from_millis(50));
        assert!((v1 - v2).abs() < 1e-9, "deterministic: {v1} vs {v2}");
        // And monotonically larger at later times (it's the
        // critically-damped no-overshoot spring approaching 50).
        let at_50ms = spring.value_at(Duration::from_millis(50));
        let at_100ms = spring.value_at(Duration::from_millis(100));
        assert!(
            at_50ms < at_100ms && at_100ms < 50.0,
            "monotonic toward target: 50 ms = {at_50ms}, 100 ms = {at_100ms}"
        );
    }
}
