//! Wayland client bootstrap for the wallpaper engine: bind
//! `wl_compositor` + `zwlr_layer_shell_v1`, enumerate outputs, and create
//! a background layer surface per target output. Mirrors the raw
//! `wayland-client` pattern used by `mlock`.

use anyhow::{Result, anyhow, bail};
use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_compositor::WlCompositor, wl_output::WlOutput, wl_region::WlRegion, wl_registry,
    wl_surface::WlSurface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{self, Layer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, Anchor, ZwlrLayerSurfaceV1},
};

/// Per-output userdata: index into `PaperState::outputs`.
#[derive(Clone, Copy)]
pub struct OutputId(pub usize);
/// Per-surface userdata: index into `PaperState::surfaces`.
#[derive(Clone, Copy)]
pub struct SurfaceId(pub usize);

pub struct OutputInfo {
    pub wl_output: WlOutput,
    pub name: String,
}

pub struct PaperSurface {
    /// Output this surface lives on (kept for diagnostics).
    #[allow(dead_code)]
    pub output_name: String,
    pub wl_surface: WlSurface,
    /// Held to keep the layer surface alive for the engine's lifetime.
    #[allow(dead_code)]
    pub layer_surface: ZwlrLayerSurfaceV1,
    pub width: i32,
    pub height: i32,
    /// True once the first `configure` arrived (size known, acked).
    pub configured: bool,
}

pub struct PaperState {
    pub compositor: Option<WlCompositor>,
    pub layer_shell: Option<ZwlrLayerShellV1>,
    pub outputs: Vec<OutputInfo>,
    pub surfaces: Vec<PaperSurface>,
    /// Set once every created surface has been configured at least once.
    pub all_configured: bool,
}

impl PaperState {
    /// Connect, bind globals, enumerate outputs (names resolved via a
    /// roundtrip). Returns the live connection + queue for the run loop.
    pub fn connect() -> Result<(
        Connection,
        wayland_client::EventQueue<PaperState>,
        PaperState,
    )> {
        let conn =
            Connection::connect_to_env().map_err(|e| anyhow!("wayland connect failed: {e}"))?;
        let (globals, mut queue) = registry_queue_init::<PaperState>(&conn)
            .map_err(|e| anyhow!("registry init failed: {e}"))?;
        let qh = queue.handle();

        let mut state = PaperState {
            compositor: None,
            layer_shell: None,
            outputs: Vec::new(),
            surfaces: Vec::new(),
            all_configured: false,
        };
        state.bind_globals(&globals, &qh);

        if state.compositor.is_none() {
            bail!("compositor doesn't expose wl_compositor");
        }
        if state.layer_shell.is_none() {
            bail!("compositor doesn't expose zwlr_layer_shell_v1");
        }

        // Roundtrip so wl_output Name events arrive before we filter.
        queue.roundtrip(&mut state)?;
        Ok((conn, queue, state))
    }

    fn bind_globals(&mut self, globals: &GlobalList, qh: &QueueHandle<PaperState>) {
        let registry = globals.registry();
        for g in globals.contents().clone_list() {
            match g.interface.as_str() {
                "wl_compositor" if self.compositor.is_none() => {
                    let v = g.version.clamp(4, 6);
                    self.compositor = Some(registry.bind(g.name, v, qh, ()));
                }
                "zwlr_layer_shell_v1" if self.layer_shell.is_none() => {
                    let v = g.version.clamp(1, 4);
                    self.layer_shell = Some(registry.bind(g.name, v, qh, ()));
                }
                "wl_output" => {
                    let idx = self.outputs.len();
                    let v = g.version.clamp(1, 4);
                    let output = registry.bind(g.name, v, qh, OutputId(idx));
                    self.outputs.push(OutputInfo {
                        wl_output: output,
                        name: String::new(),
                    });
                }
                _ => {}
            }
        }
    }

    /// Create a background layer surface on each output matching `target`
    /// (all, when `None`). Caller must roundtrip afterwards to get sizes.
    pub fn create_surfaces(&mut self, target: Option<&str>, qh: &QueueHandle<PaperState>) {
        let compositor = self.compositor.clone().expect("compositor bound");
        let layer_shell = self.layer_shell.clone().expect("layer_shell bound");
        let picks: Vec<(WlOutput, String)> = self
            .outputs
            .iter()
            .filter(|o| target.is_none_or(|t| o.name == t))
            .map(|o| (o.wl_output.clone(), o.name.clone()))
            .collect();

        for (wl_output, name) in picks {
            let idx = self.surfaces.len();
            let wl_surface = compositor.create_surface(qh, ());
            let layer_surface = layer_shell.get_layer_surface(
                &wl_surface,
                Some(&wl_output),
                Layer::Background,
                "mplay".to_string(),
                qh,
                SurfaceId(idx),
            );
            layer_surface.set_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right);
            layer_surface.set_exclusive_zone(-1);
            layer_surface
                .set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::None);
            // Empty input region — clicks pass through to the desktop.
            let region: WlRegion = compositor.create_region(qh, ());
            wl_surface.set_input_region(Some(&region));
            region.destroy();
            wl_surface.commit();

            self.surfaces.push(PaperSurface {
                output_name: name,
                wl_surface,
                layer_surface,
                width: 0,
                height: 0,
                configured: false,
            });
        }
    }
}

// ── Dispatch impls ─────────────────────────────────────────

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for PaperState {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlCompositor, ()> for PaperState {
    fn event(
        _: &mut Self,
        _: &WlCompositor,
        _: <WlCompositor as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlRegion, ()> for PaperState {
    fn event(
        _: &mut Self,
        _: &WlRegion,
        _: <WlRegion as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSurface, ()> for PaperState {
    fn event(
        _: &mut Self,
        _: &WlSurface,
        _: <WlSurface as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrLayerShellV1, ()> for PaperState {
    fn event(
        _: &mut Self,
        _: &ZwlrLayerShellV1,
        _: zwlr_layer_shell_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlOutput, OutputId> for PaperState {
    fn event(
        state: &mut Self,
        _: &WlOutput,
        event: <WlOutput as Proxy>::Event,
        id: &OutputId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wayland_client::protocol::wl_output::Event::Name { name } = event
            && let Some(o) = state.outputs.get_mut(id.0)
        {
            o.name = name;
        }
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, SurfaceId> for PaperState {
    fn event(
        state: &mut Self,
        layer_surface: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        id: &SurfaceId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                layer_surface.ack_configure(serial);
                if let Some(s) = state.surfaces.get_mut(id.0) {
                    s.width = width as i32;
                    s.height = height as i32;
                    s.configured = true;
                }
                state.all_configured = state.surfaces.iter().all(|s| s.configured);
            }
            zwlr_layer_surface_v1::Event::Closed => {
                // Output went away; mark it unconfigured so the loop can exit.
                if let Some(s) = state.surfaces.get_mut(id.0) {
                    s.configured = false;
                }
            }
            _ => {}
        }
    }
}
