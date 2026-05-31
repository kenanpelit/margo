use mshell_common::{watch, watch_cancellable};
use mshell_services::audio_service;
use relm4::{Component, ComponentSender};
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

/// Themed icon name for an application stream. Prefers the stream's
/// `application.icon_name` PulseAudio property; falls back to a generic
/// app glyph (recording rows pass their own mic fallback).
pub fn stream_icon_name(stream: &AudioStream, fallback: &'static str) -> String {
    stream
        .properties
        .get()
        .get("application.icon_name")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| fallback.to_string())
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
