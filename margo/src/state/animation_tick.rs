//! Per-frame animation tick.
//!
//! Extracted from `state.rs` (W4.2 follow-up, roadmap Q1). Pure
//! function over `&mut [MargoClient]` + a couple of auxiliary
//! collections — no `MargoState` coupling — so it lifts cleanly
//! into its own translation unit. The compositor's main event-loop
//! tick (`main.rs`) calls this once per repaint via the standard
//! `crate::state::tick_animations` path.
//!
//! Five categories of animation are advanced in a single pass:
//!
//!   1. **Opacity crossfade** on every client (focus highlight
//!      colour + alpha).
//!   2. **Opening animation** (zoom + fade-in of a fresh toplevel).
//!   3. **Layer-surface open/close** (slide of bar / launcher).
//!   4. **Closing client** (close-time texture fade-out).
//!   5. **Move/resize animation** — bezier OR spring physics
//!      depending on `AnimTickSpec::use_spring`.

use super::{ClosingClient, LayerSurfaceAnim, MargoClient};
use crate::animation::{AnimationCurves, AnimationType};

/// Per-call parameters for [`tick_animations`]. Bundles the
/// move-animation duration (used for both bezier ticks and resize-
/// snapshot expiry) with the spring physics configuration, so the
/// call site doesn't have to thread four scalars individually.
#[derive(Debug, Clone, Copy)]
pub struct AnimTickSpec {
    /// Total bezier duration in `now_ms` units. Also bounds resize
    /// snapshot life-time regardless of which clock is in use.
    pub duration_move: u32,
    /// `true` → spring physics integrator drives the move animation;
    /// `false` → original bezier sampling.
    pub use_spring: bool,
    /// Pre-built spring (stiffness/damping/mass already resolved
    /// from the damping ratio). Ignored when `use_spring` is false.
    pub spring: crate::animation::spring::Spring,
}

pub fn tick_animations(
    clients: &mut [MargoClient],
    curves: &AnimationCurves,
    now_ms: u32,
    spec: AnimTickSpec,
    closing_clients: &mut Vec<ClosingClient>,
    layer_animations: &mut std::collections::HashMap<
        smithay::reexports::wayland_server::backend::ObjectId,
        LayerSurfaceAnim,
    >,
) -> bool {
    let _span = tracy_client::span!("tick_animations");
    let mut changed = false;
    // Advance focus highlight (border colour + opacity) crossfades.
    // `OpacityAnimation` does double duty: focused_opacity ↔ unfocused_opacity
    // for the alpha, focuscolor ↔ bordercolor for the border. Both
    // sample the `Focus` curve. Border refresh reads the current
    // colour from this struct on every refresh so the cross-fade
    // shows even between renders.
    for c in clients.iter_mut() {
        let oa = &mut c.opacity_animation;
        if !oa.running {
            continue;
        }
        let elapsed = now_ms.wrapping_sub(oa.time_started);
        if elapsed >= oa.duration {
            oa.running = false;
            oa.current_opacity = oa.target_opacity;
            oa.current_border_color = oa.target_border_color;
            changed = true;
            continue;
        }
        let t = elapsed as f64 / oa.duration as f64;
        let s = curves.sample(t, AnimationType::Focus) as f32;
        oa.current_opacity = oa.initial_opacity + (oa.target_opacity - oa.initial_opacity) * s;
        for i in 0..4 {
            let a = oa.initial_border_color[i];
            let b = oa.target_border_color[i];
            oa.current_border_color[i] = a + (b - a) * s;
        }
        changed = true;
    }

    // Advance opening animations on each client. Settles drop both
    // the animation state and the captured texture so the live
    // wl_surface takes over on the next frame.
    for c in clients.iter_mut() {
        if let Some(anim) = c.opening_animation.as_mut() {
            let elapsed = now_ms.wrapping_sub(anim.time_started);
            if elapsed >= anim.duration {
                c.opening_animation = None;
                c.opening_texture = None;
                c.opening_capture_pending = false;
                changed = true;
            } else {
                let raw = elapsed as f64 / anim.duration as f64;
                anim.progress = curves.sample(raw, AnimationType::Open) as f32;
                changed = true;
            }
        }
    }

    // Advance layer-surface open/close animations. Settled entries
    // are removed from the map; the open path then falls back to
    // unmodulated layer rendering, the close path stops drawing the
    // texture (the underlying smithay layer was already unmapped at
    // `layer_destroyed` time).
    {
        let mut to_drop: Vec<smithay::reexports::wayland_server::backend::ObjectId> = Vec::new();
        for (id, anim) in layer_animations.iter_mut() {
            let elapsed = now_ms.wrapping_sub(anim.time_started);
            if elapsed >= anim.duration {
                to_drop.push(id.clone());
                continue;
            }
            let raw = elapsed as f64 / anim.duration as f64;
            let action = if anim.is_close {
                AnimationType::Close
            } else {
                AnimationType::Open
            };
            anim.progress = curves.sample(raw, action) as f32;
            changed = true;
        }
        for id in to_drop {
            layer_animations.remove(&id);
            changed = true;
        }
    }

    // Advance close animations and pop entries that have settled.
    // Iterate in reverse so we can `swap_remove` cleanly without
    // resampling indices. (Order doesn't matter visually — closing
    // clients don't interact with each other beyond stacking, which
    // we don't preserve in this list anyway.)
    let mut i = 0;
    while i < closing_clients.len() {
        let cc = &mut closing_clients[i];
        let elapsed = now_ms.wrapping_sub(cc.time_started);
        if elapsed >= cc.duration {
            closing_clients.swap_remove(i);
            changed = true;
            continue;
        }
        let raw = elapsed as f64 / cc.duration as f64;
        cc.progress = curves.sample(raw, AnimationType::Close) as f32;
        changed = true;
        i += 1;
    }

    for c in clients.iter_mut() {
        // Resize-snapshot expiry. Bezier mode caps the snapshot's life
        // at `duration_move` (its crossfade alpha is fully transparent
        // by then anyway). Spring mode has no fixed duration — the
        // snapshot is dropped when the spring settles, inside the
        // settle branch below — so we only run the wall-clock cap on
        // bezier here. Otherwise a long spring overshoot would lose
        // its snapshot mid-flight and the live surface (still at the
        // pre-resize size) would suddenly pop into view.
        if !spec.use_spring
            && let Some(snapshot) = c.resize_snapshot.as_ref()
        {
            let dur = std::time::Duration::from_millis(spec.duration_move as u64);
            if snapshot.captured_at.elapsed() >= dur {
                c.resize_snapshot = None;
                changed = true;
            }
        }

        let anim = &mut c.animation;
        if !anim.running {
            continue;
        }
        changed = true;

        if spec.use_spring {
            // Spring path — niri-style analytical solution.
            //
            // The animation already has a precomputed `duration` from
            // arrange_monitor (`Spring::clamped_duration`). We sample
            // the closed-form oscillator at `elapsed` and lerp from
            // initial → current using its [0, 1] progress. This
            // guarantees the animation ends at exactly `duration` ms;
            // the previous numerical integrator could leave the
            // running flag set indefinitely when c.geom rounded onto
            // its target while velocity was still above the velocity-
            // epsilon, producing a CPU-bound tick→render→tick loop.
            let elapsed_ms = now_ms.wrapping_sub(anim.time_started);
            if elapsed_ms >= anim.duration {
                // Hard end. Snap to the exact target — `value_at` may
                // miss it by a fraction of a pixel, and we don't want
                // the difference surviving into the next frame.
                anim.running = false;
                c.geom = anim.current;
                c.resize_snapshot = None;
                continue;
            }
            // 1D progress spring goes 0 → 1 over `duration`. Apply that
            // single progress to all four channels so x/y/w/h move
            // together — for window movement that's exactly what we
            // want (the user perceives a single object travelling, not
            // four independent ones).
            let progress_spring = crate::animation::spring::Spring {
                from: 0.0,
                to: 1.0,
                initial_velocity: 0.0,
                params: crate::animation::spring::SpringParams {
                    damping: spec.spring.params.damping,
                    mass: spec.spring.params.mass,
                    stiffness: spec.spring.params.stiffness,
                    epsilon: spec.spring.params.epsilon,
                },
            };
            let t = std::time::Duration::from_millis(elapsed_ms as u64);
            let p = progress_spring.value_at(t).clamp(0.0, 1.0);
            c.geom.x = lerp_i32(anim.initial.x, anim.current.x, p);
            c.geom.y = lerp_i32(anim.initial.y, anim.current.y, p);
            c.geom.width = lerp_i32(anim.initial.width, anim.current.width, p);
            c.geom.height = lerp_i32(anim.initial.height, anim.current.height, p);
        } else {
            // Bezier path (original behaviour).
            let elapsed = now_ms.wrapping_sub(anim.time_started);
            if elapsed >= anim.duration {
                anim.running = false;
                c.geom = anim.current;
                // Slot animation settled: also drop any lingering
                // snapshot (defensive — the expiry check above usually
                // catches it first).
                c.resize_snapshot = None;
                continue;
            }
            let t = elapsed as f64 / anim.duration as f64;
            let s = curves.sample(t, anim.action);
            c.geom.x = lerp_i32(anim.initial.x, anim.current.x, s);
            c.geom.y = lerp_i32(anim.initial.y, anim.current.y, s);
            c.geom.width = lerp_i32(anim.initial.width, anim.current.width, s);
            c.geom.height = lerp_i32(anim.initial.height, anim.current.height, s);
        }
    }
    changed
}

#[inline]
fn lerp_i32(a: i32, b: i32, t: f64) -> i32 {
    (a as f64 + (b - a) as f64 * t) as i32
}
