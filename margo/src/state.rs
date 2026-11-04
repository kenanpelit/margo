#![allow(dead_code)]

// W4.2: per-protocol handler impls extracted into sibling files
// under `state/handlers/` for incremental-compile wins. Each
// submodule reaches into `MargoState` via `crate::state::MargoState`.
mod handlers;

// Roadmap Q1 — extracting pure state-internal helpers out of the
// 6800-line state.rs into siblings under `state/`. Theme is the
// first step; others follow in the same form.
mod animation_tick;
mod arrange;
mod client_removal;
mod data;
mod debug_dump;
mod directional;
mod dispatch;
mod dpms;
mod focus_methods;
mod focus_target;
mod frame_clock_sched;
mod groups;
mod reload;
mod repaint;
pub(crate) use groups::GroupLock;
mod input_timers;
mod interactive_grab;
pub mod mru_switcher;
mod output_pending;
mod overview;
mod pacing;
mod protocols;
mod scratchpad;
pub(crate) use scratchpad::MatchOp;
mod screencast;
mod scroller_overview;
mod session_end;
mod state_file;
mod theme;
mod twilight_methods;
mod window_rules;
mod xwayland;

pub use self::animation_tick::{AnimTickSpec, tick_animations};
pub use self::data::{
    ClosingClient, FullscreenMode, HotCorner, LayerSurfaceAnim, MargoClient, MargoMonitor,
    ResizeSnapshot,
};
pub(crate) use self::data::{
    WindowRuleReason, clamp_size, logical_output_rect, matches_layer_name, matches_rule_text,
    read_toplevel_identity, read_toplevel_identity_if_changed, read_toplevel_size_hints,
};
pub use self::focus_target::FocusTarget;
pub use self::input_timers::InputTimers;
pub use self::output_pending::OutputPending;
use self::protocols::DISABLE_MIDDLE_PASTE;
pub use self::protocols::Protocols;
pub use self::scroller_overview::{ScrollerOverview, overview_cells};
pub(crate) use self::theme::ThemeBaseline;
pub use self::twilight_methods::Twilight;
use crate::ipc::server::IpcServer;

use std::{cell::RefCell, path::PathBuf, rc::Rc};

use anyhow::{Context, Result};
use smithay::{
    backend::allocator::dmabuf::Dmabuf,
    desktop::{
        PopupManager, Space, Window, WindowSurface, layer_map_for_output,
        utils::send_frames_surface_tree,
    },
    input::{Seat, SeatState, pointer::CursorImageStatus},
    output::Output,
    reexports::{
        calloop::{LoopHandle, LoopSignal, ping::Ping},
        wayland_server::{
            Display, DisplayHandle, Resource,
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
        },
    },
    utils::{Clock, Monotonic, SERIAL_COUNTER, Serial, Size},
    wayland::{
        compositor::{CompositorClientState, with_states},
        seat::WaylandFocus,
        shell::{
            wlr_layer::Layer as WlrLayer,
            xdg::{ToplevelSurface, XdgToplevelSurfaceData},
        },
        text_input::TextInputSeat,
    },
    xwayland::{X11Surface, X11Wm},
};

use margo_config::{Config, WindowRule, generations};

use crate::{
    animation::{AnimationCurves, AnimationType, ClientAnimation, OpacityAnimation},
    cursor::CursorManager,
    input::{GestureState, KeyboardState, PointerState, TouchState},
    layout::{self, LayoutId, Rect},
    protocols::{
        foreign_toplevel::{ForeignToplevelListHandler, ForeignToplevelListState},
        layer_shell::LayerSurface,
    },
};

// ── Client data attached to each Wayland client connection ───────────────────

#[derive(Default)]
pub struct MargoClientData {
    pub compositor_state: CompositorClientState,
    /// The client connected through a `wp_security_context_v1` listener
    /// (Flatpak and other sandboxes). Restricted clients don't see the
    /// privileged globals; see [`client_is_unrestricted`].
    pub restricted: bool,
}

impl ClientData for MargoClientData {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

/// Global filter for the privileged protocols (niri's approach): `false` for
/// a sandboxed client that came in through a `wp_security_context_v1`
/// listener. Those globals would let it escape the sandbox — read the screen
/// or clipboard, inject input, lock the session, reconfigure outputs, list
/// every window. Clients without `MargoClientData` (XWayland, which margo
/// spawns itself) are unrestricted.
pub fn client_is_unrestricted(client: &smithay::reexports::wayland_server::Client) -> bool {
    client
        .get_data::<MargoClientData>()
        .is_none_or(|data| !data.restricted)
}

/// Whether `force_unlock` may tear down the current lock: only once the
/// owning locker is gone. A live locker is never overridden, even with no
/// lock surface left: unplugging every monitor drops all lock surfaces of a
/// working lock, so "no surface" is no proof that the lock is stuck. A hung
/// but live locker has to be killed (TTY, ssh) first.
pub fn force_unlock_allowed(owner_alive: bool) -> bool {
    !owner_alive
}

// One-line tag for focus targets; only used in tracing so we don't have to
// pull `Debug` through whatever wrapped surface a target carries.
fn focus_target_label(t: &FocusTarget) -> String {
    match t {
        FocusTarget::Window(w) => format!("Window({:?})", w.wl_surface().map(|s| s.id())),
        FocusTarget::LayerSurface(_) => "LayerSurface".to_string(),
        FocusTarget::SessionLock(s) => format!("SessionLock({:?})", s.wl_surface().id()),
        FocusTarget::Popup(s) => format!("Popup({:?})", s.id()),
    }
}

// ── Animation tick — moved to `state/animation_tick.rs` ────────────────────
//
// `tick_animations` + `AnimTickSpec` are re-exported above. Edit
// the body in `state/animation_tick.rs` — touching state.rs no
// longer recompiles the animation tick path, and vice-versa.

// ── Top-level compositor state ────────────────────────────────────────────────

pub type DmabufImportHook = Rc<RefCell<dyn FnMut(&Dmabuf) -> bool>>;

/// Per-surface frame-callback throttling state, kept in the surface's
/// `data_map`. Mirrors niri's `SurfaceFrameThrottlingState` exactly — the
/// pair `(Output, sequence)` records when the surface last received a
/// `wl_surface.frame` done. Combined with the per-output
/// `frame_callback_sequence`, this enforces "at most one frame_done per
/// surface per refresh cycle", which is what stops gtk4-layer-shell
/// clients (mshell-frame, noctalia) from getting back-to-back callbacks
/// during a single vblank and entering the busy-commit loop that
/// previously made the bar flicker on margo while staying smooth on
/// Hyprland and niri.
#[derive(Default)]
pub(crate) struct SurfaceFrameThrottlingState {
    pub last_sent_at: std::cell::RefCell<Option<(Output, u32)>>,
}

/// Which input signal currently defines the "active output" — the
/// monitor mshell targets when a keybind/IPC opens a menu (launcher,
/// settings, every pill menu). Last-writer-wins between the two:
///
///   * `Focus` — the user just acted on the keyboard-focused monitor via
///     a margo-internal keybind (tag switch, focus move, layout, …). The
///     active output follows keyboard focus.
///   * `Pointer` — the user's cursor last crossed into a (possibly empty)
///     monitor. The active output follows the pointer.
///
/// Menu-open keybinds are `spawn mshellctl menu …`, i.e. `spawn` actions,
/// which are deliberately *neutral*: they read the current source instead
/// of overwriting it, so "I just moved the mouse to monitor B → open the
/// launcher there" and "I'm working with the keyboard on monitor A → open
/// it there" both do the intuitive thing. Resolved to a live monitor in
/// `build_state_snapshot`, so monitor hot-plug/unplug can't leave it
/// pointing at a stale index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActiveOutputSource {
    /// Keyboard focus defines the active output (default at startup).
    #[default]
    Focus,
    /// The pointer's monitor defines the active output.
    Pointer,
}

/// Does any window rule carry a title / exclude_title pattern? Cached on
/// `MargoState` as `title_rules_exist` and only recomputed when the config
/// changes — see that field.
fn config_has_title_rules(config: &Config) -> bool {
    config.window_rules.iter().any(|rule| {
        rule.title.as_ref().is_some_and(|p| !p.is_empty())
            || rule.exclude_title.as_ref().is_some_and(|p| !p.is_empty())
    })
}

/// Per-output frame counters, mirrored out of the udev backend's
/// `OutputDevice` so `mctl perf` (the `get perf` IPC topic) can read them
/// without borrowing the `BackendData` — which `MargoState` deliberately
/// doesn't hold, to keep the render borrow and the IPC borrow disjoint.
#[derive(Debug, Default, Clone)]
pub struct OutputPerf {
    pub renders: u64,
    pub queued: u64,
    pub empties: u64,
    pub queue_errors: u64,
    /// `render_frame` failures (see `OutputDevice::render_error_count`) —
    /// distinct from `queue_errors`, and surfaced in `mctl perf --json`.
    pub render_errors: u64,
    /// Rolling ~60 s window of recently rendered frames, backing the
    /// time-based metrics (`fps_*`, windowed empty ratio, `render_us_p*`)
    /// that the `perf` topic reports alongside the lifetime totals. Pushed
    /// once per rendered frame by [`MargoState::record_frame_sample`], which
    /// also prunes it to the window.
    pub samples: std::collections::VecDeque<FrameSample>,
}

/// One rendered frame's timing, retained briefly per output so `perf` can
/// report recent FPS and render-latency percentiles instead of only lifetime
/// counters. `Copy` + tiny, so the rolling window is cheap to keep.
#[derive(Debug, Clone, Copy)]
pub struct FrameSample {
    /// When this frame's `render_frame` completed (monotonic).
    pub at: std::time::Instant,
    /// The render produced no damage (nothing was submitted to KMS).
    pub empty: bool,
    /// Wall time spent in `render_frame` for this frame, in microseconds
    /// (saturated at `u32::MAX`).
    pub render_us: u32,
    /// Area composited this frame — the primary plane's newest damage
    /// group, summed rect areas in physical px² (saturated at
    /// `u32::MAX`). 0 for empty and direct-scanout frames.
    pub damage_px: u32,
    /// The primary plane presented a client buffer directly this frame
    /// (`PrimaryPlaneElement::Element`) — nothing was composited.
    pub scanout: bool,
}

pub struct MargoState {
    /// Every advertised Wayland protocol global; see `state/protocols.rs`.
    pub protocols: Protocols,
    pub dmabuf_import_hook: Option<DmabufImportHook>,
    pub seat_state: SeatState<MargoState>,
    /// Output changes queued for the backend to drain (`state/output_pending.rs`).
    pub output_pending: OutputPending,
    /// Active leases; dropping one revokes it.
    pub drm_lease_active: Vec<smithay::wayland::drm_lease::DrmLease>,
    /// Snapshot loaded from the crash autosave at startup — applied by
    /// the autosave timer once outputs are mapped, then cleared.
    pub pending_crash_restore: Option<crate::session::SessionSnapshot>,
    /// xdg-desktop-portal GlobalShortcuts registry (see
    /// `global_shortcuts.rs`); fed by margo-portal over the socket.
    pub global_shortcuts: crate::global_shortcuts::GlobalShortcutsRegistry,
    /// Handle into the udev backend's device data. The ONLY sanctioned
    /// State→backend reach — exists because `DrmLeaseHandler::lease_request`
    /// must build a `DrmLeaseBuilder` from the live `DrmDevice`
    /// synchronously (the deferred-queue pattern can't help there).
    pub(crate) udev_backend:
        Option<std::rc::Rc<std::cell::RefCell<crate::backend::udev::BackendData>>>,
    /// User-script engine. `None` until init.rhai, a plugin or
    /// `mctl run` first needs it. Boxed so the field is small + we can
    /// `Option::take()` it while a script runs (the recursion guard +
    /// borrow-checker dance lives in `scripting::with_engine`).
    pub scripting: Option<Box<crate::scripting::ScriptingState>>,
    /// Hooks registered by scripts (`on_focus_change(...)` & co.). Kept
    /// outside `scripting` because that box is checked out while the
    /// registering script runs.
    pub script_hooks: crate::scripting::ScriptingHooks,
    /// Active screencasting state — PipeWire core, list of running
    /// casts, dynamic-cast queue. `None` until xdp-gnome opens its
    /// first ScreenCast session and the lazy PipeWire init runs;
    /// margo's compositor process otherwise pays no PipeWire cost.
    /// Gated on the `xdp-gnome-screencast` feature so distro builds
    /// without screencast support drop the entire PipeWire dep tree.
    #[cfg(feature = "xdp-gnome-screencast")]
    pub screencasting: Option<Box<crate::screencasting::Screencasting>>,
    /// D-Bus shim connections so xdp-gnome can serve the
    /// ScreenCast / Screenshot / Mutter portals on margo without
    /// gnome-shell. See `crate::dbus`. Set once at startup;
    /// connections close when the field drops (compositor exit).
    /// Gated on the `dbus` feature.
    #[cfg(feature = "dbus")]
    pub dbus_servers: crate::dbus::DBusServers,
    /// Shared snapshot of monitors used by the Mutter D-Bus shims
    /// (`DisplayConfig` + `ScreenCast`). The same Arc handed to
    /// both services so a hotplug-driven `refresh_ipc_outputs()`
    /// updates both views at once. Previously each service got
    /// its own `ipc_output::snapshot(&margo)` at startup and
    /// neither refreshed — a monitor unplugged mid-cast left
    /// xdp-gnome's chooser dialog still listing the gone output.
    /// Lazy in the sense that we only re-snapshot when the
    /// monitor list actually changes, not every frame.
    #[cfg(feature = "dbus")]
    pub ipc_outputs: std::sync::Arc<std::sync::Mutex<crate::dbus::ipc_output::IpcOutputMap>>,
    /// GBM device the udev backend opened for buffer allocation.
    /// Populated at backend init; D-Bus / screencast threads pull
    /// it for `Cast::new` to allocate dmabuf-backed PipeWire
    /// buffers without re-opening the DRM node. `None` outside
    /// the udev backend (winit nested mode).
    pub cast_gbm:
        Option<smithay::backend::allocator::gbm::GbmDevice<smithay::backend::drm::DrmDeviceFd>>,
    /// Renderer-side dmabuf format constraints, snapshotted at
    /// backend init so the screencast cast lifecycle has them
    /// without crossing the borrow boundary into the udev
    /// renderer mid-D-Bus-call.
    pub cast_render_formats: smithay::backend::allocator::format::FormatSet,
    /// Active capture sessions, keyed by something we can match
    /// against an `ImageCaptureSource` later — for now we hold
    /// the `Session` handles so they don't get dropped (which
    /// would auto-stop the session). Real frame routing wires
    /// up in the rendering follow-up commit.
    pub image_copy_capture_sessions: Vec<smithay::wayland::image_copy_capture::Session>,
    /// Frames awaiting their backing source's content. The udev
    /// repaint handler drains this list after every render and
    /// fills each frame's buffer from the matching output (or
    /// fails the frame if the source has gone stale). Stored as
    /// `(session_ref, frame, source_kind)` so we can route
    /// without re-querying user_data on each iteration.
    pub pending_image_copy_frames: Vec<crate::PendingImageCopyFrame>,
    /// Currently-active inhibitors, keyed by their target wl_surface.
    /// `input_handler.rs` checks the focused surface against this map
    /// every key press; a hit short-circuits margo's keybinding match
    /// and forwards the key straight to the client.
    pub keyboard_shortcuts_inhibiting_surfaces: std::collections::HashMap<
        smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        smithay::wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitor,
    >,
    /// Surfaces that have an active idle-inhibit object. We feed
    /// `!is_empty()` to the notifier whenever this set changes.
    pub idle_inhibitors: std::collections::HashSet<
        smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    >,
    /// Indices into `config.window_rules` of `windowrule-once` rules
    /// that have already fired for some client. Consulted (and grown)
    /// in `apply_matched_window_rules`; cleared on `mctl reload` so
    /// editing the rulebook gets a clean slate.
    pub(crate) rule_once_consumed: std::collections::HashSet<usize>,
    /// Hash of the layout-affecting cached state per layer surface
    /// (size, anchor, exclusive_zone, exclusive_edge, margin, layer,
    /// keyboard_interactivity). Mirrors Hyprland's
    /// `m_current.committed != 0` check in `CLayerSurface::onCommit`:
    /// only re-`arrange()` + recompute work area + refresh keyboard
    /// focus when the layer surface actually changed something
    /// layout-affecting. A plain buffer commit (the 60-fps case
    /// gtk4-layer-shell drives during Revealer animations) keeps the
    /// same hash and short-circuits the entire arrange chain —
    /// which is what was burning the CPU and causing the bar to
    /// flicker. Entries are bounded by the number of mapped layer
    /// surfaces (handful) and dropped in `layer_destroyed`; no
    /// per-frame allocation.
    pub layer_layout_hashes:
        std::collections::HashMap<smithay::reexports::wayland_server::backend::ObjectId, u64>,
    /// Per-layer-surface hash of *just* `keyboard_interactivity`,
    /// tracked separately from `layer_layout_hashes` so we can
    /// independently dedup focus-refresh from arrange-refresh.
    /// noctalia's launcher/settings panels flip
    /// keyboard_interactivity between `Exclusive` and `None` on the
    /// same surface and need focus recomputed when that happens;
    /// mshell's bar never flips it during normal updates, so layered
    /// content commits (clock tick, network speed, CPU stats) must
    /// NOT pay the focus-refresh cost. Cleared in `layer_destroyed`.
    pub layer_kb_interactivity_hashes:
        std::collections::HashMap<smithay::reexports::wayland_server::backend::ObjectId, u64>,
    /// Layer roots that have committed their first buffer. The first
    /// one re-picks pointer focus (see `refresh_pointer_focus`) so a
    /// layer that maps under a still pointer gets `wl_pointer.enter`
    /// without the user having to move it. Cleared in `layer_destroyed`.
    pub layer_buffer_mapped:
        std::collections::HashSet<smithay::reexports::wayland_server::backend::ObjectId>,
    /// Per-output frame-callback sequence number. Bumped once per real
    /// vblank (in `note_vblank`) and once per estimated vblank (when
    /// the timer queued from the empty-render path fires). Surfaces
    /// stamp their `SurfaceFrameThrottlingState.last_sent_at` with
    /// this value; `send_frame_callbacks` then skips any surface
    /// already stamped with the current sequence, mirroring niri's
    /// sequence-based dedup. This is what prevents the GTK frame
    /// clock on mshell-frame from getting two `frame_done` events
    /// in the same refresh cycle (which is what caused the bar
    /// flicker). Keyed by output name because `Output` is not `Hash`.
    pub frame_callback_sequence: std::collections::HashMap<String, u32>,
    /// Per-output frame counters mirrored from the udev backend each render,
    /// keyed by output name (same key as `frame_callback_sequence`). Read by
    /// the `perf` IPC topic; empty under the winit/nested backend.
    pub perf_counters: std::collections::HashMap<String, OutputPerf>,
    /// One process-lifetime synthetic output used only as the identity passed
    /// to callback-only frame delivery for unmapped surfaces. Keeping it here
    /// avoids allocating a fresh `Output` on every 32 ms warm-up tick.
    frame_callback_fallback_output: Output,
    /// One pending estimated-vblank Timer per output. Inserted from
    /// the empty-render path (`render_frame` reports
    /// `is_empty == true` so no DRM page-flip + no real VBlank event
    /// is coming), and the timer's callback bumps
    /// `frame_callback_sequence` + re-sends frame callbacks so
    /// clients stay paced at the display's refresh rate even while
    /// nothing is changing on screen. Removed when the timer fires
    /// (it returns `TimeoutAction::Drop`) or when a real vblank
    /// supersedes it. Keyed by output name to match
    /// `frame_callback_sequence`.
    pub estimated_vblank_timers:
        std::collections::HashMap<String, smithay::reexports::calloop::RegistrationToken>,
    /// Per-output frame-clock state. Populated + consulted ONLY when
    /// `config.per_output_frame_clock` is true (the opt-in path). Each
    /// entry paces one output by its own refresh rate: a present timer
    /// re-armed off the output's last vblank, a dirty flag, and an
    /// in-flight gate. Keyed by output name. When the flag is false
    /// this map stays empty and the global-tick path is taken
    /// unchanged. See `backend::udev::per_output_clock`.
    pub per_output_clocks: std::collections::HashMap<String, crate::frame_clock::OutputClock>,
    /// Session-monotonic owner id for per-output present timers. Prevents a
    /// stale timer from a disable/replug cycle clearing a replacement clock.
    pub(crate) next_present_timer_id: u64,
    /// Earliest armed `wp_commit_timing_v1` deadline wake plus its ownership
    /// generation. The one-shot source is logically cancelled by replacing
    /// this pair — never via `LoopHandle::remove`. See `state/pacing.rs`.
    pub(crate) commit_timer_wake: Option<(smithay::wayland::commit_timing::Timestamp, u64)>,
    pub(crate) next_commit_timer_wake_id: u64,
    /// Outputs temporarily held behind a render/queue-error backoff. The map
    /// keeps the exact `Output` identity as well as its connector name: if a
    /// connector is unplugged and quickly recreated under the same name, an
    /// old one-shot timer must not consume the replacement output's retry.
    /// Sources are logically cancelled and never manually removed.
    pub(crate) render_retry_pending: std::collections::HashMap<String, (Output, u64)>,
    pub(crate) next_render_retry_id: u64,

    pub space: Space<Window>,
    pub popups: PopupManager,

    pub seat: Seat<MargoState>,

    pub display_handle: DisplayHandle,
    pub loop_handle: LoopHandle<'static, MargoState>,
    pub loop_signal: LoopSignal,
    /// `state snapshot` write is dirty (a change happened since the last
    /// flush). Coalesces a burst of per-change writes into one serialize
    /// per event-loop iteration — see `mark_state_dirty` /
    /// `flush_state_file_if_dirty`.
    pub state_dirty: std::cell::Cell<bool>,
    /// IPC socket server state (`ipc/server.rs`).
    pub ipc: IpcServer,
    /// Transient carousel slide direction for the next `view_tag`
    /// transition: 0 = derive from the tag-index delta (default),
    /// +1 = force forward, -1 = force backward. Set by relative tag
    /// navigation when `tag_carousel` wraps the first/last tag, then
    /// consumed + reset inside `view_tag`.
    pub tag_carousel_dir: i8,
    /// Debounce latch for edge-scroller pointer focus: armed (true)
    /// while the pointer is away from a scroller's leading/trailing
    /// edge; cleared on a focus shift so resting at the edge fires
    /// exactly once until the pointer leaves and re-enters.
    pub edge_scroller_armed: bool,
    pub clock: Clock<Monotonic>,
    pub should_quit: bool,
    /// Set whenever something dirties the scene. Drained by the udev/winit
    /// backend before each render. Source-of-truth for "does anything need
    /// to be redrawn this iteration"; the [`repaint_ping`] is only the
    /// wake-up mechanism, not the state.
    repaint_requested: bool,
    /// On-demand wake source for the redraw scheduler. The udev backend
    /// installs a calloop `Ping` source whose callback runs the render
    /// path; here we keep a sender so [`MargoState::request_repaint`] can
    /// poke it from anywhere (input handlers, commit hooks, animation
    /// ticks, IPC dispatch). Idle = no pings = loop stays asleep instead
    /// of waking 60 Hz from a polling timer.
    repaint_ping: Option<Ping>,
    /// Number of `queue_frame()` calls awaiting their matching
    /// `DrmEvent::VBlank`. Acts as a rate limiter for the redraw
    /// scheduler: while >0, [`request_repaint`] still flags the scene
    /// dirty but does *not* ping — the post-dispatch animation tick
    /// would otherwise re-arm a repaint on every loop iteration and the
    /// ping callback would fire immediately, rendering on the CPU as
    /// fast as the loop can spin (between vblanks). The vblank handler
    /// decrements this and, if zero, re-emits the deferred ping.
    pending_vblanks: u32,
    pub(crate) config_path: Option<PathBuf>,
    /// Where reloads save config generations ("last good" files boot can
    /// fall back to): `generations::generations_dir()`; tests point it at
    /// a scratch directory.
    pub(crate) generations_dir: PathBuf,
    /// `auto_reload_config` (default on) — live filesystem watcher over
    /// the config file + everything it currently `source`s/`include`s.
    /// `None` when the setting is off or the watcher itself couldn't be
    /// created; `sync_config_watcher` (called at startup and after every
    /// reload) keeps it started/stopped/re-synced to match.
    config_watcher: Option<crate::config_watcher::ConfigWatcher>,
    /// Every file the applied config was parsed from (canonical paths),
    /// from the last successful parse — the watcher's file set.
    config_sources: Vec<std::path::PathBuf>,
    /// [`config_fingerprint`] of `config_sources` as last applied.
    pub(crate) config_fingerprint: Option<u64>,
    /// Where `config_watcher`'s background thread sends once a batch of
    /// filesystem changes has settled — see `set_config_reload_sender`.
    config_reload_tx: Option<calloop::channel::Sender<()>>,

    pub config: Config,
    /// Cached "does any window rule carry a title / exclude_title
    /// pattern?" — recomputed only when `config` changes (startup +
    /// `reload_config`). Title commits fire often (browser tabs,
    /// terminal titles); without this every one re-scanned the whole
    /// `window_rules` list just to decide whether a title change could
    /// possibly re-match a rule.
    pub title_rules_exist: bool,
    /// Snapshot of theme-relevant `Config` fields captured the first
    /// time `apply_theme_preset` runs. `Theme::Default` resets to
    /// this snapshot; the snapshot is also reset on `mctl reload` so
    /// "default" always means "what config.conf says today".
    pub(crate) theme_baseline: Option<ThemeBaseline>,
    pub animation_curves: AnimationCurves,
    pub clients: Vec<MargoClient>,
    pub monitors: Vec<MargoMonitor>,

    /// Monotonic allocator for tabbed-window-group ids. Starts at 1
    /// (`0` is never a valid group id) and only ever increments, so
    /// a freed id is never reused for the lifetime of the session —
    /// keeps `group_id` comparisons unambiguous. See `state::groups`.
    pub next_group_id: u32,
    /// When set, `togglegroup` is a no-op (Hyprland `lockgroups`):
    /// prevents windows from accidentally merging into / splitting
    /// out of groups while the user arranges them. Existing groups
    /// keep working (cycle / move); only group/ungroup is frozen.
    pub groups_locked: bool,

    pub input_keyboard: KeyboardState,
    pub input_pointer: PointerState,
    pub input_touch: TouchState,
    pub input_gesture: GestureState,

    /// Client id of the drag-tile-to-tile swap target currently under the
    /// cursor, updated every motion tick by `MoveSurfaceGrab` and cleared
    /// when the grab ends. `None` outside of an active tiled/`Mosaic`
    /// drag. Purely a rendering hint (`render::drag_highlight`) — the
    /// actual swap on release goes through the same lookup
    /// (`input::grabs::find_drag_tile_target`) independently, so a stale
    /// value here can at worst mis-highlight for a frame, never mis-swap.
    pub drag_swap_target: Option<u64>,

    /// Last-writer-wins signal deciding which monitor a keybind/IPC menu
    /// open targets. Written by `refresh_pointer_monitor_tracking`
    /// (→ `Pointer`) and the keyboard keybind dispatch path (→ `Focus`);
    /// read in `build_state_snapshot` to compute `active_output`.
    pub active_output_source: ActiveOutputSource,

    /// Stable per-surface render-element ids for the decorations margo
    /// draws itself — the drop shadow and the background blur, keyed by
    /// the anchoring surface's `ObjectId` to `(shadow_id, blur_id)`. The
    /// smithay damage tracker keys element identity on `Id`; the render
    /// path used to hand it a fresh `Id::new()` every frame, so it treated
    /// every shadow/blur as brand-new and re-damaged its whole (oversized)
    /// bbox each frame — any unrelated repaint then redrew every floating
    /// window's decorations. A stable id lets unchanged decorations report
    /// zero damage. Entries are dropped when their surface's window or
    /// layer goes away (`forget_decoration_ids`).
    pub decoration_ids: std::cell::RefCell<
        std::collections::HashMap<
            smithay::reexports::wayland_server::backend::ObjectId,
            DecorationIds,
        >,
    >,

    pub layer_surfaces: Vec<LayerSurface>,
    pub lock_surfaces: Vec<(Output, smithay::wayland::session_lock::LockSurface)>,

    pub session_locked: bool,
    /// The `ext_session_lock_v1` object that owns the current lock. Kept so
    /// a second client can't take over a lock whose owner is still alive,
    /// and so `force_unlock` only acts once that owner is gone.
    pub session_lock_owner:
        Option<smithay::reexports::wayland_protocols::ext::session_lock::v1::server::ext_session_lock_v1::ExtSessionLockV1>,
    /// Human-readable name of the keyboard's currently active xkb
    /// layout (e.g. "English (US)", "Turkish"). Cached here and
    /// published in state snapshot so the shell's keyboard-layout pill
    /// can show it without re-deriving the keymap. Updated on every
    /// key event that changes the effective layout group.
    pub current_kb_layout: String,
    pub enable_gaps: bool,
    pub cursor_status: CursorImageStatus,
    pub cursor_manager: CursorManager,
    /// Animation tick pacing (see [`crate::animation::animation_tick_due`]):
    /// set by every present (`send_frame_callbacks`), cleared by the tick.
    pub animation_presented_since_tick: bool,
    /// Whether anything was still animating after the last tick.
    pub animating_at_last_tick: bool,
    /// `utils::now_ms()` of the last animation tick.
    pub animation_last_tick_ms: u32,
    /// Time the last animation tick sampled at: its frame's predicted
    /// presentation time (see [`crate::animation::animation_sample_ms`]).
    pub animation_last_sample_ms: u32,
    /// When each output (by name) last presented, real or estimated,
    /// stamped by `send_frame_callbacks`. Feeds the animation sample-time
    /// prediction where no per-output frame clock tracks the vblank.
    pub last_present_at: std::collections::HashMap<String, std::time::Instant>,
    /// An output queued a frame in the current repaint batch, so
    /// `finish_repaint_batch` owes the space / popup / protocol refresh.
    pub post_repaint_pending: bool,
    /// Compositor-painted wallpaper. `None` when no path resolves or
    /// the decode failed at startup — frame loop falls through to the
    /// solid `rootcolor` clear in that case. Re-decoded by
    /// `reload_config` if the path changes.
    pub wallpaper: Option<crate::wallpaper::WallpaperState>,
    /// Optional image painted behind the scroller-overview cells (the
    /// `overview_backdrop_image` config knob). `None` → the solid
    /// `overview_backdrop_color` is used instead. Decoded at startup and
    /// re-decoded by `reload_config` so Settings → Overview applies live.
    pub overview_backdrop: Option<crate::wallpaper::WallpaperState>,
    /// Generation of the latest overview-backdrop decode request; a decode
    /// that finishes after a newer request is dropped.
    pub overview_backdrop_request: u64,
    pub xwm: Option<X11Wm>,
    /// Latest position an override-redirect X11 surface (menu / popup /
    /// tooltip) was configured to, keyed by `window_id`. Toolkits move the
    /// popup to its anchor via a ConfigureNotify that can arrive BEFORE the
    /// map event, at which point the surface isn't in the space yet and
    /// `X11Surface::geometry()` still reads its stale (0,0) creation rect —
    /// so we stash the configured location here and consume it when the
    /// surface is finally mapped. Cleared on unmap/destroy.
    pub or_positions: std::collections::HashMap<u32, (i32, i32)>,
    pub libinput: Option<smithay::reexports::input::Libinput>,
    /// `true` while ANY output is DPMS-off. Lets `handle_input` cheaply gate
    /// the "any input wakes the screen" safety net without touching the
    /// backend, and lets a `dpms toggle` decide direction.
    pub any_dpms_off: bool,
    /// When the last DPMS-off was requested. `wake_dpms_on_input` ignores
    /// input within a short grace after this so the very keystroke / click
    /// that triggered the off (its release event, the Enter that ran `mctl`,
    /// residual pointer motion) can't immediately wake the panel back up.
    pub dpms_off_at: Option<std::time::Instant>,
    pub libinput_devices: Vec<smithay::reexports::input::Device>,
    /// Windows that have been requested to close but are still on screen
    /// for the duration of the close animation. Each entry carries a
    /// captured `GlesTexture` of the window's last visible frame plus
    /// the geometry / monitor / tags it was on, so the renderer can
    /// keep painting it after the live `wl_surface` is gone.
    /// `tick_animations` advances each entry's progress and pops it
    /// when the curve settles. Pending captures (the wl_surface was
    /// still alive at destruction time but we hadn't rendered yet)
    /// live as `None` in `texture` until the next render fills them in.
    pub closing_clients: Vec<ClosingClient>,
    /// Discovered Rhai plugins (W3.3). Empty when no
    /// `~/.config/margo/plugins/` exists; populated by
    /// `init_plugins` after init_user_scripting. Stored on state
    /// so `mctl plugin list` (future) can enumerate without
    /// re-walking the FS.
    pub plugins: Vec<crate::plugin::Plugin>,
    /// AccessKit accessibility-tree adapter (W2.4). `start()` is
    /// called once at compositor init; subsequent
    /// `publish_window_list` calls flush a fresh tree on every
    /// arrange + focus change. Off by default — only built when
    /// the `a11y` feature is on.
    #[cfg(feature = "a11y")]
    pub a11y: crate::a11y::A11yState,
    /// Layer surfaces in their open / close animation. Keyed by the
    /// layer's wl_surface object id so the render path can look up
    /// the per-layer animation state without an O(n) scan. Each
    /// entry tracks both directions: `is_close` flips at
    /// `layer_destroyed`. After settling, open entries get popped
    /// in `tick_animations` (cleared from the map); close entries
    /// also drop the captured texture along with the entry.
    pub layer_animations: std::collections::HashMap<
        smithay::reexports::wayland_server::backend::ObjectId,
        LayerSurfaceAnim,
    >,
    /// Per-arrange override for the move-animation duration (in ms). Set
    /// by `open_overview` / `close_overview` so the overview transition
    /// uses a snappy ~180 ms slide instead of the full
    /// `animation_duration_move`. `arrange_monitor` reads this and
    /// `open_overview` / `close_overview` clear it after their batched
    /// arrange is done. None ⇒ fall back to the configured duration.
    pub overview_transition_animation_ms: Option<u32>,

    /// Diagnostics from the most recent `reload_config` validation pass.
    /// Empty when the last reload was clean (or no reload has happened
    /// yet). Populated by `reload_config` after running
    /// `margo_config::validator::validate_config`. Queryable from
    /// userspace via `mctl config-errors`. The compositor keeps its
    /// previous config when `has_errors()` is true, so this field
    /// doubles as "why did the last reload not apply?".
    pub last_reload_diagnostics: Vec<margo_config::diagnostics::ConfigDiagnostic>,
    /// `Instant` the config-error overlay first appeared on screen.
    /// Cleared on a clean reload or after the banner's display
    /// window expires (driven by `tick_animations`). Drives the
    /// niri-style red-bordered banner pinned to the active output's
    /// top-right corner.
    pub config_error_overlay_until: Option<std::time::Instant>,
    /// Persistent SolidColorBuffers backing the config-error banner.
    /// Kept on `MargoState` (rather than allocated per-frame) so the
    /// buffers' Ids stay stable across frames and damage tracking
    /// stays tight.
    pub config_error_overlay: crate::render::config_error_overlay::ConfigErrorOverlay,

    /// Alt+Tab muscle-memory: when an `overview_focus_next/prev` keybind
    /// fires, the input handler snapshots which modifier(s) the user is
    /// holding and sets `overview_cycle_pending = true`. On the next key
    /// release event whose modifier state no longer overlaps that snapshot
    /// (i.e. the user let go of Alt/Super/whatever they were holding),
    /// the input handler calls `overview_activate` to commit the cycle's
    /// pick — closing overview onto the highlighted thumbnail. This is
    /// the standard Win/GNOME/Hypr "hold modifier, tap Tab to cycle,
    /// release modifier to confirm" behaviour. Cleared by
    /// `overview_activate`, `close_overview`, and `open_overview`.
    pub overview_cycle_pending: bool,
    pub overview_cycle_modifier_mask: margo_config::Modifiers,

    /// niri-style MRU window switcher (Super+Tab). `Some` while open. See
    /// `state/mru_switcher.rs`.
    pub mru_switcher: Option<mru_switcher::MruSwitcher>,
    /// Modifier mask snapshotted by the input handler when an `mru_next/prev`
    /// keybind fires, consumed when the switcher opens (release-to-commit).
    pub mru_open_mask: margo_config::Modifiers,
    /// Monotonic focus counter; each focused client records the current value
    /// as its `last_focus_serial`, giving a stable MRU order.
    pub focus_counter: u64,

    /// niri-style scroller overview: a zoomed-out, scrollable strip of
    /// per-tag mini-desktops. Entirely separate from the classic grid
    /// overview above (`is_overview` / `overview_*`) — `None` when
    /// closed. See [`ScrollerOverview`] and `state/scroller_overview.rs`.
    pub scroller_overview: Option<ScrollerOverview>,

    /// Pointer/input dwell and throttle timers (`state/input_timers.rs`).
    pub input_timers: InputTimers,

    /// Twilight (blue-light filter) engine and its timer (`state/twilight_methods.rs`).
    pub twilight: Twilight,

    /// Source of the keyboard event being processed: physical, or a
    /// `zwp_virtual_keyboard_v1` device (smithay tracks held keys per
    /// source). `virtual_input` marks the latter for the binding filter.
    pub keyboard_source: smithay::input::keyboard::KeyboardSource,
    pub virtual_input: bool,
    /// Keymap of the virtual keyboard currently loaded into the seat
    /// (`None` = the configured xkb keymap is active).
    pub active_virtual_keymap: Option<std::sync::Arc<str>>,
    /// Virtual keyboards that set modifiers directly (`modifiers`
    /// request), so their removal can clear what they left held.
    pub virtual_modifier_devices: std::collections::HashSet<String>,
    /// `session_end` in progress (windows asked to close, waiting).
    pub session_ending: Option<session_end::SessionEnd>,
}

impl MargoState {
    pub fn new(
        config: Config,
        display: &mut Display<MargoState>,
        loop_handle: LoopHandle<'static, MargoState>,
        loop_signal: LoopSignal,
        config_path: Option<PathBuf>,
    ) -> Self {
        let dh = display.handle();
        let protocols = Protocols::new(&dh, &config, &loop_handle);
        let mut seat_state = SeatState::new();
        let seat = seat_state.new_wl_seat(&dh, "seat0");
        let space = Space::default();
        let popups = PopupManager::default();
        let animation_curves = AnimationCurves::bake(&config);
        let input_keyboard = KeyboardState::new(&config);
        Self {
            protocols,
            dmabuf_import_hook: None,
            seat_state,
            output_pending: OutputPending::default(),
            drm_lease_active: Vec::new(),
            pending_crash_restore: None,
            global_shortcuts: Default::default(),
            udev_backend: None,
            scripting: None,
            script_hooks: Default::default(),
            #[cfg(feature = "xdp-gnome-screencast")]
            screencasting: None,
            #[cfg(feature = "dbus")]
            dbus_servers: crate::dbus::DBusServers::default(),
            #[cfg(feature = "dbus")]
            ipc_outputs: std::sync::Arc::new(std::sync::Mutex::new(
                crate::dbus::ipc_output::IpcOutputMap::new(),
            )),
            cast_gbm: None,
            cast_render_formats: Default::default(),
            image_copy_capture_sessions: Vec::new(),
            pending_image_copy_frames: Vec::new(),
            space,
            popups,
            seat,
            display_handle: dh,
            loop_handle,
            loop_signal,
            state_dirty: std::cell::Cell::new(false),
            ipc: crate::ipc::server::IpcServer::default(),
            tag_carousel_dir: 0,
            edge_scroller_armed: true,
            clock: Clock::new(),
            should_quit: false,
            repaint_requested: true,
            repaint_ping: None,
            pending_vblanks: 0,
            config_path,
            generations_dir: margo_config::generations::generations_dir(),
            // Set up once the full `event_loop` handle exists — see
            // `main.rs`'s post-construction setup.
            config_watcher: None,
            config_sources: Vec::new(),
            config_fingerprint: None,
            config_reload_tx: None,
            animation_curves,
            input_keyboard,
            input_pointer: Default::default(),
            input_touch: Default::default(),
            input_gesture: Default::default(),
            drag_swap_target: None,
            active_output_source: ActiveOutputSource::default(),
            decoration_ids: std::cell::RefCell::new(std::collections::HashMap::new()),
            layer_surfaces: vec![],
            lock_surfaces: vec![],
            clients: vec![],
            monitors: vec![],
            next_group_id: 1,
            groups_locked: false,
            session_locked: false,
            session_lock_owner: None,
            current_kb_layout: String::new(),
            keyboard_shortcuts_inhibiting_surfaces: std::collections::HashMap::new(),
            idle_inhibitors: std::collections::HashSet::new(),
            rule_once_consumed: std::collections::HashSet::new(),
            layer_layout_hashes: std::collections::HashMap::new(),
            layer_kb_interactivity_hashes: std::collections::HashMap::new(),
            layer_buffer_mapped: std::collections::HashSet::new(),
            frame_callback_sequence: std::collections::HashMap::new(),
            perf_counters: std::collections::HashMap::new(),
            frame_callback_fallback_output: Output::new(
                String::new(),
                smithay::output::PhysicalProperties {
                    size: (0, 0).into(),
                    subpixel: smithay::output::Subpixel::Unknown,
                    make: String::new(),
                    model: String::new(),
                    serial_number: String::new(),
                },
            ),
            estimated_vblank_timers: std::collections::HashMap::new(),
            per_output_clocks: std::collections::HashMap::new(),
            next_present_timer_id: 0,
            commit_timer_wake: None,
            next_commit_timer_wake_id: 0,
            render_retry_pending: std::collections::HashMap::new(),
            next_render_retry_id: 0,
            enable_gaps: config.enable_gaps,
            cursor_status: CursorImageStatus::default_named(),
            cursor_manager: CursorManager::new(),
            animation_presented_since_tick: false,
            animating_at_last_tick: false,
            animation_last_tick_ms: 0,
            animation_last_sample_ms: 0,
            last_present_at: std::collections::HashMap::new(),
            post_repaint_pending: false,
            wallpaper: crate::wallpaper::WallpaperState::load(config.wallpaper.as_deref()),
            overview_backdrop: config
                .overview_backdrop_image
                .as_deref()
                .and_then(crate::wallpaper::WallpaperState::load_exact),
            overview_backdrop_request: 0,
            xwm: None,
            or_positions: std::collections::HashMap::new(),
            libinput: None,
            any_dpms_off: false,
            dpms_off_at: None,
            libinput_devices: Vec::new(),
            closing_clients: Vec::new(),
            plugins: Vec::new(),
            #[cfg(feature = "a11y")]
            a11y: crate::a11y::A11yState::new(),
            layer_animations: std::collections::HashMap::new(),
            overview_transition_animation_ms: None,
            last_reload_diagnostics: Vec::new(),
            config_error_overlay_until: None,
            config_error_overlay: crate::render::config_error_overlay::ConfigErrorOverlay::new(),
            overview_cycle_pending: false,
            mru_switcher: None,
            mru_open_mask: margo_config::Modifiers::empty(),
            focus_counter: 0,
            overview_cycle_modifier_mask: margo_config::Modifiers::empty(),
            scroller_overview: None,
            input_timers: InputTimers::new(),
            twilight: Twilight::new(),
            keyboard_source: smithay::input::keyboard::KeyboardSource::MAIN,
            virtual_input: false,
            active_virtual_keymap: None,
            virtual_modifier_devices: std::collections::HashSet::new(),
            session_ending: None,
            title_rules_exist: config_has_title_rules(&config),
            config,
            theme_baseline: None,
        }
    }

    /// Path of the runtime state-file used by `mctl clients` /
    /// `mctl outputs`. Public so dispatch handlers can also
    /// trigger a write after non-arrange state changes.
    pub fn refresh_state_file(&self) {
        self.mark_state_dirty();
    }

    /// Notify xdp-gnome's window picker that the toplevel set changed
    /// so a live screencast share dialog refreshes its list. Fires
    /// the `org.gnome.Shell.Introspect.WindowsChanged` D-Bus signal
    /// against the registered `Introspect` interface. Cheap no-op if
    /// the D-Bus shim isn't running (no screencast portal use).
    /// On builds without the `dbus` feature this is a literal no-op
    /// — the call sites stay regardless so the rest of the
    /// codebase doesn't have to learn about the feature flag.
    pub fn emit_windows_changed(&self) {
        #[cfg(feature = "dbus")]
        if let Some(conn) = &self.dbus_servers.conn_introspect {
            crate::dbus::gnome_shell_introspect::emit_windows_changed_sync(conn);
        }
    }

    /// Re-build the shared `ipc_outputs` snapshot from the live
    /// `monitors` list. No-op without the `dbus` feature. Called
    /// from `remove_output` (hotplug-out) and from the udev
    /// backend's `setup_connector` (hotplug-in) so xdp-gnome's
    /// chooser dialog always reflects the actual output set —
    /// without this, a monitor unplugged mid-cast would still
    /// appear in the Entire Screen tab.
    #[cfg(feature = "dbus")]
    pub fn refresh_ipc_outputs(&self) {
        let snap = crate::dbus::ipc_output::snapshot(self);
        if let Ok(mut guard) = self.ipc_outputs.lock() {
            *guard = snap;
        }
    }

    /// No-op stub when dbus is off. Lets call sites in udev /
    /// state stay un-cfg-gated.
    #[cfg(not(feature = "dbus"))]
    pub fn refresh_ipc_outputs(&self) {}

    /// Rebuild the wlr-output-management snapshot from the current
    /// monitor list and publish it to all bound clients (kanshi,
    /// wlr-randr, way-displays, …). Cheap when nothing's changed:
    /// `snapshot_changed` early-returns on equal snapshots.
    pub fn publish_output_topology(&mut self) {
        let mut snap = std::collections::HashMap::new();
        for mon in &self.monitors {
            let pos = (mon.monitor_area.x, mon.monitor_area.y);
            snap.insert(
                mon.name.clone(),
                crate::protocols::output_management::snapshot_from_output(
                    &mon.output,
                    mon.enabled,
                    pos,
                ),
            );
        }
        self.protocols.output_management.snapshot_changed(snap);
    }

    pub fn remove_output(&mut self, output: &Output) {
        // Close and unmap the output's layers now. The client destroys the
        // surface later, when the monitor is already gone, so per-surface
        // bookkeeping is dropped here rather than in `layer_destroyed`.
        {
            let mut map = smithay::desktop::layer_map_for_output(output);
            let layers: Vec<_> = map.layers().cloned().collect();
            for layer in &layers {
                layer.layer_surface().send_close();
                let id = layer.wl_surface().id();
                self.layer_layout_hashes.remove(&id);
                self.layer_kb_interactivity_hashes.remove(&id);
                self.layer_buffer_mapped.remove(&id);
                map.unmap_layer(layer);
            }
        }
        self.layer_animations
            .retain(|_, anim| anim.output != *output);

        self.protocols.gamma_control_manager.output_removed(output);
        self.protocols.output_power_manager.output_removed(output);
        self.protocols.screencopy.remove_output(output);

        if let Some(pos) = self.monitors.iter().position(|m| m.output == *output) {
            tracing::info!(monitor = %self.monitors[pos].name, "removing monitor");
            self.monitors.remove(pos);
        }
        self.space.unmap_output(output);
        self.lock_surfaces.retain(|(o, _)| o != output);
        self.output_pending.gamma.retain(|(o, _)| o != output);
        // Per-output frame clock: drop this output's clock + cancel its
        // present timer so a gone display doesn't keep waking the loop
        // (no-op when the opt-in flag is off / no clock exists).
        self.drop_output_clock(&output.name());
        // Hotplug-out: refresh the shared D-Bus snapshot so
        // xdp-gnome's chooser dialog drops the now-gone output.
        self.refresh_ipc_outputs();
        // Keyboard focus may sit on something that just went with the
        // monitor: a lock surface (lid closed while docked) or an exclusive
        // layer such as the launcher. Move it now, with the pointer back on
        // a remaining output; otherwise keys go nowhere until the pointer
        // happens to move.
        self.clamp_pointer_to_outputs();
        self.refresh_keyboard_focus();
        self.refresh_pointer_focus();
        self.request_repaint();
    }

    /// Snapshot the client list and ship it to the AccessKit
    /// adapter so screen readers see the current toplevels +
    /// focus state. No-op without the `a11y` feature.
    #[cfg(feature = "a11y")]
    pub fn publish_a11y_window_list(&mut self) {
        let focused_idx = self.focused_client_idx();
        let snapshot: Vec<crate::a11y::WindowSnapshot> = self
            .clients
            .iter()
            .enumerate()
            .map(|(i, c)| crate::a11y::WindowSnapshot {
                app_id: c.app_id.clone(),
                title: c.title.clone(),
                is_focused: Some(i) == focused_idx,
            })
            .collect();
        self.a11y.publish_window_list(snapshot.iter());
    }

    /// Stub on builds without the `a11y` feature so call sites
    /// don't have to learn the feature flag.
    #[cfg(not(feature = "a11y"))]
    pub fn publish_a11y_window_list(&mut self) {}

    /// Force-tear-down a stuck session lock from the compositor side.
    ///
    /// Use case: the locker crashed or hung so there is nothing to type a
    /// password into, and recovery would otherwise mean a TTY. Reachable from
    /// the `force_unlock` action (keybind, whitelisted in `handle_keyboard`
    /// while locked, and `mctl dispatch`).
    ///
    /// It only acts once the owning locker is gone (see
    /// [`force_unlock_allowed`]). While the locker lives the request is
    /// logged and ignored, whatever its surfaces look like; otherwise the
    /// action would be a password bypass.
    ///
    /// We don't try to nicely tell the locker to release; we just clear our
    /// state, drop the lock surfaces, and re-show toplevels.
    pub fn force_unlock(&mut self) {
        if !self.session_locked && self.lock_surfaces.is_empty() {
            tracing::info!("force_unlock: nothing to do (already unlocked)");
            return;
        }
        let owner_alive = self
            .session_lock_owner
            .as_ref()
            .is_some_and(|lock| lock.is_alive());
        if !force_unlock_allowed(owner_alive) {
            tracing::warn!("force_unlock: ignored, a live locker holds the lock");
            return;
        }
        tracing::warn!(
            "force_unlock: tearing down stuck lock (lock_surfaces={})",
            self.lock_surfaces.len()
        );
        self.session_locked = false;
        self.session_lock_owner = None;
        self.lock_surfaces.clear();
        self.finish_lock_deferred_maps();
        self.arrange_all();
        self.refresh_keyboard_focus();
        let _ = crate::utils::spawn([
            "notify-send",
            "-a",
            "margo",
            "-i",
            "preferences-system",
            "-u",
            "critical",
            "-t",
            "3000",
            "Margo",
            "Lock force-cleared",
        ]);
    }
}

// ── Keyboard layout ──────────────────────────────────────────────────────────

impl MargoState {
    /// Re-read the seat keyboard's currently active xkb layout name
    /// and, if it changed since the last cache, store it + mark
    /// state snapshot dirty so the shell's keyboard-layout pill refreshes.
    /// Cheap (a keymap name lookup) — safe to call on every key event.
    pub fn refresh_keyboard_layout(&mut self) {
        let Some(kbd) = self.seat.get_keyboard() else {
            return;
        };
        let name = kbd.with_xkb_state(self, |ctx| {
            // Runs on every key event — recover a poisoned lock (the xkb
            // state is still valid to *read*) instead of panicking the
            // whole compositor on a keystroke if some other thread paniced
            // while holding it.
            let xkb = ctx.xkb().lock().unwrap_or_else(|e| e.into_inner());
            let layout = xkb.active_layout();
            xkb.layout_name(layout).to_string()
        });
        if self.current_kb_layout != name {
            self.current_kb_layout = name;
            self.mark_state_dirty();
        }
    }

    /// Cycle the seat keyboard to the next configured xkb layout,
    /// wrapping at the end. Triggered by the `cyclekblayout` dispatch
    /// action (mctl + the shell's keyboard-layout pill). No-op when a
    /// single layout is configured.
    pub fn cycle_keyboard_layout(&mut self) {
        let Some(kbd) = self.seat.get_keyboard() else {
            return;
        };
        kbd.with_xkb_state(self, |mut ctx| ctx.cycle_next_layout());
        self.refresh_keyboard_layout();
    }
}

// Single blanket bridge that replaces all the old per-protocol
// `delegate_*!` macros (removed upstream in the smithay Dispatch2
// rework, 2026-04-30). It implements `Dispatch`/`GlobalDispatch` for
// every resource whose user-data implements smithay's
// `Dispatch2`/`GlobalDispatch2` — i.e. every protocol whose Handler
// trait we already impl. margo's own protocol modules keep their
// hand-written `wayland_server::delegate_dispatch!` impls; those use
// concrete (local) user-data types that don't implement `Dispatch2`,
// so they don't overlap with this blanket.
smithay::delegate_dispatch2!(MargoState);

// ── ForeignToplevelListHandler ────────────────────────────────────────────────

impl ForeignToplevelListHandler for MargoState {
    fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelListState {
        &mut self.protocols.foreign_toplevel_list
    }
}

// ── XwmHandler: X11 window management ────────────────────────────────────────

impl MargoState {
    /// Stable element ids for a surface's self-drawn decorations,
    /// created once and reused across frames. See
    /// [`MargoState::decoration_ids`]. `Id` is a cheap clonable handle.
    pub fn decoration_element_ids(
        &self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) -> DecorationIds {
        self.decoration_ids
            .borrow_mut()
            .entry(surface.id())
            .or_insert_with(DecorationIds::new)
            .clone()
    }

    /// Drop a destroyed surface's decoration ids.
    pub fn forget_decoration_ids(
        &self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) {
        self.decoration_ids.borrow_mut().remove(&surface.id());
    }
}

/// Per-surface stable render-element ids (see
/// [`MargoState::decoration_element_ids`]).
#[derive(Clone)]
pub struct DecorationIds {
    pub shadow: smithay::backend::renderer::element::Id,
    pub blur: smithay::backend::renderer::element::Id,
    pub dim: smithay::backend::renderer::element::Id,
    /// The surface's own chip in its group's tab strip.
    pub group_chip: smithay::backend::renderer::element::Id,
}

impl DecorationIds {
    fn new() -> Self {
        use smithay::backend::renderer::element::Id;
        Self {
            shadow: Id::new(),
            blur: Id::new(),
            dim: Id::new(),
            group_chip: Id::new(),
        }
    }
}

// XWayland client-lifecycle methods (find/register/remove) live in
// `state::xwayland`.
