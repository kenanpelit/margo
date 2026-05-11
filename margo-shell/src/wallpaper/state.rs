//! Wallpaper renderer thread state.
//!
//! Owns a Wayland connection, binds `wl_compositor`, `wl_shm`,
//! `wlr_layer_shell_v1`, and `wl_output`. For each named output it
//! creates a Background-layer surface, waits for the compositor's
//! configure event, then on every `Command::Set` decodes the image,
//! writes ARGB8888 into an `wl_shm` pool, and attach/commits.

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

use super::Command;

pub fn run(rx: mpsc::Receiver<Command>) -> Result<()> {
    let conn = Connection::connect_to_env().context("connect to Wayland")?;
    let (globals, event_queue) =
        registry_queue_init::<State>(&conn).context("registry init")?;
    let qh: QueueHandle<State> = event_queue.handle();

    let event_loop: EventLoop<State> =
        EventLoop::try_new().context("calloop EventLoop")?;
    let loop_handle = event_loop.handle();

    // Bridge the mpsc::Receiver into a calloop::channel so the
    // event loop wakes both for Wayland events and incoming commands.
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
    /// Output's logical size (set from `OutputHandler::update_output`
    /// once we see the wl_output `geometry`/`mode`/`scale` events
    /// settle into a `done`).
    logical_size: Option<(u32, u32)>,
    surface: Option<LayerSurface>,
    /// Reusable shm pool sized for the current surface dimensions.
    pool: Option<SlotPool>,
    /// Last layered configure (committed size) — when a new image
    /// comes in we render into a buffer of this exact size.
    configured_size: Option<(u32, u32)>,
    /// Path the user *wants* on this output. May arrive before
    /// `configured_size` is known; in that case we defer the render
    /// until the configure ack lands.
    desired_path: Option<PathBuf>,
    /// Path actually committed in the current buffer, so we skip
    /// repeat decodes for the same `Command::Set`.
    rendered_path: Option<PathBuf>,
}

impl OutputEntry {
    /// Whether the renderable surface is fully ready (layer surface
    /// created + first configure done).
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
            Command::Set { output_name, path } => {
                log::info!(
                    "wallpaper renderer: set output={} path={}",
                    output_name,
                    path.display()
                );
                if let Some(entry) = self
                    .outputs
                    .iter_mut()
                    .find(|o| o.name.as_deref() == Some(&output_name))
                {
                    entry.desired_path = Some(path);
                    if entry.surface.is_none() {
                        // Try to create the surface now — needs the
                        // output to have its name + size set; if not
                        // ready yet, the next update_output() call
                        // will retry.
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
        // Initial size hint — the compositor's configure will give
        // us the real one. Use the output's logical size if known,
        // otherwise (0, 0) tells the compositor to size us itself.
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
        let Some(path) = entry.desired_path.clone() else {
            return;
        };
        if entry.rendered_path.as_ref() == Some(&path) {
            return; // already showing this image
        }
        let (w, h) = entry.configured_size.expect("ready() guards this");
        if w == 0 || h == 0 {
            return;
        }
        if let Err(e) = paint_image(shm, entry, &path, w, h) {
            log::warn!(
                "wallpaper renderer: paint failed for {} on {}: {e:#}",
                path.display(),
                entry.name.as_deref().unwrap_or("?")
            );
        } else {
            entry.rendered_path = Some(path);
        }
    }
}

/// Decode the image, scale-fit to (w, h) with Cover semantics, blit
/// into an shm-backed `wl_buffer`, attach + damage + commit.
fn paint_image(
    shm: &Shm,
    entry: &mut OutputEntry,
    path: &std::path::Path,
    w: u32,
    h: u32,
) -> Result<()> {
    use image::{imageops::FilterType, GenericImageView};

    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let img = image::load_from_memory(&bytes)
        .with_context(|| format!("decode {}", path.display()))?;
    let (iw, ih) = img.dimensions();

    // Cover fit: scale so the image fully covers the output rect,
    // then centre-crop. Matches the default for shell wallpapers.
    let scale = (w as f32 / iw as f32).max(h as f32 / ih as f32);
    let sw = ((iw as f32 * scale).ceil() as u32).max(w);
    let sh = ((ih as f32 * scale).ceil() as u32).max(h);
    let scaled = img.resize_exact(sw, sh, FilterType::Triangle);
    let off_x = (sw - w) / 2;
    let off_y = (sh - h) / 2;
    let cropped = scaled.crop_imm(off_x, off_y, w, h).to_rgba8();

    // Pool / buffer. wl_shm Argb8888 is little-endian B G R A in
    // memory; the `image` crate hands us RGBA so we swizzle.
    let stride = (w * 4) as i32;
    let pool_size = (stride as u32 * h) as usize;

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

    debug_assert_eq!(canvas.len(), cropped.as_raw().len());
    // RGBA → BGRA swap (Argb8888 wire is BGRA in little-endian).
    for (dst, src) in canvas.chunks_exact_mut(4).zip(cropped.as_raw().chunks_exact(4)) {
        dst[0] = src[2]; // B
        dst[1] = src[1]; // G
        dst[2] = src[0]; // R
        dst[3] = src[3]; // A
    }

    let surface = entry.surface.as_ref().unwrap();
    let wl_surface: &WlSurface = surface.wl_surface();
    buffer
        .attach_to(wl_surface)
        .context("attach buffer to surface")?;
    wl_surface.damage_buffer(0, 0, w as i32, h as i32);
    wl_surface.commit();
    log::info!(
        "wallpaper renderer: painted {} ({}x{}) on {}",
        path.display(),
        w,
        h,
        entry.name.as_deref().unwrap_or("?")
    );
    Ok(())
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
            desired_path: None,
            rendered_path: None,
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
        // wlr-layer-shell ack happens inside smithay-client-toolkit's
        // LayerSurface; we just record the size and (re-)render.
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
        // Resize invalidates the pool — drop it so the next paint
        // allocates a correctly-sized one.
        if entry.configured_size != Some((w, h)) {
            entry.pool = None;
            entry.rendered_path = None;
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
