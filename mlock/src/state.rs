//! Wayland client state — globals, outputs, the active session_lock,
//! and per-output lock surfaces.

use anyhow::{Context, Result, anyhow, bail};
use tracing::{debug, info, warn};
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum,
    globals::{GlobalList, GlobalListContents, registry_queue_init},
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
    /// Decoded + blurred wallpaper for the active output. Loaded once
    /// at startup and shared across all per-output surfaces (cairo
    /// scales it to fit each one). `None` if no wallpaper is set or
    /// the path didn't decode.
    pub wallpaper: Option<image::RgbaImage>,
    /// Pre-loaded user avatar (192×192 RGBA). `None` if neither
    /// `~/.face` nor `AccountsService` provided one.
    pub avatar: Option<image::RgbaImage>,
    /// Minute-precision wall clock — bumped by `tick()` so we can
    /// avoid full re-renders when nothing changed.
    last_minute: u32,

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

        // Load + blur the wallpaper once. Cheap fail mode — `None`
        // means render.rs draws the solid dark fallback instead.
        let wallpaper = crate::wallpaper::load_blurred();
        if wallpaper.is_some() {
            info!("wallpaper backdrop loaded");
        } else {
            warn!("no wallpaper backdrop — using solid fallback");
        }
        let avatar = crate::wallpaper::load_avatar(&user);
        if avatar.is_some() {
            info!("avatar loaded (~/.face or AccountsService)");
        }

        let mut state = Self {
            conn: conn.clone(),
            wallpaper,
            avatar,
            last_minute: 0,
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
        let registry = globals.registry();
        for g in globals.contents().clone_list() {
            // `globals.bind()` always picks the FIRST advertised global
            // of a given interface — which is *wrong* for multi-instance
            // globals like wl_output (each connected monitor advertises
            // one). We have to call `registry.bind(name, …)` directly
            // with the specific global's numeric `name` to bind each
            // distinct output. (Documented in wayland-client globals.rs.)
            match g.interface.as_str() {
                "wl_compositor" => {
                    if self.compositor.is_none() {
                        let v = g.version.min(6).max(4);
                        self.compositor = Some(registry.bind(g.name, v, qh, ()));
                    }
                }
                "wl_shm" => {
                    if self.shm.is_none() {
                        self.shm = Some(registry.bind(g.name, 1, qh, ()));
                    }
                }
                "wl_seat" => {
                    if self.seat.is_none() {
                        let v = g.version.min(8).max(1);
                        self.seat = Some(registry.bind(g.name, v, qh, ()));
                    }
                }
                "wl_output" => {
                    let v = g.version.min(4).max(1);
                    let output = registry.bind(g.name, v, qh, ());
                    info!(
                        idx = self.outputs.len(),
                        global_name = g.name,
                        version = v,
                        "wl_output bound (per-instance)"
                    );
                    self.outputs.push(output);
                }
                "ext_session_lock_manager_v1" => {
                    if self.session_lock_manager.is_none() {
                        self.session_lock_manager = Some(registry.bind(g.name, 1, qh, ()));
                    }
                }
                _ => {}
            }
        }
        debug!(outputs = self.outputs.len(), "globals bound");
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

        info!(
            output_count = self.outputs.len(),
            "creating per-output lock surfaces"
        );
        for (idx, output) in self.outputs.iter().enumerate() {
            let wl_surface = compositor.create_surface(qh, ());
            let lock_surface = lock.get_lock_surface(&wl_surface, output, qh, idx);
            info!(idx, "lock_surface created");
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

        match crate::auth::authenticate(&self.user, &password) {
            Ok(()) => {
                info!("authentication succeeded");
                self.unlock(qh);
            }
            Err(e) => {
                warn!("authentication failed: {e}");
                self.seat_state.fail_count = self.seat_state.fail_count.saturating_add(1);
                self.seat_state.fail_message = Some(
                    if self.seat_state.fail_count > 1 {
                        format!("Yanlış parola · {} deneme", self.seat_state.fail_count)
                    } else {
                        "Yanlış parola".to_string()
                    },
                );
                // Trigger a ~400 ms shake animation.
                self.seat_state.shake_until = Some(
                    std::time::Instant::now() + std::time::Duration::from_millis(400),
                );
                self.request_redraw_all();
            }
        }
    }

    /// Bump anything that's time-based (clock minute, fading shake,
    /// battery refresh, power-confirm expiry) and flag surfaces for
    /// redraw when the visible state changes. Called from the main
    /// poll loop once per timer expiration.
    pub fn tick(&mut self) {
        use chrono::Timelike;
        let minute = chrono::Local::now().minute();
        let mut dirty = false;
        if minute != self.last_minute {
            self.last_minute = minute;
            dirty = true;
            // Refresh battery in lock-step with the minute — only
            // need percent precision, not seconds.
            let new_bat = crate::battery::read();
            if format!("{:?}", new_bat) != format!("{:?}", self.seat_state.battery) {
                self.seat_state.battery = new_bat;
                dirty = true;
            }
        }
        if self.seat_state.shake_until.is_some() {
            if !self.seat_state.is_shaking() {
                self.seat_state.shake_until = None;
            }
            dirty = true;
        }
        // Power confirmation window — auto-cancel after 3 s.
        if let Some((_, deadline)) = self.seat_state.power_confirm
            && std::time::Instant::now() >= deadline
        {
            self.seat_state.power_confirm = None;
            dirty = true;
        }
        if dirty {
            self.request_redraw_all();
        }
    }

    /// First F-key press: arm a 3-second confirmation window.
    /// Second matching F-key while armed: execute and exit lock loop
    /// (suspend keeps us locked; shutdown/reboot tear the session
    /// down regardless).
    pub fn power_request(&mut self, action: crate::power::PowerAction) {
        let now = std::time::Instant::now();
        let confirmed = match self.seat_state.power_confirm {
            Some((pending, deadline)) if pending == action && deadline > now => true,
            _ => false,
        };
        if confirmed {
            info!(?action, "power action confirmed — executing");
            crate::power::execute(action);
            self.seat_state.power_confirm = None;
        } else {
            self.seat_state.power_confirm =
                Some((action, now + std::time::Duration::from_secs(3)));
        }
        self.request_redraw_all();
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
                surface.render(
                    shm,
                    qh,
                    &self.seat_state,
                    &self.user,
                    self.wallpaper.as_ref(),
                    self.avatar.as_ref(),
                )?;
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
            info!(idx, width, height, serial, "lock surface configure");
            lock_surface.ack_configure(serial);
            if width == 0 || height == 0 {
                warn!(idx, "configure with zero dimensions — skipping render");
                return;
            }
            if let Some(surface) = state.surfaces.get_mut(*idx) {
                surface.configured = true;
                surface.width = width;
                surface.height = height;
                surface.needs_redraw = true;
            }
            if let Some(shm) = state.shm.clone() {
                let user = state.user.clone();
                let wallpaper = state.wallpaper.clone();
                let avatar = state.avatar.clone();
                if let Some(surface) = state.surfaces.get_mut(*idx) {
                    match surface.render(
                        &shm,
                        qh,
                        &state.seat_state,
                        &user,
                        wallpaper.as_ref(),
                        avatar.as_ref(),
                    ) {
                        Ok(()) => info!(idx, "initial render dispatched"),
                        Err(e) => warn!(idx, "initial render failed: {e:#}"),
                    }
                }
            }
        }
    }
}
