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
    /// The last frame pushed to this subscription, so an unchanged one
    /// (a focus refresh, a no-op arrange) isn't sent again. `None` until
    /// the first push.
    pub last_sent: Option<std::rc::Rc<str>>,
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
        self.watches.push(Watch {
            token,
            topic,
            args,
            last_sent: None,
        });
    }
    pub fn remove_conn(&mut self, token: u32) {
        self.watches.retain(|w| w.token != token);
    }
}

impl MargoState {
    /// Push a fresh frame to every active watch subscription whose frame
    /// changed since its last push. Called from `flush_ipc_if_dirty`
    /// (rate-capped) when state changed.
    pub fn ipc_push_watches(&mut self) {
        if self.ipc.watches.watches.is_empty() {
            return;
        }
        // Build the full snapshot ONCE per flush, not once per subscriber.
        // Every `state` subscriber gets a byte-identical document, so its
        // serialized line is produced once and reused; projection topics
        // (`tags`, `twilight`, `monitor`, …) derive from the same `snap`
        // instead of each forcing another O(monitors×clients) rebuild.
        let snap = self.build_state_snapshot();
        let mut state_line: Option<std::rc::Rc<str>> = None;
        let mut sends: Vec<(u32, std::rc::Rc<str>)> = Vec::new();
        for i in 0..self.ipc.watches.watches.len() {
            let w = &self.ipc.watches.watches[i];
            // `shortcuts` is an event stream — frames come from
            // `push_global_shortcut_event` on activation, never from
            // the state-dirty fan-out.
            if w.topic == "shortcuts" {
                continue;
            }
            let line = if w.topic == "state" {
                std::rc::Rc::clone(state_line.get_or_insert_with(|| frame_line(&snap)))
            } else {
                let payload = match self.ipc_non_snapshot_topic(&w.topic) {
                    Some(payload) => payload,
                    None => project_topic(&snap, &self.current_kb_layout, &w.topic, &w.args),
                };
                frame_line(&payload)
            };
            let w = &mut self.ipc.watches.watches[i];
            if w.last_sent.as_deref() == Some(&*line) {
                continue;
            }
            w.last_sent = Some(std::rc::Rc::clone(&line));
            sends.push((w.token, line));
        }
        // Sent after the loop: a send can drop a slow connection, which
        // removes its watches from the list being walked.
        for (token, line) in sends {
            self.ipc_send_line(token, &line);
        }
    }
}

/// `value` as one newline-terminated wire frame.
fn frame_line(value: &serde_json::Value) -> std::rc::Rc<str> {
    let mut s = value.to_string();
    s.push('\n');
    s.into()
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
