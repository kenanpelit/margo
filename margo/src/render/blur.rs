//! Background blur (dual-Kawase) for translucent windows & layer-shells.
//!
//! Unlike `shadow.rs` / `rounded_border.rs` — which are single-pass
//! analytic SDF shaders that deliberately avoid offscreen buffers —
//! real background blur fundamentally needs framebuffer ping-pong.
//! This module implements the niri / Hyprland dual-Kawase filter:
//!
//!   1. **Capture** the slice of the *current* output framebuffer that
//!      sits behind the blur region into an offscreen texture.
//!      Because elements are drawn back-to-front (painter's algorithm,
//!      see `OutputDamageTracker` in smithay), by the time a blur
//!      element draws, everything beneath it (wallpaper, lower
//!      windows, lower layers) is already composited into the
//!      backbuffer — so a `glCopyTexSubImage2D` from the bound FBO
//!      reads exactly the "background" we want to blur.
//!   2. **Downsample** `num_passes` times — each pass a Kawase
//!      down-filter at half resolution. This is where the cheap, wide
//!      blur comes from: the kernel footprint doubles each halving.
//!   3. **Upsample** `num_passes` times back up with the Kawase
//!      up-filter.
//!   4. **Composite** the blurred texture back onto the output,
//!      clipped to the surface's rounded rect, applying the
//!      brightness / contrast / saturation / noise post-adjustments.
//!
//! Everything runs inside `GlesFrame::with_context` using raw GLES2
//! (the smithay GLES API at this pin exposes `pub mod ffi` + the raw
//! `link_program` helper, but renders to its *own* bound target — so
//! the only way to ping-pong through our own FBOs from within a
//! `RenderElement::draw` is raw GL). We save and restore every bit of
//! GL state we touch so smithay's renderer keeps working afterwards.
//!
//! ## Safety / bring-up
//!
//! `Config::blur` defaults to **false** (see `margo-config`), so a bug
//! here cannot break everyone's rendering — the whole pass is gated
//! behind the explicit opt-in. Program compilation and FBO setup are
//! all non-fatal: any failure logs once and the element draws nothing
//! (the translucent surface just composites over the un-blurred
//! background, exactly as it does today with `blur = off`).
//!
//! ## Damage
//!
//! The element is a smithay *framebuffer effect*: steps 1-3 run in
//! `capture_framebuffer`, which the damage tracker calls only when
//! something behind the element changed this frame — and it then redraws
//! the whole backdrop under the element first, so the capture never reads
//! stale or already-composited pixels. The blurred image is cached per
//! element and output (the tracker's effect cache); `draw` (step 4)
//! composites it over just the damaged rects. Age-based damage tracking
//! stays fully on, and a static blurred screen renders nothing.
//!
//! Wiring lives in `udev.rs::push_client_elements` /
//! `push_layer_elements`: when `Config::blur` (windows) or
//! `Config::blur_layer` (layer-shells) is set, the client/layer isn't
//! `no_blur`, and the surface is translucent, a `BlurRenderElement` is
//! pushed directly underneath the surface.

use crate::render::shader_cache::ShaderSlot;
use std::cell::RefCell;

use smithay::backend::renderer::gles::{GlesError, GlesRenderer, ffi, link_program};
use smithay::utils::{Physical, Rectangle, Size, Transform};

mod element;
mod passes;
mod shaders;

pub use element::BlurRenderElement;
use shaders::{COMPOSITE_FRAG_SRC, COMPOSITE_VERT_SRC, DOWN_FRAG_SRC, UP_FRAG_SRC, VERT_SRC};

/// Clamp the configured pass count to a sane range. 0 disables blur
/// (the element draws nothing); >6 is pointless — each pass halves
/// resolution, so by 6 passes the smallest mip is sub-pixel on any
/// real output. Hyprland / niri cap around the same.
pub const MAX_PASSES: i32 = 6;

/// Resolve the effective dual-Kawase pass count for a given blur
/// region. Each downsample halves the working resolution, so we stop
/// early once a further halving would collapse a dimension below 1px —
/// otherwise the FBO allocation / sampling math degenerates. Returns
/// the number of *down* (== *up*) passes actually usable, clamped to
/// `[0, MAX_PASSES]`.
pub fn effective_passes(num_passes: i32, region_w: i32, region_h: i32) -> i32 {
    let requested = num_passes.clamp(0, MAX_PASSES);
    if requested == 0 || region_w <= 0 || region_h <= 0 {
        return 0;
    }
    let mut w = region_w;
    let mut h = region_h;
    let mut usable = 0;
    for _ in 0..requested {
        w /= 2;
        h /= 2;
        if w < 1 || h < 1 {
            break;
        }
        usable += 1;
    }
    usable
}

/// Width / height of mip level `level` (0 = full region) when halving
/// each step. Never returns 0 in either dimension.
pub fn mip_size(region_w: i32, region_h: i32, level: i32) -> (i32, i32) {
    let mut w = region_w;
    let mut h = region_h;
    for _ in 0..level {
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    (w.max(1), h.max(1))
}

/// How many consecutive draws a pool slot may sit at more than twice the
/// area it is used for before it is shrunk to the size in use.
pub const SHRINK_AFTER_DRAWS: u32 = 120;

/// What to do with a pool slot of `slot` size asked to hold `want`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotPlan {
    /// Keep the texture; carry the new over-size streak.
    Keep { oversized_draws: u32 },
    /// Re-allocate the texture at this size.
    Resize(i32, i32),
}

/// Pool slot sizing: grow to fit, and give memory back once a slot has
/// been far larger than needed for [`SHRINK_AFTER_DRAWS`] draws in a row.
pub fn plan_slot(slot: (i32, i32), want: (i32, i32), oversized_draws: u32) -> SlotPlan {
    if slot.0 < want.0 || slot.1 < want.1 {
        // Grow to the larger of both on each axis, so a wide bar and a
        // tall window in the same frame don't re-allocate in turn.
        return SlotPlan::Resize(slot.0.max(want.0), slot.1.max(want.1));
    }
    let slot_area = i64::from(slot.0) * i64::from(slot.1);
    let want_area = i64::from(want.0) * i64::from(want.1);
    if want_area * 2 >= slot_area {
        return SlotPlan::Keep { oversized_draws: 0 };
    }
    let streak = oversized_draws.saturating_add(1);
    if streak >= SHRINK_AFTER_DRAWS {
        SlotPlan::Resize(want.0, want.1)
    } else {
        SlotPlan::Keep {
            oversized_draws: streak,
        }
    }
}

/// How long the pool may sit unused before its textures are freed.
pub const POOL_IDLE_RELEASE: std::time::Duration = std::time::Duration::from_secs(5);

/// Whether a pool last used at `last_use` should be freed at `now`.
pub fn pool_idle(last_use: Option<std::time::Instant>, now: std::time::Instant) -> bool {
    last_use.is_some_and(|t| now.saturating_duration_since(t) >= POOL_IDLE_RELEASE)
}

/// Delay for the single idle-pool cleanup timer. A timer must release
/// scratch textures even when the output stops rendering altogether.
fn cleanup_delay(
    last_use: Option<std::time::Instant>,
    armed: bool,
    now: std::time::Instant,
) -> Option<std::time::Duration> {
    if armed {
        return None;
    }
    last_use.map(|t| POOL_IDLE_RELEASE.saturating_sub(now.saturating_duration_since(t)))
}

/// Claim the one cleanup timer, when needed. No timer exists while blur
/// has never run or its pool is already empty.
pub fn next_cleanup_delay() -> Option<std::time::Duration> {
    CACHED.with(|slot| {
        let mut borrow = slot.borrow_mut();
        let gl = borrow.ready_mut()?;
        if gl.pool.is_empty() {
            return None;
        }
        let delay = cleanup_delay(gl.last_use, gl.cleanup_armed, std::time::Instant::now())?;
        gl.cleanup_armed = true;
        Some(delay)
    })
}

/// Release ownership after the timer fired (or insertion failed).
pub fn cleanup_timer_finished() {
    CACHED.with(|slot| {
        if let Some(gl) = slot.borrow_mut().ready_mut() {
            gl.cleanup_armed = false;
        }
    });
}

/// Lazily-compiled GL programs + a small per-renderer FBO/texture pool
/// for the ping-pong chain. Cached thread-local, mirroring the
/// `shadow::shader` pattern (the renderer lives on a single thread).
struct BlurGl {
    /// Kawase down-filter program.
    down_prog: ffi::types::GLuint,
    /// Kawase up-filter program.
    up_prog: ffi::types::GLuint,
    /// Final composite program (rounded-rect clip + colour adjust +
    /// noise).
    composite_prog: ffi::types::GLuint,
    /// Pool of (texture, fbo) pairs reused across frames. Index 0 is
    /// the captured background; the rest are the down/up mip chain.
    /// Each entry tracks its current allocated size so we only
    /// re-`TexImage2D` when the region grows, or shrinks for good
    /// ([`plan_slot`]); the whole pool is freed after
    /// [`POOL_IDLE_RELEASE`] without a blur pass ([`release_idle_pool`]).
    pool: Vec<PoolTex>,
    /// When a blur pass last used the pool.
    last_use: Option<std::time::Instant>,
    /// One idle cleanup timer, independent of output frame scheduling.
    cleanup_armed: bool,
    /// Fullscreen-quad vertex buffer (two triangles, NDC).
    quad_vbo: ffi::types::GLuint,
    /// FBO the last up pass renders through into a per-element
    /// [`Backdrop`] texture (attached per capture).
    cache_fbo: ffi::types::GLuint,
}

struct PoolTex {
    tex: ffi::types::GLuint,
    fbo: ffi::types::GLuint,
    w: i32,
    h: i32,
    /// Consecutive uses at under half its area ([`plan_slot`]).
    oversized_draws: u32,
}

thread_local! {
    static CACHED: RefCell<ShaderSlot<BlurGl>> = const { RefCell::new(ShaderSlot::Pending) };
}

/// One-shot diagnostic: log the first `draw_blur` invocation's geometry
/// + GL error so an on-hardware "blur shows nothing" can be triaged from
/// the log instead of the screen. Remove once blur is verified.
static BLUR_DIAG_DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static BLUR_DIAG_COMPOSITE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Where an element drawn at `dst` (output space, top-left origin) lands
/// in the framebuffer, in GL window coordinates, for a frame rendered
/// with `transform` (`GlesFrame::transformation()`, i.e. the output
/// transform already inverted by the damage tracker) into a framebuffer
/// of `fb_size` (the mode size; the GL viewport).
///
/// smithay's projection flips Y for OpenGL, so window row 0 is the top
/// row of the output buffer and, for `Transform::Normal`, the result is
/// `dst` itself. For the other seven it is `dst` moved by the frame
/// transform within the output-space size — exactly smithay's
/// `transform_rect_in`, which the tests check against the projection
/// matrix and against real GL rendering.
pub fn framebuffer_rect(
    dst: Rectangle<i32, Physical>,
    transform: Transform,
    fb_size: Size<i32, Physical>,
) -> Rectangle<i32, Physical> {
    transform.transform_rect_in(dst, &transform.transform_size(fb_size))
}

/// Marker handle proving the blur GL resources compiled successfully.
/// `push_*_elements` checks `shader()` like it does for shadows; the
/// element itself re-fetches the thread-local at draw time.
#[derive(Debug, Clone, Copy)]
pub struct BlurReady;

/// Compile-once-per-thread. Returns `Some` once the programs link and
/// the quad VBO is allocated; failures log once and return `None`, so
/// the caller simply skips the blur element.
pub fn shader(renderer: &mut GlesRenderer) -> Option<BlurReady> {
    CACHED.with(|slot| {
        slot.borrow_mut()
            .get_or_compile("blur", || {
                // SAFETY: `with_context` makes the renderer's GL context current for the closure.
                match renderer.with_context(|gl| unsafe { compile_blur_gl(gl) }) {
                    Ok(compiled) => compiled.map_err(|e| format!("{e:?}")),
                    Err(e) => Err(format!("context: {e:?}")),
                }
            })
            .map(|_| BlurReady)
    })
}

/// Free the blur scratch textures once no blur pass has run for
/// [`POOL_IDLE_RELEASE`]: one full-output blur sizes the pool to the
/// output (about 35 MB at 1440p with the mip chain), which would otherwise
/// stay allocated for the session. Cheap when there is nothing to free
/// (a thread-local check); call it once per rendered frame.
pub fn release_idle_pool(renderer: &mut GlesRenderer) {
    let idle = CACHED.with(|slot| {
        slot.borrow_mut().ready_mut().is_some_and(|gl| {
            !gl.pool.is_empty() && pool_idle(gl.last_use, std::time::Instant::now())
        })
    });
    if !idle {
        return;
    }
    // SAFETY: `with_context` makes the renderer's GL context current; the
    // pool's names were created in it.
    let freed = renderer.with_context(|gl| unsafe {
        CACHED.with(|slot| {
            let mut borrow = slot.borrow_mut();
            let Some(blur_gl) = borrow.ready_mut() else {
                return 0;
            };
            let n = blur_gl.pool.len();
            for p in blur_gl.pool.drain(..) {
                gl.DeleteTextures(1, &p.tex);
                gl.DeleteFramebuffers(1, &p.fbo);
            }
            blur_gl.last_use = None;
            n
        })
    });
    match freed {
        Ok(n) => {
            tracing::debug!(target: "margo::render::blur", "freed {n} idle blur scratch textures")
        }
        Err(e) => tracing::warn!(target: "margo::render::blur", "free idle blur pool: {e:?}"),
    }
}

/// # Safety
/// Must be called with a current GL context (inside `with_context`).
unsafe fn compile_blur_gl(gl: &ffi::Gles2) -> Result<BlurGl, GlesError> {
    // SAFETY: caller guarantees a current GL context (this fn's `# Safety`).
    let down_prog = unsafe { link_program(gl, VERT_SRC, DOWN_FRAG_SRC)? };
    // SAFETY: caller guarantees a current GL context (this fn's `# Safety`).
    let up_prog = unsafe { link_program(gl, VERT_SRC, UP_FRAG_SRC)? };
    // SAFETY: caller guarantees a current GL context (this fn's `# Safety`).
    let composite_prog = unsafe { link_program(gl, COMPOSITE_VERT_SRC, COMPOSITE_FRAG_SRC)? };

    let mut quad_vbo = 0;
    let mut cache_fbo = 0;
    // Two triangles covering NDC [-1,1]². Interleaved: pos.xy, uv.xy.
    // UVs run 0..1 with origin bottom-left (GL texture convention).
    #[rustfmt::skip]
    let verts: [f32; 24] = [
        -1.0, -1.0, 0.0, 0.0,
         1.0, -1.0, 1.0, 0.0,
         1.0,  1.0, 1.0, 1.0,
        -1.0, -1.0, 0.0, 0.0,
         1.0,  1.0, 1.0, 1.0,
        -1.0,  1.0, 0.0, 1.0,
    ];
    // SAFETY: caller guarantees a current GL context; `quad_vbo` and `quad` outlive the calls.
    unsafe {
        gl.GenBuffers(1, &mut quad_vbo);
        gl.BindBuffer(ffi::ARRAY_BUFFER, quad_vbo);
        gl.BufferData(
            ffi::ARRAY_BUFFER,
            std::mem::size_of_val(&verts) as isize,
            verts.as_ptr() as *const _,
            ffi::STATIC_DRAW,
        );
        gl.BindBuffer(ffi::ARRAY_BUFFER, 0);
        gl.GenFramebuffers(1, &mut cache_fbo);
    }

    Ok(BlurGl {
        down_prog,
        up_prog,
        composite_prog,
        pool: Vec::new(),
        last_use: None,
        cleanup_armed: false,
        quad_vbo,
        cache_fbo,
    })
}

impl BlurGl {
    /// Ensure pool slot `idx` exists and is at least `w`×`h`, growing
    /// (re-allocating the backing texture) when the region grows and
    /// shrinking it when [`plan_slot`] says it has been oversized long
    /// enough. Returns the (texture, fbo) for that slot.
    ///
    /// # Safety
    /// Current GL context required.
    unsafe fn pool_slot(
        &mut self,
        gl: &ffi::Gles2,
        idx: usize,
        w: i32,
        h: i32,
    ) -> (ffi::types::GLuint, ffi::types::GLuint) {
        while self.pool.len() <= idx {
            let mut tex = 0;
            let mut fbo = 0;
            // SAFETY: only reached from render paths that hold the renderer's GL context; `tex` / `fbo` are valid out-pointers.
            unsafe {
                gl.GenTextures(1, &mut tex);
                gl.GenFramebuffers(1, &mut fbo);
                gl.BindTexture(ffi::TEXTURE_2D, tex);
                gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MIN_FILTER, ffi::LINEAR as i32);
                gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MAG_FILTER, ffi::LINEAR as i32);
                gl.TexParameteri(
                    ffi::TEXTURE_2D,
                    ffi::TEXTURE_WRAP_S,
                    ffi::CLAMP_TO_EDGE as i32,
                );
                gl.TexParameteri(
                    ffi::TEXTURE_2D,
                    ffi::TEXTURE_WRAP_T,
                    ffi::CLAMP_TO_EDGE as i32,
                );
            }
            self.pool.push(PoolTex {
                tex,
                fbo,
                w: 0,
                h: 0,
                oversized_draws: 0,
            });
        }
        let slot = &mut self.pool[idx];
        let plan = plan_slot((slot.w, slot.h), (w, h), slot.oversized_draws);
        if let SlotPlan::Keep { oversized_draws } = plan {
            slot.oversized_draws = oversized_draws;
        }
        if let SlotPlan::Resize(nw, nh) = plan {
            slot.oversized_draws = 0;
            // SAFETY: same GL-context contract as the pool fill above; `slot.tex` was created by this pool.
            unsafe {
                gl.BindTexture(ffi::TEXTURE_2D, slot.tex);
                gl.TexImage2D(
                    ffi::TEXTURE_2D,
                    0,
                    ffi::RGBA as i32,
                    nw,
                    nh,
                    0,
                    ffi::RGBA,
                    ffi::UNSIGNED_BYTE,
                    std::ptr::null(),
                );
                gl.BindFramebuffer(ffi::FRAMEBUFFER, slot.fbo);
                gl.FramebufferTexture2D(
                    ffi::FRAMEBUFFER,
                    ffi::COLOR_ATTACHMENT0,
                    ffi::TEXTURE_2D,
                    slot.tex,
                    0,
                );
            }
            slot.w = nw;
            slot.h = nh;
        }
        (slot.tex, slot.fbo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_TRANSFORMS: [Transform; 8] = [
        Transform::Normal,
        Transform::_90,
        Transform::_180,
        Transform::_270,
        Transform::Flipped,
        Transform::Flipped90,
        Transform::Flipped180,
        Transform::Flipped270,
    ];

    /// smithay's GlesRenderer::render projection, replayed in f64: output-
    /// space pixel → NDC (top-left ortho over the transformed size), then
    /// the frame transform's matrix, then the Y flip for OpenGL, then the
    /// viewport. Returns GL window coordinates.
    fn smithay_project(p: (f64, f64), transform: Transform, fb: (i32, i32)) -> (f64, f64) {
        let out = transform.transform_size(Size::<i32, Physical>::from(fb));
        let ux = 2.0 * p.0 / f64::from(out.w) - 1.0;
        let uy = 1.0 - 2.0 * p.1 / f64::from(out.h);
        let m = transform.matrix();
        let a = f64::from(m.x_axis.x) * ux + f64::from(m.y_axis.x) * uy;
        let b = f64::from(m.x_axis.y) * ux + f64::from(m.y_axis.y) * uy;
        let (nx, ny) = (a, -b);
        (
            (nx + 1.0) / 2.0 * f64::from(fb.0),
            (ny + 1.0) / 2.0 * f64::from(fb.1),
        )
    }

    #[test]
    fn framebuffer_rect_matches_smithays_projection_for_every_transform() {
        let fb = (1920, 1080);
        let rects = [
            Rectangle::new((0, 0).into(), (300, 40).into()),
            Rectangle::new((17, 230).into(), (640, 480).into()),
        ];
        for t in ALL_TRANSFORMS {
            for dst in rects {
                let a = smithay_project((f64::from(dst.loc.x), f64::from(dst.loc.y)), t, fb);
                let b = smithay_project(
                    (
                        f64::from(dst.loc.x + dst.size.w),
                        f64::from(dst.loc.y + dst.size.h),
                    ),
                    t,
                    fb,
                );
                let expected = Rectangle::<i32, Physical>::new(
                    (a.0.min(b.0).round() as i32, a.1.min(b.1).round() as i32).into(),
                    (
                        (a.0 - b.0).abs().round() as i32,
                        (a.1 - b.1).abs().round() as i32,
                    )
                        .into(),
                );
                assert_eq!(
                    framebuffer_rect(dst, t, fb.into()),
                    expected,
                    "{t:?} {dst:?}"
                );
            }
        }
    }

    #[test]
    fn framebuffer_rect_examples() {
        // A 300x40 top bar on a 1920x1080 panel.
        let bar = Rectangle::<i32, Physical>::new((0, 0).into(), (300, 40).into());
        let fb: Size<i32, Physical> = (1920, 1080).into();
        // Normal: window row 0 is the top row, nothing moves.
        assert_eq!(framebuffer_rect(bar, Transform::Normal, fb), bar);
        // Upside down: bottom-right corner of the buffer.
        assert_eq!(
            framebuffer_rect(bar, Transform::_180, fb),
            Rectangle::new((1620, 1040).into(), (300, 40).into())
        );
        // Mirrored: top-right.
        assert_eq!(
            framebuffer_rect(bar, Transform::Flipped, fb),
            Rectangle::new((1620, 0).into(), (300, 40).into())
        );
        // Portrait (output space 1080x1920): the bar becomes a 40x300
        // column; frame transform _90 puts the output's top edge on the
        // buffer's right edge.
        assert_eq!(
            framebuffer_rect(bar, Transform::_90, fb),
            Rectangle::new((1880, 0).into(), (40, 300).into())
        );
        assert_eq!(
            framebuffer_rect(bar, Transform::_270, fb),
            Rectangle::new((0, 780).into(), (40, 300).into())
        );
    }

    #[test]
    fn pool_slots_grow_to_fit() {
        assert_eq!(plan_slot((0, 0), (300, 40), 0), SlotPlan::Resize(300, 40));
        // Grows on the axis that is short, keeps the other.
        assert_eq!(
            plan_slot((300, 40), (200, 80), 0),
            SlotPlan::Resize(300, 80)
        );
    }

    #[test]
    fn a_slot_used_at_its_size_is_kept() {
        assert_eq!(
            plan_slot((300, 80), (300, 80), 7),
            SlotPlan::Keep { oversized_draws: 0 }
        );
        // Over half the area in use: not worth a re-allocation.
        assert_eq!(
            plan_slot((300, 80), (250, 60), 7),
            SlotPlan::Keep { oversized_draws: 0 }
        );
    }

    #[test]
    fn an_oversized_slot_shrinks_after_a_while() {
        // A full-output blur (2560x1440) went away; a 300x40 bar blur is
        // all that is left. The slot must not stay output-sized forever.
        let (slot, want) = ((2560, 1440), (300, 40));
        let mut streak = 0;
        let mut plans = Vec::new();
        for _ in 0..SHRINK_AFTER_DRAWS {
            let plan = plan_slot(slot, want, streak);
            if let SlotPlan::Keep { oversized_draws } = plan {
                streak = oversized_draws;
            }
            plans.push(plan);
        }
        assert!(
            plans[..plans.len() - 1]
                .iter()
                .all(|p| matches!(p, SlotPlan::Keep { .. })),
            "no re-allocation churn while the size settles"
        );
        assert_eq!(plans.last(), Some(&SlotPlan::Resize(300, 40)));
    }

    #[test]
    fn unused_blur_memory_has_a_timer_even_if_no_more_frames_render() {
        let now = std::time::Instant::now();
        assert_eq!(
            cleanup_delay(Some(now), false, now),
            Some(POOL_IDLE_RELEASE)
        );
        assert_eq!(cleanup_delay(Some(now), true, now), None);
        assert_eq!(cleanup_delay(None, false, now), None);
        assert_eq!(
            cleanup_delay(Some(now), false, now + POOL_IDLE_RELEASE),
            Some(std::time::Duration::ZERO)
        );
    }

    #[test]
    fn the_pool_is_freed_only_after_it_sat_idle() {
        let t = std::time::Instant::now();
        assert!(!pool_idle(None, t));
        assert!(!pool_idle(Some(t), t + POOL_IDLE_RELEASE / 2));
        assert!(pool_idle(Some(t), t + POOL_IDLE_RELEASE));
    }

    #[test]
    fn passes_zero_when_disabled() {
        assert_eq!(effective_passes(0, 1000, 1000), 0);
        assert_eq!(effective_passes(3, 0, 100), 0);
        assert_eq!(effective_passes(3, 100, 0), 0);
    }

    #[test]
    fn passes_clamped_to_max() {
        // A huge region can afford every pass, but never exceed MAX.
        assert_eq!(effective_passes(100, 4096, 4096), MAX_PASSES);
    }

    #[test]
    fn passes_stop_before_subpixel() {
        // 8px region: 8 -> 4 -> 2 -> 1 -> (0, stop). 3 usable passes.
        assert_eq!(effective_passes(6, 8, 8), 3);
        // 4px: 4 -> 2 -> 1 -> stop = 2 passes.
        assert_eq!(effective_passes(6, 4, 4), 2);
        // 1px: first halving already < 1 => 0 passes.
        assert_eq!(effective_passes(3, 1, 1), 0);
    }

    #[test]
    fn passes_respect_request_when_room() {
        // Plenty of room, request 3 => exactly 3.
        assert_eq!(effective_passes(3, 1024, 1024), 3);
    }

    #[test]
    fn mip_size_halves_and_floors_at_one() {
        assert_eq!(mip_size(1000, 500, 0), (1000, 500));
        assert_eq!(mip_size(1000, 500, 1), (500, 250));
        assert_eq!(mip_size(1000, 500, 2), (250, 125));
        // Never collapses to zero.
        assert_eq!(mip_size(1, 1, 5), (1, 1));
    }
}

/// Blur placement checked on a real GL context (render node + GBM/EGL,
/// rendering into an offscreen texture, no display needed). Skips with a
/// note when no render node / EGL is available (headless CI).
#[cfg(test)]
pub(crate) mod gpu_tests;
