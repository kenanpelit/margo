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

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{
    GlesError, GlesFrame, GlesRenderer, GlesTexture, ffi, link_program,
};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::backend::renderer::{Frame, FrameContext, Offscreen};
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Logical, Physical, Rectangle, Scale, Size, Transform};

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
    /// FBO the last up pass renders through into a per-element
    /// [`Backdrop`] texture (attached per capture).
    cache_fbo: ffi::types::GLuint,
}

struct PoolTex {
    tex: ffi::types::GLuint,
    fbo: ffi::types::GLuint,
    w: i32,
    h: i32,
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
        quad_vbo,
        cache_fbo,
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
            });
        }
        let slot = &mut self.pool[idx];
        if slot.w < w || slot.h < h {
            let nw = slot.w.max(w);
            let nh = slot.h.max(h);
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

/// The blurred backdrop of one blur element, kept between frames in the
/// damage tracker's per-element effect cache (the `cache` smithay hands to
/// [`RenderElement::capture_framebuffer`] and [`RenderElement::draw`]).
///
/// It is a pure function of what lies behind the element, so it stays
/// valid for every swapchain buffer until the tracker reports a change
/// under the element and calls `capture_framebuffer` again.
struct Backdrop {
    /// Exactly region-sized, so the composite samples it with UV scale 1.
    tex: GlesTexture,
    size: (i32, i32),
    /// The last capture produced a blurred image (false when the region
    /// is too small for one pass or the programs are missing).
    ready: bool,
}

type BackdropCell = RefCell<Option<Backdrop>>;

/// Commit counter for a blur element's look: equal inputs give equal
/// counters, so a stable-id blur reports damage (and gets re-captured)
/// exactly when a reload changed its parameters or its corner radius.
pub fn blur_commit(params: &BlurParams, corner_radius: f32) -> CommitCounter {
    let words = [
        params.num_passes as u32,
        params.radius as u32,
        params.noise.to_bits(),
        params.brightness.to_bits(),
        params.contrast.to_bits(),
        params.saturation.to_bits(),
        corner_radius.to_bits(),
    ];
    CommitCounter::from(words.iter().fold(0usize, |acc, w| {
        acc.wrapping_mul(31).wrapping_add(*w as usize)
    }))
}

/// The parts of `fb_rect` that `damage` (element-relative rects in output
/// space, as smithay passes them to `draw`) covers, in the same GL window
/// coordinates as `fb_rect`.
pub fn damage_in_framebuffer(
    dst: Rectangle<i32, Physical>,
    damage: &[Rectangle<i32, Physical>],
    transform: Transform,
    fb_size: Size<i32, Physical>,
    fb_rect: Rectangle<i32, Physical>,
) -> Vec<Rectangle<i32, Physical>> {
    damage
        .iter()
        .filter_map(|d| {
            framebuffer_rect(Rectangle::new(dst.loc + d.loc, d.size), transform, fb_size)
                .intersection(fb_rect)
        })
        .collect()
}

/// The rounded-corner radius in framebuffer pixels for an element whose
/// own physical size is `own` but which is drawn at `drawn` (smaller when a
/// mirror target shows it scaled by the letterbox fit).
pub fn drawn_corner_px(
    corner_radius: f32,
    scale: f64,
    own: Size<i32, Physical>,
    drawn: Size<i32, Physical>,
) -> f32 {
    let px = corner_radius * scale as f32;
    if own.w > 0 && drawn.w > 0 {
        px * drawn.w as f32 / own.w as f32
    } else {
        px
    }
}

/// A blur draw under one translucent surface.
///
/// A smithay *framebuffer effect*: the damage tracker calls
/// [`RenderElement::capture_framebuffer`] whenever something behind the
/// element changed this frame, and then redraws everything behind the
/// whole element rect first, so the capture always reads a clean, current
/// backdrop. The blurred result is cached per element and output; a frame
/// that only changes what lies *above* the blur (a caret, the window's own
/// content) composites the cached image over just the damaged rects. That
/// keeps age-based damage tracking (and the swapchain) intact while blur is
/// on, and a static blurred screen renders nothing.
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
            commit: blur_commit(&params, corner_radius),
        }
    }

    /// Where `dst` lands in the bound framebuffer, and the region size.
    fn framebuffer_region(
        frame: &GlesFrame<'_, '_>,
        dst: Rectangle<i32, Physical>,
    ) -> (
        Transform,
        Size<i32, Physical>,
        Rectangle<i32, Physical>,
        (i32, i32),
    ) {
        // `draw_blur` addresses the framebuffer directly: CopyTexSubImage2D
        // for the capture and a quad placed in GL window coordinates for the
        // composite. `dst` is in output space; map it to where smithay's
        // projection put this element in the framebuffer (on a rotated or
        // flipped output those differ). The blur is computed in framebuffer
        // orientation, which is fine: the Kawase filter and the rounded-
        // rect mask are both symmetric under the 8 output transforms.
        let transform = frame.transformation();
        let fb_size = transform.transform_size(frame.output_size());
        let fb_rect = framebuffer_rect(dst, transform, fb_size);
        let region = (fb_rect.size.w.max(1), fb_rect.size.h.max(1));
        (transform, fb_size, fb_rect, region)
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
        // The blur output is a pure function of the framebuffer content
        // *within our own geometry* (the capture copies exactly that
        // slice). Changes behind us are the damage tracker's job: as a
        // framebuffer effect it re-captures us whenever damage below
        // overlaps our rect. Our own damage is only a changed look (blur
        // parameters / corner radius on `mctl reload`, see `blur_commit`);
        // geometry changes are caught by the tracker's per-id diff.
        if commit != Some(self.commit) {
            DamageSet::from_slice(&[Rectangle::new(
                Default::default(),
                self.geometry(scale).size,
            )])
        } else {
            DamageSet::default()
        }
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

    fn is_framebuffer_effect(&self) -> bool {
        true
    }
}

impl RenderElement<GlesRenderer> for BlurRenderElement {
    fn capture_framebuffer(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        cache: &UserDataMap,
    ) -> Result<(), GlesError> {
        let (_, _, fb_rect, region) = Self::framebuffer_region(frame, dst);
        let cell = cache.get_or_insert::<BackdropCell, _>(|| RefCell::new(None));
        let mut backdrop = cell.borrow_mut();
        let stale = backdrop.as_ref().is_none_or(|b| b.size != region);
        if stale {
            let tex: GlesTexture = Offscreen::<GlesTexture>::create_buffer(
                frame.renderer().as_mut(),
                Fourcc::Abgr8888,
                Size::<i32, Buffer>::from(region),
            )?;
            *backdrop = Some(Backdrop {
                tex,
                size: region,
                ready: false,
            });
        }
        let Some(backdrop) = backdrop.as_mut() else {
            return Ok(());
        };
        let target = backdrop.tex.tex_id();
        let params = self.params;
        // SAFETY: `with_context` makes the frame's GL context current for the closure.
        backdrop.ready = frame.with_context(|gl| unsafe {
            CACHED.with(|slot| {
                let mut borrow = slot.borrow_mut();
                let Some(blur_gl) = borrow.ready_mut() else {
                    return false;
                };
                let saved = SavedGl::save(gl);
                let blurred = blur_backdrop(
                    gl,
                    blur_gl,
                    saved.fbo,
                    fb_rect,
                    region,
                    &params,
                    BlurTarget::Texture(target),
                );
                saved.restore(gl);
                blurred.is_some()
            })
        })?;
        Ok(())
    }

    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        let (transform, fb_size, fb_rect, region) = Self::framebuffer_region(frame, dst);
        // Only the damaged rects may change: outside them the buffer
        // already holds this exact composite (the tracker's contract), and
        // re-blending the rounded corners there would darken them frame by
        // frame.
        let clip = damage_in_framebuffer(dst, damage, transform, fb_size, fb_rect);
        if clip.is_empty() {
            return Ok(());
        }
        let corner_px = drawn_corner_px(
            self.corner_radius,
            self.scale.x,
            self.geometry(self.scale).size,
            dst.size,
        );
        let params = self.params;
        // The backdrop the tracker had us capture. Without one (drawn by a
        // caller that renders everything from scratch, not through a
        // damage tracker) the framebuffer behind us is fresh: capture now.
        let cached = cache
            .and_then(|c| c.get::<BackdropCell>())
            .and_then(|cell| {
                cell.borrow()
                    .as_ref()
                    .filter(|b| b.ready && b.size == region)
                    .map(|b| b.tex.tex_id())
            });

        // SAFETY: `with_context` makes the frame's GL context current for the closure.
        frame.with_context(|gl| unsafe {
            CACHED.with(|slot| {
                let mut borrow = slot.borrow_mut();
                let Some(blur_gl) = borrow.ready_mut() else {
                    return;
                };
                let saved = SavedGl::save(gl);
                let blurred = match cached {
                    Some(tex) => Some((tex, (1.0, 1.0))),
                    None => blur_backdrop(
                        gl,
                        blur_gl,
                        saved.fbo,
                        fb_rect,
                        region,
                        &params,
                        BlurTarget::Pool,
                    ),
                };
                if let Some((tex, uv)) = blurred {
                    composite_onto_output(
                        gl, blur_gl, &saved, tex, uv, fb_rect, region, corner_px, &params, &clip,
                    );
                }
                saved.restore(gl);
            });
        })?;
        Ok(())
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

/// The GL state the blur passes touch, saved before and restored after,
/// so smithay's renderer keeps working.
struct SavedGl {
    fbo: ffi::types::GLuint,
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
    unsafe fn save(gl: &ffi::Gles2) -> Self {
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
    unsafe fn restore(&self, gl: &ffi::Gles2) {
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
enum BlurTarget {
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
unsafe fn blur_backdrop(
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
unsafe fn composite_onto_output(
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

// Composite vertex shader: the quad's `a_uv` (0..1) is mapped onto the
// destination rect, given in GL window coordinates of the framebuffer
// (`framebuffer_rect`, which already accounts for the output transform and
// smithay's Y flip), then into NDC against the viewport. `a_uv` is carried
// through unchanged for texture sampling.
const COMPOSITE_VERT_SRC: &str = r#"#version 100
precision highp float;
attribute vec2 a_pos;
attribute vec2 a_uv;
uniform vec2 dst_origin;
uniform vec2 dst_size;
uniform vec4 viewport;   // x, y, w, h in framebuffer pixels
varying vec2 v_uv;
void main() {
    v_uv = a_uv;
    vec2 px = dst_origin + a_uv * dst_size;
    vec2 ndc = 2.0 * (px - viewport.xy) / viewport.zw - 1.0;
    gl_Position = vec4(ndc, 0.0, 1.0);
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

// Composite: sample the blurred texture over the quad placed by
// COMPOSITE_VERT_SRC, apply
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
    // Texel row 0 of the blurred texture is window row `dst_origin.y`
    // (the capture copies window rows in order), and `v_uv.y = 0` sits
    // on that same row, so no flip.
    vec2 sample_uv = v_uv * uv_scale;
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
    fn corner_radius_follows_the_drawn_size() {
        let own: Size<i32, Physical> = (400, 300).into();
        // Drawn at its own size: logical radius times the output scale.
        assert_eq!(drawn_corner_px(8.0, 1.5, own, own), 12.0);
        // A mirror target showing it at half size: half the radius, so
        // the blur's corners stay under the (also halved) window corners.
        assert_eq!(drawn_corner_px(8.0, 1.5, own, (200, 150).into()), 6.0);
        // Degenerate sizes never divide by zero.
        assert_eq!(drawn_corner_px(8.0, 1.0, (0, 0).into(), (0, 0).into()), 8.0);
    }

    #[test]
    fn blur_commit_follows_every_look_input() {
        let p = BlurParams {
            num_passes: 2,
            radius: 5,
            noise: 0.02,
            brightness: 0.9,
            contrast: 0.9,
            saturation: 1.2,
        };
        let c = blur_commit(&p, 8.0);
        // Same look on the next frame: same counter, no damage.
        assert_eq!(c, blur_commit(&p, 8.0));
        let changed = [
            BlurParams { num_passes: 3, ..p },
            BlurParams { radius: 6, ..p },
            BlurParams { noise: 0.03, ..p },
            BlurParams {
                brightness: 1.0,
                ..p
            },
            BlurParams { contrast: 1.0, ..p },
            BlurParams {
                saturation: 1.0,
                ..p
            },
        ];
        for q in changed {
            assert_ne!(c, blur_commit(&q, 8.0), "{q:?}");
        }
        assert_ne!(c, blur_commit(&p, 0.0));
    }

    #[test]
    fn damage_maps_into_the_framebuffer_and_stays_inside_the_blur() {
        let fb: Size<i32, Physical> = (1920, 1080).into();
        let dst = Rectangle::<i32, Physical>::new((100, 50).into(), (300, 200).into());
        // A caret 10 px in, plus a rect hanging past the element's edge.
        let damage = [
            Rectangle::new((10, 20).into(), (4, 16).into()),
            Rectangle::new((290, 190).into(), (40, 40).into()),
        ];
        let fb_rect = framebuffer_rect(dst, Transform::Normal, fb);
        assert_eq!(
            damage_in_framebuffer(dst, &damage, Transform::Normal, fb, fb_rect),
            vec![
                Rectangle::new((110, 70).into(), (4, 16).into()),
                Rectangle::new((390, 240).into(), (10, 10).into()),
            ]
        );
        // Upside down: the caret moves with the element.
        let fb_rect = framebuffer_rect(dst, Transform::_180, fb);
        let got = damage_in_framebuffer(dst, &damage[..1], Transform::_180, fb, fb_rect);
        assert_eq!(
            got,
            vec![Rectangle::new(
                (1920 - 114, 1080 - 86).into(),
                (4, 16).into()
            )]
        );
        // Nothing to draw when nothing is damaged.
        assert!(damage_in_framebuffer(dst, &[], Transform::Normal, fb, fb_rect).is_empty());
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
pub(crate) mod gpu_tests {
    use super::*;
    use smithay::backend::allocator::Fourcc;
    use smithay::backend::allocator::gbm::GbmDevice;
    use smithay::backend::egl::{EGLContext, EGLDisplay};
    use smithay::backend::renderer::{Bind, Color32F, ExportMem, Offscreen, Renderer};

    const ALL: [Transform; 8] = [
        Transform::Normal,
        Transform::_90,
        Transform::_180,
        Transform::_270,
        Transform::Flipped,
        Transform::Flipped90,
        Transform::Flipped180,
        Transform::Flipped270,
    ];

    /// A GLES renderer on the first render node, or `None` (no GPU/EGL).
    pub(crate) fn renderer() -> Option<GlesRenderer> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/dri/renderD128")
            .ok()?;
        let fd = smithay::backend::drm::DrmDeviceFd::new(std::os::fd::OwnedFd::from(file).into());
        let gbm = GbmDevice::new(fd).ok()?;
        // SAFETY: `gbm` owns the open render node and is moved into the display.
        let display = unsafe { EGLDisplay::new(gbm) }.ok()?;
        let context = EGLContext::new(&display).ok()?;
        // SAFETY: the context was just created on this thread and is not shared.
        unsafe { GlesRenderer::new(context) }.ok()
    }

    /// Render `fb_size` (framebuffer / mode size) with the frame transform
    /// `transform`: blue clear, red at `dst` (output space), then
    /// optionally the blur element over `dst`. Returns RGBA rows in GL
    /// window order (row 0 = window y 0).
    fn render(
        renderer: &mut GlesRenderer,
        fb_size: Size<i32, Physical>,
        transform: Transform,
        dst: Rectangle<i32, Physical>,
        blur: bool,
    ) -> Vec<u8> {
        let mut tex: smithay::backend::renderer::gles::GlesTexture = renderer
            .create_buffer(Fourcc::Abgr8888, (fb_size.w, fb_size.h).into())
            .expect("offscreen buffer");
        let mut fb = renderer.bind(&mut tex).expect("bind");
        {
            let mut frame = renderer
                .render(&mut fb, fb_size, transform)
                .expect("render");
            let out_size = transform.transform_size(fb_size);
            frame
                .clear(
                    Color32F::new(0.0, 0.0, 1.0, 1.0),
                    &[Rectangle::from_size(out_size)],
                )
                .expect("clear");
            frame
                .draw_solid(
                    dst,
                    &[Rectangle::from_size(dst.size)],
                    Color32F::new(1.0, 0.0, 0.0, 1.0),
                )
                .expect("solid");
            if blur {
                let params = BlurParams {
                    num_passes: 2,
                    radius: 2,
                    noise: 0.0,
                    brightness: 0.5,
                    contrast: 1.0,
                    saturation: 1.0,
                };
                let element = BlurRenderElement::new(
                    Id::new(),
                    Rectangle::new(
                        (dst.loc.x, dst.loc.y).into(),
                        (dst.size.w, dst.size.h).into(),
                    ),
                    0.0,
                    params,
                    Scale::from(1.0),
                );
                element
                    .draw(
                        &mut frame,
                        element.src(),
                        dst,
                        &[Rectangle::from_size(dst.size)],
                        &[],
                        None,
                    )
                    .expect("blur draw");
            }
            let _ = frame.finish().expect("finish");
        }
        let mapping = renderer
            .copy_framebuffer(
                &fb,
                Rectangle::from_size((fb_size.w, fb_size.h).into()),
                Fourcc::Abgr8888,
            )
            .expect("copy");
        renderer.map_texture(&mapping).expect("map").to_vec()
    }

    smithay::backend::renderer::element::render_elements! {
        Scene<=GlesRenderer>;
        Solid=smithay::backend::renderer::element::solid::SolidColorRenderElement,
        Blur=BlurRenderElement,
    }

    /// One frame of a small desktop: a background, a square that can move
    /// under the blurred window, the blur, the translucent window over it and
    /// a caret on top of the window (damages a small part of the blur rect
    /// from above).
    #[derive(Clone, Copy)]
    struct Shot {
        square: (i32, i32),
        caret: [f32; 4],
        window: [f32; 4],
        blur: Option<BlurParams>,
    }

    struct Ids {
        bg: Id,
        square: Id,
        blur: Id,
        window: Id,
        caret: Id,
    }

    const FB: (i32, i32) = (96, 64);
    const BLUR_RECT: ((i32, i32), (i32, i32)) = ((16, 12), (48, 32));

    fn params(passes: i32) -> BlurParams {
        BlurParams {
            num_passes: passes,
            radius: 3,
            noise: 0.0,
            brightness: 0.8,
            contrast: 1.0,
            saturation: 1.0,
        }
    }

    fn colour_commit(c: [f32; 4]) -> CommitCounter {
        CommitCounter::from(c.iter().fold(0usize, |a, v| {
            a.wrapping_mul(31).wrapping_add(v.to_bits() as usize)
        }))
    }

    fn scene(ids: &Ids, shot: Shot) -> Vec<Scene> {
        use smithay::backend::renderer::element::solid::SolidColorRenderElement as S;
        let solid = |id: &Id, loc: (i32, i32), size: (i32, i32), c: [f32; 4]| {
            Scene::Solid(S::new(
                id.clone(),
                Rectangle::new(loc.into(), size.into()),
                colour_commit(c),
                Color32F::new(c[0], c[1], c[2], c[3]),
                Kind::Unspecified,
            ))
        };
        let (bl, bs) = BLUR_RECT;
        let mut out = vec![
            solid(&ids.caret, (30, 20), (3, 6), shot.caret),
            solid(&ids.window, bl, bs, shot.window),
        ];
        if let Some(p) = shot.blur {
            out.push(Scene::Blur(BlurRenderElement::new(
                ids.blur.clone(),
                Rectangle::new(bl.into(), bs.into()),
                6.0,
                p,
                Scale::from(1.0),
            )));
        }
        out.push(solid(
            &ids.square,
            shot.square,
            (10, 10),
            [0.9, 0.9, 0.1, 1.0],
        ));
        out.push(solid(&ids.bg, (0, 0), FB, [0.1, 0.2, 0.7, 1.0]));
        out
    }

    fn pixels(renderer: &mut GlesRenderer, tex: &mut GlesTexture) -> Vec<u8> {
        let fb = renderer.bind(tex).expect("bind");
        let mapping = renderer
            .copy_framebuffer(&fb, Rectangle::from_size(FB.into()), Fourcc::Abgr8888)
            .expect("copy");
        renderer.map_texture(&mapping).expect("map").to_vec()
    }

    /// The scene drawn from scratch (fresh tracker, age 0): what every
    /// frame must look like.
    fn reference(
        renderer: &mut GlesRenderer,
        ids: &Ids,
        shot: Shot,
        transform: Transform,
    ) -> Vec<u8> {
        let mut tex: GlesTexture = renderer
            .create_buffer(Fourcc::Abgr8888, FB.into())
            .expect("buffer");
        let mut tracker = OutputDamageTracker::new(FB, 1.0, transform);
        {
            let mut fb = renderer.bind(&mut tex).expect("bind");
            tracker
                .render_output(
                    renderer,
                    &mut fb,
                    0,
                    &scene(ids, shot),
                    Color32F::new(0.0, 0.0, 0.0, 1.0),
                )
                .expect("render");
        }
        pixels(renderer, &mut tex)
    }

    fn max_diff(a: &[u8], b: &[u8]) -> (u8, usize) {
        let mut worst = (0u8, 0usize);
        for (i, (x, y)) in a.iter().zip(b).enumerate() {
            let d = x.abs_diff(*y);
            if d > worst.0 {
                worst = (d, i / 4);
            }
        }
        worst
    }

    use smithay::backend::renderer::damage::OutputDamageTracker;
    use smithay::backend::renderer::gles::GlesTexture;

    /// Render `shots` through one damage tracker into a rotating 3-buffer
    /// swapchain with real buffer ages (what `DrmCompositor` does) and check
    /// every presented frame against a from-scratch render of that scene.
    /// Returns how many frames the tracker found nothing to redraw.
    fn run_swapchain(renderer: &mut GlesRenderer, shots: &[Shot], transform: Transform) -> usize {
        let ids = Ids {
            bg: Id::new(),
            square: Id::new(),
            blur: Id::new(),
            window: Id::new(),
            caret: Id::new(),
        };
        let mut bufs: Vec<GlesTexture> = (0..3)
            .map(|_| {
                renderer
                    .create_buffer(Fourcc::Abgr8888, FB.into())
                    .expect("buffer")
            })
            .collect();
        let mut last_used: [Option<usize>; 3] = [None; 3];
        let mut tracker = OutputDamageTracker::new(FB, 1.0, transform);
        let mut presented: Option<usize> = None;
        let mut frame_no = 0usize;
        let mut empty = 0;
        for (i, shot) in shots.iter().enumerate() {
            let slot = frame_no % 3;
            let age = last_used[slot].map_or(0, |u| frame_no - u);
            let damaged = {
                let mut fb = renderer.bind(&mut bufs[slot]).expect("bind");
                tracker
                    .render_output(
                        renderer,
                        &mut fb,
                        age,
                        &scene(&ids, *shot),
                        Color32F::new(0.0, 0.0, 0.0, 1.0),
                    )
                    .expect("render")
                    .damage
                    .is_some()
            };
            if damaged {
                last_used[slot] = Some(frame_no);
                presented = Some(slot);
                frame_no += 1;
            } else {
                empty += 1;
            }
            let Some(shown) = presented else { continue };
            let got = pixels(renderer, &mut bufs[shown]);
            let want = reference(renderer, &ids, *shot, transform);
            let (d, at) = max_diff(&got, &want);
            assert!(
                d <= 2,
                "{transform:?} frame {i}: pixel ({},{}) differs by {d} from a full redraw",
                at as i32 % FB.0,
                at as i32 / FB.0
            );
        }
        empty
    }

    fn base() -> Shot {
        Shot {
            square: (2, 2),
            caret: [0.0, 0.0, 0.0, 1.0],
            window: [0.05, 0.05, 0.05, 0.4],
            blur: Some(params(2)),
        }
    }

    /// The blur stays exact under age-based damage tracking: a square
    /// moving under it, a caret blinking over it, a change elsewhere, a
    /// reload that changes the blur parameters, and blur appearing on a
    /// screen that has been rendering without it. This is the invariant the
    /// old per-frame `reset_buffers()` bought with a full swapchain realloc.
    #[test]
    fn blur_matches_a_full_redraw_under_buffer_age_damage() {
        let Some(mut renderer) = renderer() else {
            eprintln!("skipped: no render node / EGL");
            return;
        };
        if shader(&mut renderer).is_none() {
            eprintln!("skipped: blur shaders did not compile");
            return;
        }
        let mut shots = Vec::new();
        // Blur enabled mid-session: frames without it first.
        for x in [2, 4] {
            shots.push(Shot {
                square: (x, 2),
                blur: None,
                ..base()
            });
        }
        // Square sliding in under the blurred window and back out.
        for x in [8, 14, 20, 26, 32, 40, 52, 60, 70] {
            shots.push(Shot {
                square: (x, 18),
                ..base()
            });
        }
        // Caret blinking on top: damage covers a sliver of the blur rect.
        for k in 0..6 {
            let on = k % 2 == 0;
            shots.push(Shot {
                square: (70, 18),
                caret: if on {
                    [0.9, 0.9, 0.9, 1.0]
                } else {
                    [0.0, 0.0, 0.0, 1.0]
                },
                ..base()
            });
        }
        // Something changes away from the blur only.
        for y in [50, 52, 54] {
            shots.push(Shot {
                square: (80, y),
                ..base()
            });
        }
        // `mctl reload` with a stronger blur, same element id.
        for _ in 0..4 {
            shots.push(Shot {
                square: (80, 54),
                blur: Some(params(3)),
                ..base()
            });
        }
        // On every output transform: the damage rects the composite is
        // clipped to go through the same framebuffer mapping as the blur.
        for t in ALL {
            run_swapchain(&mut renderer, &shots, t);
        }
    }

    /// A static screen with a blurred window renders nothing: the blur
    /// never forces a frame on its own.
    #[test]
    fn a_static_blurred_scene_lets_the_output_idle() {
        let Some(mut renderer) = renderer() else {
            eprintln!("skipped: no render node / EGL");
            return;
        };
        if shader(&mut renderer).is_none() {
            eprintln!("skipped: blur shaders did not compile");
            return;
        }
        // Fill all three buffers first (a never-used buffer has age 0 and
        // is always redrawn in full), then hold still. The tracker still
        // brings the older buffers up to date for a frame or two; after
        // that a blurred screen must go idle exactly like an unblurred one.
        let shots = |blur: Option<BlurParams>| {
            let mut shots: Vec<Shot> = [2, 4, 6]
                .into_iter()
                .map(|x| Shot {
                    square: (x, 2),
                    blur,
                    ..base()
                })
                .collect();
            shots.extend(std::iter::repeat_n(
                Shot {
                    square: (6, 2),
                    blur,
                    ..base()
                },
                6,
            ));
            shots
        };
        let n = Transform::Normal;
        let plain = run_swapchain(&mut renderer, &shots(None), n);
        assert!(plain >= 3, "the test scene itself must go idle: {plain}");
        assert_eq!(
            run_swapchain(&mut renderer, &shots(Some(params(2))), n),
            plain
        );
    }

    /// Blur at half brightness over a red rect on blue: on every output
    /// transform the blurred (darkened) red must cover exactly the rect
    /// smithay drew `dst` at, and nothing else may change. A capture or
    /// composite at the wrong framebuffer rect leaves bright red inside or
    /// darkens blue outside.
    #[test]
    fn blur_covers_exactly_the_drawn_rect_on_every_transform() {
        let Some(mut renderer) = renderer() else {
            eprintln!("skipped: no render node / EGL");
            return;
        };
        if shader(&mut renderer).is_none() {
            eprintln!("skipped: blur shaders did not compile");
            return;
        }
        // Non-square framebuffer and an off-centre rect, so every mirror
        // and rotation lands somewhere else.
        let fb_size: Size<i32, Physical> = (64, 40).into();
        let dst = Rectangle::new((3, 5).into(), (12, 8).into());
        for t in ALL {
            let fb_rect = framebuffer_rect(dst, t, fb_size);
            let px = render(&mut renderer, fb_size, t, dst, true);
            for y in 0..fb_size.h {
                for x in 0..fb_size.w {
                    let i = ((y * fb_size.w + x) * 4) as usize;
                    let (r, g, b) = (px[i], px[i + 1], px[i + 2]);
                    let inside = x >= fb_rect.loc.x
                        && x < fb_rect.loc.x + fb_rect.size.w
                        && y >= fb_rect.loc.y
                        && y < fb_rect.loc.y + fb_rect.size.h;
                    let edge = x == fb_rect.loc.x
                        || x == fb_rect.loc.x + fb_rect.size.w - 1
                        || y == fb_rect.loc.y
                        || y == fb_rect.loc.y + fb_rect.size.h - 1;
                    if !inside {
                        assert!(
                            b > 240 && r < 15,
                            "{t:?}: ({x},{y}) outside {fb_rect:?} changed to {r},{g},{b}"
                        );
                    } else if !edge {
                        assert!(
                            (100..=160).contains(&r) && g < 30 && b < 30,
                            "{t:?}: ({x},{y}) inside {fb_rect:?} is {r},{g},{b}, not blurred red"
                        );
                    }
                }
            }
        }
    }
}
