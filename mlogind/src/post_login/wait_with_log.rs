//! This module implements a thread that ensures that the log files don't exceed a specific size.

use std::io::{self, BufWriter};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::JoinHandle;

use std::io::Read;

use log::info;
use mio::unix::pipe::Receiver;
use mio::{Events, Interest, Poll, Token, Waker};

/// 64MB
const LOG_WRITER_SIZE_LIMIT: usize = 67_108_864;

struct LimitSizeWriter<W: io::Write> {
    writer: BufWriter<W>,
    current_byte_count: usize,
    size_limit: usize,
}

impl<W: io::Write> io::Write for LimitSizeWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        debug_assert!(self.current_byte_count <= self.size_limit);

        if self.current_byte_count >= self.size_limit {
            return Ok(buf.len());
        }

        let write_len = usize::min(buf.len(), self.size_limit - self.current_byte_count);

        let written_len = self.writer.write(&buf[..write_len])?;
        self.current_byte_count += write_len;

        if written_len != write_len {
            return Ok(written_len);
        }

        Ok(buf.len())
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

impl<W: io::Write> LimitSizeWriter<W> {
    pub fn new(writer: W, size_limit: usize) -> Self {
        Self {
            writer: BufWriter::new(writer),
            current_byte_count: 0,
            size_limit,
        }
    }
}

/// This is a wrapper of the rust std `Child` struct.
///
/// This makes handling spawning, killing and waiting a lot easier to combine with the
/// output log files.
pub enum SessionChild {
    NoLog(Child),
    Log(LimitedOutputChild),
}

pub struct LimitedOutputChild {
    process: Child,
    log_thread: Option<(Waker, JoinHandle<io::Result<()>>)>,
}

impl SessionChild {
    pub fn spawn(mut command: Command, log_path: Option<&Path>) -> io::Result<Self> {
        Ok(match log_path {
            None => Self::NoLog(
                command
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()?,
            ),
            Some(log_path) => Self::Log(LimitedOutputChild::spawn(command, log_path)?),
        })
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        match self {
            Self::NoLog(process) => process.wait(),
            Self::Log(process) => process.wait(),
        }
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        match self {
            Self::NoLog(process) => process.try_wait(),
            Self::Log(process) => process.try_wait(),
        }
    }

    pub fn kill(&mut self) -> io::Result<()> {
        match self {
            Self::NoLog(process) => process.kill(),
            Self::Log(process) => process.kill(),
        }
    }

    pub fn send_sigterm(&self) -> io::Result<()> {
        unsafe { libc::kill(self.id() as libc::pid_t, libc::SIGTERM) };

        Ok(())
    }

    pub fn id(&self) -> u32 {
        match self {
            Self::NoLog(process) => process.id(),
            Self::Log(process) => process.id(),
        }
    }
}

/// Copies everything readable from `receiver` into `file_handle` without
/// blocking. Returns `true` once the write end is closed (EOF).
///
/// EOF is a zero-length read and is checked on every read: an event that
/// arrives before the writer closes carries no read-closed flag, yet the pipe
/// can reach EOF by the time it is read. Looping on that zero-length read
/// spun the log thread at 100% CPU and, because it never polled again, it
/// never saw the waker either, so `wait()` hung forever on the join.
fn forward_receiver_to_file(
    receiver: &mut impl Read,
    file_handle: &mut impl io::Write,
) -> io::Result<bool> {
    let mut buf = [0u8; 2048];
    loop {
        match receiver.read(&mut buf) {
            Ok(0) => return Ok(true),
            Ok(n) => file_handle.write_all(&buf[..n])?,
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => return Err(err),
        }
    }
}

impl LimitedOutputChild {
    pub fn spawn(mut command: Command, log_path: &Path) -> io::Result<Self> {
        const STDOUT_PIPE_RECV: Token = Token(0);
        const STDERR_PIPE_RECV: Token = Token(1);
        const WAKER_TOKEN: Token = Token(2);

        let mut poll = Poll::new()?;
        let mut events = Events::with_capacity(128);

        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        use io::Write;

        // Private (0600): it carries every user's session output in turn.
        // Opened before the spawn, so a failure leaves no orphaned session.
        let file = crate::log_file::open_private_log(log_path)?;

        let mut process = command.spawn()?;

        let Some(stdout) = process.stdout.take() else {
            return Err(io::Error::other("Failed to grab stdout"));
        };

        let Some(stderr) = process.stderr.take() else {
            return Err(io::Error::other("Failed to grab stderr"));
        };

        let mut stdout_receiver = Receiver::from(stdout);
        stdout_receiver.set_nonblocking(true)?;
        poll.registry()
            .register(&mut stdout_receiver, STDOUT_PIPE_RECV, Interest::READABLE)?;

        let mut stderr_receiver = Receiver::from(stderr);
        stderr_receiver.set_nonblocking(true)?;
        poll.registry()
            .register(&mut stderr_receiver, STDERR_PIPE_RECV, Interest::READABLE)?;

        let waker = Waker::new(poll.registry(), WAKER_TOKEN)?;

        let mut file_handle = LimitSizeWriter::new(file, LOG_WRITER_SIZE_LIMIT);

        let join_handle = std::thread::spawn(move || {
            let (mut stdout_done, mut stderr_done) = (false, false);
            loop {
                poll.poll(&mut events, None)?;

                for event in events.iter() {
                    match event.token() {
                        x if x == WAKER_TOKEN => {
                            // The session exited: whatever it wrote last
                            // (usually the crash reason) is still in the
                            // pipes. Drain both before stopping.
                            if !stdout_done {
                                forward_receiver_to_file(&mut stdout_receiver, &mut file_handle)?;
                            }
                            if !stderr_done {
                                forward_receiver_to_file(&mut stderr_receiver, &mut file_handle)?;
                            }
                            file_handle.flush()?;
                            return Ok(());
                        }
                        x if x == STDOUT_PIPE_RECV && !stdout_done => {
                            if forward_receiver_to_file(&mut stdout_receiver, &mut file_handle)? {
                                stdout_done = true;
                                poll.registry().deregister(&mut stdout_receiver)?;
                            }
                        }
                        x if x == STDERR_PIPE_RECV && !stderr_done => {
                            if forward_receiver_to_file(&mut stderr_receiver, &mut file_handle)? {
                                stderr_done = true;
                                poll.registry().deregister(&mut stderr_receiver)?;
                            }
                        }
                        x if x == STDOUT_PIPE_RECV || x == STDERR_PIPE_RECV => {}
                        _ => {
                            return Err(io::Error::other("Invalid event"));
                        }
                    }
                }
            }
        });

        Ok(Self {
            process,
            log_thread: Some((waker, join_handle)),
        })
    }

    fn stop_logging(&mut self) -> io::Result<()> {
        if let Some((waker, join_handle)) = self.log_thread.take() {
            waker.wake()?;

            info!("Joining with logging thread.");

            join_handle
                .join()
                .expect("Failed to join with log thread")?;
        }

        Ok(())
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        let exit_status = self.process.wait()?;

        self.stop_logging()?;

        Ok(exit_status)
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let exit_status = match self.process.try_wait()? {
            None => return Ok(None),
            Some(exit_status) => exit_status,
        };

        self.stop_logging()?;

        Ok(Some(exit_status))
    }

    pub fn kill(&mut self) -> io::Result<()> {
        self.process.kill()?;

        self.stop_logging()?;

        Ok(())
    }

    pub fn id(&self) -> u32 {
        self.process.id()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("mlogind-waitlog-{name}-{}", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    /// Everything the session wrote before it exited, on both streams, ends
    /// up in the log, including the last lines (usually the crash reason).
    /// `wait()` wakes the forwarder right after the exit, so the forwarder
    /// must drain what is still in the pipes before it stops.
    #[test]
    fn the_log_keeps_both_streams_up_to_the_last_line() {
        let path = scratch("tail");
        for round in 0..50 {
            let mut command = Command::new("sh");
            command.arg("-c").arg(
                "i=0; while [ $i -lt 300 ]; do echo out$i; i=$((i+1)); done; echo last-err >&2",
            );
            let mut child = SessionChild::spawn(command, Some(&path)).unwrap();
            assert!(child.wait().unwrap().success());
            let log = std::fs::read_to_string(&path).unwrap();
            assert!(log.contains("out0\n"), "round {round}: head missing");
            assert!(
                log.contains("out299\n"),
                "round {round}: stdout tail missing"
            );
            assert!(
                log.contains("last-err\n"),
                "round {round}: stderr tail missing"
            );
        }
        let _ = std::fs::remove_file(&path);
    }

    /// A pipe that already reached EOF must end the forward, even when the
    /// event that woke the thread had no read-closed flag.
    #[test]
    fn forwarding_stops_at_eof_without_a_read_closed_flag() {
        use std::io::Write as _;
        let (mut tx, mut rx) = mio::unix::pipe::new().unwrap();
        tx.write_all(b"last words\n").unwrap();
        drop(tx);
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut out = Vec::new();
            let eof = forward_receiver_to_file(&mut rx, &mut out).unwrap();
            let _ = done_tx.send((eof, out));
        });
        let (eof, out) = done_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the forward spun on a zero-length read at EOF");
        assert!(eof);
        assert_eq!(out, b"last words\n");
    }

    /// An open pipe with nothing to read returns at once, not at EOF.
    #[test]
    fn forwarding_returns_when_the_pipe_is_empty() {
        let (_tx, mut rx) = mio::unix::pipe::new().unwrap();
        let mut out = Vec::new();
        assert!(!forward_receiver_to_file(&mut rx, &mut out).unwrap());
        assert!(out.is_empty());
    }

    /// The writer stops at the size limit and still reports the whole write.
    #[test]
    fn the_size_limit_truncates_silently() {
        let mut out = Vec::new();
        {
            let mut w = LimitSizeWriter::new(&mut out, 5);
            use io::Write;
            assert_eq!(w.write(b"abc").unwrap(), 3);
            assert_eq!(w.write(b"defgh").unwrap(), 5);
            assert_eq!(w.write(b"ij").unwrap(), 2);
            w.flush().unwrap();
        }
        assert_eq!(out, b"abcde");
    }
}
