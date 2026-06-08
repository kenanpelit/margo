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
//! Wiring lives in `udev.rs::push_client_elements` /
//! `push_layer_elements`: when `Config::blur` (windows) or
//! `Config::blur_layer` (layer-shells) is set, the client/layer isn't
//! `no_blur`, and the surface is translucent, a `BlurRenderElement` is
//! pushed directly underneath the surface.

use std::cell::RefCell;

use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{GlesError, GlesFrame, GlesRenderer, ffi, link_program};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Logical, Physical, Rectangle, Scale, Transform};

use margo_config::BlurParams;

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
    /// re-`TexImage2D` when the region grows.
    pool: Vec<PoolTex>,
    /// Fullscreen-quad vertex buffer (two triangles, NDC).
    quad_vbo: ffi::types::GLuint,
}

struct PoolTex {
    tex: ffi::types::GLuint,
    fbo: ffi::types::GLuint,
    w: i32,
    h: i32,
}

thread_local! {
    static CACHED: RefCell<Option<BlurGl>> = const { RefCell::new(None) };
}

/// One-shot diagnostic: log the first `draw_blur` invocation's geometry
/// + GL error so an on-hardware "blur shows nothing" can be triaged from
/// the log instead of the screen. Remove once blur is verified.
static BLUR_DIAG_DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

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
        if slot.borrow().is_some() {
            return Some(BlurReady);
        }
        let compiled = renderer.with_context(|gl| unsafe { compile_blur_gl(gl) });
        match compiled {
            Ok(Ok(blur_gl)) => {
                *slot.borrow_mut() = Some(blur_gl);
                Some(BlurReady)
            }
            Ok(Err(e)) => {
                tracing::error!("blur shader compile failed: {e:?}");
                None
            }
            Err(e) => {
                tracing::error!("blur shader compile failed (context): {e:?}");
                None
            }
        }
    })
}

/// # Safety
/// Must be called with a current GL context (inside `with_context`).
unsafe fn compile_blur_gl(gl: &ffi::Gles2) -> Result<BlurGl, GlesError> {
    let down_prog = unsafe { link_program(gl, VERT_SRC, DOWN_FRAG_SRC)? };
    let up_prog = unsafe { link_program(gl, VERT_SRC, UP_FRAG_SRC)? };
    let composite_prog = unsafe { link_program(gl, COMPOSITE_VERT_SRC, COMPOSITE_FRAG_SRC)? };

    let mut quad_vbo = 0;
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
    }

    Ok(BlurGl {
        down_prog,
        up_prog,
        composite_prog,
        pool: Vec::new(),
        quad_vbo,
    })
}

impl BlurGl {
    /// Ensure pool slot `idx` exists and is at least `w`×`h`, growing
    /// (re-allocating the backing texture) when the region grows.
    /// Returns the (texture, fbo) for that slot.
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
            });
        }
        let slot = &mut self.pool[idx];
        if slot.w < w || slot.h < h {
            let nw = slot.w.max(w);
            let nh = slot.h.max(h);
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

/// A blur draw under one translucent surface.
#[derive(Debug)]
pub struct BlurRenderElement {
    id: Id,
    /// The surface rect in logical coords (output-relative origin is
    /// applied by the caller before constructing this).
    geometry: Rectangle<i32, Logical>,
    /// Rounded-corner radius (logical px).
    corner_radius: f32,
    params: BlurParams,
    scale: Scale<f64>,
    commit: CommitCounter,
}

impl BlurRenderElement {
    pub fn new(
        id: Id,
        rect: Rectangle<i32, Logical>,
        corner_radius: f32,
        params: BlurParams,
        scale: Scale<f64>,
    ) -> Self {
        Self {
            id,
            geometry: rect,
            corner_radius,
            params,
            scale,
            commit: CommitCounter::default(),
        }
    }
}

impl Element for BlurRenderElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        Rectangle::new(
            (0.0, 0.0).into(),
            (self.geometry.size.w as f64, self.geometry.size.h as f64).into(),
        )
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry.to_physical_precise_round(self.scale)
    }

    fn transform(&self) -> Transform {
        Transform::Normal
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        // Blur depends on whatever is *behind* the surface, which can
        // change every frame independently of our own commit — so we
        // always report the full region as damaged. (A future
        // `blur_optimized` path can intersect this with actual
        // background damage; for now correctness over efficiency.)
        let _ = commit;
        DamageSet::from_slice(&[Rectangle::new(
            Default::default(),
            self.geometry(scale).size,
        )])
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        // Rounded + translucent: never claim opacity, or the
        // compositor would skip drawing the background we sample.
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        1.0
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

impl RenderElement<GlesRenderer> for BlurRenderElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        _damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        // `projection` maps output-physical pixel coords → clip space,
        // already accounting for the frame's transform. We pass it to
        // the composite shader so our final quad lands exactly on `dst`
        // regardless of output rotation.
        let projection = *frame.projection();
        let region_w = dst.size.w.max(1);
        let region_h = dst.size.h.max(1);
        let corner_px = self.corner_radius * self.scale.x as f32;
        let params = self.params;

        frame.with_context(|gl| unsafe {
            CACHED.with(|slot| {
                let mut borrow = slot.borrow_mut();
                let Some(blur_gl) = borrow.as_mut() else {
                    return;
                };
                draw_blur(
                    gl,
                    blur_gl,
                    dst,
                    region_w,
                    region_h,
                    corner_px,
                    &params,
                    &projection,
                );
            });
        })?;
        Ok(())
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

/// The actual dual-Kawase + composite pass. All GL state we touch is
/// captured up front and restored at the end.
///
/// # Safety
/// Current GL context; `blur_gl` programs/VBO are valid.
#[allow(clippy::too_many_arguments)]
unsafe fn draw_blur(
    gl: &ffi::Gles2,
    blur_gl: &mut BlurGl,
    dst: Rectangle<i32, Physical>,
    region_w: i32,
    region_h: i32,
    corner_px: f32,
    params: &BlurParams,
    projection: &[f32; 9],
) {
    let passes = effective_passes(params.num_passes, region_w, region_h);
    if passes == 0 {
        return;
    }

    unsafe {
        // ── Save GL state ────────────────────────────────────────────
        let mut prev_fbo = 0;
        let mut prev_vp = [0i32; 4];
        let mut prev_prog = 0;
        let mut prev_tex = 0;
        let mut prev_array_buf = 0;
        let mut prev_active_tex = 0;
        gl.GetIntegerv(ffi::FRAMEBUFFER_BINDING, &mut prev_fbo);
        gl.GetIntegerv(ffi::VIEWPORT, prev_vp.as_mut_ptr());
        gl.GetIntegerv(ffi::CURRENT_PROGRAM, &mut prev_prog);
        gl.GetIntegerv(ffi::TEXTURE_BINDING_2D, &mut prev_tex);
        gl.GetIntegerv(ffi::ARRAY_BUFFER_BINDING, &mut prev_array_buf);
        gl.GetIntegerv(ffi::ACTIVE_TEXTURE, &mut prev_active_tex);
        let prev_blend = gl.IsEnabled(ffi::BLEND);
        let prev_scissor = gl.IsEnabled(ffi::SCISSOR_TEST);

        let output_fbo = prev_fbo as ffi::types::GLuint;

        // ── Capture the background slice behind `dst` ────────────────
        // Slot 0 is the captured background at full region resolution.
        let (cap_tex, _cap_fbo) = blur_gl.pool_slot(gl, 0, region_w, region_h);
        gl.ActiveTexture(ffi::TEXTURE0);
        gl.BindTexture(ffi::TEXTURE_2D, cap_tex);
        // Copy from the currently-bound (output) read framebuffer.
        // `CopyTexSubImage2D`'s source `(x, y)` is in GL window coords
        // (bottom-left origin), whereas `dst.loc` is top-left physical —
        // so we flip Y against the framebuffer height. The viewport
        // smithay set covers the whole output, so `prev_vp[3]` is the
        // framebuffer height. The captured texture therefore stores the
        // region right-side-up in GL texture space (row 0 = bottom). The
        // composite step samples with V flipped so it re-emits the blur
        // in the same orientation as the background it replaces.
        let fb_h = prev_vp[3];
        let src_y = fb_h - dst.loc.y - region_h;
        gl.BindFramebuffer(ffi::FRAMEBUFFER, output_fbo);
        gl.CopyTexSubImage2D(
            ffi::TEXTURE_2D,
            0,
            0,
            0,
            dst.loc.x,
            src_y,
            region_w,
            region_h,
        );

        // One-shot diagnostic (see BLUR_DIAG_DONE). Logs the first draw so
        // a "blur invisible" report can be triaged from the journal.
        if !BLUR_DIAG_DONE.swap(true, std::sync::atomic::Ordering::Relaxed) {
            let err = gl.GetError();
            tracing::warn!(
                target: "margo::render::blur",
                "BLUR-DIAG first draw: dst.loc=({},{}) region={}x{} fb_h={} src_y={} passes={} capture_glerror=0x{:x}",
                dst.loc.x, dst.loc.y, region_w, region_h, fb_h, src_y, passes, err,
            );
        }

        gl.Disable(ffi::SCISSOR_TEST);
        gl.Disable(ffi::BLEND);
        gl.BindBuffer(ffi::ARRAY_BUFFER, blur_gl.quad_vbo);

        // Source for the first downsample is the capture; thereafter the
        // previous mip. We use pool slots 1..=passes for the down chain
        // and reuse them in reverse for the up chain.
        let mut src_tex = cap_tex;
        let mut src_w = region_w;
        let mut src_h = region_h;

        // ── Downsample ───────────────────────────────────────────────
        for level in 1..=passes {
            let (mw, mh) = mip_size(region_w, region_h, level);
            let (tex, fbo) = blur_gl.pool_slot(gl, level as usize, mw, mh);
            let (slot_w, slot_h) = (
                blur_gl.pool[level as usize].w,
                blur_gl.pool[level as usize].h,
            );
            run_pass(
                gl,
                blur_gl.down_prog,
                blur_gl.quad_vbo,
                fbo,
                mw,
                mh,
                src_tex,
                // Half-texel offset based on the *source* resolution.
                (0.5 / src_w as f32, 0.5 / src_h as f32),
                params.radius as f32,
                // The destination texture may be larger than the mip (pool
                // grows monotonically); render into the [0,mw]×[0,mh]
                // sub-rect and sample that same sub-rect on the next pass.
                (mw as f32 / slot_w as f32, mh as f32 / slot_h as f32),
            );
            src_tex = tex;
            src_w = mw;
            src_h = mh;
        }

        // ── Upsample ─────────────────────────────────────────────────
        for level in (0..passes).rev() {
            let (mw, mh) = mip_size(region_w, region_h, level);
            // Level 0 of the up chain is the final blurred result; write it
            // to a dedicated "result" slot (index `passes + 1`) so it never
            // clobbers an intermediate the chain still needs.
            let dst_idx = if level == 0 {
                (passes + 1) as usize
            } else {
                level as usize
            };
            let (tex, fbo) = blur_gl.pool_slot(gl, dst_idx, mw, mh);
            let (slot_w, slot_h) = (blur_gl.pool[dst_idx].w, blur_gl.pool[dst_idx].h);
            run_pass(
                gl,
                blur_gl.up_prog,
                blur_gl.quad_vbo,
                fbo,
                mw,
                mh,
                src_tex,
                (0.5 / src_w as f32, 0.5 / src_h as f32),
                params.radius as f32,
                (mw as f32 / slot_w as f32, mh as f32 / slot_h as f32),
            );
            src_tex = tex;
            src_w = mw;
            src_h = mh;
        }

        // `src_tex` now holds the full-region blurred background. Its valid
        // content is the [0,region]×[0,region] sub-rect of a possibly-
        // larger pool texture.
        let result_idx = (passes + 1) as usize;
        let (rslot_w, rslot_h) = (blur_gl.pool[result_idx].w, blur_gl.pool[result_idx].h);
        let result_uv = (
            region_w as f32 / rslot_w as f32,
            region_h as f32 / rslot_h as f32,
        );

        // ── Composite onto the output, clipped to the rounded rect ───
        gl.BindFramebuffer(ffi::FRAMEBUFFER, output_fbo);
        gl.Viewport(prev_vp[0], prev_vp[1], prev_vp[2], prev_vp[3]);
        gl.Enable(ffi::BLEND);
        gl.BlendFunc(ffi::ONE, ffi::ONE_MINUS_SRC_ALPHA);
        composite_pass(
            gl,
            blur_gl.composite_prog,
            blur_gl.quad_vbo,
            src_tex,
            result_uv,
            dst,
            region_w,
            region_h,
            corner_px,
            params,
            projection,
        );

        // ── Restore GL state ─────────────────────────────────────────
        gl.UseProgram(prev_prog as ffi::types::GLuint);
        gl.ActiveTexture(ffi::TEXTURE0);
        gl.BindTexture(ffi::TEXTURE_2D, prev_tex as ffi::types::GLuint);
        gl.ActiveTexture(prev_active_tex as ffi::types::GLenum);
        gl.BindBuffer(ffi::ARRAY_BUFFER, prev_array_buf as ffi::types::GLuint);
        if prev_blend == ffi::TRUE {
            gl.Enable(ffi::BLEND);
        } else {
            gl.Disable(ffi::BLEND);
        }
        if prev_scissor == ffi::TRUE {
            gl.Enable(ffi::SCISSOR_TEST);
        } else {
            gl.Disable(ffi::SCISSOR_TEST);
        }
        gl.BindFramebuffer(ffi::FRAMEBUFFER, output_fbo);
        gl.Viewport(prev_vp[0], prev_vp[1], prev_vp[2], prev_vp[3]);
    }
}

/// Bind the quad VBO's vertex attributes for program `prog`. Returns
/// the two attribute locations so the caller can disable them after.
///
/// # Safety
/// Current GL context; `vbo` bound to `ARRAY_BUFFER`.
unsafe fn bind_quad_attribs(gl: &ffi::Gles2, prog: ffi::types::GLuint) -> (i32, i32) {
    unsafe {
        let pos_loc = gl.GetAttribLocation(prog, c"a_pos".as_ptr() as *const _);
        let uv_loc = gl.GetAttribLocation(prog, c"a_uv".as_ptr() as *const _);
        let stride = (4 * std::mem::size_of::<f32>()) as i32;
        if pos_loc >= 0 {
            gl.EnableVertexAttribArray(pos_loc as u32);
            gl.VertexAttribPointer(
                pos_loc as u32,
                2,
                ffi::FLOAT,
                ffi::FALSE,
                stride,
                std::ptr::null(),
            );
        }
        if uv_loc >= 0 {
            gl.EnableVertexAttribArray(uv_loc as u32);
            gl.VertexAttribPointer(
                uv_loc as u32,
                2,
                ffi::FLOAT,
                ffi::FALSE,
                stride,
                (2 * std::mem::size_of::<f32>()) as *const _,
            );
        }
        (pos_loc, uv_loc)
    }
}

/// # Safety
/// Current GL context.
unsafe fn disable_quad_attribs(gl: &ffi::Gles2, locs: (i32, i32)) {
    unsafe {
        if locs.0 >= 0 {
            gl.DisableVertexAttribArray(locs.0 as u32);
        }
        if locs.1 >= 0 {
            gl.DisableVertexAttribArray(locs.1 as u32);
        }
    }
}

/// One Kawase pass (down or up) rendering into `fbo` at `mw`×`mh`.
///
/// # Safety
/// Current GL context.
#[allow(clippy::too_many_arguments)]
unsafe fn run_pass(
    gl: &ffi::Gles2,
    prog: ffi::types::GLuint,
    vbo: ffi::types::GLuint,
    fbo: ffi::types::GLuint,
    mw: i32,
    mh: i32,
    src_tex: ffi::types::GLuint,
    half_texel: (f32, f32),
    radius: f32,
    src_uv_scale: (f32, f32),
) {
    unsafe {
        gl.BindFramebuffer(ffi::FRAMEBUFFER, fbo);
        gl.Viewport(0, 0, mw, mh);
        gl.UseProgram(prog);
        gl.BindBuffer(ffi::ARRAY_BUFFER, vbo);
        let locs = bind_quad_attribs(gl, prog);

        gl.ActiveTexture(ffi::TEXTURE0);
        gl.BindTexture(ffi::TEXTURE_2D, src_tex);
        set_uniform_1i(gl, prog, c"tex", 0);
        set_uniform_2f(gl, prog, c"half_texel", half_texel.0, half_texel.1);
        set_uniform_1f(gl, prog, c"radius", radius);
        set_uniform_2f(gl, prog, c"uv_scale", src_uv_scale.0, src_uv_scale.1);

        gl.DrawArrays(ffi::TRIANGLES, 0, 6);
        disable_quad_attribs(gl, locs);
    }
}

/// Final composite: blurred texture → output, rounded-rect clipped,
/// colour-adjusted, with a touch of noise.
///
/// # Safety
/// Current GL context.
#[allow(clippy::too_many_arguments)]
unsafe fn composite_pass(
    gl: &ffi::Gles2,
    prog: ffi::types::GLuint,
    vbo: ffi::types::GLuint,
    blurred_tex: ffi::types::GLuint,
    uv_scale: (f32, f32),
    dst: Rectangle<i32, Physical>,
    region_w: i32,
    region_h: i32,
    corner_px: f32,
    params: &BlurParams,
    projection: &[f32; 9],
) {
    unsafe {
        gl.UseProgram(prog);
        gl.BindBuffer(ffi::ARRAY_BUFFER, vbo);
        let locs = bind_quad_attribs(gl, prog);

        gl.ActiveTexture(ffi::TEXTURE0);
        gl.BindTexture(ffi::TEXTURE_2D, blurred_tex);
        set_uniform_1i(gl, prog, c"tex", 0);
        set_uniform_2f(gl, prog, c"uv_scale", uv_scale.0, uv_scale.1);
        set_uniform_2f(gl, prog, c"region_size", region_w as f32, region_h as f32);
        set_uniform_1f(gl, prog, c"corner_radius", corner_px);
        set_uniform_1f(gl, prog, c"brightness", params.brightness);
        set_uniform_1f(gl, prog, c"contrast", params.contrast);
        set_uniform_1f(gl, prog, c"saturation", params.saturation);
        set_uniform_1f(gl, prog, c"noise", params.noise);
        set_uniform_2f(gl, prog, c"dst_origin", dst.loc.x as f32, dst.loc.y as f32);
        set_uniform_2f(gl, prog, c"dst_size", region_w as f32, region_h as f32);
        set_uniform_mat3(gl, prog, c"projection", projection);

        gl.DrawArrays(ffi::TRIANGLES, 0, 6);
        disable_quad_attribs(gl, locs);
    }
}

// ── Uniform helpers ─────────────────────────────────────────────────

/// # Safety: current GL context.
unsafe fn set_uniform_1i(gl: &ffi::Gles2, prog: ffi::types::GLuint, name: &std::ffi::CStr, v: i32) {
    unsafe {
        let loc = gl.GetUniformLocation(prog, name.as_ptr() as *const _);
        if loc >= 0 {
            gl.Uniform1i(loc, v);
        }
    }
}

/// # Safety: current GL context.
unsafe fn set_uniform_1f(gl: &ffi::Gles2, prog: ffi::types::GLuint, name: &std::ffi::CStr, v: f32) {
    unsafe {
        let loc = gl.GetUniformLocation(prog, name.as_ptr() as *const _);
        if loc >= 0 {
            gl.Uniform1f(loc, v);
        }
    }
}

/// # Safety: current GL context.
unsafe fn set_uniform_2f(
    gl: &ffi::Gles2,
    prog: ffi::types::GLuint,
    name: &std::ffi::CStr,
    a: f32,
    b: f32,
) {
    unsafe {
        let loc = gl.GetUniformLocation(prog, name.as_ptr() as *const _);
        if loc >= 0 {
            gl.Uniform2f(loc, a, b);
        }
    }
}

/// # Safety: current GL context.
unsafe fn set_uniform_mat3(
    gl: &ffi::Gles2,
    prog: ffi::types::GLuint,
    name: &std::ffi::CStr,
    m: &[f32; 9],
) {
    unsafe {
        let loc = gl.GetUniformLocation(prog, name.as_ptr() as *const _);
        if loc >= 0 {
            gl.UniformMatrix3fv(loc, 1, ffi::FALSE, m.as_ptr());
        }
    }
}

// ── Shaders ─────────────────────────────────────────────────────────
//
// Compiled via the raw `link_program` helper, which does NOT inject a
// `#version` header — so unlike the `compile_custom_pixel_shader`
// shaders in `shadow.rs` / `rounded_border.rs`, these MUST declare
// `#version 100` themselves (matching smithay's own `texture.vert`).

const VERT_SRC: &str = r#"#version 100
precision highp float;
attribute vec2 a_pos;
attribute vec2 a_uv;
varying vec2 v_uv;
void main() {
    v_uv = a_uv;
    gl_Position = vec4(a_pos, 0.0, 1.0);
}
"#;

// Composite vertex shader: the quad's `a_uv` (0..1) is mapped into the
// destination rect in output-physical pixel space, then transformed by
// the frame's projection (which encodes the output transform) into clip
// space — so the blurred texture lands exactly on `dst` regardless of
// rotation. We carry `a_uv` through unchanged for texture sampling.
const COMPOSITE_VERT_SRC: &str = r#"#version 100
precision highp float;
attribute vec2 a_pos;
attribute vec2 a_uv;
uniform vec2 dst_origin;
uniform vec2 dst_size;
uniform mat3 projection;
varying vec2 v_uv;
void main() {
    v_uv = a_uv;
    vec2 px = dst_origin + a_uv * dst_size;
    vec3 clip = projection * vec3(px, 1.0);
    gl_Position = vec4(clip.xy, 0.0, 1.0);
}
"#;

// Dual-Kawase down-filter. `half_texel` is half a texel of the SOURCE
// texture; `radius` widens the sample footprint. `uv_scale` maps our
// [0,1] quad UVs onto the valid sub-rect of a (possibly larger) pooled
// source texture.
const DOWN_FRAG_SRC: &str = r#"#version 100
precision highp float;
uniform sampler2D tex;
uniform vec2 half_texel;
uniform float radius;
uniform vec2 uv_scale;
varying vec2 v_uv;
void main() {
    vec2 uv = v_uv * uv_scale;
    vec2 ht = half_texel * radius;
    vec4 sum = texture2D(tex, uv) * 4.0;
    sum += texture2D(tex, uv - ht);
    sum += texture2D(tex, uv + ht);
    sum += texture2D(tex, uv + vec2(ht.x, -ht.y));
    sum += texture2D(tex, uv - vec2(ht.x, -ht.y));
    gl_FragColor = sum / 8.0;
}
"#;

// Dual-Kawase up-filter (tent / 8-tap).
const UP_FRAG_SRC: &str = r#"#version 100
precision highp float;
uniform sampler2D tex;
uniform vec2 half_texel;
uniform float radius;
uniform vec2 uv_scale;
varying vec2 v_uv;
void main() {
    vec2 uv = v_uv * uv_scale;
    vec2 ht = half_texel * radius;
    vec4 sum = texture2D(tex, uv + vec2(-ht.x * 2.0, 0.0));
    sum += texture2D(tex, uv + vec2(-ht.x, ht.y)) * 2.0;
    sum += texture2D(tex, uv + vec2(0.0, ht.y * 2.0));
    sum += texture2D(tex, uv + vec2(ht.x, ht.y)) * 2.0;
    sum += texture2D(tex, uv + vec2(ht.x * 2.0, 0.0));
    sum += texture2D(tex, uv + vec2(ht.x, -ht.y)) * 2.0;
    sum += texture2D(tex, uv + vec2(0.0, -ht.y * 2.0));
    sum += texture2D(tex, uv + vec2(-ht.x, -ht.y)) * 2.0;
    gl_FragColor = sum / 12.0;
}
"#;

// Composite: position the quad via the output projection (mapping
// dst-pixel coords → clip space), sample the blurred texture, apply
// brightness / contrast / saturation / noise, and mask to the rounded
// rect. Output is premultiplied alpha (smithay's blend expects it).
const COMPOSITE_FRAG_SRC: &str = r#"#version 100
precision highp float;
uniform sampler2D tex;
uniform vec2 uv_scale;
uniform vec2 region_size;
uniform float corner_radius;
uniform float brightness;
uniform float contrast;
uniform float saturation;
uniform float noise;
varying vec2 v_uv;

float rounded_box_sdf(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + vec2(r);
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

float hash(vec2 p) {
    return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453);
}

void main() {
    // The blurred texture stores the region right-side-up in GL space
    // (row 0 = bottom, set by the flipped capture). The destination
    // quad runs top-left → bottom-right via the frame projection, so we
    // flip V here to put the blur back in the orientation of the
    // background it sits under.
    vec2 sample_uv = vec2(v_uv.x, 1.0 - v_uv.y) * uv_scale;
    vec3 c = texture2D(tex, sample_uv).rgb;

    // brightness
    c *= brightness;
    // contrast around 0.5
    c = (c - 0.5) * contrast + 0.5;
    // saturation
    float luma = dot(c, vec3(0.2126, 0.7152, 0.0722));
    c = mix(vec3(luma), c, saturation);
    // noise (dither to break up banding on flat blurs)
    if (noise > 0.0) {
        c += (hash(gl_FragCoord.xy) - 0.5) * noise;
    }
    c = clamp(c, 0.0, 1.0);

    // rounded-rect mask in region pixel space
    vec2 p = v_uv * region_size - region_size * 0.5;
    vec2 half_size = region_size * 0.5;
    float dist = rounded_box_sdf(p, half_size, corner_radius);
    float mask = 1.0 - smoothstep(-1.0, 1.0, dist);

    gl_FragColor = vec4(c * mask, mask);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

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
