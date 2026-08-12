//! Output grouping: play through several outputs at once via PipeWire's
//! pulse-compat `module-combine-sink`.
//!
//! Groups are not tracked in our own state at all — they live entirely in
//! the running audio server as `module-combine-sink` instances whose
//! `sink_name` we prefix with `margo_group_`. [`list_groups`] rediscovers
//! them by asking `pactl list modules short` each time the Outputs panel
//! wants them, the same approach the audio-switcher reference plugin
//! uses. A group therefore survives an mshell restart (nothing but the
//! shell restarted) but not a PipeWire restart (there is nothing left to
//! rediscover) — expected, not a bug.

use crate::audio_service;
use std::time::Duration;
use tokio::process::Command;

/// `sink_name` prefix for every combine-sink margo creates — lets us tell
/// a group apart from a real hardware output at a glance (`is_group`).
const GROUP_PREFIX: &str = "margo_group_";

/// How long to wait for the new combine-sink's PipeWire node to appear
/// after `load-module` returns.
const NODE_WAIT: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioGroup {
    /// The pulse module index — what `pactl unload-module` takes.
    pub module_index: u32,
    /// The combine-sink's own device name (`margo_group_N`).
    pub sink_name: String,
    /// Device names of the outputs it combines.
    pub member_names: Vec<String>,
}

/// True if `device_name` is a margo-created group sink rather than real
/// hardware — the check the Outputs panel uses to render a group row
/// (Disband) instead of a normal device row (rename/hide).
pub fn is_group(device_name: &str) -> bool {
    device_name.starts_with(GROUP_PREFIX)
}

/// Discover currently-loaded margo groups straight from the audio server.
pub async fn list_groups() -> Vec<AudioGroup> {
    let text = stdout_of("pactl", &["list", "modules", "short"]).await;
    text.lines().filter_map(parse_short_line).collect()
}

fn parse_short_line(line: &str) -> Option<AudioGroup> {
    let mut cols = line.splitn(3, '\t');
    let module_index: u32 = cols.next()?.trim().parse().ok()?;
    if cols.next()? != "module-combine-sink" {
        return None;
    }
    let argument = cols.next().unwrap_or("");
    let sink_name = arg_value(argument, "sink_name")?;
    if !is_group(&sink_name) {
        return None;
    }
    let member_names = arg_value(argument, "slaves")
        .map(|s| s.split(',').map(str::to_string).collect())
        .unwrap_or_default();
    Some(AudioGroup {
        module_index,
        sink_name,
        member_names,
    })
}

/// Pull `key=value` out of a `pactl load-module`-style argument string
/// (space-separated `key=value` tokens).
fn arg_value(argument: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    argument
        .split_whitespace()
        .find_map(|tok| tok.strip_prefix(&prefix))
        .map(|v| v.trim_matches('"').to_string())
}

/// Create a combine-sink over `member_names` (≥2 real output device
/// names), make it the default output, and move current playback streams
/// onto it. Returns the new group's sink name on success.
pub async fn create_group(member_names: &[String]) -> Option<String> {
    if member_names.len() < 2 {
        return None;
    }
    let taken: Vec<String> = list_groups()
        .await
        .into_iter()
        .map(|g| g.sink_name)
        .collect();
    let sink_name = (1..)
        .map(|n| format!("{GROUP_PREFIX}{n}"))
        .find(|name| !taken.contains(name))?;

    let ok = run(
        "pactl",
        &[
            "load-module",
            "module-combine-sink",
            &format!("sink_name={sink_name}"),
            &format!("slaves={}", member_names.join(",")),
        ],
    )
    .await;
    if !ok {
        return None;
    }

    adopt_as_default(&sink_name, NODE_WAIT).await;
    Some(sink_name)
}

/// Disband a group: if it's currently the default output, hand off to
/// the first still-present member before removing it, so audio doesn't
/// just stop.
pub async fn disband_group(group: &AudioGroup) {
    let is_default = audio_service()
        .default_output
        .get()
        .map(|d| d.name.get() == group.sink_name)
        .unwrap_or(false);

    if is_default {
        let outputs = audio_service().output_devices.get();
        if let Some(fallback) = group
            .member_names
            .iter()
            .find_map(|name| outputs.iter().find(|d| &d.name.get() == name))
            && fallback.set_as_default().await.is_ok()
        {
            for stream in audio_service().playback_streams.get() {
                let _ = stream.move_to_device(fallback.key).await;
            }
        }
    }

    let _ = run("pactl", &["unload-module", &group.module_index.to_string()]).await;
}

/// Wait (bounded) for `sink_name`'s device to appear, then make it the
/// default output and migrate current playback streams onto it.
async fn adopt_as_default(sink_name: &str, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(dev) = audio_service()
            .output_devices
            .get()
            .into_iter()
            .find(|d| d.name.get() == sink_name)
        {
            if dev.set_as_default().await.is_ok() {
                for stream in audio_service().playback_streams.get() {
                    let _ = stream.move_to_device(dev.key).await;
                }
            }
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

async fn run(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd)
        .args(args)
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn stdout_of(cmd: &str, args: &[&str]) -> String {
    Command::new(cmd)
        .args(args)
        .output()
        .await
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_margo_group_line() {
        let line =
            "536870916\tmodule-combine-sink\tsink_name=margo_group_1 slaves=alsa_out,bluez_out\t";
        let group = parse_short_line(line).expect("should parse");
        assert_eq!(group.module_index, 536870916);
        assert_eq!(group.sink_name, "margo_group_1");
        assert_eq!(group.member_names, vec!["alsa_out", "bluez_out"]);
    }

    #[test]
    fn ignores_non_margo_combine_sinks() {
        let line = "5\tmodule-combine-sink\tsink_name=someone_elses_combo slaves=a,b\t";
        assert!(parse_short_line(line).is_none());
    }

    #[test]
    fn ignores_unrelated_modules() {
        let line = "3\tmodule-always-sink\tsink_name=dummy\t";
        assert!(parse_short_line(line).is_none());
    }

    #[test]
    fn is_group_checks_prefix() {
        assert!(is_group("margo_group_3"));
        assert!(!is_group("alsa_output.pci-0000_00_1f.3"));
    }
}
