// SPDX-License-Identifier: GPL-3.0-or-later

//! Network songs: internet radio stations and podcast episodes.
//!
//! A local file's metadata comes from its tags; a stream's comes from
//! where the user found it (the station directory, the podcast feed) plus
//! what the stream itself reports while playing (an Icecast `StreamTitle`,
//! the real episode length). The rules for combining them live here as
//! pure functions.

use sha2::{Digest, Sha256};

/// What a queue entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StreamKind {
    /// A local file (every song before radio/podcasts existed).
    #[default]
    File,
    /// A live internet radio stream: no duration, not seekable.
    Radio,
    /// A podcast episode: a remote file with a duration.
    Episode,
}

/// Everything needed to queue a network song.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StreamInfo {
    pub kind: StreamKind,
    /// The http(s) URL GStreamer plays.
    pub uri: String,
    /// Stable identity: the station key / the episode key.
    pub key: String,
    /// Station name / episode title.
    pub title: String,
    /// Radio: country and genres; episode: the podcast's author.
    pub artist: String,
    /// Radio: "Internet radio"; episode: the podcast's title.
    pub album: String,
    /// Seconds; 0 for radio (and for an episode whose feed gave none).
    pub duration: u64,
    /// Logo / cover image URL, fetched lazily.
    pub art_url: String,
}

/// The queue identity of a stream (what `Song::equals` compares), so the
/// same station or episode is never queued twice.
pub fn stream_uuid(kind: StreamKind, key: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"mtune-stream\0");
    h.update(match kind {
        StreamKind::File => b"file".as_slice(),
        StreamKind::Radio => b"radio".as_slice(),
        StreamKind::Episode => b"episode".as_slice(),
    });
    h.update(b"\0");
    h.update(key.as_bytes());
    format!("{:x}", h.finalize())
}

/// Clean an Icecast/Shoutcast `StreamTitle` (GStreamer's `title` tag on a
/// live stream). `None` when it carries nothing worth showing: empty, just
/// separators, or the station's own name repeated.
pub fn clean_stream_title(raw: &str, station: &str) -> Option<String> {
    let t = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let t = t.trim_matches(|c: char| c == '-' || c == '|' || c.is_whitespace());
    if t.is_empty() || t.eq_ignore_ascii_case(station.trim()) {
        return None;
    }
    // Some servers send placeholders while a jingle or ad plays.
    let lower = t.to_ascii_lowercase();
    if matches!(lower.as_str(), "unknown" | "n/a" | "null" | "untitled") {
        return None;
    }
    Some(t.to_string())
}

/// What a radio entry shows as (title, artist): the current programme
/// under the station's name while the stream names one, else the station
/// over its description. This is what MPRIS — and so the shell's media
/// widget — displays.
pub fn radio_display(
    station: &str,
    description: &str,
    now_playing: Option<&str>,
) -> (String, String) {
    match now_playing {
        Some(np) => (np.to_string(), station.to_string()),
        None => (station.to_string(), description.to_string()),
    }
}

/// GstPlay's error text without its element path: `"Error from element
/// /GstPlayBin3:…/GstSoupHTTPSrc:souphttpsrc8: GStreamer encountered a
/// general stream error."` -> `"GStreamer encountered a general stream
/// error."`.
pub fn clean_gst_error(msg: &str) -> String {
    let msg = msg.trim();
    match msg.strip_prefix("Error from element ") {
        Some(rest) => rest
            .split_once(": ")
            .map(|(_, reason)| reason.trim().to_string())
            .unwrap_or_else(|| msg.to_string()),
        None => msg.to_string(),
    }
}

/// Whether an episode counts as played at `position` of `duration`
/// seconds: in its last 30 s, or past 95 % for shorts. An unknown length
/// never finishes by position (only by reaching its end).
pub fn episode_finished(position: u64, duration: u64) -> bool {
    if duration == 0 {
        return false;
    }
    position + 30 >= duration || position * 100 >= duration * 95
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_differs_by_kind_and_key() {
        let a = stream_uuid(StreamKind::Radio, "rb:1");
        assert_eq!(a, stream_uuid(StreamKind::Radio, "rb:1"));
        assert_ne!(a, stream_uuid(StreamKind::Radio, "rb:2"));
        assert_ne!(a, stream_uuid(StreamKind::Episode, "rb:1"));
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn a_real_icy_title_is_kept_whole() {
        // Captured from https://icecast.walmradio.com:8443/jazz via GstPlay
        // (audio stream `title` tag).
        let raw = "My Foolish Heart by Bill Watrous / Pete Christlieb / The Gary Urwin Jazz \
                   Orchestra - Adroit Jazz Underground on walmradio.com";
        assert_eq!(
            clean_stream_title(raw, "Adroit Jazz Underground").as_deref(),
            Some(raw)
        );
    }

    #[test]
    fn empty_and_placeholder_titles_are_dropped() {
        for raw in [
            "",
            "   ",
            " - ",
            "-",
            "||",
            "Unknown",
            "Jazz FM",
            "  jazz fm ",
        ] {
            assert_eq!(clean_stream_title(raw, "Jazz FM"), None, "{raw:?}");
        }
        assert_eq!(
            clean_stream_title("  Artist   -  Song - ", "Jazz FM").as_deref(),
            Some("Artist - Song")
        );
    }

    #[test]
    fn radio_shows_the_programme_under_the_station() {
        assert_eq!(
            radio_display("Jazz FM", "UK · jazz", Some("Artist - Song")),
            ("Artist - Song".to_string(), "Jazz FM".to_string())
        );
        assert_eq!(
            radio_display("Jazz FM", "UK · jazz", None),
            ("Jazz FM".to_string(), "UK · jazz".to_string())
        );
    }

    #[test]
    fn gst_errors_lose_the_element_path() {
        // Captured: an unresolvable custom stream URL under GstPlay.
        let raw = "Error from element /GstPlayBin3:playbin3/GstURIDecodeBin3:uridecodebin3/\
                   GstURISourceBin:urisourcebin0/GstSoupHTTPSrc:souphttpsrc8: GStreamer \
                   encountered a general stream error.";
        assert_eq!(
            clean_gst_error(raw),
            "GStreamer encountered a general stream error."
        );
        assert_eq!(clean_gst_error(" Not found "), "Not found");
    }

    #[test]
    fn an_episode_is_played_near_its_end() {
        assert!(!episode_finished(0, 3944));
        assert!(!episode_finished(3700, 3944));
        // Past 95 %: the outro / end credits do not need to play out.
        assert!(episode_finished(3800, 3944));
        assert!(episode_finished(3914, 3944));
        assert!(episode_finished(3944, 3944));
        // A 2-minute short: the last-30-s rule.
        assert!(!episode_finished(80, 120));
        assert!(episode_finished(90, 120));
        // A 3-hour episode: 95 % comes first (9 min before the end).
        assert!(episode_finished(10_260, 10_800));
        assert!(!episode_finished(10_000, 10_800));
        // Unknown length: never by position.
        assert!(!episode_finished(10_000, 0));
    }
}
