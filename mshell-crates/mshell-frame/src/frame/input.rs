//! `Frame`'s component messages: `FrameInput`, `FrameOutput`, `FrameInit`.

use super::*;

#[derive(Debug)]
pub enum FrameInput {
    SetDrawFrame(bool),
    QueueFrameRedraw,
    SetLeftMenuExpansionType(VerticalMenuExpansion),
    SetRightMenuExpansionType(VerticalMenuExpansion),
    /// Re-place every left/right-side menu in the stack. Carries the
    /// snapshot of all menu positions the firing effect just read, so the
    /// handler can skip the (destructive) restack when nothing actually
    /// moved — the config store is coarse, so this effect re-fires on
    /// every unrelated setting write. Fired by the per-output effect that
    /// subscribes to all menu positions.
    RepositionMenus(Vec<Position>),
    /// Toggle one of the uniform menus — open path is reveal + keyboard-mode
    /// sync. The [`MenuId`] payload selects which. Menus with bespoke open
    /// logic keep their own variants below.
    ToggleMenu(MenuId),
    /// Close every open menu, then open `MenuId` (dashboard shortcuts).
    SwitchToMenu(MenuId),
    /// Re-assert the layer surface's keyboard interactivity from the
    /// current menu-reveal state. Fired once when the frame surface
    /// first maps: gtk4-layer-shell's initial commit can land with a
    /// non-`None` keyboard mode in some races (observed: the frame
    /// holding `Exclusive` at login while no menu is open, trapping
    /// keyboard focus into the invisible full-screen layer until the
    /// first menu toggle finally ran `sync_keyboard_mode`). This
    /// enforces the "Exclusive iff a menu is revealed" invariant
    /// proactively at map time instead of only reactively on toggle.
    SyncKeyboardMode,
    /// Forward a Hidden Bar IPC verb to both bars' drawers. The optional
    /// target name selects a single named drawer; `None` reaches all.
    HiddenBar(mshell_common::hidden_bar::HiddenBarVerb, Option<String>),
    /// A bar reported its target reserved height (`BarOutput::ReserveHeight`).
    /// Routed to that bar's FrameSpacer so the layer-shell exclusive zone
    /// jumps to the final value at toggle time (one smooth compositor
    /// resize), instead of being streamed from the Revealer slide.
    /// `is_top` picks which spacer; the height is the bar's natural content.
    SpacerReserve {
        is_top: bool,
        height: i32,
    },
    /// Same as `ToggleMenu(MenuId::AppLauncher)` but also forwards a
    /// `SelectCategory(tab)` into the underlying
    /// `AppLauncherModel` once the menu is open. Bridges
    /// `mshellctl menu app-launcher --tab Run` to the runtime's
    /// existing category-cycle path.
    ToggleAppLauncherMenuWithTab(String),
    /// Open the first-class plugin-panel menu hosting a plugin's WASM panel,
    /// carrying its compiled component path + resolved settings (JSON) + the
    /// granted host capabilities (comma-separated tokens; deny-by-default).
    ToggleWasmPluginPanel {
        name: String,
        entry: String,
        settings: String,
        capabilities: String,
        min_width: i32,
        max_height: i32,
    },
    /// Open a declarative plugin menu (its `[[widget.menu]]` command rows) in
    /// the first-class plugin menu — layer-shell, not a pill popover.
    TogglePluginMenu {
        name: String,
        rows: Vec<CustomMenuRow>,
        min_width: i32,
        max_height: i32,
    },
    /// Toggle a plugin's panel/menu addressed by key (from `mshellctl menu
    /// plugin <key>`). Generic — resolves the key to the plugin's derived
    /// widget, then dispatches to the panel or menu path. No per-plugin code.
    TogglePluginByKey(String),
    /// Force-reload an installed plugin's WASM panel — evict the cached
    /// instance so the next open re-instantiates from disk. Lets a plugin
    /// author wire `cargo watch` to `mshellctl plugin reload <key>` for a
    /// fast iteration loop without restarting mshell.
    ReloadPlugin(String),
    /// A global keybind for `(plugin-key, bind-id)` fired: open (or
    /// reveal) the plugin's panel and deliver a `Keybind` event to the
    /// guest with the bind id. Margo binds in the generated
    /// `binds.d/mshell-plugins.conf` spawn `mshellctl plugin keybind …`
    /// which lands here.
    FirePluginKeybind(String, String),
    ToggleSettingsMenu,
    /// Open Settings and jump to a specific sidebar section.
    /// Used by the launcher's Settings provider via the shell
    /// router. If Settings is already visible, just switches the
    /// section.
    OpenSettingsAtSection(String),
    /// Force-close Settings on this frame if it's currently open.
    /// Used by the Shell-level Settings router to keep Settings
    /// single-monitor: when a fresh toggle picks frame A, frame B's
    /// open Settings is closed out from under it so the panel
    /// doesn't linger on a monitor the user is no longer viewing.
    CloseSettingsMenu,
    CloseMenus,
    /// Esc while the clipboard `/` filter is open — tell the clipboard
    /// menu to leave search mode instead of closing the whole menu.
    ClipboardExitSearch,
    /// A `Screenshare` D-Bus request (reply channel, payload). Despite the
    /// historical name it never toggles: the chooser queues the request and
    /// sends [`FrameInput::ShowScreenshareMenu`] when it is on screen.
    ToggleScreenshareMenu(tokio::sync::oneshot::Sender<String>, String),
    /// Open the screen-share chooser if it is not open.
    ShowScreenshareMenu,
    BarToggleTop,
    BarToggleBottom,
    BarToggleAll(bool),
    BarRevealAll(bool),
    BarHideAll(bool),
}

#[derive(Debug)]
pub enum FrameOutput {}

pub struct FrameInit {
    pub monitor: gdk::Monitor,
}
