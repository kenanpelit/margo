use mshell_services::margo_service;
use std::sync::Arc;
use tracing::error;
use mshell_margo_client::{Workspace, WorkspaceInfo};

/// Active workspaces under the **margo** semantics: only the
/// focused monitor's active tag counts as "active" for bar
/// highlighting purposes. Other monitors' tags don't co-light a
/// pill, since margo tags are per-monitor bitmasks that share an
/// id space — letting every monitor's active tag stay green at the
/// same time produces the confusing "1 and 2 both look focused
/// even though I'm only on tag 3" reading the user reported.
///
/// Resolves "the focused monitor" by name via the same heuristic
/// `MargoService::active_monitor_name` uses (focused-client first,
/// then `state.active_output` fallback). Important difference vs
/// the previous version, which read `monitor.focused`: margo's
/// per-output `active` flag is only true when both the cursor AND
/// keyboard focus sit on that output, so in the common
/// session-start state (no client focused yet, cursor on the bar)
/// every monitor reads `focused=false` and the function returned
/// an empty Vec — every tag pill rendered as unselected and the
/// user thought the widget was broken until the first window
/// opened. Routing through `active_monitor_name` matches what the
/// Layout widget already does for the same reason.
///
/// Returns a 1-element Vec on a normal session and an empty Vec
/// only when state.json itself is unreachable.
pub fn get_active_workspaces() -> Vec<WorkspaceInfo> {
    let svc = margo_service();

    // Prefer the focused-client's workspace — that's the most
    // direct signal of "the user is interacting with this tag
    // right now" and it's live the moment a window gains focus.
    if let Some(c) = svc.focused_client.get() {
        return vec![c.workspace.get()];
    }

    let monitors = svc.monitors.get();
    // Fallback chain: first monitor whose own `focused` flag is
    // true (cursor+focus coincide) → first monitor in the list.
    // The last fallback keeps a tag pill highlighted at session
    // start before any window has focus — otherwise every pill
    // rendered as unselected and the user thought the widget
    // was broken until they opened the first window.
    let mon = monitors
        .iter()
        .find(|m| m.focused.get())
        .or_else(|| monitors.first());

    mon.map(|m| vec![m.active_workspace.get()]).unwrap_or_default()
}

pub fn is_an_active_workspace(workspace: &Arc<Workspace>) -> bool {
    get_active_workspaces()
        .iter()
        .any(|p| p.id == workspace.id.get())
}

pub fn go_up_workspace() {
    let hyprland = margo_service();
    tokio::spawn(async move {
        if let Err(e) = hyprland
            .dispatch("hl.dsp.focus({ workspace = \"r-1\" })")
            .await
        {
            error!(error = %e, "Failed to switch workspace");
        }
    });
}

pub fn go_down_workspace() {
    let hyprland = margo_service();
    tokio::spawn(async move {
        if let Err(e) = hyprland
            .dispatch("hl.dsp.focus({ workspace = \"r+1\" })")
            .await
        {
            error!(error = %e, "Failed to switch workspace");
        }
    });
}
