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
                self.request_repaint();
            }
            PwToNiri::FatalError => {
                tracing::warn!("stopping screencasting due to PipeWire fatal error");
                if let Some(mut casting) = self.screencasting.take() {
                    let session_ids: Vec<_> =
                        casting.casts.iter().map(|c| c.session_id).collect();
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
                let size = smithay::utils::Size::<i32, smithay::utils::Physical>::from(
                    (mode.size.w, mode.size.h),
                );
                let refresh = mode.refresh as u32;
                let weak = mon.output.downgrade();
                (
                    CastTarget::Output {
                        output: weak,
                        name,
                    },
                    size,
                    refresh,
                    false,
                )
            }
            StreamTargetId::Window { id } => {
                // Match the window-id (we hand out per-client
                // memory addresses cast to u64 from
                // `gnome_shell_introspect`). Look up by re-scanning
                // clients; the address is stable for the duration
                // of the client's life.
                let Some(client) = self
                    .clients
                    .iter()
                    .find(|c| std::ptr::addr_of!(**c) as u64 == id)
                else {
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
                let size = smithay::utils::Size::<i32, smithay::utils::Physical>::from(
                    (geom.width, geom.height),
                );
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
        if self.screencasting.is_none() {
            let casting =
                crate::screencasting::Screencasting::new(&self.loop_handle);
            self.screencasting = Some(Box::new(casting));
        }
        let casting = self.screencasting.as_mut().unwrap();

        if casting.pipewire.is_none() {
            let pw_to_compositor = casting.pw_to_compositor.clone();
            match crate::screencasting::pw_utils::PipeWire::new(
                self.loop_handle.clone(),
                pw_to_compositor,
            ) {
                Ok(pw) => casting.pipewire = Some(pw),
                Err(err) => {
                    tracing::warn!(error = ?err, "StartCast: PipeWire init failed");
                    self.stop_cast(session_id);
                    return;
                }
            }
        }
        let pw = casting.pipewire.as_ref().unwrap();

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
                tracing::info!(
                    "StartCast: session={session_id} stream={stream_id} cast pushed"
                );
            }
            Err(err) => {
                tracing::warn!(error = ?err, "StartCast: pw.start_cast failed");
                self.stop_cast(session_id);
            }
        }
    }
}
