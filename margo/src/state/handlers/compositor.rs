//! `wl_compositor` + `wl_subcompositor` handler — the surface-
//! commit hub every other protocol funnels through.
//!
//! Margo's `commit()` is the load-bearing seam where four
//! orthogonal state machines interlock:
//!
//! * **Session-lock priority.** Once `session_locked` is on, *any*
//!   commit on a lock surface re-runs `refresh_keyboard_focus`
//!   before returning. Qt's QtWayland plugin doesn't always wire
//!   `forceActiveFocus` on the QML TextInput until the
//!   QQuickWindow has both surface activation AND a paint event;
//!   re-issuing focus on first lock-surface commit is what flips
//!   the password field from "renders but is dead" to "accepts the
//!   first keystroke."
//! * **Deferred initial map (xdg-shell).** `new_toplevel` parks a
//!   `MargoClient` with `is_initial_map_pending = true` until the
//!   first commit lands here — at which point we read app_id /
//!   title, apply window rules, then map. Without the deferral
//!   CopyQ / Spotify / Helium would all flash through their
//!   default geometry before snapping to rule-driven placement.
//! * **Initial-configure pump.** Toplevel, layer, and xdg_popup
//!   surfaces all need an explicit `send_configure()` on first
//!   commit; smithay won't auto-fire it. The popup branch is the
//!   one that fixes the "GTK / Chromium right-click menu does
//!   nothing" symptom — without it the client commits an empty
//!   buffer and waits forever.
//! * **Layer-shell focus refresh.** Noctalia's bar / launcher /
//!   settings / control-center share a single per-screen MainScreen
//!   layer surface and mutate `WlrLayershell.keyboardFocus`
//!   between `Exclusive` and `None` instead of destroying the
//!   surface. Without `refresh_keyboard_focus` here, closing one
//!   of those panels with Esc leaves keyboard focus pinned to
//!   the (still-alive) layer in `None` mode and keystrokes go
//!   nowhere.
//!
//! `BufferHandler` is bundled here because smithay's surface-
//! buffer lifecycle is the natural sibling of `CompositorHandler`
//! — the trait is a single empty fn but it has to live somewhere.

use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    delegate_compositor,
    desktop::{layer_map_for_output, PopupKind, WindowSurface, WindowSurfaceType},
    reexports::wayland_server::{
        protocol::{wl_buffer::WlBuffer, wl_surface::WlSurface},
        Client, Resource,
    },
    wayland::{
        buffer::BufferHandler,
        compositor::{
            get_parent, is_sync_subsurface, with_states, CompositorClientState, CompositorHandler,
            CompositorState,
        },
        seat::WaylandFocus,
        shell::{
            wlr_layer::LayerSurfaceData,
            xdg::XdgToplevelSurfaceData,
        },
    },
    xwayland::XWaylandClientData,
};

use crate::state::{MargoClientData, MargoState};

impl CompositorHandler for MargoState {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }
    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        if let Some(state) = client.get_data::<XWaylandClientData>() {
            return &state.compositor_state;
        }
        if let Some(state) = client.get_data::<MargoClientData>() {
            return &state.compositor_state;
        }
        panic!("client_compositor_state: unknown client data type")
    }
    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }

            if self.session_locked
                && self
                    .lock_surfaces
                    .iter()
                    .any(|(_, s)| s.wl_surface() == &root)
            {
                tracing::info!(
                    "session_lock: commit on lock surface {:?}, surfaces total={}",
                    root.id(),
                    self.lock_surfaces.len()
                );
                // First commit on a lock surface = it's now mapped (has
                // a buffer attached). Run focus refresh AT THIS POINT
                // so `wl_keyboard.enter` lands on a fully-formed
                // surface — Qt's QtWayland plugin doesn't always wire
                // forceActiveFocus on the QML TextInput until the
                // QQuickWindow has received both surface activation
                // AND a paint event, so re-issuing focus once the
                // first buffer commits is what flips the password
                // field from "renders but is dead" to "accepts the
                // first keystroke." `refresh_keyboard_focus` also
                // makes sure the surface that gets focus is the one
                // on the cursor's output (not always the first
                // surface in `lock_surfaces`).
                self.refresh_keyboard_focus();
                self.request_repaint();
                return;
            }

            // First check if this commit belongs to a client we've
            // deferred (created in `new_toplevel`, not yet mapped
            // because we wanted to wait for app_id before applying
            // window rules). If so, finalise the initial map now.
            let deferred_idx = self.clients.iter().position(|c| {
                c.is_initial_map_pending
                    && c.window.wl_surface().as_deref() == Some(&root)
            });
            if let Some(idx) = deferred_idx {
                self.finalize_initial_map(idx);
            }

            let committed_window = self
                .space
                .elements()
                .find(|w| w.wl_surface().as_deref() == Some(&root))
                .cloned();
            if let Some(window) = committed_window {
                window.on_commit();
                // Send the initial configure on first commit if not yet sent.
                // xdg-shell clients perform an initial bufferless commit after
                // role assignment and then wait for this configure.
                if let WindowSurface::Wayland(toplevel) = window.underlying_surface() {
                    self.refresh_wayland_toplevel_identity(&window, toplevel);
                    let initial_sent = with_states(toplevel.wl_surface(), |states| {
                        states
                            .data_map
                            .get::<XdgToplevelSurfaceData>()
                            .and_then(|d| d.lock().ok().map(|d| d.initial_configure_sent))
                            .unwrap_or(false)
                    });
                    if !initial_sent {
                        tracing::debug!("sending initial configure for toplevel");
                        toplevel.send_configure();
                    } else {
                        tracing::trace!("commit on already-configured toplevel");
                    }
                }
                // Re-derive border geometry from the freshly-committed
                // window_geometry. Clients (notably Electron — Helium /
                // Spotify) sometimes commit at a smaller size than we
                // asked them to, and without this refresh the border
                // stays drawn around the larger layout-reserved rect,
                // leaving a wallpaper strip between the visible window
                // and its frame.
                crate::border::refresh(self);
            }

            let layer_output = self.space.outputs().find_map(|output| {
                let map = layer_map_for_output(output);
                if map.layer_for_surface(&root, WindowSurfaceType::TOPLEVEL).is_some() {
                    Some(output.clone())
                } else {
                    None
                }
            });

            if let Some(output) = layer_output {
                let initial_sent = with_states(&root, |states| {
                    states
                        .data_map
                        .get::<LayerSurfaceData>()
                        .and_then(|d| d.lock().ok().map(|d| d.initial_configure_sent))
                        .unwrap_or(false)
                });

                {
                    let mut map = layer_map_for_output(&output);
                    map.arrange();
                    if !initial_sent {
                        if let Some(layer) =
                            map.layer_for_surface(&root, WindowSurfaceType::TOPLEVEL)
                        {
                            tracing::debug!("sending initial configure for layer surface");
                            layer.layer_surface().send_configure();
                        }
                    }
                }

                self.refresh_output_work_area(&output);

                // A layer commit can flip `keyboard_interactivity` —
                // noctalia's bar / launcher / settings / control-center
                // all live on a single per-screen MainScreen layer and
                // mutate `WlrLayershell.keyboardFocus` between
                // `Exclusive` and `None` instead of destroying the
                // surface. Without recomputing focus here, closing one
                // of those panels with Esc leaves keyboard focus
                // pinned to the (still-alive) layer surface in `None`
                // mode — keys go nowhere until the user nudges the
                // mouse, which is exactly what made "rofi works but
                // the noctalia launcher does not" reproducible.
                self.refresh_keyboard_focus();
            }
        }
        // Initial configure for xdg_popups. Toplevel and layer surfaces
        // get their initial-configure pumped above; popups need the same
        // treatment or GTK / Chromium will sit forever waiting for an
        // ack and never attach a buffer — that's the "right-click menu
        // never opens" / "Helium 3-dot menu does nothing" / "Nemo
        // context menu invisible" symptom. Pattern lifted from anvil.
        if let Some(PopupKind::Xdg(xdg)) = self.popups.find_popup(surface) {
            if !xdg.is_initial_configure_sent() {
                if let Err(err) = xdg.send_configure() {
                    tracing::warn!(?err, "popup initial configure failed");
                } else {
                    tracing::debug!("sent initial configure for xdg_popup");
                }
            }
        }
        self.popups.commit(surface);
        self.request_repaint();
    }
}
delegate_compositor!(MargoState);

impl BufferHandler for MargoState {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}
