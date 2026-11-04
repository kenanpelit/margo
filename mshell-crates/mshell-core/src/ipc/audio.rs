//! Audio device selection and the `audio list/status` snapshot renderers.

use super::*;

/// Real, switchable output sinks (skips unplugged HDMI/DP ports). Sorted by
/// the stable PipeWire device index so `list`, `status` and `next`/`prev` all
/// see the SAME order — `input_devices.get()` / `output_devices.get()` don't
/// guarantee a stable order between calls, which made cycling skip a device
/// and the switch notification name the wrong one.
///
/// When the `audio.hide_hdmi_outputs` config toggle is on, sinks whose node
/// name or description matches "hdmi"/"displayport" are also excluded.
pub(super) fn usable_outputs() -> Vec<Arc<OutputDevice>> {
    let hide_hdmi = config_manager()
        .config()
        .audio()
        .hide_hdmi_outputs()
        .get_untracked();
    let mut v: Vec<_> = audio_service()
        .output_devices
        .get()
        .into_iter()
        .filter(|d| mshell_utils::audio::output_connected(d))
        .filter(|d| !(hide_hdmi && mshell_utils::audio::is_hdmi_output(d)))
        .filter(|d| !mshell_utils::audio_prefs::is_hidden(&d.name.get()))
        .collect();
    v.sort_by_key(|d| d.key.index);
    v
}

/// Real capture sources — drops PulseAudio monitor sources (the loopback
/// "Monitor of <sink>" entries), which aren't microphones. Same stable sort.
pub(super) fn usable_inputs() -> Vec<Arc<InputDevice>> {
    let mut v: Vec<_> = audio_service()
        .input_devices
        .get()
        .into_iter()
        .filter(|d| !d.is_monitor.get())
        .filter(|d| !mshell_utils::audio_prefs::is_hidden(&d.name.get()))
        .collect();
    v.sort_by_key(|d| d.key.index);
    v
}

/// Resolve a switch target against a device list. `names` is `(node_name,
/// description)` per device, `current` the default's node name. Accepts
/// `next` / `prev` / `switch`, a numeric index, or a case-insensitive
/// fragment matched against the description first then the node name.
/// A parsed `mshellctl clipboard <verb> [id]` action.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ClipboardAction {
    Copy(u64),
    TogglePin(u64),
    Delete(u64),
    Clear,
    Wipe,
}

/// Resolve a mute wire-command into the target mute state. The wire encoding is
/// shared by output and mic mute: `0` = unmute, `1` = mute, anything else =
/// toggle the current state. One source for both `MuteSet` / `MicMuteSet` arms.
pub(super) fn mute_target(mode: i32, currently_muted: bool) -> bool {
    match mode {
        0 => false,
        1 => true,
        _ => !currently_muted,
    }
}

/// Parse a `clipboard` IPC spec (`"<verb> [id]"`) into an action, or `None`
/// for an unknown verb / missing-or-malformed id where one is required.
pub(super) fn parse_clipboard_action(spec: &str) -> Option<ClipboardAction> {
    let mut it = spec.split_whitespace();
    let verb = it.next().unwrap_or("");
    let id = it.next().and_then(|s| s.parse::<u64>().ok());
    match (verb, id) {
        ("copy", Some(id)) => Some(ClipboardAction::Copy(id)),
        ("pin", Some(id)) | ("unpin", Some(id)) => Some(ClipboardAction::TogglePin(id)),
        ("delete", Some(id)) => Some(ClipboardAction::Delete(id)),
        ("clear", _) => Some(ClipboardAction::Clear),
        ("wipe", _) => Some(ClipboardAction::Wipe),
        _ => None,
    }
}

pub(super) fn pick_device(
    names: &[(String, String)],
    current: Option<&str>,
    target: &str,
) -> Option<usize> {
    if names.is_empty() {
        return None;
    }
    let cur = current.and_then(|c| names.iter().position(|(n, _)| n == c));
    let t = target.trim();
    match t.to_ascii_lowercase().as_str() {
        "next" | "switch" => return Some(cur.map(|c| (c + 1) % names.len()).unwrap_or(0)),
        "prev" | "previous" => {
            return Some(
                cur.map(|c| (c + names.len() - 1) % names.len())
                    .unwrap_or(0),
            );
        }
        _ => {}
    }
    if let Ok(i) = t.parse::<usize>()
        && i < names.len()
    {
        return Some(i);
    }
    let tl = t.to_lowercase();
    names
        .iter()
        .position(|(n, d)| d.to_lowercase().contains(&tl) || n.to_lowercase().contains(&tl))
}

#[derive(serde::Serialize)]
pub(super) struct DeviceSnapshot {
    pub(super) index: usize,
    /// Technical PipeWire node name (`alsa_output.pci-…`).
    pub(super) name: String,
    /// Friendly label ("Logitech Z205 Analog Stereo").
    pub(super) description: String,
    pub(super) volume_percent: u32,
    pub(super) muted: bool,
    /// Whether this is the current default device.
    pub(super) active: bool,
}

#[derive(serde::Serialize)]
pub(super) struct AudioSnapshot {
    pub(super) outputs: Vec<DeviceSnapshot>,
    pub(super) inputs: Vec<DeviceSnapshot>,
}

/// Read the live audio service into a serialisable snapshot (sync `.get()`s,
/// same direct-global pattern as `notification_count`).
pub(super) fn audio_snapshot() -> AudioSnapshot {
    let svc = audio_service();
    let def_out = svc.default_output.get().map(|d| d.name.get());
    let def_in = svc.default_input.get().map(|d| d.name.get());
    let snap = |i: usize,
                name: String,
                description: String,
                vol: f64,
                muted: bool,
                def: &Option<String>| {
        DeviceSnapshot {
            index: i,
            active: def.as_deref() == Some(name.as_str()),
            volume_percent: (vol * 100.0).round() as u32,
            muted,
            description,
            name,
        }
    };
    let outputs = usable_outputs()
        .iter()
        .enumerate()
        .map(|(i, d)| {
            snap(
                i,
                d.name.get(),
                d.description.get(),
                d.volume.get().average(),
                d.muted.get(),
                &def_out,
            )
        })
        .collect();
    let inputs = usable_inputs()
        .iter()
        .enumerate()
        .map(|(i, d)| {
            snap(
                i,
                d.name.get(),
                d.description.get(),
                d.volume.get().average(),
                d.muted.get(),
                &def_in,
            )
        })
        .collect();
    AudioSnapshot { outputs, inputs }
}

pub(super) fn fmt_device_line(d: &DeviceSnapshot, icon: &str) -> String {
    let mut tags = String::new();
    if d.active {
        tags.push_str("  [active]");
    }
    if d.muted {
        tags.push_str("  [muted]");
    }
    format!(
        "  {}: {} {:<42} {:>3}%{}",
        d.index, icon, d.description, d.volume_percent, tags
    )
}

/// `mshellctl audio list` body — human table or `--json`.
pub(super) fn render_audio_list(as_json: bool) -> String {
    let snap = audio_snapshot();
    if as_json {
        return serde_json::to_string_pretty(&snap).unwrap_or_else(|_| "{}".into());
    }
    let mut out = String::from("Outputs:\n");
    if snap.outputs.is_empty() {
        out.push_str("  (none)\n");
    }
    for d in &snap.outputs {
        out.push_str(&fmt_device_line(d, "🔊"));
        out.push('\n');
    }
    out.push_str("\nInputs:\n");
    if snap.inputs.is_empty() {
        out.push_str("  (none)\n");
    }
    for d in &snap.inputs {
        out.push_str(&fmt_device_line(d, "🎤"));
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// `mshellctl audio status` body — the current default out/in.
pub(super) fn render_audio_status(as_json: bool) -> String {
    let snap = audio_snapshot();
    let out = snap.outputs.into_iter().find(|d| d.active);
    let inp = snap.inputs.into_iter().find(|d| d.active);
    if as_json {
        let v = serde_json::json!({ "output": out, "input": inp });
        return serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".into());
    }
    let line = |d: Option<&DeviceSnapshot>, label: &str| match d {
        Some(d) => format!(
            "{label}: {} — {}%{}",
            d.description,
            d.volume_percent,
            if d.muted { " (muted)" } else { "" }
        ),
        None => format!("{label}: (none)"),
    };
    format!(
        "{}\n{}",
        line(out.as_ref(), "Output"),
        line(inp.as_ref(), "Input")
    )
}
