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
            loop {
                poll.poll(&mut events, None)?;

                fn forward_receiver_to_file(
                    receiver: &mut Receiver,
                    file_handle: &mut LimitSizeWriter<impl io::Write>,
                    is_read_closed: bool,
                ) -> io::Result<()> {
                    let mut buf = [0u8; 2048];

                    loop {
                        if is_read_closed {
                            let mut v = Vec::new();
                            receiver.read_to_end(&mut v)?;
                            file_handle.write_all(&v)?;

                            break;
                        }

                        match receiver.read(&mut buf) {
                            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                                break;
                            }
                            Err(err) => return Err(err),
                            Ok(n) => {
                                file_handle.write_all(&buf[..n])?;
                            }
                        }
                    }

                    Ok(())
                }

                for event in events.iter() {
                    match event.token() {
                        x if x == WAKER_TOKEN => {
                            return Ok(());
                        }
                        x if x == STDOUT_PIPE_RECV => forward_receiver_to_file(
                            &mut stdout_receiver,
                            &mut file_handle,
                            event.is_read_closed(),
                        )?,
                        x if x == STDERR_PIPE_RECV => forward_receiver_to_file(
                            &mut stderr_receiver,
                            &mut file_handle,
                            event.is_read_closed(),
                        )?,
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
