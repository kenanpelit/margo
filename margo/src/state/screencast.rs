//! xdp-gnome screencast methods on `MargoState`. Extracted from
//! `state.rs` (roadmap Q1). All three entry points (`on_pw_msg`,
//! `stop_cast`, `start_cast`) are gated behind
//! `feature = "xdp-gnome-screencast"`, so the build with the
//! feature disabled compiles this file down to an empty `impl` block.
//!
//! The implementation mirrors niri's three-message protocol with
//! PipeWire (StopCast / Redraw / FatalError) and the
//! `ScreenCastToCompositor` D-Bus channel that xdp-gnome's
//! `Session.Stop` lands in.

use super::MargoState;

impl MargoState {
    /// Drain a PipeWire-side message into the compositor side.
    /// Mirrors niri's `State::on_pw_msg`. Three message types:
    ///
    ///   * `StopCast { session_id }` — tear down the cast plus
    ///     any matching streams.
    ///   * `Redraw { stream_id }` — kick the render path so this
    ///     stream's next frame renders.
    ///   * `FatalError` — PipeWire failed catastrophically; tear
    ///     down everything and let the next session start cleanly.
    #[cfg(feature = "xdp-gnome-screencast")]
    pub fn on_pw_msg(&mut self, msg: crate::screencasting::pw_utils::PwToNiri) {
        use crate::screencasting::pw_utils::PwToNiri;
        match msg {
            PwToNiri::StopCast { session_id } => self.stop_cast(session_id),
            PwToNiri::Redraw { stream_id: _ } => {
                // PipeWire only fires Redraw twice per stream
                // (initial Streaming + first dmabuf); the steady-
                // state cast cadence comes from the udev repaint
                // loop iterating every active cast on every tick.
                // We just wake the loop so the first cast frame
                // lands on the next VBlank instead of waiting on
                // unrelated input.
                self.wake_repaint_backend();
            }
            PwToNiri::FatalError => {
                tracing::warn!("stopping screencasting due to PipeWire fatal error");
                if let Some(mut casting) = self.screencasting.take() {
                    let session_ids: Vec<_> = casting.casts.iter().map(|c| c.session_id).collect();
                    casting.casts.clear();
                    casting.pipewire = None;
                    self.screencasting = Some(casting);
                    for id in session_ids {
                        self.stop_cast(id);
                    }
                    self.screencasting = None;
                }
            }
        }
    }

    /// A surface tree rooted at `root` committed; `commit_repaints` says
    /// whether the commit requested a repaint of some output. Wake the
    /// render loop when an active cast captures the committing window and
    /// nothing else will render it (see
    /// [`crate::screencasting::pacing::cast_wake_on_commit`]) — e.g. the
    /// window sits on a hidden tag, so its commits repaint no output.
    #[cfg(feature = "xdp-gnome-screencast")]
    pub(crate) fn wake_window_casts_on_commit(
        &mut self,
        root: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        commit_repaints: bool,
    ) {
        use crate::screencasting::CastTarget;
        use crate::screencasting::pacing::cast_wake_on_commit;
        use smithay::wayland::seat::WaylandFocus;

        let Some(casting) = self.screencasting.as_ref() else {
            return;
        };
        if commit_repaints
            || self.session_locked
            || !casting
                .casts
                .iter()
                .any(|cast| matches!(cast.target, CastTarget::Window { .. }))
        {
            return;
        }
        // Popups are captured with their toplevel.
        let popup_root = self
            .popups
            .find_popup(root)
            .and_then(|popup| smithay::desktop::find_popup_root_surface(&popup).ok());
        let target_root = popup_root.as_ref().unwrap_or(root);
        let Some(id) = self
            .clients
            .iter()
            .find(|client| client.window.wl_surface().as_deref() == Some(target_root))
            .map(|client| client.id)
        else {
            return;
        };
        let wake = casting.casts.iter().any(|cast| {
            matches!(cast.target, CastTarget::Window { id: cast_id } if cast_id == id)
                && cast_wake_on_commit(
                    cast.is_active(),
                    commit_repaints,
                    cast.has_scheduled_redraw(),
                )
        });
        if wake {
            self.wake_repaint_backend();
        }
    }

    /// Tear down every cast belonging to the given session. Called
    /// from xdp-gnome's `Session.Stop` D-Bus method (via the
    /// `ScreenCastToCompositor` channel) and from `on_pw_msg` when
    /// PipeWire errors out.
    #[cfg(feature = "xdp-gnome-screencast")]
    pub fn stop_cast(&mut self, session_id: crate::dbus::cast_ids::CastSessionId) {
        let Some(casting) = self.screencasting.as_mut() else {
            return;
        };
        casting.casts.retain(|cast| cast.session_id != session_id);
    }

    /// Start a cast in response to xdp-gnome's `Session.Start`
    /// D-Bus call. Margo equivalent of niri's
    /// `on_screen_cast_msg::StartCast` arm in
    /// `screencasting/mod.rs`.
    ///
    /// Steps:
    ///   1. Resolve the `StreamTargetId` against margo's monitor
    ///      list (output) or client list (toplevel) → produce a
    ///      `CastTarget` + `(size, refresh, alpha)` triple.
    ///   2. Lazy-init `Screencasting` + the PipeWire core if this
    ///      is the first cast of the session.
    ///   3. Call `pw.start_cast(...)` to mint a `Cast`. The cast
    ///      drives PipeWire negotiation; once the format is
    ///      agreed it emits `pipe_wire_stream_added(node_id)` over
    ///      the supplied `signal_ctx` so xdp-gnome / browser can
    ///      open the PipeWire node.
    ///   4. Push the cast onto `casting.casts`. Subsequent frame
    ///      production goes through the udev backend's repaint
    ///      hook (Phase E2 — render integration).
    #[cfg(feature = "xdp-gnome-screencast")]
    pub fn start_cast(
        &mut self,
        session_id: crate::dbus::cast_ids::CastSessionId,
        stream_id: crate::dbus::cast_ids::CastStreamId,
        target: crate::dbus::mutter_screen_cast::StreamTargetId,
        cursor_mode: crate::dbus::mutter_screen_cast::CursorMode,
        signal_ctx: zbus::object_server::SignalEmitter<'static>,
    ) {
        use crate::dbus::mutter_screen_cast::StreamTargetId;
        use crate::screencasting::CastTarget;

        let (target, size, refresh, alpha) = match target {
            StreamTargetId::Output { name } => {
                let Some(mon) = self.monitors.iter().find(|m| m.name == name) else {
                    tracing::warn!(output = %name, "StartCast: requested output is missing");
                    self.stop_cast(session_id);
                    return;
                };
                let Some(mode) = mon.output.current_mode() else {
                    tracing::warn!(output = %name, "StartCast: output has no current mode");
                    self.stop_cast(session_id);
                    return;
                };
                let size = smithay::utils::Size::<i32, smithay::utils::Physical>::from((
                    mode.size.w,
                    mode.size.h,
                ));
                let refresh = mode.refresh as u32;
                let weak = mon.output.downgrade();
                (
                    CastTarget::Output { output: weak, name },
                    size,
                    refresh,
                    false,
                )
            }
            StreamTargetId::Window { id } => {
                // Match the window-id handed out by
                // `gnome_shell_introspect` — the client's stable `id`
                // (minted at construction). Re-scan clients; the id
                // survives `clients` reallocation/removal, unlike the
                // old pointer-address scheme which could alias a
                // different window and capture the wrong one.
                let Some(client) = self.clients.iter().find(|c| c.id == id) else {
                    tracing::warn!(window_id = %id, "StartCast: requested window is missing");
                    self.stop_cast(session_id);
                    return;
                };
                let geom = client.geom;
                if geom.width <= 0 || geom.height <= 0 {
                    tracing::warn!(window_id = %id, "StartCast: window has degenerate geometry");
                    self.stop_cast(session_id);
                    return;
                }
                let size = smithay::utils::Size::<i32, smithay::utils::Physical>::from((
                    geom.width,
                    geom.height,
                ));
                // Use the focused monitor's refresh as a stand-in;
                // PipeWire negotiates an actual pacing later.
                let refresh = self
                    .monitors
                    .get(client.monitor)
                    .and_then(|m| m.output.current_mode())
                    .map(|m| m.refresh as u32)
                    .unwrap_or(60_000);
                (CastTarget::Window { id }, size, refresh, true)
            }
        };

        let Some(gbm) = self.cast_gbm.clone() else {
            tracing::warn!("StartCast: udev GBM device unavailable (winit?)");
            self.stop_cast(session_id);
            return;
        };
        let render_formats = self.cast_render_formats.clone();

        // Lazy-init Screencasting + PipeWire on first cast.
        let loop_handle = self.loop_handle.clone();
        let casting = self.screencasting.get_or_insert_with(|| {
            Box::new(crate::screencasting::Screencasting::new(&loop_handle))
        });

        let pw = match &mut casting.pipewire {
            Some(pw) => pw,
            pipewire @ None => {
                let pw_to_compositor = casting.pw_to_compositor.clone();
                match crate::screencasting::pw_utils::PipeWire::new(loop_handle, pw_to_compositor) {
                    Ok(pw) => pipewire.insert(pw),
                    Err(err) => {
                        tracing::warn!(error = ?err, "StartCast: PipeWire init failed");
                        self.stop_cast(session_id);
                        return;
                    }
                }
            }
        };

        match pw.start_cast(
            gbm,
            render_formats,
            session_id,
            stream_id,
            target,
            size,
            refresh,
            alpha,
            cursor_mode,
            signal_ctx,
        ) {
            Ok(cast) => {
                casting.casts.push(cast);
                tracing::info!("StartCast: session={session_id} stream={stream_id} cast pushed");
            }
            Err(err) => {
                tracing::warn!(error = ?err, "StartCast: pw.start_cast failed");
                self.stop_cast(session_id);
            }
        }
    }
}
