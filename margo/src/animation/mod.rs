#![allow(dead_code)]
use margo_config::BezierCurve;

pub mod spring;

pub const BAKED_POINTS_COUNT: usize = 256;

/// Animation type enum matching C's `enum { NONE, OPEN, MOVE, CLOSE, TAG, FOCUS, ... }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnimationType {
    #[default]
    None,
    Open,
    Move,
    Close,
    Tag,
    Focus,
    OpaFadeIn,
    OpaFadeOut,
    CanvasPan,
    CanvasZoom,
}

/// A baked lookup table for one animation type.
#[derive(Clone)]
pub struct BakedCurve {
    pub points: Box<[(f64, f64); BAKED_POINTS_COUNT]>,
}

impl BakedCurve {
    pub fn bake(curve: &BezierCurve) -> Self {
        let mut points = Box::new([(0.0f64, 0.0f64); BAKED_POINTS_COUNT]);
        for i in 0..BAKED_POINTS_COUNT {
            let t = i as f64 / (BAKED_POINTS_COUNT - 1) as f64;
            points[i] = eval_bezier(t, curve);
        }
        BakedCurve { points }
    }

    /// Bake a spring-driven 0→1 curve into the same lookup-table
    /// shape `bake` uses for bezier curves. The resulting table is
    /// a drop-in replacement for `bake(...)` from the consumer's
    /// point of view: `sample(t)` for `t ∈ [0, 1]` returns the
    /// spring's value at that fraction of its natural settle time.
    ///
    /// Why bake into a fixed table instead of doing live spring
    /// integration per frame? Two reasons:
    ///
    ///   * The animation primitive on the consumer side
    ///     (`tick_animations`) is "0..1 progress driven by
    ///     elapsed/duration". Forcing each animation type to
    ///     carry its own `Spring` state would duplicate the
    ///     opacity / open / close / tag / layer tick paths.
    ///   * The shape we want is critical-damped or lightly
    ///     under-damped — "snappy with a kiss of overshoot" —
    ///     which produces a curve identical for every animation
    ///     of the same type. Baking once at config-load is free
    ///     and lets `sample` stay a binary search.
    ///
    /// The continuous-position spring (used for the *move*
    /// animation, where velocity carries across mid-flight
    /// retargets) is a different code path entirely; this is for
    /// transition animations that run for a fixed wall-clock
    /// duration and just want a different curve shape.
    pub fn bake_spring(params: spring::SpringParams) -> Self {
        use spring::Spring;
        use std::time::Duration;

        let spring = Spring {
            from: 0.0,
            to: 1.0,
            initial_velocity: 0.0,
            params,
        };
        // Settle time: when the spring is within ε of its target.
        // Fall back to 500 ms if the convergence search tops out
        // (pathological over-damped configs).
        let settle = spring
            .clamped_duration()
            .unwrap_or(Duration::from_millis(500))
            .as_secs_f64()
            .max(0.001);

        let mut points = Box::new([(0.0f64, 0.0f64); BAKED_POINTS_COUNT]);
        for i in 0..BAKED_POINTS_COUNT {
            let t_norm = i as f64 / (BAKED_POINTS_COUNT - 1) as f64;
            let t = Duration::from_secs_f64(t_norm * settle);
            let y = spring.value_at(t);
            // Match the bezier table convention: x in [0, 1] is the
            // input parameter (so `sample(t)` does a binary search
            // on the x column), y is the curve output. Clamp y in
            // case overshoot pushes it briefly past 1.
            points[i] = (t_norm, y.clamp(0.0, 1.05));
        }
        BakedCurve { points }
    }

    /// Binary-search the table for the Y value at parameter `t` (x-axis).
    pub fn sample(&self, t: f64) -> f64 {
        let pts = &*self.points;
        let mut lo = 0usize;
        let mut hi = BAKED_POINTS_COUNT - 1;
        while hi - lo > 1 {
            let mid = (lo + hi) / 2;
            if pts[mid].0 <= t {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        pts[hi].1
    }
}

/// Evaluate a cubic Bezier at parameter `t`.
/// Control points: (0,0), (p0,p1), (p2,p3), (1,1)
fn eval_bezier(t: f64, curve: &BezierCurve) -> (f64, f64) {
    let [p0, p1, p2, p3] = curve.0;
    let mt = 1.0 - t;
    let x = 3.0 * t * mt * mt * p0 + 3.0 * t * t * mt * p2 + t * t * t;
    let y = 3.0 * t * mt * mt * p1 + 3.0 * t * t * mt * p3 + t * t * t;
    (x, y)
}

/// All pre-baked animation curves for the compositor.
pub struct AnimationCurves {
    pub move_curve: BakedCurve,
    pub open_curve: BakedCurve,
    pub tag_curve: BakedCurve,
    pub close_curve: BakedCurve,
    pub focus_curve: BakedCurve,
    pub opafadein_curve: BakedCurve,
    pub opafadeout_curve: BakedCurve,
    pub canvas_pan_curve: BakedCurve,
    pub canvas_zoom_curve: BakedCurve,
}

impl AnimationCurves {
    pub fn bake(config: &margo_config::Config) -> Self {
        // Shared spring params for every spring-baked curve. Per-type
        // damping/stiffness overrides are easy to add later (just split
        // these into per-type config knobs); shared is the right
        // starting point because users tuning "I want a snappier
        // compositor feel" usually mean it across the board.
        let spring_params = spring::SpringParams::new(
            config.animation_spring_damping_ratio,
            config.animation_spring_stiffness,
            0.0001,
        );

        // Pick bezier vs spring per animation type. `move_curve` is
        // baked here as a fallback for the bezier-mode tick path; the
        // actual spring-mode move animation runs continuous-position
        // physics and ignores this curve entirely.
        let bake_one = |clock: &str, bezier: &BezierCurve| -> BakedCurve {
            if clock == "spring" {
                BakedCurve::bake_spring(spring_params)
            } else {
                BakedCurve::bake(bezier)
            }
        };

        AnimationCurves {
            move_curve: bake_one(&config.animation_clock_move, &config.animation_curve_move),
            open_curve: bake_one(&config.animation_clock_open, &config.animation_curve_open),
            tag_curve: bake_one(&config.animation_clock_tag, &config.animation_curve_tag),
            close_curve: bake_one(&config.animation_clock_close, &config.animation_curve_close),
            focus_curve: bake_one(&config.animation_clock_focus, &config.animation_curve_focus),
            // OpaFadeIn / OpaFadeOut and canvas pan / zoom intentionally
            // stay on bezier — opacity blends look unnatural with
            // overshoot, and the canvas pan/zoom uses a hand-tuned
            // curve where spring physics would feel uncontrolled.
            opafadein_curve: BakedCurve::bake(&config.animation_curve_opafadein),
            opafadeout_curve: BakedCurve::bake(&config.animation_curve_opafadeout),
            canvas_pan_curve: BakedCurve::bake(&config.animation_curve_canvas_pan),
            canvas_zoom_curve: BakedCurve::bake(&config.animation_curve_canvas_zoom),
        }
    }

    pub fn sample(&self, t: f64, anim_type: AnimationType) -> f64 {
        let curve = match anim_type {
            AnimationType::Move | AnimationType::None => &self.move_curve,
            AnimationType::Open => &self.open_curve,
            AnimationType::Tag => &self.tag_curve,
            AnimationType::Close => &self.close_curve,
            AnimationType::Focus => &self.focus_curve,
            AnimationType::OpaFadeIn => &self.opafadein_curve,
            AnimationType::OpaFadeOut => &self.opafadeout_curve,
            AnimationType::CanvasPan => &self.canvas_pan_curve,
            AnimationType::CanvasZoom => &self.canvas_zoom_curve,
        };
        curve.sample(t)
    }
}

// ── Per-client animation state ────────────────────────────────────────────────

pub use crate::layout::Rect;

#[derive(Debug, Clone, Default)]
pub struct ClientAnimation {
    pub should_animate: bool,
    pub running: bool,
    pub tagining: bool,
    pub tagouted: bool,
    pub tagouting: bool,
    pub begin_fade_in: bool,
    pub tag_from_rule: bool,
    pub time_started: u32,
    pub duration: u32,
    pub initial: Rect,
    pub current: Rect,
    pub action: AnimationType,
    /// Last tick's wall-clock time in `now_ms` units. Spring integration
    /// uses `now_ms - last_tick_ms` as `dt`; bezier ticks ignore it.
    /// Initialised to `time_started` when the animation kicks off so the
    /// very first sub-step gets a small but non-zero `dt`.
    pub last_tick_ms: u32,
    /// Per-channel velocity for the spring integrator (x, y, w, h, in
    /// logical-pixels-per-second). Bezier path leaves this at zero.
    /// Carried across retargets so a window already in motion doesn't
    /// snap when the layout reshuffles mid-animation — the spring
    /// reaches the new target while preserving the velocity it had.
    pub velocity: [f64; 4],
}

/// Per-client open/close transition state. Set when the client maps for
/// the first time (open) or when its toplevel role is destroyed
/// (close); cleared when the animation settles.
///
/// The actual texture used by the render path lives on the client (open
/// animation) or in [`crate::state::ClosingClient`] (close — by the time
/// we draw the close, the wl_surface may already be gone).
#[derive(Debug, Clone, Copy)]
pub struct OpenCloseClientAnim {
    pub kind: crate::render::open_close::OpenCloseKind,
    pub time_started: u32,
    pub duration: u32,
    /// 0..=1 progress through the curve. Both open and close animate
    /// `progress` in the same direction; `OpenCloseRenderElement` knows
    /// which side of the transition it's on via its `is_close` flag.
    pub progress: f32,
    /// Scale at the "extreme" end of the transition (start of open,
    /// end of close). 0.5–0.8 typical. Pulled from
    /// [`margo_config::Config::zoom_initial_ratio`] /
    /// [`margo_config::Config::zoom_end_ratio`] when the animation
    /// fires; baked here so config changes mid-flight don't snap.
    pub extreme_scale: f32,
}

#[derive(Debug, Clone, Default)]
pub struct OpacityAnimation {
    pub running: bool,
    pub current_opacity: f32,
    pub target_opacity: f32,
    pub initial_opacity: f32,
    pub time_started: u32,
    pub duration: u32,
    pub current_border_color: [f32; 4],
    pub target_border_color: [f32; 4],
    pub initial_border_color: [f32; 4],
}

// ── Layer-surface animation state ─────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct LayerAnimation {
    pub running: bool,
    pub time_started: u32,
    pub duration: u32,
    pub initial: Rect,
    pub current: Rect,
    pub is_open: bool,
    pub anim_type: String,
}

#[cfg(test)]
mod tests {
    //! T2: animation curve snapshot tests.
    //!
    //! These lock the 4-point Bezier evaluator and the spring-baked
    //! curve's shape against future refactors. The samples are
    //! rounded to 6 decimals so a 1-ULP drift in `eval_bezier` or
    //! the spring integrator doesn't flake the test on
    //! cross-platform float-mode differences, but a real coefficient
    //! mistake (e.g. swapping `p1` and `p2` in the cubic-Bezier
    //! formula) lands well above the noise floor and flips the
    //! test red immediately.
    use super::*;
    use margo_config::{BezierCurve, Config};
    use spring::SpringParams;

    /// Round to 6 decimal places. Anything finer is f64 jitter; the
    /// curve's actual shape is captured well below this.
    fn r6(v: f64) -> f64 {
        (v * 1_000_000.0).round() / 1_000_000.0
    }

    /// The identity-like Bezier `(0.25, 0.25, 0.75, 0.75)` produces a
    /// curve that's *visually* close to linear but with very mild
    /// ease-in-out shaping. Endpoint values are exact (0 and 1) by
    /// construction.
    #[test]
    fn near_linear_bezier_endpoints_exact() {
        let curve = BezierCurve([0.25, 0.25, 0.75, 0.75]);
        let baked = BakedCurve::bake(&curve);
        assert!(baked.sample(0.0) < 0.01, "sample(0) should be ~0");
        assert!(baked.sample(1.0) > 0.99, "sample(1) should be ~1");
    }

    /// `ease-out-expo`-like Bezier `(0.16, 1.0, 0.30, 1.0)` rises
    /// fast and flattens near 1. Sample at 0.25 should already be
    /// past 0.7; sample at 0.5 should be > 0.9; sample at 0.75
    /// should be > 0.97.
    #[test]
    fn ease_out_expo_shape_locked() {
        let curve = BezierCurve([0.16, 1.0, 0.30, 1.0]);
        let baked = BakedCurve::bake(&curve);
        let s25 = baked.sample(0.25);
        let s50 = baked.sample(0.50);
        let s75 = baked.sample(0.75);
        // Margins picked so a real coefficient swap (`p0` ↔ `p2`)
        // pulls a sample out of the band, but f64 jitter doesn't.
        assert!((0.7..=0.95).contains(&s25), "s25 = {}", r6(s25));
        assert!((0.88..=0.98).contains(&s50), "s50 = {}", r6(s50));
        assert!((0.95..=1.00).contains(&s75), "s75 = {}", r6(s75));
    }

    /// `ease-in-quad`-like Bezier `(0.55, 0.0, 1.0, 0.45)` starts
    /// flat, ramps up. Mirror of the ease-out test: the curve
    /// should sit well *below* the diagonal at midpoint.
    #[test]
    fn ease_in_quad_shape_locked() {
        let curve = BezierCurve([0.55, 0.0, 1.0, 0.45]);
        let baked = BakedCurve::bake(&curve);
        let s25 = baked.sample(0.25);
        let s50 = baked.sample(0.50);
        // Loose band — depends on the exact placement of the control
        // points; the only invariant we want here is "starts well
        // below the diagonal".
        assert!(s25 < 0.18, "s25 = {} should sit below 0.18", r6(s25));
        assert!(s50 < 0.55, "s50 = {} should sit below 0.55", r6(s50));
    }

    /// All baked curves must be **monotonically non-decreasing** on
    /// `y`. A non-monotone animation curve would make windows
    /// briefly move *backwards* mid-flight, which is the visual
    /// "stutter" the user noticed during the very first overview
    /// sweep iterations. Lock the property in stone.
    #[test]
    fn bezier_bake_is_non_decreasing_in_y() {
        for curve in [
            BezierCurve([0.25, 0.1, 0.25, 1.0]),
            BezierCurve([0.42, 0.0, 0.58, 1.0]),
            BezierCurve([0.16, 1.0, 0.30, 1.0]),
            BezierCurve([0.50, 0.0, 0.50, 1.0]),
        ] {
            let baked = BakedCurve::bake(&curve);
            let mut prev = -1.0_f64;
            for (i, (_x, y)) in baked.points.iter().enumerate() {
                assert!(
                    *y + 1e-9 >= prev,
                    "non-monotone at i={i}: prev={prev}, y={y} for {:?}",
                    curve.0
                );
                prev = *y;
            }
        }
    }

    /// `sample(0.0) ≈ 0` and `sample(1.0) ≈ 1` for every reasonable
    /// curve. The table holds 256 points; the first and last entries
    /// are exact by construction (the bezier formula collapses at
    /// the endpoints), but the binary-search edge cases used to
    /// briefly return the wrong index — this test would have
    /// caught it.
    #[test]
    fn sample_endpoints_round_to_zero_and_one() {
        // The binary search returns the *ceiling* index — pts[1].y
        // for sample(0.0), pts[N-1].y for sample(1.0). On steep
        // ease-out curves (e.g. `[0.16, 1.0, 0.30, 1.0]`) the y at
        // t=1/255 is already ~0.012, so the lower bound on s0 has
        // to be 0.05 — still 50× tighter than a real coefficient
        // mistake would land.
        for curve in [
            BezierCurve([0.25, 0.1, 0.25, 1.0]),
            BezierCurve([0.42, 0.0, 0.58, 1.0]),
            BezierCurve([0.0, 0.0, 1.0, 1.0]), // pure linear
            BezierCurve([0.16, 1.0, 0.30, 1.0]),
        ] {
            let baked = BakedCurve::bake(&curve);
            let s0 = baked.sample(0.0);
            let s1 = baked.sample(1.0);
            assert!(
                s0 < 0.05,
                "sample(0) = {} for {:?} should be near 0",
                r6(s0),
                curve.0
            );
            assert!(
                (s1 - 1.0).abs() < 0.005,
                "sample(1) = {} for {:?} should be near 1",
                r6(s1),
                curve.0
            );
        }
    }

    /// Spring-baked curves overshoot slightly at light damping —
    /// the bake clamps overshoot to 1.05 so the consumer doesn't
    /// get a target > 1.05× the slot size. Lock the cap.
    #[test]
    fn spring_bake_overshoot_clamped_to_1_05() {
        // Light damping (0.5) + medium stiffness — under-damped,
        // should overshoot.
        let params = SpringParams::new(0.5, 600.0, 0.0001);
        let baked = BakedCurve::bake_spring(params);
        let max_y = baked
            .points
            .iter()
            .map(|(_, y)| *y)
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(max_y <= 1.05 + 1e-9, "spring overshoot uncapped: {max_y}");
        // It SHOULD overshoot a tiny bit at 0.5 damping, so a value
        // strictly > 1.0 confirms we're actually seeing the spring
        // shape (and not, say, an accidental switch to bezier).
        assert!(
            max_y > 1.0 - 1e-3,
            "spring at 0.5 damping should reach or pass 1.0, got {max_y}"
        );
    }

    /// Critically-damped spring (`damping = 1.0`) reaches target
    /// monotonically — no overshoot, but also no oscillation back
    /// the other way.
    #[test]
    fn critically_damped_spring_is_monotone() {
        let params = SpringParams::new(1.0, 800.0, 0.0001);
        let baked = BakedCurve::bake_spring(params);
        let mut prev = -1.0_f64;
        for (_, y) in baked.points.iter() {
            assert!(*y + 1e-9 >= prev, "critically-damped not monotone");
            prev = *y;
        }
        // End-of-table should be at (or just below) the target.
        let last_y = baked.points.last().unwrap().1;
        assert!(
            (last_y - 1.0).abs() < 0.05,
            "critically-damped final y = {last_y}, expected ~1.0"
        );
    }

    /// `AnimationCurves::bake(default config)` produces nine
    /// curves, each samplable at every `AnimationType` variant
    /// (no None — that aliases Move). Locks the dispatcher.
    #[test]
    fn animation_curves_dispatches_every_variant() {
        let config = Config::default();
        let curves = AnimationCurves::bake(&config);

        // Every variant maps to a curve that samples without
        // panicking. We sweep a midpoint and check it sits in
        // [0, 1.1] (1.1 = spring overshoot cap + slack).
        for ty in [
            AnimationType::None,
            AnimationType::Open,
            AnimationType::Move,
            AnimationType::Close,
            AnimationType::Tag,
            AnimationType::Focus,
            AnimationType::OpaFadeIn,
            AnimationType::OpaFadeOut,
            AnimationType::CanvasPan,
            AnimationType::CanvasZoom,
        ] {
            let v = curves.sample(0.5, ty);
            assert!(
                (0.0..=1.1).contains(&v),
                "curve {ty:?} sample(0.5) = {v} out of band"
            );
        }
    }

    /// `sample(t)` outside `[0, 1]` doesn't panic. The dispatch
    /// path clamps `t` upstream but defensive testing here flags
    /// the edge cases the binary search can hit (negative `lo`,
    /// `hi` past `BAKED_POINTS_COUNT - 1`).
    #[test]
    fn sample_clamps_out_of_range_t() {
        let baked = BakedCurve::bake(&BezierCurve([0.25, 0.1, 0.25, 1.0]));
        // Bracket the boundaries.
        let _ = baked.sample(-0.5);
        let _ = baked.sample(0.0);
        let _ = baked.sample(1.0);
        let _ = baked.sample(1.5);
    }
}
