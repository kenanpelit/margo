//! Compositor-facing types for margo-shell.
//!
//! margo-shell only ever talks to one compositor — margo — so the
//! original ashell multi-backend enum has been collapsed. There is
//! no `CompositorChoice` and no `ActiveWindow::{Hyprland,Niri}`; a
//! single plain struct carries the focused-window info and the
//! state struct holds the workspace/monitor snapshot that the
//! `workspaces` and `window_title` modules render.

/// One margo tag rendered as an ashell-style workspace cell. margo
/// is tag-based (1..=9 per monitor), so each tag bit becomes one
/// workspace entry; the active tagset determines which is active /
/// visible / hidden.
#[derive(Debug, Clone, PartialEq)]
pub struct CompositorWorkspace {
    pub id: i32,
    pub index: i32,
    pub name: String,
    pub monitor: String,
    pub monitor_id: Option<i128>,
    pub windows: u16,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompositorMonitor {
    pub id: i128,
    pub name: String,
    /// Lowest set bit in the monitor's active tagmask, expressed as
    /// 1..=9. 0 means "no tag visible" (rare — empty tagset).
    pub active_workspace_id: i32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ActiveWindow {
    pub title: String,
    pub class: String,
    pub address: String,
}

#[derive(Debug, Clone, Default)]
pub struct CompositorState {
    pub workspaces: Vec<CompositorWorkspace>,
    pub monitors: Vec<CompositorMonitor>,
    pub active_workspace_id: Option<i32>,
    pub active_window: Option<ActiveWindow>,
    pub keyboard_layout: String,
}

#[derive(Debug, Clone)]
pub struct CompositorService {
    pub state: CompositorState,
}

#[derive(Debug, Clone)]
pub enum CompositorEvent {
    /// Acknowledge that a command was dispatched. No state-change
    /// data; the next `StateChanged` (driven by the state.json
    /// inotify loop) carries the resulting world.
    ActionPerformed,
    StateChanged(Box<CompositorState>),
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum CompositorCommand {
    /// Switch the active monitor to a specific tag (1..=9).
    FocusWorkspace(i32),
    /// Scroll the active tagset along the 1..=9 ring by `±1`.
    ScrollWorkspace(i32),
    /// Cycle to the next layout on the focused tag.
    NextLayout,
    /// Forward a raw dispatch action to mctl. `(action, arg)` —
    /// arg is forwarded as the string slot (slot 4 in margo's
    /// dispatch ABI), suitable for `spawn` and friends.
    CustomDispatch(String, String),
}
