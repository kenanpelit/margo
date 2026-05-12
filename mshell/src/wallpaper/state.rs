//! Wallpaper renderer thread state.
//!
//! Owns a Wayland connection, binds `wl_compositor`, `wl_shm`,
//! `wlr_layer_shell_v1`, and `wl_output`. For each named output it
//! creates a Background-layer surface, waits for the compositor's
//! configure event, then on every `Command::Set` decodes the image
//! (or paints a solid fallback colour when `path = None`), writes
//! ARGB8888 into an `wl_shm` pool, and attach/commits.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{
            channel::{self, Event},
            EventLoop, LoopHandle,
        },
        calloop_wayland_source::WaylandSource,
        client::{
            globals::registry_queue_init,
            protocol::{wl_output::WlOutput, wl_shm::Format, wl_surface::WlSurface},
            Connection, QueueHandle,
        },
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};

use super::{Command, WallpaperFit};

pub fn run(rx: mpsc::Receiver<Command>) -> Result<()> {
    let conn = Connection::connect_to_env().context("connect to Wayland")?;
    let (globals, event_queue) =
        registry_queue_init::<State>(&conn).context("registry init")?;
    let qh: QueueHandle<State> = event_queue.handle();

    let event_loop: EventLoop<State> =
        EventLoop::try_new().context("calloop EventLoop")?;
    let loop_handle = event_loop.handle();

    // Bridge std::mpsc → calloop::channel so a single event loop
    // wakes on both Wayland events and incoming commands.
    let (cmd_tx, cmd_rx) = channel::channel::<Command>();
    std::thread::Builder::new()
        .name("mshell-wallpaper-bridge".to_owned())
        .spawn(move || {
            while let Ok(cmd) = rx.recv() {
                if cmd_tx.send(cmd).is_err() {
                    break;
                }
            }
        })?;
    loop_handle
        .insert_source(cmd_rx, |event, _, state| {
            if let Event::Msg(cmd) = event {
                state.handle_command(cmd);
            }
        })
        .map_err(|e| anyhow::anyhow!("calloop channel insert: {e}"))?;

    WaylandSource::new(conn.clone(), event_queue)
        .insert(loop_handle.clone())
        .map_err(|e| anyhow::anyhow!("WaylandSource insert: {e}"))?;

    let compositor_state =
        CompositorState::bind(&globals, &qh).context("bind wl_compositor")?;
    let layer_shell =
        LayerShell::bind(&globals, &qh).context("bind wlr-layer-shell")?;
    let shm = Shm::bind(&globals, &qh).context("bind wl_shm")?;

    let mut state = State {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        compositor_state,
        layer_shell,
        shm,
        outputs: Vec::new(),
        qh,
        conn,
        loop_handle: loop_handle.clone(),
        quit: false,
    };

    log::info!("wallpaper renderer thread started");

    let mut event_loop = event_loop;
    while !state.quit {
        event_loop
            .dispatch(Some(Duration::from_millis(250)), &mut state)
            .map_err(|e| anyhow::anyhow!("event loop dispatch: {e}"))?;
    }
    log::info!("wallpaper renderer thread exiting");
    Ok(())
}

// ── Per-output rendering state ─────────────────────────────────────────────

struct OutputEntry {
    wl_output: WlOutput,
    name: Option<String>,
    /// Output's logical size — useful as a pre-configure fallback
    /// so the layer surface gets created with reasonable dimensions
    /// before the compositor's first configure event.
    logical_size: Option<(u32, u32)>,
    surface: Option<LayerSurface>,
    /// Reusable shm pool sized for the current surface dimensions.
    pool: Option<SlotPool>,
    /// Last layered configure — when a new image comes in we
    /// render into a buffer of this exact size.
    configured_size: Option<(u32, u32)>,
    /// Pending render request. May arrive before `configured_size`
    /// is known; in that case we defer the render until the
    /// configure ack lands.
    desired: Option<RenderRequest>,
    /// Last committed render so we skip duplicate work.
    rendered: Option<RenderRequest>,
}

#[derive(Debug, Clone, PartialEq)]
struct RenderRequest {
    /// `None` → paint a solid `fallback_color` buffer (also the
    /// recovery path for missing-file / decode-error).
    path: Option<PathBuf>,
    fit: WallpaperFit,
    fallback_color: [u8; 3],
}

impl OutputEntry {
    fn ready(&self) -> bool {
        self.surface.is_some() && self.configured_size.is_some()
    }
}

pub struct State {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor_state: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    outputs: Vec<OutputEntry>,
    qh: QueueHandle<State>,
    /// Kept alive so the Wayland connection stays open while the
    /// event loop runs; never read directly.
    #[allow(dead_code)]
    conn: Connection,
    #[allow(dead_code)]
    loop_handle: LoopHandle<'static, State>,
    quit: bool,
}

impl State {
    fn handle_command(&mut self, cmd: Command) {
        match cmd {
            Command::Set {
                output_name,
                path,
                fit,
                fallback_color,
            } => {
                log::info!(
                    "wallpaper renderer: set output={} path={} fit={:?}",
                    output_name,
                    path.as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "<solid>".into()),
                    fit
                );
                if let Some(entry) = self
                    .outputs
                    .iter_mut()
                    .find(|o| o.name.as_deref() == Some(&output_name))
                {
                    entry.desired = Some(RenderRequest {
                        path,
                        fit,
                        fallback_color,
                    });
                    if entry.surface.is_none() {
                        Self::try_create_surface(
                            &self.layer_shell,
                            &self.compositor_state,
                            &self.qh,
                            entry,
                        );
                    }
                    Self::render_if_ready(&self.shm, entry);
                } else {
                    log::warn!(
                        "wallpaper renderer: no output named {} (have {})",
                        output_name,
                        self.outputs
                            .iter()
                            .filter_map(|o| o.name.clone())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
            }
            Command::Quit => {
                self.quit = true;
            }
        }
    }

    fn try_create_surface(
        layer_shell: &LayerShell,
        compositor_state: &CompositorState,
        qh: &QueueHandle<State>,
        entry: &mut OutputEntry,
    ) {
        if entry.surface.is_some() {
            return;
        }
        let Some(name) = entry.name.clone() else {
            return;
        };
        let surface = compositor_state.create_surface(qh);
        let layer = layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Background,
            Some(format!("mshell-wallpaper-{name}")),
            Some(&entry.wl_output),
        );
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        let (w, h) = entry.logical_size.unwrap_or((0, 0));
        layer.set_size(w, h);
        layer.commit();
        entry.surface = Some(layer);
        log::info!("wallpaper renderer: layer surface created for {name}");
    }

    fn render_if_ready(shm: &Shm, entry: &mut OutputEntry) {
        if !entry.ready() {
            return;
        }
        let Some(req) = entry.desired.clone() else {
            return;
        };
        if entry.rendered.as_ref() == Some(&req) {
            return;
        }
        let (w, h) = entry.configured_size.expect("ready() guards this");
        if w == 0 || h == 0 {
            return;
        }
        if let Err(e) = paint(shm, entry, &req, w, h) {
            log::warn!(
                "wallpaper renderer: paint failed on {} req={:?}: {e:#}",
                entry.name.as_deref().unwrap_or("?"),
                req
            );
            // On failure fall back to a solid colour buffer so the
            // user doesn't stare at a black/transparent surface.
            let fallback_req = RenderRequest {
                path: None,
                fit: req.fit,
                fallback_color: req.fallback_color,
            };
            if let Err(e2) = paint(shm, entry, &fallback_req, w, h) {
                log::error!(
                    "wallpaper renderer: solid fallback also failed on {}: {e2:#}",
                    entry.name.as_deref().unwrap_or("?")
                );
            } else {
                entry.rendered = Some(fallback_req);
            }
        } else {
            entry.rendered = Some(req);
        }
    }
}

/// Single render entry-point — handles image and solid-colour
/// paths. Returns Ok only when a buffer was successfully attached
/// and committed.
fn paint(shm: &Shm, entry: &mut OutputEntry, req: &RenderRequest, w: u32, h: u32) -> Result<()> {
    let stride = (w * 4) as i32;
    let pool_size = (stride as u32 * h) as usize;

    // (Re)allocate the pool when the surface is resized.
    let pool = match entry.pool.as_mut() {
        Some(p) if p.len() >= pool_size => p,
        _ => {
            entry.pool = Some(
                SlotPool::new(pool_size.max(stride as usize * 8), shm)
                    .context("SlotPool::new")?,
            );
            entry.pool.as_mut().unwrap()
        }
    };

    let (buffer, canvas) = pool
        .create_buffer(w as i32, h as i32, stride, Format::Argb8888)
        .context("SlotPool::create_buffer")?;

    let [r, g, b] = req.fallback_color;
    // Fill the canvas with the solid fallback first. wl_shm
    // Argb8888 wire is little-endian BGRA in memory.
    for px in canvas.chunks_exact_mut(4) {
        px[0] = b;
        px[1] = g;
        px[2] = r;
        px[3] = 0xff;
    }

    let mut painted_image = false;
    if let Some(path) = req.path.as_ref() {
        match render_image(canvas, path, req.fit, w, h) {
            Ok(()) => painted_image = true,
            Err(e) => {
                log::warn!(
                    "wallpaper renderer: image render failed ({}): {e:#} — falling back to solid",
                    path.display()
                );
                // canvas still holds the solid fallback we wrote
                // above, so the buffer is well-defined.
            }
        }
    }

    let surface = entry.surface.as_ref().unwrap();
    let wl_surface: &WlSurface = surface.wl_surface();
    buffer
        .attach_to(wl_surface)
        .context("attach buffer to surface")?;
    wl_surface.damage_buffer(0, 0, w as i32, h as i32);
    wl_surface.commit();

    log::info!(
        "wallpaper renderer: {} on {} ({}x{})",
        if painted_image {
            format!("painted {}", req.path.as_ref().unwrap().display())
        } else {
            format!("painted solid #{:02x}{:02x}{:02x}", r, g, b)
        },
        entry.name.as_deref().unwrap_or("?"),
        w,
        h
    );
    Ok(())
}

/// Decode an image and blit it into `canvas` with the requested
/// fit mode. `canvas` is `(w * h * 4)` BGRA bytes, already
/// initialised with the fallback colour so letterboxed modes get
/// proper backing pixels for free.
fn render_image(
    canvas: &mut [u8],
    path: &std::path::Path,
    fit: WallpaperFit,
    w: u32,
    h: u32,
) -> Result<()> {
    use image::{imageops::FilterType, GenericImageView};

    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let img = image::load_from_memory(&bytes)
        .with_context(|| format!("decode {}", path.display()))?;
    let (iw, ih) = img.dimensions();

    let (target_w, target_h, off_x, off_y) = match fit {
        WallpaperFit::Cover => {
            // Scale so the image fully covers the output; centre-
            // crop the overflow.
            let scale = (w as f32 / iw as f32).max(h as f32 / ih as f32);
            let sw = ((iw as f32 * scale).ceil() as u32).max(w);
            let sh = ((ih as f32 * scale).ceil() as u32).max(h);
            let scaled = img.resize_exact(sw, sh, FilterType::Triangle);
            let off_x = (sw - w) / 2;
            let off_y = (sh - h) / 2;
            let cropped = scaled.crop_imm(off_x, off_y, w, h).to_rgba8();
            blit_centered(canvas, &cropped, w, h, 0, 0);
            return Ok(());
        }
        WallpaperFit::Contain => {
            // Scale so the image fits inside; letterbox with
            // fallback colour.
            let scale = (w as f32 / iw as f32).min(h as f32 / ih as f32);
            let sw = ((iw as f32 * scale).round() as u32).max(1);
            let sh = ((ih as f32 * scale).round() as u32).max(1);
            let scaled = img.resize_exact(sw, sh, FilterType::Triangle).to_rgba8();
            let off_x = (w.saturating_sub(sw)) / 2;
            let off_y = (h.saturating_sub(sh)) / 2;
            blit_centered(canvas, &scaled, w, h, off_x as i32, off_y as i32);
            return Ok(());
        }
        WallpaperFit::Fill => (w, h, 0, 0),
        WallpaperFit::None => {
            // 1:1, centred (or top-left if image is bigger than
            // surface — the simple crop_imm path).
            let off_x = (w.saturating_sub(iw)) / 2;
            let off_y = (h.saturating_sub(ih)) / 2;
            let cw = iw.min(w);
            let ch = ih.min(h);
            let crop_x = (iw.saturating_sub(w)) / 2;
            let crop_y = (ih.saturating_sub(h)) / 2;
            let cropped = img.crop_imm(crop_x, crop_y, cw, ch).to_rgba8();
            blit_centered(canvas, &cropped, w, h, off_x as i32, off_y as i32);
            return Ok(());
        }
    };

    let _ = (target_w, target_h, off_x, off_y);
    // Fill path — exact resize, full-surface blit.
    let scaled = img.resize_exact(w, h, FilterType::Triangle).to_rgba8();
    blit_centered(canvas, &scaled, w, h, 0, 0);
    Ok(())
}

/// Copy an RGBA `src` image into the BGRA `canvas` at offset
/// `(off_x, off_y)`. Pixels outside the canvas are clipped. Source
/// must be `image::RgbaImage` (raw `width * height * 4` RGBA bytes).
fn blit_centered(canvas: &mut [u8], src: &image::RgbaImage, w: u32, h: u32, off_x: i32, off_y: i32) {
    let (sw, sh) = src.dimensions();
    let src_raw = src.as_raw();
    for sy in 0..sh as i32 {
        let dy = sy + off_y;
        if dy < 0 || dy >= h as i32 {
            continue;
        }
        for sx in 0..sw as i32 {
            let dx = sx + off_x;
            if dx < 0 || dx >= w as i32 {
                continue;
            }
            let s_idx = (sy as usize * sw as usize + sx as usize) * 4;
            let d_idx = (dy as usize * w as usize + dx as usize) * 4;
            canvas[d_idx] = src_raw[s_idx + 2]; // B
            canvas[d_idx + 1] = src_raw[s_idx + 1]; // G
            canvas[d_idx + 2] = src_raw[s_idx]; // R
            canvas[d_idx + 3] = src_raw[s_idx + 3]; // A
        }
    }
}

// ── smithay-client-toolkit handler impls ──────────────────────────────────

impl CompositorHandler for State {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _new_transform: smithay_client_toolkit::reexports::client::protocol::wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _time: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _output: &WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _output: &WlOutput,
    ) {
    }
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        output: WlOutput,
    ) {
        let info = self.output_state.info(&output);
        let name = info.as_ref().and_then(|i| i.name.clone());
        let logical_size = info
            .as_ref()
            .and_then(|i| i.logical_size)
            .map(|(w, h)| (w as u32, h as u32));
        log::info!(
            "wallpaper renderer: new_output name={:?} logical_size={:?}",
            name,
            logical_size
        );
        let mut entry = OutputEntry {
            wl_output: output,
            name,
            logical_size,
            surface: None,
            pool: None,
            configured_size: None,
            desired: None,
            rendered: None,
        };
        Self::try_create_surface(&self.layer_shell, &self.compositor_state, qh, &mut entry);
        self.outputs.push(entry);
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        output: WlOutput,
    ) {
        let info = self.output_state.info(&output);
        let name = info.as_ref().and_then(|i| i.name.clone());
        let logical_size = info
            .as_ref()
            .and_then(|i| i.logical_size)
            .map(|(w, h)| (w as u32, h as u32));
        if let Some(entry) = self.outputs.iter_mut().find(|o| o.wl_output == output) {
            entry.name = name;
            entry.logical_size = logical_size;
            if entry.surface.is_none() {
                Self::try_create_surface(
                    &self.layer_shell,
                    &self.compositor_state,
                    qh,
                    entry,
                );
            }
        }
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: WlOutput,
    ) {
        self.outputs.retain(|o| o.wl_output != output);
    }
}

impl LayerShellHandler for State {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface) {
        self.outputs.retain(|o| o.surface.as_ref() != Some(layer));
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let (w, h) = configure.new_size;
        let Some(entry) = self
            .outputs
            .iter_mut()
            .find(|o| o.surface.as_ref() == Some(layer))
        else {
            return;
        };
        let (w, h) = (
            if w == 0 {
                entry.logical_size.map(|(w, _)| w).unwrap_or(0)
            } else {
                w
            },
            if h == 0 {
                entry.logical_size.map(|(_, h)| h).unwrap_or(0)
            } else {
                h
            },
        );
        if w == 0 || h == 0 {
            return;
        }
        if entry.configured_size != Some((w, h)) {
            entry.pool = None;
            entry.rendered = None;
        }
        entry.configured_size = Some((w, h));
        log::info!(
            "wallpaper renderer: configure {}x{} on {}",
            w,
            h,
            entry.name.as_deref().unwrap_or("?")
        );
        State::render_if_ready(&self.shm, entry);
    }
}

impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

delegate_compositor!(State);
delegate_output!(State);
delegate_shm!(State);
delegate_layer!(State);
delegate_registry!(State);
