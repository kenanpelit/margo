//! Settings panel — pluggable into the frame's menu stack.
//!
//! Settings used to launch as its own `gtk::Window` toplevel.
//! That made the panel feel like a different app from the rest
//! of the shell, took longer to open (the compositor had to
//! allocate a new surface + ack a configure), and made the
//! decoration / titlebar fight the rest of the layer-shell UI.
//!
//! Now it lives in the frame's menu stack like every other menu
//! surface. The frame registers itself as the toggle backend via
//! [`set_toggle_backend`]; quick-action buttons and the
//! `mshellctl settings open` IPC call route through that backend
//! to flip the menu's reveal state.

use std::sync::OnceLock;

mod bar_pill_settings;
mod bar_settings;
mod display_settings;
mod fonts_settings;
mod general_settings;
mod idle_settings;
mod launcher_settings;
mod layout_settings;
mod menu_settings;
mod notification_settings;
mod session_settings;
pub mod settings;
mod theme_settings;
mod wallpaper_settings;
mod widget_menu_settings;

pub use settings::{
    SettingsWindowCommandOutput, SettingsWindowInit, SettingsWindowInput, SettingsWindowModel,
    SettingsWindowOutput,
};

/// Toggle backend: a thunk the frame registers so external
/// callers (quick-action button, IPC) can flip the settings
/// menu's reveal state without owning a `Sender<FrameInput>`
/// directly.
type ToggleBackend = Box<dyn Fn() + Send + Sync + 'static>;
static TOGGLE_BACKEND: OnceLock<ToggleBackend> = OnceLock::new();

/// Register the toggle backend. Called once by the frame at
/// startup. Idempotent: a second call is silently ignored.
pub fn set_toggle_backend<F>(f: F)
where
    F: Fn() + Send + Sync + 'static,
{
    let _ = TOGGLE_BACKEND.set(Box::new(f));
}

/// External-facing toggle. Called by the quick-action button and
/// by the IPC layer. Routes to the frame's `ToggleSettingsMenu`
/// if a backend is registered; otherwise logs a warning and
/// does nothing (the shell hasn't finished constructing the
/// frame yet, which would only happen if a hot path opened
/// settings before the layer surface was up).
pub fn open_settings() {
    if let Some(toggle) = TOGGLE_BACKEND.get() {
        toggle();
    } else {
        tracing::warn!("settings: toggle backend not registered yet");
    }
}

/// Backwards-compatibility shim for the old "close" IPC. Since
/// the panel is a menu now, "close" is just another toggle —
/// the frame's `toggle_menu` flips visible menus off.
pub fn close_settings() {
    open_settings();
}

/// Section-navigation backend: a thunk the frame registers so
/// external callers can jump to a specific Settings sidebar
/// section without owning a `Sender<SettingsWindowInput>`. The
/// backend's argument is the stack-child name
/// (`general`/`bar`/`display`/`fonts`/`idle`/`menus`/`theme`/
/// `wallpaper`/`widgets`) — anything else is silently ignored
/// inside the settings widget.
type SectionBackend = Box<dyn Fn(&str) + Send + Sync + 'static>;
static SECTION_BACKEND: OnceLock<SectionBackend> = OnceLock::new();

/// Register the section-navigation backend. Called once by the
/// frame at startup. Idempotent.
pub fn set_section_backend<F>(f: F)
where
    F: Fn(&str) + Send + Sync + 'static,
{
    let _ = SECTION_BACKEND.set(Box::new(f));
}

/// External-facing section navigator. Activates the matching
/// sidebar button and ensures Settings is visible. Used by the
/// launcher's Settings provider via the `mshellctl settings open
/// --section <id>` IPC chain.
pub fn open_settings_at_section(section: &str) {
    if let Some(backend) = SECTION_BACKEND.get() {
        backend(section);
    } else {
        tracing::warn!(section, "settings: section backend not registered yet");
    }
    // Always toggle the panel visible — the backend only switches
    // the inner stack, it doesn't open the menu.
    open_settings();
}
