//! Per-output lock surface, holding the lock_surface protocol object,
//! a shm pool for the buffer, and the current frame dimensions.

use anyhow::{Context, Result};
use std::os::fd::AsFd;
use wayland_client::{
    QueueHandle,
    protocol::{wl_output, wl_shm, wl_surface},
};
use wayland_protocols::ext::session_lock::v1::client::ext_session_lock_surface_v1;

use crate::seat::SeatState;
use crate::state::MlockState;

pub struct MlockSurface {
    /// Registry name of the `wl_output` global this surface covers.
    #[allow(dead_code)]
    pub global_name: u32,
    #[allow(dead_code)]
    pub output: wl_output::WlOutput,
    pub wl_surface: wl_surface::WlSurface,
    /// Dropping a wayland-client proxy sends nothing; the compositor only
    /// learns the output is no longer covered from `destroy()`.
    pub lock_surface: ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
    pub width: u32,
    pub height: u32,
    pub configured: bool,
    pub needs_redraw: bool,
}

impl MlockSurface {
    pub fn new(
        global_name: u32,
        output: wl_output::WlOutput,
        wl_surface: wl_surface::WlSurface,
        lock_surface: ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
    ) -> Self {
        Self {
            global_name,
            output,
            wl_surface,
            lock_surface,
            width: 0,
            height: 0,
            configured: false,
            needs_redraw: false,
        }
    }

    /// Tear the surface down when its output disappears. The protocol
    /// requires the lock surface role object to go before its wl_surface.
    pub fn destroy(self) {
        self.lock_surface.destroy();
        self.wl_surface.destroy();
    }

    /// Build a fresh ARGB8888 wl_shm buffer for this surface, paint
    /// the lock UI into it with cairo+pango, and attach+commit.
    pub fn render(
        &mut self,
        shm: &wl_shm::WlShm,
        qh: &QueueHandle<MlockState>,
        seat_state: &SeatState,
        user: &str,
        backdrop: Option<&crate::wallpaper::Backdrop>,
        avatar: Option<&image::RgbaImage>,
        accent: (f64, f64, f64),
        toggles: &crate::config::LockToggles,
        info: &crate::sidecar::LockInfo,
    ) -> Result<()> {
        if self.width == 0 || self.height == 0 {
            return Ok(());
        }
        let Some(BufferLayout {
            width,
            height,
            stride,
            len,
        }) = buffer_layout(self.width, self.height)
        else {
            anyhow::bail!(
                "lock surface {}x{} is too large for a wl_shm buffer",
                self.width,
                self.height
            );
        };

        // memfd-backed shm pool — `memfd_create` keeps the buffer
        // anonymous so nothing lands on disk.
        let fd = create_memfd("mlock-buf", len)?;

        // mmap into our process for cairo to draw into.
        let map_ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                std::os::fd::AsRawFd::as_raw_fd(&fd.as_fd()),
                0,
            )
        };
        if map_ptr == libc::MAP_FAILED {
            anyhow::bail!("mmap failed: {}", std::io::Error::last_os_error());
        }
        let pixels = unsafe { std::slice::from_raw_parts_mut(map_ptr as *mut u8, len) };

        // Draw via cairo. ImageSurface borrows the buffer; we flush
        // before unmap.
        crate::render::draw_lock_frame(
            pixels, width, height, stride, seat_state, user, backdrop, avatar, accent, toggles,
            info,
        )?;

        // Hand the buffer off to the compositor. wl_shm_pool needs an
        // OwnedFd; we kept `fd` until now.
        let pool = shm.create_pool(fd.as_fd(), len_i32(len), qh, ());
        let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, qh, ());
        // The pool can be released as soon as the buffer references it.
        pool.destroy();

        // Unmap our view — the compositor still owns its mapping
        // through the fd we passed.
        unsafe {
            libc::munmap(map_ptr, len);
        }

        // attach + damage + commit.
        self.wl_surface.attach(Some(&buffer), 0, 0);
        self.wl_surface.damage_buffer(0, 0, width, height);
        self.wl_surface.commit();

        self.needs_redraw = false;
        Ok(())
    }
}

/// Geometry of one ARGB8888 shm buffer, in the integer types the Wayland
/// and cairo calls take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BufferLayout {
    pub width: i32,
    pub height: i32,
    pub stride: i32,
    pub len: usize,
}

/// Size the buffer for a `width`×`height` configure, or `None` when it
/// can't be represented. The compositor picks the size and the protocol
/// sends it as u32, while wl_shm wants i32 width, height, stride and pool
/// size: the old `width as i32 * 4` and `stride * height` could wrap to a
/// negative or a too-small length for an absurd configure, and the mmap
/// and cairo would then disagree about the buffer size.
pub(crate) fn buffer_layout(width: u32, height: u32) -> Option<BufferLayout> {
    let w = i32::try_from(width).ok()?;
    let h = i32::try_from(height).ok()?;
    let stride = w.checked_mul(4)?;
    let len_i32 = stride.checked_mul(h)?;
    Some(BufferLayout {
        width: w,
        height: h,
        stride,
        len: usize::try_from(len_i32).ok()?,
    })
}

/// `len` came out of [`buffer_layout`], which proved it fits in an i32.
fn len_i32(len: usize) -> i32 {
    i32::try_from(len).unwrap_or(i32::MAX)
}

/// Anonymous shm fd via memfd_create. Falls back to memfd_create_syscall
/// if the libc wrapper is unavailable.
fn create_memfd(name: &str, size: usize) -> Result<std::os::fd::OwnedFd> {
    use std::os::fd::{FromRawFd, OwnedFd};
    let cname = std::ffi::CString::new(name).context("memfd name")?;
    // memfd_create(2) — kernel ≥ 3.17.
    let raw =
        unsafe { libc::memfd_create(cname.as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING) };
    if raw < 0 {
        anyhow::bail!("memfd_create: {}", std::io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let rc = unsafe { libc::ftruncate(std::os::fd::AsRawFd::as_raw_fd(&fd.as_fd()), size as i64) };
    if rc < 0 {
        anyhow::bail!("ftruncate: {}", std::io::Error::last_os_error());
    }
    Ok(fd)
}

#[cfg(test)]
mod tests {
    use super::{BufferLayout, buffer_layout};

    #[test]
    fn a_normal_output_gets_an_argb8888_layout() {
        assert_eq!(
            buffer_layout(1920, 1080),
            Some(BufferLayout {
                width: 1920,
                height: 1080,
                stride: 7680,
                len: 7680 * 1080,
            })
        );
    }

    #[test]
    fn sizes_that_overflow_i32_are_refused() {
        // stride * height wraps an i32.
        assert_eq!(buffer_layout(40_000, 40_000), None);
        // width * 4 wraps an i32.
        assert_eq!(buffer_layout(600_000_000, 1), None);
        // Not representable as i32 at all.
        assert_eq!(buffer_layout(u32::MAX, 1), None);
        assert_eq!(buffer_layout(1, u32::MAX), None);
    }

    #[test]
    fn the_largest_size_that_fits_is_accepted() {
        let l = buffer_layout(16_384, 32_767).expect("fits in i32");
        assert_eq!(l.len, 16_384 * 4 * 32_767);
    }
}
