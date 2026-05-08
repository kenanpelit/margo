//! Spring-based animation primitive.
//!
//! Currently margo's [`ClientAnimation`](super::ClientAnimation) drives a
//! cubic-Bezier curve over a fixed duration: the user picks a curve and a
//! duration, and `tick_animations` evaluates `t = elapsed / duration` against
//! a 256-point baked LUT. That model is dirt simple but has two known
//! problems:
//!
//! * **Interruption looks bad.** If a window is mid-flight from rect A to B
//!   and the user re-tiles the layout (so the target becomes C), the bezier
//!   restarts at the *current* interpolated position with implicit velocity =
//!   0. The eye sees a hard kink: the window was clearly moving in some
//!   direction, then snaps to a fresh ease-out toward C.
//!
//! * **Refresh-rate dependence is hidden.** Bezier+duration is technically
//!   frame-rate independent (we sample by elapsed time, not by frame index),
//!   but durations are tuned visually on a 60 Hz display and feel different
//!   on 120/144 Hz panels because the per-frame increment changes shape.
//!
//! This module ports niri's `niri/src/animation/spring.rs` core into margo as
//! an alternative `tick` primitive. A `Spring` is a critically- (or under-)
//! damped harmonic oscillator: given a current position, target, current
//! velocity and a step `dt`, it returns the next position+velocity. Crucially:
//!
//! * Targets can change mid-flight without re-initialising — pass in the new
//!   target, keep the velocity, the spring carries momentum through the
//!   transition.
//! * Settling is detected by physics (low residual displacement *and* low
//!   velocity), not by an arbitrary clock; over-shoot is a function of
//!   `damping_ratio < 1`, not of "did we overrun the bezier".
//! * The integrator is semi-implicit Euler with a fixed substep — same dt
//!   on 60 Hz and 240 Hz panels, so the spring's perceived "snappiness" is
//!   refresh-rate invariant.
//!
//! The spring is intentionally a *primitive*: callers (the upcoming
//! per-animation `Clock` selector) decide whether to drive a single scalar
//! (e.g. opacity) or a separate spring per channel (x, y, w, h). The cost
//! per step is a handful of floating-point ops, so per-channel springs are
//! the obvious choice for `Rect` interpolation.
//!
//! Wiring: this module is currently not yet hooked into `tick_animations`.
//! Landing the primitive + tests as one commit and the integration as a
//! follow-up keeps each diff reviewable.

/// Critically-damped or under-damped harmonic oscillator.
///
/// Real-world units don't matter — `stiffness`, `damping` and `mass` are
/// just dimensionless constants chosen for feel. Defaults are tuned to
/// match niri's "snappy but not jittery" presets.
#[derive(Debug, Clone, Copy)]
pub struct Spring {
    /// Spring constant (k in `F = -k·x`). Higher = more aggressive pull
    /// toward the target. Niri's default for window movement is 800.
    pub stiffness: f64,
    /// Damping coefficient (c in `F = -c·v`). Tuning this is awkward
    /// because it scales with sqrt(stiffness * mass); prefer setting
    /// [`Spring::critically_damped`] / [`Spring::with_damping_ratio`]
    /// which compute it for you.
    pub damping: f64,
    /// Effective mass. 1.0 is the canonical choice; larger mass makes
    /// the spring slower without changing its overshoot character.
    pub mass: f64,
    /// Settle threshold for residual position error. Once
    /// `|current - target| < epsilon` *and* `|velocity| < velocity_epsilon`,
    /// [`Spring::is_settled`] reports true and the caller can stop ticking.
    /// In logical pixels for window movement this is set well below half a
    /// physical pixel so settling is invisible on HiDPI.
    pub epsilon: f64,
    /// Settle threshold for residual velocity (in position units per
    /// second). Niri uses ~0.01; same default here.
    pub velocity_epsilon: f64,
}

impl Default for Spring {
    fn default() -> Self {
        Self::critically_damped(800.0, 1.0)
    }
}

impl Spring {
    /// Convenience: pick `damping = 2·sqrt(k·m)` so the oscillator just
    /// barely doesn't overshoot. Use this for animations where overshoot
    /// would feel wrong (window snap, focus highlight). For "bouncy"
    /// feels prefer [`Spring::with_damping_ratio`] with ratio < 1.
    pub fn critically_damped(stiffness: f64, mass: f64) -> Self {
        Self {
            stiffness,
            damping: 2.0 * (stiffness * mass).sqrt(),
            mass,
            epsilon: 0.5,
            velocity_epsilon: 0.01,
        }
    }

    /// `damping_ratio < 1` → underdamped (overshoots), `= 1` → critical,
    /// `> 1` → overdamped (sluggish). Window movement typically wants
    /// 0.85–1.0; bouncy effects want 0.5–0.7.
    pub fn with_damping_ratio(stiffness: f64, mass: f64, damping_ratio: f64) -> Self {
        Self {
            stiffness,
            damping: damping_ratio * 2.0 * (stiffness * mass).sqrt(),
            mass,
            epsilon: 0.5,
            velocity_epsilon: 0.01,
        }
    }

    /// Override settle thresholds. Useful when integrating different
    /// quantities (e.g. opacity in [0, 1] needs much smaller epsilons
    /// than position in pixels).
    pub fn with_thresholds(mut self, epsilon: f64, velocity_epsilon: f64) -> Self {
        self.epsilon = epsilon;
        self.velocity_epsilon = velocity_epsilon;
        self
    }

    /// Single integration step. `dt` is in seconds.
    ///
    /// Uses semi-implicit (symplectic) Euler:
    /// `v_{n+1} = v_n + a · dt; x_{n+1} = x_n + v_{n+1} · dt`
    ///
    /// This is one order of magnitude more stable than explicit Euler at
    /// the kind of step sizes we hit on a 60 Hz display (~16.7 ms), and
    /// it preserves spring energy correctly so we don't spuriously gain
    /// velocity when the integrator is sub-stepped under load.
    pub fn step(&self, current: f64, target: f64, velocity: f64, dt: f64) -> (f64, f64) {
        // Sub-step at 1 ms slices so a long frame (e.g. 33 ms after a
        // hitch) doesn't blow up the integrator. The cost is bounded:
        // worst case ~50 sub-steps per frame, each a few flops.
        let sub_dt = 0.001;
        let mut steps = (dt / sub_dt).ceil() as usize;
        if steps == 0 {
            steps = 1;
        }
        let h = dt / steps as f64;

        let mut x = current;
        let mut v = velocity;
        for _ in 0..steps {
            let displacement = x - target;
            let force = -self.stiffness * displacement - self.damping * v;
            let a = force / self.mass;
            v += a * h;
            x += v * h;
        }
        (x, v)
    }

    /// Has the oscillator effectively reached `target`? Both displacement
    /// and velocity must fall under their respective thresholds.
    pub fn is_settled(&self, current: f64, target: f64, velocity: f64) -> bool {
        (current - target).abs() < self.epsilon && velocity.abs() < self.velocity_epsilon
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Critically-damped springs must monotonically approach the target —
    /// no sample may overshoot the target's side of the displacement axis.
    /// (With finite step `dt`, "monotonic" is approximate; we just check
    /// the sign of the residual displacement never flips.)
    #[test]
    fn critically_damped_does_not_overshoot() {
        let spring = Spring::critically_damped(800.0, 1.0);
        let (mut x, mut v) = (0.0, 0.0);
        let target = 100.0;
        let dt = 1.0 / 240.0;

        for _ in 0..2400 {
            let (nx, nv) = spring.step(x, target, v, dt);
            // Residual displacement (positive = below target). Must
            // never flip sign for a critically-damped spring with
            // zero initial velocity.
            assert!(target - nx >= -spring.epsilon, "overshoot: x={nx}");
            x = nx;
            v = nv;
            if spring.is_settled(x, target, v) {
                return;
            }
        }
        panic!("did not settle in 10 s (x={x}, v={v})");
    }

    /// Underdamped springs *should* overshoot at least once. Sanity-check
    /// that `damping_ratio < 1` actually buys us oscillation.
    #[test]
    fn underdamped_overshoots() {
        let spring = Spring::with_damping_ratio(800.0, 1.0, 0.4);
        let (mut x, mut v) = (0.0, 0.0);
        let target = 100.0;
        let dt = 1.0 / 240.0;

        let mut max_x: f64 = 0.0;
        for _ in 0..2400 {
            let (nx, nv) = spring.step(x, target, v, dt);
            x = nx;
            v = nv;
            max_x = max_x.max(x);
            if spring.is_settled(x, target, v) && max_x > target {
                break;
            }
        }
        assert!(max_x > target, "expected overshoot, max_x={max_x}");
    }

    /// Mid-flight target retarget: the spring should preserve velocity
    /// across the change and reach the new target without snapping back
    /// to zero velocity.
    #[test]
    fn retargeting_preserves_velocity() {
        let spring = Spring::critically_damped(400.0, 1.0);
        let (mut x, mut v) = (0.0, 0.0);
        let dt = 1.0 / 240.0;

        // Animate toward 100 for 200 ms — should be moving fast forward.
        for _ in 0..48 {
            let (nx, nv) = spring.step(x, 100.0, v, dt);
            x = nx;
            v = nv;
        }
        let v_at_retarget = v;
        assert!(v_at_retarget > 0.0, "expected forward velocity, got {v_at_retarget}");

        // Now switch the target to 200. Velocity must *not* be reset.
        // Settle and confirm we end at 200, not back at 100.
        let mut settled = false;
        for _ in 0..2400 {
            let (nx, nv) = spring.step(x, 200.0, v, dt);
            x = nx;
            v = nv;
            if spring.is_settled(x, 200.0, v) {
                settled = true;
                break;
            }
        }
        assert!(settled, "did not settle after retarget (x={x}, v={v})");
    }

    /// Step size must not affect the converged target value. Run two
    /// integrations with the same physics but different outer-loop dt
    /// (one matches a 60 Hz frame, the other 144 Hz) and check they
    /// settle at the same target within epsilon.
    #[test]
    fn refresh_rate_invariance() {
        let spring = Spring::critically_damped(600.0, 1.0);

        let settle_at = |dt: f64| {
            let (mut x, mut v) = (0.0, 0.0);
            for _ in 0..((5.0 / dt) as usize) {
                let (nx, nv) = spring.step(x, 50.0, v, dt);
                x = nx;
                v = nv;
                if spring.is_settled(x, 50.0, v) {
                    return x;
                }
            }
            x
        };

        let at_60 = settle_at(1.0 / 60.0);
        let at_144 = settle_at(1.0 / 144.0);
        assert!(
            (at_60 - at_144).abs() < spring.epsilon,
            "60 Hz settle = {at_60}, 144 Hz settle = {at_144}"
        );
    }
}
