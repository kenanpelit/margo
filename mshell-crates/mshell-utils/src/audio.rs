use mshell_common::{watch, watch_cancellable};
use mshell_services::audio_service;
use relm4::{Component, ComponentSender};
use std::collections::HashMap;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use wayle_audio::core::device::input::InputDevice;
use wayle_audio::core::device::output::OutputDevice;
use wayle_audio::core::stream::AudioStream;

/// Returns `true` if this output sink looks like an HDMI or DisplayPort audio
/// device. Detection is a case-insensitive substring match on the PipeWire
/// node name and human-readable description. Used by the optional
/// `audio.hide_hdmi_outputs` config toggle — must never be called unless the
/// toggle is on, so a slightly broad match (any sink whose name/desc contains
/// "hdmi", "displayport", or "display port") is acceptable.
pub fn is_hdmi_output(d: &OutputDevice) -> bool {
    let name = d.name.get().to_lowercase();
    let desc = d.description.get().to_lowercase();
    let hit =
        |s: &str| s.contains("hdmi") || s.contains("displayport") || s.contains("display port");
    hit(&name) || hit(&desc)
}

pub fn get_audio_out_icon(device: &Arc<OutputDevice>) -> &'static str {
    if device.muted.get() {
        return "audio-volume-muted-symbolic";
    }
    let percentage = device.volume.get().average_percentage().round() as u16;
    if percentage > 66 {
        "audio-volume-high-symbolic"
    } else if percentage > 33 {
        "audio-volume-medium-symbolic"
    } else if percentage > 0 {
        "audio-volume-low-symbolic"
    } else {
        "audio-volume-muted-symbolic"
    }
}

/// Output icon that reflects the *device type* on top of the volume level,
/// used by the Audio Dashboard bar pill so its glyph doubles as a route
/// indicator (the standalone Audio Route pill is gone). Mute still wins so a
/// muted output stays visibly muted; otherwise a headset shows the headset
/// glyph, an HDMI/DisplayPort sink the display glyph, and everything else
/// falls back to the volume-level speaker icon from [`get_audio_out_icon`].
pub fn get_audio_out_icon_device_aware(device: &Arc<OutputDevice>) -> &'static str {
    if device.muted.get() {
        return "audio-volume-muted-symbolic";
    }
    if out_is_headset(device) {
        return "audio-headset-symbolic";
    }
    if is_hdmi_output(device) {
        return "video-display-symbolic";
    }
    get_audio_out_icon(device)
}

pub fn get_audio_in_icon(device: &Arc<InputDevice>) -> &'static str {
    if device.muted.get() {
        return "microphone-sensitivity-muted-symbolic";
    }
    let percentage = device.volume.get().average_percentage().round() as u16;
    if percentage > 66 {
        "microphone-sensitivity-high-symbolic"
    } else if percentage > 33 {
        "microphone-sensitivity-medium-symbolic"
    } else if percentage > 0 {
        "microphone-sensitivity-low-symbolic"
    } else {
        "microphone-sensitivity-muted-symbolic"
    }
}

pub fn spawn_default_output_watcher<C>(
    sender: &ComponentSender<C>,
    cancellation_token: Option<CancellationToken>,
    map_state: impl Fn() -> C::CommandOutput + Send + Sync + 'static,
) where
    C: Component,
    C::CommandOutput: Send + 'static,
{
    let default_output = audio_service().default_output.clone();

    if let Some(cancellation_token) = cancellation_token {
        watch_cancellable!(
            sender,
            cancellation_token,
            [default_output.watch()],
            |out| {
                let _ = out.send(map_state());
            }
        );
    } else {
        watch!(sender, [default_output.watch()], |out| {
            let _ = out.send(map_state());
        });
    }
}

pub fn spawn_output_devices_watcher<C>(
    sender: &ComponentSender<C>,
    cancellation_token: CancellationToken,
    map_state: impl Fn() -> C::CommandOutput + Send + Sync + 'static,
) where
    C: Component,
    C::CommandOutput: Send + 'static,
{
    let out_devices = audio_service().output_devices.clone();

    watch_cancellable!(sender, cancellation_token, [out_devices.watch()], |out| {
        let _ = out.send(map_state());
    });
}

pub fn spawn_output_device_volume_mute_watcher<C>(
    output_device: &Arc<OutputDevice>,
    cancellation_token: CancellationToken,
    sender: &ComponentSender<C>,
    map_state: impl Fn() -> C::CommandOutput + Send + Sync + 'static,
) where
    C: Component,
    C::CommandOutput: Send + 'static,
{
    let volume = output_device.volume.clone();
    let muted = output_device.muted.clone();
    watch_cancellable!(
        sender,
        cancellation_token,
        [volume.watch(), muted.watch()],
        |out| {
            let _ = out.send(map_state());
        }
    );
}

pub fn spawn_default_input_watcher<C>(
    sender: &ComponentSender<C>,
    cancellation_token: Option<CancellationToken>,
    map_state: impl Fn() -> C::CommandOutput + Send + Sync + 'static,
) where
    C: Component,
    C::CommandOutput: Send + 'static,
{
    let default = audio_service().default_input.clone();

    if let Some(cancellation_token) = cancellation_token {
        watch_cancellable!(sender, cancellation_token, [default.watch()], |out| {
            let _ = out.send(map_state());
        });
    } else {
        watch!(sender, [default.watch()], |out| {
            let _ = out.send(map_state());
        });
    }
}

pub fn spawn_input_devices_watcher<C>(
    sender: &ComponentSender<C>,
    cancellation_token: CancellationToken,
    map_state: impl Fn() -> C::CommandOutput + Send + Sync + 'static,
) where
    C: Component,
    C::CommandOutput: Send + 'static,
{
    let devices = audio_service().input_devices.clone();

    watch_cancellable!(sender, cancellation_token, [devices.watch()], |out| {
        let _ = out.send(map_state());
    });
}

pub fn spawn_input_device_volume_mute_watcher<C>(
    input_device: &Arc<InputDevice>,
    cancellation_token: CancellationToken,
    sender: &ComponentSender<C>,
    map_state: impl Fn() -> C::CommandOutput + Send + Sync + 'static,
) where
    C: Component,
    C::CommandOutput: Send + 'static,
{
    let volume = input_device.volume.clone();
    let muted = input_device.muted.clone();
    watch_cancellable!(
        sender,
        cancellation_token,
        [volume.watch(), muted.watch()],
        |out| {
            let _ = out.send(map_state());
        }
    );
}

/// Watch an output device's port set + active port — the route
/// switcher's analogue of the volume/mute watcher.
pub fn spawn_output_device_ports_watcher<C>(
    output_device: &Arc<OutputDevice>,
    cancellation_token: CancellationToken,
    sender: &ComponentSender<C>,
    map_state: impl Fn() -> C::CommandOutput + Send + Sync + 'static,
) where
    C: Component,
    C::CommandOutput: Send + 'static,
{
    let ports = output_device.ports.clone();
    let active = output_device.active_port.clone();
    watch_cancellable!(
        sender,
        cancellation_token,
        [ports.watch(), active.watch()],
        |out| {
            let _ = out.send(map_state());
        }
    );
}

/// Watch an input device's port set + active port.
pub fn spawn_input_device_ports_watcher<C>(
    input_device: &Arc<InputDevice>,
    cancellation_token: CancellationToken,
    sender: &ComponentSender<C>,
    map_state: impl Fn() -> C::CommandOutput + Send + Sync + 'static,
) where
    C: Component,
    C::CommandOutput: Send + 'static,
{
    let ports = input_device.ports.clone();
    let active = input_device.active_port.clone();
    watch_cancellable!(
        sender,
        cancellation_token,
        [ports.watch(), active.watch()],
        |out| {
            let _ = out.send(map_state());
        }
    );
}

// ── Per-application streams (QSAP-style app mixer) ───────────────────────────

/// Watch the set of playback streams (one per app currently producing
/// sound). Fires whenever an app starts/stops playing so the app-mixer
/// section can rebuild its rows.
pub fn spawn_playback_streams_watcher<C>(
    sender: &ComponentSender<C>,
    map_state: impl Fn() -> C::CommandOutput + Send + Sync + 'static,
) where
    C: Component,
    C::CommandOutput: Send + 'static,
{
    let streams = audio_service().playback_streams.clone();
    watch!(sender, [streams.watch()], |out| {
        let _ = out.send(map_state());
    });
}

/// Watch the set of recording streams (apps capturing the microphone).
pub fn spawn_recording_streams_watcher<C>(
    sender: &ComponentSender<C>,
    map_state: impl Fn() -> C::CommandOutput + Send + Sync + 'static,
) where
    C: Component,
    C::CommandOutput: Send + 'static,
{
    let streams = audio_service().recording_streams.clone();
    watch!(sender, [streams.watch()], |out| {
        let _ = out.send(map_state());
    });
}

/// Per-stream volume + mute watcher — the stream analogue of
/// [`spawn_output_device_volume_mute_watcher`]. Reset the token when
/// the row is rebuilt for a different stream.
pub fn spawn_stream_volume_mute_watcher<C>(
    stream: &Arc<AudioStream>,
    cancellation_token: CancellationToken,
    sender: &ComponentSender<C>,
    map_state: impl Fn() -> C::CommandOutput + Send + Sync + 'static,
) where
    C: Component,
    C::CommandOutput: Send + 'static,
{
    let volume = stream.volume.clone();
    let muted = stream.muted.clone();
    watch_cancellable!(
        sender,
        cancellation_token,
        [volume.watch(), muted.watch()],
        |out| {
            let _ = out.send(map_state());
        }
    );
}

/// Best-effort display name for an application stream: the PulseAudio
/// `application_name` (e.g. "Spotify", "Firefox"), falling back to the
/// raw stream name, then "Application".
pub fn stream_display_name(stream: &AudioStream) -> String {
    stream
        .application_name
        .get()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            let n = stream.name.get();
            (!n.is_empty()).then_some(n)
        })
        .unwrap_or_else(|| "Application".to_string())
}

/// Volume icon tier for a stream (mirrors the device helpers).
pub fn get_stream_volume_icon(stream: &AudioStream) -> &'static str {
    if stream.muted.get() {
        return "audio-volume-muted-symbolic";
    }
    let percentage = stream.volume.get().average_percentage().round() as u16;
    if percentage > 66 {
        "audio-volume-high-symbolic"
    } else if percentage > 33 {
        "audio-volume-medium-symbolic"
    } else if percentage > 0 {
        "audio-volume-low-symbolic"
    } else {
        "audio-volume-muted-symbolic"
    }
}

// ── Audio Route (output cycling + headset classification) ──────────────────
//
// Shared by the `mshellctl audio route-next` IPC action (`mshell-core`), the
// Settings → Sound device switcher, and the Audio Dashboard pill's device-aware
// glyph ([`get_audio_out_icon_device_aware`]), so they all classify + route
// audio identically. Detection prefers PipeWire's structured, machine-portable
// metadata over any name guessing.

/// Name-only headset keywords — the LAST-resort fallback in [`is_headset`],
/// used only when a device advertises no structured PipeWire metadata.
/// Deliberately excludes "hdmi" / "line-out": those are their own devices but
/// are NOT wearable headsets, so they must never read as one.
const DEVICE_HEADSET_KEYS: &[&str] = &["headset", "headphone", "earphone", "earbud", "airpod"];

/// Classify a device as a headset, preferring PipeWire's **structured,
/// machine-portable** metadata over any name guessing (name-only matching was
/// fragile across machines). Signals, most authoritative first:
///
///   1. `device.form_factor` — the PulseAudio/PipeWire standard. BlueZ sets
///      `headset`/`headphone`; ALSA UCM sets `speaker`/`internal`. Decides when
///      present, no guessing.
///   2. `device.icon_name` — `audio-headset*` / `audio-headphones` ⇒ headset;
///      `audio-speakers` / `video-display` / `audio-card-analog` ⇒ explicitly
///      NOT (the negative match stops speakers/HDMI reading as a headset).
///   3. `device.bus == "bluetooth"` ⇒ a Bluetooth audio output (a rare BT
///      *speaker* still carries `device.form_factor = "speaker"`, caught above).
///   4. Last resort — substring match on the node name + description.
///
/// Only used to pick the mic's side + the pill's glyph; it never gates which
/// outputs you can cycle to, so a misclassification can't hide a device.
pub fn is_headset(props: &HashMap<String, String>, name: &str, desc: &str) -> bool {
    // 1. form_factor — authoritative when present.
    if let Some(ff) = props.get("device.form_factor") {
        return matches!(
            ff.to_ascii_lowercase().as_str(),
            "headset" | "headphone" | "hands-free" | "handset" | "earpiece"
        );
    }
    // 2. icon_name — positive then negative.
    if let Some(icon) = props
        .get("device.icon_name")
        .map(|s| s.to_ascii_lowercase())
    {
        if icon.contains("headset") || icon.contains("headphone") || icon.contains("earbud") {
            return true;
        }
        if icon.contains("speaker")
            || icon.contains("video-display")
            || icon.contains("card-analog")
            || icon.contains("hdmi")
        {
            return false;
        }
    }
    // 3. Bluetooth bus.
    if props
        .get("device.bus")
        .is_some_and(|b| b.eq_ignore_ascii_case("bluetooth"))
    {
        return true;
    }
    // 4. Name/description keyword heuristic (last resort).
    is_headset_name(name, desc)
}

/// Name-only headset heuristic — the final fallback in [`is_headset`]. Pure +
/// unit-testable.
pub fn is_headset_name(name: &str, desc: &str) -> bool {
    let hay = format!(
        "{} {}",
        name.to_ascii_lowercase(),
        desc.to_ascii_lowercase()
    );
    DEVICE_HEADSET_KEYS.iter().any(|k| hay.contains(k))
}

/// Device-level headset test for an output device (structured props first).
pub fn out_is_headset(d: &OutputDevice) -> bool {
    is_headset(&d.properties.get(), &d.name.get(), &d.description.get())
}

/// Device-level headset test for an input device (structured props first).
pub fn in_is_headset(d: &InputDevice) -> bool {
    is_headset(&d.properties.get(), &d.name.get(), &d.description.get())
}

/// The routable output devices — every sink except HDMI/DisplayPort — in a
/// deterministic (name-sorted) order so the cycle is stable across reloads and
/// identical between the bar pill and the CLI.
pub fn routable_outputs() -> Vec<Arc<OutputDevice>> {
    let mut outs: Vec<_> = audio_service()
        .output_devices
        .get()
        .into_iter()
        .filter(|d| !is_hdmi_output(d))
        .collect();
    outs.sort_by_key(|d| d.name.get());
    outs
}

/// Index of the next device to cycle to: one past `current` (wrapping), or 0
/// when the current default isn't in the list (e.g. it's an HDMI sink). Pure +
/// unit-testable.
pub fn next_index(names: &[String], current: Option<&str>) -> usize {
    if names.is_empty() {
        return 0;
    }
    match current.and_then(|c| names.iter().position(|n| n == c)) {
        Some(i) => (i + 1) % names.len(),
        None => 0,
    }
}

/// The input device the microphone should hop to so it matches the output's
/// headset side, or `None` when the mic is already on the right side or no
/// suitable input exists. Callers await/spawn `set_as_default()` on the result
/// in their own context (GTK thread spawns, the IPC command loop awaits).
pub fn mic_follow_target(to_headset: bool) -> Option<Arc<InputDevice>> {
    let audio = audio_service();
    let already = audio
        .default_input
        .get()
        .as_ref()
        .is_some_and(|d| in_is_headset(d));
    if already == to_headset {
        return None;
    }
    let inputs = audio.input_devices.get();
    if to_headset {
        inputs.iter().find(|d| in_is_headset(d)).cloned()
    } else {
        inputs.iter().find(|d| !in_is_headset(d)).cloned()
    }
}

#[cfg(test)]
mod route_tests {
    use super::*;

    fn props(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn headset_form_factor_is_authoritative() {
        // A BlueZ headset advertises device.form_factor=headset — decisive.
        let bt = props(&[
            ("device.form_factor", "headset"),
            ("device.bus", "bluetooth"),
        ]);
        assert!(is_headset(&bt, "bluez_output.F4_9D", "SLP4"));

        // A Bluetooth *speaker* carries form_factor=speaker → NOT a headset.
        let bt_speaker = props(&[
            ("device.form_factor", "speaker"),
            ("device.bus", "bluetooth"),
        ]);
        assert!(!is_headset(
            &bt_speaker,
            "bluez_output.SOUNDCORE",
            "Speaker"
        ));
    }

    #[test]
    fn headset_icon_name_positive_and_negative() {
        assert!(is_headset(
            &props(&[("device.icon_name", "audio-headset")]),
            "alsa_output.usb-Headset",
            "USB Headset"
        ));
        // HDMI / speakers / analog card must NOT read as headsets.
        assert!(!is_headset(
            &props(&[("device.icon_name", "video-display")]),
            "alsa_output.pci.HiFi__HDMI1__sink",
            "HDMI 1"
        ));
        assert!(!is_headset(
            &props(&[("device.icon_name", "audio-speakers")]),
            "alsa_output.pci.HiFi__Speaker__sink",
            "Speaker"
        ));
        assert!(!is_headset(
            &props(&[
                ("device.icon_name", "audio-card-analog"),
                ("device.bus", "usb")
            ]),
            "alsa_output.usb-Logitech_Z205",
            "Logitech Z205"
        ));
    }

    #[test]
    fn headset_bluetooth_bus_without_form_factor() {
        assert!(is_headset(
            &props(&[("device.bus", "bluetooth")]),
            "bluez_output.xx",
            ""
        ));
    }

    #[test]
    fn headset_name_fallback_when_no_metadata() {
        assert!(is_headset(
            &props(&[]),
            "alsa_output.usb-X",
            "Gaming Headset"
        ));
        assert!(!is_headset(
            &props(&[]),
            "alsa_output.usb-Z205",
            "Logitech Z205"
        ));
        assert!(is_headset_name("x", "My Headphones"));
        assert!(!is_headset_name("alsa_output.hdmi", "HDMI Audio"));
    }

    #[test]
    fn cycle_next_index_wraps() {
        let names = vec![
            "alsa_output.pci.Speaker".to_string(),
            "alsa_output.usb-Logitech".to_string(),
            "bluez_output.SLP4".to_string(),
        ];
        // Middle → next.
        assert_eq!(next_index(&names, Some("alsa_output.usb-Logitech")), 2);
        // Last → wraps to first.
        assert_eq!(next_index(&names, Some("bluez_output.SLP4")), 0);
        // Current not in list (e.g. on HDMI) → starts at first.
        assert_eq!(next_index(&names, Some("alsa_output.pci.HDMI1")), 0);
        // No current → first.
        assert_eq!(next_index(&names, None), 0);
        // Empty list → 0 (guarded, no modulo-by-zero).
        assert_eq!(next_index(&[], Some("x")), 0);
    }
}
