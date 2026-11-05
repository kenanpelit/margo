//! DRM gamma: per-CRTC gamma properties (atomic GAMMA_LUT or the legacy
//! ioctl), the non-blocking commit flags and the unchanged-ramp check.

use std::{num::NonZeroU64, os::unix::io::AsFd};

use anyhow::{Context, Result};
use smithay::{
    backend::drm::DrmDevice,
    reexports::drm::control::{Device as DrmDeviceTrait, crtc, property},
};

/// Atomic gamma updates must never wait for a vblank on the event-loop
/// thread. Legacy drivers have no atomic ioctl and retain SETPROPERTY.
fn gamma_commit_flags(atomic: bool) -> Option<smithay::reexports::drm::control::AtomicCommitFlags> {
    atomic.then_some(smithay::reexports::drm::control::AtomicCommitFlags::NONBLOCK)
}

#[cfg(test)]
mod gamma_commit_tests {
    #[test]
    fn atomic_gamma_commits_are_nonblocking_and_have_no_flip_event() {
        use smithay::reexports::drm::control::AtomicCommitFlags as F;
        assert_eq!(super::gamma_commit_flags(true), Some(F::NONBLOCK));
        assert_eq!(super::gamma_commit_flags(false), None);
    }
}

// ── DRM gamma properties ──────────────────────────────────────────────────────
//
// Adapted from niri's `src/backend/tty.rs` GammaProps.

pub(in crate::backend) struct GammaProps {
    crtc: crtc::Handle,
    gamma_lut: property::Handle,
    gamma_lut_size: property::Handle,
    /// Currently-active LUT blob id, so we can free it when replacing.
    previous_blob: Option<NonZeroU64>,
    /// The ramp last committed (`Some(None)` = identity), `None` while
    /// unknown. Lets `set_gamma` skip a commit that would change nothing.
    applied: Option<Option<Vec<u16>>>,
    /// When the last GAMMA_LUT commit went in (rate limit, see
    /// `frame::GAMMA_MIN_INTERVAL`).
    pub(in crate::backend) last_commit: Option<std::time::Instant>,
    /// Deadline of the armed wake-up for a rate-limited ramp, if any.
    pub(in crate::backend) wake_at: Option<std::time::Instant>,
}

impl GammaProps {
    pub(in crate::backend) fn discover(device: &DrmDevice, crtc: crtc::Handle) -> Option<Self> {
        let props = device.get_properties(crtc).ok()?;
        let mut gamma_lut = None;
        let mut gamma_lut_size = None;
        for (prop, _) in props {
            let Ok(info) = device.get_property(prop) else {
                continue;
            };
            let Ok(name) = info.name().to_str() else {
                continue;
            };
            match name {
                "GAMMA_LUT" => {
                    if matches!(info.value_type(), property::ValueType::Blob) {
                        gamma_lut = Some(prop);
                    }
                }
                "GAMMA_LUT_SIZE" => {
                    if matches!(info.value_type(), property::ValueType::UnsignedRange(_, _)) {
                        gamma_lut_size = Some(prop);
                    }
                }
                _ => (),
            }
        }
        Some(Self {
            crtc,
            gamma_lut: gamma_lut?,
            gamma_lut_size: gamma_lut_size?,
            previous_blob: None,
            applied: None,
            last_commit: None,
            wake_at: None,
        })
    }

    pub(in crate::backend) fn gamma_size(&self, device: &DrmDevice) -> Option<u32> {
        let props = device.get_properties(self.crtc).ok()?;
        for (prop, value) in props {
            if prop == self.gamma_lut_size {
                // value is the raw u64 property value.
                return Some(value as u32);
            }
        }
        None
    }

    /// Forget what was committed and return it, so the next `set_gamma`
    /// commits even an unchanged ramp. Used when another DRM master may
    /// have changed the LUT (a VT switch).
    pub(in crate::backend) fn forget_applied(&mut self) -> Option<Option<Vec<u16>>> {
        self.applied.take()
    }

    /// Apply a gamma ramp (R, G, B planes concatenated, each `gamma_size`
    /// u16 entries). Pass `None` to clear/restore the default identity LUT.
    ///
    /// Atomic drivers use a NONBLOCK commit with no flip event: the event
    /// loop never waits for the LUT to latch. Legacy drivers keep the
    /// property write. An unchanged ramp is skipped (`Ok(false)`);
    /// `Ok(true)` means the commit was accepted.
    pub(in crate::backend) fn set_gamma(
        &mut self,
        device: &DrmDevice,
        gamma: Option<&[u16]>,
    ) -> Result<bool> {
        if gamma_unchanged(self.applied.as_ref(), gamma) {
            return Ok(false);
        }
        // Unknown until the commit below succeeds.
        self.applied = None;
        #[allow(non_camel_case_types)]
        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct drm_color_lut {
            red: u16,
            green: u16,
            blue: u16,
            reserved: u16,
        }

        let blob_id: Option<NonZeroU64> = if let Some(gamma) = gamma {
            let n = self.gamma_size(device).context("gamma_size unreadable")? as usize;
            anyhow::ensure!(
                gamma.len() == n * 3,
                "wrong gamma length: got {}, expected {}",
                gamma.len(),
                n * 3
            );
            // wlr-gamma-control orders the ramp as R, G, B planes (per niri's
            // observation; some implementations swap G/B but this matches what
            // sunsetr / gammastep send).
            let (red, rest) = gamma.split_at(n);
            let (green, blue) = rest.split_at(n);
            let mut data: Vec<drm_color_lut> = red
                .iter()
                .zip(green.iter())
                .zip(blue.iter())
                .map(|((&r, &g), &b)| drm_color_lut {
                    red: r,
                    green: g,
                    blue: b,
                    reserved: 0,
                })
                .collect();
            let bytes = bytemuck::cast_slice_mut::<drm_color_lut, u8>(&mut data);
            let blob = drm_ffi::mode::create_property_blob(device.as_fd(), bytes)
                .map_err(|e| anyhow::anyhow!("create_property_blob: {e}"))?;
            NonZeroU64::new(u64::from(blob.blob_id))
        } else {
            None
        };

        let blob_value = blob_id.map(NonZeroU64::get).unwrap_or(0);
        let raw: property::RawValue = property::Value::Blob(blob_value).into();
        let commit = if let Some(flags) = gamma_commit_flags(device.is_atomic()) {
            let mut request = smithay::reexports::drm::control::atomic::AtomicModeReq::new();
            request.add_property(self.crtc, self.gamma_lut, property::Value::Blob(blob_value));
            // No PAGE_FLIP_EVENT: this update is not a compositor frame,
            // so it must not consume frame_submitted / presentation feedback.
            // If queue_frame races this update, its ordinary EBUSY retry
            // path will re-render once the LUT has latched.
            device.atomic_commit(flags, request)
        } else {
            device.set_property(self.crtc, self.gamma_lut, raw)
        };
        if let Err(e) = commit {
            // The blob created above was never attached: free it, or
            // every failed (retried) commit would leak one.
            if let Some(blob) = blob_id {
                let _ = device.destroy_property_blob(blob.get());
            }
            return Err(anyhow::Error::new(e).context("set GAMMA_LUT property"));
        }
        self.last_commit = Some(std::time::Instant::now());

        if let Some(old) = std::mem::replace(&mut self.previous_blob, blob_id) {
            let _ = device.destroy_property_blob(old.get());
        }
        self.applied = Some(gamma.map(<[u16]>::to_vec));
        Ok(true)
    }
}

/// Whether committing `new` would leave GAMMA_LUT as it is. `applied` is the
/// last ramp committed (`Some(None)` = the identity), `None` when unknown.
fn gamma_unchanged(applied: Option<&Option<Vec<u16>>>, new: Option<&[u16]>) -> bool {
    applied.is_some_and(|applied| applied.as_deref() == new)
}

#[cfg(test)]
mod tests {
    use super::gamma_unchanged;

    #[test]
    fn the_same_ramp_is_not_committed_twice() {
        let ramp = vec![1u16, 2, 3];
        assert!(gamma_unchanged(Some(&Some(ramp.clone())), Some(&ramp)));
        assert!(gamma_unchanged(Some(&None), None));
    }

    #[test]
    fn a_different_or_unknown_ramp_is_committed() {
        let ramp = vec![1u16, 2, 3];
        assert!(!gamma_unchanged(
            Some(&Some(ramp.clone())),
            Some(&[1, 2, 4])
        ));
        assert!(!gamma_unchanged(Some(&Some(ramp.clone())), None));
        assert!(!gamma_unchanged(Some(&None), Some(&ramp)));
        // Nothing known about the CRTC (startup, after a VT switch).
        assert!(!gamma_unchanged(None, None));
        assert!(!gamma_unchanged(None, Some(&ramp)));
    }
}
