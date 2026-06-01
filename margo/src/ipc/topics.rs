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
        // The `state` topic is the whole snapshot — return it by move so
        // the hot watch-state path never clones the document.
        if topic == "state" {
            return self.ipc_state_snapshot();
        }
        let snap = self.ipc_state_snapshot();
        project_topic(&snap, &self.current_kb_layout, topic, args)
    }
}

/// Project a `get`/`watch` topic out of an already-built state snapshot.
/// Pure (no compositor state) so the topic/arg routing — including the
/// `error` frames for missing ids — is unit-testable in isolation.
pub fn project_topic(snap: &Value, kb_layout: &str, topic: &str, args: &[String]) -> Value {
    match topic {
        "state" => snap.clone(),
        "clients" => json!({ "clients": snap["clients"].clone() }),
        "monitors" => json!({ "monitors": snap["outputs"].clone() }),
        "layouts" => json!({ "layouts": snap["layouts"].clone() }),
        "twilight" => snap["twilight"].clone(),
        "config-errors" => json!({ "config_errors": snap["config_errors"].clone() }),
        "keyboard-layout" => json!({ "keyboard_layout": kb_layout }),
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

#[cfg(test)]
mod tests {
    use super::project_topic;
    use serde_json::{Value, json};

    fn snap() -> Value {
        json!({
            "clients": [
                { "idx": 0, "focused": false, "title": "a" },
                { "idx": 1, "focused": true,  "title": "b" },
            ],
            "outputs": [
                {
                    "name": "DP-1",
                    "active_tag_mask": 1,
                    "occupied_tag_mask": 3,
                    "layout_idx": 2
                }
            ],
            "layouts": ["tile", "scroller"],
            "twilight": { "temperature": 4000 },
            "config_errors": [],
        })
    }

    fn args(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn state_topic_returns_whole_snapshot() {
        assert_eq!(project_topic(&snap(), "tr", "state", &[]), snap());
    }

    #[test]
    fn collection_topics_are_wrapped() {
        let s = snap();
        assert_eq!(
            project_topic(&s, "tr", "clients", &[])["clients"],
            s["clients"]
        );
        assert_eq!(
            project_topic(&s, "tr", "monitors", &[])["monitors"],
            s["outputs"]
        );
        assert_eq!(
            project_topic(&s, "tr", "layouts", &[])["layouts"],
            s["layouts"]
        );
    }

    #[test]
    fn keyboard_layout_uses_passed_value() {
        assert_eq!(
            project_topic(&snap(), "us-intl", "keyboard-layout", &[]),
            json!({ "keyboard_layout": "us-intl" })
        );
    }

    #[test]
    fn focused_picks_the_focused_client() {
        let f = project_topic(&snap(), "tr", "focused", &[]);
        assert_eq!(f["focused"]["idx"], json!(1));
    }

    #[test]
    fn client_by_id_found_and_missing() {
        let s = snap();
        assert_eq!(
            project_topic(&s, "tr", "client", &args(&["1"]))["idx"],
            json!(1)
        );
        assert_eq!(
            project_topic(&s, "tr", "client", &args(&["99"])),
            json!({ "error": "no such client" })
        );
        assert_eq!(
            project_topic(&s, "tr", "client", &[]),
            json!({ "error": "usage: get client <id>" })
        );
    }

    #[test]
    fn monitor_by_name_found_and_missing() {
        let s = snap();
        assert_eq!(
            project_topic(&s, "tr", "monitor", &args(&["DP-1"]))["name"],
            json!("DP-1")
        );
        assert_eq!(
            project_topic(&s, "tr", "monitor", &args(&["HDMI-9"])),
            json!({ "error": "no such monitor" })
        );
        assert_eq!(
            project_topic(&s, "tr", "monitor", &[]),
            json!({ "error": "usage: get monitor <name>" })
        );
    }

    #[test]
    fn tags_projection_and_errors() {
        let s = snap();
        let t = project_topic(&s, "tr", "tags", &args(&["DP-1"]));
        assert_eq!(t["monitor"], json!("DP-1"));
        assert_eq!(t["active_tag_mask"], json!(1));
        assert_eq!(t["occupied_tag_mask"], json!(3));
        assert_eq!(t["layout_idx"], json!(2));
        assert_eq!(
            project_topic(&s, "tr", "tags", &args(&["nope"])),
            json!({ "error": "no such monitor" })
        );
        assert_eq!(
            project_topic(&s, "tr", "tags", &[]),
            json!({ "error": "usage: get tags <monitor>" })
        );
    }

    #[test]
    fn unknown_topic_is_an_error() {
        assert_eq!(
            project_topic(&snap(), "tr", "frobnicate", &[]),
            json!({ "error": "unknown topic: frobnicate" })
        );
    }
}
