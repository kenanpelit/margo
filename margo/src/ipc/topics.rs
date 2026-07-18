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
        // `perf` is not a projection of the snapshot: the per-output frame
        // counters live in `perf_counters`, mirrored from the udev backend,
        // and are deliberately kept out of the hot `state` document.
        // Event-stream topic: the initial frame lists the currently
        // bound portal shortcut sessions; live frames arrive from
        // `push_global_shortcut_event`, not the state-dirty fan-out.
        if topic == "shortcuts" {
            return self.global_shortcuts.summary();
        }
        if topic == "perf" {
            return build_perf_payload(&self.perf_counters, std::time::Instant::now());
        }
        let snap = self.ipc_state_snapshot();
        project_topic(&snap, &self.current_kb_layout, topic, args)
    }
}

/// Build the `perf` topic payload from the mirrored per-output counters.
/// `now` is threaded in (rather than read inside) so the windowed FPS /
/// empty-ratio / latency-percentile math is deterministic and unit-testable in
/// isolation. Pure (no compositor state). Outputs are sorted by name for stable
/// output.
pub fn build_perf_payload(
    counters: &std::collections::HashMap<String, crate::state::OutputPerf>,
    now: std::time::Instant,
) -> Value {
    let mut names: Vec<&String> = counters.keys().collect();
    names.sort();
    let outputs: Vec<Value> = names
        .into_iter()
        .map(|name| {
            let p = &counters[name];
            // Lifetime (since-boot) ratio — kept for compatibility.
            let empty_ratio = if p.renders > 0 {
                p.empties as f64 / p.renders as f64
            } else {
                0.0
            };

            // Time-based metrics over the rolling sample window: "how is it
            // running *now*", which the cumulative counters can't show.
            let secs_ago =
                |s: &crate::state::FrameSample| now.saturating_duration_since(s.at).as_secs_f64();
            let fps = |window: f64| {
                let n = p.samples.iter().filter(|s| secs_ago(s) <= window).count();
                n as f64 / window
            };
            // Recent (last 10 s) empty ratio: over-scheduling shows up here even
            // when the lifetime ratio looks fine.
            let (recent_total, recent_empty) = p
                .samples
                .iter()
                .filter(|s| secs_ago(s) <= 10.0)
                .fold((0u64, 0u64), |(t, e), s| (t + 1, e + s.empty as u64));
            let empty_ratio_10s = if recent_total > 0 {
                recent_empty as f64 / recent_total as f64
            } else {
                0.0
            };
            // Render-latency percentiles (µs) across the window — the tail (p95/
            // p99) is where jank hides that the mean averages away.
            let mut lat: Vec<u32> = p.samples.iter().map(|s| s.render_us).collect();
            lat.sort_unstable();
            let pct = |q: f64| -> u64 {
                if lat.is_empty() {
                    return 0;
                }
                let idx = (((lat.len() - 1) as f64) * q).round() as usize;
                lat[idx] as u64
            };

            json!({
                "name": name,
                "renders": p.renders,
                "queued": p.queued,
                "empties": p.empties,
                "empty_ratio": empty_ratio,
                "queue_errors": p.queue_errors,
                "render_errors": p.render_errors,
                "fps_1s": fps(1.0),
                "fps_10s": fps(10.0),
                "fps_60s": fps(60.0),
                "empty_ratio_10s": empty_ratio_10s,
                "render_us_p50": pct(0.50),
                "render_us_p95": pct(0.95),
                "render_us_p99": pct(0.99),
                "window_samples": p.samples.len(),
            })
        })
        .collect();
    json!({ "outputs": outputs })
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

    #[test]
    fn perf_payload_sorts_outputs_and_computes_empty_ratio() {
        use crate::state::OutputPerf;
        let mut counters = std::collections::HashMap::new();
        counters.insert(
            "DP-1".to_string(),
            OutputPerf {
                renders: 100,
                queued: 75,
                empties: 25,
                queue_errors: 2,
                ..Default::default()
            },
        );
        counters.insert(
            "eDP-1".to_string(),
            OutputPerf {
                renders: 0,
                queued: 0,
                empties: 0,
                queue_errors: 0,
                ..Default::default()
            },
        );
        let out = super::build_perf_payload(&counters, std::time::Instant::now());
        let arr = out["outputs"].as_array().unwrap();
        // Sorted by name: DP-1 before eDP-1.
        assert_eq!(arr[0]["name"], json!("DP-1"));
        assert_eq!(arr[1]["name"], json!("eDP-1"));
        // 25/100 = 0.25.
        assert_eq!(arr[0]["empty_ratio"], json!(0.25));
        assert_eq!(arr[0]["queue_errors"], json!(2));
        // No divide-by-zero on a never-rendered output.
        assert_eq!(arr[1]["empty_ratio"], json!(0.0));
    }

    #[test]
    fn perf_payload_computes_windowed_fps_and_latency_percentiles() {
        use crate::state::{FrameSample, OutputPerf};
        use std::time::{Duration, Instant};

        // Fix a reference "now" 30 s after the samples' base so every sample
        // sits at a known age; build_perf_payload takes `now` as a parameter.
        let now = Instant::now() + Duration::from_secs(30);
        let mut samples = std::collections::VecDeque::new();
        // 25 s old — inside the 60 s window only. Empty frame, 5 ms render.
        samples.push_back(FrameSample {
            at: now - Duration::from_secs(25),
            empty: true,
            render_us: 5000,
        });
        // 5 s old — inside 10 s + 60 s. Non-empty, 3 ms.
        samples.push_back(FrameSample {
            at: now - Duration::from_secs(5),
            empty: false,
            render_us: 3000,
        });
        // 0.5 s old — inside all three windows. Non-empty, 1 ms.
        samples.push_back(FrameSample {
            at: now - Duration::from_millis(500),
            empty: false,
            render_us: 1000,
        });

        let mut counters = std::collections::HashMap::new();
        counters.insert(
            "DP-1".to_string(),
            OutputPerf {
                renders: 3,
                queued: 2,
                empties: 1,
                queue_errors: 0,
                render_errors: 4,
                samples,
            },
        );
        let out = super::build_perf_payload(&counters, now);
        let o = &out["outputs"][0];

        let approx = |v: &Value, want: f64| (v.as_f64().unwrap() - want).abs() < 1e-9;
        // 1 frame in the last 1 s, 2 in 10 s, 3 in 60 s → count / window.
        assert!(approx(&o["fps_1s"], 1.0));
        assert!(approx(&o["fps_10s"], 0.2));
        assert!(approx(&o["fps_60s"], 0.05));
        assert_eq!(o["window_samples"], json!(3));
        // Recent (10 s) window holds the two non-empty frames → 0 empty ratio,
        // even though the lifetime ratio is 1/3.
        assert!(approx(&o["empty_ratio_10s"], 0.0));
        // Percentiles over sorted [1000, 3000, 5000] µs.
        assert_eq!(o["render_us_p50"], json!(3000));
        assert_eq!(o["render_us_p95"], json!(5000));
        assert_eq!(o["render_us_p99"], json!(5000));
        // Lifetime render-error total is surfaced (mctl perf --json).
        assert_eq!(o["render_errors"], json!(4));
    }
}
