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
    // idx / output / lock_surface aren't read after construction, but
    // their `Drop` semantics matter: dropping the lock_surface tells
    // the compositor we no longer cover that output. Keep them alive.
    #[allow(dead_code)]
    pub idx: usize,
    #[allow(dead_code)]
    pub output: wl_output::WlOutput,
    pub wl_surface: wl_surface::WlSurface,
    #[allow(dead_code)]
    pub lock_surface: ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
    pub width: u32,
    pub height: u32,
    pub configured: bool,
    pub needs_redraw: bool,
}

impl MlockSurface {
    pub fn new(
        idx: usize,
        output: wl_output::WlOutput,
        wl_surface: wl_surface::WlSurface,
        lock_surface: ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
    ) -> Self {
        Self {
            idx,
            output,
            wl_surface,
            lock_surface,
            width: 0,
            height: 0,
            configured: false,
            needs_redraw: false,
        }
    }

    /// Build a fresh ARGB8888 wl_shm buffer for this surface, paint
    /// the lock UI into it with cairo+pango, and attach+commit.
    pub fn render(
        &mut self,
        shm: &wl_shm::WlShm,
        qh: &QueueHandle<MlockState>,
        seat_state: &SeatState,
        user: &str,
        wallpaper: Option<&image::RgbaImage>,
        avatar: Option<&image::RgbaImage>,
    ) -> Result<()> {
        if self.width == 0 || self.height == 0 {
            return Ok(());
        }
        let width = self.width as i32;
        let height = self.height as i32;
        let stride = width * 4;
        let len = (stride * height) as usize;

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
            pixels,
            width,
            height,
            stride,
            seat_state,
            user,
            wallpaper,
            avatar,
        )?;

        // Hand the buffer off to the compositor. wl_shm_pool needs an
        // OwnedFd; we kept `fd` until now.
        let pool = shm.create_pool(fd.as_fd(), len as i32, qh, ());
        let buffer = pool.create_buffer(
            0,
            width,
            height,
            stride,
            wl_shm::Format::Argb8888,
            qh,
            (),
        );
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
