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
mod data;
mod debug_dump;
mod dispatch;
mod dpms;
mod focus_methods;
mod focus_target;
mod frame_clock_sched;
mod groups;
pub(crate) use groups::GroupLock;
pub mod mru_switcher;
mod overview;
mod scratchpad;
pub(crate) use scratchpad::MatchOp;
mod screencast;
mod scroller_overview;
mod state_file;
mod theme;
mod twilight_methods;
mod window_rules;

pub use self::animation_tick::{AnimTickSpec, tick_animations};
pub use self::data::{
    ClosingClient, FullscreenMode, HotCorner, LayerSurfaceAnim, MargoClient, MargoMonitor,
    ResizeSnapshot,
};
pub(crate) use self::data::{
    WindowRuleReason, clamp_size, matches_layer_name, matches_rule_text, read_toplevel_identity,
    read_toplevel_identity_if_changed,
};
pub use self::focus_target::FocusTarget;
pub use self::scroller_overview::{ScrollerOverview, overview_cells};
pub(crate) use self::theme::ThemeBaseline;

use std::{cell::RefCell, path::PathBuf, rc::Rc};

use anyhow::{Context, Result};
use smithay::{
    backend::allocator::dmabuf::Dmabuf,
    desktop::{PopupManager, Space, Window, WindowSurface, layer_map_for_output},
    input::{Seat, SeatHandler, SeatState, pointer::CursorImageStatus},
    output::Output,
    reexports::{
        calloop::{LoopHandle, LoopSignal, ping::Ping},
        wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as XdgDecorationMode,
        wayland_server::{
            Display, DisplayHandle, Resource,
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
        },
    },
    utils::{Clock, Monotonic, SERIAL_COUNTER, Size},
    wayland::{
        compositor::{CompositorClientState, CompositorState, with_states},
        dmabuf::{DmabufGlobal, DmabufState},
        drm_syncobj::DrmSyncobjState,
        input_method::InputMethodManagerState,
        output::{OutputHandler, OutputManagerState},
        pointer_constraints::PointerConstraintsState,
        presentation::PresentationState,
        relative_pointer::RelativePointerManagerState,
        seat::WaylandFocus,
        selection::{
            data_device::{DataDeviceState, set_data_device_focus},
            ext_data_control::DataControlState as ExtDataControlState,
            primary_selection::{PrimarySelectionState, set_primary_focus},
            wlr_data_control::DataControlState,
        },
        shell::{
            wlr_layer::WlrLayerShellState,
            xdg::{
                ToplevelSurface, XdgShellState, XdgToplevelSurfaceData,
                decoration::XdgDecorationState,
            },
        },
        shm::{ShmHandler, ShmState},
        text_input::TextInputManagerState,
        viewporter::ViewporterState,
        xdg_activation::XdgActivationState,
        xwayland_shell::XWaylandShellState,
    },
    xwayland::{X11Surface, X11Wm},
};

use margo_config::{Config, WindowRule, parse_config_with_defaults};

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
    pub output_management_state: crate::protocols::output_management::OutputManagementManagerState,
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
    pub cast_gbm:
        Option<smithay::backend::allocator::gbm::GbmDevice<smithay::backend::drm::DrmDeviceFd>>,
    /// Renderer-side dmabuf format constraints, snapshotted at
    /// backend init so the screencast cast lifecycle has them
    /// without crossing the borrow boundary into the udev
    /// renderer mid-D-Bus-call.
    pub cast_render_formats: smithay::backend::allocator::format::FormatSet,
    /// `ext-image-capture-source-v1` core state. Mints opaque
    /// source handles that clients pass to ext-image-copy-capture
    /// to identify what they want to capture. xdp-wlr 0.8+ uses
    /// these for the per-window screencast path.
    pub image_capture_source_state: smithay::wayland::image_capture_source::ImageCaptureSourceState,
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
    pub image_copy_capture_state: smithay::wayland::image_copy_capture::ImageCopyCaptureState,
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
    /// `zwp_keyboard_shortcuts_inhibit_manager_v1`: lets a client
    /// (vncviewer, RDP, VirtualBox, browser remote-desktop apps)
    /// request that margo stop matching its own keybindings while the
    /// client's surface has keyboard focus. The protocol state owns
    /// the global; the live inhibitors are tracked separately so the
    /// input filter can do an O(1) lookup by focused wl_surface.
    pub keyboard_shortcuts_inhibit_state:
        smithay::wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitState,
    /// `zwp_pointer_gestures_v1`: forwards touchpad pinch / swipe /
    /// hold gestures to clients (Firefox pinch-zoom, GNOME apps,
    /// Inkscape). Smithay drives the global from `SeatHandler`; state
    /// is held only so the delegate macro has a home.
    pub pointer_gestures_state: smithay::wayland::pointer_gestures::PointerGesturesState,
    /// `wp_single_pixel_buffer_v1`: lets clients allocate solid-color
    /// buffers without a real shm/dmabuf allocation. Pure smithay
    /// state — no handler trait, no policy.
    pub single_pixel_buffer_state: smithay::wayland::single_pixel_buffer::SinglePixelBufferState,
    /// `xdg_foreign_v2`: cross-process surface embedding — Firefox /
    /// Chromium Picture-in-Picture, xdg-desktop-portal screencast
    /// window targeting. A client exports a surface, sends the handle
    /// to another client, which imports it as a parent.
    pub xdg_foreign_state: smithay::wayland::xdg_foreign::XdgForeignState,
    /// `zwp_tablet_manager_v2`: Wacom / Huion drawing tablets — pen
    /// pressure, tilt, eraser, buttons. Without this, tablets fall
    /// back to mouse-emulation only.
    pub tablet_manager_state: smithay::wayland::tablet_manager::TabletManagerState,
    /// `wp_security_context_v1`: lets Flatpak / sandboxed clients
    /// connect through a separate socket the sandbox engine
    /// pre-allocates. The handler inserts the listener source into
    /// margo's calloop so sandboxed apps can talk to the compositor.
    pub security_context_state: smithay::wayland::security_context::SecurityContextState,
    /// `org_kde_kwin_server_decoration`: legacy KDE decoration
    /// protocol — some older Qt5 / KDE apps still expect it.
    /// xdg-decoration covers most modern apps.
    pub kde_decoration_state: smithay::wayland::shell::kde::decoration::KdeDecorationState,
    /// `wp_content_type_v1`: clients tag their surfaces as video /
    /// game / photo so the compositor can adjust scheduling
    /// (disable presentation-time hints for games, allow tearing,
    /// etc.).
    pub content_type_state: smithay::wayland::content_type::ContentTypeState,
    /// `wp_fifo_v1`: newer presentation-pacing protocol — clients
    /// can request FIFO commit ordering.
    pub fifo_manager_state: smithay::wayland::fifo::FifoManagerState,
    /// `wp_commit_timing_v1`: companion to fifo — explicit
    /// commit-time targets per surface.
    pub commit_timing_manager_state: smithay::wayland::commit_timing::CommitTimingManagerState,
    /// `wp_alpha_modifier_v1`: per-surface alpha hint so apps can
    /// fade themselves without going through compositor effects.
    pub alpha_modifier_state: smithay::wayland::alpha_modifier::AlphaModifierState,
    /// `xdg_wm_dialog_v1`: modal-dialog hint — compositor can place
    /// / decorate dialogs differently from regular toplevels.
    pub xdg_dialog_state: smithay::wayland::shell::xdg::dialog::XdgDialogState,
    /// `zwp_xwayland_keyboard_grab_v1`: lets XWayland clients ask
    /// the compositor to install an exclusive keyboard grab on
    /// their behalf. Companion to the v0.5.0 X11 focus fix and the
    /// new `zwp_keyboard_shortcuts_inhibit_v1` global — same VNC /
    /// VM / remote-desktop story, X11-side mechanism.
    pub xwayland_keyboard_grab_state:
        smithay::wayland::xwayland_keyboard_grab::XWaylandKeyboardGrabState,
    /// `xdg_toplevel_icon_v1`: toplevels can ship their own icon
    /// inline (PNG / SVG buffer) instead of the bar inferring it
    /// from the desktop file. mshell's active-window pill can
    /// surface this once a UI consumer is wired up.
    pub xdg_toplevel_icon_state: smithay::wayland::xdg_toplevel_icon::XdgToplevelIconManager,
    /// `xdg_system_bell_v1`: clients ring the system bell. We just
    /// advertise the global for now and log; routing to a sound
    /// daemon / mshell notification is a future enhancement.
    pub xdg_system_bell_state: smithay::wayland::xdg_system_bell::XdgSystemBellState,
    /// `wp_pointer_warp_v1`: clients can ask the compositor to
    /// move the cursor to a surface-local position. Default no-op
    /// — programmatic cursor movement is opt-in policy.
    pub pointer_warp_state: smithay::wayland::pointer_warp::PointerWarpManager,
    /// `xdg_toplevel_tag_v1`: clients attach semantic tags +
    /// description strings to toplevels. Default no-op; could feed
    /// into window-rule matching later.
    pub xdg_toplevel_tag_state: smithay::wayland::xdg_toplevel_tag::XdgToplevelTagManager,
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
    /// `state snapshot` write is dirty (a change happened since the last
    /// flush). Coalesces a burst of per-change writes into one serialize
    /// per event-loop iteration — see `mark_state_dirty` /
    /// `flush_state_file_if_dirty`.
    pub state_dirty: std::cell::Cell<bool>,
    /// Active IPC socket connections, keyed by a monotonic token.
    pub ipc_conns: std::collections::HashMap<u32, crate::ipc::server::IpcConn>,
    /// Next IPC connection token.
    pub ipc_next_token: u32,
    /// `watch`-mode IPC subscriptions, fanned out on each dirty flush.
    pub ipc_watches: crate::ipc::watch::WatchRegistry,
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
    config_path: Option<PathBuf>,

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
    /// zero damage. Entries for closed surfaces just linger (an `Id` is a
    /// cheap handle); not worth pruning.
    pub decoration_ids: std::cell::RefCell<
        std::collections::HashMap<
            smithay::reexports::wayland_server::backend::ObjectId,
            (
                smithay::backend::renderer::element::Id,
                smithay::backend::renderer::element::Id,
            ),
        >,
    >,

    pub foreign_toplevel_list: ForeignToplevelListState,
    pub wlr_foreign_toplevel: crate::protocols::wlr_foreign_toplevel::WlrForeignToplevelState,
    pub ext_workspace_state: crate::protocols::ext_workspace::ExtWorkspaceManagerState,
    pub virtual_pointer_state: crate::protocols::virtual_pointer::VirtualPointerManagerState,
    pub layer_surfaces: Vec<LayerSurface>,
    pub lock_surfaces: Vec<(Output, smithay::wayland::session_lock::LockSurface)>,

    pub session_locked: bool,
    /// Human-readable name of the keyboard's currently active xkb
    /// layout (e.g. "English (US)", "Turkish"). Cached here and
    /// published in state snapshot so the shell's keyboard-layout pill
    /// can show it without re-deriving the keymap. Updated on every
    /// key event that changes the effective layout group.
    pub current_kb_layout: String,
    pub enable_gaps: bool,
    pub cursor_status: CursorImageStatus,
    pub cursor_manager: CursorManager,
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
    pub xwm: Option<X11Wm>,
    /// Latest position an override-redirect X11 surface (menu / popup /
    /// tooltip) was configured to, keyed by `window_id`. Toolkits move the
    /// popup to its anchor via a ConfigureNotify that can arrive BEFORE the
    /// map event, at which point the surface isn't in the space yet and
    /// `X11Surface::geometry()` still reads its stale (0,0) creation rect —
    /// so we stash the configured location here and consume it when the
    /// surface is finally mapped. Cleared on unmap/destroy.
    pub or_positions: std::collections::HashMap<u32, (i32, i32)>,
    pub xwayland_shell_state: XWaylandShellState,
    pub libinput: Option<smithay::reexports::input::Libinput>,
    pub gamma_control_manager_state: crate::protocols::gamma_control::GammaControlManagerState,
    pub output_power_manager_state: crate::protocols::output_power::OutputPowerManagerState,
    /// Pending gamma ramp updates drained by the udev backend each frame.
    /// Tuple is (output, ramp). `None` ramp = restore default. The udev
    /// backend pops these and applies them via DRM `GAMMA_LUT`. Winit just
    /// drops them silently.
    pub pending_gamma: Vec<(Output, Option<Vec<u16>>)>,
    /// Pending DPMS power changes drained by the udev backend each frame.
    /// `(output, on)` — `on = false` calls `DrmCompositor::clear()` (panel
    /// off); `true` re-renders the output (panel on). Winit drops them.
    pub pending_dpms: Vec<(Output, bool)>,
    /// `true` while ANY output is DPMS-off. Lets `handle_input` cheaply gate
    /// the "any input wakes the screen" safety net without touching the
    /// backend, and lets a `dpms toggle` decide direction.
    pub any_dpms_off: bool,
    /// When the last DPMS-off was requested. `wake_dpms_on_input` ignores
    /// input within a short grace after this so the very keystroke / click
    /// that triggered the off (its release event, the Enter that ran `mctl`,
    /// residual pointer motion) can't immediately wake the panel back up.
    pub dpms_off_at: Option<std::time::Instant>,
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
    /// Token of the single self-re-arming twilight tick timer. A
    /// force-tick (from `mctl twilight …` / `reload_config`) removes
    /// this one and re-inserts at the near-term interval instead of
    /// stacking a fresh permanent ticker per dispatch — the old
    /// `bool` guard was never consulted, so every toggle leaked a
    /// timer that kept waking the loop forever.
    pub twilight_timer_token: Option<smithay::reexports::calloop::RegistrationToken>,
    /// Whether a non-identity twilight ramp is currently applied. Starts
    /// `true` so the first disabled tick clears once (guaranteeing
    /// identity at startup); thereafter `clear_twilight_ramp` no-ops
    /// while already cleared, so a disabled-twilight session stops
    /// re-pushing gamma + repainting every 60 s.
    pub twilight_ramp_active: bool,
    /// Cached `(schedule_dir, ScheduleData)` for Schedule mode. The schedule
    /// presets are static between explicit changes, but `tick_twilight` runs
    /// as often as every 50 ms during a sunrise/sunset sweep — re-reading +
    /// parsing the seven preset files on every one of those ticks was pure
    /// waste. We load once and serve from here; the cache is cleared on
    /// config reload and on every `mctl twilight` command (both invalidation
    /// points users actually change presets through).
    pub twilight_schedule_cache: Option<(String, crate::twilight::preset::ScheduleData)>,

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
        let ext_data_control_state =
            ExtDataControlState::new::<Self, _>(&dh, Some(&primary_selection_state), |_| true);
        let cursor_shape_manager_state =
            smithay::wayland::cursor_shape::CursorShapeManagerState::new::<Self>(&dh);
        let fractional_scale_manager_state =
            smithay::wayland::fractional_scale::FractionalScaleManagerState::new::<Self>(&dh);
        let layer_shell_state = WlrLayerShellState::new::<Self>(&dh);
        let session_lock_state =
            smithay::wayland::session_lock::SessionLockManagerState::new::<Self, _>(&dh, |_| true);
        let text_input_state = TextInputManagerState::new::<Self>(&dh);
        let input_method_state = InputMethodManagerState::new::<Self, _>(&dh, |_client| true);
        let pointer_constraints_state = PointerConstraintsState::new::<Self>(&dh);
        let relative_pointer_state = RelativePointerManagerState::new::<Self>(&dh);
        let xdg_activation_state = XdgActivationState::new::<Self>(&dh);
        let output_management_state =
            crate::protocols::output_management::OutputManagementManagerState::new::<Self, _>(
                &dh,
                |_client| true,
            );
        // wp_color_management_v1 (staging) — Phase 1 scaffolding.
        // Standing the global up early lets HDR-aware clients
        // (Chromium, mpv) detect "this compositor speaks colour
        // management" and enable their decode paths even though
        // composite is still SDR. See `protocols/color_management.rs`
        // and `docs/hdr-design.md` for the four-phase rollout.
        let color_management_state = crate::protocols::color_management::ColorManagementState::new::<
            Self,
            _,
        >(&dh, |_client| true);
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
        // `zwp_keyboard_shortcuts_inhibit_v1` — required by vncviewer,
        // RDP clients, VirtualBox, and any app that needs the host
        // compositor to stop intercepting its own keybindings (Super,
        // Alt+Tab, …) while the client surface has focus.
        let keyboard_shortcuts_inhibit_state =
            smithay::wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitState::new::<Self>(
                &dh,
            );
        // `zwp_pointer_gestures_v1` — touchpad gestures (pinch /
        // swipe / hold) forwarded to focused client.
        let pointer_gestures_state =
            smithay::wayland::pointer_gestures::PointerGesturesState::new::<Self>(&dh);
        // `wp_single_pixel_buffer_v1` — solid-color buffer fast-path.
        let single_pixel_buffer_state =
            smithay::wayland::single_pixel_buffer::SinglePixelBufferState::new::<Self>(&dh);
        // `xdg_foreign_v2` — cross-process surface embedding.
        let xdg_foreign_state = smithay::wayland::xdg_foreign::XdgForeignState::new::<Self>(&dh);
        // `zwp_tablet_manager_v2` — pen tablets.
        let tablet_manager_state =
            smithay::wayland::tablet_manager::TabletManagerState::new::<Self>(&dh);
        // `wp_security_context_v1` — sandboxed-client socket
        // pre-allocation. Filter accepts every client; the protocol's
        // listener-fd mechanism is itself the access boundary.
        let security_context_state = smithay::wayland::security_context::SecurityContextState::new::<
            Self,
            _,
        >(&dh, |_client| true);
        // `org_kde_kwin_server_decoration` — legacy KDE deco.
        // Default to server-side decoration since margo's existing
        // xdg-decoration policy is SSD-first.
        let kde_decoration_state =
            smithay::wayland::shell::kde::decoration::KdeDecorationState::new::<Self>(
                &dh,
                smithay::reexports::wayland_protocols_misc::server_decoration::server::org_kde_kwin_server_decoration_manager::Mode::Server,
            );
        // `wp_content_type_v1` — surface content-type hints.
        let content_type_state = smithay::wayland::content_type::ContentTypeState::new::<Self>(&dh);
        // `wp_fifo_v1` + `wp_commit_timing_v1` — newer presentation
        // pacing protocols.
        let fifo_manager_state = smithay::wayland::fifo::FifoManagerState::new::<Self>(&dh);
        let commit_timing_manager_state =
            smithay::wayland::commit_timing::CommitTimingManagerState::new::<Self>(&dh);
        // `wp_alpha_modifier_v1` — per-surface alpha hint.
        let alpha_modifier_state =
            smithay::wayland::alpha_modifier::AlphaModifierState::new::<Self>(&dh);
        // `xdg_wm_dialog_v1` — modal-dialog hint.
        let xdg_dialog_state =
            smithay::wayland::shell::xdg::dialog::XdgDialogState::new::<Self>(&dh);
        // `zwp_xwayland_keyboard_grab_v1` — XWayland-side keyboard grab.
        let xwayland_keyboard_grab_state =
            smithay::wayland::xwayland_keyboard_grab::XWaylandKeyboardGrabState::new::<Self>(&dh);
        // `xdg_toplevel_icon_v1` — inline app icons on toplevels.
        let xdg_toplevel_icon_state =
            smithay::wayland::xdg_toplevel_icon::XdgToplevelIconManager::new::<Self>(&dh);
        // `xdg_system_bell_v1` — system bell.
        let xdg_system_bell_state =
            smithay::wayland::xdg_system_bell::XdgSystemBellState::new::<Self>(&dh);
        // `wp_pointer_warp_v1` — programmatic pointer position requests.
        let pointer_warp_state =
            smithay::wayland::pointer_warp::PointerWarpManager::new::<Self>(&dh);
        // `xdg_toplevel_tag_v1` — semantic toplevel tags.
        let xdg_toplevel_tag_state =
            smithay::wayland::xdg_toplevel_tag::XdgToplevelTagManager::new::<Self>(&dh);
        let space = Space::default();
        let popups = PopupManager::default();
        let animation_curves = AnimationCurves::bake(&config);
        let input_keyboard = KeyboardState::new(&config);

        let xwayland_shell_state = XWaylandShellState::new::<Self>(&dh);
        let foreign_toplevel_list = ForeignToplevelListState::new::<Self>(&dh);
        let wlr_foreign_toplevel =
            crate::protocols::wlr_foreign_toplevel::WlrForeignToplevelState::new::<Self, _>(
                &dh,
                |_client| true,
            );
        let ext_workspace_state = crate::protocols::ext_workspace::ExtWorkspaceManagerState::new::<
            Self,
            _,
        >(&dh, |_client| true);
        let virtual_pointer_state =
            crate::protocols::virtual_pointer::VirtualPointerManagerState::new::<Self, _>(
                &dh,
                |_client| true,
            );

        // wlr_gamma_control_v1 — sunsetr / gammastep / wlsunset use this to
        // push night-light ramps to outputs. Allow all clients (no privileged
        // filter) so user services can drive it freely.
        let gamma_control_manager_state =
            crate::protocols::gamma_control::GammaControlManagerState::new::<Self, _>(
                &dh,
                |_client| true,
            );

        // wlr-output-power-management-v1: lets idle daemons (`swayidle`) and
        // `wlr-randr` power outputs off/on via DPMS. All clients allowed.
        let output_power_manager_state =
            crate::protocols::output_power::OutputPowerManagerState::new::<Self, _>(
                &dh,
                |_client| true,
            );

        // wlr-screencopy-unstable-v1: lets `grim`, `wf-recorder`, `screen rec`
        // etc. capture compositor outputs.
        let screencopy_state =
            crate::protocols::screencopy::ScreencopyManagerState::new::<Self, _>(&dh, |_client| {
                true
            });

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
            state_dirty: std::cell::Cell::new(false),
            ipc_conns: std::collections::HashMap::new(),
            ipc_next_token: 0,
            ipc_watches: crate::ipc::watch::WatchRegistry::default(),
            tag_carousel_dir: 0,
            edge_scroller_armed: true,
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
            active_output_source: ActiveOutputSource::default(),
            decoration_ids: std::cell::RefCell::new(std::collections::HashMap::new()),
            foreign_toplevel_list,
            wlr_foreign_toplevel,
            ext_workspace_state,
            virtual_pointer_state,
            layer_surfaces: vec![],
            lock_surfaces: vec![],
            clients: vec![],
            monitors: vec![],
            next_group_id: 1,
            groups_locked: false,
            session_locked: false,
            current_kb_layout: String::new(),
            idle_notifier_state,
            idle_inhibit_state,
            virtual_keyboard_manager_state,
            keyboard_shortcuts_inhibit_state,
            keyboard_shortcuts_inhibiting_surfaces: std::collections::HashMap::new(),
            pointer_gestures_state,
            single_pixel_buffer_state,
            xdg_foreign_state,
            tablet_manager_state,
            security_context_state,
            kde_decoration_state,
            content_type_state,
            fifo_manager_state,
            commit_timing_manager_state,
            alpha_modifier_state,
            xdg_dialog_state,
            xwayland_keyboard_grab_state,
            xdg_toplevel_icon_state,
            xdg_system_bell_state,
            pointer_warp_state,
            xdg_toplevel_tag_state,
            idle_inhibitors: std::collections::HashSet::new(),
            layer_layout_hashes: std::collections::HashMap::new(),
            layer_kb_interactivity_hashes: std::collections::HashMap::new(),
            frame_callback_sequence: std::collections::HashMap::new(),
            estimated_vblank_timers: std::collections::HashMap::new(),
            per_output_clocks: std::collections::HashMap::new(),
            cursor_shape_manager_state,
            fractional_scale_manager_state,
            enable_gaps: config.enable_gaps,
            cursor_status: CursorImageStatus::default_named(),
            cursor_manager: CursorManager::new(),
            wallpaper: crate::wallpaper::WallpaperState::load(config.wallpaper.as_deref()),
            overview_backdrop: config
                .overview_backdrop_image
                .as_deref()
                .and_then(crate::wallpaper::WallpaperState::load_exact),
            xwm: None,
            or_positions: std::collections::HashMap::new(),
            xwayland_shell_state,
            libinput: None,
            gamma_control_manager_state,
            output_power_manager_state,
            pending_gamma: Vec::new(),
            pending_dpms: Vec::new(),
            any_dpms_off: false,
            dpms_off_at: None,
            screencopy_state,
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
            hot_corner_dwelling: None,
            twilight: crate::twilight::TwilightState::default(),
            twilight_timer_token: None,
            twilight_ramp_active: true,
            twilight_schedule_cache: None,
            hotplug_last_event_at: None,
            hotplug_rescan_pending: false,
            hot_corner_armed_at: None,
            title_rules_exist: config_has_title_rules(&config),
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
        self.output_power_manager_state.output_removed(output);
        self.screencopy_state.remove_output(output);

        if let Some(pos) = self.monitors.iter().position(|m| m.output == *output) {
            tracing::info!(monitor = %self.monitors[pos].name, "removing monitor");
            self.monitors.remove(pos);
        }
        self.space.unmap_output(output);
        self.lock_surfaces.retain(|(o, _)| o != output);
        self.pending_gamma.retain(|(o, _)| o != output);
        // Per-output frame clock: drop this output's clock + cancel its
        // present timer so a gone display doesn't keep waking the loop
        // (no-op when the opt-in flag is off / no clock exists).
        self.drop_output_clock(&output.name());
        // Hotplug-out: refresh the shared D-Bus snapshot so
        // xdp-gnome's chooser dialog drops the now-gone output.
        self.refresh_ipc_outputs();
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

    /// Start an interactive move grab on the currently focused window.
    /// Triggered by the `moveresize,curmove` action (typically a super+
    /// left-drag mousebind). No-op if there's no focused client or no
    /// pointer button is currently pressed.
    pub fn start_interactive_move(&mut self) {
        let Some(idx) = self.focused_client_idx() else {
            return;
        };
        let window = self.clients[idx].window.clone();
        let initial_loc = smithay::utils::Point::<i32, smithay::utils::Logical>::from((
            self.clients[idx].geom.x,
            self.clients[idx].geom.y,
        ));
        // Remember the pre-grab tiled state so the grab's drop
        // handler can decide between "swap with target tile" and
        // "restore to original float geometry".
        let was_tiled = !self.clients[idx].is_floating;
        let original_float_geom = self.clients[idx].float_geom;
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
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
            was_tiled,
            original_float_geom,
        };
        pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
    }

    /// Start an interactive resize grab on the focused window. Edge
    /// defaults to bottom-right (the natural drag-corner gesture). If
    /// you want a specific edge, pass it in the action arg later.
    pub fn start_interactive_resize(&mut self) {
        let Some(idx) = self.focused_client_idx() else {
            return;
        };
        let c = &self.clients[idx];
        let window = c.window.clone();
        let initial_loc =
            smithay::utils::Point::<i32, smithay::utils::Logical>::from((c.geom.x, c.geom.y));
        let initial_size = smithay::utils::Size::<i32, smithay::utils::Logical>::from((
            c.geom.width.max(1),
            c.geom.height.max(1),
        ));
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
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
        // Opt-in per-output frame clock: a global repaint can affect
        // any output, so flag every output's clock dirty and ensure its
        // present timer is armed. The per-output timers gate *when* each
        // output actually renders (at its own refresh), and the repaint
        // ping below still wakes the loop so a due output renders this
        // turn. With the flag off this is a no-op and the original
        // global path runs unchanged.
        if self.per_output_frame_clock_enabled() {
            self.mark_all_clocks_dirty();
            if let Some(ping) = &self.repaint_ping {
                ping.ping();
            }
            return;
        }
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

    /// Clone of the repaint-ping handle, for the per-output frame clock
    /// to wake the redraw scheduler from a present-timer callback
    /// without going through `request_repaint` (which would re-dirty
    /// every output's clock and re-arm every timer — we only want to
    /// nudge the loop so it renders whichever outputs are already due).
    pub(crate) fn repaint_ping_handle(&self) -> Option<Ping> {
        self.repaint_ping.clone()
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
                    self.config_error_overlay_until =
                        Some(std::time::Instant::now() + std::time::Duration::from_secs(10));
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

        let new_config = parse_config_with_defaults(self.config_path.as_deref())
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
        // Re-decode the overview backdrop image if the configured path
        // changed (incl. cleared → `None`), so Settings → Overview
        // applies live. Only re-reads the file when the path actually
        // differs — avoids a needless decode on every `mctl reload`.
        if new_config.overview_backdrop_image != self.config.overview_backdrop_image {
            self.overview_backdrop = new_config
                .overview_backdrop_image
                .as_deref()
                .and_then(crate::wallpaper::WallpaperState::load_exact);
        }
        // `client.border_width` is seeded from `config.borderpx` at map time
        // (see `state/data.rs`), so a bare config swap would leave every
        // already-open window on the OLD border width — `borderpx` would only
        // affect windows opened after the reload. Re-apply the new global to
        // every client that was tracking it (i.e. whose width still equals the
        // old global), leaving per-window `windowrule` border overrides alone.
        let old_borderpx = self.config.borderpx;
        let new_borderpx = new_config.borderpx;
        self.config = new_config;
        self.title_rules_exist = config_has_title_rules(&self.config);

        // Re-apply the file-logging knobs live so `mctl config reload`
        // (and Settings → Logging via the compositor `.conf`) takes effect
        // without a restart. `keep_sessions` only matters at the next start.
        if let Some(h) = crate::LOG_HANDLE.get() {
            let _ = h.set_enabled(self.config.log_to_file);
            let _ = h.set_level(&self.config.log_file_level);
        }

        if new_borderpx != old_borderpx {
            for client in self.clients.iter_mut() {
                if client.border_width == old_borderpx {
                    client.border_width = new_borderpx;
                }
            }
        }
        // Per-tag tiling layouts: re-assert the configured `taglayout`
        // directives on reload so Settings → Tiling Layout "Apply"
        // (which writes the config + runs `mctl reload`) takes effect
        // live. `seed_taglayouts` only touches tags that have an
        // explicit directive, so unconfigured "(default)" tags keep
        // whatever the running session set. `current_layout()` reads
        // `ltidxs[curtag]` directly, so the `arrange_all()` below
        // re-tiles the visible tag immediately.
        let taglayouts = self.config.taglayouts.clone();
        if !taglayouts.is_empty() {
            for mon in &mut self.monitors {
                mon.pertag.seed_taglayouts(&taglayouts);
            }
        }
        // Apply the global `default_layout` to every tag that has neither an
        // explicit `taglayout` override nor a live user-picked layout, so
        // changing "Default layout" in Settings → Tiling Layout takes effect
        // on reload (not only at the next start). user_picked tags (a manual
        // `setlayout`) are left alone — the session keeps them. Runs before
        // `apply_tag_rules_to_monitor`, so a per-tag `tagrule` layout still
        // wins over the global default (precedence: taglayout > tagrule >
        // default).
        if let Some(def) = LayoutId::from_name(&self.config.default_layout) {
            let override_tags: std::collections::HashSet<usize> =
                taglayouts.iter().map(|(t, _)| *t as usize).collect();
            for mon in &mut self.monitors {
                for tag in 1..mon.pertag.ltidxs.len() {
                    let picked = mon
                        .pertag
                        .user_picked_layout
                        .get(tag)
                        .copied()
                        .unwrap_or(false);
                    if !override_tags.contains(&tag) && !picked {
                        mon.pertag.ltidxs[tag] = def;
                    }
                }
            }
        }
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
        self.mark_state_dirty();
        self.request_repaint();
        // Config swap may have flipped twilight on/off or changed
        // day/night temps — force a resample so the new values
        // take effect immediately instead of waiting for the next
        // tick. Drop the schedule cache first so edited presets /
        // a changed schedule dir are re-read on this resample.
        self.twilight_schedule_cache = None;
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
                // MRU recency key for the Super+Tab switcher — but NOT while the
                // switcher is cycling (preview-focus must not rewrite the MRU
                // order; only the committed pick does, in `mru_confirm`).
                if self.mru_switcher.is_none() {
                    self.focus_counter += 1;
                    self.clients[idx].last_focus_serial = self.focus_counter;
                }
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
                    let initial_color = self.clients[idx].opacity_animation.current_border_color;
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
                    let initial_color = self.clients[idx].opacity_animation.current_border_color;
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

        // Mark dirty so IPC watch subscribers see the new focus (
        // waybar-dwl, …). The struct gets its title / appid /
        // fullscreen / floating fields from `focused_client_idx`,
        // which we just changed; without this the bar would keep
        // showing the previously-focused window's title until the
        // next tag-switch / arrange caused some other broadcast to
        // fire. mango broadcasts on every focus change too — this
        // is straight parity.
        if prev_focus_idx != new_focus_idx {
            self.mark_state_dirty();
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
        // Mirror the toplevel set to wlr-foreign-toplevel-management clients.
        // Diffing + idempotent, so running it per-output post-render is fine.
        crate::protocols::wlr_foreign_toplevel::refresh(self);
        // Mirror tag state to ext-workspace clients (same diffing contract).
        crate::protocols::ext_workspace::refresh(self);
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

        // Warm-up nudge: for `warmup_hidden_ms` after a window first maps, keep
        // delivering frame callbacks to it even while it sits on a hidden tag
        // (it isn't in `space.elements()` above). Frame-throttled clients —
        // Electron / CEF (Spotify, Webcord, Ferdium, …) — stall their renderer
        // when no frames arrive, so apps launched at login onto a background
        // tag never finish initialising until the tag is first visited. This
        // lets them warm up regardless. The sequence-based `should_send` dedup
        // keeps current-tag windows already served above from a double send.
        let warmup = self.config.warmup_hidden_ms;
        if warmup > 0
            && let Some(mon_idx) = self.monitors.iter().position(|m| &m.output == output)
        {
            let now = std::time::Instant::now();
            let window = std::time::Duration::from_millis(warmup as u64);
            let tagset = self.monitors[mon_idx].current_tagset();
            for c in &mut self.clients {
                let Some(mapped_at) = c.mapped_at else {
                    continue;
                };

                if now.saturating_duration_since(mapped_at) >= window {
                    c.mapped_at = None;
                    continue;
                }

                if c.monitor == mon_idx
                    && !c.is_initial_map_pending
                    && !c.is_visible_on(mon_idx, tagset)
                    && !c.is_minimized
                    && !c.is_killing
                    && !c.is_in_scratchpad
                {
                    c.window.send_frame(output, time, throttle, should_send);
                }
            }
        }

        // While the scroller overview is open it renders EVERY tag's
        // windows live — including ones unmapped from `space` (off-screen
        // tags). Those never appear in `space.elements()`, so without
        // this they get no frame callbacks and frame-throttled clients
        // (GTK, Electron, …) won't repaint to the slot size
        // `prearrange_overview_tags` configured for them — leaving them
        // stale-sized in the overview until their tag is visited once.
        // Nudge every overview-shown window on this output's monitor; the
        // sequence-based `should_send` dedup keeps the current-tag windows
        // already served above from being sent twice.
        if self.is_scroller_overview_open()
            && let Some(mon_idx) = self.monitors.iter().position(|m| &m.output == output)
        {
            for c in &self.clients {
                if c.monitor == mon_idx
                    && !c.is_initial_map_pending
                    && !c.is_minimized
                    && !c.is_killing
                    && !c.is_in_scratchpad
                {
                    c.window.send_frame(output, time, throttle, should_send);
                }
            }
        }

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
            RegistrationToken,
            timer::{TimeoutAction, Timer},
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

    /// Single post-mount window-rule reapply path. All three trigger
    /// sites — initial XDG mount, late `app_id` settle, config reload —
    /// route through this with a [`WindowRuleReason`] tag so the debug
    /// log says *why* a rule fired.
    pub(crate) fn reapply_rules(&mut self, idx: usize, reason: WindowRuleReason) -> bool {
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

    pub(crate) fn matching_window_rules(&self, app_id: &str, title: &str) -> Vec<WindowRule> {
        self.config
            .window_rules
            .iter()
            .filter(|rule| self.window_rule_matches(rule, app_id, title))
            .cloned()
            .collect()
    }

    pub(crate) fn window_rule_matches(&self, rule: &WindowRule, app_id: &str, title: &str) -> bool {
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
}

// ── Smithay delegate: XDG decoration ─────────────────────────────────────────

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
        if let Some(client) = self
            .clients
            .iter()
            .find(|c| c.window.wl_surface().as_deref() == Some(wl_surface))
        {
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
        // A named request (wp_cursor_shape_v1 / set_cursor → e.g. the `pointer`
        // hand over a link) only changes anything if we point the cursor
        // manager at that icon — the `Named` render path draws whatever it
        // currently holds.
        if let CursorImageStatus::Named(icon) = &image {
            self.cursor_manager.set_named(icon.name(), icon.alt_names());
        }
        self.cursor_status = image;
        self.request_repaint();
    }
}

// `wp_cursor_shape_v1`: clients send a shape name instead of their
// own cursor surface; we draw via `cursor_manager` at our own size.
// Required for GTK4 layer-shell surfaces to avoid oversized cursor.
// (TabletSeatHandler lives in state/handlers/tablet_manager.rs.)

// ── Smithay delegate: Output ──────────────────────────────────────────────────

impl OutputHandler for MargoState {}

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
        &mut self.foreign_toplevel_list
    }
}

// ── XwmHandler: X11 window management ────────────────────────────────────────

impl MargoState {
    /// Stable `(shadow_id, blur_id)` for a surface's self-drawn
    /// decorations, created once and reused across frames. See
    /// [`MargoState::decoration_ids`]. The pair is created lazily on first
    /// use; `Id` is a cheap clonable handle.
    pub fn decoration_element_ids(
        &self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) -> (
        smithay::backend::renderer::element::Id,
        smithay::backend::renderer::element::Id,
    ) {
        use smithay::backend::renderer::element::Id;
        self.decoration_ids
            .borrow_mut()
            .entry(surface.id())
            .or_insert_with(|| (Id::new(), Id::new()))
            .clone()
    }
}

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
        let tags = self
            .monitors
            .get(mon_idx)
            .map(|m| m.current_tagset())
            .unwrap_or(1);
        let mut client = MargoClient::new(window.clone(), mon_idx, tags, &self.config);
        client.surface_type = crate::SurfaceType::X11;
        client.title = window.x11_surface().map(|s| s.title()).unwrap_or_default();
        client.app_id = window.x11_surface().map(|s| s.class()).unwrap_or_default();
        // XWayland toplevels opt out of the tiling slide/move animation by
        // default. Animating an X11 window's slot — the tag-switch slide in
        // particular — drives some X11 clients (notably TigerVNC's vncviewer)
        // into a state where they keep rendering the remote framebuffer live
        // but silently stop forwarding pointer/keyboard input to the remote,
        // and only a real resize kicks them back out of it. `animations = 0`
        // avoids it globally; scoping the opt-out to X11 keeps native Wayland
        // animations intact. Set before `apply_window_rules` so an explicit
        // `no_animation` window-rule can still override it per app.
        client.no_animation = true;
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
        let ft_handle = self
            .foreign_toplevel_list
            .new_toplevel::<Self>(&client.title, &client.app_id);
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
            let group = self.group_of(idx);
            self.mru_remove_window(&window);
            self.space.unmap_elem(&window);
            self.clients.remove(idx);
            self.shift_indices_after_remove(idx);
            if let Some(gid) = group {
                self.repair_group(gid);
            }
            let mon_idx = self.focused_monitor();
            if !self.monitors.is_empty() {
                self.arrange_monitor(mon_idx);
            }
            // Refresh xdp-gnome's window picker — same path the
            // Wayland toplevel_destroyed handler uses.
            self.emit_windows_changed();
            crate::scripting::fire_window_close(self, &app_id, &title);
            return;
        }

        // Override-redirect window (menu / popup / tooltip). These are never
        // in `self.clients` — they live only in the space, mapped by
        // `mapped_override_redirect_window`. Without unmapping them here a
        // dismissed menu lingers in the space forever; the next menu from the
        // same client then opens on top of a stale element and the
        // `or_positions` handoff drifts (the classic "second open lands in the
        // wrong place" XWayland-menu symptom). Unmap it explicitly.
        let id = x11surface.window_id();
        let elem = self
            .space
            .elements()
            .find(
                |e| matches!(e.underlying_surface(), WindowSurface::X11(s) if s.window_id() == id),
            )
            .cloned();
        if let Some(elem) = elem {
            self.space.unmap_elem(&elem);
            self.request_repaint();
        }
    }
}

// ── Smithay delegate: Viewporter ───────────────────────────────────────────────

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
