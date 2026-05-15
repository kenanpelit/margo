#![allow(dead_code)]

// W4.2: per-protocol handler impls extracted into sibling files
// under `state/handlers/` for incremental-compile wins. Each
// submodule reaches into `MargoState` via `crate::state::MargoState`.
mod handlers;

// Roadmap Q1 — extracting pure state-internal helpers out of the
// 6800-line state.rs into siblings under `state/`. Theme is the
// first step; others follow in the same form.
mod animation_tick;
mod data;
mod debug_dump;
mod dispatch;
mod focus_target;
mod overview;
mod scratchpad;
mod screencast;
mod state_file;
mod theme;
mod twilight_methods;

pub use self::animation_tick::{tick_animations, AnimTickSpec};
pub use self::data::{
    ClosingClient, FullscreenMode, HotCorner, LayerSurfaceAnim, MargoClient, MargoMonitor,
    ResizeSnapshot,
};
pub(crate) use self::data::{
    clamp_size, matches_layer_name, matches_rule_text, read_toplevel_identity, WindowRuleReason,
};
pub use self::focus_target::FocusTarget;
pub(crate) use self::theme::ThemeBaseline;

use std::{cell::RefCell, path::PathBuf, rc::Rc};

use anyhow::{Context, Result};
use smithay::{
    backend::allocator::dmabuf::Dmabuf,
    delegate_output,
    delegate_seat, delegate_shm,
    delegate_presentation,
    desktop::{layer_map_for_output, PopupManager, Space, Window, WindowSurface},
    input::{
        Seat, SeatHandler, SeatState,
        pointer::CursorImageStatus,
    },
    output::Output,
    reexports::{
        calloop::{ping::Ping, LoopHandle, LoopSignal},
        wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as XdgDecorationMode,
        wayland_server::{
            DisplayHandle, Resource,
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
            Display,
        },
    },
    utils::{Clock, Monotonic, Size, SERIAL_COUNTER},
    wayland::{
        compositor::{with_states, CompositorClientState, CompositorState},
        output::{OutputHandler, OutputManagerState},
        seat::WaylandFocus,
        selection::{
            data_device::{set_data_device_focus, DataDeviceState},
            ext_data_control::DataControlState as ExtDataControlState,
            primary_selection::{set_primary_focus, PrimarySelectionState},
            wlr_data_control::DataControlState,
        },
        shell::{
            wlr_layer::WlrLayerShellState,
            xdg::{
                decoration::XdgDecorationState,
                ToplevelSurface, XdgShellState, XdgToplevelSurfaceData,
            },
        },
        shm::{ShmHandler, ShmState},
        input_method::InputMethodManagerState,
        pointer_constraints::PointerConstraintsState,
        presentation::PresentationState,
        relative_pointer::RelativePointerManagerState,
        text_input::TextInputManagerState,
        xdg_activation::XdgActivationState,
        viewporter::ViewporterState,
        dmabuf::{DmabufGlobal, DmabufState},
        drm_syncobj::DrmSyncobjState,
        xwayland_shell::XWaylandShellState,
    },
    xwayland::{X11Surface, X11Wm},
};

use margo_config::{parse_config, Config, WindowRule};

/// Filesystem path of the runtime state file consumed by mctl's
/// rich subcommands (`clients`, `outputs`, the prettier
/// `status`). Default location: `$XDG_RUNTIME_DIR/margo/state.json`,
/// fallback `/run/user/$UID/margo/state.json` if XDG isn't set.
pub fn state_file_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let uid = unsafe { libc::getuid() };
            PathBuf::from(format!("/run/user/{uid}"))
        });
    dir.join("margo").join("state.json")
}

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
}

impl ClientData for MargoClientData {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
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

#[allow(dead_code)]
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

pub struct MargoState {
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub xdg_decoration_state: XdgDecorationState,
    pub shm_state: ShmState,
    pub viewporter_state: ViewporterState,
    pub dmabuf_state: DmabufState,
    pub dmabuf_global: Option<DmabufGlobal>,
    pub dmabuf_import_hook: Option<DmabufImportHook>,
    /// `wp_linux_drm_syncobj_v1` global state. `None` until the udev
    /// backend opens the primary DRM node and confirms it supports
    /// `syncobj_eventfd` — older kernels (< 5.18) and devices without
    /// `DRM_CAP_SYNCOBJ_TIMELINE` can't drive explicit-sync, so we
    /// don't expose the protocol there. Modern Chromium / Firefox
    /// prefers explicit sync when the global is advertised: per-
    /// surface `wp_linux_drm_syncobj_surface_v1` carries acquire +
    /// release fences alongside the dmabuf, eliminating the implicit
    /// fence wait that otherwise drops frames under GPU load.
    pub drm_syncobj_state: Option<DrmSyncobjState>,
    pub seat_state: SeatState<MargoState>,
    pub layer_shell_state: WlrLayerShellState,
    pub output_manager_state: OutputManagerState,
    pub data_device_state: DataDeviceState,
    pub primary_selection_state: PrimarySelectionState,
    pub data_control_state: DataControlState,
    /// `ext_data_control_v1` — standardized successor to
    /// `wlr_data_control_v1`. mshell-clipboard / wl-clipboard 3.x
    /// prefer this; without it the clipboard watcher dies at
    /// startup with "Missing ext_data_control_manager_v1 or wl_seat".
    pub ext_data_control_state: ExtDataControlState,
    pub session_lock_state: smithay::wayland::session_lock::SessionLockManagerState,
    /// `wp_text_input_v3` global. Qt clients (Quickshell/noctalia, KDE,
    /// QtWidgets apps) probe for this when a TextInput field becomes
    /// active — without it, Qt's QML password fields silently drop
    /// keystrokes on the lock screen even when wl_keyboard.enter is
    /// delivered. We don't drive an IME ourselves; smithay routes the
    /// protocol traffic correctly with just the global registered.
    pub text_input_state: TextInputManagerState,
    /// `zwp_input_method_v2` global. Goes hand-in-hand with text_input —
    /// Qt's text-input plugin won't activate without both.
    pub input_method_state: InputMethodManagerState,
    /// `zwp_pointer_constraints_v1` global. Lets clients request that
    /// the pointer be locked (held in place, FPS games / Blender's
    /// rotate-around-camera) or confined to a region (Krita canvas
    /// drag, remote-desktop client). Activated through
    /// `PointerConstraintsHandler::new_constraint`; honoured in
    /// `handle_pointer_motion`.
    pub pointer_constraints_state: PointerConstraintsState,
    /// `zwp_relative_pointer_manager_v1` global. Required complement
    /// to pointer constraints — when the pointer is locked, clients
    /// still need to know the cursor *would have moved by Δ*. Each
    /// pointer-motion event already calls `pointer.relative_motion`,
    /// so all this state needs to do is exist so clients can bind
    /// the global and get a `wp_relative_pointer_v1` per pointer.
    pub relative_pointer_state: RelativePointerManagerState,
    /// `xdg_activation_v1` global. The polite focus-stealing
    /// channel: launchers (rofi, wofi, xdg-desktop-portal-wlr's
    /// activate request), notification daemons (notify-send action
    /// buttons), and chained-launcher flows (browser handles a
    /// mailto: by activating the running mail client) hand a token
    /// to the target surface; we honour or reject it. We accept
    /// when the request comes with a valid recent keyboard
    /// interaction serial (ie. the user was actively typing on the
    /// requesting client when it generated the token), reject
    /// otherwise — that's the spec-recommended anti-focus-steal
    /// gate.
    pub xdg_activation_state: XdgActivationState,
    /// `wlr_output_management_v1` state. Lets `kanshi`,
    /// `wlr-randr`, `way-displays` etc. discover the output
    /// topology and apply scale / transform / position changes
    /// at runtime. Disable still rejected; mode changes are now
    /// queued via `pending_output_mode_changes` for the udev
    /// backend to drain at the next repaint.
    pub output_management_state:
        crate::protocols::output_management::OutputManagementManagerState,
    /// Mode changes accepted by `apply_output_pending` but not yet
    /// applied at the DRM layer. The udev repaint handler drains
    /// this and feeds each entry through `DrmCompositor::use_mode`,
    /// then updates the smithay `Output` state so wl_output mode
    /// events fire for any client (kanshi watcher, status bar).
    /// Held outside the apply path because the handler runs on
    /// MargoState and doesn't have a borrow on the udev BackendData.
    pub pending_output_mode_changes: Vec<crate::PendingOutputModeChange>,
    /// `wp_color_management_v1` (staging) — Phase 1 scaffolding.
    /// The global is registered so Chromium / mpv probe-detection
    /// finds a colour-managed compositor and lights up their HDR
    /// decode paths. Composite stays sRGB; per-surface descriptions
    /// are stored on the surface tracker, not yet read by render.
    /// Phase 2 (linear-light fp16 composite) consumes these.
    pub color_management_state: crate::protocols::color_management::ColorManagementState,
    /// User-script engine + compiled AST + registered event hooks.
    /// `None` if no `~/.config/margo/init.rhai` is present. Boxed so
    /// the field is small + we can `Option::take()` it during hook
    /// invocation (the recursion guard + borrow-checker dance lives
    /// in `scripting::fire_hook`).
    pub scripting: Option<Box<crate::scripting::ScriptingState>>,
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
    pub cast_gbm: Option<smithay::backend::allocator::gbm::GbmDevice<smithay::backend::drm::DrmDeviceFd>>,
    /// Renderer-side dmabuf format constraints, snapshotted at
    /// backend init so the screencast cast lifecycle has them
    /// without crossing the borrow boundary into the udev
    /// renderer mid-D-Bus-call.
    pub cast_render_formats: smithay::backend::allocator::format::FormatSet,
    /// `ext-image-capture-source-v1` core state. Mints opaque
    /// source handles that clients pass to ext-image-copy-capture
    /// to identify what they want to capture. xdp-wlr 0.8+ uses
    /// these for the per-window screencast path.
    pub image_capture_source_state:
        smithay::wayland::image_capture_source::ImageCaptureSourceState,
    /// `ext-output-image-capture-source-manager-v1` global —
    /// "give me a capture source for this wl_output". Backs the
    /// monitor-share path in xdp-wlr.
    pub output_capture_source_state:
        smithay::wayland::image_capture_source::OutputCaptureSourceState,
    /// `ext-foreign-toplevel-image-capture-source-manager-v1`
    /// global — "give me a capture source for this toplevel".
    /// Margo's `ForeignToplevelListState` already implements the
    /// matching `ext-foreign-toplevel-list-v1`, so xdp-wlr can
    /// enumerate windows + ask for per-window capture; this is
    /// the protocol that lights up the **Window tab** in
    /// browser-based meeting clients (Google Meet, Zoom Web,
    /// Discord, Jitsi).
    pub toplevel_capture_source_state:
        smithay::wayland::image_capture_source::ToplevelCaptureSourceState,
    /// `ext-image-copy-capture-v1` — the actual capture transport.
    /// Clients open a session against an `ImageCaptureSource`,
    /// receive buffer constraints, allocate a matching buffer,
    /// then request a frame which margo renders into the buffer.
    pub image_copy_capture_state:
        smithay::wayland::image_copy_capture::ImageCopyCaptureState,
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
    /// `wp_presentation` global. Lets clients (kitty, mpv, native
    /// Wayland Vulkan games via DXVK / VKD3D, video conferencing
    /// apps that adapt their pacing to the actual display refresh)
    /// register `wp_presentation_feedback` per-frame and learn the
    /// real `presented` timestamp + refresh interval. Without this
    /// they're stuck guessing — kitty falls back to a 60 Hz tick,
    /// mpv ships its own debouncer, vsync-sensitive games stutter.
    pub presentation_state: PresentationState,
    /// `ext_idle_notifier_v1`: pings clients (swayidle, noctalia) once
    /// the seat has been idle for the duration they registered.
    pub idle_notifier_state: smithay::wayland::idle_notify::IdleNotifierState<MargoState>,
    /// `zwp_idle_inhibit_manager_v1`: clients (mpv, video players,
    /// presentation tools) can request "don't go idle while my surface
    /// is on screen". The notifier is paused while the set is non-empty.
    pub idle_inhibit_state: smithay::wayland::idle_inhibit::IdleInhibitManagerState,
    /// `zwp_virtual_keyboard_v1`: lets unprivileged clients (wayvnc
    /// for VNC sessions, `wtype` / `ydotool` for synthetic input,
    /// IMEs) inject key events into the focused surface. The state
    /// is stored only so `delegate_virtual_keyboard_manager!` has a
    /// home; smithay drives the global from there.
    pub virtual_keyboard_manager_state:
        smithay::wayland::virtual_keyboard::VirtualKeyboardManagerState,
    /// Surfaces that have an active idle-inhibit object. We feed
    /// `!is_empty()` to the notifier whenever this set changes.
    pub idle_inhibitors: std::collections::HashSet<
        smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    >,
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
    pub layer_layout_hashes: std::collections::HashMap<
        smithay::reexports::wayland_server::backend::ObjectId,
        u64,
    >,
    /// Per-layer-surface hash of *just* `keyboard_interactivity`,
    /// tracked separately from `layer_layout_hashes` so we can
    /// independently dedup focus-refresh from arrange-refresh.
    /// noctalia's launcher/settings panels flip
    /// keyboard_interactivity between `Exclusive` and `None` on the
    /// same surface and need focus recomputed when that happens;
    /// mshell's bar never flips it during normal updates, so layered
    /// content commits (clock tick, network speed, CPU stats) must
    /// NOT pay the focus-refresh cost. Cleared in `layer_destroyed`.
    pub layer_kb_interactivity_hashes: std::collections::HashMap<
        smithay::reexports::wayland_server::backend::ObjectId,
        u64,
    >,
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
    pub estimated_vblank_timers: std::collections::HashMap<
        String,
        smithay::reexports::calloop::RegistrationToken,
    >,
    /// `wp_cursor_shape_v1` — clients (GTK/Qt/Chromium) request a
    /// cursor by name instead of attaching their own surface. Without
    /// this, GTK rolls its own cursor surface and the buffer scale
    /// gets out of sync on layer-shell surfaces (mshell bar/menus),
    /// producing an oversized cursor. With it, GTK takes the named-
    /// cursor path and the compositor draws using `cursor_manager`
    /// at its own size.
    pub cursor_shape_manager_state: smithay::wayland::cursor_shape::CursorShapeManagerState,
    /// `wp_fractional_scale_v1` — clients ask for the preferred
    /// fractional scale per surface, so toolkits (GTK4, Qt6) can
    /// render at the output's actual fractional scale (1.25, 1.5,
    /// …) instead of an integer fallback that gets scaled back up
    /// by the compositor. Without this protocol, GTK4 layer-shell
    /// surfaces (mshell-frame) commit at the wrong physical pixel
    /// pitch and the bar pixel-grid drifts off the output grid —
    /// which manifests as per-state-poll micro-flicker because every
    /// fresh buffer's edges round differently. niri implements this
    /// in `niri/src/niri.rs:2295` and `handlers/mod.rs:845`.
    pub fractional_scale_manager_state:
        smithay::wayland::fractional_scale::FractionalScaleManagerState,

    pub space: Space<Window>,
    pub popups: PopupManager,

    pub seat: Seat<MargoState>,

    pub display_handle: DisplayHandle,
    pub loop_handle: LoopHandle<'static, MargoState>,
    pub loop_signal: LoopSignal,
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
    config_path: Option<PathBuf>,

    pub config: Config,
    /// Snapshot of theme-relevant `Config` fields captured the first
    /// time `apply_theme_preset` runs. `Theme::Default` resets to
    /// this snapshot; the snapshot is also reset on `mctl reload` so
    /// "default" always means "what config.conf says today".
    pub(crate) theme_baseline: Option<ThemeBaseline>,
    pub animation_curves: AnimationCurves,
    pub clients: Vec<MargoClient>,
    pub monitors: Vec<MargoMonitor>,

    pub input_keyboard: KeyboardState,
    pub input_pointer: PointerState,
    pub input_touch: TouchState,
    pub input_gesture: GestureState,

    pub foreign_toplevel_list: ForeignToplevelListState,
    pub layer_surfaces: Vec<LayerSurface>,
    pub lock_surfaces: Vec<(Output, smithay::wayland::session_lock::LockSurface)>,

    pub session_locked: bool,
    pub enable_gaps: bool,
    pub cursor_status: CursorImageStatus,
    pub cursor_manager: CursorManager,
    /// Compositor-painted wallpaper. `None` when no path resolves or
    /// the decode failed at startup — frame loop falls through to the
    /// solid `rootcolor` clear in that case. Re-decoded by
    /// `reload_config` if the path changes.
    pub wallpaper: Option<crate::wallpaper::WallpaperState>,
    pub xwm: Option<X11Wm>,
    pub xwayland_shell_state: XWaylandShellState,
    pub libinput: Option<smithay::reexports::input::Libinput>,
    pub gamma_control_manager_state: crate::protocols::gamma_control::GammaControlManagerState,
    /// Pending gamma ramp updates drained by the udev backend each frame.
    /// Tuple is (output, ramp). `None` ramp = restore default. The udev
    /// backend pops these and applies them via DRM `GAMMA_LUT`. Winit just
    /// drops them silently.
    pub pending_gamma: Vec<(Output, Option<Vec<u16>>)>,
    pub screencopy_state: crate::protocols::screencopy::ScreencopyManagerState,
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
    /// Active region-selection UI for the in-compositor screenshot
    /// flow (W2.1). `Some(...)` while the user is dragging /
    /// pondering a rect; cleared on Escape, on confirm (after
    /// spawning mscreenshot with `MARGO_REGION_GEOM`), and on
    /// session-lock (so the selector doesn't leak across login
    /// boundaries). Render path overlays the rect; input path
    /// intercepts pointer + keyboard while this is `Some`.
    pub region_selector: Option<crate::screenshot_region::ActiveRegionSelector>,
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

    /// Which hot corner the pointer is currently dwelling in (if any).
    /// `None` while pointer is anywhere else; set on entry, cleared on
    /// exit. Together with [`hot_corner_armed_at`] drives the dwell
    /// threshold before the corner's action fires.
    pub hot_corner_dwelling: Option<HotCorner>,

    /// Last udev `Changed` event timestamp, used by the hotplug
    /// debouncer in `backend/udev/mod.rs`. A burst of udev events
    /// (gamma daemon racing the lock screen, kernel firing
    /// `udev_device_get_changed` for every property tweak, …) used
    /// to call `rescan_outputs` synchronously per event — hundreds
    /// of times in 100 ms during the worst-case crash that
    /// surfaced this. `Some` ⇒ a coalesce timer is currently armed
    /// and a rescan will fire ~50 ms after the last event.
    /// Twilight (blue-light filter) state. Always present; activity
    /// is gated by `Config::twilight`. `tick_twilight()` advances
    /// this on the calloop timer and pushes gamma ramps into
    /// `pending_gamma` for every connected output.
    pub twilight: crate::twilight::TwilightState,
    /// `Some` while the twilight tick timer is in flight. We
    /// re-insert on every tick (single-shot pattern) and key off
    /// this to avoid double-arming.
    pub twilight_timer_armed: bool,

    pub hotplug_last_event_at: Option<std::time::Instant>,
    /// Sentinel: a debounce timer is already armed in the event
    /// loop. Set true when the first event of a burst arrives,
    /// cleared by the timer once it actually runs the rescan.
    pub hotplug_rescan_pending: bool,
    /// `Instant` the pointer entered the current dwell corner. The
    /// dwell threshold (`Config::hot_corner_dwell_ms`) is checked in
    /// the same `pointer_motion` handler that sets / clears
    /// `hot_corner_dwelling`. Cleared together with `hot_corner_dwelling`.
    pub hot_corner_armed_at: Option<std::time::Instant>,
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
        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let xdg_decoration_state = XdgDecorationState::new::<Self>(&dh);
        let shm_formats = vec![
            smithay::reexports::wayland_server::protocol::wl_shm::Format::Argb8888,
            smithay::reexports::wayland_server::protocol::wl_shm::Format::Xrgb8888,
            smithay::reexports::wayland_server::protocol::wl_shm::Format::Xbgr8888,
            smithay::reexports::wayland_server::protocol::wl_shm::Format::Abgr8888,
            smithay::reexports::wayland_server::protocol::wl_shm::Format::Rgb565,
        ];
        let shm_state = ShmState::new::<Self>(&dh, shm_formats);
        let viewporter_state = ViewporterState::new::<Self>(&dh);
        let dmabuf_state = DmabufState::new();
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let mut seat_state = SeatState::new();
        let seat = seat_state.new_wl_seat(&dh, "seat0");
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let primary_selection_state = PrimarySelectionState::new::<Self>(&dh);
        let data_control_state =
            DataControlState::new::<Self, _>(&dh, Some(&primary_selection_state), |_| true);
        let ext_data_control_state = ExtDataControlState::new::<Self, _>(
            &dh,
            Some(&primary_selection_state),
            |_| true,
        );
        let cursor_shape_manager_state =
            smithay::wayland::cursor_shape::CursorShapeManagerState::new::<Self>(&dh);
        let fractional_scale_manager_state =
            smithay::wayland::fractional_scale::FractionalScaleManagerState::new::<Self>(&dh);
        let layer_shell_state = WlrLayerShellState::new::<Self>(&dh);
        let session_lock_state = smithay::wayland::session_lock::SessionLockManagerState::new::<Self, _>(&dh, |_| true);
        let text_input_state = TextInputManagerState::new::<Self>(&dh);
        let input_method_state = InputMethodManagerState::new::<Self, _>(&dh, |_client| true);
        let pointer_constraints_state = PointerConstraintsState::new::<Self>(&dh);
        let relative_pointer_state = RelativePointerManagerState::new::<Self>(&dh);
        let xdg_activation_state = XdgActivationState::new::<Self>(&dh);
        let output_management_state =
            crate::protocols::output_management::OutputManagementManagerState::new::<
                Self,
                _,
            >(&dh, |_client| true);
        // wp_color_management_v1 (staging) — Phase 1 scaffolding.
        // Standing the global up early lets HDR-aware clients
        // (Chromium, mpv) detect "this compositor speaks colour
        // management" and enable their decode paths even though
        // composite is still SDR. See `protocols/color_management.rs`
        // and `docs/hdr-design.md` for the four-phase rollout.
        let color_management_state =
            crate::protocols::color_management::ColorManagementState::new::<Self, _>(
                &dh,
                |_client| true,
            );
        // ext-image-capture-source-v1 + ext-image-copy-capture-v1
        // — the modern Wayland screencast stack. Without these
        // globals, xdp-wlr 0.8+ can't expose per-window share
        // (Window tab in meeting clients). Smithay ships full
        // helpers; output and toplevel source globals are
        // independent so we can advertise both.
        let image_capture_source_state =
            smithay::wayland::image_capture_source::ImageCaptureSourceState::new();
        let output_capture_source_state =
            smithay::wayland::image_capture_source::OutputCaptureSourceState::new::<Self>(&dh);
        let toplevel_capture_source_state =
            smithay::wayland::image_capture_source::ToplevelCaptureSourceState::new::<Self>(&dh);
        let image_copy_capture_state =
            smithay::wayland::image_copy_capture::ImageCopyCaptureState::new::<Self>(&dh);
        // Clock id 1 = CLOCK_MONOTONIC. That's the same domain
        // `monotonic_now()` in the udev backend uses, so the
        // timestamps we publish are consistent with the ones
        // clients see in their own `clock_gettime(CLOCK_MONOTONIC)`.
        let presentation_state = PresentationState::new::<Self>(&dh, 1);
        let idle_notifier_state =
            smithay::wayland::idle_notify::IdleNotifierState::<Self>::new(&dh, loop_handle.clone());
        let idle_inhibit_state =
            smithay::wayland::idle_inhibit::IdleInhibitManagerState::new::<Self>(&dh);
        // `zwp_virtual_keyboard_v1` — required by wayvnc to inject
        // keystrokes from VNC clients, and used by `wtype` /
        // `ydotool` for synthetic input. Open to all clients; the
        // wayland socket is already per-user.
        let virtual_keyboard_manager_state =
            smithay::wayland::virtual_keyboard::VirtualKeyboardManagerState::new::<Self, _>(
                &dh,
                |_client| true,
            );
        let space = Space::default();
        let popups = PopupManager::default();
        let animation_curves = AnimationCurves::bake(&config);
        let input_keyboard = KeyboardState::new(&config);

        // Register dwl-ipc-v2 global
        dh.create_global::<Self, crate::protocols::generated::dwl_ipc::zdwl_ipc_manager_v2::ZdwlIpcManagerV2, _>(
            2,
            crate::protocols::dwl_ipc::DwlIpcGlobalData,
        );

        let xwayland_shell_state = XWaylandShellState::new::<Self>(&dh);
        let foreign_toplevel_list = ForeignToplevelListState::new::<Self>(&dh);

        // wlr_gamma_control_v1 — sunsetr / gammastep / wlsunset use this to
        // push night-light ramps to outputs. Allow all clients (no privileged
        // filter) so user services can drive it freely.
        let gamma_control_manager_state =
            crate::protocols::gamma_control::GammaControlManagerState::new::<Self, _>(
                &dh,
                |_client| true,
            );

        // wlr-screencopy-unstable-v1: lets `grim`, `wf-recorder`, `screen rec`
        // etc. capture compositor outputs.
        let screencopy_state =
            crate::protocols::screencopy::ScreencopyManagerState::new::<Self, _>(
                &dh,
                |_client| true,
            );

        Self {
            compositor_state,
            xdg_shell_state,
            xdg_decoration_state,
            shm_state,
            viewporter_state,
            dmabuf_state,
            dmabuf_global: None,
            dmabuf_import_hook: None,
            drm_syncobj_state: None,
            seat_state,
            layer_shell_state,
            output_manager_state,
            data_device_state,
            primary_selection_state,
            data_control_state,
            ext_data_control_state,
            session_lock_state,
            text_input_state,
            input_method_state,
            pointer_constraints_state,
            relative_pointer_state,
            xdg_activation_state,
            output_management_state,
            pending_output_mode_changes: Vec::new(),
            color_management_state,
            scripting: None,
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
            image_capture_source_state,
            output_capture_source_state,
            toplevel_capture_source_state,
            image_copy_capture_state,
            image_copy_capture_sessions: Vec::new(),
            pending_image_copy_frames: Vec::new(),
            presentation_state,
            space,
            popups,
            seat,
            display_handle: dh,
            loop_handle,
            loop_signal,
            clock: Clock::new(),
            should_quit: false,
            repaint_requested: true,
            repaint_ping: None,
            pending_vblanks: 0,
            config_path,
            animation_curves,
            input_keyboard,
            input_pointer: Default::default(),
            input_touch: Default::default(),
            input_gesture: Default::default(),
            foreign_toplevel_list,
            layer_surfaces: vec![],
            lock_surfaces: vec![],
            clients: vec![],
            monitors: vec![],
            session_locked: false,
            idle_notifier_state,
            idle_inhibit_state,
            virtual_keyboard_manager_state,
            idle_inhibitors: std::collections::HashSet::new(),
            layer_layout_hashes: std::collections::HashMap::new(),
            layer_kb_interactivity_hashes: std::collections::HashMap::new(),
            frame_callback_sequence: std::collections::HashMap::new(),
            estimated_vblank_timers: std::collections::HashMap::new(),
            cursor_shape_manager_state,
            fractional_scale_manager_state,
            enable_gaps: config.enable_gaps,
            cursor_status: CursorImageStatus::default_named(),
            cursor_manager: CursorManager::new(),
            wallpaper: crate::wallpaper::WallpaperState::load(config.wallpaper.as_deref()),
            xwm: None,
            xwayland_shell_state,
            libinput: None,
            gamma_control_manager_state,
            pending_gamma: Vec::new(),
            screencopy_state,
            libinput_devices: Vec::new(),
            closing_clients: Vec::new(),
            plugins: Vec::new(),
            #[cfg(feature = "a11y")]
            a11y: crate::a11y::A11yState::new(),
            region_selector: None,
            layer_animations: std::collections::HashMap::new(),
            overview_transition_animation_ms: None,
            last_reload_diagnostics: Vec::new(),
            config_error_overlay_until: None,
            config_error_overlay:
                crate::render::config_error_overlay::ConfigErrorOverlay::new(),
            overview_cycle_pending: false,
            overview_cycle_modifier_mask: margo_config::Modifiers::empty(),
            hot_corner_dwelling: None,
            twilight: crate::twilight::TwilightState::default(),
            twilight_timer_armed: false,
            hotplug_last_event_at: None,
            hotplug_rescan_pending: false,
            hot_corner_armed_at: None,
            config,
            theme_baseline: None,
        }
    }

    /// Rebuild the wlr-output-management snapshot from the current
    /// monitor list and publish it to all bound clients (kanshi,
    /// wlr-randr, way-displays, …). Cheap when nothing's changed:
    /// `snapshot_changed` early-returns on equal snapshots.
    /// Path of the runtime state-file used by `mctl clients` /
    /// `mctl outputs`. Public so dispatch handlers can also
    /// trigger a write after non-arrange state changes.
    pub fn refresh_state_file(&self) {
        self.write_state_file();
    }

    /// Open the in-compositor region selector at the current cursor
    /// position. Replaces the previous "spawn slurp via mscreenshot"
    /// flow — the selector lives entirely inside margo's render +
    /// input loops so there's no second window fighting focus, no
    /// IPC round-trip, no stale-frame artifacts. Subsequent pointer
    /// button + motion + key events route through the selector
    /// until [`Self::confirm_region_selection`] or
    /// [`Self::cancel_region_selection`] runs.
    pub fn open_region_selector(
        &mut self,
        mode: crate::screenshot_region::SelectorMode,
    ) {
        let cursor = (self.input_pointer.x, self.input_pointer.y);
        self.region_selector = Some(
            crate::screenshot_region::ActiveRegionSelector::at(cursor, mode),
        );
        self.request_repaint();
        tracing::info!(
            "region selector opened at ({:.0}, {:.0}) mode={:?}",
            cursor.0,
            cursor.1,
            mode
        );
    }

    /// User pressed Enter / released the drag button — finalize
    /// the selection: spawn `mscreenshot <mode>` with
    /// `MARGO_REGION_GEOM` set, then close the selector. Re-arms
    /// (keeps the selector open) if the selection is degenerate
    /// — user clicked but didn't drag. Caller decides whether to
    /// route Enter through this immediately or wait for a real
    /// drag.
    pub fn confirm_region_selection(&mut self) {
        let Some(sel) = self.region_selector.take() else {
            return;
        };
        let Some(geom) = sel.geom_string() else {
            // Degenerate rect — re-arm so user can try again.
            self.region_selector = Some(sel);
            return;
        };
        let mode = sel.mode.subcommand();
        let cmd = format!("MARGO_REGION_GEOM='{}' mscreenshot {}", geom, mode);
        tracing::info!(cmd = %cmd, "region selector confirm");
        if let Err(e) = crate::utils::spawn_shell(&cmd) {
            tracing::error!(error = ?e, "spawn mscreenshot failed");
        }
        self.request_repaint();
    }

    /// User pressed Escape — drop the selector without spawning
    /// mscreenshot.
    pub fn cancel_region_selection(&mut self) {
        if self.region_selector.take().is_some() {
            tracing::info!("region selector cancelled");
            self.request_repaint();
        }
    }

    /// Soft-disable a monitor: mark it inactive, migrate every client
    /// to the first remaining enabled monitor, and clear focus from it.
    /// Render and arrange paths skip disabled monitors so the panel
    /// stops getting dirty repaints; the underlying smithay `Output`
    /// stays alive so a later `enable_monitor` call can restore it
    /// without a full hotplug round-trip. Pertag state survives across
    /// the cycle.
    ///
    /// Note: the DRM connector is NOT powered off here — that needs
    /// the udev backend's DrmCompositor handle, plumbed separately.
    /// What this fixes: the wlr-output-management protocol-level
    /// "disable" request now succeeds, kanshi profiles that toggle
    /// outputs flip cleanly, and the bar / state file see the right
    /// active-output set. Power-off of the panel is a follow-up.
    pub fn disable_monitor(&mut self, mon_idx: usize) {
        if mon_idx >= self.monitors.len() {
            return;
        }
        if !self.monitors[mon_idx].enabled {
            return;
        }
        // Pick a migration target — first OTHER enabled monitor.
        let target = (0..self.monitors.len())
            .find(|&i| i != mon_idx && self.monitors[i].enabled);
        let Some(target) = target else {
            tracing::warn!(
                "disable_monitor: refusing to disable {} — no other enabled monitor",
                self.monitors[mon_idx].name
            );
            return;
        };
        let target_tagset = self.monitors[target].current_tagset();
        let target_name = self.monitors[target].name.clone();
        let src_name = self.monitors[mon_idx].name.clone();

        // Migrate every client living on the doomed monitor.
        for c in self.clients.iter_mut() {
            if c.monitor == mon_idx {
                c.monitor = target;
                // Pull onto an active tag of the new home so the
                // client doesn't vanish into a hidden tagset.
                if c.tags & target_tagset == 0 {
                    c.tags = target_tagset;
                }
            }
        }
        // Clear focus history that points at the disabled monitor.
        if self.focused_monitor() == mon_idx {
            for mon in &mut self.monitors {
                mon.selected = None;
            }
        }
        self.monitors[mon_idx].enabled = false;
        self.arrange_monitor(target);
        self.focus_first_visible_or_clear(target);
        self.publish_output_topology();
        self.write_state_file();
        tracing::info!(
            from = %src_name,
            to = %target_name,
            "disabled output: migrated clients"
        );
    }

    /// Re-enable a previously soft-disabled monitor. New windows can
    /// land on it again; arrange picks it up; render starts drawing
    /// it on the next frame.
    pub fn enable_monitor(&mut self, mon_idx: usize) {
        if mon_idx >= self.monitors.len() {
            return;
        }
        if self.monitors[mon_idx].enabled {
            return;
        }
        self.monitors[mon_idx].enabled = true;
        self.arrange_monitor(mon_idx);
        self.publish_output_topology();
        self.write_state_file();
        tracing::info!(output = %self.monitors[mon_idx].name, "re-enabled output");
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
        self.output_management_state.snapshot_changed(snap);
    }

    pub fn remove_output(&mut self, output: &Output) {
        for layer in smithay::desktop::layer_map_for_output(output).layers() {
            layer.layer_surface().send_close();
        }

        self.gamma_control_manager_state.output_removed(output);
        self.screencopy_state.remove_output(output);

        if let Some(pos) = self.monitors.iter().position(|m| m.output == *output) {
            tracing::info!(monitor = %self.monitors[pos].name, "removing monitor");
            self.monitors.remove(pos);
        }
        self.space.unmap_output(output);
        self.lock_surfaces.retain(|(o, _)| o != output);
        self.pending_gamma.retain(|(o, _)| o != output);
        // Hotplug-out: refresh the shared D-Bus snapshot so
        // xdp-gnome's chooser dialog drops the now-gone output.
        self.refresh_ipc_outputs();
        self.request_repaint();
    }

    pub fn arrange_all(&mut self) {
        for mon_idx in 0..self.monitors.len() {
            self.arrange_monitor(mon_idx);
        }
        self.request_repaint();
        self.write_state_file();
        self.publish_a11y_window_list();
    }

    /// Arrange just the listed monitors. Used by `open_overview` and
    /// `close_overview` so a multi-monitor setup doesn't pay the cost
    /// of re-laying out outputs that didn't flip overview state. Skips
    /// out-of-range indices defensively — the caller is the same
    /// process that built the list, but `monitors` can shrink under us
    /// during multi-output hot-unplug and we don't want to panic mid-
    /// arrange.
    pub fn arrange_monitors(&mut self, indices: &[usize]) {
        for &idx in indices {
            if idx < self.monitors.len() {
                self.arrange_monitor(idx);
            }
        }
        self.request_repaint();
        self.write_state_file();
        self.publish_a11y_window_list();
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


    /// Start an interactive move grab on the currently focused window.
    /// Triggered by the `moveresize,curmove` action (typically a super+
    /// left-drag mousebind). No-op if there's no focused client or no
    /// pointer button is currently pressed.
    pub fn start_interactive_move(&mut self) {
        let Some(idx) = self.focused_client_idx() else { return };
        let window = self.clients[idx].window.clone();
        let initial_loc = smithay::utils::Point::<i32, smithay::utils::Logical>::from((
            self.clients[idx].geom.x,
            self.clients[idx].geom.y,
        ));
        let Some(pointer) = self.seat.get_pointer() else { return };
        // Use the most recent serial we've seen — we're driving the grab
        // ourselves from a synthesized command, so just take the next one.
        let serial = SERIAL_COUNTER.next_serial();
        let start_data = smithay::input::pointer::GrabStartData {
            focus: None,
            button: 0x110, // BTN_LEFT
            location: smithay::utils::Point::<f64, smithay::utils::Logical>::from((
                self.input_pointer.x,
                self.input_pointer.y,
            )),
        };
        let grab = crate::input::grabs::MoveSurfaceGrab {
            start_data,
            window,
            initial_loc,
        };
        pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
    }

    /// Start an interactive resize grab on the focused window. Edge
    /// defaults to bottom-right (the natural drag-corner gesture). If
    /// you want a specific edge, pass it in the action arg later.
    pub fn start_interactive_resize(&mut self) {
        let Some(idx) = self.focused_client_idx() else { return };
        let c = &self.clients[idx];
        let window = c.window.clone();
        let initial_loc = smithay::utils::Point::<i32, smithay::utils::Logical>::from((
            c.geom.x, c.geom.y,
        ));
        let initial_size = smithay::utils::Size::<i32, smithay::utils::Logical>::from((
            c.geom.width.max(1),
            c.geom.height.max(1),
        ));
        let Some(pointer) = self.seat.get_pointer() else { return };
        let serial = SERIAL_COUNTER.next_serial();
        let start_data = smithay::input::pointer::GrabStartData {
            focus: None,
            button: 0x111, // BTN_RIGHT
            location: smithay::utils::Point::<f64, smithay::utils::Logical>::from((
                self.input_pointer.x,
                self.input_pointer.y,
            )),
        };
        let grab = crate::input::grabs::ResizeSurfaceGrab {
            start_data,
            window,
            edges:
                smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge::BottomRight,
            initial_loc,
            initial_size,
        };
        pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
    }

    /// Dump a one-shot diagnostic summary at INFO level — outputs, focused
    /// client, layer surfaces, lock state, idle inhibitors, recent counters.
    /// Force-tear-down a stuck session lock from the compositor side.
    ///
    /// Use case: the user pressed alt+l, noctalia's lock screen came up,
    /// and now the password input is unresponsive — they can't type to
    /// unlock. Without this, recovery means switching to a TTY and
    /// killing the locker process. With this they can hit a hard-coded
    /// keybind (the action is whitelisted in `handle_keyboard` even
    /// while `session_locked`) and get back to the desktop.
    ///
    /// We don't try to nicely tell the locker to release; we just clear
    /// our state, drop the lock surfaces, and re-show toplevels. The
    /// noctalia process will see its surfaces destroyed and recover on
    /// its own.
    pub fn force_unlock(&mut self) {
        if !self.session_locked && self.lock_surfaces.is_empty() {
            tracing::info!("force_unlock: nothing to do (already unlocked)");
            return;
        }
        tracing::warn!(
            "force_unlock: tearing down stuck lock (lock_surfaces={})",
            self.lock_surfaces.len()
        );
        self.session_locked = false;
        self.lock_surfaces.clear();
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


    pub fn request_repaint(&mut self) {
        self.repaint_requested = true;
        // Wake the redraw scheduler so the loop drains the flag this
        // iteration. Coalesces: many request_repaint() calls between two
        // dispatches still produce a single Ping event (eventfd semantics
        // — see calloop ping source), so we don't need to track whether
        // a wake is already pending.
        //
        // Suppress the ping while a previously-queued frame is still
        // waiting for its vblank. The DRM compositor only accepts one
        // pending page-flip per output, and the post-dispatch animation
        // tick re-arms repaint every iteration; without this gate the
        // ping callback would fire between vblanks and either render an
        // identical scene (wasted work) or hit `queue_frame` "frame
        // already pending" errors. The vblank handler re-emits the ping
        // once it counts back down to zero.
        if self.pending_vblanks == 0 {
            if let Some(ping) = &self.repaint_ping {
                ping.ping();
            }
        }
    }

    /// Called by the udev backend after a successful `queue_frame`.
    /// Pushes the redraw scheduler into "frame in flight" mode so further
    /// repaint requests stay deferred until the page-flip completes.
    pub fn note_frame_queued(&mut self) {
        self.pending_vblanks += 1;
    }

    /// Called by the udev backend's `DrmEvent::VBlank` handler after
    /// `frame_submitted`. Decrements the in-flight counter and re-arms
    /// the redraw scheduler if the scene is dirty. Frame callbacks
    /// were already sent at `queue_frame` time (niri pattern), so
    /// VBlank itself does NOT bump sequence — `send_frame_callbacks`
    /// here is a safety-net call that dedups via the per-surface
    /// `last_sent_at` and only does work for surfaces that somehow
    /// missed the queue_frame round (rare: surfaces freshly attached
    /// after the queue, completely occluded surfaces hitting the
    /// 995 ms throttle, etc.). Also cancels any in-flight
    /// estimated-vblank timer for the output since the real vblank
    /// supersedes it.
    pub fn note_vblank(&mut self, output: &Output) {
        self.pending_vblanks = self.pending_vblanks.saturating_sub(1);

        let name = output.name();
        // Pulling the entry from the map cancels the timer at the
        // *logical* level — the timer source itself might still fire
        // (calloop has no atomic cancel), but its callback will find
        // the map empty and no-op. We intentionally do NOT call
        // `loop_handle.remove(token)` here: that would race against
        // the source's own `TimeoutAction::Drop` and produce the
        // "Received an event for non-existent source" warning that
        // was flooding the journal.
        self.estimated_vblank_timers.remove(&name);

        let now = self.clock.now();
        self.send_frame_callbacks(output, now);

        if self.pending_vblanks == 0 && self.repaint_requested {
            if let Some(ping) = &self.repaint_ping {
                ping.ping();
            }
        }
    }

    /// Install the wake handle the udev/winit backend created. Call once
    /// at startup; subsequent [`request_repaint`] calls will wake the
    /// loop via the supplied [`Ping`] sender.
    pub fn set_repaint_ping(&mut self, ping: Ping) {
        self.repaint_ping = Some(ping);
    }

    pub fn take_repaint_request(&mut self) -> bool {
        let requested = self.repaint_requested;
        self.repaint_requested = false;
        requested
    }

    pub fn reload_config(&mut self) -> Result<()> {
        // Validate first. The parser is permissive (silent defaults
        // on malformed values), so a "successful" parse can still mean
        // the user's intent was misread. Run the structured validator
        // and bail before swapping config if it found errors —
        // compositor stays on the previous good config.
        match margo_config::validator::validate_config(self.config_path.as_deref()) {
            Ok(report) => {
                self.last_reload_diagnostics = report.diagnostics.clone();
                if report.has_errors() {
                    // Trigger the C2 on-screen banner. 10 s ought to be
                    // long enough to read "your config is broken, run
                    // mctl check-config" without being a pest.
                    self.config_error_overlay_until = Some(
                        std::time::Instant::now()
                            + std::time::Duration::from_secs(10),
                    );
                    self.request_repaint();
                    let err_count = report.errors().count();
                    return Err(anyhow::anyhow!(
                        "config has {err_count} error(s) — run `mctl check-config` for details"
                    ));
                }
            }
            Err(e) => {
                tracing::warn!(error = ?e, "config validator could not read file");
                // Fall through: let parse_config produce the canonical
                // error so the caller's message says "I/O failure"
                // rather than "validator missing".
            }
        }

        let new_config = parse_config(self.config_path.as_deref())
            .with_context(|| "reload margo config")?;

        // Successful reload — clear any stale diagnostics + overlay
        // (warnings from the validation pass above are still in
        // last_reload_diagnostics, intentionally; the user can still
        // query them via mctl config-errors).
        self.config_error_overlay_until = None;

        if let Some(keyboard) = self.seat.get_keyboard() {
            let xkb_options = if new_config.xkb_rules.options.is_empty() {
                None
            } else {
                Some(new_config.xkb_rules.options.clone())
            };
            keyboard
                .set_xkb_config(
                    self,
                    smithay::input::keyboard::XkbConfig {
                        rules: &new_config.xkb_rules.rules,
                        model: &new_config.xkb_rules.model,
                        layout: &new_config.xkb_rules.layout,
                        variant: &new_config.xkb_rules.variant,
                        options: xkb_options,
                    },
                )
                .map_err(|e| anyhow::anyhow!("reload xkb config: {e:?}"))?;
            keyboard.change_repeat_info(new_config.repeat_rate, new_config.repeat_delay);
        }

        self.input_keyboard.repeat_rate = new_config.repeat_rate;
        self.input_keyboard.repeat_delay = new_config.repeat_delay;

        for device in &mut self.libinput_devices {
            crate::libinput_config::apply_to_device(device, &new_config);
        }

        for mon in &mut self.monitors {
            mon.gappih = new_config.gappih as i32;
            mon.gappiv = new_config.gappiv as i32;
            mon.gappoh = new_config.gappoh as i32;
            mon.gappov = new_config.gappov as i32;
        }

        self.animation_curves = AnimationCurves::bake(&new_config);
        self.enable_gaps = new_config.enable_gaps;
        self.config = new_config;
        // Reload re-establishes "what the file says" — invalidate the
        // theme baseline so a subsequent `mctl theme default` resets
        // to the freshly-parsed values.
        self.theme_baseline = None;
        for idx in 0..self.clients.len() {
            self.reapply_rules(idx, WindowRuleReason::Reload);
        }
        for mon_idx in 0..self.monitors.len() {
            self.apply_tag_rules_to_monitor(mon_idx);
        }
        self.arrange_all();
        crate::protocols::dwl_ipc::broadcast_all(self);
        self.request_repaint();
        // Config swap may have flipped twilight on/off or changed
        // day/night temps — force a resample so the new values
        // take effect immediately instead of waiting for the next
        // tick.
        self.tick_twilight();
        tracing::info!("config reloaded");
        Ok(())
    }


    pub(crate) fn refresh_output_work_area(&mut self, output: &Output) {
        let work_area = {
            let map = layer_map_for_output(output);
            map.non_exclusive_zone()
        };

        if let Some(mon_idx) = self.monitors.iter().position(|m| m.output == *output) {
            let monitor_area = self.monitors[mon_idx].monitor_area;
            let new_work_area = crate::layout::Rect {
                x: monitor_area.x + work_area.loc.x,
                y: monitor_area.y + work_area.loc.y,
                width: work_area.size.w,
                height: work_area.size.h,
            };
            // Compositor commit handler calls us on every layer
            // surface commit. mshell-frame's GTK4 + gtk4-layer-shell
            // pair commits a fresh buffer 60 times a second during
            // every menu Revealer animation — without this guard
            // each one re-tiles the monitor (`arrange_monitor`
            // re-computes geometry for every visible client and
            // can spawn move-animation snapshots, each a full-size
            // GlesTexture). That's the asymmetry between mshell
            // (heavy churn → bar flicker, GPU memory spikes) and
            // noctalia / Qt (commits less aggressively, work_area
            // change check skips this entirely). Skip when the
            // geometry is identical; the layer-shell exclusive zone
            // is the only thing that can actually move the work
            // area, and mshell-frame doesn't claim one.
            if self.monitors[mon_idx].work_area == new_work_area {
                return;
            }
            self.monitors[mon_idx].work_area = new_work_area;
            self.arrange_monitor(mon_idx);
        }
    }


    pub fn arrange_monitor(&mut self, mon_idx: usize) {
        let _span = tracy_client::span!("arrange_monitor");
        if mon_idx >= self.monitors.len() {
            return;
        }
        // Soft-disabled monitor: don't lay out — clients have already
        // been migrated off, and laying out against a panel that isn't
        // being rendered just produces stale geometry.
        if !self.monitors[mon_idx].enabled {
            return;
        }

        // Adaptive layout: when `Config::auto_layout` is on AND the
        // user hasn't explicitly picked a layout for the current tag
        // (`pertag.user_picked_layout[curtag]` sticky bit), pick a
        // layout based on the visible-client count and the monitor's
        // aspect ratio. Sets `pertag.ltidxs[curtag]` *before* we read
        // it for `layout` below, so a single arrange pass picks up
        // the new value naturally.
        if self.config.auto_layout && !self.monitors[mon_idx].is_overview {
            self.maybe_apply_adaptive_layout(mon_idx);
        }

        let mon = &self.monitors[mon_idx];
        let is_overview = mon.is_overview;
        // Overview path: a single Grid arrangement over the
        // (already-zoomed) work area, holding every tag's clients
        // simultaneously. Mango/Hypr-style geometric continuity —
        // each window keeps a deterministic spot in the thumbnail,
        // and the keyboard-first MRU navigation
        // (`overview_focus_next/prev`) cycles through them with
        // focus + border tracking the selection.
        let layout = if is_overview { crate::layout::LayoutId::Grid } else { mon.current_layout() };
        let tagset = if is_overview { !0 } else { mon.current_tagset() };
        let nmaster = mon.current_nmaster();
        let mfact = mon.current_mfact();
        let monitor_area = mon.monitor_area;
        // Apply `overview_zoom` to the work area so the overview Grid
        // arranges every visible window inside a *centered* sub-rect
        // smaller than the full work area — niri's "zoom 0.5" feeling
        // without a true scene-tree transform. Centering keeps the
        // overview rect inside the layer-shell exclusion zone, so the
        // bar and other top/overlay layers stay anchored to the panel
        // edges (niri pattern: top + overlay layers stay at 1.0,
        // background + bottom would zoom in lock-step — margo doesn't
        // depend on the latter today, so we only zoom the workspace
        // surface).
        let work_area = if is_overview {
            let zoom = self.config.overview_zoom.clamp(0.1, 1.0) as f64;
            let wa = mon.work_area;
            let new_w = ((wa.width as f64) * zoom).round() as i32;
            let new_h = ((wa.height as f64) * zoom).round() as i32;
            let dx = (wa.width - new_w) / 2;
            let dy = (wa.height - new_h) / 2;
            crate::layout::Rect {
                x: wa.x + dx,
                y: wa.y + dy,
                width: new_w.max(1),
                height: new_h.max(1),
            }
        } else {
            mon.work_area
        };
        let mut gaps = if is_overview {
            let inner = self.config.overview_gap_inner.max(0);
            let outer = self.config.overview_gap_outer.max(0);
            layout::GapConfig {
                gappih: inner,
                gappiv: inner,
                gappoh: outer,
                gappov: outer,
            }
        } else {
            layout::GapConfig {
                gappih: if self.enable_gaps { mon.gappih } else { 0 },
                gappiv: if self.enable_gaps { mon.gappiv } else { 0 },
                gappoh: if self.enable_gaps { mon.gappoh } else { 0 },
                gappov: if self.enable_gaps { mon.gappov } else { 0 },
            }
        };
        let visible_in_pass = |c: &MargoClient| {
            // Skip clients that haven't gone through their deferred
            // initial map yet — they exist in `self.clients` but
            // haven't been placed in `space` and don't have rules
            // applied. Including them in arrange would map them at
            // the layout's default position, which is exactly the
            // pre-rule flicker we deferred to avoid.
            !c.is_initial_map_pending
                && c.is_visible_on(mon_idx, tagset)
                && (!is_overview || (!c.is_minimized && !c.is_killing && !c.is_in_scratchpad))
        };

        let tiled: Vec<usize> = if is_overview {
            // In overview, the visual cell order should match what
            // alt+Tab walks — so the user can read the grid as
            // "left = most-recently-touched, right = older" (or tag
            // 1-9 / mixed, depending on `overview_cycle_order`).
            // Re-uses the same ordering the cycle path computes.
            self.overview_visible_clients_for_monitor(mon_idx)
        } else {
            self.clients
                .iter()
                .enumerate()
                .filter(|(_, c)| visible_in_pass(c) && c.is_tiled())
                .map(|(i, _)| i)
                .collect()
        };

        let scroller_proportions: Vec<f32> =
            tiled.iter().map(|&i| self.clients[i].scroller_proportion).collect();
        let focused_tiled_pos = self
            .focused_client_idx()
            .and_then(|focused_idx| tiled.iter().position(|&idx| idx == focused_idx));

        if !is_overview && self.config.smartgaps && tiled.len() <= 1 {
            gaps.gappoh = 0;
            gaps.gappov = 0;
        }

        let curtag = self.monitors[mon_idx].pertag.curtag;
        let canvas_pan = (
            self.monitors[mon_idx]
                .pertag
                .canvas_pan_x
                .get(curtag)
                .copied()
                .unwrap_or(0.0),
            self.monitors[mon_idx]
                .pertag
                .canvas_pan_y
                .get(curtag)
                .copied()
                .unwrap_or(0.0),
        );
        let ctx = layout::ArrangeCtx {
            work_area,
            tiled: &tiled,
            nmaster,
            mfact,
            gaps: &gaps,
            scroller_proportions: &scroller_proportions,
            default_scroller_proportion: self.config.scroller_default_proportion,
            focused_tiled_pos,
            scroller_structs: self.config.scroller_structs,
            scroller_focus_center: self.config.scroller_focus_center,
            scroller_prefer_center: self.config.scroller_prefer_center,
            scroller_prefer_overspread: self.config.scroller_prefer_overspread,
            canvas_pan,
        };

        // Overview path — mango-ext pattern (`overview(m) { grid(m); }`).
        // Above we forced `layout = Grid` and `tagset = !0` when
        // `is_overview`, and the `tiled` filter at line ~2977 admits
        // floating clients in overview too. So a single Grid arrange
        // over every visible window produces the right shape: 1 window
        // ≈ 90%×90% centred, 2 → side-by-side halves, 4 → 2×2 quarters,
        // 9 → 3×3 evenly. Cells shrink as window count grows, which is
        // the natural Mango/Hypr feel — no fixed 3×3 per-tag thumbnails.
        let geometries = layout::arrange(layout, &ctx);
        let now = crate::utils::now_ms();
        for (client_idx, mut rect) in geometries {
            // Apply per-client size constraints from window rules. The layout
            // algorithm is constraint-agnostic; we clamp post-hoc so that
            // e.g. picture-in-picture players keep their pinned dimensions
            // even when the surrounding scroller column would prefer wider.
            let c = &self.clients[client_idx];
            if c.min_width > 0 || c.min_height > 0 || c.max_width > 0 || c.max_height > 0 {
                clamp_size(
                    &mut rect.width,
                    &mut rect.height,
                    c.min_width,
                    c.min_height,
                    c.max_width,
                    c.max_height,
                );
            }
            let old = self.clients[client_idx].geom;

            // If we're already animating toward exactly this target,
            // leave the in-flight animation alone. arrange_monitor gets
            // called from many event sources (title change → window-
            // rule reapply, focus shift, output resize, scroller pan
            // recompute, …) and a long-running browser like Helium can
            // tick those off every frame while it's playing video. The
            // old behaviour was: each call saw `old != rect` (because
            // `old = c.geom` is the *interpolated* mid-flight value, not
            // the target), restarted the move animation with `initial
            // = old`, and reset `time_started = now`. Result: the
            // animation never finishes — every 16 ms it inches a few
            // pixels toward the target and then resets, producing the
            // exact 1-pixel-per-frame oscillation we kept seeing in the
            // arrange traces (-1794 → -1795 → -1794 → …).
            let already_animating_to_target =
                self.clients[client_idx].animation.running
                    && self.clients[client_idx].animation.current == rect;

            let should_animate = self.config.animations
                && self.config.animation_duration_move > 0
                && !self.clients[client_idx].no_animation
                && !self.clients[client_idx].is_tag_switching
                && old.width > 0
                && old.height > 0
                && old != rect
                && !already_animating_to_target;

            // Diagnostic: every layout decision per visible client.
            // Fires per-client on every tag switch / move / focus
            // arrange — at INFO it floods the journal during normal
            // use (~30-60 lines/sec) and shows up as input latency
            // and journal contention. Trace level keeps it available
            // for `RUST_LOG=margo=trace` debugging without polluting
            // the steady-state log.
            let actual_geom = self.clients[client_idx].window.geometry().size;
            tracing::trace!(
                "arrange[{}]: client_idx={} old={}x{}+{}+{} slot={}x{}+{}+{} actual_buf={}x{} animate={} already_to_target={}",
                self.clients[client_idx].app_id.as_str(),
                client_idx,
                old.width,
                old.height,
                old.x,
                old.y,
                rect.width,
                rect.height,
                rect.x,
                rect.y,
                actual_geom.w,
                actual_geom.h,
                should_animate,
                already_animating_to_target,
            );
            if should_animate {
                // Animate the slot fully — both position AND size lerp
                // from `old` to `rect` over `animation_duration_move`.
                // Combined with the niri-style crossfade that runs in
                // parallel (snapshot rendered on top with fading
                // alpha, scaled to the *current* interpolated slot),
                // this gives the smooth resize transition the user
                // sees from niri/Hyprland's animated layouts: the
                // pre-resize content scales down while the post-
                // resize content fades up.
                //
                // Earlier we used to snap the size to the target on
                // frame 0 (initial.width = rect.width) so the buffer
                // and the slot would always match dimensions — but
                // that left the snapshot fixed at the new slot size
                // for the entire animation, which meant the snapshot
                // was rendered at a *different* size from the captured
                // content for 150 ms and the user saw a stretched/
                // squished version of the pre-resize image. The
                // crossfade infrastructure makes the size-snap
                // unnecessary: we always render BOTH layers at the
                // interpolated slot, and the buffer/slot mismatch on
                // the live layer is hidden under the snapshot until
                // alpha drops.
                let initial = old;
                // niri-style resize transition: if the slot size
                // changes (not just the position), flag a snapshot so
                // the next render captures the *current* surface tree
                // to a `GlesTexture`. While the move animation
                // interpolates the slot from old to new, the render
                // path draws that snapshot scaled to the live slot
                // instead of the live surface — the OLD content stays
                // pinned visually until the client (Electron, slow
                // ack) commits a buffer at the new size, which drops
                // the snapshot. Without this, Helium's 50–100 ms
                // ack-and-reflow window leaks the buffer-vs-slot
                // mismatch onto the screen.
                let slot_size_changed =
                    old.width != rect.width || old.height != rect.height;
                if slot_size_changed
                    && self.clients[client_idx].resize_snapshot.is_none()
                {
                    self.clients[client_idx].snapshot_pending = true;
                }
                // Spring retarget: if the previous animation was still
                // running, carry its per-channel velocity forward.
                // Without this, the integrator would re-start from rest
                // every time the layout reshuffled mid-flight and the
                // window would visibly hitch — the whole point of the
                // spring clock is that retargets stay continuous.
                // Bezier ignores this field; harmless if it's set.
                // Decide the animation's hard duration. With bezier
                // we honour the user's `animation_duration_move`; with
                // spring we let the physics tell us how long it'll
                // take to settle to within `epsilon` of the target,
                // capped between a sane floor and ceiling so a single
                // bad config value can't produce a 10-second slide.
                let use_spring = self
                    .config
                    .animation_clock_move
                    .eq_ignore_ascii_case("spring");
                let duration_ms = if use_spring {
                    let max_disp = ((rect.x - initial.x).abs())
                        .max((rect.y - initial.y).abs())
                        .max((rect.width - initial.width).abs())
                        .max((rect.height - initial.height).abs())
                        as f64;
                    if max_disp <= 0.5 {
                        // Already at target (sub-pixel). Take the
                        // bezier-style fallback so we still log a
                        // meaningful animation start, but the tick
                        // will settle on the very next frame.
                        self.config.animation_duration_move.max(1)
                    } else {
                        let spring = crate::animation::spring::Spring {
                            from: 0.0,
                            to: max_disp,
                            initial_velocity: 0.0,
                            params: crate::animation::spring::SpringParams::new(
                                self.config.animation_spring_damping_ratio,
                                self.config.animation_spring_stiffness,
                                0.5, // half-pixel epsilon
                            ),
                        };
                        let dur = spring
                            .clamped_duration()
                            .map(|d| d.as_millis() as u32)
                            // Pathological overdamped → fall back.
                            .unwrap_or(self.config.animation_duration_move.max(1));
                        // Clamp: 60 ms floor (one vblank), 1500 ms
                        // ceiling (anything longer is almost certainly
                        // a misconfiguration).
                        dur.clamp(60, 1500)
                    }
                } else {
                    // Overview transitions override the configured
                    // move duration with a snappier value (set by
                    // open_overview/close_overview); falls through to
                    // the user's animation_duration_move otherwise.
                    self.overview_transition_animation_ms
                        .unwrap_or(self.config.animation_duration_move)
                        .max(1)
                };
                self.clients[client_idx].animation = ClientAnimation {
                    should_animate: true,
                    running: true,
                    time_started: now,
                    last_tick_ms: now,
                    duration: duration_ms,
                    initial,
                    current: rect,
                    action: AnimationType::Move,
                    ..Default::default()
                };
                self.clients[client_idx].geom = initial;
            } else if already_animating_to_target {
                // Existing animation still converging on the right
                // target — leave its `time_started`, `initial`, and the
                // current interpolated `c.geom` exactly where they are.
            } else {
                self.clients[client_idx].animation.running = false;
                self.clients[client_idx].geom = rect;
            }
            self.clients[client_idx].is_tag_switching = false;
        }

        // Apply fullscreen / floating overrides outside overview. Overview
        // intentionally thumbnails every visible window in the grid.
        if !is_overview {
            for i in 0..self.clients.len() {
                let c = &self.clients[i];
                if c.monitor != mon_idx || !visible_in_pass(c) {
                    continue;
                }
                // Fullscreen geometry per mode:
                //   * Exclusive — full panel, bar will be suppressed
                //     by the render path so the window literally
                //     covers everything.
                //   * WorkArea  — `monitors[mon_idx].work_area`, i.e.
                //     the rect after layer-shell exclusion zones
                //     are subtracted; bar stays drawn on top.
                //   * Off       — fall through to the normal layout /
                //     floating geometry.
                match c.fullscreen_mode {
                    FullscreenMode::Exclusive => {
                        self.clients[i].geom = monitor_area;
                    }
                    FullscreenMode::WorkArea => {
                        self.clients[i].geom = work_area;
                    }
                    FullscreenMode::Off => {
                        if c.is_floating && c.float_geom.width > 0 {
                            self.clients[i].geom = self.clients[i].float_geom;
                        }
                    }
                }
            }
        }

        // Collect windows to show/hide (avoid borrow conflict during space ops)
        let visible: Vec<(Window, Rect, Rect)> = self
            .clients
            .iter()
            .filter(|c| visible_in_pass(c))
            .map(|c| {
                let configure_geom = if c.animation.running { c.animation.current } else { c.geom };
                (c.window.clone(), c.geom, configure_geom)
            })
            .collect();

        let hidden: Vec<Window> = self
            .clients
            .iter()
            .filter(|c| c.monitor == mon_idx && !visible_in_pass(c))
            .map(|c| c.window.clone())
            .collect();

        for w in hidden {
            self.space.unmap_elem(&w);
        }

        for (window, geom, configure_geom) in visible {
            self.space.map_element(window.clone(), (geom.x, geom.y), false);

            if let WindowSurface::Wayland(toplevel) = window.underlying_surface() {
                tracing::debug!(
                    "arrange: setting toplevel size {}x{}",
                    configure_geom.width,
                    configure_geom.height
                );
                toplevel.with_pending_state(|state| {
                    state.size = Some(Size::from((configure_geom.width, configure_geom.height)));
                });
                // Only send the configure if the initial configure has already
                // gone out. The initial configure must be sent during the first
                // commit (see CompositorHandler::commit).
                let initial_sent = with_states(toplevel.wl_surface(), |states| {
                    states
                        .data_map
                        .get::<XdgToplevelSurfaceData>()
                        .and_then(|d| d.lock().ok().map(|d| d.initial_configure_sent))
                        .unwrap_or(false)
                });
                if initial_sent {
                    toplevel.send_pending_configure();
                }
            }
        }
        self.enforce_z_order();
        crate::border::refresh(self);
        self.request_repaint();
        // Refresh the IPC channels so `mctl clients`/`focused`/`status`
        // and any dwl-ipc-v2 bar (waybar-dwl, noctalia, fnott) see new
        // windows the moment they're laid out. arrange_all already
        // covered both, but arrange_monitor (the path most map/unmap/
        // tag-move events take) didn't — leaving state.json + the bar
        // tag-counts stuck on the boot snapshot of zero.
        self.write_state_file();
        crate::protocols::dwl_ipc::broadcast_monitor(self, mon_idx);
    }

    /// Smithay's `Space::map_element` always inserts the touched
    /// element at the top of the stack — there's no way to map at an
    /// explicit z. So every time `arrange_monitor` re-maps a tile-
    /// layer window during a layout change or a move animation, that
    /// tile silently leaps above any floating window (CopyQ,
    /// pavucontrol, picker dialogs) that happened to be on screen.
    ///
    /// To keep "floating sits on top of tiled" actually true, run
    /// this after every `map_element` storm. We re-`raise_element`
    /// floats first, then overlays/scratchpads, in `clients`-vec
    /// forward order — `raise_element` itself moves to top, so the
    /// last raise per band wins, which means the most-recently-
    /// created float of each band ends up at the top of its band
    /// (sane default for "newly opened picker shows on top").
    pub fn enforce_z_order(&mut self) {
        let floats: Vec<smithay::desktop::Window> = self
            .clients
            .iter()
            .filter(|c| (c.is_floating || c.is_in_scratchpad) && !c.is_overlay)
            .map(|c| c.window.clone())
            .collect();
        for w in &floats {
            self.space.raise_element(w, false);
        }
        let overlays: Vec<smithay::desktop::Window> = self
            .clients
            .iter()
            .filter(|c| c.is_overlay)
            .map(|c| c.window.clone())
            .collect();
        for w in &overlays {
            self.space.raise_element(w, false);
        }
    }

    pub fn focus_surface(&mut self, target: Option<FocusTarget>) {
        let _span = tracy_client::span!("focus_surface");
        // W3.4: push to per-monitor focus_history when a new client
        // takes focus. Walks `target` to a client index, drops dups
        // (same client re-focused = front of queue, no churn), caps
        // at FOCUS_HISTORY_DEPTH.
        const FOCUS_HISTORY_DEPTH: usize = 5;
        if let Some(FocusTarget::Window(w)) = &target {
            let new_idx = self.clients.iter().position(|c| &c.window == w);
            if let Some(idx) = new_idx {
                let mon = self.clients[idx].monitor;
                if mon < self.monitors.len() {
                    let hist = &mut self.monitors[mon].focus_history;
                    hist.retain(|&i| i != idx);
                    hist.push_front(idx);
                    while hist.len() > FOCUS_HISTORY_DEPTH {
                        hist.pop_back();
                    }
                }
            }
        }
        // Capture the *previously* focused client BEFORE we rewrite
        // the keyboard focus — we need the old + new pair to drive
        // the border-colour cross-fade animation below.
        let prev_focus_idx = self.focused_client_idx();

        // Track focus history per-monitor so toplevel_destroyed can recall
        // the previously focused window (niri-style).
        if let Some(FocusTarget::Window(ref w)) = target {
            if let Some(new_idx) = self.clients.iter().position(|c| c.window == *w) {
                let mon_idx = self.clients[new_idx].monitor;
                if mon_idx < self.monitors.len() {
                    let cur = self.monitors[mon_idx].selected;
                    if cur != Some(new_idx) {
                        self.monitors[mon_idx].prev_selected = cur;
                        self.monitors[mon_idx].selected = Some(new_idx);
                    }
                }
            }
        }

        let serial = SERIAL_COUNTER.next_serial();
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, target, serial);
        }

        // Focus highlight cross-fade. When focus moves between two
        // windows, animate both: the outgoing window's border colour
        // fades from `focuscolor` toward `bordercolor`, the incoming
        // one fades the other way. `tick_animations` drives the
        // sample; `border::refresh` reads the in-flight colour from
        // `opacity_animation.current_border_color` and renders that
        // instead of the static color_for() value.
        //
        // Per-client `opacity` (focused_opacity / unfocused_opacity)
        // is animated through the same struct so an unfocused window
        // also dims smoothly instead of snapping to its dimmer alpha
        // — same trick mango/dwl uses but with the right curve.
        let new_focus_idx = self.focused_client_idx();
        if prev_focus_idx != new_focus_idx
            && self.config.animations
            && self.config.animation_duration_focus > 0
        {
            let now = crate::utils::now_ms();
            let dur = self.config.animation_duration_focus;
            let bordercolor = self.config.bordercolor.0;
            let focuscolor = self.config.focuscolor.0;
            // Outgoing: drop focus highlight back to bordercolor +
            // dim opacity to unfocused.
            if let Some(idx) = prev_focus_idx {
                if idx < self.clients.len() {
                    let initial_color =
                        self.clients[idx].opacity_animation.current_border_color;
                    let initial_color = if initial_color == [0.0, 0.0, 0.0, 0.0] {
                        focuscolor
                    } else {
                        initial_color
                    };
                    let initial_opacity = self.clients[idx].focused_opacity;
                    self.clients[idx].opacity_animation = OpacityAnimation {
                        running: true,
                        initial_opacity,
                        target_opacity: self.clients[idx].unfocused_opacity,
                        current_opacity: initial_opacity,
                        time_started: now,
                        duration: dur,
                        initial_border_color: initial_color,
                        target_border_color: bordercolor,
                        current_border_color: initial_color,
                    };
                }
            }
            // Incoming: ramp up to focuscolor + brighten opacity.
            if let Some(idx) = new_focus_idx {
                if idx < self.clients.len() {
                    let initial_color =
                        self.clients[idx].opacity_animation.current_border_color;
                    let initial_color = if initial_color == [0.0, 0.0, 0.0, 0.0] {
                        bordercolor
                    } else {
                        initial_color
                    };
                    let initial_opacity = self.clients[idx].unfocused_opacity;
                    self.clients[idx].opacity_animation = OpacityAnimation {
                        running: true,
                        initial_opacity,
                        target_opacity: self.clients[idx].focused_opacity,
                        current_opacity: initial_opacity,
                        time_started: now,
                        duration: dur,
                        initial_border_color: initial_color,
                        target_border_color: focuscolor,
                        current_border_color: initial_color,
                    };
                }
            }
        }

        // Refresh border colors so the focused/unfocused distinction
        // updates without waiting for the next arrange.
        crate::border::refresh(self);
        self.request_repaint();

        // Broadcast the new focus to dwl-ipc-v2 clients (noctalia,
        // waybar-dwl, …). The struct gets its title / appid /
        // fullscreen / floating fields from `focused_client_idx`,
        // which we just changed; without this the bar would keep
        // showing the previously-focused window's title until the
        // next tag-switch / arrange caused some other broadcast to
        // fire. mango broadcasts on every focus change too — this
        // is straight parity.
        if prev_focus_idx != new_focus_idx {
            crate::protocols::dwl_ipc::broadcast_all(self);
            // Phase 3 scripting: invoke any `on_focus_change`
            // handlers the user registered in init.rhai. Hooks
            // see the new focused state via `focused_appid()` /
            // `focused_title()`. Wrapped in a `prev != new` gate
            // because focus_surface is called speculatively from
            // `refresh_keyboard_focus` and we don't want to fire
            // hooks for no-op refreshes.
            crate::scripting::fire_focus_change(self);
        }
    }

    pub fn post_repaint(&mut self, output: &Output, time: impl Into<std::time::Duration>) {
        // Frame callbacks are now driven by `note_vblank`
        // (real VBlank) and `on_estimated_vblank_timer` (empty render)
        // — both bump the per-output frame_callback_sequence and call
        // `send_frame_callbacks`. The post-render hook stays only to
        // refresh the smithay desktop space + popup map.
        let _ = time;
        let _ = output;
        self.space.refresh();
        self.popups.cleanup();
    }

    /// Send `wl_surface.frame` done callbacks to every surface visible
    /// on `output`, with niri-style sequence-based dedup: each surface
    /// gets at most ONE callback per output refresh cycle, no matter
    /// how many commits the client has fired since the last cycle.
    /// This is what stops gtk4-layer-shell clients (mshell-frame,
    /// noctalia) from getting back-to-back `frame_done` events and
    /// entering a busy commit loop — the same loop that made
    /// mshell-on-margo flicker while mshell-on-Hyprland and
    /// mshell-on-niri are smooth.
    pub fn send_frame_callbacks(&mut self, output: &Output, time: impl Into<std::time::Duration>) {
        let time = time.into();
        // Current sequence for this output. Bumped by `note_vblank`
        // and `on_estimated_vblank_timer`; surfaces stamped with this
        // value already received a frame_done this cycle.
        let sequence = *self
            .frame_callback_sequence
            .entry(output.name())
            .or_insert(0);

        // Backup throttle handed to smithay's send_frame. niri uses
        // 995 ms — long enough that it never interferes with our
        // per-cycle dedup. The real cadence comes from how often *we*
        // call this function (once per real or estimated vblank).
        let throttle = Some(std::time::Duration::from_millis(995));

        // ONLY sequence-based dedup; we deliberately do NOT add
        // niri's `surface_primary_scanout_output` filter here. Even
        // though `update_primary_scanout_outputs` now runs in the
        // udev render path, subsurfaces without per-frame buffers
        // (gtk4-layer-shell widget containers, IME composition
        // surfaces, etc.) end up with `primary_scanout = None`
        // because smithay's `element_was_presented` reports false
        // for any surface that produced no render element this
        // frame. Applying the filter then silently drops frame_done
        // callbacks for those surfaces, and the clients that own
        // them (Helium toolbar surface, mshell GTK frame clock)
        // enter a "frame_done never arrived → render harder"
        // recovery loop that burns RAM and locks the system in
        // ~10 s. Output-level filtering is still done upstream by
        // `space.outputs_for_element` / `layer_map_for_output`
        // iteration in the caller, so dropping the primary_scanout
        // gate doesn't cause cross-output spam.
        let should_send = |_surface: &WlSurface,
                           states: &smithay::wayland::compositor::SurfaceData|
         -> Option<Output> {
            let frame_state = states
                .data_map
                .get_or_insert(SurfaceFrameThrottlingState::default);
            let mut last_sent_at = frame_state.last_sent_at.borrow_mut();
            if let Some((last_output, last_sequence)) = &*last_sent_at {
                if last_output == output && *last_sequence == sequence {
                    return None;
                }
            }
            *last_sent_at = Some((output.clone(), sequence));
            Some(output.clone())
        };

        self.space.elements().for_each(|window| {
            if self.space.outputs_for_element(window).contains(output) {
                window.send_frame(output, time, throttle, should_send);
            }
        });

        let map = layer_map_for_output(output);
        for layer in map.layers() {
            layer.send_frame(output, time, throttle, should_send);
        }
    }

    /// Schedule an estimated-vblank Timer for `output`. Called from the
    /// udev render path when `render_frame` reports `is_empty == true`
    /// (no damage, no DRM page-flip, no real VBlank coming back). Without
    /// this, gtk4-layer-shell clients would stall waiting for a
    /// `wl_surface.frame` callback that the empty-render path swallowed.
    ///
    /// At most ONE timer in flight per output. If a timer is already
    /// queued we return early — the existing timer will fire at the next
    /// estimated vblank.
    pub fn queue_estimated_vblank_timer(
        &mut self,
        output: &Output,
        refresh_interval: std::time::Duration,
    ) {
        use smithay::reexports::calloop::{
            timer::{TimeoutAction, Timer},
            RegistrationToken,
        };

        let name = output.name();
        if self.estimated_vblank_timers.contains_key(&name) {
            return;
        }

        let timer = Timer::from_duration(refresh_interval);
        let cb_name = name.clone();
        let cb_output = output.clone();
        let token: Result<RegistrationToken, _> =
            self.loop_handle.insert_source(timer, move |_, _, state| {
                // Only do work if our entry is still here. If
                // `note_vblank` (real VBlank) raced us and removed
                // the entry first, our callback fires harmlessly —
                // returning `Drop` is still correct because the
                // source is one-shot.
                if state.estimated_vblank_timers.remove(&cb_name).is_some() {
                    state.on_estimated_vblank_timer(&cb_output);
                }
                TimeoutAction::Drop
            });

        match token {
            Ok(t) => {
                self.estimated_vblank_timers.insert(name, t);
            }
            Err(e) => {
                tracing::warn!("queue_estimated_vblank_timer insert_source failed: {e}");
            }
        }
    }

    /// Fired by the estimated-vblank Timer. Bumps the per-output
    /// frame_callback_sequence and re-sends frame callbacks so clients
    /// stay paced at refresh rate even when the scene is idle.
    pub fn on_estimated_vblank_timer(&mut self, output: &Output) {
        let entry = self
            .frame_callback_sequence
            .entry(output.name())
            .or_insert(0);
        *entry = entry.wrapping_add(1);
        let now = self.clock.now();
        self.send_frame_callbacks(output, now);
        self.display_handle.flush_clients().ok();
    }

    // ── Focus helpers ─────────────────────────────────────────────────────────

    pub fn focused_client_idx(&self) -> Option<usize> {
        let keyboard = self.seat.get_keyboard()?;
        let focus = keyboard.current_focus()?;
        if let FocusTarget::Window(focused) = focus {
            self.clients.iter().position(|c| c.window == focused)
        } else {
            None
        }
    }

    pub fn focused_monitor(&self) -> usize {
        self.focused_client_idx()
            .map(|i| self.clients[i].monitor)
            .or_else(|| self.pointer_monitor())
            .unwrap_or(0)
    }

    /// Centralised "what should keyboard focus be right now?" — the niri
    /// pattern. We can't rely on transitional events (layer_destroyed
    /// alone, set_focus from new_surface) because real clients change
    /// focus state in ways those events don't fire for:
    ///
    ///   * **noctalia's launcher / settings panels** don't create or
    ///     destroy a layer surface when they open/close. They keep one
    ///     `MainScreen` `WlrLayershell` per output and just toggle its
    ///     `keyboardFocus` between `Exclusive` and `None`. The transition
    ///     surfaces only as a `wl_surface.commit` with a different
    ///     cached `keyboard_interactivity` — no destroy callback, no
    ///     unmap. Without recomputing focus on every layer commit we
    ///     never notice the panel closed and the key events keep going
    ///     into the void.
    ///   * **session lock with multiple outputs**. Quickshell creates one
    ///     `WlSessionLockSurface` per screen; only the surface on the
    ///     output the user is looking at should hold focus, and that has
    ///     to track cursor motion across outputs.
    ///
    /// This method picks a target by priority and pushes it through the
    /// existing `focus_surface` plumbing only if it differs from the
    /// current focus, so it's cheap to call after every relevant event.
    pub fn refresh_keyboard_focus(&mut self) {
        let desired = self.compute_desired_focus();

        let current = self.seat.get_keyboard().and_then(|kb| kb.current_focus());
        if current.as_ref() == desired.as_ref() {
            tracing::debug!(
                "refresh_keyboard_focus: noop (locked={}, current={:?})",
                self.session_locked,
                current.as_ref().map(focus_target_label),
            );
            return;
        }
        tracing::info!(
            "refresh_keyboard_focus: locked={} current={:?} -> desired={:?}",
            self.session_locked,
            current.as_ref().map(focus_target_label),
            desired.as_ref().map(focus_target_label),
        );
        self.focus_surface(desired);
    }

    fn compute_desired_focus(&self) -> Option<FocusTarget> {
        if self.session_locked {
            // Lock surface on the output under the cursor wins, with
            // graceful fallbacks: focused-monitor's surface, then any
            // surface (so we never end up locked with no focus at all).
            let pointer_output = self
                .monitor_at_point(self.input_pointer.x, self.input_pointer.y)
                .and_then(|i| self.monitors.get(i).map(|m| m.output.clone()));

            if let Some(out) = pointer_output {
                if let Some((_, s)) =
                    self.lock_surfaces.iter().find(|(o, _)| o == &out)
                {
                    return Some(FocusTarget::SessionLock(s.clone()));
                }
            }
            return self
                .lock_surfaces
                .first()
                .map(|(_, s)| FocusTarget::SessionLock(s.clone()));
        }

        // Highest-priority Exclusive layer on Top/Overlay anywhere.
        for layer in self.layer_shell_state.layer_surfaces().rev() {
            let exclusive = layer.with_cached_state(|data| {
                data.keyboard_interactivity
                    == smithay::wayland::shell::wlr_layer::KeyboardInteractivity::Exclusive
                    && matches!(
                        data.layer,
                        smithay::wayland::shell::wlr_layer::Layer::Top
                            | smithay::wayland::shell::wlr_layer::Layer::Overlay
                    )
            });
            if !exclusive {
                continue;
            }
            let mapped = self.space.outputs().find_map(|output| {
                let map = layer_map_for_output(output);
                map.layers()
                    .find(|m| m.layer_surface() == &layer)
                    .map(|m| m.layer_surface().clone())
            });
            if let Some(s) = mapped {
                return Some(FocusTarget::LayerSurface(s));
            }
        }

        // Otherwise: monitor's last-selected client (focus history),
        // falling back to the topmost visible client on the same monitor.
        let mon_idx = self.pointer_monitor().or_else(|| {
            self.focused_client_idx().map(|i| self.clients[i].monitor)
        })?;
        if mon_idx >= self.monitors.len() {
            return None;
        }
        let tagset = self.monitors[mon_idx].current_tagset();
        if let Some(idx) = self.monitors[mon_idx].selected.filter(|&i| {
            i < self.clients.len()
                && self.clients[i].monitor == mon_idx
                && self.clients[i].is_visible_on(mon_idx, tagset)
        }) {
            return Some(FocusTarget::Window(self.clients[idx].window.clone()));
        }
        let idx = self
            .clients
            .iter()
            .position(|c| c.monitor == mon_idx && c.is_visible_on(mon_idx, tagset))?;
        Some(FocusTarget::Window(self.clients[idx].window.clone()))
    }

    /// For scroller layout, return the client-vector index where a newly
    /// created window should land — right after the currently focused client
    /// on the same monitor. Returns `None` if the target monitor isn't using
    /// scroller (any layout) or if there's no focused client there.
    fn scroller_insert_position(&self, target_mon: usize) -> Option<usize> {
        let mon = self.monitors.get(target_mon)?;
        if mon.current_layout() != crate::layout::LayoutId::Scroller {
            return None;
        }
        let focused_idx = self.focused_client_idx()?;
        if self.clients[focused_idx].monitor != target_mon {
            return None;
        }
        Some(focused_idx + 1)
    }

    /// Inserting a client mid-vec invalidates any monitor.selected /
    /// prev_selected indices that pointed at positions ≥ insert position.
    /// Bump them up by one so they keep referring to the same client.
    fn shift_indices_at_or_after(&mut self, insert_pos: usize) {
        for mon in self.monitors.iter_mut() {
            if let Some(s) = mon.selected.as_mut() {
                if *s >= insert_pos {
                    *s += 1;
                }
            }
            if let Some(s) = mon.prev_selected.as_mut() {
                if *s >= insert_pos {
                    *s += 1;
                }
            }
        }
    }

    /// Inverse of `shift_indices_at_or_after`: a client at `removed_pos` was
    /// just dropped. Shift any monitor index pointing at a later position
    /// down by one, and clear those that pointed exactly at the removed slot.
    fn shift_indices_after_remove(&mut self, removed_pos: usize) {
        for mon in self.monitors.iter_mut() {
            for slot in [&mut mon.selected, &mut mon.prev_selected] {
                if let Some(s) = slot.as_mut() {
                    if *s == removed_pos {
                        *slot = None;
                    } else if *s > removed_pos {
                        *s -= 1;
                    }
                }
            }
        }
    }

    fn pointer_monitor(&self) -> Option<usize> {
        self.monitor_at_point(self.input_pointer.x, self.input_pointer.y)
    }

    fn monitor_at_point(&self, x: f64, y: f64) -> Option<usize> {
        self.monitors.iter().position(|mon| {
            let area = mon.monitor_area;
            x >= area.x as f64
                && y >= area.y as f64
                && x < (area.x + area.width) as f64
                && y < (area.y + area.height) as f64
        })
    }

    pub fn clamp_pointer_to_outputs(&mut self) {
        if self.monitors.is_empty() {
            return;
        }

        let mut min_x = self.monitors[0].monitor_area.x;
        let mut min_y = self.monitors[0].monitor_area.y;
        let mut max_x = self.monitors[0].monitor_area.x + self.monitors[0].monitor_area.width;
        let mut max_y = self.monitors[0].monitor_area.y + self.monitors[0].monitor_area.height;

        for mon in &self.monitors[1..] {
            let area = mon.monitor_area;
            min_x = min_x.min(area.x);
            min_y = min_y.min(area.y);
            max_x = max_x.max(area.x + area.width);
            max_y = max_y.max(area.y + area.height);
        }

        self.input_pointer.x = self.input_pointer.x.clamp(min_x as f64, (max_x - 1) as f64);
        self.input_pointer.y = self.input_pointer.y.clamp(min_y as f64, (max_y - 1) as f64);
    }

    pub fn default_layout(&self) -> LayoutId {
        LayoutId::from_name(&self.config.default_layout).unwrap_or(LayoutId::Tile)
    }

    /// Look up the "home monitor" for a given tag bitmask, by matching
    /// any single bit in the mask against `tagrule = id:N,monitor_name:X`
    /// entries. Returns the monitor index if exactly one tag is set in
    /// the mask AND a tagrule pins it. Used by `view_tag` and
    /// `new_toplevel` to route cross-monitor.
    pub fn tag_home_monitor(&self, tagmask: u32) -> Option<usize> {
        if tagmask == 0 {
            return None;
        }
        // Translate single-bit mask to 1-indexed tag id.
        let id = if tagmask.is_power_of_two() {
            (tagmask.trailing_zeros() + 1) as i32
        } else {
            // Multi-tag mask — use the lowest set bit.
            ((tagmask & tagmask.wrapping_neg()).trailing_zeros() + 1) as i32
        };
        let name = self
            .config
            .tag_rules
            .iter()
            .find(|r| r.id == id && r.monitor_name.is_some())
            .and_then(|r| r.monitor_name.clone())?;
        self.monitors.iter().position(|m| m.name == name)
    }

    pub fn apply_tag_rules_to_monitor(&mut self, mon_idx: usize) {
        let Some(mon) = self.monitors.get_mut(mon_idx) else {
            return;
        };

        for rule in &self.config.tag_rules {
            if rule.id <= 0 || rule.id as usize > crate::MAX_TAGS {
                continue;
            }
            if let Some(name) = &rule.monitor_name {
                if name != &mon.name {
                    continue;
                }
            }

            let tag = rule.id as usize;
            if let Some(layout_name) = &rule.layout_name {
                if let Some(layout) = LayoutId::from_name(layout_name) {
                    mon.pertag.ltidxs[tag] = layout;
                }
            }
            if rule.mfact > 0.0 {
                mon.pertag.mfacts[tag] = rule.mfact.clamp(0.05, 0.95);
            }
            if rule.nmaster > 0 {
                mon.pertag.nmasters[tag] = rule.nmaster as u32;
            }
            if let Some(wp) = &rule.wallpaper {
                mon.pertag.wallpapers[tag] = wp.clone();
            }
        }
    }

    /// Move keyboard focus + cursor "home" onto the given monitor. Does
    /// NOT change the monitor's current tagset — the caller (view_tag,
    /// focus_mon) is responsible for that. Used by view_tag's tag-home
    /// redirect: if the user presses super+N for a tag pinned to another
    /// monitor, we warp here first so the upcoming view operation
    /// happens in the right place.
    pub fn warp_focus_to_monitor(&mut self, mon_idx: usize) {
        if mon_idx >= self.monitors.len() {
            return;
        }
        let area = self.monitors[mon_idx].monitor_area;
        // Center the pointer on the target monitor so subsequent
        // sloppy-focus / focus-under lookups land on this output.
        self.input_pointer.x = (area.x + area.width / 2) as f64;
        self.input_pointer.y = (area.y + area.height / 2) as f64;
        self.focus_first_visible_or_clear(mon_idx);
    }

    fn focus_first_visible_or_clear(&mut self, mon_idx: usize) {
        if mon_idx >= self.monitors.len() {
            self.focus_surface(None);
            return;
        }

        let tagset = self.monitors[mon_idx].current_tagset();
        if let Some(idx) = self.clients.iter().position(|c| c.is_visible_on(mon_idx, tagset)) {
            self.monitors[mon_idx].selected = Some(idx);
            let window = self.clients[idx].window.clone();
            self.focus_surface(Some(FocusTarget::Window(window)));
        } else {
            self.monitors[mon_idx].selected = None;
            self.focus_surface(None);
        }
    }

    fn update_pertag_for_tagset(&mut self, mon_idx: usize, tagmask: u32) {
        let Some(mon) = self.monitors.get_mut(mon_idx) else {
            return;
        };

        mon.pertag.prevtag = mon.pertag.curtag;
        mon.pertag.curtag = if tagmask.count_ones() == 1 {
            tagmask.trailing_zeros() as usize + 1
        } else {
            0
        };
    }


    /// Why a window-rule reapply is happening. Lets the single
    /// reapply path log meaningfully and (in future) skip rule subsets
    /// that don't make sense for a given trigger (e.g. `tags:`
    /// shouldn't move a client on `Reload`).
    fn apply_window_rules(&self, client: &mut MargoClient) {
        // Pre-mount path (X11 + initial XDG before the client is in
        // `self.clients`). The post-mount equivalent is
        // [`reapply_rules`].
        let rules = self.matching_window_rules(&client.app_id, &client.title);
        Self::apply_matched_window_rules(&self.monitors, client, &rules);
    }

    /// Single post-mount window-rule reapply path. All three trigger
    /// sites — initial XDG mount, late `app_id` settle, config reload —
    /// route through this with a [`WindowRuleReason`] tag so the debug
    /// log says *why* a rule fired.
    pub(crate) fn reapply_rules(
        &mut self,
        idx: usize,
        reason: WindowRuleReason,
    ) -> bool {
        if idx >= self.clients.len() {
            return false;
        }
        let (app_id, title) = {
            let client = &self.clients[idx];
            (client.app_id.clone(), client.title.clone())
        };
        let rules = self.matching_window_rules(&app_id, &title);
        if rules.is_empty() {
            tracing::trace!(
                target: "windowrule",
                reason = ?reason,
                app_id = %app_id,
                title = %title,
                "reapply: no rules match",
            );
            return false;
        }
        Self::apply_matched_window_rules(&self.monitors, &mut self.clients[idx], &rules);
        tracing::debug!(
            target: "windowrule",
            reason = ?reason,
            count = rules.len(),
            app_id = %app_id,
            title = %title,
            "reapply: applied {} rules",
            rules.len(),
        );
        true
    }

    /// Live-swap the visual theme without touching `~/.config/margo/config.conf`.
    ///
    /// Three built-in presets:
    ///   * `default` — restore the values parsed from the config file at
    ///     startup (or the most recent `mctl reload`).
    ///   * `minimal` — borders thin, shadows off, blur off, square corners.
    ///     Good for low-end GPUs or anyone who likes a flat look.
    ///   * `gaudy`   — chunky borders, deep drop shadows, rounded corners,
    ///     blur on. Demo / screenshot mode.
    ///
    /// The first call captures the current config values into
    /// `theme_baseline` so `default` always means "what was on disk
    /// before the user started swapping". `mctl reload` re-invalidates
    /// the baseline so reload + `default` gives the freshly-parsed
    /// values.
    ///
    /// Returns `Err(reason)` for an unknown preset name; the dispatch
    /// handler turns this into a user-visible warning.
    pub fn apply_theme_preset(&mut self, name: &str) -> Result<(), String> {
        // Lazy capture — first preset switch establishes the
        // "what the config file said" baseline.
        if self.theme_baseline.is_none() {
            self.theme_baseline = Some(ThemeBaseline::capture(&self.config));
        }
        let baseline = self.theme_baseline.as_ref().unwrap().clone();

        match name {
            "default" => baseline.apply_to(&mut self.config),
            "minimal" => {
                self.config.shadows = false;
                self.config.layer_shadows = false;
                self.config.shadow_only_floating = false;
                self.config.blur = false;
                self.config.blur_layer = false;
                self.config.border_radius = 0;
                self.config.borderpx = 1;
            }
            "gaudy" => {
                self.config.shadows = true;
                self.config.layer_shadows = true;
                self.config.shadows_size = 32;
                self.config.shadows_blur = 18.0;
                self.config.border_radius = 14;
                self.config.borderpx = 4;
            }
            other => {
                return Err(format!(
                    "unknown theme preset `{other}` — try `default`, `minimal`, or `gaudy`"
                ));
            }
        }

        // Border / shadow / blur all read straight off `self.config`
        // every frame, so an arrange + repaint is enough — no
        // per-client mutation, no animation re-bake.
        self.arrange_all();
        self.request_repaint();
        tracing::info!(target: "theme", "applied preset `{name}`");
        Ok(())
    }

    pub(crate) fn matching_window_rules(&self, app_id: &str, title: &str) -> Vec<WindowRule> {
        self.config
            .window_rules
            .iter()
            .filter(|rule| self.window_rule_matches(rule, app_id, title))
            .cloned()
            .collect()
    }

    fn apply_matched_window_rules(
        monitors: &[MargoMonitor],
        client: &mut MargoClient,
        rules: &[WindowRule],
    ) {
        for rule in rules {
            if rule.tags != 0 {
                client.tags = rule.tags;
            }
            if let Some(monitor_name) = &rule.monitor {
                if let Some(mon_idx) = monitors.iter().position(|mon| &mon.name == monitor_name) {
                    client.monitor = mon_idx;
                }
            }

            if let Some(value) = rule.is_floating {
                client.is_floating = value;
            }
            if let Some(value) = rule.is_fullscreen {
                client.is_fullscreen = value;
            }
            if let Some(value) = rule.is_fake_fullscreen {
                client.is_fake_fullscreen = value;
            }
            if let Some(value) = rule.no_border {
                client.no_border = value;
            }
            if let Some(value) = rule.no_shadow {
                client.no_shadow = value;
            }
            if let Some(value) = rule.no_radius {
                client.no_radius = value;
            }
            if let Some(value) = rule.no_animation {
                client.no_animation = value;
            }
            if let Some(value) = rule.border_width {
                client.border_width = value;
            }
            if let Some(value) = rule.open_silent {
                client.open_silent = value;
            }
            if let Some(value) = rule.tag_silent {
                client.tag_silent = value;
            }
            if let Some(value) = rule.is_named_scratchpad {
                client.is_named_scratchpad = value;
            }
            if let Some(value) = rule.is_unglobal {
                client.is_unglobal = value;
            }
            if let Some(value) = rule.is_global {
                client.is_global = value;
            }
            if let Some(value) = rule.is_overlay {
                client.is_overlay = value;
            }
            if let Some(value) = rule.no_focus {
                client.no_focus = value;
            }
            if let Some(value) = rule.no_fade_in {
                client.no_fade_in = value;
            }
            if let Some(value) = rule.no_fade_out {
                client.no_fade_out = value;
            }
            if let Some(value) = rule.is_term {
                client.is_term = value;
            }
            if let Some(value) = rule.allow_csd {
                client.allow_csd = value;
            }
            if let Some(value) = rule.force_fake_maximize {
                client.force_fake_maximize = value;
            }
            if let Some(value) = rule.force_tiled_state {
                client.force_tiled_state = value;
                if value {
                    client.is_floating = false;
                }
            }
            if let Some(value) = rule.no_swallow {
                client.no_swallow = value;
            }
            if let Some(value) = rule.no_blur {
                client.no_blur = value;
            }
            if let Some(value) = rule.canvas_no_tile {
                client.canvas_no_tile = value;
            }
            if let Some(value) = rule.scroller_proportion {
                client.scroller_proportion = value.clamp(0.1, 1.0);
            }
            if let Some(value) = rule.scroller_proportion_single {
                client.scroller_proportion_single = value.clamp(0.1, 1.0);
            }
            if let Some(value) = rule.focused_opacity {
                client.focused_opacity = value.clamp(0.0, 1.0);
            }
            if let Some(value) = rule.unfocused_opacity {
                client.unfocused_opacity = value.clamp(0.0, 1.0);
            }
            // Per-window animation-type overrides. The rule's
            // `animation_type_open` / `animation_type_close` win over
            // the global config when the window opens or closes —
            // `finalize_initial_map` and `toplevel_destroyed` already
            // read these per-client fields and only fall back to the
            // global `Config::animation_type_*` when they're `None`.
            if let Some(value) = rule.animation_type_open.as_ref() {
                client.animation_type_open = Some(value.clone());
            }
            if let Some(value) = rule.animation_type_close.as_ref() {
                client.animation_type_close = Some(value.clone());
            }
            // Niri-style additions.
            if rule.min_width > 0 {
                client.min_width = rule.min_width;
            }
            if rule.min_height > 0 {
                client.min_height = rule.min_height;
            }
            if rule.max_width > 0 {
                client.max_width = rule.max_width;
            }
            if rule.max_height > 0 {
                client.max_height = rule.max_height;
            }
            if let Some(focused) = rule.open_focused {
                // open_focused=false → equivalent to no_focus=true
                client.no_focus = !focused;
            }
            if let Some(value) = rule.block_out_from_screencast {
                client.block_out_from_screencast = value;
            }
            if rule.width > 0 || rule.height > 0 || rule.offset_x != 0 || rule.offset_y != 0 {
                client.is_floating = true;
                client.float_geom = Self::rule_float_geometry_for(monitors, client.monitor, rule);
            }
        }
        // After all matched rules are applied, clamp the floating geometry
        // to any size constraints picked up.
        clamp_size(
            &mut client.float_geom.width,
            &mut client.float_geom.height,
            client.min_width,
            client.min_height,
            client.max_width,
            client.max_height,
        );
    }

    pub(crate) fn window_rule_matches(
        &self,
        rule: &WindowRule,
        app_id: &str,
        title: &str,
    ) -> bool {
        // Positive matches: every present pattern must match.
        let app_ok = rule
            .id
            .as_deref()
            .filter(|p| !p.is_empty())
            .map(|p| matches_rule_text(p, app_id))
            .unwrap_or(true);
        let title_ok = rule
            .title
            .as_deref()
            .filter(|p| !p.is_empty())
            .map(|p| matches_rule_text(p, title))
            .unwrap_or(true);
        if !(app_ok && title_ok) {
            return false;
        }

        // Negative matches (niri-style): if either exclude pattern matches,
        // the rule is rejected even if the positive matches succeed.
        if let Some(p) = rule.exclude_id.as_deref().filter(|p| !p.is_empty()) {
            if matches_rule_text(p, app_id) {
                return false;
            }
        }
        if let Some(p) = rule.exclude_title.as_deref().filter(|p| !p.is_empty()) {
            if matches_rule_text(p, title) {
                return false;
            }
        }
        true
    }

    fn rule_float_geometry(&self, mon_idx: usize, rule: &WindowRule) -> Rect {
        Self::rule_float_geometry_for(&self.monitors, mon_idx, rule)
    }

    fn rule_float_geometry_for(monitors: &[MargoMonitor], mon_idx: usize, rule: &WindowRule) -> Rect {
        let area = monitors
            .get(mon_idx)
            .map(|mon| mon.work_area)
            .unwrap_or_else(|| Rect::new(0, 0, 1280, 720));
        let width = if rule.width > 0 {
            rule.width.min(area.width)
        } else {
            (area.width as f32 * 0.6) as i32
        };
        let height = if rule.height > 0 {
            rule.height.min(area.height)
        } else {
            (area.height as f32 * 0.6) as i32
        };

        Rect::new(
            area.x + (area.width - width) / 2 + rule.offset_x,
            area.y + (area.height - height) / 2 + rule.offset_y,
            width,
            height,
        )
    }

    fn refresh_wayland_toplevel_identity(&mut self, window: &Window, toplevel: &ToplevelSurface) {
        let (app_id, title) = read_toplevel_identity(toplevel);
        let Some(idx) = self.clients.iter().position(|client| client.window == *window) else {
            return;
        };

        let (app_id_changed, title_changed, old_monitor, handle) = {
            let client = &mut self.clients[idx];
            let app_id_changed = client.app_id != app_id;
            let title_changed = client.title != title;
            if !app_id_changed && !title_changed {
                return;
            }

            let old_monitor = client.monitor;
            let handle = client.foreign_toplevel_handle.clone();
            client.app_id = app_id.clone();
            client.title = title.clone();
            (app_id_changed, title_changed, old_monitor, handle)
        };

        if let Some(handle) = handle {
            if app_id_changed {
                handle.send_app_id(&app_id);
            }
            if title_changed {
                handle.send_title(&title);
            }
            handle.send_done();
        }

        let title_rules_exist = self.config.window_rules.iter().any(|rule| {
            rule.title.as_ref().is_some_and(|pattern| !pattern.is_empty())
                || rule.exclude_title.as_ref().is_some_and(|pattern| !pattern.is_empty())
        });
        let should_reapply_rules = (app_id_changed && !app_id.is_empty())
            || (title_changed && !title.is_empty() && title_rules_exist);

        if should_reapply_rules && self.reapply_rules(idx, WindowRuleReason::AppIdSettled) {
            let new_monitor = self.clients[idx].monitor;
            if old_monitor != new_monitor {
                self.arrange_monitor(old_monitor);
            }
            self.arrange_monitor(new_monitor);
            crate::protocols::dwl_ipc::broadcast_all(self);
        } else if title_changed || app_id_changed {
            // Even when no rule reapply was needed (the client just
            // changed its title — e.g. browser tab switch — and no
            // title-keyed rules exist), noctalia / waybar-dwl still
            // care about the new title / app_id for their focused-
            // window indicator. Mango broadcasts on every title
            // commit; without this the bar would freeze on the
            // previous title until something else triggered a
            // broadcast.
            crate::protocols::dwl_ipc::broadcast_all(self);
        }
    }

}


// ── Smithay delegate: XDG decoration ─────────────────────────────────────────

impl MargoState {
    /// What decoration mode should we send to a freshly-bound or
    /// reset toplevel? Defaults to `ServerSide`; flips to
    /// `ClientSide` only when the client is in our `clients` vec
    /// and matches a window-rule that whitelists CSD. At the time
    /// `new_decoration` fires the toplevel may not even be in
    /// `clients` yet (xdg-decoration arrives before the first
    /// commit), in which case we ALSO check the raw window-rule
    /// list against the toplevel's current app_id / title — the
    /// rule machinery would otherwise only kick in at
    /// `finalize_initial_map`, too late to influence the very first
    /// configure.
    fn decoration_mode_for(&self, toplevel: &ToplevelSurface) -> XdgDecorationMode {
        if self.client_allows_csd(toplevel) {
            XdgDecorationMode::ClientSide
        } else {
            XdgDecorationMode::ServerSide
        }
    }

    fn client_allows_csd(&self, toplevel: &ToplevelSurface) -> bool {
        let wl_surface = toplevel.wl_surface();
        // Path A: client already mapped — read the resolved
        // `allow_csd` flag right off the `MargoClient`.
        if let Some(client) = self.clients.iter().find(|c| {
            c.window.wl_surface().as_deref() == Some(wl_surface)
        }) {
            return client.allow_csd;
        }
        // Path B: client is between role bind and first commit —
        // best we can do is look up the window-rule by the
        // toplevel's currently-set app_id / title. This is the
        // path that fires for the *first* `xdg_decoration.configure`
        // many compositors get wrong (Chromium / Firefox often
        // bind decoration before any role-data commit, so the
        // initial mode the user sees depends entirely on what we
        // send right now).
        let (app_id, title) = read_toplevel_identity(toplevel);
        self.config
            .window_rules
            .iter()
            .filter(|rule| self.window_rule_matches(rule, &app_id, &title))
            .any(|rule| rule.allow_csd == Some(true))
    }
}

// ── Smithay delegate: SHM ────────────────────────────────────────────────────

impl ShmHandler for MargoState {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}
delegate_shm!(MargoState);

// ── Smithay delegate: Seat ────────────────────────────────────────────────────

impl SeatHandler for MargoState {
    type KeyboardFocus = FocusTarget;
    /// Pointer focus is a raw `WlSurface` — that lets us route events to
    /// the actual subsurface (popups, GTK file picker child surfaces, etc.)
    /// instead of always to the toplevel. Without this, pointer events on
    /// menus / file lists land on the parent surface and the client
    /// translates them as if the parent were under the cursor — exactly the
    /// "imleç başka yerde, seçim başka yerde" symptom.
    type PointerFocus = WlSurface;
    type TouchFocus = FocusTarget;

    fn seat_state(&mut self) -> &mut SeatState<MargoState> {
        &mut self.seat_state
    }
    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&FocusTarget>) {
        // Route clipboard / primary-selection events to the newly focused
        // client. Without this, copy-paste between apps and clipboard
        // managers (CopyQ, cliphist, clipse) silently fail.
        let dh = &self.display_handle;
        let client = focused
            .and_then(|target| target.wl_surface())
            .and_then(|surface| dh.get_client(surface.id()).ok());
        set_data_device_focus(dh, seat, client.clone());
        set_primary_focus(dh, seat, client);
    }
    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        self.cursor_status = image;
        self.request_repaint();
    }
}
delegate_seat!(MargoState);
smithay::delegate_virtual_keyboard_manager!(MargoState);

// `wp_cursor_shape_v1`: clients send a shape name instead of their
// own cursor surface; we draw via `cursor_manager` at our own size.
// Required for GTK4 layer-shell surfaces to avoid oversized cursor.
impl smithay::wayland::tablet_manager::TabletSeatHandler for MargoState {}
smithay::delegate_cursor_shape!(MargoState);

// ── Smithay delegate: Output ──────────────────────────────────────────────────

impl OutputHandler for MargoState {}
delegate_output!(MargoState);



// ── ForeignToplevelListHandler ────────────────────────────────────────────────

impl ForeignToplevelListHandler for MargoState {
    fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelListState {
        &mut self.foreign_toplevel_list
    }
}

smithay::delegate_foreign_toplevel_list!(MargoState);

// ── XwmHandler: X11 window management ────────────────────────────────────────

impl MargoState {
    fn find_x11_client(&self, window: &X11Surface) -> Option<usize> {
        let id = window.window_id();
        self.clients.iter().position(|c| {
            matches!(c.window.underlying_surface(), WindowSurface::X11(s) if s.window_id() == id)
        })
    }

    fn register_x11_window(&mut self, x11surface: X11Surface) {
        let window = Window::new_x11_window(x11surface);
        let mon_idx = self.focused_monitor();
        let tags = self.monitors.get(mon_idx).map(|m| m.current_tagset()).unwrap_or(1);
        let mut client = MargoClient::new(window.clone(), mon_idx, tags, &self.config);
        client.surface_type = crate::SurfaceType::X11;
        client.title = window.x11_surface().map(|s| s.title()).unwrap_or_default();
        client.app_id = window.x11_surface().map(|s| s.class()).unwrap_or_default();
        self.apply_window_rules(&mut client);

        // Tag-home redirect: if a windowrule set `tags:N` but DIDN'T pin
        // a `monitor:`, route to the tag's home monitor as defined by
        // `tagrule = id:N, monitor_name:X`. Lets the user write
        //   tagrule = id:7, monitor_name:eDP-1
        //   windowrule = tags:7, appid:^transmission$
        // and the windowrule doesn't have to repeat `monitor:eDP-1`.
        let no_explicit_monitor = !self
            .matching_window_rules(&client.app_id, &client.title)
            .iter()
            .any(|r| r.monitor.is_some());
        if no_explicit_monitor {
            if let Some(home) = self.tag_home_monitor(client.tags) {
                client.monitor = home;
            }
        }

        let target_mon = client.monitor;
        let focus_new = !client.no_focus && !client.open_silent;
        let ft_handle = self.foreign_toplevel_list.new_toplevel::<Self>(&client.title, &client.app_id);
        ft_handle.send_done();
        client.foreign_toplevel_handle = Some(ft_handle);
        self.clients.push(client);
        let map_loc = self
            .monitors
            .get(target_mon)
            .map(|m| (m.monitor_area.x, m.monitor_area.y))
            .unwrap_or((0, 0));
        self.space.map_element(window.clone(), map_loc, true);
        if focus_new {
            self.focus_surface(Some(FocusTarget::Window(window)));
        }
        if !self.monitors.is_empty() {
            self.arrange_monitor(target_mon);
        }
        tracing::info!(
            app_id = %self.clients.last().map(|c| c.app_id.as_str()).unwrap_or(""),
            monitor = target_mon,
            "new x11 toplevel",
        );
        // Refresh xdp-gnome's window picker — same path the
        // Wayland finalize_initial_map handler uses.
        self.emit_windows_changed();
    }

    fn remove_x11_window(&mut self, x11surface: &X11Surface) {
        if let Some(idx) = self.find_x11_client(x11surface) {
            let app_id = self.clients[idx].app_id.clone();
            let title = self.clients[idx].title.clone();
            if let Some(handle) = self.clients[idx].foreign_toplevel_handle.take() {
                handle.send_closed();
            }
            let window = self.clients[idx].window.clone();
            self.space.unmap_elem(&window);
            self.clients.remove(idx);
            self.shift_indices_after_remove(idx);
            let mon_idx = self.focused_monitor();
            if !self.monitors.is_empty() {
                self.arrange_monitor(mon_idx);
            }
            // Refresh xdp-gnome's window picker — same path the
            // Wayland toplevel_destroyed handler uses.
            self.emit_windows_changed();
            crate::scripting::fire_window_close(self, &app_id, &title);
        }
    }
}


// ── Smithay delegate: Viewporter ───────────────────────────────────────────────

smithay::delegate_viewporter!(MargoState);

// ── Smithay delegate: text-input-v3 + input-method-v2 ────────────────────────
//
// Qt's `text-input-v3` plugin is what backs every `QML.TextInput` field on
// Wayland. It probes for both `wp_text_input_v3` and `zwp_input_method_v2`
// globals at activate-time; if either one is missing, Qt falls back to a
// degraded path where keystrokes are NOT routed to the focused TextInput
// even though `wl_keyboard.key` is being delivered to the surface. The
// most visible symptom: noctalia's lock screen receives wl_keyboard.enter
// just fine, the cursor blinks, MouseArea forces focus — and yet the
// password field stays empty no matter what you type.
//
// Smithay handles all the protocol plumbing as long as the globals are
// registered. We do NOT drive an IME ourselves (no fcitx/ibus integration
// here), so the handler is intentionally minimal: input-method popups
// just get tracked through the regular xdg popup manager so they render
// at the right location, and dismissal hooks back into PopupManager.


// ── Smithay delegate: pointer constraints + relative pointer ─────────────────
//
// `wp_pointer_constraints_v1` lets clients lock or confine the cursor to
// their surface. Two flavours:
//   * Lock: the pointer's *position* on screen freezes at request time;
//     the client still receives relative_motion events, but nothing else.
//     This is the FPS / Blender / DCC-app pattern — the user moves the
//     mouse, the camera turns, the cursor itself doesn't visibly drift.
//   * Confine: the pointer is allowed to move freely, but only inside
//     the surface (and an optional sub-region). Used by Krita to keep
//     the brush from leaving the canvas during a drag, and by remote-
//     desktop clients to keep the host pointer trapped inside the
//     remote view.
//
// `wp_relative_pointer_manager_v1` is the natural complement: it lets
// clients listen for pure delta-only motion events, so a locked pointer
// still reports "the user moved the mouse by Δ" without leaking an
// absolute position. Our `handle_pointer_motion` already calls
// `pointer.relative_motion(...)` on every libinput delta, so once the
// global is registered all clients can bind a `wp_relative_pointer_v1`
// per pointer and get the full event stream.
//
// Constraint *enforcement* (lock the cursor, clamp to region) lives in
// `input_handler::handle_pointer_motion`; this module only wires the
// protocol surface.

// ── Smithay delegate: xdg-activation-v1 ──────────────────────────────────────
//
// xdg-activation is the polite focus-stealing channel. Use cases:
//   * Notification daemon "Reply" / "Open" action buttons asking the
//     compositor to activate the conversation thread in the messenger
//     app the notification came from.
//   * `notify-send -A` style scripts that want the user to come back
//     to a long-running task after the OK click.
//   * `xdg-desktop-portal-wlr`'s `Activate` request, used by Discord
//     screen-share, Telegram desktop, etc., to bring themselves to
//     the front when the user clicks a system-tray icon.
//   * Browser → mailto: → Thunderbird already running → activate.
//
// Anti-focus-steal: spec recommends rejecting any token whose creating
// client wasn't the most recently keyboard-focused one. We follow
// anvil's reading: the token is valid only if its bundled serial is
// no older than our seat keyboard's last `enter` event, AND the seat
// in the token matches our seat. Without this, anything that knows
// the protocol could steal focus by spinning up a token at any time.
//
// On accept we route through the same focus path the user's bindings
// use: switch to the target window's tag, restore that monitor, focus
// the window. That keeps activation-driven jumps consistent with
// alt+tab / explicit `mctl dispatch view N`.


delegate_presentation!(MargoState);

