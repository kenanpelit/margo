//! Where frames actually go.
//!
//! The runner and its greeter share a `SOCK_SEQPACKET` pair, so the transport
//! is message-oriented: one `send` is one frame, one `recv` is one frame. That
//! is the whole reason for choosing `SOCK_SEQPACKET` over `SOCK_STREAM` — no
//! reassembly buffer, no partial-read state machine in the login path.

use std::collections::VecDeque;
use std::os::fd::RawFd;

use crate::{MAX_FRAME, ProtoError};

/// A message-oriented byte channel.
///
/// Behind a trait so the conversation engine can be driven from a `Vec` in unit
/// tests — the same seam `mgreet` already uses for `trait Authenticate`.
pub trait Transport {
    /// Send exactly one frame.
    fn send(&mut self, frame: &[u8]) -> Result<(), ProtoError>;

    /// Receive exactly one frame into `buf`, which the caller has cleared.
    /// Returns the frame length, or `0` for a clean EOF.
    fn recv(&mut self, buf: &mut Vec<u8>) -> Result<usize, ProtoError>;
}

/// A [`Transport`] over a borrowed socket fd.
///
/// Borrowed, not owned: the runner keeps the `OwnedFd` so it can hand a failed
/// `Authenticator` to `Drop` and open a fresh `pam_start` on the same socket.
/// `Authenticator::with_handler` takes its `Converse` by value, which would
/// otherwise swallow the fd on the first failed login attempt.
pub struct FdTransport {
    fd: RawFd,
}

impl FdTransport {
    /// # Safety
    /// `fd` must be a valid `SOCK_SEQPACKET` socket that outlives this value.
    pub unsafe fn new(fd: RawFd) -> Self {
        Self { fd }
    }
}

impl Transport for FdTransport {
    fn send(&mut self, frame: &[u8]) -> Result<(), ProtoError> {
        loop {
            // MSG_NOSIGNAL: a greeter that died mid-conversation must give us
            // EPIPE to handle, not SIGPIPE to die from.
            let n = unsafe {
                libc::send(
                    self.fd,
                    frame.as_ptr().cast(),
                    frame.len(),
                    libc::MSG_NOSIGNAL,
                )
            };
            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(ProtoError::Io(err));
            }
            // SOCK_SEQPACKET is all-or-nothing; a short send cannot happen.
            debug_assert_eq!(n as usize, frame.len());
            return Ok(());
        }
    }

    fn recv(&mut self, buf: &mut Vec<u8>) -> Result<usize, ProtoError> {
        buf.clear();
        buf.resize(MAX_FRAME, 0);
        loop {
            let n = unsafe { libc::recv(self.fd, buf.as_mut_ptr().cast(), MAX_FRAME, 0) };
            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                buf.clear();
                return Err(ProtoError::Io(err));
            }
            // A datagram larger than MAX_FRAME is silently truncated by the
            // kernel; the frame's own length header will then disagree with the
            // bytes we hold, and `split` rejects it as truncated.
            buf.truncate(n as usize);
            return Ok(n as usize);
        }
    }
}

/// An in-memory [`Transport`] for tests: scripted input, recorded output.
#[derive(Default)]
pub struct MemTransport {
    /// Frames the peer will "send" us, in order. Exhausted queue reads as EOF.
    pub incoming: VecDeque<Vec<u8>>,
    /// Every frame written, in order.
    pub sent: Vec<Vec<u8>>,
}

impl MemTransport {
    pub fn with_incoming(frames: Vec<Vec<u8>>) -> Self {
        Self {
            incoming: frames.into(),
            sent: Vec::new(),
        }
    }

    /// Queue one more frame for the peer to receive.
    pub fn push_incoming(&mut self, frame: Vec<u8>) {
        self.incoming.push_back(frame);
    }
}

impl Transport for MemTransport {
    fn send(&mut self, frame: &[u8]) -> Result<(), ProtoError> {
        self.sent.push(frame.to_vec());
        Ok(())
    }

    fn recv(&mut self, buf: &mut Vec<u8>) -> Result<usize, ProtoError> {
        buf.clear();
        match self.incoming.pop_front() {
            Some(frame) => {
                buf.extend_from_slice(&frame);
                Ok(frame.len())
            }
            None => Ok(0),
        }
    }
}
