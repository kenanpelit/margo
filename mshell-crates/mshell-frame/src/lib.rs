mod agents;
pub mod audio_inhibit;
mod bars;
mod common_widgets;
mod countdown;
mod crashes;
mod discord;
pub mod eyecare;
mod drive_health;
pub mod frame;
mod frame_draw_widget;
mod frame_spacer;
mod github;
mod keep_awake;
mod keybinds;
mod lyrics;
pub mod mdock_surface;
mod menus;
mod plugin_cmd;
#[cfg(feature = "wasm-plugins")]
mod plugin_providers;
pub mod pomodoro;
mod removable_drives;
pub mod rss;
pub mod screen_corners;
mod shelf;
mod ssh;

/// `mshellctl qr …`: the request the QR menu shows when it next opens, and
/// the bus-argument validator. The IPC handler queues it, then opens the
/// menu (`MenuId::Qr`).
pub use crate::menus::menu_widgets::qr::{QrRequest, parse_qr_request, request as request_qr};

/// Headless screenshot capture — drives the shell's own screenshot engine
/// (the same one the screenshot menu uses: in-shell selectors + save /
/// clipboard / editor / notify) without opening the menu. Called from the
/// IPC handler so `mshellctl screenshot <area>` and a keybind capture run
/// the exact same path as the GUI.
pub fn capture_screenshot(
    area: mshell_screenshot::CaptureArea,
    target: mshell_screenshot::OutputTarget,
    delay: std::time::Duration,
) {
    crate::menus::menu_widgets::screenshot::screenshot_menu_widget::capture(area, target, delay);
}

/// Headless screen recording — `start` / `stop` / `toggle`, sharing the
/// menu's recording engine + state (the recording-indicator pill tracks it).
/// Called from the IPC handler so `mshellctl screenrecord …` and a keybind
/// run the same path as the GUI.
pub fn screen_record(
    action: &str,
    area: mshell_screenshot::CaptureArea,
    audio: Option<String>,
    backend: Option<mshell_screenshot::record::RecordBackend>,
    webcam: Option<mshell_screenshot::webcam::WebcamSize>,
) {
    use crate::menus::menu_widgets::screen_record::screen_record_menu_widget as r;
    match action {
        "stop" => r::record_stop(),
        "toggle" => r::record_toggle(area, audio, backend, webcam),
        _ => r::record_start(area, audio, backend, webcam), // "start"
    }
}

mod stopwatch;
mod system_update;
mod twilight;
mod valent;
