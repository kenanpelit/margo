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
    desktop::{
        PopupKind, WindowSurface, WindowSurfaceType, find_popup_root_surface, layer_map_for_output,
    },
    input::pointer::CursorImageStatus,
    reexports::{
        calloop::Interest,
        wayland_server::{
            Client, Resource,
            protocol::{wl_buffer::WlBuffer, wl_surface::WlSurface},
        },
    },
    wayland::{
        buffer::BufferHandler,
        compositor::{
            BufferAssignment, CompositorClientState, CompositorHandler, CompositorState,
            SurfaceAttributes, add_blocker, add_pre_commit_hook, get_parent, is_sync_subsurface,
            with_states,
        },
        dmabuf::get_dmabuf,
        seat::WaylandFocus,
        shell::{wlr_layer::LayerSurfaceData, xdg::XdgToplevelSurfaceData},
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

    /// Install a default dmabuf pre-commit hook on every new surface.
    ///
    /// Without this, every commit whose pending buffer is a DMA-BUF
    /// is applied IMMEDIATELY, even if the client hasn't yet finished
    /// the GPU work that produced the buffer. The compositor then
    /// samples the texture before its content fence has signalled,
    /// reads half-rendered pixels, and shows them to the user — the
    /// exact pattern that makes a gtk4-layer-shell bar
    /// (mshell-frame) flicker on margo while staying smooth on
    /// Hyprland and niri. Both of them install this hook.
    ///
    /// The hook produces a `dmabuf.generate_blocker(Interest::READ)`
    /// blocker on every DMA-BUF commit and feeds it to smithay's
    /// `add_blocker`, which delays the queued state until the
    /// blocker reports `Released`. The blocker's calloop source
    /// fires when the dmabuf's READ fence is signalled, at which
    /// point we call `CompositorClientState::blocker_cleared` to
    /// re-pump the surface's transaction queue. Net effect: commits
    /// land in render order with GPU-ready buffers, no torn texture
    /// sample, no flicker.
    ///
    /// Mirrors `niri/src/handlers/compositor.rs::add_default_dmabuf_pre_commit_hook`.
    /// When a subsurface is created, push the parent root's output
    /// scale / transform to it immediately. GTK4 layer-shell creates
    /// subsurfaces lazily as widgets render; without this hook the
    /// subsurface never receives `wl_surface.preferred_buffer_scale`
    /// or `wp_fractional_scale_v1.preferred_scale` until the next
    /// output mode change, so it commits at the wrong physical
    /// pixel pitch and the bar pixel grid drifts off the output
    /// grid — visible as per-state-poll micro-flicker. Mirrors
    /// `niri/src/handlers/compositor.rs:38-51`.
    fn new_subsurface(&mut self, surface: &WlSurface, parent: &WlSurface) {
        let mut root = parent.clone();
        while let Some(p) = get_parent(&root) {
            root = p;
        }
        if let Some(output) = self::output_for_root(self, &root) {
            let scale = output.current_scale();
            let transform = output.current_transform();
            send_scale_transform(surface, scale, transform);
        }
    }

    fn new_surface(&mut self, surface: &WlSurface) {
        if !surface.is_alive() {
            return;
        }
        let _hook = add_pre_commit_hook::<Self, _>(surface, |state, _dh, surface| {
            let maybe_dmabuf = with_states(surface, |surface_data| {
                surface_data
                    .cached_state
                    .get::<SurfaceAttributes>()
                    .pending()
                    .buffer
                    .as_ref()
                    .and_then(|assignment| match assignment {
                        BufferAssignment::NewBuffer(buffer) => get_dmabuf(buffer).cloned().ok(),
                        _ => None,
                    })
            });
            if let Some(dmabuf) = maybe_dmabuf {
                if let Ok((blocker, source)) = dmabuf.generate_blocker(Interest::READ) {
                    if let Some(client) = surface.client() {
                        let res = state.loop_handle.insert_source(source, move |_, _, state| {
                            let dh = state.display_handle.clone();
                            state
                                .client_compositor_state(&client)
                                .blocker_cleared(state, &dh);
                            Ok(())
                        });
                        if res.is_ok() {
                            add_blocker(surface, blocker);
                            tracing::trace!("added default dmabuf blocker");
                        }
                    }
                }
            }
        });
        // HookId is intentionally dropped: the hook lives on the
        // surface's private state and is cleaned up automatically
        // when the surface is destroyed, so we don't need to track
        // it for explicit removal. niri keeps a HashMap so it can
        // swap the default hook for a fancier mapped-toplevel hook
        // that also handles transactions; margo doesn't have
        // transactions yet, so we just install one hook per surface
        // and let surface destruction drop it.
        let _ = _hook;
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);

        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }

        // A synchronized child's pending state is applied atomically with its
        // parent commit. Repainting here is both premature and particularly
        // expensive for Chromium video trees (several child commits can make
        // up one frame). The eventual root commit performs all scene work and
        // schedules exactly one repaint.
        if is_sync_subsurface(surface) {
            return;
        }

        {
            let committed_lock = self.session_locked.then(|| {
                self.lock_surfaces
                    .iter()
                    .find(|(_, lock)| lock.wl_surface() == &root)
                    .map(|(output, lock)| (output.clone(), lock.clone()))
            });
            if let Some(Some((output, lock))) = committed_lock {
                if !Self::lock_surface_ready(&lock) {
                    self.refresh_keyboard_focus();
                    return;
                }
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
                self.request_repaint_output(&output);
                return;
            }

            // First check if this commit belongs to a client we've
            // deferred (created in `new_toplevel`, not yet mapped
            // because we wanted to wait for app_id before applying
            // window rules). If so, finalise the initial map now.
            let deferred_idx = self.clients.iter().position(|c| {
                c.is_initial_map_pending && c.window.wl_surface().as_deref() == Some(&root)
            });
            if let Some(idx) = deferred_idx.filter(|_| !self.session_locked) {
                self.finalize_initial_map(idx);
            }

            // Hidden-tag windows are deliberately unmapped from `Space`, but
            // their surface trees can still commit (especially after the
            // low-frequency frame callback fallback). Smithay requires
            // `Window::on_commit` for the toplevel and every desynchronised
            // child commit so its cached bbox stays correct. Resolve through
            // the authoritative client list first; the Space fallback covers
            // any auxiliary window not represented there.
            let committed_client_idx = self
                .clients
                .iter()
                .position(|client| client.window.wl_surface().as_deref() == Some(&root));
            let committed_window = committed_client_idx
                .and_then(|idx| self.clients.get(idx).map(|client| client.window.clone()))
                .or_else(|| {
                    self.space
                        .elements()
                        .find(|window| window.wl_surface().as_deref() == Some(&root))
                        .cloned()
                });
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
                // If a resize snapshot is in flight and this commit
                // brings the live buffer up to (≈) the target slot
                // size, the client has finished reflowing — drop the
                // snapshot now so the crisp live surface is revealed
                // immediately instead of waiting out the animation/grace
                // clock. This is the "whichever comes first" reveal that
                // keeps the border pinned to the slot through a grow:
                // without it, a client that reflows AFTER the move
                // animation ends would have its border collapse onto the
                // stale buffer for a frame (the super+r lag symptom).
                // Damage-driven renders don't run `tick_animations`, so
                // the commit handler is where this drop has to happen.
                if let Some(idx) = committed_client_idx {
                    if self.clients[idx].resize_snapshot.is_some() {
                        let target = if self.clients[idx].animation.running {
                            self.clients[idx].animation.current
                        } else {
                            self.clients[idx].geom
                        };
                        let actual = self.clients[idx].window.geometry().size;
                        const TOL: i32 = 4;
                        if (actual.w - target.width).abs() <= TOL
                            && (actual.h - target.height).abs() <= TOL
                        {
                            self.clients[idx].resize_snapshot = None;
                            self.clients[idx].snapshot_pending = false;
                        }
                    }
                }
                // Re-derive border geometry from the freshly-committed
                // window_geometry. Clients (notably Electron — Helium /
                // Spotify) sometimes commit at a smaller size than we
                // asked them to, and without this refresh the border
                // stays drawn around the larger layout-reserved rect,
                // leaving a wallpaper strip between the visible window
                // and its frame. Only the committing window's geometry
                // can have changed, so refresh just its border rather
                // than looping every client's `window.geometry()` lock.
                if let Some(idx) = committed_client_idx {
                    crate::border::refresh_one(self, idx);
                }
            }

            let layer_output = self.space.outputs().find_map(|output| {
                let map = layer_map_for_output(output);
                if map
                    .layer_for_surface(&root, WindowSurfaceType::TOPLEVEL)
                    .is_some()
                {
                    Some(output.clone())
                } else {
                    None
                }
            });

            if let Some(output) = layer_output {
                let layer_role = {
                    let map = layer_map_for_output(&output);
                    map.layer_for_surface(&root, WindowSurfaceType::TOPLEVEL)
                        .map(|layer| layer.layer())
                };
                if let Some(role) = layer_role
                    && let Some(animation) = self.layer_animations.get_mut(&root.id())
                {
                    animation.output = output.clone();
                    animation.layer = role;
                }
                let layer_visible =
                    layer_role.is_some_and(|role| self.layer_renders_on_output(&output, role));
                let initial_sent = with_states(&root, |states| {
                    states
                        .data_map
                        .get::<LayerSurfaceData>()
                        .and_then(|d| d.lock().ok().map(|d| d.initial_configure_sent))
                        .unwrap_or(false)
                });

                // Hash check #1: full layout state (size, anchor,
                // exclusive_zone, margin, layer). Drives arrange +
                // work_area recompute.
                //
                // Hash check #2 (split out): keyboard_interactivity
                // ONLY. Drives `refresh_keyboard_focus`. mshell's bar
                // updates content (clock, network speed, CPU) several
                // times per second; each content update can re-flow
                // the gtk4-layer-shell surface and produce a new
                // size/margin pair, which would tick the full hash
                // and uselessly re-run focus refresh at 3 Hz under a
                // bursty config. The 3 Hz focus churn is exactly
                // what the journal showed (refresh_keyboard_focus
                // ~330 ms apart) — niri's layer commit handler
                // doesn't call focus refresh at all (see
                // `niri/src/handlers/layer_shell.rs`), only
                // arrange + output_resized; we go one step softer
                // and refresh focus only on actual
                // keyboard_interactivity changes (noctalia/rofi
                // launcher flips `Exclusive <-> None`).
                let (new_layout_hash, new_kb_hash) = {
                    let layer = {
                        let map = layer_map_for_output(&output);
                        map.layer_for_surface(&root, WindowSurfaceType::TOPLEVEL)
                            .cloned()
                    };
                    match layer {
                        Some(layer) => {
                            use smithay::wayland::shell::wlr_layer::ExclusiveZone;
                            layer.layer_surface().with_cached_state(|cur| {
                                // ExclusiveZone / exclusive_edge aren't primitive;
                                // normalise to the (tag, value) shape the pure
                                // hasher takes, so the raw values are hashed (not
                                // their Debug strings — the old `format!("{:?}")`
                                // path heap-allocated a String per layer commit,
                                // and a mshell bar re-flows several times a second).
                                let ez: (u8, u32) = match cur.exclusive_zone {
                                    ExclusiveZone::Exclusive(z) => (0, z),
                                    ExclusiveZone::Neutral => (1, 0),
                                    ExclusiveZone::DontCare => (2, 0),
                                };
                                let edge: (u8, u32) = match cur.exclusive_edge {
                                    Some(a) => (1, a.bits()),
                                    None => (0, 0),
                                };
                                layer_commit_hashes(
                                    (cur.size.w, cur.size.h),
                                    cur.anchor.bits(),
                                    ez,
                                    edge,
                                    (
                                        cur.margin.top,
                                        cur.margin.bottom,
                                        cur.margin.left,
                                        cur.margin.right,
                                    ),
                                    cur.layer as u32,
                                    cur.keyboard_interactivity as u32,
                                )
                            })
                        }
                        // No layer surface for this root (shouldn't happen on a
                        // layer commit): fall back to the empty-hasher pair, the
                        // same constants the previous inline code produced.
                        None => layer_commit_hashes_empty(),
                    }
                };

                // Store the PURE layout hash (not a layout^kb combination).
                // The old XOR-combined hash meant a keyboard_interactivity
                // flip also changed this value → a spurious full arrange +
                // work-area recompute on every Exclusive↔None toggle. kb is
                // tracked in its own map below.
                let key = root.id();
                let prev_layout = self.layer_layout_hashes.get(&key).copied();
                let layout_changed =
                    prev_layout.map(|h| h != new_layout_hash).unwrap_or(true) || !initial_sent;
                if layout_changed {
                    self.layer_layout_hashes
                        .insert(key.clone(), new_layout_hash);
                }

                // Track keyboard_interactivity separately so we only
                // trigger focus refresh when it actually flips.
                let kb_changed = self
                    .layer_kb_interactivity_hashes
                    .get(&key)
                    .copied()
                    .map(|prev| prev != new_kb_hash)
                    .unwrap_or(true);
                if kb_changed {
                    self.layer_kb_interactivity_hashes
                        .insert(key.clone(), new_kb_hash);
                }

                if layout_changed {
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

                    if layer_visible {
                        self.refresh_output_work_area(&output);
                    } else {
                        self.update_output_work_area(&output);
                    }
                }

                // Independent of arrange: only refresh keyboard focus
                // when keyboard_interactivity flipped (Exclusive ↔
                // None / OnDemand). noctalia's launcher/settings
                // panels mutate this on the same surface instead of
                // destroying it, and without the refresh keystrokes
                // go nowhere; but mshell's bar never flips it during
                // normal updates, so we shouldn't be ticking focus
                // refresh on every content commit. First commit of a
                // layer surface always trips `kb_changed` (no prev
                // entry), so initial focus still resolves correctly.
                if kb_changed {
                    self.refresh_keyboard_focus();
                }
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
        match repaint_target_for_root(self, &root) {
            CommitRepaint::Output(output) => self.request_repaint_output(&output),
            CommitRepaint::Outputs(outputs) => {
                for output in outputs {
                    self.request_repaint_output(&output);
                }
            }
            CommitRepaint::None => {}
            // Unknown auxiliary roles (cursor, DnD icon, a popup whose parent
            // is not mapped yet) retain the conservative global fallback.
            CommitRepaint::All => self.request_repaint(),
        }
    }
}

impl BufferHandler for MargoState {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

/// `wp_fractional_scale_v1` — empty handler is enough; the
/// per-surface "preferred fractional scale" event is pushed by
/// [`crate::state::handlers::compositor::send_scale_transform`]
/// after we know which output the surface lives on. GTK4 4.20+
/// requires this protocol to commit at the output's actual
/// fractional scale; without it gtk4-layer-shell clients
/// (mshell-frame) render at an integer fallback scale and the bar
/// pixel-grid drifts off the output grid every state-poll cycle.
/// niri wires this the same way (`niri/src/handlers/mod.rs:845`).
impl smithay::wayland::fractional_scale::FractionalScaleHandler for MargoState {}

/// Push `wl_surface.preferred_buffer_scale` (integer) +
/// `wp_fractional_scale_v1.preferred_scale` (fractional) events to
/// a surface and all of its subsurfaces, matching the output the
/// surface is currently mapped to. Call after a surface is mapped
/// or whenever the output's scale changes. Mirrors niri's
/// `send_scale_transform` (`niri/src/utils/mod.rs:258`).
pub fn send_scale_transform(
    surface: &WlSurface,
    scale: smithay::output::Scale,
    transform: smithay::utils::Transform,
) {
    use smithay::wayland::compositor::{send_surface_state, with_states};
    use smithay::wayland::fractional_scale::with_fractional_scale;
    with_states(surface, |data| {
        send_surface_state(surface, data, scale.integer_scale(), transform);
        with_fractional_scale(data, |fractional| {
            fractional.set_preferred_scale(scale.fractional_scale());
        });
    });
}

/// Find the output that owns this root surface — walks the layer
/// map of every output looking for a layer whose root surface
/// matches, then falls back to `Space::outputs_for_element` for
/// toplevels. Returns `None` for cursor / DnD icon / freshly-
/// created surfaces that aren't yet mapped anywhere.
pub fn output_for_root(state: &MargoState, root: &WlSurface) -> Option<smithay::output::Output> {
    use smithay::desktop::{WindowSurfaceType, layer_map_for_output};
    use smithay::wayland::seat::WaylandFocus;
    for output in state.space.outputs() {
        let map = layer_map_for_output(output);
        if map
            .layer_for_surface(root, WindowSurfaceType::TOPLEVEL)
            .is_some()
        {
            return Some(output.clone());
        }
    }
    for window in state.space.elements() {
        if window.wl_surface().as_deref() == Some(root) {
            return state.space.outputs_for_element(window).into_iter().next();
        }
    }
    None
}

enum CommitRepaint {
    Output(smithay::output::Output),
    Outputs(Vec<smithay::output::Output>),
    None,
    All,
}

/// Resolve a committed root to the smallest safe physical repaint scope.
/// Hidden clients keep receiving the low-frequency frame-callback fallback,
/// but their commits do not dirty KMS until the tag is shown again. Freshly
/// mapped hidden clients are warmed by a separate callback-only timer; both
/// overview modes keep live off-tag content repainting.
fn repaint_target_for_root(state: &mut MargoState, root: &WlSurface) -> CommitRepaint {
    if let CursorImageStatus::Surface(cursor) = &state.cursor_status
        && cursor == root
    {
        return state
            .space
            .output_under((state.input_pointer.x, state.input_pointer.y))
            .next()
            .cloned()
            .map(CommitRepaint::Output)
            .unwrap_or(CommitRepaint::None);
    }

    // The locked render path emits only the lock surface and cursor. Lock
    // roots already take the exact-output early return in `commit`; every
    // other client/layer/auxiliary commit is invisible and must not drive an
    // empty physical frame behind the lock screen.
    if state.session_locked {
        return CommitRepaint::None;
    }

    // Popups are rendered as part of their owning toplevel/layer tree. Resolve
    // that owner before selecting a repaint scope so an animated GTK/Chromium
    // menu does not dirty every physical output.
    let popup_root = state
        .popups
        .find_popup(root)
        .and_then(|popup| find_popup_root_surface(&popup).ok());
    let target_root = popup_root.as_ref().unwrap_or(root);

    if let Some(client) = state
        .clients
        .iter()
        .find(|client| client.window.wl_surface().as_deref() == Some(target_root))
    {
        let Some(monitor) = state
            .monitors
            .get(client.monitor)
            .filter(|monitor| monitor.enabled)
        else {
            return CommitRepaint::None;
        };

        let tagset = if monitor.is_overview {
            !0
        } else {
            monitor.current_tagset()
        };
        let physically_visible = !client.is_initial_map_pending
            && !client.is_minimized
            && !client.is_killing
            && (client.is_visible_on(client.monitor, tagset)
                || (state.client_renders_on_output(client, &monitor.output)
                    && state.is_scroller_overview_open()));

        return if physically_visible {
            // Union three footprints: cached pre-commit Space membership
            // (erase a shrinking/detached buffer), the compositor slot
            // (scroller/floating geometry), and the freshly-updated surface
            // bbox (raw, unclipped buffers when rounded clipping is disabled).
            // This catches both old and new damage without a full
            // `Space::refresh` on every client commit.
            let mut outputs = state.space.outputs_for_element(&client.window);
            let membership_changed =
                state.window_space_membership_is_stale(&client.window, &outputs);
            outputs.retain(|output| {
                state
                    .monitors
                    .iter()
                    .any(|candidate| candidate.enabled && candidate.output == *output)
            });
            let geom = client.geom;
            let surface_bbox = state.space.element_bbox(&client.window);
            for candidate in state.monitors.iter().filter(|candidate| candidate.enabled) {
                let area = candidate.monitor_area;
                let slot_intersects = geom.x < area.x + area.width
                    && geom.x + geom.width > area.x
                    && geom.y < area.y + area.height
                    && geom.y + geom.height > area.y;
                let surface_intersects = surface_bbox.is_some_and(|bbox| {
                    bbox.loc.x < area.x + area.width
                        && bbox.loc.x + bbox.size.w > area.x
                        && bbox.loc.y < area.y + area.height
                        && bbox.loc.y + bbox.size.h > area.y
                });
                if (slot_intersects || surface_intersects) && !outputs.contains(&candidate.output) {
                    outputs.push(candidate.output.clone());
                }
            }
            if state.is_scroller_overview_open() && !outputs.contains(&monitor.output) {
                outputs.push(monitor.output.clone());
            }
            outputs.retain(|output| state.client_renders_on_output(client, output));
            // A commit can change a surface-tree bbox before Smithay updates
            // its cached output membership (Chromium does this while restoring
            // a hidden video tab). Refresh after retaining old∪new repaint
            // scope, but before either output can render the stale scene.
            if membership_changed {
                state.space.refresh();
            }
            match outputs.len() {
                0 => CommitRepaint::None,
                1 => CommitRepaint::Output(outputs.swap_remove(0)),
                _ => CommitRepaint::Outputs(outputs),
            }
        } else {
            CommitRepaint::None
        };
    }

    if let Some((output, _)) = state
        .lock_surfaces
        .iter()
        .find(|(_, lock)| lock.wl_surface() == target_root)
    {
        return CommitRepaint::Output(output.clone());
    }

    // Use the exact same layer-role visibility rule as rendering and frame
    // callbacks. Hidden Top/Overlay surfaces must stay off KMS, while the
    // Background/Bottom wallpaper embedded in scroller cells stays live.
    for output in state.space.outputs() {
        let map = layer_map_for_output(output);
        if let Some(layer) = map.layer_for_surface(target_root, WindowSurfaceType::TOPLEVEL) {
            if !state.layer_renders_on_output(output, layer.layer()) {
                return CommitRepaint::None;
            }
        }
    }

    output_for_root(state, target_root)
        .map(CommitRepaint::Output)
        .unwrap_or(CommitRepaint::All)
}

/// Hash a layer surface's committed state into a `(layout, keyboard)` pair.
///
/// The two hashers are deliberately separate: `keyboard_interactivity` feeds
/// only the second, so an `Exclusive`↔`None` toggle changes the kb hash but
/// leaves the layout hash untouched. That's what stops a bar/panel flipping
/// its keyboard grab from forcing a full re-arrange + work-area recompute
/// (the layout hash drives arrange; the kb hash drives focus refresh).
///
/// Primitive-typed on purpose — the caller normalises smithay's `Anchor` /
/// `ExclusiveZone` / `Layer` enums to their raw values, so this stays pure
/// and unit-testable without a live compositor.
#[allow(clippy::too_many_arguments)]
pub(crate) fn layer_commit_hashes(
    size: (i32, i32),
    anchor_bits: u32,
    exclusive_zone: (u8, u32),
    exclusive_edge: (u8, u32),
    margins: (i32, i32, i32, i32),
    layer: u32,
    keyboard_interactivity: u32,
) -> (u64, u64) {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut layout = DefaultHasher::new();
    let mut kb = DefaultHasher::new();
    size.hash(&mut layout);
    anchor_bits.hash(&mut layout);
    exclusive_zone.hash(&mut layout);
    exclusive_edge.hash(&mut layout);
    margins.hash(&mut layout);
    layer.hash(&mut layout);
    keyboard_interactivity.hash(&mut kb);
    (layout.finish(), kb.finish())
}

/// The `(layout, keyboard)` pair for a layer commit with no resolvable layer
/// surface — two empty-hasher finishes, matching the previous inline code.
fn layer_commit_hashes_empty() -> (u64, u64) {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::Hasher;
    (DefaultHasher::new().finish(), DefaultHasher::new().finish())
}

#[cfg(test)]
mod layer_hash_tests {
    use super::layer_commit_hashes;

    /// A baseline layer state (an anchored top bar, keyboard None).
    fn base() -> (u64, u64) {
        layer_commit_hashes((1920, 40), 0b0001, (0, 40), (0, 0), (0, 0, 0, 0), 2, 0)
    }

    #[test]
    fn keyboard_flip_changes_only_the_kb_hash() {
        let (layout0, kb0) = base();
        // Same layout, keyboard_interactivity None(0) → Exclusive(1).
        let (layout1, kb1) =
            layer_commit_hashes((1920, 40), 0b0001, (0, 40), (0, 0), (0, 0, 0, 0), 2, 1);
        assert_eq!(layout0, layout1, "layout hash must not move on a kb flip");
        assert_ne!(kb0, kb1, "kb hash must move on a kb flip");
    }

    #[test]
    fn a_layout_field_changes_only_the_layout_hash() {
        let (layout0, kb0) = base();
        // Same keyboard, exclusive zone 40 → 48 (a bar height change).
        let (layout1, kb1) =
            layer_commit_hashes((1920, 40), 0b0001, (0, 48), (0, 0), (0, 0, 0, 0), 2, 0);
        assert_ne!(layout0, layout1, "layout hash must move on a layout change");
        assert_eq!(kb0, kb1, "kb hash must not move on a layout change");
    }
}
