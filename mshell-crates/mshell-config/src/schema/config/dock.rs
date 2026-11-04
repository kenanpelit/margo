//! The standalone dock (mdock) and the system tray.

use super::*;

/// How the standalone mdock surface is presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DockStyle {
    /// A session-menu-style popup: opens on demand as a floating panel, closes
    /// on Esc / click-away. Not pinned to the screen edge.
    Popup,
    /// An edge-anchored dock (Always visible / Auto-hide), pinned to the screen.
    #[default]
    LayerShell,
}

/// Standalone-dock (mdock) reveal behaviour (only used in LayerShell style).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DockBehavior {
    /// Always visible; reserves an exclusive zone.
    Always,
    /// Hidden; a thin edge trigger reveals it on hover (hydock style).
    #[default]
    AutoHide,
    /// Hidden; shown/hidden via `mshellctl dock toggle` / a keybind.
    Toggle,
}

/// Which screen edge the standalone mdock surface anchors to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DockPosition {
    Top,
    #[default]
    Bottom,
    Left,
    Right,
}

/// mdock — the running/pinned app dock. Two modes: a bar-widget pill and a
/// standalone per-output layer-shell surface (always / auto-hide / toggle).
/// Tunables surfaced under Settings → Widgets → mdock. (Pins live in the
/// `pinned_apps_store` cache, not here.) serde key stays `dock`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Dock {
    /// App-icon pixel size.
    pub icon_size: u32,
    /// Hover tooltip listing the app + its open window titles.
    pub show_tooltips: bool,
    /// Include running apps that aren't pinned (off = pinned-only dock).
    pub show_running: bool,
    /// Per-app icon overrides — map a window class to an icon name or an
    /// absolute file path. For apps launched with a synthetic `--class` that
    /// has no matching `.desktop` (e.g. isolated browser profiles), which
    /// would otherwise fall back to a generic icon.
    pub icon_overrides: Vec<DockIconOverride>,

    // ── mdock additions ─────────────────────────────────────────────
    /// Show the dock as a bar-widget pill (the classic mode).
    pub in_bar: bool,
    /// Run the standalone dock surface.
    pub standalone: bool,
    /// How the standalone surface is presented (popup vs edge layer-shell).
    pub style: DockStyle,
    /// Standalone reveal behaviour (LayerShell style only).
    pub behavior: DockBehavior,
    /// Screen edge the standalone dock anchors to.
    pub position: DockPosition,
    /// App classes never shown in the dock (case-insensitive).
    pub ignore: Vec<String>,
    /// Spacing between dock items, px.
    pub spacing: u32,
    /// Show a small preview card on icon hover.
    pub hover_preview: bool,
    /// Show the separator between apps and the launcher button.
    pub separator: bool,
    /// Show the app-launcher button on the dock.
    pub launcher_enabled: bool,
    /// Launcher button icon (themed name or path).
    pub launcher_icon: String,
    /// Shell command the launcher button runs (empty = toggle mshell launcher).
    pub launcher_command: String,
}

impl Default for Dock {
    fn default() -> Self {
        Self {
            icon_size: 32,
            show_tooltips: true,
            show_running: true,
            icon_overrides: Vec::new(),
            in_bar: true,
            standalone: false,
            style: DockStyle::LayerShell,
            behavior: DockBehavior::AutoHide,
            position: DockPosition::Bottom,
            ignore: Vec::new(),
            spacing: 6,
            hover_preview: true,
            separator: true,
            launcher_enabled: true,
            launcher_icon: "view-app-grid-symbolic".to_string(),
            launcher_command: String::new(),
        }
    }
}

impl PatchField for DockStyle {
    fn patch_field(
        &mut self,
        new: Self,
        path: &StorePath,
        notify: &mut dyn FnMut(&StorePath),
        _keys: Option<&KeyMap>,
    ) {
        if *self != new {
            *self = new;
            notify(path);
        }
    }
}

impl PatchField for DockBehavior {
    fn patch_field(
        &mut self,
        new: Self,
        path: &StorePath,
        notify: &mut dyn FnMut(&StorePath),
        _keys: Option<&KeyMap>,
    ) {
        if *self != new {
            *self = new;
            notify(path);
        }
    }
}

impl PatchField for DockPosition {
    fn patch_field(
        &mut self,
        new: Self,
        path: &StorePath,
        notify: &mut dyn FnMut(&StorePath),
        _keys: Option<&KeyMap>,
    ) {
        if *self != new {
            *self = new;
            notify(path);
        }
    }
}

/// A single dock icon override (`class` → `icon`).
#[derive(
    Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize, Store, Patch, JsonSchema,
)]
#[serde(default)]
pub struct DockIconOverride {
    /// Window class / app_id to match (case-insensitive).
    pub class: String,
    /// Icon name (themed) or an absolute file path / `file://` URI.
    pub icon: String,
}

/// System Tray bar widget (the StatusNotifierItem icon strip). Tunables
/// surfaced under Settings → Widgets → System Tray.
#[derive(
    Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize, Store, Patch, JsonSchema,
)]
#[serde(default)]
pub struct SystemTray {
    /// Start with the tray icons revealed (expanded) instead of collapsed
    /// behind the tray button. The tray button still toggles them at runtime;
    /// this only sets the state the widget comes up in.
    pub default_expanded: bool,
}
