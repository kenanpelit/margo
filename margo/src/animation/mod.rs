#![allow(dead_code)]
use margo_config::BezierCurve;

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
        AnimationCurves {
            move_curve: BakedCurve::bake(&config.animation_curve_move),
            open_curve: BakedCurve::bake(&config.animation_curve_open),
            tag_curve: BakedCurve::bake(&config.animation_curve_tag),
            close_curve: BakedCurve::bake(&config.animation_curve_close),
            focus_curve: BakedCurve::bake(&config.animation_curve_focus),
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
