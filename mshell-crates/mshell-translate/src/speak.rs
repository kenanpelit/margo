//! Microsoft Edge neural-voice text-to-speech.
//!
//! Reverse-engineered protocol — the same one the `edge-tts` Python package
//! uses: a WebSocket to Microsoft's internal "read aloud" speech service,
//! authenticated with a `Sec-MS-GEC` token derived from the current time.
//! Not a public API; if Microsoft changes the endpoint, token formula or
//! headers, this needs the same update.
//!
//! Chosen over the free Google Translate TTS endpoint (used for text
//! translation elsewhere in this crate) for noticeably better voice quality
//! and real Turkish neural voices (`tr-TR-AhmetNeural` / `tr-TR-EmelNeural`).

use sha2::{Digest, Sha256};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tungstenite::Message;
use tungstenite::client::IntoClientRequest;
use uuid::Uuid;

/// Selectable English voices (Settings → Translate's "English voice" picker).
pub const EN_VOICES: &[&str] = &[
    "en-US-JennyNeural",
    "en-US-AriaNeural",
    "en-US-GuyNeural",
    "en-US-AndrewNeural",
    "en-GB-SoniaNeural",
    "en-GB-RyanNeural",
];

/// Selectable Turkish voices (Settings → Translate's "Turkish voice" picker).
pub const TR_VOICES: &[&str] = &["tr-TR-EmelNeural", "tr-TR-AhmetNeural"];

/// Known-good voices — a small curated set (not the service's full
/// catalogue, which numbers in the hundreds) covering English + Turkish.
/// [`synthesize`] validates against this; [`EN_VOICES`]/[`TR_VOICES`] are
/// the per-language subsets Settings actually offers a dropdown over.
pub const VOICES: &[&str] = &[
    "en-US-JennyNeural",
    "en-US-AriaNeural",
    "en-US-GuyNeural",
    "en-US-AndrewNeural",
    "en-GB-SoniaNeural",
    "en-GB-RyanNeural",
    "tr-TR-EmelNeural",
    "tr-TR-AhmetNeural",
];

/// A short, friendly label for a voice id, for the Settings dropdowns.
pub fn voice_label(voice: &str) -> &'static str {
    match voice {
        "en-US-JennyNeural" => "Jenny (US, female)",
        "en-US-AriaNeural" => "Aria (US, female)",
        "en-US-GuyNeural" => "Guy (US, male)",
        "en-US-AndrewNeural" => "Andrew (US, male)",
        "en-GB-SoniaNeural" => "Sonia (UK, female)",
        "en-GB-RyanNeural" => "Ryan (UK, male)",
        "tr-TR-EmelNeural" => "Emel (female)",
        "tr-TR-AhmetNeural" => "Ahmet (male)",
        _ => "Unknown",
    }
}

/// Per-request text cap the service enforces (empirically — longer requests
/// get a plain `400`).
pub const MAX_TEXT_CHARS: usize = 500;
const MIN_AUDIO_BYTES: usize = 200;

const TRUSTED_CLIENT_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
const CHROMIUM_MAJOR_VERSION: &str = "143";
const SEC_MS_GEC_VERSION: &str = "1-143.0.3650.75";
const HOST: &str = "speech.platform.bing.com";
const WIN_EPOCH_SECONDS: f64 = 11_644_473_600.0;
const TIMEOUT: Duration = Duration::from_secs(10);

fn now_unix_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// The `Sec-MS-GEC` value: SHA-256 of the Windows file time (rounded down to
/// five minutes) concatenated with the trusted client token, uppercase hex.
fn sec_ms_gec(unix_seconds: f64) -> String {
    let mut ticks = unix_seconds + WIN_EPOCH_SECONDS;
    ticks -= ticks % 300.0;
    let file_time = (ticks * 1e7).round() as i64;
    let mut hasher = Sha256::new();
    hasher.update(format!("{file_time}{TRUSTED_CLIENT_TOKEN}").as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect()
}

/// "tr-TR-AhmetNeural" -> "Microsoft Server Speech Text to Speech Voice (tr-TR, AhmetNeural)".
fn long_voice_name(voice: &str) -> Option<String> {
    let first_dash = voice.find('-')?;
    let second_dash = voice[first_dash + 1..].find('-')? + first_dash + 1;
    Some(format!(
        "Microsoft Server Speech Text to Speech Voice ({}, {})",
        &voice[..second_dash],
        &voice[second_dash + 1..]
    ))
}

/// The service rejects a few control characters outright.
fn clean_text(text: &str) -> String {
    text.chars()
        .map(|c| {
            let code = c as u32;
            if code <= 8 || code == 11 || code == 12 || (14..=31).contains(&code) {
                ' '
            } else {
                c
            }
        })
        .collect()
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn build_ssml(long_name: &str, text: &str) -> String {
    format!(
        "<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='en-US'>\
         <voice name='{long_name}'><prosody pitch='+0Hz' rate='+0%' volume='+0%'>{}</prosody></voice></speak>",
        escape_xml(&clean_text(text))
    )
}

/// `Date.toString()`-shaped timestamp the service expects in `X-Timestamp`.
fn timestamp() -> String {
    chrono::Utc::now()
        .format("%a %b %d %Y %H:%M:%S GMT+0000 (Coordinated Universal Time)")
        .to_string()
}

/// 32 lowercase hex chars — a v4 UUID's bytes are random enough for a
/// connection id / request id / cookie value, no separate `rand` dependency.
fn random_hex32() -> String {
    Uuid::new_v4().simple().to_string()
}

enum AttemptErr {
    /// 403 with a `Date` header. In practice this is almost always the
    /// backend transiently rejecting a well-formed request (rate limiting
    /// / server routing — a plain retry often just works) rather than an
    /// actual clock problem; the server's reported time is applied to the
    /// retry anyway in case this machine's clock genuinely is off.
    Skew(f64),
    Msg(String),
}

/// Pick the voice to speak `lang` with, per the user's configured
/// `voice_en`/`voice_tr` (Settings → Translate) — falls back to
/// `settings.voice_en` for anything that isn't Turkish, same as before
/// these were configurable.
pub fn voice_for_lang(lang: &str, settings: &crate::config::TranslateSettings) -> String {
    match lang.trim().to_lowercase().as_str() {
        "tr" => settings.voice_tr.clone(),
        _ => settings.voice_en.clone(),
    }
}

/// Synthesize `text` spoken by `voice` (must be one of [`VOICES`]),
/// returning raw mp3 bytes.
pub fn synthesize(voice: &str, text: &str) -> Result<Vec<u8>, String> {
    // rustls 0.23+ panics in `ClientConfig::builder()` (tungstenite's TLS
    // connect path) unless a process-wide default CryptoProvider was
    // installed first. Idempotent — a second call just returns `Err`
    // ("already installed"), which is exactly what we want here.
    let _ = rustls::crypto::ring::default_provider().install_default();

    if !VOICES.contains(&voice) {
        return Err("unknown voice".into());
    }
    let clean = text.trim();
    if clean.is_empty() || clean.chars().count() > MAX_TEXT_CHARS {
        return Err("text length".into());
    }

    // The backend 403s a well-formed request often enough (observed: fails,
    // then an identical retry a moment later succeeds) that a single
    // attempt isn't reliable — a short, limited retry absorbs that without
    // the caller needing to click the button again.
    const MAX_ATTEMPTS: u32 = 3;
    let mut skew = 0.0;
    let mut last_err = String::new();
    for n in 0..MAX_ATTEMPTS {
        match attempt(voice, clean, skew) {
            Ok(audio) => return Ok(audio),
            Err(AttemptErr::Skew(server_seconds)) => {
                skew = server_seconds - now_unix_seconds();
                last_err = "rejected by the service".into();
            }
            Err(AttemptErr::Msg(m)) => last_err = m,
        }
        if n + 1 < MAX_ATTEMPTS {
            std::thread::sleep(Duration::from_millis(400));
        }
    }
    Err(last_err)
}

fn attempt(voice: &str, text: &str, skew_seconds: f64) -> Result<Vec<u8>, AttemptErr> {
    let long_name = long_voice_name(voice).ok_or_else(|| AttemptErr::Msg("bad voice".into()))?;
    let now = now_unix_seconds() + skew_seconds;

    let url = format!(
        "wss://{HOST}/consumer/speech/synthesize/readaloud/edge/v1\
         ?TrustedClientToken={TRUSTED_CLIENT_TOKEN}\
         &ConnectionId={}&Sec-MS-GEC={}&Sec-MS-GEC-Version={SEC_MS_GEC_VERSION}",
        random_hex32(),
        sec_ms_gec(now),
    );

    let mut request = url
        .into_client_request()
        .map_err(|e| AttemptErr::Msg(format!("bad request: {e}")))?;
    let user_agent = format!(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) \
         Chrome/{v}.0.0.0 Safari/537.36 Edg/{v}.0.0.0",
        v = CHROMIUM_MAJOR_VERSION
    );
    let cookie = format!("muid={};", random_hex32());
    for (name, value) in [
        ("Pragma", "no-cache".to_string()),
        ("Cache-Control", "no-cache".to_string()),
        (
            "Origin",
            "chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold".to_string(),
        ),
        ("User-Agent", user_agent),
        ("Accept-Language", "en-US,en;q=0.9".to_string()),
        ("Cookie", cookie),
    ] {
        let header_value = value
            .parse()
            .map_err(|e| AttemptErr::Msg(format!("bad header: {e}")))?;
        request.headers_mut().insert(name, header_value);
    }

    let addr = (HOST, 443u16)
        .to_socket_addrs()
        .map_err(|e| AttemptErr::Msg(format!("dns: {e}")))?
        .next()
        .ok_or_else(|| AttemptErr::Msg("dns: no address".into()))?;
    let tcp = TcpStream::connect_timeout(&addr, TIMEOUT)
        .map_err(|e| AttemptErr::Msg(format!("connect: {e}")))?;
    tcp.set_read_timeout(Some(TIMEOUT)).ok();
    tcp.set_write_timeout(Some(TIMEOUT)).ok();

    let (mut socket, _response) = match tungstenite::client_tls(request, tcp) {
        Ok(ok) => ok,
        Err(tungstenite::HandshakeError::Failure(tungstenite::Error::Http(response))) => {
            if response.status() == 403
                && let Some(server_time) = response
                    .headers()
                    .get("Date")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|d| chrono::DateTime::parse_from_rfc2822(d).ok())
            {
                return Err(AttemptErr::Skew(server_time.timestamp() as f64));
            }
            return Err(AttemptErr::Msg(format!("HTTP {}", response.status())));
        }
        Err(e) => return Err(AttemptErr::Msg(format!("handshake failed: {e}"))),
    };

    let config_msg = format!(
        "X-Timestamp:{}\r\n\
         Content-Type:application/json; charset=utf-8\r\n\
         Path:speech.config\r\n\r\n\
         {{\"context\":{{\"synthesis\":{{\"audio\":{{\"metadataoptions\":{{\
         \"sentenceBoundaryEnabled\":\"false\",\"wordBoundaryEnabled\":\"false\"}},\
         \"outputFormat\":\"audio-24khz-48kbitrate-mono-mp3\"}}}}}}}}\r\n",
        timestamp()
    );
    socket
        .send(Message::text(config_msg))
        .map_err(|e| AttemptErr::Msg(format!("send config: {e}")))?;

    let ssml = build_ssml(&long_name, text);
    let ssml_msg = format!(
        "X-RequestId:{}\r\n\
         Content-Type:application/ssml+xml\r\n\
         X-Timestamp:{}Z\r\n\
         Path:ssml\r\n\r\n{ssml}",
        random_hex32(),
        timestamp()
    );
    socket
        .send(Message::text(ssml_msg))
        .map_err(|e| AttemptErr::Msg(format!("send ssml: {e}")))?;

    let mut audio = Vec::new();
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if Instant::now() >= deadline {
            return Err(AttemptErr::Msg("timeout".into()));
        }
        let msg = match socket.read() {
            Ok(m) => m,
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => break,
            Err(e) => return Err(AttemptErr::Msg(format!("read failed: {e}"))),
        };
        match msg {
            Message::Text(text) if text.as_str().contains("Path:turn.end") => break,
            Message::Binary(data) => {
                if data.len() < 2 {
                    continue;
                }
                let header_len = u16::from_be_bytes([data[0], data[1]]) as usize;
                let start = 2 + header_len;
                if start > data.len() {
                    continue;
                }
                let header_text = String::from_utf8_lossy(&data[2..start]);
                if header_text.contains("Path:audio") && data.len() > start {
                    audio.extend_from_slice(&data[start..]);
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    let _ = socket.close(None);

    if audio.len() < MIN_AUDIO_BYTES {
        return Err(AttemptErr::Msg("no audio received".into()));
    }
    Ok(audio)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gec_is_stable_within_a_five_minute_window() {
        let a = sec_ms_gec(1_700_000_000.0);
        let b = sec_ms_gec(1_700_000_010.0);
        assert_eq!(a, b);
        let c = sec_ms_gec(1_700_000_400.0);
        assert_ne!(a, c);
    }

    #[test]
    fn long_voice_name_shape() {
        assert_eq!(
            long_voice_name("tr-TR-AhmetNeural").unwrap(),
            "Microsoft Server Speech Text to Speech Voice (tr-TR, AhmetNeural)"
        );
        assert!(
            long_voice_name("no-dashes-here-enough")
                .unwrap()
                .starts_with("Microsoft Server Speech Text to Speech Voice (")
        );
        assert!(long_voice_name("onedash").is_none());
    }

    #[test]
    fn ssml_escapes_and_builds() {
        let ssml = build_ssml(
            "Microsoft Server Speech Text to Speech Voice (en-US, GuyNeural)",
            "<hello> & \"world\"",
        );
        assert!(ssml.contains("&lt;hello&gt; &amp; &quot;world&quot;"));
        assert!(ssml.contains("GuyNeural"));
    }

    #[test]
    fn rejects_unknown_voice_and_bad_text() {
        assert!(synthesize("not-a-real-voice", "hello").is_err());
        assert!(synthesize("en-US-GuyNeural", "   ").is_err());
        let long = "a".repeat(MAX_TEXT_CHARS + 1);
        assert!(synthesize("en-US-GuyNeural", &long).is_err());
    }
}
