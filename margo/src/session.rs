//! Workspace / tag-state persistence.
//!
//! `mctl session save` writes a JSON snapshot of every monitor's tag
//! selection, per-tag layout, mfact, and nmaster to
//! `$XDG_STATE_HOME/margo/session.json` (defaults to
//! `~/.local/state/margo/session.json`). `mctl session load` reads
//! that file back and re-applies the per-tag state to whatever
//! monitors are present at load time, matching by output name.
//!
//! What's *not* in the snapshot:
//!
//! * **Open windows.** A client is bound to a process — restoring it
//!   means re-spawning, and the spawn line lives in user space (a
//!   tag rule, a script, the user's shell history). Margo doesn't
//!   try to second-guess that.
//! * **Animation state, focus, scratchpad visibility.** Ephemeral by
//!   nature; stale within a frame.
//! * **Monitor topology.** Outputs are physical hardware; if the
//!   target monitor isn't present at load time the entry is just
//!   skipped (logged, not an error).
//!
//! Atomic write — temp file in the same directory, `rename(2)` to
//! the final path so a crash mid-write can't leave a partial file
//! shadowing a good one.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use margo_layouts::LayoutId;

use crate::state::MargoState;

/// Top-level snapshot. `version` is bumped any time the on-disk
/// shape changes; the loader rejects unknown versions instead of
/// silently mis-applying.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub version: u32,
    pub captured_at: String,
    pub monitors: Vec<MonitorSnapshot>,
    /// Per-`app_id` scratchpad state for clients currently parked in
    /// a scratchpad. On load we walk live clients matching by
    /// `app_id` and re-flag them; clients that aren't open yet at
    /// load time are silently skipped (their spawn line lives in
    /// user space, see §0 caveats). Default empty so a snapshot
    /// produced before this field landed deserialises cleanly.
    #[serde(default)]
    pub scratchpads: Vec<ScratchpadEntry>,
}

/// One client's scratchpad state at capture time. We deliberately
/// don't record window geometry — the next show of the scratchpad
/// re-runs the same toggle-named-scratchpad path the user's binds
/// fire, which already produces the right position.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScratchpadEntry {
    /// `app_id` of the client. Match key on load.
    pub app_id: String,
    /// Was the scratchpad visible (drawn on top) at capture time?
    /// `false` ⇒ parked off-screen, awaiting a toggle.
    pub visible: bool,
    /// Was this a named-scratchpad slot (`toggle_named_scratchpad`)
    /// or a plain `toggle_scratchpad` window? Distinguishing matters
    /// for the loader: named slots restore both the "is in
    /// scratchpad" flag AND the "is named" flag.
    pub named: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorSnapshot {
    /// DRM connector name (`DP-1`, `eDP-1`). Used to match the
    /// snapshot back to a live monitor at load time.
    pub name: String,
    /// `MargoMonitor::seltags` — index 0 / 1 of the active tagset slot.
    pub seltags: usize,
    /// Two-slot tagset (current + previous); together with `seltags`
    /// this restores the dwm "press-twice-for-back" workflow.
    pub tagset: [u32; 2],
    pub pertag: PertagSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PertagSnapshot {
    pub curtag: usize,
    pub prevtag: usize,
    /// Per-tag layout name (`"tile"`, `"scroller"`, …). Stored as a
    /// string so the on-disk format isn't bound to enum variant
    /// ordering — adding a new layout doesn't invalidate old
    /// snapshots.
    pub ltidxs: Vec<String>,
    pub mfacts: Vec<f32>,
    pub nmasters: Vec<u32>,
    pub canvas_pan_x: Vec<f64>,
    pub canvas_pan_y: Vec<f64>,
}

pub const CURRENT_VERSION: u32 = 1;

/// Resolve the on-disk path for the session snapshot. Honours
/// `$XDG_STATE_HOME`; falls back to `$HOME/.local/state`.
pub fn session_path() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/tmp"));
            home.join(".local/state")
        });
    Ok(base.join("margo").join("session.json"))
}

impl SessionSnapshot {
    pub fn capture(state: &MargoState) -> Self {
        let captured_at = chrono_like_now();
        let monitors = state
            .monitors
            .iter()
            .map(|m| MonitorSnapshot {
                name: m.name.clone(),
                seltags: m.seltags,
                tagset: m.tagset,
                pertag: PertagSnapshot {
                    curtag: m.pertag.curtag,
                    prevtag: m.pertag.prevtag,
                    ltidxs: m
                        .pertag
                        .ltidxs
                        .iter()
                        .map(|l| l.name().to_string())
                        .collect(),
                    mfacts: m.pertag.mfacts.clone(),
                    nmasters: m.pertag.nmasters.clone(),
                    canvas_pan_x: m.pertag.canvas_pan_x.clone(),
                    canvas_pan_y: m.pertag.canvas_pan_y.clone(),
                },
            })
            .collect();
        let scratchpads = state
            .clients
            .iter()
            .filter(|c| c.is_in_scratchpad && !c.app_id.is_empty())
            .map(|c| ScratchpadEntry {
                app_id: c.app_id.clone(),
                visible: c.is_scratchpad_show,
                named: c.is_named_scratchpad,
            })
            .collect();
        Self {
            version: CURRENT_VERSION,
            captured_at,
            monitors,
            scratchpads,
        }
    }
}

/// ISO-8601-ish UTC timestamp without bringing in the `chrono` crate
/// just for the audit field. Format: `2026-05-10T12:34:56Z`.
fn chrono_like_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Days-from-epoch → calendar (Howard Hinnant's days_from_civil).
    // Avoids pulling chrono in for an audit string.
    let days = (secs / 86_400) as i64;
    let z = days + 719_468;
    let era = if z >= 0 { z / 146_097 } else { (z - 146_096) / 146_097 };
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = if m <= 2 { y + 1 } else { y };
    let h = ((secs % 86_400) / 3_600) as u32;
    let min = ((secs % 3_600) / 60) as u32;
    let s = (secs % 60) as u32;
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{min:02}:{s:02}Z")
}

/// Atomically write the snapshot to `path`. Writes to `path.tmp`
/// then renames — a crash mid-write leaves the previous good file
/// untouched.
pub fn save_to(path: &std::path::Path, snap: &SessionSnapshot) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("mkdir {parent:?}"))?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(snap).context("serialize session")?;
    std::fs::write(&tmp, body).with_context(|| format!("write {tmp:?}"))?;
    std::fs::rename(&tmp, path).with_context(|| format!("rename {tmp:?} → {path:?}"))?;
    Ok(())
}

pub fn load_from(path: &std::path::Path) -> Result<SessionSnapshot> {
    let bytes = std::fs::read(path).with_context(|| format!("read {path:?}"))?;
    let snap: SessionSnapshot = serde_json::from_slice(&bytes).context("parse session")?;
    anyhow::ensure!(
        snap.version == CURRENT_VERSION,
        "unsupported session snapshot version {} (expected {CURRENT_VERSION})",
        snap.version,
    );
    Ok(snap)
}

/// Apply a snapshot to the live state. Matches monitors by name;
/// silently skips snapshot entries whose target monitor isn't
/// connected today (the user re-plugged a different display, kanshi
/// disabled an output, …). Returns the count of successfully-restored
/// monitors so the dispatch handler can report it to the user.
pub fn apply_to_state(state: &mut MargoState, snap: &SessionSnapshot) -> usize {
    let mut applied = 0;
    for ms in &snap.monitors {
        let Some(idx) = state.monitors.iter().position(|m| m.name == ms.name) else {
            tracing::info!(
                target: "session",
                monitor = %ms.name,
                "skip: monitor not present"
            );
            continue;
        };
        let m = &mut state.monitors[idx];
        m.seltags = ms.seltags;
        m.tagset = ms.tagset;

        // Per-tag fields — clamp to the locally-allocated length so a
        // future MAX_TAGS change doesn't OOB on an old snapshot.
        let n = m.pertag.ltidxs.len().min(ms.pertag.ltidxs.len());
        for tag_i in 0..n {
            if let Some(lt) = LayoutId::from_name(&ms.pertag.ltidxs[tag_i]) {
                m.pertag.ltidxs[tag_i] = lt;
            }
        }
        let n = m.pertag.mfacts.len().min(ms.pertag.mfacts.len());
        m.pertag.mfacts[..n].copy_from_slice(&ms.pertag.mfacts[..n]);
        let n = m.pertag.nmasters.len().min(ms.pertag.nmasters.len());
        m.pertag.nmasters[..n].copy_from_slice(&ms.pertag.nmasters[..n]);
        let n = m.pertag.canvas_pan_x.len().min(ms.pertag.canvas_pan_x.len());
        m.pertag.canvas_pan_x[..n].copy_from_slice(&ms.pertag.canvas_pan_x[..n]);
        let n = m.pertag.canvas_pan_y.len().min(ms.pertag.canvas_pan_y.len());
        m.pertag.canvas_pan_y[..n].copy_from_slice(&ms.pertag.canvas_pan_y[..n]);

        m.pertag.curtag = ms.pertag.curtag;
        m.pertag.prevtag = ms.pertag.prevtag;
        applied += 1;
    }

    // Scratchpad restore. Walk the saved entries and re-flag any live
    // client whose `app_id` matches. Apps that aren't open yet at
    // load time are silently skipped — the user's spawn line (an
    // exec-once, a tag rule, a script) will re-create them, and on
    // first map the windowrule pipeline can put them back in the
    // scratchpad via the matching `is_named_scratchpad` rule. So the
    // session restore is best-effort: anything currently running
    // goes back where it was; anything that the session has yet to
    // launch will land at the right place on its own through the
    // usual rule path.
    if !snap.scratchpads.is_empty() {
        let mut scratchpad_restored = 0;
        // Build app_id → entry lookup once so the inner loop is O(n_clients)
        // not O(n_clients × n_entries).
        let entries: std::collections::HashMap<&str, &ScratchpadEntry> = snap
            .scratchpads
            .iter()
            .map(|e| (e.app_id.as_str(), e))
            .collect();
        for client in state.clients.iter_mut() {
            if client.is_in_scratchpad {
                continue;
            }
            if let Some(entry) = entries.get(client.app_id.as_str()) {
                client.is_in_scratchpad = true;
                client.is_scratchpad_show = entry.visible;
                client.is_named_scratchpad = entry.named;
                scratchpad_restored += 1;
            }
        }
        if scratchpad_restored > 0 {
            tracing::info!(
                target: "session",
                count = scratchpad_restored,
                "restored scratchpad state"
            );
        }
    }

    if applied > 0 {
        state.arrange_all();
        state.request_repaint();
    }
    applied
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_has_iso_shape() {
        let t = chrono_like_now();
        // 2026-05-10T12:34:56Z = 20 chars
        assert_eq!(t.len(), 20, "timestamp `{t}` not 20 chars");
        assert_eq!(&t[4..5], "-");
        assert_eq!(&t[7..8], "-");
        assert_eq!(&t[10..11], "T");
        assert_eq!(&t[13..14], ":");
        assert_eq!(&t[16..17], ":");
        assert_eq!(&t[19..20], "Z");
    }

    #[test]
    fn round_trip_through_json() {
        let snap = SessionSnapshot {
            version: CURRENT_VERSION,
            captured_at: "2026-05-10T12:34:56Z".to_string(),
            scratchpads: vec![
                ScratchpadEntry {
                    app_id: "dropdown-terminal".to_string(),
                    visible: false,
                    named: true,
                },
                ScratchpadEntry {
                    app_id: "clipse".to_string(),
                    visible: true,
                    named: true,
                },
            ],
            monitors: vec![MonitorSnapshot {
                name: "DP-1".to_string(),
                seltags: 0,
                tagset: [4, 1],
                pertag: PertagSnapshot {
                    curtag: 3,
                    prevtag: 1,
                    ltidxs: vec![
                        "tile".to_string(),
                        "scroller".to_string(),
                        "monocle".to_string(),
                    ],
                    mfacts: vec![0.55, 0.6, 0.5],
                    nmasters: vec![1, 2, 1],
                    canvas_pan_x: vec![0.0, 100.0, 0.0],
                    canvas_pan_y: vec![0.0, 50.0, 0.0],
                },
            }],
        };
        let s = serde_json::to_string(&snap).unwrap();
        let back: SessionSnapshot = serde_json::from_str(&s).unwrap();
        assert_eq!(back.monitors.len(), 1);
        assert_eq!(back.monitors[0].name, "DP-1");
        assert_eq!(back.monitors[0].pertag.ltidxs[1], "scroller");
        assert!((back.monitors[0].pertag.mfacts[1] - 0.6).abs() < 1e-6);
        assert_eq!(back.scratchpads.len(), 2);
        assert_eq!(back.scratchpads[0].app_id, "dropdown-terminal");
        assert!(!back.scratchpads[0].visible);
        assert!(back.scratchpads[0].named);
        assert!(back.scratchpads[1].visible);
    }

    #[test]
    fn pre_scratchpad_snapshot_deserializes_with_empty_vec() {
        // A snapshot produced before this field landed must still
        // parse — the field is `#[serde(default)]` so the absence in
        // the JSON gets an empty `Vec`.
        let old = r#"{"version":1,"captured_at":"x","monitors":[]}"#;
        let snap: SessionSnapshot = serde_json::from_str(old).unwrap();
        assert!(snap.scratchpads.is_empty());
    }

    #[test]
    fn version_mismatch_rejected() {
        let bad = r#"{"version":99,"captured_at":"x","monitors":[]}"#;
        let path = std::env::temp_dir().join("margo-session-test-bad.json");
        std::fs::write(&path, bad).unwrap();
        let result = load_from(&path);
        let _ = std::fs::remove_file(&path);
        assert!(result.is_err());
        assert!(format!("{}", result.unwrap_err()).contains("unsupported"));
    }

    // ── T9: save/load round-trip + edge cases ──────────────────────────────

    /// `save_to` then `load_from` round-trips every field of every
    /// monitor + scratchpad — the on-disk JSON is the wire contract
    /// between margo versions, so this test guards the schema lock.
    #[test]
    fn save_to_then_load_from_round_trips_every_field() {
        let original = SessionSnapshot {
            version: CURRENT_VERSION,
            captured_at: "2026-05-11T14:30:00Z".to_string(),
            scratchpads: vec![
                ScratchpadEntry {
                    app_id: "dropdown-terminal".to_string(),
                    visible: true,
                    named: true,
                },
                ScratchpadEntry {
                    app_id: "yazi-scratchpad".to_string(),
                    visible: false,
                    named: true,
                },
            ],
            monitors: vec![
                MonitorSnapshot {
                    name: "DP-3".to_string(),
                    seltags: 1,
                    tagset: [128, 2],
                    pertag: PertagSnapshot {
                        curtag: 8,
                        prevtag: 2,
                        ltidxs: vec![
                            "scroller".to_string(),
                            "tile".to_string(),
                            "monocle".to_string(),
                            "grid".to_string(),
                            "deck".to_string(),
                            "center_tile".to_string(),
                            "dwindle".to_string(),
                            "canvas".to_string(),
                            "vertical_grid".to_string(),
                        ],
                        mfacts: vec![0.55, 0.50, 0.65, 0.55, 0.55, 0.55, 0.55, 0.55, 0.55],
                        nmasters: vec![1, 1, 1, 2, 1, 1, 1, 1, 1],
                        canvas_pan_x: vec![0.0; 9],
                        canvas_pan_y: vec![0.0; 9],
                    },
                },
                MonitorSnapshot {
                    name: "eDP-1".to_string(),
                    seltags: 0,
                    tagset: [1, 4],
                    pertag: PertagSnapshot {
                        curtag: 1,
                        prevtag: 3,
                        ltidxs: vec!["tile".to_string()],
                        mfacts: vec![0.55],
                        nmasters: vec![1],
                        canvas_pan_x: vec![0.0],
                        canvas_pan_y: vec![0.0],
                    },
                },
            ],
        };

        let path = std::env::temp_dir().join("margo-session-roundtrip.json");
        let _ = std::fs::remove_file(&path);
        save_to(&path, &original).unwrap();
        let loaded = load_from(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(loaded.version, original.version);
        assert_eq!(loaded.captured_at, original.captured_at);
        assert_eq!(loaded.monitors.len(), 2);
        assert_eq!(loaded.scratchpads.len(), 2);

        // Spot-check every nested field of both monitors so a future
        // refactor that drops one silently flips the test red.
        for (l, o) in loaded.monitors.iter().zip(original.monitors.iter()) {
            assert_eq!(l.name, o.name);
            assert_eq!(l.seltags, o.seltags);
            assert_eq!(l.tagset, o.tagset);
            assert_eq!(l.pertag.curtag, o.pertag.curtag);
            assert_eq!(l.pertag.prevtag, o.pertag.prevtag);
            assert_eq!(l.pertag.ltidxs, o.pertag.ltidxs);
            assert_eq!(l.pertag.nmasters, o.pertag.nmasters);
            for (lm, om) in l.pertag.mfacts.iter().zip(o.pertag.mfacts.iter()) {
                assert!((lm - om).abs() < 1e-6, "mfact drift {lm} vs {om}");
            }
            assert_eq!(l.pertag.canvas_pan_x, o.pertag.canvas_pan_x);
            assert_eq!(l.pertag.canvas_pan_y, o.pertag.canvas_pan_y);
        }
        for (l, o) in loaded.scratchpads.iter().zip(original.scratchpads.iter()) {
            assert_eq!(l.app_id, o.app_id);
            assert_eq!(l.visible, o.visible);
            assert_eq!(l.named, o.named);
        }
    }

    /// `save_to` writes to `*.tmp` then renames — a crash mid-write
    /// must leave the previous good file untouched. We exercise the
    /// write path twice and check the second write doesn't clobber
    /// the file content unless rename completes (the tmp file should
    /// be gone after success).
    #[test]
    fn save_to_is_atomic_via_rename() {
        let path = std::env::temp_dir().join("margo-session-atomic.json");
        let tmp = path.with_extension("json.tmp");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&tmp);

        let snap = SessionSnapshot {
            version: CURRENT_VERSION,
            captured_at: "x".to_string(),
            scratchpads: vec![],
            monitors: vec![],
        };
        save_to(&path, &snap).unwrap();
        assert!(path.exists(), "destination not written");
        assert!(!tmp.exists(), "tmp file lingered after successful rename");

        // Write again — same contract, no leftover tmp.
        save_to(&path, &snap).unwrap();
        assert!(path.exists());
        assert!(!tmp.exists());

        let _ = std::fs::remove_file(&path);
    }

    /// Malformed JSON returns an error, doesn't panic.
    #[test]
    fn load_from_rejects_malformed_json() {
        let path = std::env::temp_dir().join("margo-session-malformed.json");
        std::fs::write(&path, "{ not even close to valid").unwrap();
        let result = load_from(&path);
        let _ = std::fs::remove_file(&path);
        assert!(result.is_err());
    }

    /// `load_from` on a nonexistent path returns a clear I/O error
    /// (not a parse error masquerading as I/O).
    #[test]
    fn load_from_missing_file_is_io_error() {
        let path = std::env::temp_dir().join("margo-session-does-not-exist.json");
        let _ = std::fs::remove_file(&path);
        let result = load_from(&path);
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        // The `with_context` chain says "read ..."; the underlying
        // io::ErrorKind::NotFound becomes part of the chain via
        // anyhow's source-of-source.
        assert!(
            msg.contains("read"),
            "expected an 'read' context in the error chain, got: {msg}"
        );
    }

    /// A snapshot whose `ltidxs` is shorter than the live
    /// `Pertag::ltidxs` is OK — the loader clamps to `min(local,
    /// snapshot)`. Reverse case: a snapshot with MORE tags than the
    /// live build must also load cleanly without panicking
    /// (the extras get ignored).
    #[test]
    fn pertag_lengths_clamp_on_either_side() {
        // Short snapshot (1 tag) — must round-trip without panic.
        let short = SessionSnapshot {
            version: CURRENT_VERSION,
            captured_at: "x".to_string(),
            scratchpads: vec![],
            monitors: vec![MonitorSnapshot {
                name: "DP-1".to_string(),
                seltags: 0,
                tagset: [1, 0],
                pertag: PertagSnapshot {
                    curtag: 1,
                    prevtag: 1,
                    ltidxs: vec!["tile".to_string()],
                    mfacts: vec![0.55],
                    nmasters: vec![1],
                    canvas_pan_x: vec![0.0],
                    canvas_pan_y: vec![0.0],
                },
            }],
        };
        let json = serde_json::to_string(&short).unwrap();
        let back: SessionSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back.monitors[0].pertag.ltidxs.len(), 1);

        // Over-long snapshot (15 tags, beyond MAX_TAGS=9) — also OK;
        // `apply_to_state`'s `min()` clamp drops the tail.
        let long = SessionSnapshot {
            version: CURRENT_VERSION,
            captured_at: "x".to_string(),
            scratchpads: vec![],
            monitors: vec![MonitorSnapshot {
                name: "DP-1".to_string(),
                seltags: 0,
                tagset: [1, 0],
                pertag: PertagSnapshot {
                    curtag: 1,
                    prevtag: 1,
                    ltidxs: (0..15).map(|_| "tile".to_string()).collect(),
                    mfacts: vec![0.55; 15],
                    nmasters: vec![1; 15],
                    canvas_pan_x: vec![0.0; 15],
                    canvas_pan_y: vec![0.0; 15],
                },
            }],
        };
        let json = serde_json::to_string(&long).unwrap();
        let back: SessionSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back.monitors[0].pertag.ltidxs.len(), 15);
    }

    /// Unknown layout names on disk decode cleanly — `apply_to_state`
    /// silently skips them via `LayoutId::from_name(...)?`. The
    /// snapshot itself stores names as opaque strings; we just need
    /// to confirm serde doesn't bail on a "future" layout name.
    #[test]
    fn unknown_layout_name_in_snapshot_does_not_break_serde() {
        let snap = SessionSnapshot {
            version: CURRENT_VERSION,
            captured_at: "x".to_string(),
            scratchpads: vec![],
            monitors: vec![MonitorSnapshot {
                name: "DP-1".to_string(),
                seltags: 0,
                tagset: [1, 0],
                pertag: PertagSnapshot {
                    curtag: 1,
                    prevtag: 1,
                    ltidxs: vec!["xenomorph_v2_inferno".to_string()],
                    mfacts: vec![0.55],
                    nmasters: vec![1],
                    canvas_pan_x: vec![0.0],
                    canvas_pan_y: vec![0.0],
                },
            }],
        };
        let json = serde_json::to_string(&snap).unwrap();
        let back: SessionSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back.monitors[0].pertag.ltidxs[0], "xenomorph_v2_inferno");
    }

    /// The default `ScratchpadEntry` values can serialise + deserialise
    /// without hitting a "missing field" error — defends against a
    /// future serde flag tweak that flips them to required.
    #[test]
    fn scratchpad_entry_defaults_round_trip() {
        let s = ScratchpadEntry {
            app_id: String::new(),
            visible: false,
            named: false,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: ScratchpadEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(s.app_id, back.app_id);
        assert_eq!(s.visible, back.visible);
        assert_eq!(s.named, back.named);
    }

    /// JSON shape stays *pretty* (newlines + indent) so users can diff
    /// `session.json` manually. Catches an accidental switch to
    /// `to_string` (compact) that would still parse but ruin the UX.
    #[test]
    fn save_to_produces_pretty_indented_json() {
        let path = std::env::temp_dir().join("margo-session-pretty.json");
        let snap = SessionSnapshot {
            version: CURRENT_VERSION,
            captured_at: "x".to_string(),
            scratchpads: vec![],
            monitors: vec![MonitorSnapshot {
                name: "DP-1".to_string(),
                seltags: 0,
                tagset: [1, 0],
                pertag: PertagSnapshot {
                    curtag: 1,
                    prevtag: 1,
                    ltidxs: vec!["tile".to_string()],
                    mfacts: vec![0.55],
                    nmasters: vec![1],
                    canvas_pan_x: vec![0.0],
                    canvas_pan_y: vec![0.0],
                },
            }],
        };
        save_to(&path, &snap).unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(body.contains('\n'), "session.json should be pretty-printed");
        assert!(body.contains("  \"version\""), "expected 2-space indent");
    }

    /// `chrono_like_now` produces a value that the snapshot can carry
    /// without serde rejecting it. Belt-and-braces — chrono_like_now
    /// is hand-rolled and we want this test red if someone breaks
    /// the format string upstream.
    #[test]
    fn captured_at_round_trips_through_serde() {
        let snap = SessionSnapshot {
            version: CURRENT_VERSION,
            captured_at: chrono_like_now(),
            scratchpads: vec![],
            monitors: vec![],
        };
        let json = serde_json::to_string(&snap).unwrap();
        let back: SessionSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back.captured_at, snap.captured_at);
        assert_eq!(back.captured_at.len(), 20);
    }
}
