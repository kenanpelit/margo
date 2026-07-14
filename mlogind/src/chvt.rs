//! Adapted From https://github.com/jonay2000/chvt-rs

#[cfg(not(target_env = "musl"))]
type RequestType = libc::c_ulong;
#[cfg(target_env = "musl")]
type RequestType = libc::c_int;

use libc::c_int;
use nix::errno::Errno;
use nix::fcntl::{self, OFlag};
use nix::sys::stat::Mode;
use nix::unistd::close;
use std::error::Error;
use std::fmt::{self, Debug, Display, Formatter};

const VT_OPENQRY: RequestType = 0x5600;
const VT_ACTIVATE: RequestType = 0x5606;
const VT_WAITACTIVE: RequestType = 0x5607;

// Request Number to get Keyboard Type
const KDGKBTYPE: RequestType = 0x4B33;

const KB_101: u8 = 0x02;
const KB_84: u8 = 0x01;

#[derive(Debug)]
pub enum ChvtError {
    Activate,
    WaitActive,
    Close,
    OpenConsole,
    NotAConsole,
    GetFD,
    OpenQry,
}

impl Error for ChvtError {}
impl Display for ChvtError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        <dyn Debug>::fmt(self, f)
    }
}

fn is_a_console(fd: c_int) -> bool {
    let mut arg = 0;
    if unsafe { libc::ioctl(fd, KDGKBTYPE, &mut arg) } > 0 {
        return false;
    }

    (arg == KB_101) || (arg == KB_84)
}

fn open_a_console(filename: &str) -> Result<c_int, ChvtError> {
    for oflag in [OFlag::O_RDWR, OFlag::O_RDONLY, OFlag::O_WRONLY] {
        match fcntl::open(filename, oflag, Mode::empty()) {
            Ok(fd) => {
                if !is_a_console(fd) {
                    close(fd).map_err(|_| ChvtError::Close)?;
                    return Err(ChvtError::NotAConsole);
                }

                return Ok(fd);
            }
            Err(Errno::EACCES) => continue,
            _ => break,
        }
    }

    Err(ChvtError::OpenConsole)
}

fn get_fd() -> Result<c_int, ChvtError> {
    if let Ok(fd) = open_a_console("/dev/tty") {
        return Ok(fd);
    }

    if let Ok(fd) = open_a_console("/dev/tty") {
        return Ok(fd);
    }

    if let Ok(fd) = open_a_console("/dev/tty0") {
        return Ok(fd);
    }

    if let Ok(fd) = open_a_console("/dev/vc/0") {
        return Ok(fd);
    }

    if let Ok(fd) = open_a_console("/dev/console") {
        return Ok(fd);
    }

    for fd in 0..3 {
        if is_a_console(fd) {
            return Ok(fd);
        }
    }

    // If all attempts fail Error
    Err(ChvtError::GetFD)
}

pub unsafe fn chvt(ttynum: i32) -> Result<(), ChvtError> {
    let fd = get_fd()?;

    // `ioctl` reports errors as -1; the `> 0` checks this fork inherited were
    // dead code, silently swallowing every activate/wait failure.
    let activate = unsafe { libc::ioctl(fd, VT_ACTIVATE, ttynum as c_int) };
    if activate < 0 {
        let _ = close(fd);
        return Err(ChvtError::Activate);
    }

    let wait = unsafe { libc::ioctl(fd, VT_WAITACTIVE, ttynum) };
    if wait < 0 {
        let _ = close(fd);
        return Err(ChvtError::WaitActive);
    }

    close(fd).map_err(|_| ChvtError::Close)?;

    Ok(())
}

/// Ask the kernel for the first console with no process attached
/// (`VT_OPENQRY`) — the classic "find me a free VT" a display manager runs
/// before taking one over.
pub fn first_free_vt() -> Result<i32, ChvtError> {
    let fd = get_fd()?;
    let mut vt: c_int = -1;
    // SAFETY: `fd` is a console fd; VT_OPENQRY writes the offered VT number
    // through the pointer.
    let rc = unsafe { libc::ioctl(fd, VT_OPENQRY, &mut vt) };
    let _ = close(fd);
    // `vt == -1` is the kernel's own "no free VT" answer.
    if rc < 0 || vt <= 0 {
        return Err(ChvtError::OpenQry);
    }
    Ok(vt)
}
