//! `wlr-layer-shell-unstable-v1` handler — bar / notification /
//! launcher / OSD layers.
//!
//! Owns the open / close animation pipeline for layer surfaces
//! (slide / fade keyed off the same `OpenCloseKind` curves the
//! toplevel pipeline uses) AND the focus-restore dance that fires
//! when an exclusive-keyboard layer (rofi / noctalia launcher) is
//! destroyed: focus lands back on the monitor's `selected` toplevel
//! instead of the dead surface.

use smithay::{
    delegate_layer_shell,
    desktop::{layer_map_for_output, LayerSurface as DesktopLayerSurface},
    output::Output,
    reexports::wayland_server::{protocol::wl_output::WlOutput, Resource},
    wayland::shell::wlr_layer::{
        Layer, LayerSurface as WlrLayerSurface, WlrLayerShellHandler, WlrLayerShellState,
    },
};

use crate::{
    layout::Rect,
    state::{FocusTarget, LayerSurfaceAnim, MargoState},
};

impl WlrLayerShellHandler for MargoState {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }
    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        output: Option<WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        let smithay_output = output
            .as_ref()
            .and_then(Output::from_resource)
            .or_else(|| {
                self.monitors
                    .get(self.focused_monitor())
                    .map(|mon| mon.output.clone())
            })
            .or_else(|| self.space.outputs().next().cloned());

        let Some(smithay_output) = smithay_output else { return };

        let desktop_layer = DesktopLayerSurface::new(surface, namespace.clone());
        let wl_surface_clone = desktop_layer.wl_surface().clone();
        {
            let mut map = layer_map_for_output(&smithay_output);
            map.map_layer(&desktop_layer).unwrap();
            map.arrange();
        }
        // Push `preferred_buffer_scale` + `wp_fractional_scale_v1`
        // preferred-scale events to the new layer surface BEFORE the
        // client commits its first buffer. GTK4 4.20+ reads these
        // synchronously during initial roundtrip — without them it
        // commits at integer scale=1 and the compositor has to
        // rescale every frame, producing the per-state-poll bar
        // flicker we've been chasing. niri sends these too (see
        // `niri/src/handlers/layer_shell.rs` initial-configure
        // branch). Subsurfaces created later get their own events
        // via `CompositorHandler::new_subsurface`.
        let scale = smithay_output.current_scale();
        let transform = smithay_output.current_transform();
        crate::state::handlers::compositor::send_scale_transform(
            &wl_surface_clone,
            scale,
            transform,
        );
        self.refresh_output_work_area(&smithay_output);

        // Resolve layer-rule overrides for this namespace. Rules are
        // matched by regex against the `namespace` string the client
        // chose at layer-shell creation (e.g. `noctalia-osd`,
        // `rofi`, `screenshot`). Latest matching rule wins for the
        // animation-type override; any matching rule's `noanim:1`
        // disables open/close animations entirely.
        let matched_rules: Vec<&margo_config::LayerRule> = self
            .config
            .layer_rules
            .iter()
            .filter(|r| super::super::matches_layer_name(r, &namespace))
            .collect();
        let layer_no_anim = matched_rules.iter().any(|r| r.no_anim);
        let kind_str = matched_rules
            .iter()
            .rev()
            .find_map(|r| r.animation_type_open.clone())
            .unwrap_or_else(|| self.config.layer_animation_type_open.clone());

        if !layer_no_anim
            && self.config.animations
            && self.config.layer_animations
            && self.config.animation_duration_open > 0
        {
            let kind = crate::render::open_close::OpenCloseKind::parse(&kind_str);
            let now = crate::utils::now_ms();
            self.layer_animations.insert(
                wl_surface_clone.id(),
                LayerSurfaceAnim {
                    time_started: now,
                    duration: self.config.animation_duration_open,
                    progress: 0.0,
                    is_close: false,
                    texture: None,
                    capture_pending: false,
                    geom: Rect::default(),
                    kind,
                    source_surface: None,
                },
            );
            self.request_repaint();
        }

        tracing::info!(
            "new layer surface: namespace={namespace} output={} anim={}",
            smithay_output.name(),
            self.layer_animations.contains_key(&wl_surface_clone.id()),
        );
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        // Find the monitor index that owns this layer surface
        let mut found_mon: Option<usize> = None;
        for i in 0..self.monitors.len() {
            let output = self.monitors[i].output.clone();
            let found = {
                let map = layer_map_for_output(&output);
                let mut found_layer = false;
                for l in map.layers() {
                    if l.layer_surface() == &surface {
                        found_layer = true;
                        break;
                    }
                }
                found_layer
            };
            if found {
                found_mon = Some(i);
                break;
            }
        }

        let Some(mon_idx) = found_mon else {
            tracing::info!("layer surface destroyed (not found)");
            return;
        };

        let output = self.monitors[mon_idx].output.clone();

        // Collect layer to remove
        let layer = {
            let map = layer_map_for_output(&output);
            let mut result = None;
            for l in map.layers() {
                if l.layer_surface() == &surface {
                    result = Some(l.clone());
                    break;
                }
            }
            result
        };

        // Close animation: capture the layer's wl_surface tree to a
        // texture and push a `LayerSurfaceAnim` entry so the renderer
        // keeps painting it sliding/fading away after smithay's
        // `LayerMap::unmap_layer` removes it.
        let wl_surf = surface.wl_surface().clone();
        let namespace = layer
            .as_ref()
            .map(|l| l.namespace().to_string())
            .unwrap_or_default();
        let matched_rules: Vec<&margo_config::LayerRule> = self
            .config
            .layer_rules
            .iter()
            .filter(|r| super::super::matches_layer_name(r, &namespace))
            .collect();
        let layer_no_anim = matched_rules.iter().any(|r| r.no_anim);
        let kind_str = matched_rules
            .iter()
            .rev()
            .find_map(|r| r.animation_type_close.clone())
            .unwrap_or_else(|| self.config.layer_animation_type_close.clone());

        if !layer_no_anim
            && self.config.animations
            && self.config.layer_animations
            && self.config.animation_duration_close > 0
        {
            // Read geometry off the layer map BEFORE we unmap it.
            let geom = layer.as_ref().and_then(|l| {
                let map = layer_map_for_output(&output);
                map.layer_geometry(l).map(|g| Rect {
                    x: g.loc.x,
                    y: g.loc.y,
                    width: g.size.w,
                    height: g.size.h,
                })
            });
            if let Some(geom) = geom {
                let kind = crate::render::open_close::OpenCloseKind::parse(&kind_str);
                let now = crate::utils::now_ms();
                self.layer_animations.insert(
                    wl_surf.id(),
                    LayerSurfaceAnim {
                        time_started: now,
                        duration: self.config.animation_duration_close,
                        progress: 0.0,
                        is_close: true,
                        texture: None,
                        capture_pending: true,
                        geom,
                        kind,
                        source_surface: Some(wl_surf.clone()),
                    },
                );
                self.request_repaint();
            }
        }

        if let Some(layer) = layer {
            let mut map = layer_map_for_output(&output);
            map.unmap_layer(&layer);
            map.arrange();
        }

        // Drop the per-surface layout hash so a future re-mapped
        // layer with the same id doesn't get short-circuited on its
        // first commit. The HashMap is unbounded only in pathological
        // create-without-destroy patterns; one entry per mapped
        // layer in steady state.
        self.layer_layout_hashes.remove(&wl_surf.id());
        self.layer_kb_interactivity_hashes.remove(&wl_surf.id());

        self.refresh_output_work_area(&output);

        // Hand keyboard focus back to a real window when the layer that
        // had grabbed it (typically noctalia's launcher / settings panel
        // / control-center, all `keyboard-interactivity: exclusive`)
        // goes away. Without this, keyboard.current_focus is left
        // pointing at the just-destroyed surface and every key press
        // is delivered to nothing.
        let current_focus_was_layer = self
            .seat
            .get_keyboard()
            .and_then(|kb| kb.current_focus())
            .map(|f| match f {
                FocusTarget::LayerSurface(s) => s == surface,
                _ => false,
            })
            .unwrap_or(false);

        if current_focus_was_layer {
            let restore = self.monitors[mon_idx]
                .selected
                .filter(|&idx| {
                    idx < self.clients.len()
                        && self.clients[idx].is_visible_on(
                            mon_idx,
                            self.monitors[mon_idx].current_tagset(),
                        )
                })
                .or_else(|| {
                    let tagset = self.monitors[mon_idx].current_tagset();
                    self.clients
                        .iter()
                        .position(|c| c.monitor == mon_idx && c.is_visible_on(mon_idx, tagset))
                });

            match restore {
                Some(idx) => {
                    let window = self.clients[idx].window.clone();
                    self.monitors[mon_idx].selected = Some(idx);
                    self.focus_surface(Some(FocusTarget::Window(window)));
                }
                None => self.focus_surface(None),
            }
        }

        tracing::info!("layer surface destroyed");
    }
}
delegate_layer_shell!(MargoState);
