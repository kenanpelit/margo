//! Discord local-RPC protocol + voice-state model for the Discord voice
//! pill/menu (discord-voice port). Pure and tested; the socket client
//! that drives it lives in `mshell-frame/src/discord.rs`.
//!
//! Discord's desktop client listens on `$XDG_RUNTIME_DIR/discord-ipc-N`
//! (also under the Flatpak/Snap app dirs and `$TMPDIR`). Frames are
//! `u32 opcode (LE) + u32 length (LE) + JSON`. After a handshake with a
//! client id, commands are `{cmd, args, nonce}` and Discord answers with
//! the same nonce or pushes `DISPATCH` events we `SUBSCRIBE`d to.
//!
//! Authorisation uses Discord's public StreamKit client id (the same
//! identity the official OBS overlay uses), so no app secret is needed:
//! `AUTHORIZE` shows Discord's consent prompt and returns a code, the
//! StreamKit broker exchanges it for an access token, `AUTHENTICATE`
//! with that token unlocks the voice scopes. Only `rpc`,
//! `rpc.voice.read` and `rpc.voice.write` are requested.

use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const STREAMKIT_CLIENT_ID: &str = "207646673902501888";
pub const OAUTH_SCOPES: [&str; 3] = ["rpc", "rpc.voice.read", "rpc.voice.write"];
pub const TOKEN_EXCHANGE_URL: &str = "https://streamkit.discord.com/overlay/token";

pub const OP_HANDSHAKE: u32 = 0;
pub const OP_FRAME: u32 = 1;
pub const OP_CLOSE: u32 = 2;
pub const OP_PING: u32 = 3;
pub const OP_PONG: u32 = 4;

/// Refuse frames larger than this (a corrupt length must not make us
/// allocate gigabytes).
pub const MAX_FRAME: u32 = 8 * 1024 * 1024;

/// Encode one IPC frame.
pub fn encode_frame(op: u32, payload: &Value) -> Vec<u8> {
    let body = payload.to_string().into_bytes();
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&op.to_le_bytes());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// Decode a frame header → (opcode, body length), rejecting oversize.
pub fn decode_header(h: &[u8; 8]) -> Result<(u32, u32), String> {
    let op = u32::from_le_bytes([h[0], h[1], h[2], h[3]]);
    let len = u32::from_le_bytes([h[4], h[5], h[6], h[7]]);
    if len > MAX_FRAME {
        return Err(format!("Discord RPC frame too large ({len} bytes)"));
    }
    Ok((op, len))
}

/// Where Discord may have put its sockets, in probe order.
pub fn socket_candidates(runtime: Option<&Path>, tmp_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(r) = runtime {
        dirs.push(r.to_path_buf());
        dirs.push(r.join("app/com.discordapp.Discord"));
        dirs.push(r.join("app/com.discordapp.DiscordCanary"));
        dirs.push(r.join("snap.discord"));
    }
    dirs.extend(tmp_dirs.iter().cloned());
    dirs.push(PathBuf::from("/tmp"));
    let mut out = Vec::new();
    for d in dirs {
        for i in 0..10 {
            let p = d.join(format!("discord-ipc-{i}"));
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}

pub fn handshake_payload() -> Value {
    json!({ "v": 1, "client_id": STREAMKIT_CLIENT_ID })
}

/// `{cmd, args, nonce[, evt]}`.
pub fn command(cmd: &str, args: Value, nonce: &str, evt: Option<&str>) -> Value {
    let mut v = json!({ "cmd": cmd, "args": args, "nonce": nonce });
    if let Some(e) = evt {
        v["evt"] = json!(e);
    }
    v
}

pub fn authorize_args() -> Value {
    json!({ "client_id": STREAMKIT_CLIENT_ID, "scopes": OAUTH_SCOPES, "prompt": "none" })
}

/// Events subscribed per voice channel.
pub const CHANNEL_EVENTS: [&str; 5] = [
    "VOICE_STATE_CREATE",
    "VOICE_STATE_UPDATE",
    "VOICE_STATE_DELETE",
    "SPEAKING_START",
    "SPEAKING_STOP",
];
/// Global events (no channel argument).
pub const GLOBAL_EVENTS: [&str; 2] = ["VOICE_CHANNEL_SELECT", "VOICE_SETTINGS_UPDATE"];

// ── State ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiscordStatus {
    /// No Discord desktop socket.
    #[default]
    NotRunning,
    /// Connected, but no (valid) token — the user must authorise once.
    NeedsAuth,
    /// Consent prompt open in Discord / token exchange running.
    Authorizing,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Participant {
    pub id: String,
    pub name: String,
    pub speaking: bool,
    /// Muted by self or server.
    pub muted: bool,
    pub deafened: bool,
    /// Local playback volume 0..=200 (%).
    pub volume: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiscordState {
    pub status: DiscordStatus,
    /// Last error worth showing (auth failure, rejected command).
    pub message: String,
    pub user_id: String,
    pub channel_id: Option<String>,
    pub channel_name: String,
    pub participants: BTreeMap<String, Participant>,
    pub self_mute: bool,
    pub self_deaf: bool,
}

impl DiscordState {
    pub fn in_voice(&self) -> bool {
        self.channel_id.is_some()
    }

    pub fn speaking_count(&self) -> usize {
        self.participants.values().filter(|p| p.speaking).count()
    }

    /// Participants sorted: you first, then by name.
    pub fn sorted_participants(&self) -> Vec<&Participant> {
        let mut v: Vec<&Participant> = self.participants.values().collect();
        v.sort_by(|a, b| {
            (a.id != self.user_id)
                .cmp(&(b.id != self.user_id))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        v
    }

    /// Enter a channel from a `GET_SELECTED_VOICE_CHANNEL` response
    /// (`null` / no id = not in voice). Speaking flags of people still
    /// present survive a refresh.
    pub fn set_channel(&mut self, data: &Value) {
        let Some(id) = data.get("id").and_then(Value::as_str) else {
            self.leave();
            return;
        };
        let same = self.channel_id.as_deref() == Some(id);
        let old = std::mem::take(&mut self.participants);
        self.channel_id = Some(id.to_string());
        self.channel_name = data
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Voice channel")
            .to_string();
        if let Some(states) = data.get("voice_states").and_then(Value::as_array) {
            for s in states {
                self.upsert(s);
            }
        }
        if same {
            for (id, p) in self.participants.iter_mut() {
                p.speaking = old.get(id).is_some_and(|o| o.speaking);
            }
        }
    }

    pub fn leave(&mut self) {
        self.channel_id = None;
        self.channel_name.clear();
        self.participants.clear();
    }

    /// Insert/update a participant from a voice-state object.
    pub fn upsert(&mut self, data: &Value) {
        let user = data.get("user").cloned().unwrap_or(Value::Null);
        let Some(id) = user
            .get("id")
            .and_then(Value::as_str)
            .or_else(|| data.get("user_id").and_then(Value::as_str))
        else {
            return;
        };
        let voice = data.get("voice_state").cloned().unwrap_or(Value::Null);
        let flag = |k: &str| voice.get(k).and_then(Value::as_bool).unwrap_or(false);
        let prev = self.participants.get(id).cloned().unwrap_or_default();
        let name = data
            .get("nick")
            .and_then(Value::as_str)
            .or_else(|| user.get("global_name").and_then(Value::as_str))
            .or_else(|| user.get("username").and_then(Value::as_str))
            .map(str::to_string)
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| {
                if prev.name.is_empty() {
                    "Unknown".to_string()
                } else {
                    prev.name.clone()
                }
            });
        let volume = data
            .get("volume")
            .and_then(Value::as_f64)
            .map(|v| v.round().clamp(0.0, 200.0) as u32)
            .unwrap_or(if prev.id.is_empty() { 100 } else { prev.volume });
        self.participants.insert(
            id.to_string(),
            Participant {
                id: id.to_string(),
                name,
                speaking: prev.speaking,
                muted: flag("mute") || flag("self_mute"),
                deafened: flag("deaf") || flag("self_deaf"),
                volume,
            },
        );
    }

    /// `GET_VOICE_SETTINGS` response / `VOICE_SETTINGS_UPDATE` event.
    pub fn set_voice_settings(&mut self, data: &Value) {
        if let Some(m) = data.get("mute").and_then(Value::as_bool) {
            self.self_mute = m;
        }
        if let Some(d) = data.get("deaf").and_then(Value::as_bool) {
            self.self_deaf = d;
        }
    }

    /// Apply a `DISPATCH` event. Returns `true` when the selected channel
    /// changed and the client should re-fetch it (and re-subscribe).
    pub fn apply_dispatch(&mut self, evt: &str, data: &Value) -> bool {
        match evt {
            "VOICE_CHANNEL_SELECT" => {
                let new = data.get("channel_id").and_then(Value::as_str);
                if new.is_none() {
                    self.leave();
                }
                new.is_some()
            }
            "VOICE_SETTINGS_UPDATE" => {
                self.set_voice_settings(data);
                false
            }
            "VOICE_STATE_CREATE" | "VOICE_STATE_UPDATE" => {
                if self.in_voice() {
                    self.upsert(data);
                }
                false
            }
            "VOICE_STATE_DELETE" => {
                let id = data
                    .pointer("/user/id")
                    .and_then(Value::as_str)
                    .or_else(|| data.get("user_id").and_then(Value::as_str));
                if let Some(id) = id {
                    self.participants.remove(id);
                }
                false
            }
            "SPEAKING_START" | "SPEAKING_STOP" => {
                if let Some(p) = data
                    .get("user_id")
                    .and_then(Value::as_str)
                    .and_then(|id| self.participants.get_mut(id))
                {
                    p.speaking = evt == "SPEAKING_START";
                }
                false
            }
            _ => false,
        }
    }
}

/// Bar pill text: channel name (+ speaking count), empty when idle.
pub fn pill_label(s: &DiscordState) -> String {
    if !s.in_voice() {
        return String::new();
    }
    match s.speaking_count() {
        0 => s.channel_name.clone(),
        n => format!("{} · {n}", s.channel_name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip_and_size_guard() {
        let f = encode_frame(OP_FRAME, &json!({"a": 1}));
        let mut h = [0u8; 8];
        h.copy_from_slice(&f[..8]);
        let (op, len) = decode_header(&h).unwrap();
        assert_eq!(op, OP_FRAME);
        assert_eq!(len as usize, f.len() - 8);
        assert_eq!(&f[8..], br#"{"a":1}"#);
        let mut big = [0u8; 8];
        big[4..].copy_from_slice(&(MAX_FRAME + 1).to_le_bytes());
        assert!(decode_header(&big).is_err());
    }

    #[test]
    fn socket_probe_order() {
        let c = socket_candidates(Some(Path::new("/run/user/1000")), &[PathBuf::from("/tmp")]);
        assert_eq!(c[0], PathBuf::from("/run/user/1000/discord-ipc-0"));
        assert!(c.contains(&PathBuf::from(
            "/run/user/1000/app/com.discordapp.Discord/discord-ipc-3"
        )));
        // /tmp listed once even though it's both a tmp dir and the fallback.
        assert_eq!(
            c.iter()
                .filter(|p| p.ends_with("discord-ipc-0") && p.starts_with("/tmp"))
                .count(),
            1
        );
    }

    #[test]
    fn command_shape() {
        let c = command(
            "SUBSCRIBE",
            json!({"channel_id": "9"}),
            "n1",
            Some("SPEAKING_START"),
        );
        assert_eq!(c["cmd"], "SUBSCRIBE");
        assert_eq!(c["evt"], "SPEAKING_START");
        assert_eq!(c["nonce"], "n1");
        let a = authorize_args();
        assert_eq!(
            a["scopes"],
            json!(["rpc", "rpc.voice.read", "rpc.voice.write"])
        );
    }

    // GET_SELECTED_VOICE_CHANNEL response shape (Discord RPC docs).
    fn channel() -> Value {
        json!({
            "id": "199737254929760256", "name": "General", "type": 2, "guild_id": "1",
            "voice_states": [
                {"nick": "kenan", "mute": false, "volume": 100,
                 "voice_state": {"mute": false, "deaf": false, "self_mute": true, "self_deaf": false},
                 "user": {"id": "10", "username": "kenan", "global_name": "Kenan"}},
                {"mute": false, "volume": 150.4,
                 "voice_state": {"mute": false, "deaf": true, "self_mute": false, "self_deaf": false},
                 "user": {"id": "20", "username": "alice", "global_name": null}}
            ]
        })
    }

    #[test]
    fn channel_and_participants() {
        let mut s = DiscordState {
            user_id: "10".into(),
            ..Default::default()
        };
        s.set_channel(&channel());
        assert!(s.in_voice());
        assert_eq!(s.channel_name, "General");
        let p = s.sorted_participants();
        assert_eq!(p[0].name, "kenan", "you first, nick wins");
        assert!(p[0].muted);
        assert_eq!(p[1].name, "alice", "username when no global name");
        assert!(p[1].deafened);
        assert_eq!(p[1].volume, 150);
        s.set_channel(&Value::Null);
        assert!(!s.in_voice());
        assert!(s.participants.is_empty());
    }

    #[test]
    fn dispatch_events() {
        let mut s = DiscordState::default();
        s.set_channel(&channel());
        assert!(!s.apply_dispatch("SPEAKING_START", &json!({"user_id": "20"})));
        assert_eq!(s.speaking_count(), 1);
        assert_eq!(pill_label(&s), "General · 1");
        // A refresh of the same channel keeps the speaking flag.
        s.set_channel(&channel());
        assert_eq!(s.speaking_count(), 1);
        s.apply_dispatch("SPEAKING_STOP", &json!({"user_id": "20"}));
        assert_eq!(pill_label(&s), "General");
        s.apply_dispatch(
            "VOICE_STATE_CREATE",
            &json!({"user": {"id": "30", "username": "bob"}, "voice_state": {}}),
        );
        assert_eq!(s.participants["30"].volume, 100);
        s.apply_dispatch("VOICE_STATE_DELETE", &json!({"user": {"id": "30"}}));
        assert!(!s.participants.contains_key("30"));
        s.apply_dispatch(
            "VOICE_SETTINGS_UPDATE",
            &json!({"mute": true, "deaf": false}),
        );
        assert!(s.self_mute && !s.self_deaf);
        assert!(s.apply_dispatch("VOICE_CHANNEL_SELECT", &json!({"channel_id": "5"})));
        assert!(!s.apply_dispatch("VOICE_CHANNEL_SELECT", &json!({"channel_id": null})));
        assert!(!s.in_voice());
        assert_eq!(pill_label(&s), "");
        // Voice-state events outside a channel are ignored.
        s.apply_dispatch("VOICE_STATE_UPDATE", &json!({"user": {"id": "40"}}));
        assert!(s.participants.is_empty());
    }
}
