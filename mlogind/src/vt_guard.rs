//! Hold the login VT for a graphical greeter host: stop the kernel text
//! console from **drawing** and from **reading keys**, restoring both on drop.
//!
//! **Drawing (`KD_GRAPHICS`).** Between `chvt` to the greeter's VT and the
//! compositor's first modeset (~1.5 s on this hardware), fbcon still owns the
//! screen — a black field with a blinking cursor, which reads as a flash of
//! "the console" before the graphical greeter appears. `KD_GRAPHICS` tells
//! the kernel to stop drawing the text console, so the VT is already black
//! when the compositor arrives; margo does not set the mode itself, so the
//! hold is additive, and it spans the whole graphical-host lifetime so
//! greeter↔session handovers never flash either.
//!
//! **Typing (`KDSKBMODE K_OFF`, phase B).** The compositor reads input
//! through evdev, but the kernel keyboard driver *also* cooks every key
//! pressed on the active VT into the tty's input buffer. Anything typed
//! before the compositor's first frame — half a password and an Enter, say —
//! accumulates there invisibly and replays into the next thing that reads the
//! VT: the TTY-greeter fallback, a getty, a root shell on that console.
//! `K_OFF` turns the duplicate stream off. The previous mode is snapshotted
//! with `KDGKBMODE` first and restored on drop; **a mode we cannot snapshot
//! is a mode we must not set** — a VT with an unrestorable dead keyboard is a
//! lockout, the one thing a login manager may never risk — so a failed
//! snapshot skips suppression and keeps only the blank.
//!
//! Guard semantics, unchanged from the blank-only days: the fd stays open for
//! the guard's lifetime (the kernel may revert modes on last close), `Drop`
//! restores the keyboard first and `KD_TEXT` second, and the caller MUST drop
//! the guard before the TTY greeter draws — its prompt on a blanked, key-less
//! console would be invisible and un-loginnable. Best-effort throughout: a
//! failed hold is a cosmetic flash or a harmless duplicate input stream,
//! never a blocked login.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;

use log::warn;

// `ioctl`'s request argument is `c_ulong` on glibc and `c_int` on musl — the
// same split `chvt.rs` handles. The values fit either.
#[cfg(not(target_env = "musl"))]
type RequestType = libc::c_ulong;
#[cfg(target_env = "musl")]
type RequestType = libc::c_int;

const KDSETMODE: RequestType = 0x4B3A;
const KD_TEXT: libc::c_int = 0x00;
const KD_GRAPHICS: libc::c_int = 0x01;

const KDGKBMODE: RequestType = 0x4B44;
const KDSKBMODE: RequestType = 0x4B45;
const K_UNICODE: libc::c_int = 0x03;
const K_OFF: libc::c_int = 0x04;

fn set_mode(file: &File, mode: libc::c_int, what: &str) {
    // SAFETY: `file` owns a valid fd for the VT; KDSETMODE takes its mode by value.
    if unsafe { libc::ioctl(file.as_raw_fd(), KDSETMODE, mode) } != 0 {
        warn!(
            "vt guard: KDSETMODE({what}) failed: {}",
            std::io::Error::last_os_error()
        );
    }
}

fn set_kb_mode(file: &File, mode: libc::c_int, what: &str) {
    // SAFETY: `file` owns a valid fd for the VT; KDSKBMODE takes its mode by value.
    if unsafe { libc::ioctl(file.as_raw_fd(), KDSKBMODE, mode) } != 0 {
        warn!(
            "vt guard: KDSKBMODE({what}) failed: {}",
            std::io::Error::last_os_error()
        );
    }
}

/// Turn the VT keyboard off, returning the mode to restore on drop.
fn keyboard_off(file: &File) -> Option<libc::c_int> {
    let mut current: libc::c_int = 0;
    // SAFETY: `file` owns a valid fd for the VT; KDGKBMODE writes the current
    // mode through the pointer.
    if unsafe { libc::ioctl(file.as_raw_fd(), KDGKBMODE, &mut current) } != 0 {
        warn!(
            "vt guard: KDGKBMODE failed ({}); leaving the VT keyboard on",
            std::io::Error::last_os_error()
        );
        return None;
    }
    // A previous holder that died un-restored would make us "restore" K_OFF —
    // a dead keyboard, faithfully preserved. Snap that to the modern default.
    let restore_to = if current == K_OFF { K_UNICODE } else { current };
    set_kb_mode(file, K_OFF, "suppress");
    Some(restore_to)
}

/// Holds the login VT blanked (`KD_GRAPHICS`) and key-less (`K_OFF`) for as
/// long as it lives; restores the keyboard mode and `KD_TEXT` on drop. Keep it
/// alive across the graphical host, and drop it before any TTY rendering.
pub struct VtGuard {
    file: File,
    /// The keyboard mode to restore, when the snapshot succeeded.
    kb_mode: Option<libc::c_int>,
}

impl Drop for VtGuard {
    fn drop(&mut self) {
        if let Some(mode) = self.kb_mode {
            set_kb_mode(&self.file, mode, "restore");
        }
        set_mode(&self.file, KD_TEXT, "restore");
    }
}

/// Take the graphical hold on `tty`: stop the kernel text console drawing and
/// reading keys before the compositor's first frame. `None` if the VT cannot
/// be opened — then we simply tolerate the flash (and the duplicate key
/// stream) rather than risk the login path. Drop the returned guard before
/// rendering the TTY greeter.
pub fn graphics(tty: u8) -> Option<VtGuard> {
    let path = format!("/dev/tty{tty}");
    let file = match OpenOptions::new().read(true).write(true).open(&path) {
        Ok(file) => file,
        Err(e) => {
            warn!("vt guard: cannot open {path} to hold it: {e}");
            return None;
        }
    };
    set_mode(&file, KD_GRAPHICS, "blank");
    let kb_mode = keyboard_off(&file);
    Some(VtGuard { file, kb_mode })
}
