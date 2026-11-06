//! Bounded drive trend history: temperature samples from the minute scan
//! (at most one per `history_interval_minutes`) and SMART counters from
//! every full read, per drive, pruned to `history_retention_days` and a
//! hard per-drive cap. Pure data + serde; the service persists it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::menus::menu_widgets::cpu_dashboard::drives::SmartReport;

/// Hard cap per drive (365 days at the 15-minute minimum is ~35 000; the
/// cap keeps the file small even then — the oldest go first).
pub(crate) const MAX_SAMPLES: usize = 20_000;
/// A SMART read this soon after the previous SMART sample replaces it
/// (reading twice in a row adds no information).
const SMART_MERGE_SECS: u64 = 600;

/// One point in time. Missing fields were not measured then.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub(crate) struct Sample {
    /// Unix seconds.
    pub t: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temp: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wear: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spare: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_errors: Option<u64>,
    /// Host writes in GB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub written_gb: Option<u64>,
}

impl Sample {
    fn is_smart(&self) -> bool {
        self.wear.is_some()
            || self.spare.is_some()
            || self.media_errors.is_some()
            || self.written_gb.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub(crate) struct DriveHistory {
    /// Model name, for a drive that is no longer attached.
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub samples: Vec<Sample>,
}

/// Every drive's history, keyed by drive id (serial).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub(crate) struct History {
    #[serde(default)]
    pub drives: BTreeMap<String, DriveHistory>,
}

impl History {
    fn drive(&mut self, id: &str, label: &str) -> &mut DriveHistory {
        let d = self.drives.entry(id.to_string()).or_default();
        if !label.is_empty() {
            d.label = label.to_string();
        }
        d
    }

    /// Add a temperature sample unless the last one is younger than
    /// `interval` seconds. Returns whether it was added.
    pub(crate) fn record_temp(
        &mut self,
        id: &str,
        label: &str,
        t: u64,
        temp: i64,
        interval: u64,
    ) -> bool {
        let d = self.drive(id, label);
        let last = d
            .samples
            .iter()
            .rev()
            .find(|s| s.temp.is_some() && !s.is_smart())
            .map(|s| s.t);
        if last.is_some_and(|l| t < l.saturating_add(interval)) {
            return false;
        }
        d.samples.push(Sample {
            t,
            temp: Some(temp),
            ..Sample::default()
        });
        true
    }

    /// Add a sample from a full SMART read (replacing a SMART sample taken
    /// within the last [`SMART_MERGE_SECS`]).
    pub(crate) fn record_smart(&mut self, id: &str, t: u64, r: &SmartReport) {
        let sample = Sample {
            t,
            temp: r.temp_c,
            wear: r.wear_pct,
            spare: r.spare_pct,
            media_errors: r.media_errors,
            written_gb: r.written_tb.map(|tb| (tb * 1000.0).round() as u64),
        };
        if !sample.is_smart() {
            return;
        }
        let d = self.drive(id, &r.model);
        if let Some(last) = d.samples.last_mut()
            && last.is_smart()
            && t.saturating_sub(last.t) < SMART_MERGE_SECS
        {
            *last = sample;
            return;
        }
        d.samples.push(sample);
    }

    /// Drop samples older than `retention` seconds before `now`, keep at
    /// most [`MAX_SAMPLES`] per drive, drop drives left empty.
    pub(crate) fn prune(&mut self, now: u64, retention: u64) {
        let cutoff = now.saturating_sub(retention);
        for d in self.drives.values_mut() {
            d.samples.retain(|s| s.t >= cutoff);
            d.samples.sort_by_key(|s| s.t);
            let excess = d.samples.len().saturating_sub(MAX_SAMPLES);
            d.samples.drain(..excess);
        }
        self.drives.retain(|_, d| !d.samples.is_empty());
    }

    /// `(t, value)` points of one metric for one drive, oldest first.
    pub(crate) fn series(
        &self,
        id: &str,
        metric: impl Fn(&Sample) -> Option<f64>,
    ) -> Vec<(u64, f64)> {
        self.drives
            .get(id)
            .map(|d| {
                d.samples
                    .iter()
                    .filter_map(|s| metric(s).map(|v| (s.t, v)))
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temperature_samples_respect_the_interval() {
        let mut h = History::default();
        assert!(h.record_temp("S1", "Disk", 1000, 40, 3600));
        assert!(!h.record_temp("S1", "Disk", 2000, 41, 3600));
        assert!(h.record_temp("S1", "Disk", 4600, 42, 3600));
        let s = h.series("S1", |s| s.temp.map(|t| t as f64));
        assert_eq!(s, [(1000, 40.0), (4600, 42.0)]);
        assert!(h.series("nope", |s| s.temp.map(|t| t as f64)).is_empty());
    }

    fn rep(wear: u64) -> SmartReport {
        SmartReport {
            dev: "/dev/nvme0".into(),
            model: "Disk".into(),
            temp_c: Some(45),
            wear_pct: Some(wear),
            media_errors: Some(0),
            written_tb: Some(15.6474),
            ..SmartReport::default()
        }
    }

    #[test]
    fn smart_samples_merge_when_read_twice_in_a_row() {
        let mut h = History::default();
        h.record_smart("S1", 1000, &rep(3));
        h.record_smart("S1", 1200, &rep(4));
        h.record_smart("S1", 5000, &rep(5));
        let wear = h.series("S1", |s| s.wear.map(|w| w as f64));
        assert_eq!(wear, [(1200, 4.0), (5000, 5.0)]);
        assert_eq!(h.drives["S1"].samples[0].written_gb, Some(15647));
        // A SMART sample doesn't block the next temperature sample.
        assert!(h.record_temp("S1", "", 5001, 44, 3600));
        assert_eq!(h.drives["S1"].label, "Disk");
        // A report with no SMART counters adds nothing.
        h.record_smart("S2", 1, &SmartReport::default());
        assert!(!h.drives.contains_key("S2"));
    }

    #[test]
    fn prune_drops_old_samples_caps_and_empty_drives() {
        let mut h = History::default();
        for t in 0..(MAX_SAMPLES as u64 + 10) {
            h.record_temp("S1", "Disk", 1_000_000 + t, 40, 0);
        }
        h.record_temp("OLD", "Gone", 10, 40, 0);
        h.prune(1_000_000 + MAX_SAMPLES as u64 + 10, 10 * 86_400);
        assert_eq!(h.drives["S1"].samples.len(), MAX_SAMPLES);
        assert_eq!(h.drives["S1"].samples[0].t, 1_000_010, "oldest dropped");
        assert!(!h.drives.contains_key("OLD"));
    }

    #[test]
    fn round_trips_through_json_compactly() {
        let mut h = History::default();
        h.record_temp("S1", "Disk", 1000, 40, 3600);
        let json = serde_json::to_string(&h).unwrap();
        assert_eq!(
            json,
            r#"{"drives":{"S1":{"label":"Disk","samples":[{"t":1000,"temp":40}]}}}"#
        );
        assert_eq!(serde_json::from_str::<History>(&json).unwrap(), h);
        assert_eq!(
            serde_json::from_str::<History>("{}").unwrap(),
            History::default()
        );
    }
}
