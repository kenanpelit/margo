//! CPU busy sampling from `/proc/stat`.
//!
//! A sample captures the aggregate counters plus every per-core line; busy%
//! is computed from the delta between two samples (the only meaningful way —
//! `/proc/stat` holds cumulative jiffies since boot). We track the hottest
//! core separately from the aggregate so a single pegged core can still ask
//! for performance even when the average looks idle.

/// One reading of `/proc/stat`: `(total_jiffies, idle_jiffies)` for the
/// aggregate line and for each `cpuN` line.
#[derive(Debug, Clone)]
pub struct CpuSample {
    pub total: u64,
    pub idle: u64,
    pub cores: Vec<(u64, u64)>,
}

/// Read `/proc/stat` once. `None` if it can't be read/parsed.
pub fn sample() -> Option<CpuSample> {
    let stat = std::fs::read_to_string("/proc/stat").ok()?;
    let mut total = 0u64;
    let mut idle = 0u64;
    let mut cores = Vec::new();
    let mut saw_aggregate = false;

    for line in stat.lines() {
        let mut it = line.split_whitespace();
        let Some(label) = it.next() else { continue };
        if !label.starts_with("cpu") {
            continue;
        }
        // Fields after the label: user nice system idle iowait irq softirq …
        let nums: Vec<u64> = it.filter_map(|t| t.parse::<u64>().ok()).collect();
        if nums.len() < 5 {
            continue;
        }
        let t: u64 = nums.iter().sum();
        let id = nums[3] + nums[4]; // idle + iowait

        if label == "cpu" {
            total = t;
            idle = id;
            saw_aggregate = true;
        } else {
            cores.push((t, id));
        }
    }

    if !saw_aggregate {
        return None;
    }
    Some(CpuSample { total, idle, cores })
}

/// Busy% between two samples as `(aggregate, hottest_core)`. `None` if the
/// aggregate delta is empty (no time elapsed). When per-core data is
/// unusable the hottest-core figure falls back to the aggregate.
pub fn busy(prev: &CpuSample, cur: &CpuSample) -> Option<(f64, f64)> {
    let avg = pct(prev.total, prev.idle, cur.total, cur.idle)?;

    let mut max = 0.0_f64;
    let mut any_core = false;
    if prev.cores.len() == cur.cores.len() {
        for (p, c) in prev.cores.iter().zip(cur.cores.iter()) {
            if let Some(b) = pct(p.0, p.1, c.0, c.1) {
                any_core = true;
                if b > max {
                    max = b;
                }
            }
        }
    }
    let max = if any_core { max } else { avg };
    Some((avg, max))
}

/// Busy fraction (0–100) from one counter pair to the next.
fn pct(prev_total: u64, prev_idle: u64, total: u64, idle: u64) -> Option<f64> {
    let delta_total = total.checked_sub(prev_total)?;
    if delta_total == 0 {
        return None;
    }
    let delta_idle = idle.saturating_sub(prev_idle);
    let busy = delta_total.saturating_sub(delta_idle) as f64 / delta_total as f64 * 100.0;
    Some(busy.clamp(0.0, 100.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_busy_is_one_minus_idle_fraction() {
        // 100 total jiffies elapsed, 25 of them idle → 75% busy.
        let prev = CpuSample { total: 1000, idle: 500, cores: vec![] };
        let cur = CpuSample { total: 1100, idle: 525, cores: vec![] };
        let (avg, max) = busy(&prev, &cur).unwrap();
        assert!((avg - 75.0).abs() < 0.01);
        // No usable cores → max falls back to aggregate.
        assert!((max - 75.0).abs() < 0.01);
    }

    #[test]
    fn hottest_core_exceeds_average() {
        // Two cores: one pegged (0 idle of 100), one idle (90 idle of 100).
        let prev = CpuSample { total: 200, idle: 90, cores: vec![(100, 0), (100, 90)] };
        let cur = CpuSample {
            total: 400,
            idle: 180,
            cores: vec![(200, 0), (200, 180)],
        };
        let (avg, max) = busy(&prev, &cur).unwrap();
        assert!((max - 100.0).abs() < 0.01, "hottest core should read ~100%");
        assert!(avg < max, "average should be below the hottest core");
    }

    #[test]
    fn no_time_elapsed_is_none() {
        let s = CpuSample { total: 1000, idle: 500, cores: vec![] };
        assert!(busy(&s, &s).is_none());
    }
}
