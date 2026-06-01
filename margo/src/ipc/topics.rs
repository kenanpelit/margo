//! Topic → JSON payload builders for `get` / `watch`.
//!
//! The `state` topic is the full snapshot (the same document the shell
//! and mctl parse); the other topics are projections of it.

use crate::state::MargoState;
use serde_json::{Value, json};

impl MargoState {
    /// The full state snapshot — the `state` topic payload. Thin
    /// wrapper over the canonical builder so topic code has one entry
    /// point.
    pub fn ipc_state_snapshot(&self) -> Value {
        self.build_state_snapshot()
    }

    /// Build the JSON payload for a `get`/`watch` topic. Returns an
    /// error frame value (`{"error":…}`) for unknown topics / bad args.
    pub fn ipc_topic(&self, topic: &str, args: &[String]) -> Value {
        let snap = self.ipc_state_snapshot();
        match topic {
            "state" => snap,
            "clients" => json!({ "clients": snap["clients"].clone() }),
            "monitors" => json!({ "monitors": snap["outputs"].clone() }),
            "layouts" => json!({ "layouts": snap["layouts"].clone() }),
            "twilight" => snap["twilight"].clone(),
            "config-errors" => json!({ "config_errors": snap["config_errors"].clone() }),
            "keyboard-layout" => json!({ "keyboard_layout": self.current_kb_layout }),
            "focused" => {
                let f = snap["clients"]
                    .as_array()
                    .and_then(|cs| cs.iter().find(|c| c["focused"] == json!(true)))
                    .cloned()
                    .unwrap_or(Value::Null);
                json!({ "focused": f })
            }
            "client" => match args.first().and_then(|s| s.parse::<i64>().ok()) {
                Some(id) => snap["clients"]
                    .as_array()
                    .and_then(|cs| cs.iter().find(|c| c["idx"] == json!(id)).cloned())
                    .unwrap_or_else(|| json!({ "error": "no such client" })),
                None => json!({ "error": "usage: get client <id>" }),
            },
            "monitor" => match args.first() {
                Some(name) => snap["outputs"]
                    .as_array()
                    .and_then(|ms| ms.iter().find(|m| m["name"] == json!(name)).cloned())
                    .unwrap_or_else(|| json!({ "error": "no such monitor" })),
                None => json!({ "error": "usage: get monitor <name>" }),
            },
            "tags" => match args.first() {
                Some(name) => snap["outputs"]
                    .as_array()
                    .and_then(|ms| ms.iter().find(|m| m["name"] == json!(name)))
                    .map(|m| {
                        json!({
                            "monitor": name,
                            "active_tag_mask": m["active_tag_mask"].clone(),
                            "occupied_tag_mask": m["occupied_tag_mask"].clone(),
                            "layout_idx": m["layout_idx"].clone(),
                        })
                    })
                    .unwrap_or_else(|| json!({ "error": "no such monitor" })),
                None => json!({ "error": "usage: get tags <monitor>" }),
            },
            other => json!({ "error": format!("unknown topic: {other}") }),
        }
    }
}
