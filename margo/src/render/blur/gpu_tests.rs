use super::*;
use smithay::backend::allocator::Fourcc;
use smithay::backend::allocator::gbm::GbmDevice;
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::backend::renderer::{Bind, Color32F, ExportMem, Frame, Offscreen, Renderer};
use smithay::utils::Scale;

use margo_config::BlurParams;

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
fn reference(renderer: &mut GlesRenderer, ids: &Ids, shot: Shot, transform: Transform) -> Vec<u8> {
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
