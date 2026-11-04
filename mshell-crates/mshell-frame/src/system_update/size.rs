//! Estimated download size of the pending official-repo updates.
//!
//! `pacman -Si` is asked against the same temporary sync DB that
//! `checkupdates` just refreshed (so the sizes are for the *new*
//! versions, not whatever the system DB last saw), and packages whose
//! new file is already in the pacman cache are left out (pacman won't
//! download those again). AUR and Flatpak aren't sized: AUR builds
//! from source and Flatpak has no cheap size query.

use super::UpdateEntry;

pub(crate) const PKG_CACHE: &str = "/var/cache/pacman/pkg";

/// `checkupdates`' default temp DB: `${TMPDIR:-/tmp}/checkup-db-${UID}`.
pub(crate) fn checkupdates_db() -> std::path::PathBuf {
    // SAFETY: getuid() has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    std::env::temp_dir().join(format!("checkup-db-{uid}"))
}

/// Parse `Download Size   : 95.98 MiB` style values into bytes. pacman
/// prints B / KiB / MiB / GiB, and a comma decimal in some locales
/// (we run it with `LC_ALL=C`, but tolerate both).
pub(crate) fn parse_size(value: &str) -> Option<u64> {
    let mut parts = value.split_whitespace();
    let num: f64 = parts.next()?.replace(',', ".").parse().ok()?;
    let mult = match parts.next()? {
        "B" => 1.0,
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        "TiB" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some((num * mult).round() as u64)
}

/// `(name, download bytes)` per package in `pacman -Si` output. A
/// package present in several repos (CachyOS `cachyos-extra-v3` +
/// `extra`) is printed once per repo; pacman installs from the first,
/// so only the first block of each name counts.
pub(crate) fn parse_si(stdout: &str) -> Vec<(String, u64)> {
    let mut out: Vec<(String, u64)> = Vec::new();
    let mut name: Option<String> = None;
    for line in stdout.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        match key.trim() {
            "Name" => name = Some(value.trim().to_string()),
            "Download Size" => {
                if let (Some(n), Some(bytes)) = (name.take(), parse_size(value))
                    && !out.iter().any(|(seen, _)| *seen == n)
                {
                    out.push((n, bytes));
                }
            }
            _ => {}
        }
    }
    out
}

/// True when the package file for `name`-`version` is already in the
/// cache listing (`name-version-arch.pkg.tar.zst`, any arch/compression,
/// signatures ignored).
pub(crate) fn is_cached(name: &str, version: &str, cache_files: &[String]) -> bool {
    let prefix = format!("{name}-{version}-");
    cache_files.iter().any(|f| {
        f.strip_prefix(&prefix).is_some_and(|rest| {
            // The remainder is `arch.pkg.tar.*` — no further `-`, which
            // rules out `name-version-…` of a *different* package whose
            // name merely starts with ours.
            !rest.contains('-') && rest.contains(".pkg.tar") && !rest.ends_with(".sig")
        })
    })
}

/// Sum of what will actually be downloaded.
pub(crate) fn total_download(
    entries: &[&UpdateEntry],
    sizes: &[(String, u64)],
    cache_files: &[String],
) -> u64 {
    entries
        .iter()
        .filter(|e| {
            !e.new_version
                .as_deref()
                .is_some_and(|v| is_cached(&e.name, v, cache_files))
        })
        .filter_map(|e| sizes.iter().find(|(n, _)| *n == e.name).map(|(_, b)| *b))
        .sum()
}

/// Human size, binary units like pacman (`95.98 MiB`).
pub(crate) fn human(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut v = bytes as f64 / 1024.0;
    let mut unit = UNITS[0];
    for u in &UNITS[1..] {
        if v < 1024.0 {
            break;
        }
        v /= 1024.0;
        unit = u;
    }
    format!("{v:.1} {unit}")
}

/// Probe the size for the given repo entries. `None` when pacman is
/// missing or reports nothing (the panel then just omits the size).
pub(crate) async fn probe(entries: &[&UpdateEntry]) -> Option<u64> {
    if entries.is_empty() {
        return Some(0);
    }
    let db = checkupdates_db();
    let mut cmd = tokio::process::Command::new("pacman");
    cmd.env("LC_ALL", "C").arg("-Si");
    if db.join("sync").is_dir() {
        cmd.arg("--dbpath").arg(&db);
    }
    cmd.args(entries.iter().map(|e| e.name.as_str()));
    let out = cmd.output().await.ok()?;
    let sizes = parse_si(&String::from_utf8_lossy(&out.stdout));
    if sizes.is_empty() {
        return None;
    }
    let cache_files: Vec<String> = std::fs::read_dir(PKG_CACHE)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    Some(total_download(entries, &sizes, &cache_files))
}

#[cfg(test)]
mod tests {
    use super::super::Source;
    use super::*;

    const SI: &str = include_str!("testdata/pacman-si.txt");

    fn entry(name: &str, new: &str) -> UpdateEntry {
        UpdateEntry {
            source: Source::Repo,
            name: name.into(),
            old_version: None,
            new_version: Some(new.into()),
        }
    }

    #[test]
    fn parses_units() {
        assert_eq!(parse_size(" 95.98 MiB"), Some(100_642_324));
        assert_eq!(parse_size("512.00 KiB"), Some(524_288));
        assert_eq!(parse_size("1,50 GiB"), Some(1_610_612_736));
        assert_eq!(parse_size("17 B"), Some(17));
        assert_eq!(parse_size("lots"), None);
        assert_eq!(parse_size("1.0 PB"), None);
    }

    #[test]
    fn real_si_output_counts_each_package_once() {
        // Captured `LC_ALL=C pacman -Si electron43 firefox` on CachyOS:
        // firefox is listed twice (cachyos-extra-v3, then extra).
        let sizes = parse_si(SI);
        let names: Vec<_> = sizes.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["electron43", "firefox"]);
        assert_eq!(sizes[0].1, parse_size("95.98 MiB").unwrap());
        // The first repo's (cachyos-extra-v3) size, not extra's.
        assert_eq!(sizes[1].1, parse_size("85.59 MiB").unwrap());
    }

    #[test]
    fn cached_packages_are_not_downloaded_again() {
        let cache = vec![
            "firefox-159.0.1-1.1-x86_64_v3.pkg.tar.zst".to_string(),
            "firefox-159.0.1-1.1-x86_64_v3.pkg.tar.zst.sig".to_string(),
            "electron43-43.8.6-1-x86_64.pkg.tar.zst".to_string(),
        ];
        assert!(is_cached("firefox", "159.0.1-1.1", &cache));
        assert!(!is_cached("electron43", "43.8.7-1", &cache));
        // A longer name sharing the prefix is not a hit.
        let cache2 = vec!["firefox-i18n-159.0.1-1.1-any.pkg.tar.zst".to_string()];
        assert!(!is_cached("firefox", "i18n", &cache2));

        let sizes = parse_si(SI);
        let e1 = entry("electron43", "43.8.7-1");
        let e2 = entry("firefox", "159.0.1-1.1");
        let total = total_download(&[&e1, &e2], &sizes, &cache);
        assert_eq!(total, sizes[0].1);
        let total_all = total_download(&[&e1, &e2], &sizes, &[]);
        assert_eq!(total_all, sizes[0].1 + sizes[1].1);
    }

    #[test]
    fn human_sizes() {
        assert_eq!(human(17), "17 B");
        assert_eq!(human(524_288), "512.0 KiB");
        assert_eq!(human(100_642_324), "96.0 MiB");
        assert_eq!(human(1_610_612_736), "1.5 GiB");
    }
}
