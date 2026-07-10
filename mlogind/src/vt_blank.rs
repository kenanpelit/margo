//! Blank the login VT while a compositor takes the DRM master.
//!
//! Between `chvt` to the greeter's VT and the compositor's first modeset
//! (~1.5 s on this hardware), the kernel framebuffer console still owns the
//! screen — a black field with a blinking cursor, which reads as a flash of
//! "the console" before the graphical greeter appears. `margo` clears it only
//! once smithay opens the DRM node and the kernel hands the CRTC over; nothing
//! covers the gap before that.
//!
//! `KD_GRAPHICS` closes the gap: it tells the kernel to stop drawing the text
//! console, so the VT is already black when the compositor arrives and nothing
//! flashes. margo does not set it itself (it relies on the DRM-master handover),
//! so setting it here is additive — margo keeps whatever mode it finds, and the
//! mode is held for the whole graphical-host lifetime, so greeter↔session
//! handovers never flash the console either.
//!
//! [`graphics`] returns a guard whose fd stays open for the duration (the kernel
//! may revert `KD_GRAPHICS` on last close, so we do not close early) and whose
//! `Drop` restores `KD_TEXT`. That restore MUST happen before the TTY greeter
//! (`run_tty_host`) draws, or its text lands on a console the kernel is no
//! longer painting: an invisible, un-loginnable prompt — so the caller drops the
//! guard before falling through to the TTY host. Best-effort throughout: a login
//! screen never blocks on cosmetics, and a failed blank is a flash, not a lockout.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;

use log::warn;

// `ioctl`'s request argument is `c_ulong` on glibc and `c_int` on musl — the
// same split `chvt.rs` handles. The value fits either.
#[cfg(not(target_env = "musl"))]
const KDSETMODE: libc::c_ulong = 0x4B3A;
#[cfg(target_env = "musl")]
const KDSETMODE: libc::c_int = 0x4B3A;

const KD_TEXT: libc::c_int = 0x00;
const KD_GRAPHICS: libc::c_int = 0x01;

fn set_mode(file: &File, mode: libc::c_int, what: &str) {
    // SAFETY: `file` owns a valid fd for the VT; KDSETMODE takes its mode by value.
    if unsafe { libc::ioctl(file.as_raw_fd(), KDSETMODE, mode) } != 0 {
        warn!(
            "vt blank: KDSETMODE({what}) failed: {}",
            std::io::Error::last_os_error()
        );
    }
}

/// Holds the login VT in `KD_GRAPHICS` for as long as it lives; restores
/// `KD_TEXT` on drop. Keep it alive across the graphical host, and drop it
/// before any TTY rendering.
pub struct VtBlank(File);

impl Drop for VtBlank {
    fn drop(&mut self) {
        set_mode(&self.0, KD_TEXT, "restore");
    }
}

/// Stop the kernel text console on `tty` so the VT is black before the
/// compositor's first frame. `None` if the VT cannot be opened — then we simply
/// tolerate the flash rather than risk the login path. Drop the returned guard
/// (which restores text) before rendering the TTY greeter.
pub fn graphics(tty: u8) -> Option<VtBlank> {
    let path = format!("/dev/tty{tty}");
    let file = match OpenOptions::new().read(true).write(true).open(&path) {
        Ok(file) => file,
        Err(e) => {
            warn!("vt blank: cannot open {path} to blank it: {e}");
            return None;
        }
    };
    set_mode(&file, KD_GRAPHICS, "blank");
    Some(VtBlank(file))
}
