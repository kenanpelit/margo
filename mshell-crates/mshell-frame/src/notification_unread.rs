//! The notification centre's read / unread split, shared by every surface
//! that shows a "new notifications" count (the bar bell's dot, mdash's
//! status chip, the Overview card).
//!
//! `notification_service().notifications` is the whole stored history —
//! often well over a thousand entries — so it is never "waiting"
//! notifications. What the centre treats as **unread** is what arrived
//! since the user last opened it: the history present when the shell
//! started counts as read, and opening the centre (the bell, or the
//! Notifications menu becoming visible) marks everything read. Before
//! this module the bell kept that line privately and the dashboard
//! printed the history size ("1662 notifications waiting").

use std::sync::{LazyLock, Mutex};
use tokio::sync::watch;

/// The read line over a history whose size changes.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SeenCounter {
    /// How many history entries are read; `None` until the first
    /// observation (startup history counts as read).
    seen: Option<usize>,
}

impl SeenCounter {
    /// Unread entries for a history of `total`. The first observation
    /// takes the whole history as read; a shrunk history (cleared or a
    /// dismissal) pulls the line down so new arrivals count again.
    pub(crate) fn unread(&mut self, total: usize) -> usize {
        let seen = self.seen.get_or_insert(total);
        if *seen > total {
            *seen = total;
        }
        total - *seen
    }

    /// The user opened the centre: everything in a history of `total` is
    /// read.
    pub(crate) fn mark_all_read(&mut self, total: usize) {
        self.seen = Some(total);
    }
}

static STATE: LazyLock<Mutex<SeenCounter>> = LazyLock::new(Mutex::default);
static CHANGES: LazyLock<watch::Sender<u64>> = LazyLock::new(|| watch::channel(0).0);

fn history_len() -> usize {
    mshell_services::notification_service()
        .map(|s| s.notifications.get().len())
        .unwrap_or(0)
}

fn with_state<R>(f: impl FnOnce(&mut SeenCounter) -> R) -> R {
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut state)
}

/// Unread notifications right now.
pub(crate) fn unread() -> usize {
    let total = history_len();
    with_state(|s| s.unread(total))
}

/// Mark the current history read (the centre was opened) and tell every
/// surface showing the count.
pub(crate) fn mark_all_read() {
    let total = history_len();
    let changed = with_state(|s| {
        let before = *s;
        s.mark_all_read(total);
        before != *s
    });
    if changed {
        CHANGES.send_modify(|g| *g = g.wrapping_add(1));
    }
}

/// Fires whenever the read line moves (not on new arrivals — watch the
/// notification service for those).
pub(crate) fn changes() -> watch::Receiver<u64> {
    CHANGES.subscribe()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 1662-entry history the dashboard used to call "waiting" is
    /// read at startup; only what arrives afterwards is unread.
    #[test]
    fn startup_history_is_read_and_new_arrivals_are_unread() {
        let mut c = SeenCounter::default();
        assert_eq!(c.unread(1662), 0);
        assert_eq!(c.unread(1665), 3);
        c.mark_all_read(1665);
        assert_eq!(c.unread(1665), 0);
        assert_eq!(c.unread(1666), 1);
    }

    #[test]
    fn a_cleared_history_resets_the_line() {
        let mut c = SeenCounter::default();
        assert_eq!(c.unread(10), 0);
        assert_eq!(c.unread(0), 0, "cleared");
        assert_eq!(c.unread(2), 2, "both new ones are unread");
    }

    #[test]
    fn an_empty_start_counts_everything_after_it() {
        let mut c = SeenCounter::default();
        assert_eq!(c.unread(0), 0);
        assert_eq!(c.unread(4), 4);
    }
}
