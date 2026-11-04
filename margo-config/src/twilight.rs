//! Twilight schedule + preset files, parsed one way for margo and mshell.
//!
//! Layout (margo-owned):
//!
//! ```text
//!   <twilight_schedule_dir>/        default <config_home>/margo/twilight
//!     schedule.conf                 HH:MM PRESET_NAME, one per line
//!     presets/<name>.toml           `static_temp` + `static_gamma`
//! ```
//!
//! `static_temp` is Kelvin (clamped to 1000–25000), `static_gamma` a
//! brightness percentage (clamped to 10–200). The preset format is a tiny
//! flat `key = value` subset of TOML; `#` comments and `[section]` headers
//! are skipped. The compositor's schedule loader, the shell's Twilight menu
//! and Settings → Display used three copies of this, with slightly different
//! rules and two of them ignoring `twilight_schedule_dir`.

use std::path::{Path, PathBuf};

/// `static_temp` / `static_gamma` from a preset file's text, clamped.
pub fn parse_preset(text: &str) -> Option<(u32, u32)> {
    let mut temp_k: Option<u32> = None;
    let mut gamma_pct: Option<u32> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let val = v.trim().trim_matches('"');
        match k.trim() {
            "static_temp" => {
                if let Ok(n) = val.parse::<u32>() {
                    temp_k = Some(n.clamp(1000, 25000));
                }
            }
            "static_gamma" => {
                if let Ok(n) = val.parse::<u32>() {
                    gamma_pct = Some(n.clamp(10, 200));
                }
            }
            _ => {}
        }
    }
    Some((temp_k?, gamma_pct?))
}

/// `HH:MM` → seconds since midnight.
pub fn parse_hhmm(s: &str) -> Option<u32> {
    let (h, m) = s.split_once(':')?;
    let h: u32 = h.trim().parse().ok()?;
    let m: u32 = m.trim().parse().ok()?;
    if h >= 24 || m >= 60 {
        return None;
    }
    Some(h * 3600 + m * 60)
}

/// One valid `schedule.conf` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleLine {
    /// The time as written (`"07:30"`).
    pub time: String,
    /// Seconds since local midnight.
    pub time_sec: u32,
    pub preset: String,
    /// 1-based line number, for diagnostics.
    pub line: usize,
}

/// The valid lines of a `schedule.conf`, in file order. Blank lines and
/// `#` comments are skipped; a line without a valid `HH:MM` and a preset
/// name is returned in `Err` (line number, text) for the caller to report.
pub fn parse_schedule(text: &str) -> (Vec<ScheduleLine>, Vec<(usize, String)>) {
    let mut ok = Vec::new();
    let mut bad = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let t = raw.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let mut parts = t.split_whitespace();
        match (parts.next(), parts.next()) {
            (Some(time), Some(name)) => match parse_hhmm(time) {
                Some(time_sec) => ok.push(ScheduleLine {
                    time: time.to_string(),
                    time_sec,
                    preset: name.to_string(),
                    line: i + 1,
                }),
                None => bad.push((i + 1, t.to_string())),
            },
            _ => bad.push((i + 1, t.to_string())),
        }
    }
    (ok, bad)
}

/// The schedule directory for a `twilight_schedule_dir` value: empty means
/// `<config_home>/margo/twilight`; a leading `~/` is the home directory.
pub fn resolve_schedule_dir(configured: &str) -> PathBuf {
    let raw = configured.trim();
    if raw.is_empty() {
        return margo_fs::xdg::margo_config_dir().join("twilight");
    }
    if let Some(rest) = raw.strip_prefix("~/")
        && let Some(home) = margo_fs::xdg::home()
    {
        return home.join(rest);
    }
    PathBuf::from(raw)
}

/// A preset with its values (and, from the schedule, its time).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preset {
    pub name: String,
    pub temp_k: u32,
    pub gamma_pct: u32,
    /// `HH:MM` when the preset comes from `schedule.conf`.
    pub time: Option<String>,
}

/// Read one `presets/<name>.toml`.
pub fn load_preset(dir: &Path, name: &str) -> Option<(u32, u32)> {
    let text = std::fs::read_to_string(dir.join("presets").join(format!("{name}.toml"))).ok()?;
    parse_preset(&text)
}

/// The scheduled presets in time order (lines whose preset doesn't load are
/// skipped). Empty when there is no readable `schedule.conf`.
pub fn load_schedule(dir: &Path) -> Vec<Preset> {
    let Ok(text) = std::fs::read_to_string(dir.join("schedule.conf")) else {
        return Vec::new();
    };
    let (lines, _) = parse_schedule(&text);
    let mut out: Vec<(u32, Preset)> = lines
        .into_iter()
        .filter_map(|l| {
            let (temp_k, gamma_pct) = load_preset(dir, &l.preset)?;
            Some((
                l.time_sec,
                Preset {
                    name: l.preset,
                    temp_k,
                    gamma_pct,
                    time: Some(l.time),
                },
            ))
        })
        .collect();
    out.sort_by_key(|(t, _)| *t);
    out.into_iter().map(|(_, p)| p).collect()
}

/// Every loadable `presets/*.toml`, by name.
pub fn list_presets(dir: &Path) -> Vec<Preset> {
    let mut out: Vec<Preset> = std::fs::read_dir(dir.join("presets"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) != Some("toml") {
                return None;
            }
            let name = p.file_stem()?.to_str()?.to_string();
            let (temp_k, gamma_pct) = parse_preset(&std::fs::read_to_string(&p).ok()?)?;
            Some(Preset {
                name,
                temp_k,
                gamma_pct,
                time: None,
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_values_are_parsed_and_clamped() {
        let text = "# margo twilight preset\n[meta]\nstatic_temp = 90000\nstatic_gamma = \"5\"\n";
        assert_eq!(parse_preset(text), Some((25000, 10)));
        assert_eq!(
            parse_preset("static_temp = 3000\n"),
            None,
            "both keys needed"
        );
        assert_eq!(
            parse_preset("# static_temp = 1\nstatic_temp = 3000\nstatic_gamma = 90\n"),
            Some((3000, 90)),
            "a commented key is ignored"
        );
    }

    #[test]
    fn schedule_lines_and_bad_lines() {
        let (ok, bad) =
            parse_schedule("# c\n\n07:30 morning\n25:00 nope\nlonely\n00:00 deep-night x\n");
        assert_eq!(
            ok.iter()
                .map(|l| (l.time.as_str(), l.time_sec, l.preset.as_str()))
                .collect::<Vec<_>>(),
            vec![("07:30", 27000, "morning"), ("00:00", 0, "deep-night")]
        );
        assert_eq!(bad.iter().map(|(n, _)| *n).collect::<Vec<_>>(), vec![4, 5]);
    }

    #[test]
    fn configured_dir_wins_and_tilde_expands() {
        assert_eq!(resolve_schedule_dir("/srv/tw"), PathBuf::from("/srv/tw"));
        if let Some(home) = margo_fs::xdg::home() {
            assert_eq!(resolve_schedule_dir("~/tw"), home.join("tw"));
        }
        assert!(resolve_schedule_dir("  ").ends_with("margo/twilight"));
    }

    #[test]
    fn load_schedule_reads_presets_in_time_order() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("presets")).unwrap();
        std::fs::write(
            dir.path().join("schedule.conf"),
            "19:00 sunset\n07:30 morning\n08:00 missing\n",
        )
        .unwrap();
        let preset = |t, g| format!("static_temp = {t}\nstatic_gamma = {g}\n");
        std::fs::write(dir.path().join("presets/sunset.toml"), preset(2400, 93)).unwrap();
        std::fs::write(dir.path().join("presets/morning.toml"), preset(2400, 95)).unwrap();
        let got = load_schedule(dir.path());
        assert_eq!(
            got.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["morning", "sunset"]
        );
        assert_eq!(list_presets(dir.path()).len(), 2);
    }
}
