//! Output changes queued on `MargoState` for the backend to apply.
//!
//! Protocol handlers (gamma control, DPMS, output management) and the udev
//! hotplug debouncer run without a borrow on the backend, so they queue
//! work here; the udev repaint path drains it. Winit drops it.

use smithay::output::Output;

#[derive(Default)]
pub struct OutputPending {
    /// Pending gamma ramp updates drained by the udev backend each frame.
    /// Tuple is (output, ramp). `None` ramp = restore default. The udev
    /// backend pops these and applies them via DRM `GAMMA_LUT`. Winit just
    /// drops them silently.
    pub gamma: Vec<(Output, Option<Vec<u16>>)>,
    /// Pending DPMS power changes drained by the udev backend each frame.
    /// `(output, on)` — `on = false` calls `DrmCompositor::clear()` (panel
    /// off); `true` re-renders the output (panel on). Winit drops them.
    pub dpms: Vec<(Output, bool)>,
    /// Mode changes accepted by `apply_output_pending` but not yet
    /// applied at the DRM layer. The udev repaint handler drains
    /// this and feeds each entry through `DrmCompositor::use_mode`,
    /// then updates the smithay `Output` state so wl_output mode
    /// events fire for any client (kanshi watcher, status bar).
    /// Held outside the apply path because the handler runs on
    /// MargoState and doesn't have a borrow on the udev BackendData.
    pub mode_changes: Vec<crate::PendingOutputModeChange>,
    /// The backend drains `mode_changes` (udev sets this at startup).
    /// Under winit / headless nothing does, so wlr-output-management
    /// refuses mode changes there instead of reporting success.
    pub mode_changes_supported: bool,
    /// Last udev `Changed` event timestamp, used by the hotplug
    /// debouncer in `backend/udev/mod.rs`. A burst of udev events
    /// (gamma daemon racing the lock screen, kernel firing
    /// `udev_device_get_changed` for every property tweak, …) used
    /// to call `rescan_outputs` synchronously per event — hundreds
    /// of times in 100 ms during the worst-case crash that
    /// surfaced this. `Some` ⇒ a coalesce timer is currently armed
    /// and a rescan will fire ~50 ms after the last event.
    pub hotplug_last_event_at: Option<std::time::Instant>,
    /// Sentinel: a debounce timer is already armed in the event
    /// loop. Set true when the first event of a burst arrives,
    /// cleared by the timer once it actually runs the rescan.
    pub hotplug_rescan_pending: bool,
}
