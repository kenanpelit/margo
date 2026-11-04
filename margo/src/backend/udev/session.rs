//! The DRM half of a libseat session pause / resume (VT switch, logind
//! making the session inactive).
//!
//! On pause the device must be told it is inactive and give up DRM master:
//! otherwise every repaint keeps committing to a CRTC another session now
//! owns (each commit fails, and the failures feed the render retry
//! backoff), and smithay never learns that the hardware state it cached can
//! be stale. On resume it re-takes master and becomes active again, after
//! which every `DrmCompositor` re-reads the CRTC state (see `mod.rs`).
//!
//! The sequencing sits behind [`SessionDrm`] so it is testable without a GPU.

use smithay::backend::drm::{DrmDevice, DrmError};
use tracing::warn;

/// The part of `DrmDevice` a session transition drives.
pub(super) trait SessionDrm {
    fn pause(&mut self);
    /// Re-take master and mark the device active. `false`: keep the
    /// connectors as they are (no blanking modeset), like niri and anvil.
    fn activate(&mut self) -> Result<(), DrmError>;
    fn is_active(&self) -> bool;
}

impl SessionDrm for DrmDevice {
    fn pause(&mut self) {
        DrmDevice::pause(self);
    }

    fn activate(&mut self) -> Result<(), DrmError> {
        DrmDevice::activate(self, false)
    }

    fn is_active(&self) -> bool {
        DrmDevice::is_active(self)
    }
}

/// Session went inactive: stop driving KMS and release DRM master.
pub(super) fn pause_drm(drm: &mut impl SessionDrm) {
    drm.pause();
}

/// Session active again: take the device back. A failure (master not
/// granted) is logged and the resume carries on, like niri: the outputs
/// still get their state reset so the next successful commit is a full one.
pub(super) fn activate_drm(drm: &mut impl SessionDrm) {
    if let Err(err) = drm.activate() {
        warn!("session resume: activating the DRM device failed: {err:?}");
    }
}

/// Whether a repaint pass may commit to KMS (frames, gamma, mode changes)
/// or rescan connectors into new outputs. Everything that does waits for
/// the resume while the session is paused; it is re-driven from there.
pub(super) fn kms_usable(drm: &impl SessionDrm) -> bool {
    drm.is_active()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Models smithay's `DrmDevice`: `pause` drops master and the active
    /// flag, `activate` takes master back (or fails to).
    #[derive(Default)]
    struct FakeDrm {
        active: bool,
        master: bool,
        grant_master: bool,
        calls: Vec<&'static str>,
    }

    impl SessionDrm for FakeDrm {
        fn pause(&mut self) {
            self.calls.push("pause");
            self.active = false;
            self.master = false;
        }

        fn activate(&mut self) -> Result<(), DrmError> {
            self.calls.push("activate");
            if !self.grant_master {
                return Err(DrmError::DrmMasterFailed);
            }
            self.master = true;
            self.active = true;
            Ok(())
        }

        fn is_active(&self) -> bool {
            self.active
        }
    }

    fn running() -> FakeDrm {
        FakeDrm {
            active: true,
            master: true,
            grant_master: true,
            ..FakeDrm::default()
        }
    }

    #[test]
    fn pause_releases_master_and_stops_kms_work() {
        let mut drm = running();
        assert!(kms_usable(&drm));
        pause_drm(&mut drm);
        assert_eq!(drm.calls, ["pause"]);
        assert!(!drm.master, "the next session must be able to take master");
        assert!(
            !kms_usable(&drm),
            "no commits while another session owns KMS"
        );
    }

    #[test]
    fn resume_takes_the_device_back() {
        let mut drm = running();
        pause_drm(&mut drm);
        activate_drm(&mut drm);
        assert_eq!(drm.calls, ["pause", "activate"]);
        assert!(drm.master);
        assert!(kms_usable(&drm));
    }

    #[test]
    fn a_refused_master_keeps_kms_work_off() {
        let mut drm = running();
        pause_drm(&mut drm);
        drm.grant_master = false;
        activate_drm(&mut drm);
        assert_eq!(drm.calls, ["pause", "activate"]);
        assert!(!kms_usable(&drm));
        // The next resume tries again.
        drm.grant_master = true;
        activate_drm(&mut drm);
        assert!(kms_usable(&drm));
    }
}
