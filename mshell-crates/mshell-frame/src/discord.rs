//! Discord local-RPC client for the Discord voice pill + menu.
//!
//! One shared connection for the whole shell (`subscribe` / `send`),
//! started lazily the first time a pill or menu subscribes and driven on
//! the shared wayle tokio runtime. Protocol framing and the voice-state
//! reducer live in `mshell_utils::discord` (pure, tested); this file is
//! only the socket loop:
//!
//! * find Discord's IPC socket and accept it only when its peer UID is
//!   ours (a `discord-ipc-N` in a shared `/tmp` could belong to someone
//!   else);
//! * handshake with the StreamKit client id, `AUTHENTICATE` with the
//!   stored token (or wait for the user's *Authorize*);
//! * subscribe to voice events, keep [`DiscordState`] current and
//!   publish it on a `watch` channel.
//!
//! While Discord isn't running the loop re-checks for the socket every
//! [`RECONNECT`] (a few `stat`s — no process spawn), but only while
//! something subscribes: with no pill and no open menu it parks and drops
//! any Discord connection, until the next [`subscribe`]. The access
//! token is written to `$XDG_STATE_HOME/margo/discord-token` with mode
//! 0600 and never logged.

use mshell_utils::discord::{
    self, CHANNEL_EVENTS, DiscordState, DiscordStatus, GLOBAL_EVENTS, OP_CLOSE, OP_FRAME,
    OP_HANDSHAKE, OP_PING, OP_PONG, TOKEN_EXCHANGE_URL,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
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

// ── Token storage ───────────────────────────────────────────────────────────

fn token_path() -> PathBuf {
    margo_fs::xdg::state_home().join("margo/discord-token")
}

fn load_token() -> Option<String> {
    std::fs::read_to_string(token_path())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn save_token(token: &str) {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let path = token_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path);
    if let Ok(mut f) = file {
        let _ = f.write_all(token.as_bytes());
    }
}

fn clear_token() {
    let _ = std::fs::remove_file(token_path());
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
    Other,
}

struct Session {
    w: OwnedWriteHalf,
    state: DiscordState,
    pending: HashMap<String, Kind>,
    nonce: u64,
    token: Option<String>,
    /// Channel whose events we're subscribed to.
    subscribed: Option<String>,
}

impl Session {
    async fn request(&mut self, cmd: &str, args: Value, kind: Kind, evt: Option<&str>) {
        self.nonce += 1;
        let nonce = self.nonce.to_string();
        self.pending.insert(nonce.clone(), kind);
        let payload = discord::command(cmd, args, &nonce, evt);
        if let Err(e) = write_frame(&mut self.w, OP_FRAME, &payload).await {
            debug!(error = %e, cmd, "discord write failed");
        }
    }

    async fn sync_subscriptions(&mut self) {
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
                )
                .await;
            }
        }
        if let Some(new) = want {
            for evt in CHANNEL_EVENTS {
                self.request(
                    "SUBSCRIBE",
                    json!({ "channel_id": new }),
                    Kind::Other,
                    Some(evt),
                )
                .await;
            }
            self.subscribed = Some(new);
        }
    }

    async fn on_ready_authenticated(&mut self) {
        for evt in GLOBAL_EVENTS {
            self.request("SUBSCRIBE", json!({}), Kind::Other, Some(evt))
                .await;
        }
        self.request(
            "GET_SELECTED_VOICE_CHANNEL",
            json!({}),
            Kind::SelectedChannel,
            None,
        )
        .await;
        self.request("GET_VOICE_SETTINGS", json!({}), Kind::VoiceSettings, None)
            .await;
    }

    /// Handle one payload; returns an AUTHORIZE code to exchange, if any.
    async fn on_payload(&mut self, p: Value) -> Option<String> {
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
                    clear_token();
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
                    save_token(t);
                }
                self.on_ready_authenticated().await;
            }
            Some(Kind::SelectedChannel) => {
                self.state.set_channel(&data);
                self.sync_subscriptions().await;
            }
            Some(Kind::VoiceSettings) => self.state.set_voice_settings(&data),
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
                    )
                    .await;
                } else if evt == "VOICE_CHANNEL_SELECT" {
                    // Left the channel: drop its event subscriptions.
                    self.sync_subscriptions().await;
                }
            }
        }
        None
    }

    async fn authenticate(&mut self, token: String) {
        self.token = Some(token.clone());
        self.state.status = DiscordStatus::Authorizing;
        self.request(
            "AUTHENTICATE",
            json!({ "access_token": token }),
            Kind::Authenticate,
            None,
        )
        .await;
    }

    async fn on_command(&mut self, cmd: DiscordCommand) {
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
                )
                .await;
            }
            DiscordCommand::SetMute(m) if ready => {
                self.request(
                    "SET_VOICE_SETTINGS",
                    json!({ "mute": m }),
                    Kind::VoiceSettings,
                    None,
                )
                .await;
            }
            DiscordCommand::SetDeaf(d) if ready => {
                self.request(
                    "SET_VOICE_SETTINGS",
                    json!({ "deaf": d }),
                    Kind::VoiceSettings,
                    None,
                )
                .await;
            }
            DiscordCommand::Leave if ready => {
                self.request(
                    "SELECT_VOICE_CHANNEL",
                    json!({ "channel_id": null }),
                    Kind::Other,
                    None,
                )
                .await;
                self.state.leave();
                self.sync_subscriptions().await;
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
                )
                .await;
            }
            _ => {}
        }
    }
}

async fn session(
    stream: UnixStream,
    state_tx: &watch::Sender<DiscordState>,
    cmd_rx: &mut mpsc::UnboundedReceiver<DiscordCommand>,
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
    let mut s = Session {
        w,
        state: DiscordState {
            status: DiscordStatus::NeedsAuth,
            ..Default::default()
        },
        pending: HashMap::new(),
        nonce: 0,
        token: None,
        subscribed: None,
    };
    if let Some(t) = load_token() {
        s.authenticate(t).await;
    }
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
                    OP_PING => { let _ = write_frame(&mut s.w, OP_PONG, &v).await; }
                    OP_CLOSE => break,
                    OP_FRAME => {
                        if let Some(code) = s.on_payload(v).await {
                            let tx = code_tx.clone();
                            tokio::task::spawn_blocking(move || {
                                let _ = tx.send(exchange_code(&code));
                            });
                        }
                    }
                    _ => {}
                }
            }
            Some(token) = code_rx.recv() => match token {
                Ok(t) => s.authenticate(t).await,
                Err(e) => {
                    s.state.status = DiscordStatus::NeedsAuth;
                    s.state.message = e;
                }
            },
            Some(cmd) = cmd_rx.recv() => s.on_command(cmd).await,
        }
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
                () = session(stream, &state_tx, &mut cmd_rx) => {}
                // The last subscriber left: drop the connection too.
                () = state_tx.closed() => {}
            }
        }
        state_tx.send_if_modified(|cur| {
            let idle = DiscordState::default();
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static CONNECTS: AtomicUsize = AtomicUsize::new(0);

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
