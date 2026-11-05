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
/// One-shot "timer finished" chime (Pomodoro phase end, eye-care break).
const TIMER_SOUND: &[u8] = include_bytes!("../assets/timer.ogg");

/// Whether the looping alarm tone is currently ringing. Drives both the loop
/// thread and `alarm_is_ringing()`.
static ALARM_PLAYING: AtomicBool = AtomicBool::new(false);

/// Called (from any thread) whenever [`alarm_is_ringing`] flips, so a UI can
/// react instead of polling the flag.
static ALARM_LISTENER: std::sync::Mutex<Option<fn()>> = std::sync::Mutex::new(None);

/// Register the callback run whenever the ringing state changes. One
/// listener; a later call replaces it. It may run on the alarm thread.
pub fn set_alarm_state_listener(listener: fn()) {
    if let Ok(mut slot) = ALARM_LISTENER.lock() {
        *slot = Some(listener);
    }
}

fn alarm_state_changed() {
    let listener = ALARM_LISTENER.lock().ok().and_then(|slot| *slot);
    if let Some(listener) = listener {
        listener();
    }
}

/// Clear the ringing flag, telling the listener if it was set.
fn clear_ringing() {
    if ALARM_PLAYING.swap(false, Ordering::SeqCst) {
        alarm_state_changed();
    }
}

/// Start the alarm tone, looping until [`stop_alarm`]. No-op if already ringing.
pub fn play_alarm_loop() {
    if ALARM_PLAYING.swap(true, Ordering::SeqCst) {
        return; // already ringing
    }
    alarm_state_changed();
    std::thread::spawn(|| {
        let Ok(mut handle) = rodio::DeviceSinkBuilder::open_default_sink() else {
            clear_ringing();
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
        clear_ringing();
    });
}

/// Stop the looping alarm tone.
pub fn stop_alarm() {
    clear_ringing();
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

/// Extensions rodio can decode here (vorbis + mp3 features; no WAV/FLAC).
const PLAYABLE_EXTENSIONS: &[&str] = &["ogg", "oga", "mp3"];

/// Resolve a user-configured sound (a notification sound rule): a file is
/// returned as-is; a folder yields one of its playable files, picked by
/// `seed` from the name-sorted list (so tests are deterministic and the
/// caller supplies the randomness). `None` for an empty folder or a
/// missing path.
pub fn resolve_sound_path(path: &str, seed: u64) -> Option<std::path::PathBuf> {
    let path = std::path::Path::new(path.trim());
    if path.as_os_str().is_empty() {
        return None;
    }
    if path.is_file() {
        return Some(path.to_path_buf());
    }
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(path)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| PLAYABLE_EXTENSIONS.contains(&x.to_ascii_lowercase().as_str()))
        })
        .collect();
    if files.is_empty() {
        return None;
    }
    files.sort();
    let idx = (seed % files.len() as u64) as usize;
    Some(files.swap_remove(idx))
}

/// Play a user-configured sound file, or a random file from a folder (see
/// [`resolve_sound_path`]). Returns `false` when the path resolves to no
/// playable file (moved, deleted, empty folder) so the caller can fall back
/// to the default sound instead of staying silent; that case is logged once
/// per path. An undecodable file still counts as played (silent).
pub fn play_sound_path_or_folder(path: &str) -> bool {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 ^ d.as_secs())
        .unwrap_or(0);
    match resolve_sound_path(path, seed) {
        Some(file) => {
            play_notification_file(&file.to_string_lossy());
            true
        }
        None => {
            if first_miss(path) {
                tracing::warn!(
                    path,
                    "sound: no playable file at this path; using the default sound"
                );
            }
            false
        }
    }
}

/// `true` the first time `path` is reported missing (one warning per path,
/// not one per notification).
fn first_miss(path: &str) -> bool {
    static WARNED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let Ok(mut warned) = WARNED.lock() else {
        return false;
    };
    if warned.iter().any(|p| p == path) {
        return false;
    }
    warned.push(path.to_string());
    true
}

/// Fire-and-forget playback of already-fetched/generated audio bytes (e.g.
/// an mp3 TTS clip) on a background thread — same shape as
/// [`play_notification_file`], but for bytes already in hand rather than a
/// path. Decode/playback failures are silent.
pub fn play_bytes(bytes: Vec<u8>) {
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

/// Fire-and-forget playback of an embedded clip on its own thread.
fn play_embedded(bytes: &'static [u8]) {
    play_embedded_padded(bytes, Duration::ZERO, Duration::ZERO);
}

/// [`play_embedded`] with a delay before opening the device and a hold after
/// the clip ends. No audio device (or a busy one) is logged and skipped —
/// a sound effect must never take a thread down.
fn play_embedded_padded(bytes: &'static [u8], before: Duration, after: Duration) {
    std::thread::spawn(move || {
        if !before.is_zero() {
            std::thread::sleep(before);
        }
        let mut handle = match rodio::DeviceSinkBuilder::open_default_sink() {
            Ok(h) => h,
            Err(e) => {
                tracing::debug!(error = %e, "sounds: no default audio device; skipping clip");
                return;
            }
        };
        handle.log_on_drop(false);
        if let Ok(player) = rodio::play(handle.mixer(), Cursor::new(bytes)) {
            player.sleep_until_end();
            if !after.is_zero() {
                std::thread::sleep(after);
            }
        }
    });
}

/// Play the "timer finished" chime once (Pomodoro phase end, eye-care break
/// start). Unlike [`play_alarm_loop`] it never loops and needs no stop.
pub fn play_timer() {
    play_embedded(TIMER_SOUND);
}

pub fn play_shutter() {
    play_embedded(CAMERA_SHUTTER_SOUND);
}

pub fn play_audio_volume_change() {
    // Give the volume change a moment to land first; hold the (very short)
    // clip's device open briefly so it actually plays out.
    play_embedded_padded(
        AUDIO_VOLUME_CHANGED_SOUND,
        Duration::from_millis(50),
        Duration::from_millis(100),
    );
}

pub fn play_battery_low() {
    play_embedded(BATTERY_LOW_SOUND);
}

pub fn play_power_plug_sound() {
    play_embedded(POWER_PLUG_SOUND);
}

pub fn play_power_unplug_sound() {
    play_embedded(POWER_UNPLUG_SOUND);
}

#[cfg(test)]
mod tests {
    use super::*;

    static LISTENER_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    fn count_call() {
        LISTENER_CALLS.fetch_add(1, Ordering::SeqCst);
    }

    /// The listener fires on a ringing → silent edge only, so a UI can wait
    /// on it instead of polling `alarm_is_ringing`. (`play_alarm_loop` is not
    /// driven here: it opens an audio device.)
    #[test]
    fn stopping_the_alarm_notifies_the_listener_once() {
        set_alarm_state_listener(count_call);
        ALARM_PLAYING.store(true, Ordering::SeqCst);
        let before = LISTENER_CALLS.load(Ordering::SeqCst);
        stop_alarm();
        assert!(!alarm_is_ringing());
        assert_eq!(LISTENER_CALLS.load(Ordering::SeqCst), before + 1);
        stop_alarm(); // already silent: no edge, no call
        assert_eq!(LISTENER_CALLS.load(Ordering::SeqCst), before + 1);
    }

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
            ("timer", TIMER_SOUND),
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

    #[test]
    fn a_missing_sound_reports_not_played_so_the_caller_can_fall_back() {
        assert!(!play_sound_path_or_folder(
            "/nonexistent/mshell-sounds-test.ogg"
        ));
        assert!(!play_sound_path_or_folder("   "));
    }

    #[test]
    fn a_missing_sound_is_warned_about_once_per_path() {
        let path = format!("/nonexistent/mshell-sounds-warn-{}.ogg", std::process::id());
        assert!(first_miss(&path));
        assert!(!first_miss(&path));
        assert!(first_miss(&format!("{path}.other")));
    }

    #[test]
    fn resolve_sound_path_handles_file_folder_and_missing() {
        let dir =
            std::env::temp_dir().join(format!("mshell-sounds-resolve-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.ogg");
        let b = dir.join("b.MP3");
        std::fs::write(&a, b"x").unwrap();
        std::fs::write(&b, b"x").unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        std::fs::write(dir.join("c.wav"), b"x").unwrap(); // not decodable here

        // A file resolves to itself.
        assert_eq!(resolve_sound_path(a.to_str().unwrap(), 7), Some(a.clone()));
        // A folder picks among playable files only, by seed.
        let d = dir.to_str().unwrap();
        assert_eq!(resolve_sound_path(d, 0), Some(a.clone()));
        assert_eq!(resolve_sound_path(d, 1), Some(b.clone()));
        assert_eq!(resolve_sound_path(d, 2), Some(a.clone()));
        // Missing / empty / blank.
        assert_eq!(resolve_sound_path("/nonexistent/x.ogg", 0), None);
        assert_eq!(resolve_sound_path("  ", 0), None);
        let empty = dir.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert_eq!(resolve_sound_path(empty.to_str().unwrap(), 0), None);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
