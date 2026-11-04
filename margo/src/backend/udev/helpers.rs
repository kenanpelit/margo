//! Small standalone helpers extracted from `udev/mod.rs` during the
//! W4.1 split. No state, no MargoState reach — pure utilities that
//! every udev sub-module can depend on.

use smithay::{
    backend::drm::DrmDeviceFd,
    output::Output,
    reexports::drm::control::{connector, crtc},
    utils::Transform,
};

/// Map margo's integer transform-id (parsed from the config file —
/// matches the dwl convention) to smithay's `Transform` enum.
pub(super) fn smithay_transform(n: i32) -> Transform {
    match n {
        1 => Transform::_90,
        2 => Transform::_180,
        3 => Transform::_270,
        4 => Transform::Flipped,
        5 => Transform::Flipped90,
        6 => Transform::Flipped180,
        7 => Transform::Flipped270,
        _ => Transform::Normal,
    }
}

/// Pick the first CRTC compatible with `conn` that isn't already in
/// `used_crtcs`. Returns `None` when every encoder's CRTC bitmask
/// collides with the in-use set — caller treats that as a hotplug
/// race and skips the connector for this pass.
pub(super) fn find_crtc(
    drm: &DrmDeviceFd,
    conn: &connector::Info,
    resources: &smithay::reexports::drm::control::ResourceHandles,
    used_crtcs: &std::collections::HashSet<crtc::Handle>,
) -> Option<crtc::Handle> {
    use smithay::reexports::drm::control::Device as _;
    for enc_handle in conn.encoders() {
        let Ok(enc) = drm.get_encoder(*enc_handle) else {
            continue;
        };
        for c in resources.filter_crtcs(enc.possible_crtcs()) {
            if !used_crtcs.contains(&c) {
                return Some(c);
            }
        }
    }
    None
}

/// Whether a connector carries the DRM "non-desktop" property (VR
/// headsets and similar) — margo never drives those as outputs and
/// offers them for `wp_drm_lease_device_v1` instead.
pub(super) fn connector_is_non_desktop(drm: &DrmDeviceFd, conn: connector::Handle) -> bool {
    use smithay::reexports::drm::control::Device as _;
    let Ok(props) = drm.get_properties(conn) else {
        return false;
    };
    for (prop, value) in props.iter() {
        let Ok(info) = drm.get_property(*prop) else {
            continue;
        };
        if info.name().to_str() == Ok("non-desktop") {
            return info
                .value_type()
                .convert_value(*value)
                .as_boolean()
                .unwrap_or(false);
        }
    }
    false
}

/// Frame interval derived from the output's current mode. Falls
/// back to 60 Hz when the mode is unset or reports a zero refresh
/// rate (some virtual outputs).
pub(super) fn output_refresh_duration(output: &Output) -> std::time::Duration {
    output
        .current_mode()
        .map(|m| {
            // mode.refresh is in mHz.
            let hz = (m.refresh as f64) / 1000.0;
            if hz > 0.0 {
                std::time::Duration::from_secs_f64(1.0 / hz)
            } else {
                std::time::Duration::from_secs_f64(1.0 / 60.0)
            }
        })
        .unwrap_or_else(|| std::time::Duration::from_secs_f64(1.0 / 60.0))
}

/// Real CLOCK_MONOTONIC time as a `Duration`. Used as the `now`
/// argument for `wp_presentation_feedback.presented` and wlr-screencopy
/// `ready`. These timestamps MUST be in CLOCK_MONOTONIC — the same clock
/// margo advertises (`PresentationState::new`) and that clients read via
/// `clock_gettime` — or A/V-sync consumers (mpv --vo=gpu-next) see the
/// presentation time offset by the machine's uptime-at-launch. The old
/// process-start `Instant` epoch broke that; delegate to the shared
/// clock helper pw_utils already uses.
pub(super) fn monotonic_now() -> std::time::Duration {
    crate::utils::get_monotonic_time()
}

/// Copy a tightly packed frame (`rows` rows of `row_bytes` bytes each) into a
/// `wl_shm` buffer that lives somewhere inside its pool.
///
/// `smithay::wayland::shm::with_buffer_contents_mut` hands out the *pool's*
/// pointer and length; the buffer starts at `buffer.offset` and its rows are
/// `buffer.stride` bytes apart. Returns `true` only when the whole frame was
/// written; a buffer that does not fit inside the pool, or a frame shorter
/// than the buffer, writes nothing.
///
/// # Safety
/// `pool_ptr` must be valid for writes of `pool_len` bytes for the duration of
/// the call and must not overlap `pixels`.
pub(super) unsafe fn copy_frame_into_shm_pool(
    pool_ptr: *mut u8,
    pool_len: usize,
    buffer: &smithay::wayland::shm::BufferData,
    row_bytes: usize,
    rows: usize,
    pixels: &[u8],
) -> bool {
    let (Ok(offset), Ok(stride)) = (
        usize::try_from(buffer.offset),
        usize::try_from(buffer.stride),
    ) else {
        return false;
    };
    if rows == 0 || row_bytes == 0 || stride < row_bytes {
        return false;
    }
    let Some(frame_len) = row_bytes.checked_mul(rows) else {
        return false;
    };
    // The last row only needs `row_bytes`, not a full stride.
    let fits = stride
        .checked_mul(rows - 1)
        .and_then(|n| n.checked_add(row_bytes))
        .and_then(|n| n.checked_add(offset))
        .is_some_and(|end| end <= pool_len);
    if !fits || pixels.len() < frame_len {
        return false;
    }
    if stride == row_bytes {
        // SAFETY: `offset + frame_len <= pool_len` (checked above) and
        // `frame_len <= pixels.len()`; the caller guarantees the regions don't
        // overlap.
        unsafe { std::ptr::copy_nonoverlapping(pixels.as_ptr(), pool_ptr.add(offset), frame_len) };
    } else {
        for (row, src) in pixels[..frame_len].chunks_exact(row_bytes).enumerate() {
            // SAFETY: `offset + row * stride + row_bytes <= pool_len` for every
            // `row < rows` (checked above).
            unsafe {
                std::ptr::copy_nonoverlapping(
                    src.as_ptr(),
                    pool_ptr.add(offset + row * stride),
                    row_bytes,
                );
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::copy_frame_into_shm_pool;
    use smithay::reexports::wayland_server::protocol::wl_shm::Format;
    use smithay::wayland::shm::BufferData;

    fn buffer(offset: i32, w: i32, h: i32, stride: i32) -> BufferData {
        BufferData {
            offset,
            width: w,
            height: h,
            stride,
            format: Format::Xrgb8888,
        }
    }

    fn copy(pool: &mut [u8], b: &BufferData, pixels: &[u8]) -> bool {
        let row_bytes = b.width as usize * 4;
        // SAFETY: `pool` is a plain Vec-backed slice, valid for its length.
        unsafe {
            copy_frame_into_shm_pool(
                pool.as_mut_ptr(),
                pool.len(),
                b,
                row_bytes,
                b.height as usize,
                pixels,
            )
        }
    }

    #[test]
    fn writes_at_the_buffer_offset_and_leaves_the_rest_of_the_pool_alone() {
        // Two 2x2 XRGB buffers sub-allocated from one 32-byte pool; the
        // target is the second one (offset 16).
        let mut pool = vec![0xAAu8; 32];
        let b = buffer(16, 2, 2, 8);
        let pixels: Vec<u8> = (1..=16).collect();
        assert!(copy(&mut pool, &b, &pixels));
        assert_eq!(&pool[..16], &[0xAA; 16], "the other buffer was overwritten");
        assert_eq!(&pool[16..], &pixels[..]);
    }

    #[test]
    fn honours_a_stride_wider_than_the_row() {
        let mut pool = vec![0u8; 4 + 2 * 12];
        let b = buffer(4, 2, 2, 12);
        let pixels: Vec<u8> = (1..=16).collect();
        assert!(copy(&mut pool, &b, &pixels));
        assert_eq!(&pool[4..12], &pixels[..8]);
        assert_eq!(&pool[12..16], &[0; 4], "row padding must stay untouched");
        assert_eq!(&pool[16..24], &pixels[8..]);
    }

    #[test]
    fn a_buffer_running_past_the_pool_writes_nothing() {
        let mut pool = vec![0u8; 20];
        let b = buffer(16, 2, 2, 8);
        let pixels = vec![7u8; 16];
        assert!(!copy(&mut pool, &b, &pixels));
        assert!(pool.iter().all(|&x| x == 0));
    }

    #[test]
    fn a_short_frame_writes_nothing() {
        let mut pool = vec![0u8; 16];
        let b = buffer(0, 2, 2, 8);
        assert!(!copy(&mut pool, &b, &[9u8; 15]));
        assert!(pool.iter().all(|&x| x == 0));
    }

    #[test]
    fn a_negative_offset_or_stride_writes_nothing() {
        let mut pool = vec![0u8; 32];
        let pixels = vec![7u8; 16];
        assert!(!copy(&mut pool, &buffer(-4, 2, 2, 8), &pixels));
        assert!(!copy(&mut pool, &buffer(0, 2, 2, -8), &pixels));
        assert!(pool.iter().all(|&x| x == 0));
    }
}
