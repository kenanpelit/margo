//! Discord local-RPC client for the Discord voice pill + menu.
//!
//! One shared connection for the whole shell (`subscribe` / `send`),
//! started lazily the first time a pill or menu subscribes and driven on
//! the shared wayle tokio runtime. Protocol framing and the voice-state
//! reducer live in `mshell_utils::discord` (pure, tested); this file is
//! the session state machine ([`Session`], I/O-free, tested in
//! `discord_tests.rs`) and the socket loop:
//!
//! * find Discord's IPC socket and accept it only when its peer UID is
//!   ours (a `discord-ipc-N` in a shared `/tmp` could belong to someone
//!   else);
//! * handshake with the StreamKit client id, `AUTHENTICATE` with the
//!   stored token (or wait for the user's *Authorize*);
//! * subscribe to voice events, keep [`DiscordState`] current and
//!   publish it on a `watch` channel.
//!
//! Recent and favourite voice channels (for one-click rejoin) are kept in
//! `$XDG_STATE_HOME/margo/discord-channels.json` (mode 0600) and shown
//! even while Discord isn't running.
//!
//! While Discord isn't running the loop re-checks for the socket every
//! [`RECONNECT`] (a few `stat`s — no process spawn), but only while
//! something subscribes: with no pill and no open menu it parks and drops
//! any Discord connection, until the next [`subscribe`]. The access
//! token is written to `$XDG_STATE_HOME/margo/discord-token` with mode
//! 0600 and never logged.

use mshell_utils::discord::{
    self, CHANNEL_EVENTS, ChannelHistory, DiscordState, DiscordStatus, GLOBAL_EVENTS, OP_CLOSE,
    OP_FRAME, OP_HANDSHAKE, OP_PING, OP_PONG, TOKEN_EXCHANGE_URL,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{Notify, mpsc, watch};
use tracing::{debug, warn};

const RECONNECT: Duration = Duration::from_secs(5);

/// User actions from the menu.
#[derive(Debug, Clone)]
pub(crate) enum DiscordCommand {
    Authorize,
    SetMute(bool),
    SetDeaf(bool),
    Leave,
    /// (user id, volume 0..=200)
    SetUserVolume(String, u32),
    /// Your microphone input volume, 0..=100.
    SetInputVolume(u32),
    /// Join a saved (recent / favourite) voice channel by id.
    JoinChannel(String),
    /// Star / unstar a channel (the current one or a recent one).
    SetFavorite(String, bool),
}

struct Handle {
    /// Its receiver count is the number of live subscribers.
    state: watch::Sender<DiscordState>,
    cmd: mpsc::UnboundedSender<DiscordCommand>,
    /// Wakes a parked client when someone subscribes.
    wake: Arc<Notify>,
}

fn handle() -> &'static Handle {
    static H: OnceLock<Handle> = OnceLock::new();
    H.get_or_init(|| {
        let (state_tx, _) = watch::channel(DiscordState::default());
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let wake = Arc::new(Notify::new());
        mshell_services::tokio_rt_spawn(run(state_tx.clone(), cmd_rx, wake.clone(), connect_boxed));
        Handle {
            state: state_tx,
            cmd: cmd_tx,
            wake,
        }
    })
}

/// A receiver of the live state. The client runs while at least one
/// receiver exists: hold it only while showing the state.
pub(crate) fn subscribe() -> watch::Receiver<DiscordState> {
    let h = handle();
    let rx = h.state.subscribe();
    h.wake.notify_one();
    rx
}

pub(crate) fn send(cmd: DiscordCommand) {
    let _ = handle().cmd.send(cmd);
}

// ── Launching Discord ───────────────────────────────────────────────────────

/// The Discord program to start: the configured one, else the first
/// installed client (see [`discord::launch_candidates`]), on `path`.
fn discord_program_in(configured: &str, path: &std::ffi::OsStr) -> Option<PathBuf> {
    discord::launch_candidates(configured)
        .into_iter()
        .find_map(|c| {
            if c.contains('/') {
                let p = match (c.strip_prefix("~/"), margo_fs::xdg::home()) {
                    (Some(rest), Some(home)) => home.join(rest),
                    _ => PathBuf::from(&c),
                };
                margo_fs::path::is_executable(&p).then_some(p)
            } else {
                margo_fs::path::find_in_path_in(&c, path)
            }
        })
}

/// The Discord program for `configured` on `$PATH`, if any (stats only).
pub(crate) fn discord_program(configured: &str) -> Option<PathBuf> {
    discord_program_in(configured, &std::env::var_os("PATH").unwrap_or_default())
}

/// The argv after the program: `--url -- <deep link>` to open a channel.
fn launch_args(link: Option<&str>) -> Vec<String> {
    link.map(|l| vec!["--url".to_string(), "--".to_string(), l.to_string()])
        .unwrap_or_default()
}

/// Start Discord (in its own scope, so it outlives an mshell restart),
/// optionally on a channel's deep link. No shell is involved.
pub(crate) fn launch(configured: &str, link: Option<&str>) -> Result<(), String> {
    use mshell_process::SpawnReaped;
    let program = discord_program(configured).ok_or_else(|| {
        if configured.trim().is_empty() {
            "Discord isn't installed (discord, discord-canary or discord-ptb)".to_string()
        } else {
            format!("Can't run {}", configured.trim())
        }
    })?;
    mshell_process::outliving_shell(&program, launch_args(link), "discord")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn_reaped()
        .map(|_| ())
        .map_err(|e| format!("Couldn't start Discord: {e}"))
}

/// [`launch`], with a failure surfaced as a toast (pill + menu clicks).
pub(crate) fn launch_or_toast(configured: &str, link: Option<&str>) {
    if let Err(e) = launch(configured, link) {
        use mshell_config::toast::{ToastEvent, ToastSeverity, push_toast};
        warn!(error = %e, "discord: launch failed");
        push_toast(ToastEvent {
            icon: "call-start-symbolic".to_string(),
            title: "Couldn't start Discord".to_string(),
            body: Some(e),
            severity: ToastSeverity::Warn,
        });
    }
}

// ── Token storage ───────────────────────────────────────────────────────────

fn token_path() -> PathBuf {
    margo_fs::xdg::state_home().join("margo/discord-token")
}

/// The channel history lives beside the token.
fn history_path(token_file: &Path) -> PathBuf {
    token_file.with_file_name("discord-channels.json")
}

fn load_history(path: &Path) -> ChannelHistory {
    std::fs::read_to_string(path)
        .map(|t| ChannelHistory::from_json(&t))
        .unwrap_or_default()
}

/// The idle state (Discord not running): only the saved channels.
pub(crate) fn idle_state() -> DiscordState {
    DiscordState {
        history: load_history(&history_path(&token_path())),
        ..Default::default()
    }
}

fn load_token(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Write a private file (mode 0600 from creation, never world-readable
/// in between) via a sibling temp file + rename.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = margo_fs::atomic_write::tmp_sibling(path);
    let written = (|| {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

fn save_token(path: &Path, token: &str) {
    if let Err(e) = write_private(path, token.as_bytes()) {
        warn!(error = %e, "discord: could not store the access token");
    }
}

fn save_history(path: &Path, history: &ChannelHistory) {
    if let Err(e) = write_private(path, history.to_json().as_bytes()) {
        warn!(error = %e, "discord: could not save the channel history");
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn clear_token(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// StreamKit's broker turns the AUTHORIZE code into an access token.
/// Blocking (ureq) — run on a blocking thread.
fn exchange_code(code: &str) -> Result<String, String> {
    let resp = ureq::post(TOKEN_EXCHANGE_URL)
        .set("Content-Type", "application/json")
        .set("User-Agent", "mshell-discord-voice")
        .timeout(Duration::from_secs(15))
        .send_string(&json!({ "code": code }).to_string())
        .map_err(|e| format!("token exchange failed: {e}"))?;
    let v: Value = resp
        .into_json()
        .map_err(|e| format!("token exchange: {e}"))?;
    v.get("access_token")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "token exchange returned no token".to_string())
}

// ── Socket ──────────────────────────────────────────────────────────────────

fn own_uid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self").ok().map(|m| m.uid())
}

async fn connect() -> Option<UnixStream> {
    let runtime = margo_fs::xdg::runtime_dir();
    let tmp: Vec<PathBuf> = ["TMPDIR", "TMP", "TEMP"]
        .iter()
        .filter_map(|v| std::env::var_os(v).map(PathBuf::from))
        .collect();
    let uid = own_uid()?;
    for path in discord::socket_candidates(runtime.as_deref(), &tmp) {
        if !path.exists() {
            continue;
        }
        let Ok(stream) = UnixStream::connect(&path).await else {
            continue;
        };
        match stream.peer_cred() {
            Ok(c) if c.uid() == uid => return Some(stream),
            _ => warn!(?path, "rejected Discord IPC socket owned by another user"),
        }
    }
    None
}

async fn write_frame(w: &mut OwnedWriteHalf, op: u32, payload: &Value) -> std::io::Result<()> {
    w.write_all(&discord::encode_frame(op, payload)).await
}

async fn read_frame(r: &mut OwnedReadHalf) -> Result<(u32, Value), String> {
    let mut h = [0u8; 8];
    r.read_exact(&mut h).await.map_err(|e| e.to_string())?;
    let (op, len) = discord::decode_header(&h)?;
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body).await.map_err(|e| e.to_string())?;
    let v = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    Ok((op, v))
}

// ── Session ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Authorize,
    Authenticate,
    SelectedChannel,
    VoiceSettings,
    /// GET_GUILD for this server id (its name for the header / history).
    Guild(String),
    /// SELECT_VOICE_CHANNEL from a saved-channel click.
    JoinChannel,
    Other,
}

/// The protocol state machine, free of I/O: requests are queued in
/// [`Session::out`] and written by [`session`] after each step, so the
/// decisions are unit-tested without a socket (`discord_tests.rs`).
struct Session {
    state: DiscordState,
    pending: HashMap<String, Kind>,
    nonce: u64,
    token: Option<String>,
    /// Channel whose events we're subscribed to.
    subscribed: Option<String>,
    /// Where the access token is stored.
    token_file: PathBuf,
    /// Where the recent / favourite channels are stored.
    history_file: PathBuf,
    /// Command payloads not yet written to the socket, in order.
    out: Vec<Value>,
}

impl Session {
    fn new(token_file: PathBuf) -> Self {
        let history_file = history_path(&token_file);
        Self {
            state: DiscordState {
                status: DiscordStatus::NeedsAuth,
                history: load_history(&history_file),
                ..Default::default()
            },
            pending: HashMap::new(),
            nonce: 0,
            token: None,
            subscribed: None,
            token_file,
            history_file,
            out: Vec::new(),
        }
    }

    /// A channel object (GET_SELECTED_VOICE_CHANNEL / SELECT_VOICE_CHANNEL
    /// response): enter it, record a new join in the history and look up
    /// the server's name once.
    fn enter_channel(&mut self, data: &Value) {
        let before = self.state.channel_id.clone();
        self.state.set_channel(data);
        self.sync_subscriptions();
        if self.state.channel_id.is_none() || self.state.channel_id == before {
            return;
        }
        if let Some(ch) = self.state.current_channel(now_secs())
            && self.state.history.remember(ch)
        {
            save_history(&self.history_file, &self.state.history);
        }
        let guild = self.state.guild_id.clone();
        if !guild.is_empty() {
            self.request(
                "GET_GUILD",
                json!({ "guild_id": guild }),
                Kind::Guild(guild.clone()),
                None,
            );
        }
    }

    fn on_guild(&mut self, guild_id: &str, data: &Value) {
        let Some(name) = data.get("name").and_then(Value::as_str) else {
            return;
        };
        if self.state.guild_id == guild_id {
            self.state.guild_name = name.to_string();
        }
        if self.state.history.set_guild_name(guild_id, name) {
            save_history(&self.history_file, &self.state.history);
        }
    }

    fn request(&mut self, cmd: &str, args: Value, kind: Kind, evt: Option<&str>) {
        self.nonce += 1;
        let nonce = self.nonce.to_string();
        self.pending.insert(nonce.clone(), kind);
        self.out.push(discord::command(cmd, args, &nonce, evt));
    }

    /// Write the queued requests.
    async fn flush(&mut self, w: &mut OwnedWriteHalf) {
        for payload in std::mem::take(&mut self.out) {
            if let Err(e) = write_frame(w, OP_FRAME, &payload).await {
                debug!(error = %e, cmd = ?payload.get("cmd"), "discord write failed");
            }
        }
    }

    fn sync_subscriptions(&mut self) {
        let want = self.state.channel_id.clone();
        if want == self.subscribed {
            return;
        }
        if let Some(old) = self.subscribed.take() {
            for evt in CHANNEL_EVENTS {
                self.request(
                    "UNSUBSCRIBE",
                    json!({ "channel_id": old }),
                    Kind::Other,
                    Some(evt),
                );
            }
        }
        if let Some(new) = want {
            for evt in CHANNEL_EVENTS {
                self.request(
                    "SUBSCRIBE",
                    json!({ "channel_id": new }),
                    Kind::Other,
                    Some(evt),
                );
            }
            self.subscribed = Some(new);
        }
    }

    fn on_ready_authenticated(&mut self) {
        for evt in GLOBAL_EVENTS {
            self.request("SUBSCRIBE", json!({}), Kind::Other, Some(evt));
        }
        self.request(
            "GET_SELECTED_VOICE_CHANNEL",
            json!({}),
            Kind::SelectedChannel,
            None,
        );
        self.request("GET_VOICE_SETTINGS", json!({}), Kind::VoiceSettings, None);
    }

    /// Handle one payload; returns an AUTHORIZE code to exchange, if any.
    fn on_payload(&mut self, p: Value) -> Option<String> {
        let nonce = p.get("nonce").and_then(Value::as_str).map(str::to_string);
        let evt = p
            .get("evt")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let data = p.get("data").cloned().unwrap_or(Value::Null);
        let kind = nonce.as_ref().and_then(|n| self.pending.remove(n));

        if evt == "ERROR" {
            let msg = data
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Discord RPC error")
                .to_string();
            match kind {
                Some(Kind::Authenticate) => {
                    clear_token(&self.token_file);
                    self.token = None;
                    self.state.status = DiscordStatus::NeedsAuth;
                    self.state.message = "Authorization expired — authorize again".to_string();
                }
                Some(Kind::Authorize) => {
                    self.state.status = DiscordStatus::NeedsAuth;
                    self.state.message = msg;
                }
                _ => self.state.message = msg,
            }
            return None;
        }
        match kind {
            Some(Kind::Authorize) => {
                return data
                    .get("code")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| {
                        self.state.status = DiscordStatus::NeedsAuth;
                        self.state.message = "Authorization was not completed".to_string();
                        None
                    });
            }
            Some(Kind::Authenticate) => {
                self.state.user_id = data
                    .pointer("/user/id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                self.state.status = DiscordStatus::Ready;
                self.state.message.clear();
                if let Some(t) = &self.token {
                    save_token(&self.token_file, t);
                }
                self.on_ready_authenticated();
            }
            Some(Kind::SelectedChannel | Kind::JoinChannel) => self.enter_channel(&data),
            Some(Kind::VoiceSettings) => self.state.set_voice_settings(&data),
            Some(Kind::Guild(id)) => self.on_guild(&id, &data),
            Some(Kind::Other) => {}
            None => {
                if p.get("cmd").and_then(Value::as_str) == Some("DISPATCH")
                    && self.state.apply_dispatch(&evt, &data)
                {
                    self.request(
                        "GET_SELECTED_VOICE_CHANNEL",
                        json!({}),
                        Kind::SelectedChannel,
                        None,
                    );
                } else if evt == "VOICE_CHANNEL_SELECT" {
                    // Left the channel: drop its event subscriptions.
                    self.sync_subscriptions();
                }
            }
        }
        None
    }

    fn authenticate(&mut self, token: String) {
        self.token = Some(token.clone());
        self.state.status = DiscordStatus::Authorizing;
        self.request(
            "AUTHENTICATE",
            json!({ "access_token": token }),
            Kind::Authenticate,
            None,
        );
    }

    /// Result of the StreamKit code exchange.
    fn on_token_exchange(&mut self, token: Result<String, String>) {
        match token {
            Ok(t) => self.authenticate(t),
            Err(e) => {
                self.state.status = DiscordStatus::NeedsAuth;
                self.state.message = e;
            }
        }
    }

    fn on_command(&mut self, cmd: DiscordCommand) {
        let ready = self.state.status == DiscordStatus::Ready;
        match cmd {
            DiscordCommand::Authorize if self.state.status == DiscordStatus::NeedsAuth => {
                self.state.status = DiscordStatus::Authorizing;
                self.state.message.clear();
                self.request(
                    "AUTHORIZE",
                    discord::authorize_args(),
                    Kind::Authorize,
                    None,
                );
            }
            DiscordCommand::SetMute(m) if ready => {
                self.request(
                    "SET_VOICE_SETTINGS",
                    json!({ "mute": m }),
                    Kind::VoiceSettings,
                    None,
                );
            }
            DiscordCommand::SetDeaf(d) if ready => {
                self.request(
                    "SET_VOICE_SETTINGS",
                    json!({ "deaf": d }),
                    Kind::VoiceSettings,
                    None,
                );
            }
            DiscordCommand::Leave if ready => {
                self.request(
                    "SELECT_VOICE_CHANNEL",
                    json!({ "channel_id": null }),
                    Kind::Other,
                    None,
                );
                self.state.leave();
                self.sync_subscriptions();
            }
            DiscordCommand::SetUserVolume(id, v) if ready => {
                let v = v.min(200);
                if let Some(p) = self.state.participants.get_mut(&id) {
                    p.volume = v; // optimistic; Discord echoes via VOICE_STATE_UPDATE
                }
                self.request(
                    "SET_USER_VOICE_SETTINGS",
                    json!({ "user_id": id, "volume": v }),
                    Kind::Other,
                    None,
                );
            }
            DiscordCommand::SetInputVolume(v) if ready => {
                let v = v.min(100);
                self.state.input_volume = Some(v); // optimistic; the reply carries the settings
                self.request(
                    "SET_VOICE_SETTINGS",
                    json!({ "input": { "volume": v } }),
                    Kind::VoiceSettings,
                    None,
                );
            }
            DiscordCommand::JoinChannel(id)
                if ready && !self.state.in_voice() && self.state.history.find(&id).is_some() =>
            {
                self.state.message.clear();
                self.request(
                    "SELECT_VOICE_CHANNEL",
                    json!({ "channel_id": id }),
                    Kind::JoinChannel,
                    None,
                );
            }
            DiscordCommand::SetFavorite(id, on) => {
                let current = self.state.current_channel(now_secs());
                if self.state.history.set_favorite(&id, on, current.as_ref()) {
                    save_history(&self.history_file, &self.state.history);
                }
            }
            _ => {}
        }
    }
}

async fn session(
    stream: UnixStream,
    state_tx: &watch::Sender<DiscordState>,
    cmd_rx: &mut mpsc::UnboundedReceiver<DiscordCommand>,
    token_file: PathBuf,
) {
    let (mut r, mut w) = stream.into_split();
    if write_frame(&mut w, OP_HANDSHAKE, &discord::handshake_payload())
        .await
        .is_err()
    {
        return;
    }
    match read_frame(&mut r).await {
        Ok((OP_FRAME, v)) if v.get("evt").and_then(Value::as_str) == Some("READY") => {}
        _ => return,
    }
    let mut s = Session::new(token_file);
    if let Some(t) = load_token(&s.token_file) {
        s.authenticate(t);
    }
    s.flush(&mut w).await;
    state_tx.send_replace(s.state.clone());

    // `read_exact` isn't cancel-safe, so frames are read on their own task
    // and handed over a channel the select below can poll safely.
    let (frame_tx, mut frame_rx) = mpsc::unbounded_channel::<(u32, Value)>();
    let reader = tokio::spawn(async move {
        while let Ok(frame) = read_frame(&mut r).await {
            if frame_tx.send(frame).is_err() {
                break;
            }
        }
    });
    // Also when the session is dropped mid-way (last subscriber left).
    let _reader = AbortOnDrop(reader.abort_handle());
    let (code_tx, mut code_rx) = mpsc::unbounded_channel::<Result<String, String>>();
    loop {
        tokio::select! {
            frame = frame_rx.recv() => {
                let Some((op, v)) = frame else { break };
                match op {
                    OP_PING => { let _ = write_frame(&mut w, OP_PONG, &v).await; }
                    OP_CLOSE => break,
                    OP_FRAME => {
                        if let Some(code) = s.on_payload(v) {
                            let tx = code_tx.clone();
                            tokio::task::spawn_blocking(move || {
                                let _ = tx.send(exchange_code(&code));
                            });
                        }
                    }
                    _ => {}
                }
            }
            Some(token) = code_rx.recv() => s.on_token_exchange(token),
            Some(cmd) = cmd_rx.recv() => s.on_command(cmd),
        }
        s.flush(&mut w).await;
        state_tx.send_if_modified(|cur| {
            let changed = *cur != s.state;
            if changed {
                *cur = s.state.clone();
            }
            changed
        });
    }
}

/// Aborts a task when dropped.
struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Park until `tx` has a receiver (`wake` is notified on subscribe).
async fn wait_for_subscriber<T>(tx: &watch::Sender<T>, wake: &Notify) {
    while tx.receiver_count() == 0 {
        wake.notified().await;
    }
}

type ConnectFuture = std::pin::Pin<Box<dyn Future<Output = Option<UnixStream>> + Send>>;

fn connect_boxed() -> ConnectFuture {
    Box::pin(connect())
}

async fn run(
    state_tx: watch::Sender<DiscordState>,
    mut cmd_rx: mpsc::UnboundedReceiver<DiscordCommand>,
    wake: Arc<Notify>,
    connect: impl Fn() -> ConnectFuture,
) {
    loop {
        // Nobody shows Discord state: no socket probing at all.
        wait_for_subscriber(&state_tx, &wake).await;
        if let Some(stream) = connect().await {
            tokio::select! {
                () = session(stream, &state_tx, &mut cmd_rx, token_path()) => {}
                // The last subscriber left: drop the connection too.
                () = state_tx.closed() => {}
            }
        }
        let idle = idle_state();
        state_tx.send_if_modified(|cur| {
            let changed = *cur != idle;
            *cur = idle;
            changed
        });
        // Wait for Discord; drop commands that arrive meanwhile (there is
        // nothing to send them to).
        let sleep = tokio::time::sleep(RECONNECT);
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                () = &mut sleep => break,
                () = state_tx.closed() => break,
                c = cmd_rx.recv() => if c.is_none() { return },
            }
        }
    }
}

#[path = "discord_tests.rs"]
#[cfg(test)]
mod discord_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static CONNECTS: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn launcher_prefers_the_configured_program_then_the_first_client() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let exe = |name: &str| {
            let p = dir.path().join(name);
            std::fs::write(&p, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        };
        let path = dir.path().as_os_str();
        assert_eq!(discord_program_in("", path), None, "nothing installed");
        let canary = exe("discord-canary");
        assert_eq!(discord_program_in("", path), Some(canary.clone()));
        let stable = exe("discord");
        assert_eq!(discord_program_in("", path), Some(stable), "stable first");
        assert_eq!(
            discord_program_in("discord-canary", path),
            Some(canary.clone())
        );
        assert_eq!(
            discord_program_in(canary.to_str().unwrap(), std::ffi::OsStr::new("")),
            Some(canary),
            "a full path needs no $PATH"
        );
        assert_eq!(
            discord_program_in("vesktop", path),
            None,
            "configured but missing"
        );
        assert!(launch_args(None).is_empty());
        assert_eq!(
            launch_args(Some("discord://-/channels/1/2")),
            ["--url", "--", "discord://-/channels/1/2"]
        );
    }

    fn no_discord() -> ConnectFuture {
        CONNECTS.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { None })
    }

    async fn settle() {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }

    /// The reconnect loop probes for Discord's socket only while someone
    /// subscribes; with no pill and no open menu it parks.
    #[tokio::test(start_paused = true)]
    async fn probes_only_while_subscribed() {
        let (state_tx, _) = watch::channel(DiscordState::default());
        let (_cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let wake = Arc::new(Notify::new());
        tokio::spawn(run(state_tx.clone(), cmd_rx, wake.clone(), no_discord));
        tokio::time::sleep(RECONNECT * 12).await;
        settle().await;
        assert_eq!(
            CONNECTS.load(Ordering::SeqCst),
            0,
            "no subscriber, no probe"
        );

        let rx = state_tx.subscribe();
        wake.notify_one();
        settle().await;
        assert_eq!(CONNECTS.load(Ordering::SeqCst), 1, "probes on subscribe");
        tokio::time::sleep(RECONNECT * 3).await;
        settle().await;
        let while_shown = CONNECTS.load(Ordering::SeqCst);
        assert!(while_shown >= 3, "keeps looking while shown: {while_shown}");

        drop(rx);
        tokio::time::sleep(RECONNECT * 12).await;
        settle().await;
        assert!(
            CONNECTS.load(Ordering::SeqCst) <= while_shown + 1,
            "parks once the last subscriber is gone"
        );
    }
}
