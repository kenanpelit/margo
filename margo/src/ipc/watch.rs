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

#[derive(Default)]
pub struct WatchRegistry {
    pub watches: Vec<Watch>,
}

impl WatchRegistry {
    pub fn add(&mut self, token: u32, topic: String, args: Vec<String>) {
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
