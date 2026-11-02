//! Wayland protocol globals, grouped out of `MargoState`.
//!
//! Every field here is the Smithay (or margo `protocols::`) state object that
//! owns one advertised global. Most are held only so the global stays alive
//! and the `delegate_*!` macros have a home; the handler trait impls in
//! `state/handlers/` hand out references through `state.protocols.<name>`.
//! [`Protocols::new`] registers every global; the few that depend on the
//! backend (`dmabuf_global`, `drm_syncobj`, `drm_lease`) start as `None` and
//! are filled in by the udev backend once the DRM node is open.

use smithay::{
    reexports::calloop::LoopHandle,
    wayland::{
        compositor::CompositorState,
        dmabuf::{DmabufGlobal, DmabufState},
        drm_syncobj::DrmSyncobjState,
        input_method::InputMethodManagerState,
        output::OutputManagerState,
        pointer_constraints::PointerConstraintsState,
        presentation::PresentationState,
        relative_pointer::RelativePointerManagerState,
        selection::{
            data_device::DataDeviceState,
            ext_data_control::DataControlState as ExtDataControlState,
            primary_selection::PrimarySelectionState, wlr_data_control::DataControlState,
        },
        shell::{
            wlr_layer::WlrLayerShellState,
            xdg::{XdgShellState, decoration::XdgDecorationState},
        },
        shm::ShmState,
        text_input::TextInputManagerState,
        viewporter::ViewporterState,
        xdg_activation::XdgActivationState,
        xwayland_shell::XWaylandShellState,
    },
};

use margo_config::Config;

use super::{MargoState, client_is_unrestricted};
use crate::protocols::foreign_toplevel::ForeignToplevelListState;

/// Backing store for `disable_middle_paste`, read live by the
/// `wp_primary_selection` global's bind filter (set in [`Protocols::new`],
/// re-synced on every `mctl reload`). A plain static because the filter
/// closure Smithay stores alongside the global is `'static` and has no
/// path back to `MargoState` to read the live config through.
pub(super) static DISABLE_MIDDLE_PASTE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn middle_paste_disabled() -> bool {
    DISABLE_MIDDLE_PASTE.load(std::sync::atomic::Ordering::Relaxed)
}

pub struct Protocols {
    pub compositor: CompositorState,
    pub xdg_shell: XdgShellState,
    pub xdg_decoration: XdgDecorationState,
    pub shm: ShmState,
    pub viewporter: ViewporterState,
    pub dmabuf: DmabufState,
    pub dmabuf_global: Option<DmabufGlobal>,
    /// `wp_linux_drm_syncobj_v1` global state. `None` until the udev
    /// backend opens the primary DRM node and confirms it supports
    /// `syncobj_eventfd` — older kernels (< 5.18) and devices without
    /// `DRM_CAP_SYNCOBJ_TIMELINE` can't drive explicit-sync, so we
    /// don't expose the protocol there. Modern Chromium / Firefox
    /// prefers explicit sync when the global is advertised: per-
    /// surface `wp_linux_drm_syncobj_surface_v1` carries acquire +
    /// release fences alongside the dmabuf, eliminating the implicit
    /// fence wait that otherwise drops frames under GPU load.
    pub drm_syncobj: Option<DrmSyncobjState>,
    pub layer_shell: WlrLayerShellState,
    pub output_manager: OutputManagerState,
    pub data_device: DataDeviceState,
    pub primary_selection: PrimarySelectionState,
    pub data_control: DataControlState,
    /// `ext_data_control_v1` — standardized successor to
    /// `wlr_data_control_v1`. mshell-clipboard / wl-clipboard 3.x
    /// prefer this; without it the clipboard watcher dies at
    /// startup with "Missing ext_data_control_manager_v1 or wl_seat".
    pub ext_data_control: ExtDataControlState,
    pub session_lock: smithay::wayland::session_lock::SessionLockManagerState,
    /// `wp_text_input_v3` global. Qt clients (Quickshell/noctalia, KDE,
    /// QtWidgets apps) probe for this when a TextInput field becomes
    /// active — without it, Qt's QML password fields silently drop
    /// keystrokes on the lock screen even when wl_keyboard.enter is
    /// delivered. We don't drive an IME ourselves; smithay routes the
    /// protocol traffic correctly with just the global registered.
    pub text_input: TextInputManagerState,
    /// `zwp_input_method_v2` global. Goes hand-in-hand with text_input —
    /// Qt's text-input plugin won't activate without both.
    pub input_method: InputMethodManagerState,
    /// `zwp_pointer_constraints_v1` global. Lets clients request that
    /// the pointer be locked (held in place, FPS games / Blender's
    /// rotate-around-camera) or confined to a region (Krita canvas
    /// drag, remote-desktop client). Activated through
    /// `PointerConstraintsHandler::new_constraint`; honoured in
    /// `handle_pointer_motion`.
    pub pointer_constraints: PointerConstraintsState,
    /// `zwp_relative_pointer_manager_v1` global. Required complement
    /// to pointer constraints — when the pointer is locked, clients
    /// still need to know the cursor *would have moved by Δ*. Each
    /// pointer-motion event already calls `pointer.relative_motion`,
    /// so all this state needs to do is exist so clients can bind
    /// the global and get a `wp_relative_pointer_v1` per pointer.
    pub relative_pointer: RelativePointerManagerState,
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
    pub xdg_activation: XdgActivationState,
    /// `wlr_output_management_v1` state. Lets `kanshi`,
    /// `wlr-randr`, `way-displays` etc. discover the output
    /// topology and apply scale / transform / position changes
    /// at runtime. Disable still rejected; mode changes are now
    /// queued via `pending_output_mode_changes` for the udev
    /// backend to drain at the next repaint.
    pub output_management: crate::protocols::output_management::OutputManagementManagerState,
    /// `wp_color_management_v1` (staging) — Phase 1 scaffolding.
    /// The global is registered so Chromium / mpv probe-detection
    /// finds a colour-managed compositor and lights up their HDR
    /// decode paths. Composite stays sRGB; per-surface descriptions
    /// are stored on the surface tracker, not yet read by render.
    /// Phase 2 (linear-light fp16 composite) consumes these.
    pub color_management: crate::protocols::color_management::ColorManagementState,
    pub color_representation: crate::protocols::color_representation::ColorRepresentationState,
    /// `wp_drm_lease_device_v1` — registered by the udev backend once
    /// the primary DRM node is open; stays `None` under winit/headless.
    pub drm_lease: Option<smithay::wayland::drm_lease::DrmLeaseState>,
    /// `ext-image-capture-source-v1` core state. Mints opaque
    /// source handles that clients pass to ext-image-copy-capture
    /// to identify what they want to capture. xdp-wlr 0.8+ uses
    /// these for the per-window screencast path.
    pub image_capture_source: smithay::wayland::image_capture_source::ImageCaptureSourceState,
    /// `ext-output-image-capture-source-manager-v1` global —
    /// "give me a capture source for this wl_output". Backs the
    /// monitor-share path in xdp-wlr.
    pub output_capture_source: smithay::wayland::image_capture_source::OutputCaptureSourceState,
    /// `ext-foreign-toplevel-image-capture-source-manager-v1`
    /// global — "give me a capture source for this toplevel".
    /// Margo's `ForeignToplevelListState` already implements the
    /// matching `ext-foreign-toplevel-list-v1`, so xdp-wlr can
    /// enumerate windows + ask for per-window capture; this is
    /// the protocol that lights up the **Window tab** in
    /// browser-based meeting clients (Google Meet, Zoom Web,
    /// Discord, Jitsi).
    pub toplevel_capture_source: smithay::wayland::image_capture_source::ToplevelCaptureSourceState,
    /// `ext-image-copy-capture-v1` — the actual capture transport.
    /// Clients open a session against an `ImageCaptureSource`,
    /// receive buffer constraints, allocate a matching buffer,
    /// then request a frame which margo renders into the buffer.
    pub image_copy_capture: smithay::wayland::image_copy_capture::ImageCopyCaptureState,
    /// `wp_presentation` global. Lets clients (kitty, mpv, native
    /// Wayland Vulkan games via DXVK / VKD3D, video conferencing
    /// apps that adapt their pacing to the actual display refresh)
    /// register `wp_presentation_feedback` per-frame and learn the
    /// real `presented` timestamp + refresh interval. Without this
    /// they're stuck guessing — kitty falls back to a 60 Hz tick,
    /// mpv ships its own debouncer, vsync-sensitive games stutter.
    pub presentation: PresentationState,
    /// `ext_idle_notifier_v1`: pings clients (swayidle, noctalia) once
    /// the seat has been idle for the duration they registered.
    pub idle_notifier: smithay::wayland::idle_notify::IdleNotifierState<MargoState>,
    /// `zwp_idle_inhibit_manager_v1`: clients (mpv, video players,
    /// presentation tools) can request "don't go idle while my surface
    /// is on screen". The notifier is paused while the set is non-empty.
    pub idle_inhibit: smithay::wayland::idle_inhibit::IdleInhibitManagerState,
    /// `zwp_virtual_keyboard_v1`: lets unprivileged clients (wayvnc
    /// for VNC sessions, `wtype` / `ydotool` for synthetic input,
    /// IMEs) inject key events into the focused surface. The state
    /// is stored only so `delegate_virtual_keyboard_manager!` has a
    /// home; smithay drives the global from there.
    pub virtual_keyboard_manager: smithay::wayland::virtual_keyboard::VirtualKeyboardManagerState,
    /// `zwp_keyboard_shortcuts_inhibit_manager_v1`: lets a client
    /// (vncviewer, RDP, VirtualBox, browser remote-desktop apps)
    /// request that margo stop matching its own keybindings while the
    /// client's surface has keyboard focus. The protocol state owns
    /// the global; the live inhibitors are tracked separately so the
    /// input filter can do an O(1) lookup by focused wl_surface.
    pub keyboard_shortcuts_inhibit:
        smithay::wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitState,
    /// `zwp_pointer_gestures_v1`: forwards touchpad pinch / swipe /
    /// hold gestures to clients (Firefox pinch-zoom, GNOME apps,
    /// Inkscape). Smithay drives the global from `SeatHandler`; state
    /// is held only so the delegate macro has a home.
    pub pointer_gestures: smithay::wayland::pointer_gestures::PointerGesturesState,
    /// `wp_single_pixel_buffer_v1`: lets clients allocate solid-color
    /// buffers without a real shm/dmabuf allocation. Pure smithay
    /// state — no handler trait, no policy.
    pub single_pixel_buffer: smithay::wayland::single_pixel_buffer::SinglePixelBufferState,
    /// `xdg_foreign_v2`: cross-process surface embedding — Firefox /
    /// Chromium Picture-in-Picture, xdg-desktop-portal screencast
    /// window targeting. A client exports a surface, sends the handle
    /// to another client, which imports it as a parent.
    pub xdg_foreign: smithay::wayland::xdg_foreign::XdgForeignState,
    /// `zwp_tablet_manager_v2`: Wacom / Huion drawing tablets — pen
    /// pressure, tilt, eraser, buttons. Without this, tablets fall
    /// back to mouse-emulation only.
    pub tablet_manager: smithay::wayland::tablet_manager::TabletManagerState,
    /// `wp_security_context_v1`: lets Flatpak / sandboxed clients
    /// connect through a separate socket the sandbox engine
    /// pre-allocates. The handler inserts the listener source into
    /// margo's calloop so sandboxed apps can talk to the compositor.
    pub security_context: smithay::wayland::security_context::SecurityContextState,
    /// `org_kde_kwin_server_decoration`: legacy KDE decoration
    /// protocol — some older Qt5 / KDE apps still expect it.
    /// xdg-decoration covers most modern apps.
    pub kde_decoration: smithay::wayland::shell::kde::decoration::KdeDecorationState,
    /// `wp_content_type_v1`: clients tag their surfaces as video /
    /// game / photo so the compositor can adjust scheduling
    /// (disable presentation-time hints for games, allow tearing,
    /// etc.).
    pub content_type: smithay::wayland::content_type::ContentTypeState,
    /// `wp_fifo_v1`: FIFO commit ordering. Smithay's managed state installs
    /// commit barriers; `state/pacing.rs` releases them at each output's
    /// present (and via the hidden-surface fallback), which is what makes
    /// advertising this global safe — see road_map P15.
    pub fifo_manager: smithay::wayland::fifo::FifoManagerState,
    /// `wp_commit_timing_v1`: explicit commit-time targets. Barriers are
    /// released by the same pacing scheduler (present pass + exact deadline
    /// wake armed from the surface pre-commit hook).
    pub commit_timing_manager: smithay::wayland::commit_timing::CommitTimingManagerState,
    /// `wp_alpha_modifier_v1`: per-surface alpha hint so apps can
    /// fade themselves without going through compositor effects.
    pub alpha_modifier: smithay::wayland::alpha_modifier::AlphaModifierState,
    /// `xdg_wm_dialog_v1`: modal-dialog hint — compositor can place
    /// / decorate dialogs differently from regular toplevels.
    pub xdg_dialog: smithay::wayland::shell::xdg::dialog::XdgDialogState,
    /// `zwp_xwayland_keyboard_grab_v1`: lets XWayland clients ask
    /// the compositor to install an exclusive keyboard grab on
    /// their behalf. Companion to the v0.5.0 X11 focus fix and the
    /// new `zwp_keyboard_shortcuts_inhibit_v1` global — same VNC /
    /// VM / remote-desktop story, X11-side mechanism.
    pub xwayland_keyboard_grab: smithay::wayland::xwayland_keyboard_grab::XWaylandKeyboardGrabState,
    /// `xdg_toplevel_icon_v1`: toplevels can ship their own icon
    /// inline (PNG / SVG buffer) instead of the bar inferring it
    /// from the desktop file. mshell's active-window pill can
    /// surface this once a UI consumer is wired up.
    pub xdg_toplevel_icon: smithay::wayland::xdg_toplevel_icon::XdgToplevelIconManager,
    /// `xdg_system_bell_v1`: clients ring the system bell. We just
    /// advertise the global for now and log; routing to a sound
    /// daemon / mshell notification is a future enhancement.
    pub xdg_system_bell: smithay::wayland::xdg_system_bell::XdgSystemBellState,
    /// `wp_pointer_warp_v1`: clients can ask the compositor to
    /// move the cursor to a surface-local position. Default no-op
    /// — programmatic cursor movement is opt-in policy.
    pub pointer_warp: smithay::wayland::pointer_warp::PointerWarpManager,
    /// `xdg_toplevel_tag_v1`: clients attach semantic tags +
    /// description strings to toplevels. Default no-op; could feed
    /// into window-rule matching later.
    pub xdg_toplevel_tag: smithay::wayland::xdg_toplevel_tag::XdgToplevelTagManager,
    /// `wp_cursor_shape_v1` — clients (GTK/Qt/Chromium) request a
    /// cursor by name instead of attaching their own surface. Without
    /// this, GTK rolls its own cursor surface and the buffer scale
    /// gets out of sync on layer-shell surfaces (mshell bar/menus),
    /// producing an oversized cursor. With it, GTK takes the named-
    /// cursor path and the compositor draws using `cursor_manager`
    /// at its own size.
    pub cursor_shape_manager: smithay::wayland::cursor_shape::CursorShapeManagerState,
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
    pub fractional_scale_manager: smithay::wayland::fractional_scale::FractionalScaleManagerState,
    pub foreign_toplevel_list: ForeignToplevelListState,
    pub wlr_foreign_toplevel: crate::protocols::wlr_foreign_toplevel::WlrForeignToplevelState,
    pub ext_workspace: crate::protocols::ext_workspace::ExtWorkspaceManagerState,
    pub virtual_pointer: crate::protocols::virtual_pointer::VirtualPointerManagerState,
    pub xwayland_shell: XWaylandShellState,
    pub gamma_control_manager: crate::protocols::gamma_control::GammaControlManagerState,
    pub output_power_manager: crate::protocols::output_power::OutputPowerManagerState,
    pub screencopy: crate::protocols::screencopy::ScreencopyManagerState,
}

impl Protocols {
    /// Register every protocol global margo advertises.
    pub fn new(
        dh: &smithay::reexports::wayland_server::DisplayHandle,
        config: &Config,
        loop_handle: &LoopHandle<'static, MargoState>,
    ) -> Self {
        let compositor = CompositorState::new::<MargoState>(dh);
        let xdg_shell = XdgShellState::new::<MargoState>(dh);
        let xdg_decoration = XdgDecorationState::new::<MargoState>(dh);
        let shm_formats = vec![
            smithay::reexports::wayland_server::protocol::wl_shm::Format::Argb8888,
            smithay::reexports::wayland_server::protocol::wl_shm::Format::Xrgb8888,
            smithay::reexports::wayland_server::protocol::wl_shm::Format::Xbgr8888,
            smithay::reexports::wayland_server::protocol::wl_shm::Format::Abgr8888,
            smithay::reexports::wayland_server::protocol::wl_shm::Format::Rgb565,
        ];
        let shm = ShmState::new::<MargoState>(dh, shm_formats);
        let viewporter = ViewporterState::new::<MargoState>(dh);
        let dmabuf = DmabufState::new();
        let output_manager = OutputManagerState::new_with_xdg_output::<MargoState>(dh);
        let data_device = DataDeviceState::new::<MargoState>(dh);
        DISABLE_MIDDLE_PASTE.store(
            config.disable_middle_paste,
            std::sync::atomic::Ordering::Relaxed,
        );
        let primary_selection = PrimarySelectionState::new_with_filter::<MargoState, _>(dh, |_| {
            !middle_paste_disabled()
        });
        let data_control = DataControlState::new::<MargoState, _>(
            dh,
            Some(&primary_selection),
            client_is_unrestricted,
        );
        let ext_data_control = ExtDataControlState::new::<MargoState, _>(
            dh,
            Some(&primary_selection),
            client_is_unrestricted,
        );
        let cursor_shape_manager =
            smithay::wayland::cursor_shape::CursorShapeManagerState::new::<MargoState>(dh);
        let fractional_scale_manager =
            smithay::wayland::fractional_scale::FractionalScaleManagerState::new::<MargoState>(dh);
        let layer_shell =
            WlrLayerShellState::new_with_filter::<MargoState, _>(dh, client_is_unrestricted);
        let session_lock = smithay::wayland::session_lock::SessionLockManagerState::new::<
            MargoState,
            _,
        >(dh, client_is_unrestricted);
        let text_input = TextInputManagerState::new::<MargoState>(dh);
        let input_method =
            InputMethodManagerState::new::<MargoState, _>(dh, client_is_unrestricted);
        let pointer_constraints = PointerConstraintsState::new::<MargoState>(dh);
        let relative_pointer = RelativePointerManagerState::new::<MargoState>(dh);
        let xdg_activation = XdgActivationState::new::<MargoState>(dh);
        let output_management =
            crate::protocols::output_management::OutputManagementManagerState::new::<MargoState, _>(
                dh,
                client_is_unrestricted,
            );
        // wp_color_management_v1 (staging) — opt-in via the
        // `color_management = 1` knob. Standing the global up lets
        // HDR-aware clients (Chromium, mpv) detect "this compositor
        // speaks colour management" and enable their decode paths.
        // Default off until the probe path is hardware-verified; see
        // `protocols/color_management.rs` and `docs/hdr-design.md`.
        let color_management = crate::protocols::color_management::ColorManagementState::new::<
            MargoState,
            _,
        >(dh, config.color_management, |_client| true);
        // wp_color_representation_v1 (staging) — advertised for real
        // (unlike the gated colour-management manager above): the
        // supported set (premultiplied alpha, identity/full RGB) is
        // exactly what the render path already does.
        let color_representation =
            crate::protocols::color_representation::ColorRepresentationState::new::<MargoState, _>(
                dh,
                |_client| true,
            );
        // ext-image-capture-source-v1 + ext-image-copy-capture-v1
        // — the modern Wayland screencast stack. Without these
        // globals, xdp-wlr 0.8+ can't expose per-window share
        // (Window tab in meeting clients). Smithay ships full
        // helpers; output and toplevel source globals are
        // independent so we can advertise both. Hidden from sandboxed
        // clients: they capture through the portal.
        let image_capture_source =
            smithay::wayland::image_capture_source::ImageCaptureSourceState::new();
        let output_capture_source =
            smithay::wayland::image_capture_source::OutputCaptureSourceState::new_with_filter::<
                MargoState,
                _,
            >(dh, client_is_unrestricted);
        let toplevel_capture_source =
            smithay::wayland::image_capture_source::ToplevelCaptureSourceState::new_with_filter::<
                MargoState,
                _,
            >(dh, client_is_unrestricted);
        let image_copy_capture =
            smithay::wayland::image_copy_capture::ImageCopyCaptureState::new_with_filter::<
                MargoState,
                _,
            >(dh, client_is_unrestricted);
        // Clock id 1 = CLOCK_MONOTONIC. That's the same domain
        // `monotonic_now()` in the udev backend uses, so the
        // timestamps we publish are consistent with the ones
        // clients see in their own `clock_gettime(CLOCK_MONOTONIC)`.
        let presentation = PresentationState::new::<MargoState>(dh, 1);
        let idle_notifier = smithay::wayland::idle_notify::IdleNotifierState::<MargoState>::new(
            dh,
            loop_handle.clone(),
        );
        let idle_inhibit =
            smithay::wayland::idle_inhibit::IdleInhibitManagerState::new::<MargoState>(dh);
        // `zwp_virtual_keyboard_v1` — required by wayvnc to inject
        // keystrokes from VNC clients, and used by `wtype` /
        // `ydotool` for synthetic input. Hidden from sandboxed clients
        // (`client_is_unrestricted`): injected input escapes the sandbox.
        let virtual_keyboard_manager =
            smithay::wayland::virtual_keyboard::VirtualKeyboardManagerState::new::<MargoState, _>(
                dh,
                client_is_unrestricted,
            );
        // `zwp_keyboard_shortcuts_inhibit_v1` — required by vncviewer,
        // RDP clients, VirtualBox, and any app that needs the host
        // compositor to stop intercepting its own keybindings (Super,
        // Alt+Tab, …) while the client surface has focus.
        let keyboard_shortcuts_inhibit =
            smithay::wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitState::new::<
                MargoState,
            >(dh);
        // `zwp_pointer_gestures_v1` — touchpad gestures (pinch /
        // swipe / hold) forwarded to focused client.
        let pointer_gestures =
            smithay::wayland::pointer_gestures::PointerGesturesState::new::<MargoState>(dh);
        // `wp_single_pixel_buffer_v1` — solid-color buffer fast-path.
        let single_pixel_buffer =
            smithay::wayland::single_pixel_buffer::SinglePixelBufferState::new::<MargoState>(dh);
        // `xdg_foreign_v2` — cross-process surface embedding.
        let xdg_foreign = smithay::wayland::xdg_foreign::XdgForeignState::new::<MargoState>(dh);
        // `zwp_tablet_manager_v2` — pen tablets.
        let tablet_manager =
            smithay::wayland::tablet_manager::TabletManagerState::new::<MargoState>(dh);
        // `wp_security_context_v1` — sandboxed-client socket
        // pre-allocation. Clients connecting through such a socket are
        // marked restricted (see `state/handlers/security_context.rs`);
        // they can't see this global themselves, so a sandboxed app can't
        // mint a new context to shed its restrictions.
        let security_context = smithay::wayland::security_context::SecurityContextState::new::<
            MargoState,
            _,
        >(dh, client_is_unrestricted);
        // `org_kde_kwin_server_decoration` — legacy KDE deco.
        // Default to server-side decoration since margo's existing
        // xdg-decoration policy is SSD-first.
        let kde_decoration =
            smithay::wayland::shell::kde::decoration::KdeDecorationState::new::<MargoState>(
                dh,
                smithay::reexports::wayland_protocols_misc::server_decoration::server::org_kde_kwin_server_decoration_manager::Mode::Server,
            );
        // `wp_content_type_v1` — surface content-type hints.
        let content_type = smithay::wayland::content_type::ContentTypeState::new::<MargoState>(dh);
        // `wp_fifo_v1` + `wp_commit_timing_v1` — presentation pacing. These
        // were withdrawn while nothing released Smithay's managed commit
        // barriers (a hidden-then-shown Chromium surface stalled behind
        // them); `state/pacing.rs` now signals FIFO barriers at every
        // per-output present, releases hidden surfaces from the fallback
        // timer, and honours commit-timing deadlines with an exact one-shot
        // wake, so the globals are safe to advertise again.
        let fifo_manager = smithay::wayland::fifo::FifoManagerState::new::<MargoState>(dh);
        let commit_timing_manager =
            smithay::wayland::commit_timing::CommitTimingManagerState::new::<MargoState>(dh);
        // `wp_alpha_modifier_v1` — per-surface alpha hint.
        let alpha_modifier =
            smithay::wayland::alpha_modifier::AlphaModifierState::new::<MargoState>(dh);
        // `xdg_wm_dialog_v1` — modal-dialog hint.
        let xdg_dialog =
            smithay::wayland::shell::xdg::dialog::XdgDialogState::new::<MargoState>(dh);
        // `zwp_xwayland_keyboard_grab_v1` — XWayland-side keyboard grab.
        let xwayland_keyboard_grab =
            smithay::wayland::xwayland_keyboard_grab::XWaylandKeyboardGrabState::new::<MargoState>(
                dh,
            );
        // `xdg_toplevel_icon_v1` — inline app icons on toplevels.
        let xdg_toplevel_icon =
            smithay::wayland::xdg_toplevel_icon::XdgToplevelIconManager::new::<MargoState>(dh);
        // `xdg_system_bell_v1` — system bell.
        let xdg_system_bell =
            smithay::wayland::xdg_system_bell::XdgSystemBellState::new::<MargoState>(dh);
        // `wp_pointer_warp_v1` — programmatic pointer position requests.
        let pointer_warp =
            smithay::wayland::pointer_warp::PointerWarpManager::new::<MargoState>(dh);
        // `xdg_toplevel_tag_v1` — semantic toplevel tags.
        let xdg_toplevel_tag =
            smithay::wayland::xdg_toplevel_tag::XdgToplevelTagManager::new::<MargoState>(dh);

        let xwayland_shell = XWaylandShellState::new::<MargoState>(dh);
        // Every privileged global below is hidden from sandboxed clients
        // via `client_is_unrestricted` (window lists, workspace control,
        // synthetic pointer, gamma, DPMS, screen capture).
        let foreign_toplevel_list =
            ForeignToplevelListState::new_with_filter::<MargoState>(dh, client_is_unrestricted);
        let wlr_foreign_toplevel =
            crate::protocols::wlr_foreign_toplevel::WlrForeignToplevelState::new::<MargoState, _>(
                dh,
                client_is_unrestricted,
            );
        let ext_workspace = crate::protocols::ext_workspace::ExtWorkspaceManagerState::new::<
            MargoState,
            _,
        >(dh, client_is_unrestricted);
        let virtual_pointer = crate::protocols::virtual_pointer::VirtualPointerManagerState::new::<
            MargoState,
            _,
        >(dh, client_is_unrestricted);

        // wlr_gamma_control_v1 — sunsetr / gammastep / wlsunset use this to
        // push night-light ramps to outputs.
        let gamma_control_manager = crate::protocols::gamma_control::GammaControlManagerState::new::<
            MargoState,
            _,
        >(dh, client_is_unrestricted);

        // wlr-output-power-management-v1: lets idle daemons (`swayidle`) and
        // `wlr-randr` power outputs off/on via DPMS.
        let output_power_manager = crate::protocols::output_power::OutputPowerManagerState::new::<
            MargoState,
            _,
        >(dh, client_is_unrestricted);

        // wlr-screencopy-unstable-v1: lets `grim`, `wf-recorder`, `screen rec`
        // etc. capture compositor outputs.
        let screencopy = crate::protocols::screencopy::ScreencopyManagerState::new::<MargoState, _>(
            dh,
            client_is_unrestricted,
        );

        Self {
            compositor,
            xdg_shell,
            xdg_decoration,
            shm,
            viewporter,
            dmabuf,
            dmabuf_global: None,
            drm_syncobj: None,
            layer_shell,
            output_manager,
            data_device,
            primary_selection,
            data_control,
            ext_data_control,
            session_lock,
            text_input,
            input_method,
            pointer_constraints,
            relative_pointer,
            xdg_activation,
            output_management,
            color_management,
            color_representation,
            drm_lease: None,
            image_capture_source,
            output_capture_source,
            toplevel_capture_source,
            image_copy_capture,
            presentation,
            idle_notifier,
            idle_inhibit,
            virtual_keyboard_manager,
            keyboard_shortcuts_inhibit,
            pointer_gestures,
            single_pixel_buffer,
            xdg_foreign,
            tablet_manager,
            security_context,
            kde_decoration,
            content_type,
            fifo_manager,
            commit_timing_manager,
            alpha_modifier,
            xdg_dialog,
            xwayland_keyboard_grab,
            xdg_toplevel_icon,
            xdg_system_bell,
            pointer_warp,
            xdg_toplevel_tag,
            cursor_shape_manager,
            fractional_scale_manager,
            foreign_toplevel_list,
            wlr_foreign_toplevel,
            ext_workspace,
            virtual_pointer,
            xwayland_shell,
            gamma_control_manager,
            output_power_manager,
            screencopy,
        }
    }
}
