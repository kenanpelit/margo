//! The GL side of a blur: saving/restoring GL state around smithay, the
//! down/up sample passes over the mip chain, and compositing the result
//! onto the output.

use smithay::backend::renderer::gles::ffi;
use smithay::utils::{Physical, Rectangle};

use margo_config::BlurParams;

use super::{BLUR_DIAG_COMPOSITE, BLUR_DIAG_DONE, BlurGl, effective_passes, mip_size};

/// The GL state the blur passes touch, saved before and restored after,
/// so smithay's renderer keeps working.
pub(super) struct SavedGl {
    pub(super) fbo: ffi::types::GLuint,
    viewport: [i32; 4],
    prog: i32,
    tex: i32,
    array_buf: i32,
    active_tex: i32,
    blend: bool,
    scissor: bool,
    scissor_box: [i32; 4],
}

impl SavedGl {
    /// # Safety
    /// Current GL context.
    pub(super) unsafe fn save(gl: &ffi::Gles2) -> Self {
        let mut fbo = 0;
        let mut viewport = [0i32; 4];
        let mut prog = 0;
        let mut tex = 0;
        let mut array_buf = 0;
        let mut active_tex = 0;
        let mut scissor_box = [0i32; 4];
        // SAFETY: caller holds the GL context; every out-pointer is a live local.
        unsafe {
            gl.GetIntegerv(ffi::FRAMEBUFFER_BINDING, &mut fbo);
            gl.GetIntegerv(ffi::VIEWPORT, viewport.as_mut_ptr());
            gl.GetIntegerv(ffi::CURRENT_PROGRAM, &mut prog);
            gl.GetIntegerv(ffi::TEXTURE_BINDING_2D, &mut tex);
            gl.GetIntegerv(ffi::ARRAY_BUFFER_BINDING, &mut array_buf);
            gl.GetIntegerv(ffi::ACTIVE_TEXTURE, &mut active_tex);
            gl.GetIntegerv(ffi::SCISSOR_BOX, scissor_box.as_mut_ptr());
            Self {
                fbo: fbo as ffi::types::GLuint,
                viewport,
                prog,
                tex,
                array_buf,
                active_tex,
                blend: gl.IsEnabled(ffi::BLEND) == ffi::TRUE,
                scissor: gl.IsEnabled(ffi::SCISSOR_TEST) == ffi::TRUE,
                scissor_box,
            }
        }
    }

    /// # Safety
    /// Current GL context (the one `save` ran in).
    pub(super) unsafe fn restore(&self, gl: &ffi::Gles2) {
        // SAFETY: caller holds the GL context; the saved names came from it.
        unsafe {
            gl.UseProgram(self.prog as ffi::types::GLuint);
            gl.ActiveTexture(ffi::TEXTURE0);
            gl.BindTexture(ffi::TEXTURE_2D, self.tex as ffi::types::GLuint);
            gl.ActiveTexture(self.active_tex as ffi::types::GLenum);
            gl.BindBuffer(ffi::ARRAY_BUFFER, self.array_buf as ffi::types::GLuint);
            if self.blend {
                gl.Enable(ffi::BLEND);
            } else {
                gl.Disable(ffi::BLEND);
            }
            gl.Scissor(
                self.scissor_box[0],
                self.scissor_box[1],
                self.scissor_box[2],
                self.scissor_box[3],
            );
            if self.scissor {
                gl.Enable(ffi::SCISSOR_TEST);
            } else {
                gl.Disable(ffi::SCISSOR_TEST);
            }
            gl.BindFramebuffer(ffi::FRAMEBUFFER, self.fbo);
            gl.Viewport(
                self.viewport[0],
                self.viewport[1],
                self.viewport[2],
                self.viewport[3],
            );
        }
    }
}

/// Where the last (full-resolution) up pass of [`blur_backdrop`] writes.
#[derive(Clone, Copy)]
pub(super) enum BlurTarget {
    /// The shared pool's result slot, which may be larger than the region.
    Pool,
    /// A region-sized texture: a per-element [`Backdrop`].
    Texture(ffi::types::GLuint),
}

/// Capture the slice of `output_fbo` behind `dst` (GL window coordinates,
/// [`framebuffer_rect`]) and run the dual-Kawase chain over it. Returns the
/// blurred texture and the UV scale of its valid sub-rect, or `None` when
/// the region is too small for a single pass.
///
/// # Safety
/// Current GL context; `blur_gl` programs/VBO are valid. Changes GL state:
/// wrap in [`SavedGl::save`] / [`SavedGl::restore`].
pub(super) unsafe fn blur_backdrop(
    gl: &ffi::Gles2,
    blur_gl: &mut BlurGl,
    output_fbo: ffi::types::GLuint,
    dst: Rectangle<i32, Physical>,
    (region_w, region_h): (i32, i32),
    params: &BlurParams,
    target: BlurTarget,
) -> Option<(ffi::types::GLuint, (f32, f32))> {
    let passes = effective_passes(params.num_passes, region_w, region_h);
    if passes == 0 {
        return None;
    }
    blur_gl.last_use = Some(std::time::Instant::now());

    // SAFETY: caller holds the GL context (render path); all GL objects used were created in that context.
    unsafe {
        // ── Capture the background slice behind `dst` ────────────────
        // Slot 0 is the captured background at full region resolution.
        let (cap_tex, _cap_fbo) = blur_gl.pool_slot(gl, 0, region_w, region_h);
        gl.ActiveTexture(ffi::TEXTURE0);
        gl.BindTexture(ffi::TEXTURE_2D, cap_tex);
        // Copy from the currently-bound (output) read framebuffer.
        // `dst` is already in GL window coordinates (`framebuffer_rect`):
        // smithay's projection flips Y for OpenGL, so window row 0 is the
        // top row of the output buffer and no further flip is needed. The
        // captured texture's row 0 is window row `dst.loc.y`, and the
        // composite places texel rows the same way round.
        gl.BindFramebuffer(ffi::FRAMEBUFFER, output_fbo);
        gl.CopyTexSubImage2D(
            ffi::TEXTURE_2D,
            0,
            0,
            0,
            dst.loc.x,
            dst.loc.y,
            region_w,
            region_h,
        );

        // One-shot diagnostic (see BLUR_DIAG_DONE). Logs the first draw so
        // a "blur invisible" report can be triaged from the journal.
        if !BLUR_DIAG_DONE.swap(true, std::sync::atomic::Ordering::Relaxed) {
            let err = gl.GetError();
            tracing::warn!(
                target: "margo::render::blur",
                "BLUR-DIAG first capture: dst.loc=({},{}) region={}x{} passes={} capture_glerror=0x{:x}",
                dst.loc.x, dst.loc.y, region_w, region_h, passes, err,
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
                // The destination texture may be larger than the mip (a
                // pool slot fits the largest recent region); render into the [0,mw]×[0,mh]
                // sub-rect and sample that same sub-rect on the next pass.
                (mw as f32 / slot_w as f32, mh as f32 / slot_h as f32),
            );
            src_tex = tex;
            src_w = mw;
            src_h = mh;
        }

        // ── Upsample ─────────────────────────────────────────────────
        let mut result_uv = (1.0, 1.0);
        for level in (0..passes).rev() {
            let (mw, mh) = mip_size(region_w, region_h, level);
            // Level 0 of the up chain is the final blurred result. It goes
            // to `target`: a backdrop texture of exactly the region size,
            // or a dedicated pool "result" slot (index `passes + 1`) so it
            // never clobbers an intermediate the chain still needs.
            let (tex, fbo, slot_w, slot_h) = match (level, target) {
                (0, BlurTarget::Texture(tex)) => {
                    gl.BindFramebuffer(ffi::FRAMEBUFFER, blur_gl.cache_fbo);
                    gl.BindTexture(ffi::TEXTURE_2D, tex);
                    for (pname, value) in [
                        (ffi::TEXTURE_MIN_FILTER, ffi::LINEAR),
                        (ffi::TEXTURE_MAG_FILTER, ffi::LINEAR),
                        (ffi::TEXTURE_WRAP_S, ffi::CLAMP_TO_EDGE),
                        (ffi::TEXTURE_WRAP_T, ffi::CLAMP_TO_EDGE),
                    ] {
                        gl.TexParameteri(ffi::TEXTURE_2D, pname, value as i32);
                    }
                    gl.FramebufferTexture2D(
                        ffi::FRAMEBUFFER,
                        ffi::COLOR_ATTACHMENT0,
                        ffi::TEXTURE_2D,
                        tex,
                        0,
                    );
                    (tex, blur_gl.cache_fbo, mw, mh)
                }
                _ => {
                    let idx = if level == 0 {
                        (passes + 1) as usize
                    } else {
                        level as usize
                    };
                    let (tex, fbo) = blur_gl.pool_slot(gl, idx, mw, mh);
                    (tex, fbo, blur_gl.pool[idx].w, blur_gl.pool[idx].h)
                }
            };
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
            result_uv = (mw as f32 / slot_w as f32, mh as f32 / slot_h as f32);
        }
        // `src_tex` now holds the full-region blurred background in the
        // [0,region]×[0,region] sub-rect `result_uv` describes.
        Some((src_tex, result_uv))
    }
}

/// Composite a blurred backdrop onto the output framebuffer at `dst`,
/// clipped to the rounded rect and to the `clip` rects (GL window
/// coordinates).
///
/// # Safety
/// Current GL context; `saved` was taken in it before the blur passes.
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn composite_onto_output(
    gl: &ffi::Gles2,
    blur_gl: &BlurGl,
    saved: &SavedGl,
    blurred_tex: ffi::types::GLuint,
    uv_scale: (f32, f32),
    dst: Rectangle<i32, Physical>,
    (region_w, region_h): (i32, i32),
    corner_px: f32,
    params: &BlurParams,
    clip: &[Rectangle<i32, Physical>],
) {
    let vp = saved.viewport;
    // SAFETY: caller holds the GL context; the programs/VBO belong to it.
    unsafe {
        gl.BindFramebuffer(ffi::FRAMEBUFFER, saved.fbo);
        gl.Viewport(vp[0], vp[1], vp[2], vp[3]);
        gl.Enable(ffi::BLEND);
        gl.BlendFunc(ffi::ONE, ffi::ONE_MINUS_SRC_ALPHA);
        composite_pass(
            gl,
            blur_gl.composite_prog,
            blur_gl.quad_vbo,
            blurred_tex,
            uv_scale,
            dst,
            region_w,
            region_h,
            corner_px,
            params,
            vp,
            clip,
        );

        // One-shot diagnostic: GL error after the composite (the capture
        // diag runs before it, so this is the missing half).
        if !BLUR_DIAG_COMPOSITE.swap(true, std::sync::atomic::Ordering::Relaxed) {
            let err = gl.GetError();
            tracing::warn!(
                target: "margo::render::blur",
                "BLUR-DIAG composite: viewport=[{},{},{},{}] dst=({},{} {}x{}) composite_glerror=0x{:x}",
                vp[0], vp[1], vp[2], vp[3],
                dst.loc.x, dst.loc.y, dst.size.w, dst.size.h, err,
            );
        }
    }
}

/// Bind the quad VBO's vertex attributes for program `prog`. Returns
/// the two attribute locations so the caller can disable them after.
///
/// # Safety
/// Current GL context; `vbo` bound to `ARRAY_BUFFER`.
unsafe fn bind_quad_attribs(gl: &ffi::Gles2, prog: ffi::types::GLuint) -> (i32, i32) {
    // SAFETY: current GL context: called from `with_context` / a caller under the same contract (see the fn's `# Safety`).
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
    // SAFETY: current GL context: called from `with_context` / a caller under the same contract (see the fn's `# Safety`).
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
    // SAFETY: current GL context: called from `with_context` / a caller under the same contract (see the fn's `# Safety`).
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
/// colour-adjusted, with a touch of noise. Drawn once per `clip` rect
/// (scissored); leaves `SCISSOR_TEST` enabled.
///
/// # Safety
/// Current GL context.
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
    viewport: [i32; 4],
    clip: &[Rectangle<i32, Physical>],
) {
    // SAFETY: current GL context: called from `with_context` / a caller under the same contract (see the fn's `# Safety`).
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
        // Viewport (x, y, w, h) so the vertex shader maps dst-pixels → NDC
        // directly, bypassing smithay's projection-matrix convention.
        set_uniform_4f(
            gl,
            prog,
            c"viewport",
            viewport[0] as f32,
            viewport[1] as f32,
            viewport[2] as f32,
            viewport[3] as f32,
        );

        gl.Enable(ffi::SCISSOR_TEST);
        for r in clip {
            gl.Scissor(r.loc.x, r.loc.y, r.size.w, r.size.h);
            gl.DrawArrays(ffi::TRIANGLES, 0, 6);
        }
        disable_quad_attribs(gl, locs);
    }
}

// ── Uniform helpers ─────────────────────────────────────────────────

/// # Safety: current GL context.
unsafe fn set_uniform_1i(gl: &ffi::Gles2, prog: ffi::types::GLuint, name: &std::ffi::CStr, v: i32) {
    // SAFETY: current GL context: called from `with_context` / a caller under the same contract (see the fn's `# Safety`).
    unsafe {
        let loc = gl.GetUniformLocation(prog, name.as_ptr() as *const _);
        if loc >= 0 {
            gl.Uniform1i(loc, v);
        }
    }
}

/// # Safety: current GL context.
unsafe fn set_uniform_1f(gl: &ffi::Gles2, prog: ffi::types::GLuint, name: &std::ffi::CStr, v: f32) {
    // SAFETY: current GL context: called from `with_context` / a caller under the same contract (see the fn's `# Safety`).
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
    // SAFETY: current GL context: called from `with_context` / a caller under the same contract (see the fn's `# Safety`).
    unsafe {
        let loc = gl.GetUniformLocation(prog, name.as_ptr() as *const _);
        if loc >= 0 {
            gl.Uniform2f(loc, a, b);
        }
    }
}

/// # Safety: current GL context.
unsafe fn set_uniform_4f(
    gl: &ffi::Gles2,
    prog: ffi::types::GLuint,
    name: &std::ffi::CStr,
    a: f32,
    b: f32,
    c: f32,
    d: f32,
) {
    // SAFETY: current GL context: called from `with_context` / a caller under the same contract (see the fn's `# Safety`).
    unsafe {
        let loc = gl.GetUniformLocation(prog, name.as_ptr() as *const _);
        if loc >= 0 {
            gl.Uniform4f(loc, a, b, c, d);
        }
    }
}
