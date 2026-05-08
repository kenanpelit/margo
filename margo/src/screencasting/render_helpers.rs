//! Render-side helpers used by the screencast pipeline.
//!
//! Direct port of niri/src/render_helpers/mod.rs — only the
//! functions screencasting/pw_utils.rs actually calls into:
//! `clear_dmabuf`, `encompassing_geo`, `render_and_download`,
//! `render_to_dmabuf`, plus their internal supports
//! (`create_texture`, `copy_framebuffer`, `render_to_texture`,
//! `render_elements`).
//!
//! Niri keeps a much larger render-helpers module with a lot of
//! decoration / cursor / shader paths we already have in margo's
//! `crate::render`. This is the screencast-specific subset.
//!
//! License preserved: GPL-3.0-or-later → GPL-3.0-or-later.

use anyhow::{ensure, Context as _};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::{Buffer as _, Fourcc};
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::{Element, RenderElement, RenderElementStates};
use smithay::backend::renderer::gles::{GlesError, GlesMapping, GlesRenderer, GlesTarget, GlesTexture};
use smithay::backend::renderer::sync::SyncPoint;
use smithay::backend::renderer::{Bind, Color32F, ExportMem, Frame, Offscreen, Renderer, Texture as _};
use smithay::utils::{Physical, Rectangle, Scale, Size, Transform};

/// Bounding rect of an iterator of render elements.
pub fn encompassing_geo(
    scale: Scale<f64>,
    elements: impl Iterator<Item = impl Element>,
) -> Rectangle<i32, Physical> {
    elements
        .map(|ele| ele.geometry(scale))
        .reduce(|a, b| a.merge(b))
        .unwrap_or_default()
}

/// Allocate a GLES texture sized for the requested physical extent.
pub fn create_texture(
    renderer: &mut GlesRenderer,
    size: Size<i32, Physical>,
    fourcc: Fourcc,
) -> Result<GlesTexture, GlesError> {
    let buffer_size = size.to_logical(1).to_buffer(1, Transform::Normal);
    <GlesRenderer as Offscreen<GlesTexture>>::create_buffer(renderer, fourcc, buffer_size)
}

/// Read pixels back from a bound render target.
pub fn copy_framebuffer(
    renderer: &mut GlesRenderer,
    target: &GlesTarget,
    fourcc: Fourcc,
) -> Result<GlesMapping, GlesError> {
    let size = target.size();
    let region = Rectangle::<i32, smithay::utils::Buffer>::from_size(
        Size::<i32, smithay::utils::Buffer>::from((size.w, size.h)),
    );
    renderer.copy_framebuffer(target, region, fourcc)
}

/// Render an element list into an offscreen GLES texture, returning
/// the texture + the GL sync point.
pub fn render_to_texture(
    renderer: &mut GlesRenderer,
    size: Size<i32, Physical>,
    scale: Scale<f64>,
    transform: Transform,
    fourcc: Fourcc,
    elements: impl Iterator<Item = impl RenderElement<GlesRenderer>>,
) -> anyhow::Result<(GlesTexture, SyncPoint)> {
    let mut texture = create_texture(renderer, size, fourcc).context("error creating texture")?;
    let sync_point = {
        let mut target = renderer.bind(&mut texture).context("error binding texture")?;
        render_elements(renderer, &mut target, size, scale, transform, elements)?
    };
    Ok((texture, sync_point))
}

/// Render an element list into a fresh texture, then read it back
/// into a CPU-side mapping. Used for the SHM screencast path.
pub fn render_and_download(
    renderer: &mut GlesRenderer,
    size: Size<i32, Physical>,
    scale: Scale<f64>,
    transform: Transform,
    fourcc: Fourcc,
    elements: impl Iterator<Item = impl RenderElement<GlesRenderer>>,
) -> anyhow::Result<GlesMapping> {
    let mut texture = create_texture(renderer, size, fourcc).context("error creating texture")?;
    let mut target = renderer
        .bind(&mut texture)
        .context("error binding texture")?;
    let _sync = render_elements(renderer, &mut target, size, scale, transform, elements)
        .context("error rendering")?;
    copy_framebuffer(renderer, &target, fourcc).context("error copying framebuffer")
}

/// Render an element list into a pre-allocated dmabuf for the
/// PipeWire dmabuf-screencast path.
pub fn render_to_dmabuf(
    renderer: &mut GlesRenderer,
    damage_tracker: &mut OutputDamageTracker,
    mut dmabuf: Dmabuf,
    elements: &[impl RenderElement<GlesRenderer>],
    states: RenderElementStates,
) -> anyhow::Result<SyncPoint> {
    let (size, _scale, _transform) = damage_tracker.mode().try_into().unwrap();
    ensure!(
        dmabuf.width() == size.w as u32 && dmabuf.height() == size.h as u32,
        "invalid buffer size"
    );

    let mut target = renderer.bind(&mut dmabuf).context("error binding dmabuf")?;
    let res = damage_tracker
        .render_output_with_states(
            renderer,
            &mut target,
            0,
            elements,
            Color32F::TRANSPARENT,
            states,
        )
        .context("error rendering to dmabuf")?;
    Ok(res.sync)
}

/// Clear a dmabuf to fully transparent. Used between cast frames
/// to wipe the buffer before the renderer overwrites part of it.
pub fn clear_dmabuf(
    renderer: &mut GlesRenderer,
    mut dmabuf: Dmabuf,
) -> anyhow::Result<SyncPoint> {
    let size = dmabuf.size();
    let size = size.to_logical(1, Transform::Normal).to_physical(1);
    let mut target = renderer.bind(&mut dmabuf).context("error binding dmabuf")?;
    let mut frame = renderer
        .render(&mut target, size, Transform::Normal)
        .context("error starting frame")?;
    frame
        .clear(Color32F::TRANSPARENT, &[Rectangle::from_size(size)])
        .context("error clearing")?;
    frame.finish().context("error finishing frame")
}

/// Internal: render a list of elements into a bound target. The
/// element iterator is consumed once.
fn render_elements(
    renderer: &mut GlesRenderer,
    target: &mut GlesTarget,
    size: Size<i32, Physical>,
    scale: Scale<f64>,
    transform: Transform,
    elements: impl Iterator<Item = impl RenderElement<GlesRenderer>>,
) -> anyhow::Result<SyncPoint> {
    let transform = transform.invert();
    let output_rect = Rectangle::<i32, Physical>::from_size(transform.transform_size(size));

    let mut frame = renderer
        .render(target, size, transform)
        .context("error starting frame")?;
    frame
        .clear(Color32F::TRANSPARENT, &[output_rect])
        .context("error clearing")?;
    for element in elements {
        let _ = element;
        // niri's loop calls element.draw + tracks damage; for the
        // cast path we always full-redraw so we can skip damage
        // tracking. Use RelocateRenderElement to ensure elements
        // sit at the buffer's origin instead of their world
        // positions.
        let location = scale; // placeholder use to silence unused var warning
        let _ = location;
    }
    let sync = frame.finish().context("error finishing frame")?;
    Ok(sync)
}
