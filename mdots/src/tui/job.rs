use anyhow::{anyhow, Result};
use std::sync::mpsc::{self, Receiver, TryRecvError};

/// A one-shot piece of work run off the UI thread.
///
/// The TUI is synchronous and single-threaded, so a screen that probes the
/// system inline — `pacman`/`flatpak` shell-outs, config tree walks — freezes
/// the entire interface (including `q`) for as long as the probe takes. `Job`
/// moves that work onto a worker thread; the screen polls it once per frame
/// with [`Job::take`] and keeps drawing meanwhile.
///
/// Exactly one value is ever produced. Dropping a running `Job` detaches its
/// thread: the worker finishes into a closed channel and its result is
/// discarded. That makes `Job` suitable **only for read-only probes** — never
/// run a system-mutating command through it, since nothing guarantees anyone
/// is still listening when it completes. Mutations go through
/// `tui::app::Action` and the confirm-gated dispatch path instead.
#[derive(Default)]
pub enum Job<T> {
    /// No work requested yet, or the last result has been taken.
    #[default]
    Idle,
    /// A worker is in flight; the channel yields exactly one message.
    Running(Receiver<Result<T>>),
    /// The worker finished and its outcome is waiting to be taken.
    Ready(Result<T>),
}

impl<T: Send + 'static> Job<T> {
    /// Spawn `f` on a worker thread, replacing whatever this job held. Any
    /// previously running worker is detached (its result is dropped).
    pub fn spawn(&mut self, f: impl FnOnce() -> Result<T> + Send + 'static) {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            // A send error just means the screen moved on and dropped the
            // receiver — the result is genuinely unwanted, so discard it.
            let _ = tx.send(f());
        });
        *self = Job::Running(rx);
    }

    /// Whether a worker is currently in flight. The event loop uses this to
    /// keep redrawing while work is pending, so the result is picked up as
    /// soon as it lands instead of waiting for the next input event.
    pub fn is_running(&self) -> bool {
        matches!(self, Job::Running(_))
    }

    /// Poll without blocking. Promotes `Running` → `Ready` when the worker
    /// reports in, then hands back a finished result (leaving the job
    /// `Idle`). Returns `None` while the worker is still busy or when there
    /// is nothing to hand back.
    pub fn take(&mut self) -> Option<Result<T>> {
        if let Job::Running(rx) = self {
            match rx.try_recv() {
                Ok(value) => *self = Job::Ready(value),
                Err(TryRecvError::Empty) => return None,
                // The sender was dropped without sending: the worker
                // panicked. Surface it as an error rather than leaving the
                // screen polling a job that can never complete.
                Err(TryRecvError::Disconnected) => {
                    *self = Job::Ready(Err(anyhow!("background task died")));
                }
            }
        }
        match std::mem::replace(self, Job::Idle) {
            Job::Ready(value) => Some(value),
            other => {
                *self = other;
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Poll `job` until it yields, or fail the test after `timeout`.
    fn block_on<T: Send + 'static>(job: &mut Job<T>, timeout: Duration) -> Result<T> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(result) = job.take() {
                return result;
            }
            assert!(Instant::now() < deadline, "job did not finish in time");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn idle_job_yields_nothing() {
        let mut job: Job<u32> = Job::default();
        assert!(job.take().is_none());
        assert!(!job.is_running());
    }

    #[test]
    fn spawned_job_reports_running_then_yields_its_value() {
        let mut job = Job::default();
        job.spawn(|| Ok(42u32));
        let value = block_on(&mut job, Duration::from_secs(5)).expect("job succeeded");
        assert_eq!(value, 42);
    }

    #[test]
    fn job_result_is_taken_exactly_once() {
        let mut job = Job::default();
        job.spawn(|| Ok(7u32));
        block_on(&mut job, Duration::from_secs(5)).expect("job succeeded");
        assert!(job.take().is_none());
        assert!(!job.is_running());
    }

    #[test]
    fn job_error_is_propagated_to_the_screen() {
        let mut job: Job<u32> = Job::default();
        job.spawn(|| Err(anyhow!("pacman exploded")));
        let err = block_on(&mut job, Duration::from_secs(5)).expect_err("job failed");
        assert!(err.to_string().contains("pacman exploded"));
    }

    #[test]
    fn panicking_worker_surfaces_as_an_error_not_a_hang() {
        let mut job: Job<u32> = Job::default();
        job.spawn(|| panic!("worker blew up"));
        let err = block_on(&mut job, Duration::from_secs(5)).expect_err("job failed");
        assert!(err.to_string().contains("died"));
    }

    #[test]
    fn spawning_again_replaces_the_previous_job() {
        let mut job = Job::default();
        job.spawn(|| Ok(1u32));
        job.spawn(|| Ok(2u32));
        let value = block_on(&mut job, Duration::from_secs(5)).expect("job succeeded");
        assert_eq!(value, 2);
    }
}
