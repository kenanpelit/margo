//! `audio` palette — switch PipeWire default sink/source via
//! `wpctl`.
//!
//! Parses `wpctl status` output (the same tree `wpctl status`
//! prints for humans), extracts sinks and sources, and surfaces
//! one row per device. The currently-default device gets a `*`
//! prefix in `wpctl`'s output, which we use to highlight the
//! active selection.
//!
//! Activation runs `wpctl set-default <id>`.

use crate::{item::LauncherItem, notify::toast, provider::Provider};
use std::process::Command;
use std::rc::Rc;

pub struct WireplumberProvider;

impl WireplumberProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WireplumberProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// One parsed device.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Device {
    /// Numeric wpctl id.
    id: u32,
    /// Human-readable name.
    name: String,
    /// True if it's the current default sink/source.
    is_default: bool,
    /// "sink" or "source".
    kind: &'static str,
}

/// Parse `wpctl status` output. Indented lines under each
/// header (`Sinks:` / `Sources:`) carry the id + name. The
/// default has a `*` marker.
fn parse_wpctl_status(stdout: &str) -> Vec<Device> {
    let mut devices = Vec::new();
    let mut kind: Option<&'static str> = None;
    for raw in stdout.lines() {
        let line = raw.trim_end();
        // `wpctl status` decorates lines with box-drawing
        // characters (`│ ├ └ ─`); strip them + surrounding
        // whitespace before checking for section headers like
        // `├─ Sinks:`. Without this the header check below
        // would never match on real output.
        let stripped = line.trim_start_matches(|c: char| {
            c == '│' || c == '├' || c == '└' || c == '─' || c.is_whitespace()
        });
        if stripped.starts_with("Sinks:") {
            kind = Some("sink");
            continue;
        }
        if stripped.starts_with("Sources:") {
            kind = Some("source");
            continue;
        }
        if stripped.starts_with("Sink endpoints")
            || stripped.starts_with("Source endpoints")
            || stripped.starts_with("Filters")
            || stripped.starts_with("Streams")
            || stripped.starts_with("Audio")
            || stripped.starts_with("Video")
            || stripped.starts_with("Settings")
        {
            kind = None;
            continue;
        }
        let Some(k) = kind else {
            continue;
        };
        // Device lines look like:
        //   │  *  62. Built-in Audio Analog Stereo  [vol: 0.40]
        // Tree characters already gone (we stripped them above
        // for the header check). The `stripped` value here
        // starts at `*  62. ...` or `62. ...` depending on
        // whether this is the current default.
        let mut cur = stripped;
        let is_default = if let Some(rest) = cur.strip_prefix('*') {
            cur = rest.trim_start();
            true
        } else {
            false
        };
        // Now expect "NN. Name [vol: …]"
        let dot = match cur.find('.') {
            Some(d) => d,
            None => continue,
        };
        let id_str = &cur[..dot];
        let Ok(id) = id_str.trim().parse::<u32>() else {
            continue;
        };
        let after_id = cur[dot + 1..].trim_start();
        let name = match after_id.find('[') {
            Some(b) => after_id[..b].trim().to_string(),
            None => after_id.trim().to_string(),
        };
        devices.push(Device {
            id,
            name,
            is_default,
            kind: k,
        });
    }
    devices
}

fn snapshot() -> Vec<Device> {
    Command::new("wpctl")
        .arg("status")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| parse_wpctl_status(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

impl Provider for WireplumberProvider {
    fn name(&self) -> &str {
        "Audio"
    }

    fn category(&self) -> &str {
        "System"
    }

    fn handles_search(&self) -> bool {
        false
    }

    fn handles_command(&self, query: &str) -> bool {
        let q = query.trim_start();
        q == "audio" || q.starts_with("audio ") || q == "sink" || q.starts_with("sink ")
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![LauncherItem {
            id: "audio:palette".into(),
            name: "audio".into(),
            description: "Switch default audio sink / source (PipeWire)".into(),
            icon: "audio-volume-high-symbolic".into(),
            icon_is_path: false,
            score: 0.0,
            provider_name: "Audio".into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let q = query.trim_start();
        if !(q == "audio" || q.starts_with("audio ") || q == "sink" || q.starts_with("sink ")) {
            return Vec::new();
        }
        let filter = q
            .trim_start_matches("audio")
            .trim_start_matches("sink")
            .trim()
            .to_ascii_lowercase();

        let mut devices = snapshot();
        if !filter.is_empty() {
            devices.retain(|d| d.name.to_ascii_lowercase().contains(&filter));
        }

        // Sinks first (most-common ask), then sources.
        devices.sort_by(|a, b| a.kind.cmp(b.kind).then(a.id.cmp(&b.id)));

        devices
            .into_iter()
            .enumerate()
            .map(|(idx, dev)| {
                let id = dev.id;
                let name = dev.name.clone();
                let kind_label = if dev.kind == "sink" {
                    "Output"
                } else {
                    "Input"
                };
                let prefix = if dev.is_default { "★ " } else { "" };
                let id_str = id.to_string();
                let toast_name = name.clone();
                let icon = match (dev.kind, dev.is_default) {
                    ("sink", true) => "audio-volume-high-symbolic",
                    ("sink", false) => "audio-speakers-symbolic",
                    ("source", true) => "audio-input-microphone-symbolic",
                    _ => "audio-card-symbolic",
                };
                LauncherItem {
                    id: format!("audio:{}:{id}", dev.kind),
                    name: format!("{prefix}{name}"),
                    description: format!(
                        "{kind_label} · id {id}{}",
                        if dev.is_default { " · default" } else { "" }
                    ),
                    icon: icon.into(),
                    icon_is_path: false,
                    score: if dev.is_default {
                        200.0 - idx as f64
                    } else {
                        180.0 - idx as f64
                    },
                    provider_name: "Audio".into(),
                    usage_key: Some(format!("audio:{}:{}", dev.kind, dev.id)),
                    on_activate: Rc::new(move || {
                        if let Err(err) = Command::new("wpctl")
                            .args(["set-default", &id_str])
                            .spawn()
                        {
                            tracing::warn!(?err, id = id_str, "wpctl set-default failed");
                        } else {
                            toast("Audio default", toast_name.clone());
                        }
                    }),
                }
            })
            .collect()
    }

    /// System tab — show every sink + source without the `audio`
    /// prefix; `filter` narrows by device name substring.
    fn browse(&self, filter: &str) -> Vec<LauncherItem> {
        if filter.is_empty() {
            self.search("audio")
        } else {
            self.search(&format!("audio {filter}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn does_not_handle_regular_search() {
        let p = WireplumberProvider::new();
        assert!(p.search("firefox").is_empty());
    }

    #[test]
    fn parse_handles_basic_status_tree() {
        let sample = r#"PipeWire 'pipewire-0' [1.4.10, kenan@hay, cookie:42]
 └─ Clients:
        32. WirePlumber                         [1.0.0]

Audio
 ├─ Devices:
 ├─ Sinks:
 │      *  62. Built-in Audio Analog Stereo     [vol: 0.40]
 │         88. HDMI Audio                       [vol: 0.50]
 ├─ Sources:
 │      *  61. Built-in Audio Analog Stereo Mic [vol: 0.45]
 ├─ Filters:
"#;
        let devs = parse_wpctl_status(sample);
        let sinks: Vec<_> = devs.iter().filter(|d| d.kind == "sink").collect();
        let sources: Vec<_> = devs.iter().filter(|d| d.kind == "source").collect();
        assert_eq!(sinks.len(), 2);
        assert_eq!(sources.len(), 1);
        assert_eq!(sinks[0].id, 62);
        assert!(sinks[0].is_default);
        assert!(!sinks[1].is_default);
        assert_eq!(sources[0].id, 61);
        assert!(sources[0].is_default);
    }
}
