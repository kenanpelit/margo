//! Wayland client state — globals, outputs, the active session_lock,
//! and per-output lock surfaces.

use anyhow::{Context, Result, anyhow, bail};
use tracing::{debug, info, warn};
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum,
    globals::{Global, GlobalList, GlobalListContents, registry_queue_init},
    protocol::{
        wl_buffer, wl_callback, wl_compositor, wl_keyboard, wl_output, wl_registry, wl_seat,
        wl_shm, wl_shm_pool, wl_surface,
    },
};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_surface_v1, ext_session_lock_v1,
};

use crate::seat::SeatState;
use crate::surface::MlockSurface;

pub struct MlockState {
    #[allow(dead_code)]
    pub conn: Connection,

    // Required globals — all populated during the initial roundtrip.
    pub compositor: Option<wl_compositor::WlCompositor>,
    pub shm: Option<wl_shm::WlShm>,
    pub seat: Option<wl_seat::WlSeat>,
    pub session_lock_manager:
        Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,

    // Outputs discovered through the registry. Each entry becomes one
    // MlockSurface once we hold the session_lock.
    pub outputs: Vec<wl_output::WlOutput>,

    pub session_lock: Option<ext_session_lock_v1::ExtSessionLockV1>,
    pub surfaces: Vec<MlockSurface>,

    pub seat_state: SeatState,

    /// Set to `true` once PAM authentication succeeds and we have
    /// called `session_lock.unlock_and_destroy()`. The main loop
    /// exits on the next dispatch.
    pub unlocked: bool,

    /// User name authentication targets — the locker's own process
    /// owner. Read once at startup.
    pub user: String,
}

impl MlockState {
    pub fn new(
        conn: &Connection,
        qh: &QueueHandle<MlockState>,
    ) -> Result<Self> {
        let (globals, _) = registry_queue_init::<MlockState>(conn).map_err(|e| {
            anyhow!("registry init failed: {e}. compositor doesn't speak Wayland properly?")
        })?;

        // Re-register registry listeners on our event queue so we
        // also see *future* outputs (hot-plug).
        let _registry = globals.registry().clone();

        let user = crate::auth::current_user()
            .unwrap_or_else(|| std::env::var("USER").unwrap_or_else(|_| "user".to_string()));

        let mut state = Self {
            conn: conn.clone(),
            compositor: None,
            shm: None,
            seat: None,
            session_lock_manager: None,
            outputs: Vec::new(),
            session_lock: None,
            surfaces: Vec::new(),
            seat_state: SeatState::new(),
            unlocked: false,
            user,
        };

        state.bind_globals(&globals, qh);
        Ok(state)
    }

    fn bind_globals(&mut self, globals: &GlobalList, qh: &QueueHandle<MlockState>) {
        for global in globals.contents().clone_list() {
            self.try_bind_global(globals, qh, &global);
        }
    }

    fn try_bind_global(
        &mut self,
        globals: &GlobalList,
        qh: &QueueHandle<MlockState>,
        g: &Global,
    ) {
        match g.interface.as_str() {
            "wl_compositor" => {
                self.compositor = globals.bind(qh, 4..=6, ()).ok();
            }
            "wl_shm" => {
                self.shm = globals.bind(qh, 1..=1, ()).ok();
            }
            "wl_seat" => {
                self.seat = globals.bind(qh, 1..=8, ()).ok();
            }
            "wl_output" => {
                if let Ok(output) = globals.bind(qh, 1..=4, ()) {
                    debug!("output discovered: name={} v{}", g.name, g.version);
                    self.outputs.push(output);
                }
            }
            "ext_session_lock_manager_v1" => {
                self.session_lock_manager = globals.bind(qh, 1..=1, ()).ok();
            }
            _ => {}
        }
    }

    pub fn assert_globals(&self) -> Result<()> {
        if self.compositor.is_none() {
            bail!("compositor doesn't expose wl_compositor");
        }
        if self.shm.is_none() {
            bail!("compositor doesn't expose wl_shm");
        }
        if self.seat.is_none() {
            bail!("compositor doesn't expose wl_seat");
        }
        if self.session_lock_manager.is_none() {
            bail!(
                "compositor doesn't speak ext-session-lock-v1 — \
                 margo ≥ 0.3.2 or another supporting compositor required"
            );
        }
        if self.outputs.is_empty() {
            bail!("no outputs to lock");
        }
        Ok(())
    }

    /// Take the session lock and request a lock surface for every
    /// known output. After this call the compositor has hidden every
    /// non-lock surface; the user can't see anything else until we
    /// `unlock_and_destroy`.
    pub fn lock_session(&mut self, qh: &QueueHandle<MlockState>) -> Result<()> {
        let manager = self
            .session_lock_manager
            .as_ref()
            .context("session_lock_manager missing")?;
        let compositor = self.compositor.as_ref().context("compositor missing")?;

        let lock = manager.lock(qh, ());
        self.session_lock = Some(lock.clone());

        for (idx, output) in self.outputs.iter().enumerate() {
            let wl_surface = compositor.create_surface(qh, ());
            let lock_surface = lock.get_lock_surface(&wl_surface, output, qh, idx);
            self.surfaces.push(MlockSurface::new(
                idx,
                output.clone(),
                wl_surface,
                lock_surface,
            ));
        }

        Ok(())
    }

    /// Called from the keyboard handler when Enter is pressed.
    pub fn try_authenticate(&mut self, qh: &QueueHandle<MlockState>) {
        if self.seat_state.password.is_empty() {
            return;
        }
        let password = std::mem::take(&mut self.seat_state.password);
        self.seat_state.fail_message = None;

        // PAM call is synchronous; runs on the main thread for now.
        // The lock loop blocks during this (~50-300 ms typical) but
        // since we're already blocked on keyboard input this is fine.
        match crate::auth::authenticate(&self.user, &password) {
            Ok(()) => {
                info!("authentication succeeded");
                self.unlock(qh);
            }
            Err(e) => {
                warn!("authentication failed: {e}");
                self.seat_state.fail_message = Some("Yanlış parola".to_string());
                self.request_redraw_all();
            }
        }
    }

    fn unlock(&mut self, _qh: &QueueHandle<MlockState>) {
        if let Some(lock) = self.session_lock.take() {
            lock.unlock_and_destroy();
        }
        self.unlocked = true;
    }

    pub fn request_redraw_all(&mut self) {
        for s in self.surfaces.iter_mut() {
            s.needs_redraw = true;
        }
    }

    #[allow(dead_code)]
    pub fn render_pending(&mut self, qh: &QueueHandle<MlockState>) -> Result<()> {
        let shm = self.shm.as_ref().context("shm missing")?;
        for surface in self.surfaces.iter_mut() {
            if surface.needs_redraw && surface.configured {
                surface.render(shm, qh, &self.seat_state, &self.user)?;
            }
        }
        Ok(())
    }
}

// ── Dispatch impls ────────────────────────────────────────────────────

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for MlockState {
    fn event(
        _state: &mut Self,
        _registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Hot-plug handling could go here. For MVP we lock the
        // outputs present at startup; new outputs would need a fresh
        // lock surface. Keeping it simple — just log.
        if let wl_registry::Event::Global { interface, name, .. } = event {
            debug!("registry global: {interface} (name={name})");
        }
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for MlockState {
    fn event(
        _: &mut Self,
        _: &wl_compositor::WlCompositor,
        _: wl_compositor::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_shm::WlShm, ()> for MlockState {
    fn event(
        _: &mut Self,
        _: &wl_shm::WlShm,
        _: wl_shm::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_shm_pool::WlShmPool, ()> for MlockState {
    fn event(
        _: &mut Self,
        _: &wl_shm_pool::WlShmPool,
        _: wl_shm_pool::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for MlockState {
    fn event(
        _: &mut Self,
        buffer: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            // Buffer ownership returned to us. We don't double-buffer
            // yet so we can just destroy it — render() always allocs
            // fresh anyway.
            buffer.destroy();
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for MlockState {
    fn event(
        _: &mut Self,
        _: &wl_output::WlOutput,
        _: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for MlockState {
    fn event(
        _: &mut Self,
        _: &wl_surface::WlSurface,
        _: wl_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_callback::WlCallback, usize> for MlockState {
    fn event(
        _: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        _: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for MlockState {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(caps) } = event
            && caps.contains(wl_seat::Capability::Keyboard)
            && state.seat_state.keyboard.is_none()
        {
            let kb = seat.get_keyboard(qh, ());
            state.seat_state.keyboard = Some(kb);
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for MlockState {
    fn event(
        state: &mut Self,
        _kb: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        crate::seat::handle_keyboard_event(state, event, qh);
    }
}

impl Dispatch<ext_session_lock_manager_v1::ExtSessionLockManagerV1, ()> for MlockState {
    fn event(
        _: &mut Self,
        _: &ext_session_lock_manager_v1::ExtSessionLockManagerV1,
        _: ext_session_lock_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ext_session_lock_v1::ExtSessionLockV1, ()> for MlockState {
    fn event(
        _state: &mut Self,
        _: &ext_session_lock_v1::ExtSessionLockV1,
        event: ext_session_lock_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_session_lock_v1::Event::Locked => {
                info!("compositor confirmed lock");
            }
            ext_session_lock_v1::Event::Finished => {
                warn!("compositor terminated the lock (e.g. denied or replaced)");
                _state.unlocked = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<ext_session_lock_surface_v1::ExtSessionLockSurfaceV1, usize> for MlockState {
    fn event(
        state: &mut Self,
        lock_surface: &ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
        event: ext_session_lock_surface_v1::Event,
        idx: &usize,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            debug!(idx, width, height, serial, "lock surface configure");
            lock_surface.ack_configure(serial);
            if let Some(surface) = state.surfaces.get_mut(*idx) {
                surface.configured = true;
                surface.width = width;
                surface.height = height;
                surface.needs_redraw = true;
            }
            if let Some(shm) = state.shm.clone() {
                // Render NOW so the surface has its first buffer
                // attached before the next event loop pass.
                let user = state.user.clone();
                // Take seat_state out briefly to satisfy the borrow
                // checker (state is borrowed mutably below). The
                // seat is otherwise idle here.
                if let Some(surface) = state.surfaces.get_mut(*idx)
                    && let Err(e) =
                        surface.render(&shm, qh, &state.seat_state, &user)
                {
                    warn!("initial render for output {idx} failed: {e}");
                }
            }
        }
    }
}
