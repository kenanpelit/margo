//! Aggregate CPU usage via /proc/stat.
//!
//! `(idle_delta / total_delta) * 100` between two consecutive
//! reads; the caller is expected to keep a `Sampler` alive across
//! polls so the deltas are meaningful (the first sample seeds the
//! state and reports 0 %).

use std::cell::Cell;

#[derive(Default)]
pub struct Sampler {
    prev_idle: Cell<u64>,
    prev_total: Cell<u64>,
}

impl Sampler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read /proc/stat and return current CPU usage in 0..=100.
    /// Returns 0 on the very first call (no delta yet).
    pub fn sample(&self) -> u8 {
        let Some((idle, total)) = read_aggregate() else {
            return 0;
        };
        let prev_idle = self.prev_idle.replace(idle);
        let prev_total = self.prev_total.replace(total);
        if prev_total == 0 || total <= prev_total {
            return 0;
        }
        let didle = idle.saturating_sub(prev_idle);
        let dtotal = total - prev_total;
        if dtotal == 0 {
            return 0;
        }
        let busy = dtotal.saturating_sub(didle);
        ((busy as f64 / dtotal as f64) * 100.0).round().clamp(0.0, 100.0) as u8
    }
}

fn read_aggregate() -> Option<(u64, u64)> {
    let raw = std::fs::read_to_string("/proc/stat").ok()?;
    let line = raw.lines().next()?; // "cpu  user nice system idle iowait irq softirq …"
    let mut parts = line.split_whitespace();
    if parts.next()? != "cpu" {
        return None;
    }
    let values: Vec<u64> = parts.filter_map(|p| p.parse().ok()).collect();
    if values.len() < 4 {
        return None;
    }
    let idle = values[3] + values.get(4).copied().unwrap_or(0); // idle + iowait
    let total: u64 = values.iter().sum();
    Some((idle, total))
}
