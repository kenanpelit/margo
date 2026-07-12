//! `watch`-subscription registry + change fan-out.

use crate::ipc::topics::project_topic;
use crate::state::MargoState;

/// One active `watch` subscription.
pub struct Watch {
    /// Token identifying the client connection.
    pub token: u32,
    /// Topic the client subscribed to (e.g. "state").
    pub topic: String,
    /// Topic args (e.g. monitor name for `watch tags <mon>`).
    pub args: Vec<String>,
}

/// A single connection may hold several *distinct* subscriptions (e.g.
/// `watch state` + `watch tags DP-1`), but no more than this — bounds a
/// client that keeps sending fresh topics.
const MAX_WATCHES_PER_CONN: usize = 16;
/// Registry-wide ceiling across all connections.
const MAX_WATCHES_TOTAL: usize = 512;

#[derive(Default)]
pub struct WatchRegistry {
    pub watches: Vec<Watch>,
}

impl WatchRegistry {
    /// Register a subscription. Idempotent per `(token, topic, args)` and
    /// bounded: re-sending an identical `watch` line is a no-op rather than
    /// stacking a duplicate that gets cloned + fanned out on every state
    /// change, and per-connection / global caps stop a flooding client from
    /// growing the registry without limit.
    pub fn add(&mut self, token: u32, topic: String, args: Vec<String>) {
        if self
            .watches
            .iter()
            .any(|w| w.token == token && w.topic == topic && w.args == args)
        {
            return;
        }
        if self.watches.len() >= MAX_WATCHES_TOTAL
            || self.watches.iter().filter(|w| w.token == token).count() >= MAX_WATCHES_PER_CONN
        {
            return;
        }
        self.watches.push(Watch { token, topic, args });
    }
    pub fn remove_conn(&mut self, token: u32) {
        self.watches.retain(|w| w.token != token);
    }
}

impl MargoState {
    /// Push a fresh frame to every active watch subscription. Called
    /// once per loop iteration when state changed.
    pub fn ipc_push_watches(&mut self) {
        if self.ipc_watches.watches.is_empty() {
            return;
        }
        // Snapshot (token, topic, args) first to avoid borrow conflicts
        // with `ipc_send` (which mutates `ipc_conns`).
        let subs: Vec<(u32, String, Vec<String>)> = self
            .ipc_watches
            .watches
            .iter()
            .map(|w| (w.token, w.topic.clone(), w.args.clone()))
            .collect();
        // Build the full snapshot ONCE per flush, not once per subscriber.
        // Every `state` subscriber gets a byte-identical document, so its
        // serialized line is produced once and reused; projection topics
        // (`tags`, `twilight`, `monitor`, …) derive from the same `snap`
        // instead of each forcing another O(monitors×clients) rebuild.
        let snap = self.build_state_snapshot();
        let mut state_line: Option<String> = None;
        for (token, topic, args) in subs {
            if topic == "state" {
                let line: &str = state_line.get_or_insert_with(|| {
                    let mut s = snap.to_string();
                    s.push('\n');
                    s
                });
                self.ipc_send_line(token, line);
            } else {
                let payload = project_topic(&snap, &self.current_kb_layout, &topic, &args);
                self.ipc_send(token, &payload);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn identical_watch_is_deduped() {
        let mut reg = WatchRegistry::default();
        for _ in 0..100 {
            reg.add(1, "state".into(), vec![]);
        }
        assert_eq!(
            reg.watches.len(),
            1,
            "re-sending `watch state` must not stack"
        );
    }

    #[test]
    fn distinct_topics_on_one_connection_coexist() {
        let mut reg = WatchRegistry::default();
        reg.add(1, "state".into(), vec![]);
        reg.add(1, "tags".into(), args(&["DP-1"]));
        reg.add(1, "tags".into(), args(&["HDMI-1"]));
        assert_eq!(reg.watches.len(), 3);
    }

    #[test]
    fn per_connection_cap_bounds_a_flood() {
        let mut reg = WatchRegistry::default();
        for i in 0..1000 {
            reg.add(1, "tags".into(), args(&[&i.to_string()]));
        }
        assert_eq!(reg.watches.len(), MAX_WATCHES_PER_CONN);
    }

    #[test]
    fn remove_conn_clears_only_that_token() {
        let mut reg = WatchRegistry::default();
        reg.add(1, "state".into(), vec![]);
        reg.add(2, "state".into(), vec![]);
        reg.remove_conn(1);
        assert_eq!(reg.watches.len(), 1);
        assert_eq!(reg.watches[0].token, 2);
    }
}
