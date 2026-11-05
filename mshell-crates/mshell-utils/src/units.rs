//! Byte and rate formatting — one convention for every number the shell
//! shows (DESIGN §1 "Units").
//!
//! * **Sizes** (memory, disks, files, packages, images): IEC binary units,
//!   1024-based — `B`, `KiB`, `MiB`, `GiB`, `TiB`, `PiB`. [`bytes`], [`kib`].
//! * **Rates** (network / disk throughput): SI decimal units, 1000-based —
//!   `B/s`, `kB/s`, `MB/s`, `GB/s`, `TB/s`. [`rate`]; the bar pill's
//!   width-stable form is [`rate_compact`] (`4.2k`, same SI prefixes).
//!   Link / speed-test figures in bits stay SI too (`Mbit/s`, 10⁶ bit/s).
//!
//! Precision is the same everywhere: one decimal below 100 of a unit
//! (`4.2 MiB`, `96.0 MiB`), whole numbers from 100 up (`217 MiB`), plain
//! bytes always whole (`512 B`). A value that would round up to the next
//! unit's base moves to that unit, so `1023.97 KiB` reads `1.0 MiB`.

const IEC: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
const SI_RATE: [&str; 5] = ["B/s", "kB/s", "MB/s", "GB/s", "TB/s"];
const SI_COMPACT: [&str; 5] = ["B", "k", "M", "G", "T"];

/// A byte count as an IEC size: `512 B`, `4.2 KiB`, `217 MiB`, `1.5 GiB`.
pub fn bytes(n: u64) -> String {
    scaled(n as f64, 1024.0, &IEC, 100.0, " ")
}

/// A size given in KiB (`/proc/meminfo`, `/proc/<pid>/status`, `df -k`).
pub fn kib(n: u64) -> String {
    bytes(n.saturating_mul(1024))
}

/// Bytes per second as an SI rate: `512 B/s`, `4.2 kB/s`, `1.5 MB/s`.
pub fn rate(bytes_per_sec: f64) -> String {
    scaled(bytes_per_sec.max(0.0), 1000.0, &SI_RATE, 100.0, " ")
}

/// The bar pill's compact rate (no `/s`, no space; the pill's arrows say
/// what it is): `0B`, `512B`, `4.2k`, `42k`, `1.5M`. Same SI prefixes as
/// [`rate`]; one decimal below 10 so the label stays ≤ 4 characters.
pub fn rate_compact(bytes_per_sec: u64) -> String {
    scaled(bytes_per_sec as f64, 1000.0, &SI_COMPACT, 10.0, "")
}

/// `v` in the largest unit that keeps it below `base`, one decimal below
/// `decimals_below`, whole numbers above; the base unit is always whole.
fn scaled(mut v: f64, base: f64, units: &[&str], decimals_below: f64, sep: &str) -> String {
    let mut i = 0;
    loop {
        let last = i + 1 == units.len();
        let tenths = (v * 10.0).round() / 10.0;
        let whole = i == 0 || tenths >= decimals_below;
        let shown = if whole { v.round() } else { tenths };
        if shown < base || last {
            return if whole {
                format!("{shown:.0}{sep}{}", units[i])
            } else {
                format!("{shown:.1}{sep}{}", units[i])
            };
        }
        v /= base;
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_iec() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(17), "17 B");
        assert_eq!(bytes(1023), "1023 B");
        assert_eq!(bytes(1024), "1.0 KiB");
        assert_eq!(bytes(524_288), "512 KiB");
        assert_eq!(bytes(100_642_324), "96.0 MiB");
        assert_eq!(bytes(1_610_612_736), "1.5 GiB");
        // A "32 GB" stick is 29.8 GiB — sizes never mix SI labels in.
        assert_eq!(bytes(32_000_000_000), "29.8 GiB");
        assert_eq!(bytes(u64::MAX), "16384 PiB");
    }

    #[test]
    fn kib_inputs() {
        assert_eq!(kib(512), "512 KiB");
        assert_eq!(kib(222_496), "217 MiB");
        assert_eq!(kib(3 * 1024 * 1024), "3.0 GiB");
        assert_eq!(kib(16_000_000), "15.3 GiB");
    }

    /// A value just under the next base must not read `1024 KiB`.
    #[test]
    fn rounding_moves_up_a_unit() {
        assert_eq!(bytes(1024 * 1024 - 10), "1.0 MiB");
        assert_eq!(rate(999.6), "1.0 kB/s");
        assert_eq!(rate_compact(9_999), "10k");
        assert_eq!(rate_compact(999_999), "1.0M");
    }

    #[test]
    fn rates_are_si() {
        assert_eq!(rate(0.0), "0 B/s");
        assert_eq!(rate(-5.0), "0 B/s");
        assert_eq!(rate(512.0), "512 B/s");
        assert_eq!(rate(4_200.0), "4.2 kB/s");
        assert_eq!(rate(4_096.0), "4.1 kB/s");
        assert_eq!(rate(420_000.0), "420 kB/s");
        assert_eq!(rate(3_000_000.0), "3.0 MB/s");
        assert_eq!(rate(2.5e9), "2.5 GB/s");
    }

    #[test]
    fn compact_rate_for_the_bar() {
        assert_eq!(rate_compact(0), "0B");
        assert_eq!(rate_compact(512), "512B");
        assert_eq!(rate_compact(4_200), "4.2k");
        assert_eq!(rate_compact(42_000), "42k");
        assert_eq!(rate_compact(1_500_000), "1.5M");
        assert_eq!(rate_compact(15_000_000), "15M");
    }
}
