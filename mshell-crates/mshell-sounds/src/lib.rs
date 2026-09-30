use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const CAMERA_SHUTTER_SOUND: &[u8] = include_bytes!("../assets/camera-shutter.ogg");
const AUDIO_VOLUME_CHANGED_SOUND: &[u8] = include_bytes!("../assets/audio-volume-change.ogg");
const BATTERY_LOW_SOUND: &[u8] = include_bytes!("../assets/battery-low.ogg");
const POWER_PLUG_SOUND: &[u8] = include_bytes!("../assets/power-plug.ogg");
const POWER_UNPLUG_SOUND: &[u8] = include_bytes!("../assets/power-unplug.ogg");
/// Alarm tone (converted from the DMS alarmClock plugin's `alarm.wav` to ogg
/// so it decodes with rodio's vorbis feature).
const ALARM_SOUND: &[u8] = include_bytes!("../assets/alarm.ogg");
/// Default notification chime (gentle two-tone, synthesized in-tree).
/// OGG/Vorbis, not WAV: rodio is built with `default-features = false,
/// features = ["vorbis", "playback"]`, so it has **no WAV decoder** — a `.wav`
/// here never decodes (symphonia probes to EOF, logs `probe reach EOF`, and the
/// chime is silent). Every shipped sound is OGG for this reason (see the alarm
/// note above). Keep notification sounds OGG.
const NOTIFICATION_SOUND: &[u8] = include_bytes!("../assets/notification.ogg");
/// Critical-urgency notification tone (three rising tones, brighter). OGG, per
/// the note above — rodio decodes vorbis only.
const NOTIFICATION_CRITICAL_SOUND: &[u8] = include_bytes!("../assets/notification-critical.ogg");

/// Whether the looping alarm tone is currently ringing. Drives both the loop
/// thread and `alarm_is_ringing()`.
static ALARM_PLAYING: AtomicBool = AtomicBool::new(false);

/// Start the alarm tone, looping until [`stop_alarm`]. No-op if already ringing.
pub fn play_alarm_loop() {
    if ALARM_PLAYING.swap(true, Ordering::SeqCst) {
        return; // already ringing
    }
    std::thread::spawn(|| {
        let Ok(mut handle) = rodio::DeviceSinkBuilder::open_default_sink() else {
            ALARM_PLAYING.store(false, Ordering::SeqCst);
            return;
        };
        handle.log_on_drop(false);
        // Replay the clip until stopped; poll the flag so Stop is responsive
        // (≤120 ms) even mid-clip rather than waiting for the clip to end.
        while ALARM_PLAYING.load(Ordering::SeqCst) {
            let Ok(player) = rodio::play(handle.mixer(), Cursor::new(ALARM_SOUND)) else {
                break;
            };
            while ALARM_PLAYING.load(Ordering::SeqCst) && !player.empty() {
                std::thread::sleep(Duration::from_millis(120));
            }
            player.stop();
        }
        ALARM_PLAYING.store(false, Ordering::SeqCst);
    });
}

/// Stop the looping alarm tone.
pub fn stop_alarm() {
    ALARM_PLAYING.store(false, Ordering::SeqCst);
}

/// Whether the alarm tone is currently ringing.
pub fn alarm_is_ringing() -> bool {
    ALARM_PLAYING.load(Ordering::SeqCst)
}

/// Play the default notification chime (normal urgency).
pub fn play_notification() {
    play_embedded(NOTIFICATION_SOUND);
}

/// Play the critical-urgency notification tone.
pub fn play_notification_critical() {
    play_embedded(NOTIFICATION_CRITICAL_SOUND);
}

/// Upper bound on a client-supplied `sound-file`'s size. Any real
/// notification chime is well under 1 MiB; this is generous headroom
/// while still ruling out `fs::read` turning into an unbounded-memory
/// DoS on a multi-GB (or sparse/huge) file a hostile local app points at.
const MAX_NOTIFICATION_SOUND_BYTES: u64 = 20 * 1024 * 1024;

/// `true` when `path` is safe to hand to `fs::read` for sound playback: a
/// plain, reasonably-sized regular file. `path` is untrusted (the spec's
/// client-controlled `sound-file` hint) — `is_file()` alone rules out a
/// FIFO/char-device/socket that could block the caller forever on read;
/// the size check rules out a huge file turning into an unbounded
/// allocation. Split out from [`play_notification_file`] so it's testable
/// without a real audio device.
fn is_safe_sound_file(path: &str) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    meta.is_file() && meta.len() > 0 && meta.len() <= MAX_NOTIFICATION_SOUND_BYTES
}

/// Play a client-supplied sound file (the spec's `sound-file` hint).
/// Decode failures and missing files degrade silently — a bad hint must
/// never take the shell down or block the toast.
pub fn play_notification_file(path: &str) {
    let path = path.trim().to_string();
    if path.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        if !is_safe_sound_file(&path) {
            return;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        let Ok(mut handle) = rodio::DeviceSinkBuilder::open_default_sink() else {
            return;
        };
        handle.log_on_drop(false);
        if let Ok(player) = rodio::play(handle.mixer(), Cursor::new(bytes)) {
            player.sleep_until_end();
        }
    });
}

/// Fire-and-forget playback of an embedded clip on its own thread.
fn play_embedded(bytes: &'static [u8]) {
    std::thread::spawn(move || {
        let Ok(mut handle) = rodio::DeviceSinkBuilder::open_default_sink() else {
            return;
        };
        handle.log_on_drop(false);
        if let Ok(player) = rodio::play(handle.mixer(), Cursor::new(bytes)) {
            player.sleep_until_end();
        }
    });
}

pub fn play_shutter() {
    std::thread::spawn(|| {
        let mut handle =
            rodio::DeviceSinkBuilder::open_default_sink().expect("open default audio device");
        handle.log_on_drop(false);
        let cursor = Cursor::new(CAMERA_SHUTTER_SOUND);
        if let Ok(player) = rodio::play(handle.mixer(), cursor) {
            player.sleep_until_end();
        }
    });
}

pub fn play_audio_volume_change() {
    std::thread::spawn(|| {
        // give volume changes a moment to happen
        std::thread::sleep(Duration::from_millis(50));
        let mut handle =
            rodio::DeviceSinkBuilder::open_default_sink().expect("open default audio device");
        handle.log_on_drop(false);
        let cursor = Cursor::new(AUDIO_VOLUME_CHANGED_SOUND);
        if let Ok(player) = rodio::play(handle.mixer(), cursor) {
            player.sleep_until_end();
            // sleep to make sure the sounds plays.  It's very short and might not without the sleep.
            std::thread::sleep(Duration::from_millis(100));
        }
    });
}

pub fn play_battery_low() {
    std::thread::spawn(|| {
        let mut handle =
            rodio::DeviceSinkBuilder::open_default_sink().expect("open default audio device");
        handle.log_on_drop(false);
        let cursor = Cursor::new(BATTERY_LOW_SOUND);
        if let Ok(player) = rodio::play(handle.mixer(), cursor) {
            player.sleep_until_end();
        }
    });
}

pub fn play_power_plug_sound() {
    std::thread::spawn(|| {
        let mut handle =
            rodio::DeviceSinkBuilder::open_default_sink().expect("open default audio device");
        handle.log_on_drop(false);
        let cursor = Cursor::new(POWER_PLUG_SOUND);
        if let Ok(player) = rodio::play(handle.mixer(), cursor) {
            player.sleep_until_end();
        }
    });
}

pub fn play_power_unplug_sound() {
    std::thread::spawn(|| {
        let mut handle =
            rodio::DeviceSinkBuilder::open_default_sink().expect("open default audio device");
        handle.log_on_drop(false);
        let cursor = Cursor::new(POWER_UNPLUG_SOUND);
        if let Ok(player) = rodio::play(handle.mixer(), cursor) {
            player.sleep_until_end();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every embedded clip MUST decode with rodio's configured feature set
    /// (`vorbis` only — no WAV decoder). A `.wav` asset slips silently past a
    /// plain build but fails to decode at runtime (symphonia logs
    /// `probe reach EOF` and the sound never plays). Decoding here needs no
    /// audio device, so it guards the regression in CI: add a WAV and this
    /// fails instead of going quiet in production.
    #[test]
    fn every_embedded_clip_decodes() {
        let clips: &[(&str, &[u8])] = &[
            ("camera-shutter", CAMERA_SHUTTER_SOUND),
            ("audio-volume-change", AUDIO_VOLUME_CHANGED_SOUND),
            ("battery-low", BATTERY_LOW_SOUND),
            ("power-plug", POWER_PLUG_SOUND),
            ("power-unplug", POWER_UNPLUG_SOUND),
            ("alarm", ALARM_SOUND),
            ("notification", NOTIFICATION_SOUND),
            ("notification-critical", NOTIFICATION_CRITICAL_SOUND),
        ];
        for (name, bytes) in clips {
            rodio::Decoder::try_from(Cursor::new(*bytes))
                .unwrap_or_else(|e| panic!("embedded sound `{name}` failed to decode: {e}"));
        }
    }

    /// Regression: `play_notification_file` used to hand a client-supplied
    /// `sound-file` path straight to `fs::read` with no size or type check —
    /// an unbounded-memory-allocation DoS waiting to happen.
    #[test]
    fn safe_sound_file_rejects_missing_empty_oversized_and_dirs() {
        let dir = std::env::temp_dir().join(format!(
            "mshell-sounds-safe-file-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let missing = dir.join("nope.ogg");
        assert!(!is_safe_sound_file(missing.to_str().unwrap()));

        let empty = dir.join("empty.ogg");
        std::fs::write(&empty, []).unwrap();
        assert!(!is_safe_sound_file(empty.to_str().unwrap()));

        let normal = dir.join("normal.ogg");
        std::fs::write(&normal, b"not really ogg but nonzero").unwrap();
        assert!(is_safe_sound_file(normal.to_str().unwrap()));

        // Sparse file — no real disk use, just an inflated reported size.
        let oversized = dir.join("huge.ogg");
        let f = std::fs::File::create(&oversized).unwrap();
        f.set_len(MAX_NOTIFICATION_SOUND_BYTES + 1).unwrap();
        assert!(!is_safe_sound_file(oversized.to_str().unwrap()));

        assert!(!is_safe_sound_file(dir.to_str().unwrap()), "a directory");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
