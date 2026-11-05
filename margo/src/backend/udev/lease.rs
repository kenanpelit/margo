//! wp_drm_lease_device_v1: lease connectors (with a CRTC and primary
//! plane) to clients such as VR compositors.

use anyhow::Result;
use smithay::{
    backend::drm::DrmNode,
    reexports::drm::control::{Device as DrmDeviceTrait, crtc},
};
use tracing::{info, warn};

use crate::state::MargoState;

// ── wp_drm_lease_device_v1 handler ───────────────────────────────────────────
//
// Lives here (not in state/handlers/) because `lease_request` must
// synchronously reach the live `DrmDevice` inside `BackendData` —
// the one thing `MargoState` holds an `Rc` handle to precisely for
// this protocol. Resources leased per requested connector: the
// connector itself, a free compatible CRTC, and its primary plane.

use crate::backend::udev::find_crtc;
use smithay::wayland::drm_lease::{
    DrmLease, DrmLeaseBuilder, DrmLeaseHandler, DrmLeaseRequest, DrmLeaseState, LeaseRejected,
};

impl DrmLeaseHandler for MargoState {
    fn drm_lease_state(&mut self, _node: DrmNode) -> &mut DrmLeaseState {
        self.protocols
            .drm_lease
            .as_mut()
            .expect("drm_lease global is only registered by the udev backend")
    }

    fn lease_request(
        &mut self,
        _node: DrmNode,
        request: DrmLeaseRequest,
    ) -> Result<DrmLeaseBuilder, LeaseRejected> {
        let bd_rc = self
            .udev_backend
            .clone()
            .ok_or_else(LeaseRejected::default)?;
        let bd = bd_rc.borrow();
        let resources = bd
            .drm
            .resource_handles()
            .map_err(LeaseRejected::with_cause)?;
        // CRTCs driving real outputs are off the table; each granted
        // connector reserves one more within this request.
        let mut used: std::collections::HashSet<crtc::Handle> =
            bd.outputs.keys().copied().collect();
        let mut builder = DrmLeaseBuilder::new(&bd.drm);
        for conn in request.connectors {
            if !bd.non_desktop_connectors.contains(&conn) {
                warn!("lease request for a connector that is not non-desktop; rejecting");
                return Err(LeaseRejected::default());
            }
            let conn_info = bd
                .drm
                .get_connector(conn, false)
                .map_err(LeaseRejected::with_cause)?;
            let crtc = find_crtc(bd.drm.device_fd(), &conn_info, &resources, &used)
                .ok_or_else(LeaseRejected::default)?;
            used.insert(crtc);
            builder.add_connector(conn);
            builder.add_crtc(crtc);
            let planes = bd.drm.planes(&crtc).map_err(LeaseRejected::with_cause)?;
            let (plane, claim) = planes
                .primary
                .iter()
                .find_map(|p| bd.drm.claim_plane(p.handle, crtc).map(|c| (p, c)))
                .ok_or_else(LeaseRejected::default)?;
            builder.add_plane(plane.handle, claim);
        }
        Ok(builder)
    }

    fn new_active_lease(&mut self, _node: DrmNode, lease: DrmLease) {
        info!(lease = lease.id(), "DRM lease activated");
        self.drm_lease_active.push(lease);
    }

    fn lease_destroyed(&mut self, _node: DrmNode, lease_id: u32) {
        info!(lease = lease_id, "DRM lease destroyed");
        self.drm_lease_active.retain(|l| l.id() != lease_id);
    }
}
// Dispatch wiring arrives through the blanket `delegate_dispatch2!`
// in state.rs — this smithay revision has no per-protocol macro.
