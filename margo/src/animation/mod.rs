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
