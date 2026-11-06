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
pub const GLOBAL_EVENTS: [&str; 3] = [
    "VOICE_CHANNEL_SELECT",
    "VOICE_SETTINGS_UPDATE",
    "VOICE_CONNECTION_STATUS",
];

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
    /// The current channel's server (empty for a group DM call).
    pub guild_id: String,
    pub guild_name: String,
    /// Your microphone input volume 0..=100 (%), once Discord reported it.
    pub input_volume: Option<u32>,
    /// `VOICE_CONNECTION_STATUS` state (`VOICE_CONNECTED`, `NO_ROUTE`, …).
    pub connection_state: String,
    /// Average voice-server ping in ms (0 = not measured yet).
    pub ping_ms: u32,
    /// Recent + favourite voice channels (persisted by the client).
    pub history: ChannelHistory,
}

/// How the voice connection reads in the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Latency {
    /// Connecting / checking (any state Discord reports in between).
    Connecting,
    /// Connected; the ping (ms, 0 = not measured yet).
    Good(u32),
    Fair(u32),
    Poor(u32),
    NoRoute,
    Disconnected,
}

impl Latency {
    /// The plugin's thresholds: ≤ 60 ms good, ≤ 120 ms fair, else poor.
    pub fn from_status(state: &str, ping_ms: u32) -> Self {
        match state {
            "VOICE_CONNECTED" => match ping_ms {
                0..=60 => Self::Good(ping_ms),
                61..=120 => Self::Fair(ping_ms),
                _ => Self::Poor(ping_ms),
            },
            "NO_ROUTE" => Self::NoRoute,
            "" | "DISCONNECTED" | "VOICE_DISCONNECTED" => Self::Disconnected,
            _ => Self::Connecting,
        }
    }

    pub fn label(self) -> String {
        match self {
            Self::Good(0) => "Connected".to_string(),
            Self::Good(ms) | Self::Fair(ms) | Self::Poor(ms) => format!("{ms} ms"),
            Self::Connecting => "Connecting…".to_string(),
            Self::NoRoute => "No route".to_string(),
            Self::Disconnected => "Disconnected".to_string(),
        }
    }
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
        let guild = data
            .get("guild_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if guild != self.guild_id {
            // A known name for this server (from the history) until
            // GET_GUILD answers.
            self.guild_name = self.history.guild_name(guild).unwrap_or_default();
            self.guild_id = guild.to_string();
        }
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
        self.guild_id.clear();
        self.guild_name.clear();
        self.connection_state.clear();
        self.ping_ms = 0;
    }

    pub fn latency(&self) -> Latency {
        Latency::from_status(&self.connection_state, self.ping_ms)
    }

    /// The current channel as a history entry (`None` outside voice).
    pub fn current_channel(&self, now: i64) -> Option<SavedChannel> {
        Some(SavedChannel {
            id: self.channel_id.clone()?,
            name: self.channel_name.clone(),
            guild_id: self.guild_id.clone(),
            guild_name: self.guild_name.clone(),
            last_joined: now,
        })
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
        if let Some(v) = data.pointer("/input/volume").and_then(Value::as_f64) {
            self.input_volume = Some(v.round().clamp(0.0, 100.0) as u32);
        }
    }

    /// `VOICE_CONNECTION_STATUS` event: state + average ping.
    pub fn set_connection_status(&mut self, data: &Value) {
        if let Some(st) = data.get("state").and_then(Value::as_str) {
            self.connection_state = st.to_string();
        }
        if let Some(p) = data.get("average_ping").and_then(Value::as_f64) {
            self.ping_ms = p.round().clamp(0.0, 60_000.0) as u32;
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
            "VOICE_CONNECTION_STATUS" => {
                self.set_connection_status(data);
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

// ── Channel history ────────────────────────────────────────────────────────

/// Saved channels kept per list (the plugin's limit).
pub const MAX_SAVED: usize = 5;

/// A voice channel you joined (or starred), for one-click rejoin.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SavedChannel {
    pub id: String,
    pub name: String,
    pub guild_id: String,
    pub guild_name: String,
    /// Unix seconds of the last join (sort key, newest first).
    pub last_joined: i64,
}

/// Recent (last [`MAX_SAVED`] joined) and favourite channels.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ChannelHistory {
    pub recent: Vec<SavedChannel>,
    pub favorites: Vec<SavedChannel>,
}

/// Snowflakes are decimal ids; anything else is not a channel we saved.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 24 && id.bytes().all(|b| b.is_ascii_digit())
}

fn clean(list: Vec<SavedChannel>) -> Vec<SavedChannel> {
    let mut out: Vec<SavedChannel> = Vec::new();
    for mut c in list {
        if !valid_id(&c.id) || out.iter().any(|o| o.id == c.id) {
            continue;
        }
        if !c.guild_id.is_empty() && !valid_id(&c.guild_id) {
            c.guild_id.clear();
        }
        if c.name.trim().is_empty() {
            c.name = "Voice channel".to_string();
        }
        out.push(c);
        if out.len() == MAX_SAVED {
            break;
        }
    }
    out
}

impl ChannelHistory {
    /// Parse the saved file; a broken or foreign file reads as empty (it
    /// is only a convenience list), bad entries are dropped.
    pub fn from_json(text: &str) -> Self {
        let h: Self = serde_json::from_str(text).unwrap_or_default();
        Self {
            recent: clean(h.recent),
            favorites: clean(h.favorites),
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn is_favorite(&self, id: &str) -> bool {
        self.favorites.iter().any(|c| c.id == id)
    }

    fn guild_name(&self, guild_id: &str) -> Option<String> {
        if guild_id.is_empty() {
            return None;
        }
        self.recent
            .iter()
            .chain(&self.favorites)
            .find(|c| c.guild_id == guild_id && !c.guild_name.is_empty())
            .map(|c| c.guild_name.clone())
    }

    /// Record a join: the channel moves to the front of the recent list
    /// and a favourite copy is refreshed (name / server may have changed).
    /// Returns whether anything changed (i.e. the file needs saving).
    pub fn remember(&mut self, ch: SavedChannel) -> bool {
        if !valid_id(&ch.id) {
            return false;
        }
        let before = self.clone();
        self.recent.retain(|c| c.id != ch.id);
        self.recent.insert(0, ch.clone());
        self.recent.truncate(MAX_SAVED);
        for f in self.favorites.iter_mut().filter(|f| f.id == ch.id) {
            *f = ch.clone();
        }
        *self != before
    }

    /// Star / unstar. Starring needs the channel's details: the current
    /// channel or a recent one. A new favourite goes first; the oldest
    /// falls off past [`MAX_SAVED`]. Returns whether anything changed.
    pub fn set_favorite(&mut self, id: &str, on: bool, current: Option<&SavedChannel>) -> bool {
        if !on {
            let n = self.favorites.len();
            self.favorites.retain(|c| c.id != id);
            return self.favorites.len() != n;
        }
        if self.is_favorite(id) {
            return false;
        }
        let Some(ch) = current
            .filter(|c| c.id == id)
            .or_else(|| self.recent.iter().find(|c| c.id == id))
            .cloned()
        else {
            return false;
        };
        self.favorites.insert(0, ch);
        self.favorites.truncate(MAX_SAVED);
        true
    }

    /// One list for the menu: favourites, then the other recent ones,
    /// each newest join first.
    pub fn entries(&self) -> Vec<(&SavedChannel, bool)> {
        let mut favs: Vec<&SavedChannel> = self.favorites.iter().collect();
        favs.sort_by_key(|c| std::cmp::Reverse(c.last_joined));
        let mut rest: Vec<&SavedChannel> = self
            .recent
            .iter()
            .filter(|c| !self.is_favorite(&c.id))
            .collect();
        rest.sort_by_key(|c| std::cmp::Reverse(c.last_joined));
        favs.into_iter()
            .map(|c| (c, true))
            .chain(rest.into_iter().map(|c| (c, false)))
            .collect()
    }

    /// GET_GUILD answered: fill the server name into every saved entry
    /// of that server. Returns whether anything changed.
    pub fn set_guild_name(&mut self, guild_id: &str, name: &str) -> bool {
        let mut changed = false;
        for c in self
            .recent
            .iter_mut()
            .chain(self.favorites.iter_mut())
            .filter(|c| c.guild_id == guild_id && c.guild_name != name)
        {
            c.guild_name = name.to_string();
            changed = true;
        }
        changed
    }

    pub fn find(&self, id: &str) -> Option<&SavedChannel> {
        self.favorites
            .iter()
            .chain(&self.recent)
            .find(|c| c.id == id)
    }
}

/// `discord://-/channels/<guild>/<channel>` — opens the channel in a
/// (starting) Discord client. `None` for a DM call or a bad id.
pub fn channel_deep_link(guild_id: &str, channel_id: &str) -> Option<String> {
    (valid_id(guild_id) && valid_id(channel_id))
        .then(|| format!("discord://-/channels/{guild_id}/{channel_id}"))
}

/// Programs tried to launch Discord, in order: the configured one (a
/// bare name or a path), else the stable, Canary and PTB clients.
pub fn launch_candidates(configured: &str) -> Vec<String> {
    let configured = configured.trim();
    if configured.is_empty() {
        ["discord", "discord-canary", "discord-ptb"]
            .map(str::to_string)
            .to_vec()
    } else {
        vec![configured.to_string()]
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

    // GET_VOICE_SETTINGS response (Discord RPC docs; volume is a float).
    #[test]
    fn voice_settings_read_the_input_volume() {
        let mut s = DiscordState::default();
        assert_eq!(s.input_volume, None, "unknown until Discord says");
        s.set_voice_settings(&json!({
            "input": {"available_devices": [{"id": "default", "name": "Default"}],
                      "device_id": "default", "volume": 49.803_92},
            "output": {"device_id": "default", "volume": 100.0},
            "mode": {"type": "VOICE_ACTIVITY"}, "automatic_gain_control": false,
            "mute": false, "deaf": false
        }));
        assert_eq!(s.input_volume, Some(50));
        s.set_voice_settings(&json!({"input": {"volume": 180.0}}));
        assert_eq!(s.input_volume, Some(100), "clamped to the input range");
        s.set_voice_settings(&json!({"mute": true}));
        assert_eq!(s.input_volume, Some(100), "a partial update keeps it");
    }

    #[test]
    fn connection_status_drives_the_latency_readout() {
        let mut s = DiscordState::default();
        s.set_channel(&channel());
        assert_eq!(s.latency(), Latency::Disconnected);
        s.apply_dispatch(
            "VOICE_CONNECTION_STATUS",
            &json!({"state": "AWAITING_ENDPOINT", "hostname": "", "pings": [], "average_ping": 0, "last_ping": 0}),
        );
        assert_eq!(s.latency(), Latency::Connecting);
        s.apply_dispatch(
            "VOICE_CONNECTION_STATUS",
            &json!({"state": "VOICE_CONNECTED", "hostname": "frankfurt1234.discord.media",
                    "pings": [30, 33], "average_ping": 31.5, "last_ping": 33}),
        );
        assert_eq!(s.latency(), Latency::Good(32));
        assert_eq!(s.latency().label(), "32 ms");
        assert_eq!(
            Latency::from_status("VOICE_CONNECTED", 0).label(),
            "Connected"
        );
        assert_eq!(
            Latency::from_status("VOICE_CONNECTED", 61),
            Latency::Fair(61)
        );
        assert_eq!(
            Latency::from_status("VOICE_CONNECTED", 121),
            Latency::Poor(121)
        );
        assert_eq!(Latency::from_status("NO_ROUTE", 0), Latency::NoRoute);
        s.leave();
        assert_eq!(s.latency(), Latency::Disconnected, "leaving resets it");
    }

    fn saved(id: &str, t: i64) -> SavedChannel {
        SavedChannel {
            id: id.into(),
            name: format!("ch{id}"),
            guild_id: "1".into(),
            guild_name: String::new(),
            last_joined: t,
        }
    }

    #[test]
    fn recent_channels_are_newest_first_unique_and_capped() {
        let mut h = ChannelHistory::default();
        for i in 1..=7 {
            assert!(h.remember(saved(&i.to_string(), i)));
        }
        assert_eq!(h.recent.len(), MAX_SAVED);
        assert_eq!(h.recent[0].id, "7");
        assert!(h.remember(saved("4", 10)), "a rejoin moves it up");
        let ids: Vec<&str> = h.recent.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["4", "7", "6", "5", "3"]);
        assert!(!h.remember(saved("4", 10)), "nothing new, no save");
        assert!(!h.remember(saved("../x", 11)), "not a snowflake");
    }

    #[test]
    fn favourites_star_unstar_and_list_first() {
        let mut h = ChannelHistory::default();
        h.remember(saved("2", 2));
        h.remember(saved("3", 3));
        assert!(!h.set_favorite("9", true, None), "unknown channel");
        let cur = saved("9", 9);
        assert!(h.set_favorite("9", true, Some(&cur)), "the current channel");
        assert!(h.set_favorite("2", true, None), "a recent one");
        assert!(!h.set_favorite("2", true, None), "already starred");
        let order: Vec<(&str, bool)> = h
            .entries()
            .iter()
            .map(|(c, f)| (c.id.as_str(), *f))
            .collect();
        assert_eq!(order, [("9", true), ("2", true), ("3", false)]);
        // A rejoin refreshes the favourite's copy too.
        h.remember(SavedChannel {
            name: "renamed".into(),
            ..saved("2", 20)
        });
        assert_eq!(
            h.favorites.iter().find(|c| c.id == "2").unwrap().name,
            "renamed"
        );
        assert!(h.set_favorite("9", false, None));
        assert!(!h.is_favorite("9"));
        assert!(!h.set_favorite("9", false, None));
        assert!(h.set_guild_name("1", "Margo"));
        assert!(h.recent.iter().all(|c| c.guild_name == "Margo"));
        assert!(!h.set_guild_name("1", "Margo"));
    }

    #[test]
    fn history_file_round_trips_and_drops_bad_entries() {
        let mut h = ChannelHistory::default();
        h.remember(saved("5", 5));
        h.set_favorite("5", true, None);
        assert_eq!(ChannelHistory::from_json(&h.to_json()), h);
        let messy = r#"{"recent": [{"id": "1", "name": ""}, {"id": "1"}, {"id": "x/y"},
                         {"id": "2", "guild_id": "../../etc"}], "favorites": "nope"}"#;
        let h = ChannelHistory::from_json(messy);
        assert!(
            h.favorites.is_empty() && h.recent.is_empty(),
            "wrong shape reads as empty"
        );
        let h = ChannelHistory::from_json(
            r#"{"recent": [{"id": "1", "name": ""}, {"id": "1"}, {"id": "x/y"}, {"id": "2", "guild_id": "../../etc"}]}"#,
        );
        let ids: Vec<&str> = h.recent.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["1", "2"]);
        assert_eq!(h.recent[0].name, "Voice channel");
        assert_eq!(h.recent[1].guild_id, "", "a bad server id is dropped");
        assert_eq!(
            ChannelHistory::from_json("not json"),
            ChannelHistory::default()
        );
    }

    #[test]
    fn entering_a_channel_keeps_its_server() {
        let mut s = DiscordState::default();
        s.history.remember(SavedChannel {
            guild_name: "Margo".into(),
            ..saved("7", 1)
        });
        s.set_channel(&channel());
        assert_eq!(s.guild_id, "1");
        assert_eq!(s.guild_name, "Margo", "known before GET_GUILD answers");
        let cur = s.current_channel(42).unwrap();
        assert_eq!(
            (cur.id.as_str(), cur.last_joined),
            ("199737254929760256", 42)
        );
    }

    #[test]
    fn deep_links_and_launch_candidates() {
        assert_eq!(
            channel_deep_link("1", "22").as_deref(),
            Some("discord://-/channels/1/22")
        );
        assert_eq!(
            channel_deep_link("", "22"),
            None,
            "DM call: no server route"
        );
        assert_eq!(channel_deep_link("1", "2; rm"), None);
        assert_eq!(
            launch_candidates(" "),
            ["discord", "discord-canary", "discord-ptb"]
        );
        assert_eq!(
            launch_candidates("/opt/discord/Discord"),
            ["/opt/discord/Discord"]
        );
    }
}
