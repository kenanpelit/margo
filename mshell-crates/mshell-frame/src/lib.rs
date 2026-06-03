mod bars;
mod common_widgets;
pub mod frame;
mod frame_draw_widget;
mod frame_spacer;
mod keep_awake;
mod keybinds;
mod menus;
#[cfg(feature = "wasm-plugins")]
mod plugin_providers;
pub mod screen_corners;
mod ssh;

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
pub fn screen_record(action: &str, area: mshell_screenshot::CaptureArea, audio: Option<String>) {
    use crate::menus::menu_widgets::screen_record::screen_record_menu_widget as r;
    match action {
        "stop" => r::record_stop(),
        "toggle" => r::record_toggle(area, audio),
        _ => r::record_start(area, audio), // "start"
    }
}

mod stopwatch;
mod system_update;
mod twilight;
mod valent;
