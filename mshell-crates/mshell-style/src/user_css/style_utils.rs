use crate::user_css::paths::{style_path, styles_dir};
use crate::user_css::style::Style;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, ThemeStoreFields};
use notify::{Event, EventKind};
use reactive_graph::prelude::GetUntracked;
use reactive_stores::{ArcStore, Patch};
use std::ops::Not;
use std::{
    fs, io,
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};
use tracing::info;

pub(crate) fn load_style(active_style: String) -> Result<Style, io::Error> {
    if active_style.is_empty().not() {
        let css = fs::read_to_string(style_path(active_style.as_str()))?;
        Ok(Style { css })
    } else {
        Ok(Style::default())
    }
}

pub fn list_available_styles() -> Vec<String> {
    let dir = styles_dir();
    let mut out = Vec::new();

    let Ok(rd) = fs::read_dir(dir) else {
        return out;
    };
    for ent in rd.flatten() {
        let path = ent.path();
        if path.extension().and_then(|s| s.to_str()) != Some("css") {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            out.push(stem.to_string());
        }
    }
    out.sort();
    out
}

/// What [`debounce_loop`] waits on: the notify channel in production, a
/// scripted fake in the tests.
pub(crate) trait EventSource<E> {
    /// Block until the next event (or the sender is gone).
    fn recv_blocking(&self) -> Result<E, mpsc::RecvTimeoutError>;
    fn recv_timeout(&self, wait: Duration) -> Result<E, mpsc::RecvTimeoutError>;
}

impl<E> EventSource<E> for mpsc::Receiver<E> {
    fn recv_blocking(&self) -> Result<E, mpsc::RecvTimeoutError> {
        self.recv()
            .map_err(|_| mpsc::RecvTimeoutError::Disconnected)
    }

    fn recv_timeout(&self, wait: Duration) -> Result<E, mpsc::RecvTimeoutError> {
        mpsc::Receiver::recv_timeout(self, wait)
    }
}

/// Call `fire` once `debounce` has passed since the last event `relevant`
/// accepted. Returns when the sender side is dropped.
pub(crate) fn debounce_loop<E>(
    rx: &impl EventSource<E>,
    debounce: Duration,
    mut relevant: impl FnMut(E) -> bool,
    mut fire: impl FnMut(),
) {
    let mut pending = false;
    let mut last_event_at = Instant::now();
    loop {
        // Idle: block until the next event. Only a pending change waits on
        // a deadline (the rest of the debounce). The old unconditional
        // `recv_timeout(50ms)` woke this thread 20 times a second for the
        // whole session.
        let recv = if pending {
            rx.recv_timeout(debounce.saturating_sub(last_event_at.elapsed()))
        } else {
            rx.recv_blocking()
        };
        match recv {
            Ok(event) => {
                if relevant(event) {
                    pending = true;
                    last_event_at = Instant::now();
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if pending && last_event_at.elapsed() >= debounce {
            pending = false;
            fire();
        }
    }
}

pub(crate) fn watch_style_loop(rx: mpsc::Receiver<notify::Result<Event>>, style: ArcStore<Style>) {
    const DEBOUNCE: Duration = Duration::from_millis(200);
    debounce_loop(
        &rx,
        DEBOUNCE,
        |event| match event {
            Ok(event) => {
                let active_style = config_manager().config().theme().css_file().get_untracked();
                is_relevant_style_event(&event, &style_path(active_style.as_str()))
            }
            Err(e) => {
                eprintln!("config: watch error: {e}");
                false
            }
        },
        || {
            let active = config_manager().config().theme().css_file().get_untracked();
            match load_style(active) {
                Ok(new_style) => {
                    style.patch(new_style);
                    info!("New style loaded in watch loop");
                }
                Err(e) => eprintln!("style: reload failed (keeping last-good): {e}"),
            }
        },
    );
}

pub(crate) fn is_relevant_style_event(event: &Event, active_style_path: &PathBuf) -> bool {
    // Only respond to writes/creates/removes/renames (not Access/Other)
    match event.kind {
        EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_) => {}
        _ => return false,
    }

    event.paths.iter().any(|path| {
        // ignore editor temp files
        if let Some(name) = path.file_name().and_then(|s| s.to_str())
            && (name.ends_with("~")
                || name.ends_with(".swp")
                || name.ends_with(".swx")
                || name.ends_with(".tmp")
                || name.starts_with(".#")
                || name.starts_with('#'))
        {
            return false;
        }

        if path == active_style_path {
            return true;
        }
        false
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    /// Replays scripted results and logs how the loop waited.
    struct Script {
        results: RefCell<VecDeque<Result<bool, mpsc::RecvTimeoutError>>>,
        calls: RefCell<Vec<&'static str>>,
    }

    impl Script {
        fn next(&self) -> Result<bool, mpsc::RecvTimeoutError> {
            self.results
                .borrow_mut()
                .pop_front()
                .unwrap_or(Err(mpsc::RecvTimeoutError::Disconnected))
        }
    }

    impl EventSource<bool> for Script {
        fn recv_blocking(&self) -> Result<bool, mpsc::RecvTimeoutError> {
            self.calls.borrow_mut().push("block");
            self.next()
        }

        fn recv_timeout(&self, wait: Duration) -> Result<bool, mpsc::RecvTimeoutError> {
            self.calls.borrow_mut().push("timeout");
            // A timeout really waits, so the debounce deadline passes.
            std::thread::sleep(wait);
            self.next()
        }
    }

    #[test]
    fn idle_blocks_and_only_a_pending_change_uses_a_timeout() {
        let rx = Script {
            results: RefCell::new(VecDeque::from([
                Ok(false), // irrelevant event: still idle
                Ok(true),  // the active style changed
                Err(mpsc::RecvTimeoutError::Timeout),
            ])),
            calls: RefCell::new(Vec::new()),
        };
        let mut fired = 0;
        debounce_loop(&rx, Duration::from_millis(20), |e| e, || fired += 1);
        assert_eq!(fired, 1);
        // Idle waits never time out (no 20 Hz wakeup); the one timed wait
        // is the debounce, then it blocks again until the channel closes.
        let calls = rx.calls.borrow();
        assert_eq!(calls.first(), Some(&"block"));
        assert_eq!(calls.last(), Some(&"block"));
        assert_eq!(
            calls.iter().filter(|c| **c == "timeout").count(),
            1,
            "{calls:?}"
        );
    }
}
