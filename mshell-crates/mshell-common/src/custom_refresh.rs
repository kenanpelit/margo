//! Force-refresh bus for `exec`-driven custom bar pills.
//!
//! `mshellctl custom-refresh [name]` (IPC) calls [`request`]; every exec
//! source (one per distinct pill config, shared by that pill's copies on
//! every monitor) holds a [`subscribe`] receiver and re-runs its `exec`
//! once when the request targets it (an empty name reaches every pill).

use std::sync::LazyLock;
use tokio::sync::broadcast;

static BUS: LazyLock<broadcast::Sender<String>> = LazyLock::new(|| broadcast::channel(16).0);

/// Ask the pill called `name` (or every pill, for an empty name) to re-run
/// its `exec` now. Returns how many exec sources are listening (a pill
/// shown on several monitors counts once).
pub fn request(name: &str) -> usize {
    BUS.send(name.trim().to_string()).unwrap_or(0)
}

/// A receiver for refresh requests.
pub fn subscribe() -> broadcast::Receiver<String> {
    BUS.subscribe()
}

/// Whether a request for `target` applies to the pill called `name`.
pub fn targets(target: &str, name: &str) -> bool {
    target.is_empty() || target == name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_target_reaches_every_pill_and_a_name_only_its_own() {
        assert!(targets("", "cpu"));
        assert!(targets("cpu", "cpu"));
        assert!(!targets("cpu", "gpu"));
    }

    #[test]
    fn request_reaches_subscribers() {
        let mut rx = subscribe();
        assert!(request(" cpu ") >= 1);
        assert_eq!(rx.try_recv().ok().as_deref(), Some("cpu"));
    }
}
