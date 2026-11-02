//! Wayland client state — globals, outputs, the active session_lock,
//! and per-output lock surfaces.

use std::collections::BTreeMap;

use anyhow::{Context, Result, anyhow, bail};
use tracing::{debug, info, warn};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
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

/// Build the on-screen failure banner from the running fail count.
///
/// First failure shows a plain "Wrong password"; every subsequent one
/// appends the attempt tally as a soft deterrent. Split out from
/// `try_authenticate` so the retry-message formatting is unit-testable
/// without a PAM round-trip.
pub(crate) fn fail_message(fail_count: u32) -> String {
    if fail_count > 1 {
        format!("Wrong password · {fail_count} attempts")
    } else {
        "Wrong password".to_string()
    }
}

/// One monitor as the registry advertised it, plus the lock surface that
/// covers it once we hold the session lock.
pub(crate) struct OutputSlot<O, S> {
    pub output: O,
    pub surface: Option<S>,
}

/// Outputs and their lock surfaces, keyed by the `wl_output` global's
/// registry `name`.
///
/// The registry name is the only identity that survives hot-plug: it is
/// what `wl_registry.global_remove` reports when a monitor goes away, and
/// it is never reused for a different global during the connection. The
/// previous layout kept outputs and surfaces in two `Vec`s matched by
/// position and handed each lock surface its index as user data, so
/// removing one monitor would have shifted every later index onto the
/// wrong surface. Generic over the output and surface types so the
/// bookkeeping can be tested without a compositor.
pub(crate) struct OutputTable<O, S> {
    slots: BTreeMap<u32, OutputSlot<O, S>>,
}

impl<O, S> OutputTable<O, S> {
    pub fn new() -> Self {
        Self {
            slots: BTreeMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Record a newly advertised output. Returns `false` (and keeps the
    /// existing entry) if this global name is already known.
    pub fn insert_output(&mut self, name: u32, output: O) -> bool {
        if self.slots.contains_key(&name) {
            return false;
        }
        self.slots.insert(
            name,
            OutputSlot {
                output,
                surface: None,
            },
        );
        true
    }

    /// Forget an output whose global was removed, handing back the output
    /// and its lock surface (if any) so the caller can destroy them.
    /// `None` for names that are not outputs (seats, other globals).
    pub fn remove(&mut self, name: u32) -> Option<OutputSlot<O, S>> {
        self.slots.remove(&name)
    }

    /// Global names of the outputs that have no lock surface yet.
    pub fn without_surface(&self) -> Vec<u32> {
        self.slots
            .iter()
            .filter(|(_, slot)| slot.surface.is_none())
            .map(|(name, _)| *name)
            .collect()
    }

    pub fn output(&self, name: u32) -> Option<&O> {
        self.slots.get(&name).map(|slot| &slot.output)
    }

    /// Attach the lock surface for `name`. Refuses (returns `false`) for an
    /// unknown output or one that already has a surface — the protocol
    /// allows only one lock surface per output.
    pub fn set_surface(&mut self, name: u32, surface: S) -> bool {
        match self.slots.get_mut(&name) {
            Some(slot) if slot.surface.is_none() => {
                slot.surface = Some(surface);
                true
            }
            _ => false,
        }
    }

    pub fn surface_mut(&mut self, name: u32) -> Option<&mut S> {
        self.slots
            .get_mut(&name)
            .and_then(|slot| slot.surface.as_mut())
    }

    pub fn surfaces_mut(&mut self) -> impl Iterator<Item = &mut S> {
        self.slots
            .values_mut()
            .filter_map(|slot| slot.surface.as_mut())
    }
}

pub struct MlockState {
    pub conn: Connection,
    /// What goes behind the auth column: the user's flat colour, or a decoded +
    /// blurred image. Resolved once at startup and shared across all per-output
    /// surfaces (cairo scales it to fit each one). `None` when an image was
    /// wanted but no candidate decoded.
    pub backdrop: Option<crate::wallpaper::Backdrop>,
    /// Pre-loaded user avatar (192×192 RGBA). `None` if neither
    /// `~/.face` nor `AccountsService` provided one.
    pub avatar: Option<image::RgbaImage>,
    /// Matugen accent (primary) read once at startup; tints the
    /// password dots / placeholder so the locker tracks the theme.
    pub accent: (f64, f64, f64),
    /// Per-element visibility, read once from `mlock.conf`.
    pub toggles: crate::config::LockToggles,
    /// Live desktop info (notifications / weather / now-playing)
    /// published by the shell; re-read every ~2 s in `tick()`.
    pub info: crate::sidecar::LockInfo,
    /// Throttle for the sidecar re-read.
    last_info_check: std::time::Instant,
    /// Minute-precision wall clock — bumped by `tick()` so we can
    /// avoid full re-renders when nothing changed.
    last_minute: u32,

    // Required globals — all populated during the initial roundtrip.
    pub compositor: Option<wl_compositor::WlCompositor>,
    pub shm: Option<wl_shm::WlShm>,
    pub seat: Option<wl_seat::WlSeat>,
    pub session_lock_manager: Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,

    // Outputs discovered through the registry, at startup and hot-plugged
    // later, each with its MlockSurface once we hold the session_lock.
    pub(crate) outputs: OutputTable<wl_output::WlOutput, MlockSurface>,

    pub session_lock: Option<ext_session_lock_v1::ExtSessionLockV1>,

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
    /// Connect the registry and bind the startup globals. Returns the event
    /// queue the registry lives on: it must be the queue the caller
    /// dispatches, because later `global` / `global_remove` events (monitor
    /// hot-plug) are delivered to it. Discarding it, as this used to, threw
    /// every hot-plug event away.
    pub fn new(conn: &Connection) -> Result<(Self, EventQueue<MlockState>)> {
        let (globals, event_queue) = registry_queue_init::<MlockState>(conn).map_err(|e| {
            anyhow!("registry init failed: {e}. compositor doesn't speak Wayland properly?")
        })?;
        let qh = event_queue.handle();

        let user = crate::auth::current_user()
            .unwrap_or_else(|| std::env::var("USER").unwrap_or_else(|_| "user".to_string()));

        // Resolve the configured background once (wallpaper / solid
        // colour / custom image — ~/.config/margo/mlock.conf). `None`
        // means render.rs draws the solid palette fallback instead.
        let backdrop = crate::wallpaper::load_background();
        match &backdrop {
            Some(crate::wallpaper::Backdrop::Image(_)) => info!("backdrop: blurred image"),
            Some(crate::wallpaper::Backdrop::Solid(_)) => info!("backdrop: solid colour"),
            None => warn!("no backdrop image resolved — using the palette surface"),
        }
        let avatar = crate::wallpaper::load_avatar(&user);
        if avatar.is_some() {
            info!("avatar loaded (~/.face or AccountsService)");
        }
        let accent = crate::render::matugen_accent();
        let toggles = crate::config::LockToggles::load();
        let info = crate::sidecar::LockInfo::load();

        let mut state = Self {
            conn: conn.clone(),
            backdrop,
            avatar,
            accent,
            toggles,
            info,
            last_info_check: std::time::Instant::now(),
            last_minute: 0,
            compositor: None,
            shm: None,
            seat: None,
            session_lock_manager: None,
            outputs: OutputTable::new(),
            session_lock: None,
            seat_state: SeatState::new(),
            unlocked: false,
            user,
        };

        state.bind_globals(&globals, &qh);
        Ok((state, event_queue))
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
                "wl_compositor" if self.compositor.is_none() => {
                    let v = g.version.clamp(4, 6);
                    self.compositor = Some(registry.bind(g.name, v, qh, ()));
                }
                "wl_shm" if self.shm.is_none() => {
                    self.shm = Some(registry.bind(g.name, 1, qh, ()));
                }
                "wl_seat" if self.seat.is_none() => {
                    let v = g.version.clamp(1, 8);
                    self.seat = Some(registry.bind(g.name, v, qh, ()));
                }
                "wl_output" => self.add_output(registry, g.name, g.version, qh),
                "ext_session_lock_manager_v1" if self.session_lock_manager.is_none() => {
                    self.session_lock_manager = Some(registry.bind(g.name, 1, qh, ()));
                }
                _ => {}
            }
        }
        debug!(outputs = self.outputs.len(), "globals bound");
    }

    /// Bind one advertised `wl_output` and, if the session is already
    /// locked, cover it with a lock surface straight away.
    fn add_output(
        &mut self,
        registry: &wl_registry::WlRegistry,
        name: u32,
        version: u32,
        qh: &QueueHandle<MlockState>,
    ) {
        if self.outputs.output(name).is_some() {
            return;
        }
        let v = version.clamp(1, 4);
        let output: wl_output::WlOutput = registry.bind(name, v, qh, ());
        info!(global_name = name, version = v, "wl_output bound");
        self.outputs.insert_output(name, output);
        if self.session_lock.is_some() && !self.unlocked {
            self.create_lock_surface(name, qh);
        }
    }

    /// The monitor behind global `name` went away: destroy its lock surface
    /// and release the output. Ignores names that aren't outputs.
    fn remove_output(&mut self, name: u32) {
        let Some(slot) = self.outputs.remove(name) else {
            return;
        };
        info!(global_name = name, "wl_output removed");
        if let Some(surface) = slot.surface {
            surface.destroy();
        }
        if slot.output.version() >= 3 {
            slot.output.release();
        }
    }

    /// Create the lock surface for the known output `name`.
    fn create_lock_surface(&mut self, name: u32, qh: &QueueHandle<MlockState>) {
        let (Some(lock), Some(compositor)) = (self.session_lock.as_ref(), self.compositor.as_ref())
        else {
            return;
        };
        let Some(output) = self.outputs.output(name) else {
            return;
        };
        let wl_surface = compositor.create_surface(qh, ());
        let lock_surface = lock.get_lock_surface(&wl_surface, output, qh, name);
        let surface = MlockSurface::new(name, output.clone(), wl_surface, lock_surface);
        if self.outputs.set_surface(name, surface) {
            info!(global_name = name, "lock_surface created");
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
        if self.compositor.is_none() {
            bail!("compositor missing");
        }

        let lock = manager.lock(qh, ());
        self.session_lock = Some(lock);

        info!(
            output_count = self.outputs.len(),
            "creating per-output lock surfaces"
        );
        for name in self.outputs.without_surface() {
            self.create_lock_surface(name, qh);
        }

        Ok(())
    }

    /// Called from the keyboard handler when Enter is pressed.
    pub fn try_authenticate(&mut self, qh: &QueueHandle<MlockState>) {
        if self.seat_state.password.is_empty() {
            return;
        }
        // Zeroize the plaintext once auth returns — mem::take moves the real
        // bytes into this local, and the SeatState Drop only scrubs the (now
        // empty) source buffer. Without this the password survives in freed
        // heap on the lock screen, the one surface that must not leak it.
        let password = zeroize::Zeroizing::new(std::mem::take(&mut self.seat_state.password));
        self.seat_state.fail_message = None;

        match crate::auth::authenticate(&self.user, &password) {
            Ok(()) => {
                info!("authentication succeeded");
                self.unlock(qh);
            }
            Err(e) => {
                warn!("authentication failed: {e}");
                self.seat_state.fail_count = self.seat_state.fail_count.saturating_add(1);
                self.seat_state.fail_message = Some(fail_message(self.seat_state.fail_count));
                // Trigger a ~400 ms shake animation.
                self.seat_state.shake_until =
                    Some(std::time::Instant::now() + std::time::Duration::from_millis(400));
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
        // Re-read the shell's live info sidecar at most every ~2 s so a
        // notification arriving / a track changing while locked shows up
        // without a full per-poll file read.
        if self.last_info_check.elapsed() >= std::time::Duration::from_secs(2) {
            self.last_info_check = std::time::Instant::now();
            let new_info = crate::sidecar::LockInfo::load();
            if new_info != self.info {
                self.info = new_info;
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
        let confirmed = crate::power::is_confirmed(self.seat_state.power_confirm, action, now);
        if confirmed {
            info!(?action, "power action confirmed — executing");
            crate::power::execute(action);
            self.seat_state.power_confirm = None;
        } else {
            self.seat_state.power_confirm = Some((action, now + std::time::Duration::from_secs(3)));
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
        for s in self.outputs.surfaces_mut() {
            s.needs_redraw = true;
        }
    }

    pub fn render_pending(&mut self, qh: &QueueHandle<MlockState>) -> Result<()> {
        let shm = self.shm.as_ref().context("shm missing")?;
        for surface in self.outputs.surfaces_mut() {
            if surface.needs_redraw && surface.configured {
                surface.render(
                    shm,
                    qh,
                    &self.seat_state,
                    &self.user,
                    self.backdrop.as_ref(),
                    self.avatar.as_ref(),
                    self.accent,
                    &self.toggles,
                    &self.info,
                )?;
            }
        }
        Ok(())
    }
}

// ── Dispatch impls ────────────────────────────────────────────────────

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for MlockState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        // Monitor hot-plug while locked (docking, closing the lid with only
        // the external screen left). margo paints outputs without a lock
        // surface black, so nothing leaks, but without these arms a new
        // monitor stays black with no password prompt and a removed one
        // keeps a dead lock surface.
        match event {
            wl_registry::Event::Global {
                interface,
                name,
                version,
            } => {
                debug!("registry global: {interface} (name={name})");
                if interface == "wl_output" {
                    state.add_output(registry, name, version, qh);
                }
            }
            wl_registry::Event::GlobalRemove { name } => {
                debug!("registry global_remove: name={name}");
                state.remove_output(name);
            }
            _ => {}
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
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
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

impl Dispatch<ext_session_lock_surface_v1::ExtSessionLockSurfaceV1, u32> for MlockState {
    fn event(
        state: &mut Self,
        lock_surface: &ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
        event: ext_session_lock_surface_v1::Event,
        global_name: &u32,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            let name = *global_name;
            info!(
                global_name = name,
                width, height, serial, "lock surface configure"
            );
            lock_surface.ack_configure(serial);
            if width == 0 || height == 0 {
                warn!(
                    global_name = name,
                    "configure with zero dimensions — skipping render"
                );
                return;
            }
            if let Some(surface) = state.outputs.surface_mut(name) {
                surface.configured = true;
                surface.width = width;
                surface.height = height;
                surface.needs_redraw = true;
            }
            // Disjoint field borrows, not clones: `surfaces` is taken mutably
            // while the backdrop, the avatar and the seat are read. The clones
            // this replaces copied the whole decoded wallpaper — nine megabytes
            // — once per output, to satisfy a borrow the compiler was willing to
            // split all along.
            let MlockState {
                shm,
                outputs,
                seat_state,
                user,
                backdrop,
                avatar,
                accent,
                toggles,
                info,
                ..
            } = state;
            if let Some(shm) = shm.as_ref()
                && let Some(surface) = outputs.surface_mut(name)
            {
                match surface.render(
                    shm,
                    qh,
                    seat_state,
                    user,
                    backdrop.as_ref(),
                    avatar.as_ref(),
                    *accent,
                    toggles,
                    info,
                ) {
                    Ok(()) => info!(global_name = name, "initial render dispatched"),
                    Err(e) => warn!(global_name = name, "initial render failed: {e:#}"),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{OutputTable, fail_message};

    #[test]
    fn removing_an_output_leaves_the_others_on_their_own_surfaces() {
        // Keyed by registry name, not position: removing the first monitor
        // must not shift the second one's surface onto a different output.
        let mut t: OutputTable<&str, &str> = OutputTable::new();
        assert!(t.insert_output(30, "eDP-1"));
        assert!(t.insert_output(31, "HDMI-A-1"));
        assert!(t.set_surface(30, "lock-eDP"));
        assert!(t.set_surface(31, "lock-HDMI"));

        let gone = t.remove(30).expect("output 30 was known");
        assert_eq!(gone.output, "eDP-1");
        assert_eq!(gone.surface, Some("lock-eDP"));

        assert_eq!(t.len(), 1);
        assert_eq!(t.surface_mut(31).copied(), Some("lock-HDMI"));
        assert_eq!(t.surface_mut(30), None);
    }

    #[test]
    fn removing_a_non_output_global_is_ignored() {
        let mut t: OutputTable<&str, &str> = OutputTable::new();
        t.insert_output(30, "eDP-1");
        assert!(t.remove(7).is_none(), "a seat / other global name");
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn a_hotplugged_output_is_the_only_one_without_a_surface() {
        let mut t: OutputTable<&str, &str> = OutputTable::new();
        t.insert_output(30, "eDP-1");
        t.set_surface(30, "lock-eDP");
        // Docked while locked.
        t.insert_output(45, "DP-2");
        assert_eq!(t.without_surface(), vec![45]);
        assert!(t.set_surface(45, "lock-DP"));
        assert!(t.without_surface().is_empty());
        assert_eq!(t.surfaces_mut().count(), 2);
    }

    #[test]
    fn one_lock_surface_per_output_and_no_duplicate_outputs() {
        let mut t: OutputTable<&str, &str> = OutputTable::new();
        assert!(t.insert_output(30, "eDP-1"));
        assert!(!t.insert_output(30, "eDP-1 again"), "same global twice");
        assert_eq!(t.output(30).copied(), Some("eDP-1"));
        assert!(t.set_surface(30, "first"));
        assert!(!t.set_surface(30, "second"), "ext-session-lock allows one");
        assert_eq!(t.surface_mut(30).copied(), Some("first"));
        assert!(!t.set_surface(99, "orphan"), "unknown output");
    }

    #[test]
    fn every_output_removed_then_one_returns() {
        // Lid closed on the only monitor, then reopened.
        let mut t: OutputTable<&str, &str> = OutputTable::new();
        t.insert_output(30, "eDP-1");
        t.set_surface(30, "lock-eDP");
        t.remove(30);
        assert!(t.is_empty());
        t.insert_output(52, "eDP-1");
        assert_eq!(t.without_surface(), vec![52]);
    }

    #[test]
    fn first_failure_shows_plain_message() {
        // fail_count is bumped BEFORE formatting, so the first wrong password
        // arrives here as 1 → no attempt tally yet.
        assert_eq!(fail_message(1), "Wrong password");
    }

    #[test]
    fn repeated_failures_append_the_attempt_tally() {
        assert_eq!(fail_message(2), "Wrong password · 2 attempts");
        assert_eq!(fail_message(5), "Wrong password · 5 attempts");
    }
}
