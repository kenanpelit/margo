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
/// Returns a 1-element Vec on a normal session (focused monitor's
/// active tag) and an empty Vec if no monitor is currently focused
/// (transient: just after session lock unlocks, before the first
/// `wl_keyboard.enter`).
pub fn get_active_workspaces() -> Vec<WorkspaceInfo> {
    margo_service()
        .monitors
        .get()
        .iter()
        .find(|m| m.focused.get())
        .map(|m| vec![m.active_workspace.get()])
        .unwrap_or_default()
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
